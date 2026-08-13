//! Projection work namespaces cannot be opened by external callers from a
//! caller-constructed endpoint binding:
//!
//! ```compile_fail
//! use tine_core::oplog::{ObjectStore, ProjectionEndpointBinding};
//!
//! fn preclaim(store: &ObjectStore, binding: ProjectionEndpointBinding) {
//!     let _ = store.open_projection_work_index(binding);
//! }
//! ```

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ahash::AHashMap;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions, ReadDir};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smallvec::SmallVec;
use uuid::Uuid;

use super::enrollment::{EnrollmentBindingV1, ResumePointEnrollmentBinding};
use super::hot_engine::RuntimeResumeSnapshot;
use super::identity::{parse_digest, ARCHIVE_INSTANCE_CLAIM_FILE};
use super::resume_point::{
    clear_resume_points_in, next_resume_sequence, prune_resume_points_below,
    ResumeEnrollmentAdmission, ResumePointError, ResumePointMaintenance, ResumePointScan,
    ResumePointSet, RuntimeResumePointV2, MAX_RETAINED_RESUME_POINTS, RESUME_POINT_DIR,
};
use super::scratch_store::MAX_RETAINED_SCRATCH_RUNS;
use super::shadow_projection::PromotedBootstrapProjectionBindingV1;
use super::simulator::SimulatorBootstrapFixtureIngress;
use super::sqlite::{ProjectionError, WorkspaceRuntimeProof};
#[cfg(test)]
use super::sync_layout::BLOCK_CLAIM_INDEX_DIR;
use super::sync_layout::{
    ARCHIVE_BATCHES_DIR as BATCHES_DIR, ARCHIVE_BOOTSTRAP_DIR as BOOTSTRAP_DIR,
    ARCHIVE_OBJECTS_DIR as OBJECTS_DIR, BLOCK_CLAIM_INDEX_FILE, BOOTSTRAP_AGGREGATES_DIR,
    BOOTSTRAP_COMMITS_DIR, BOOTSTRAP_EVIDENCE_DIR, BOOTSTRAP_OBJECTS_DIR, BOOTSTRAP_PARTS_DIR,
    BOOTSTRAP_PART_PACKS_DIR, BOOTSTRAP_PART_SPANS_DIR, BOOTSTRAP_SOURCE_BLOB_DIR,
    BOOTSTRAP_SOURCE_CHUNKS_DIR, BOOTSTRAP_SOURCE_INVENTORY_DIR, ENGINE_HISTORY_CLAIM_FILE,
    ENGINE_HISTORY_DIR, ENGINE_HISTORY_HEAD_FILE, ENGINE_HISTORY_NODES_DIR,
    ENGINE_HISTORY_ROOTS_DIR, ENGINE_HISTORY_ROOT_SUFFIX, ENGINE_HISTORY_TRANSITION_LOCK_FILE,
    LINEAGE_CLAIM_FILE, LOGSEQ_CLAIM_INDEX_DIR, PAGE_NAME_OWNERSHIP_INDEX_DIR,
    PORTABLE_PATH_INDEX_DIR, PROJECTION_WORK_DIR, PROMOTED_RUNTIME_STATE_FILE,
    REFERENCE_CATALOG_DIR,
};
use super::watcher_queue::WatcherQuiescedProof;
use super::{
    bootstrap_import::{
        ArchiveLocalFrontierBindingV1, BootstrapAggregateCommitV1, BootstrapAggregateDigestV1,
        BootstrapAggregateManifestV1, BootstrapImportError, BootstrapImportPartEvidenceV1,
        BootstrapManifestFingerprintV1, BootstrapPartDescriptorV1, BootstrapPartSpanIndexV1,
        BootstrapPublicationIdV1, FullObjectDescriptorV1, PayloadObjectDescriptorV1,
        SourceBlobChunkDescriptorV1, SourceBlobChunkDigestV1, SourceBlobChunkRootBuilderV1,
        SourceBlobChunkRootV1, SourceBlobIndexPageV1, SourceBlobIndexValidatorV1,
        SourceInventoryIndexPageV1, SourceInventoryIndexValidatorV1, SourceInventoryRootV1,
        SourceLeafV1, MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART,
        MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES, MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES,
        MAX_BOOTSTRAP_PART_EVIDENCE_BYTES, MAX_OPERATIONS_PER_BOOTSTRAP_PART,
        MAX_PART_SPAN_INDEX_BYTES, MAX_SOURCE_BLOB_CHUNK_BYTES, MAX_SOURCE_INDEX_PAGE_BYTES,
    },
    BatchError, BatchId, BatchOrigin, CanonicalArchiveResourceId, ContentDigest, DeviceId,
    DocumentId, ImportId, LineageDigest, ObjectDescriptor, OperationBatch, OperationObject,
    PreparedBatch, SessionId, ValidatedBatch, WorkspaceId, MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES,
};
use crate::model::HandoffSafe;

const MAX_BOOTSTRAP_PART_PACK_BYTES: u64 =
    MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART + 4 * MAX_OPERATIONS_PER_BOOTSTRAP_PART as u64;

/// Retained, O(1)-memory enumeration of immutable manifest commit markers.
///
/// The cursor deliberately preserves the filesystem iterator instead of
/// materializing and sorting the complete archive. Callers which need a full
/// audit continue to use [`ObjectStore::committed_manifests`].
pub(crate) struct ObjectStoreManifestCursor {
    entries: ReadDir,
}
const ENGINE_HISTORY_ROOT_SCHEMA_VERSION: u32 = 8;
/// Device-local promoted-runtime state, published beside the endpoint's durable
/// engine history.
/// The first honest promoted-runtime state format. No earlier experimental
/// bytes were ever published, and any other value is rejected rather than
/// reinterpreted.
pub(crate) const PROMOTED_RUNTIME_STATE_SCHEMA_VERSION: u32 = 2;
const MAX_PROMOTED_RUNTIME_STATE_BYTES: u64 = 4096;
const MAX_ENGINE_HISTORY_RECORD_BYTES: u64 = 1024 * 1024;
const MAX_ENGINE_HISTORY_INDEX_BYTES: u64 = 2 * 1024 * 1024;
const ENGINE_HISTORY_INDEX_SCHEMA_VERSION: u32 = 1;
pub(crate) const ENGINE_HISTORY_RADIX_DEPTH: u8 = 32;
const BLOCK_CLAIM_INDEX_SCHEMA_VERSION: u32 = 1;
const BLOCK_CLAIM_RADIX_DEPTH: u8 = 32;
// Large replay batches touch most hash prefixes. Keeping tens of thousands of
// compact claim records per leaf bounds point depth while avoiding hundreds
// of thousands of tiny copy-on-write page appends and syscalls. The encoded
// page byte ceiling remains the independent fail-closed bound.
const BLOCK_CLAIM_LEAF_ENTRIES: usize = 65_536;
const BLOCK_CLAIM_INDEX_LEVELS: usize = 8;
const BLOCK_CLAIM_SEGMENTS_PER_LEVEL: usize = 32;
const BLOCK_CLAIM_FILTER_BITS_PER_ENTRY: usize = 16;
const BLOCK_CLAIM_FILTER_HASHES: u64 = 7;
const BLOCK_CLAIM_GLOBAL_FILTER_BYTES: usize = 1024 * 1024;
const MAX_BLOCK_CLAIM_RECORD_BYTES: usize = 64 * 1024;
const MAX_BLOCK_CLAIM_PAGE_BYTES: usize = 8 * 1024 * 1024;

thread_local! {
    // This one hook is also used by the crate-private deterministic simulator.
    // It is deliberately narrower than a general object-store fault injector:
    // the only observable boundary is after every immutable object is durable
    // and before the manifest commit marker is published.
    static HARNESS_PUBLISH_FAIL_AFTER_OBJECTS: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    static ENROLLED_OPEN_USE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static ENROLLED_OPEN_ACT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static SEALED_HISTORY_AFTER_PREFLIGHT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static SEALED_HISTORY_AUTHORITY_WINDOW_HOOK:
        std::cell::RefCell<Option<Box<dyn FnMut(SealedHistoryAuthorityWindowStage)>>> =
        std::cell::RefCell::new(None);
    static ADVISORY_TRANSITION_CONTENTION_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static ENGINE_HISTORY_FAIL_BEFORE_HEAD_SWAP: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static ENGINE_HISTORY_FAIL_AFTER_HEAD_SWAP: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static DETACHED_BOOTSTRAP_FAIL_BEFORE_BATCH_FINISH: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static DETACHED_BOOTSTRAP_BATCH_FINISH_COUNT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum SealedHistoryAuthorityWindowStage {
    Locked,
    Validated,
}

#[cfg(test)]
pub(crate) fn fail_next_engine_history_head_swap() {
    ENGINE_HISTORY_FAIL_BEFORE_HEAD_SWAP.with(|fail| fail.set(true));
}

#[cfg(test)]
pub(crate) fn fail_next_engine_history_after_head_swap() {
    ENGINE_HISTORY_FAIL_AFTER_HEAD_SWAP.with(|fail| fail.set(true));
}

#[cfg(test)]
pub(crate) fn fail_next_detached_bootstrap_batch_finish() {
    DETACHED_BOOTSTRAP_FAIL_BEFORE_BATCH_FINISH.with(|fail| fail.set(true));
}

#[cfg(test)]
fn detached_bootstrap_batch_finish_hook() -> Result<(), StoreError> {
    DETACHED_BOOTSTRAP_FAIL_BEFORE_BATCH_FINISH.with(|fail| {
        if fail.replace(false) {
            Err(StoreError::Io(std::io::Error::other(
                "deterministic failure before detached bootstrap batch finish",
            )))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
fn note_detached_bootstrap_batch_finished() {
    DETACHED_BOOTSTRAP_BATCH_FINISH_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(not(test))]
fn note_detached_bootstrap_batch_finished() {}

#[cfg(not(test))]
fn detached_bootstrap_batch_finish_hook() -> Result<(), StoreError> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn fail_next_publish_after_objects() {
    fail_next_publish_after_objects_for_harness();
}

pub(crate) fn fail_next_publish_after_objects_for_harness() {
    HARNESS_PUBLISH_FAIL_AFTER_OBJECTS.with(|fail| fail.set(true));
}

fn publish_after_objects_hook() -> Result<(), StoreError> {
    HARNESS_PUBLISH_FAIL_AFTER_OBJECTS.with(|fail| {
        if fail.replace(false) {
            Err(StoreError::Io(std::io::Error::other(
                "deterministic failure after object publication",
            )))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
pub(crate) fn set_enrolled_open_use_hook(hook: impl FnOnce() + 'static) {
    ENROLLED_OPEN_USE_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
pub(crate) fn set_enrolled_open_act_hook(hook: impl FnOnce() + 'static) {
    ENROLLED_OPEN_ACT_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn set_sealed_history_after_preflight_hook(hook: impl FnOnce() + 'static) {
    SEALED_HISTORY_AFTER_PREFLIGHT_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn sealed_history_after_preflight_hook() {
    SEALED_HISTORY_AFTER_PREFLIGHT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn set_sealed_history_authority_window_hook(
    hook: impl FnMut(SealedHistoryAuthorityWindowStage) + 'static,
) {
    SEALED_HISTORY_AUTHORITY_WINDOW_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn sealed_history_authority_window_hook(stage: SealedHistoryAuthorityWindowStage) {
    SEALED_HISTORY_AUTHORITY_WINDOW_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(hook) = slot.as_mut() {
            hook(stage);
        }
        if matches!(stage, SealedHistoryAuthorityWindowStage::Validated) {
            slot.take();
        }
    });
}

#[cfg(test)]
fn set_advisory_transition_contention_hook(hook: impl FnOnce() + 'static) {
    ADVISORY_TRANSITION_CONTENTION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn enrolled_open_use_hook() {
    ENROLLED_OPEN_USE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn enrolled_open_use_hook() {}

#[cfg(test)]
fn enrolled_open_act_hook() {
    ENROLLED_OPEN_ACT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn enrolled_open_act_hook() {}

/// A caller-rooted, v2-candidate immutable object and batch-manifest store.
///
/// Opening this type is the only persistence trigger. It is intentionally not
/// connected to graph startup, enrollment, or the legacy managed-sync store.
#[derive(Debug)]
pub struct ObjectStore {
    root_path: PathBuf,
    workspace_id: WorkspaceId,
    capability: Dir,
    counters: Arc<StoreCounters>,
    lifecycle: ObjectStoreLifecycle,
}

/// Filesystem strength required by the caller that opened this archive.
///
/// An inactive import can be discarded and rebuilt from the unchanged graph;
/// an enrolled/promoted archive is recovery authority. Keeping that distinction
/// on the retained store prevents Android compatibility from depending on a
/// process-global helper policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObjectStoreLifecycle {
    Authority,
    ReconstructibleActivation,
}

/// One-shot enrolled-engine open token. Existing controls are exact retained
/// capabilities with authenticated heads pinned by the comprehensive
/// preflight; absent controls are rechecked before any layout is created.
pub(crate) struct EnrolledProjectionOpen {
    store: Option<ObjectStore>,
    binding: super::hot_engine::ProjectionStorageBinding,
    history: Option<SealedControl<DurableEngineHistoryStore>>,
    work: Option<SealedControl<super::ProjectionWorkIndex>>,
}

/// One-shot bootstrap installer token. It seals only durable history and never
/// opens or creates projection-work authority.
pub(crate) struct HistoryOnlyOpen {
    store: Option<ObjectStore>,
    binding: super::hot_engine::ProjectionStorageBinding,
    history: Option<SealedControl<DurableEngineHistoryStore>>,
}

enum SealedControl<T> {
    Existing(T),
    Absent(AbsentControlName),
}

struct AbsentControlName {
    namespace_name: &'static str,
    namespace: Option<Dir>,
    namespace_identity: Option<ControlDirectoryIdentity>,
    endpoint_name: String,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ControlDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ControlDirectoryIdentity {
    volume: u64,
    file_id: [u8; 16],
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ControlDirectoryIdentity;

impl ControlDirectoryIdentity {
    pub(crate) fn binding_digest(self) -> ContentDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/control-directory-identity-binding/v1\0");
        self.hash_platform_identity(&mut hasher);
        ContentDigest::from_bytes(hasher.finalize().into())
    }

    pub(crate) fn migration_backup_root_binding_digest(self) -> ContentDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/migration-backup-root-resource/v1\0");
        self.hash_platform_identity(&mut hasher);
        ContentDigest::from_bytes(hasher.finalize().into())
    }

    fn hash_platform_identity(self, hasher: &mut Sha256) {
        #[cfg(unix)]
        {
            hasher.update(b"unix-dev-inode\0");
            hasher.update(self.device.to_be_bytes());
            hasher.update(self.inode.to_be_bytes());
        }
        #[cfg(windows)]
        {
            hasher.update(b"windows-volume-file-id\0");
            hasher.update(self.volume.to_be_bytes());
            hasher.update(self.file_id);
        }
        #[cfg(not(any(unix, windows)))]
        {
            hasher.update(b"unsupported\0");
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AcceptedReadStats {
    pub manifest_reads: usize,
    pub object_reads: usize,
}

/// Process-wide `inspect_batch` cost, for the F49 quadratic probe only.
///
/// The per-store `counters` are reset with the store; these are not, so an
/// import's *total* re-read volume can be read once at the end of a run.
/// Diagnostic only — nothing reads these outside the probe.
pub static INSPECT_BATCH_CALLS: AtomicUsize = AtomicUsize::new(0);
/// `caller file:line -> (calls, required objects)`. `#[track_caller]` on
/// `inspect_batch` makes this exact without touching a single call site --
/// which matters, because guessing which callers dominate has already been
/// wrong once.
pub static INSPECT_BATCH_SITES: std::sync::Mutex<
    std::collections::BTreeMap<String, (usize, usize)>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());
pub static INSPECT_BATCH_OBJECT_READS: AtomicUsize = AtomicUsize::new(0);
pub static INSPECT_BATCH_OBJECT_BYTES: AtomicUsize = AtomicUsize::new(0);
pub static INSPECT_BATCH_DIGEST_BYTES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ObjectStoreStats {
    pub directory_enumerations: usize,
    pub accepted_manifest_reads: usize,
    pub accepted_object_reads: usize,
    pub dag_manifest_reads: usize,
    pub history_record_reads: usize,
    pub history_index_reads: usize,
    pub history_index_writes: usize,
    pub history_decodes: usize,
    pub block_claim_index_reads: usize,
    pub block_claim_index_writes: usize,
    pub block_claim_index_syncs: usize,
    pub inspected_manifest_operations: usize,
    pub inspected_manifest_bytes: usize,
    pub inspected_object_operations: usize,
    pub inspected_object_bytes: usize,
}

#[derive(Debug, Default)]
struct StoreCounters {
    directory_enumerations: AtomicUsize,
    accepted_manifest_reads: AtomicUsize,
    accepted_object_reads: AtomicUsize,
    dag_manifest_reads: AtomicUsize,
    history_record_reads: AtomicUsize,
    history_index_reads: AtomicUsize,
    history_index_writes: AtomicUsize,
    history_decodes: AtomicUsize,
    block_claim_index_reads: AtomicUsize,
    block_claim_index_writes: AtomicUsize,
    block_claim_index_syncs: AtomicUsize,
    inspected_manifest_operations: AtomicUsize,
    inspected_manifest_bytes: AtomicUsize,
    inspected_object_operations: AtomicUsize,
    inspected_object_bytes: AtomicUsize,
}

#[derive(Debug)]
pub(crate) struct EngineHistoryStore {
    capability: Dir,
    counters: Arc<StoreCounters>,
    /// Sticky, process-local evidence that this exact open has already observed
    /// a durable engine-history storage fault: an index node that is missing,
    /// oversized, stored under the wrong content address, undecodable,
    /// non-canonical or structurally invalid, or a durable publication that did
    /// not complete. It only ever moves from `false` to `true`.
    ///
    /// The latch owns one job: while it is set, the store's
    /// authenticated-transition memo is disarmed, so every proof is decided by
    /// the complete walk a fresh open would perform. It lives beside the index
    /// because that is where almost every fault is observed;
    /// [`DurableEngineHistoryStore::publish`] latches it for the rest.
    storage_fault: AtomicBool,
}

#[derive(Debug)]
pub(crate) struct DurableEngineHistoryStore {
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    control: Dir,
    /// The retained no-follow capability of the archive root this control
    /// directory lives in. It is the only thing that can prove a promoted
    /// runtime state names *this* physical archive, so it is retained here
    /// rather than re-derived from an ambient pathname by each caller.
    archive_root: Dir,
    roots: Dir,
    index: EngineHistoryStore,
    transition_lock: fs::File,
    transition: Mutex<()>,
    authoritative_head: Mutex<Option<ContentDigest>>,
    /// Set only by [`Self::authorize_promoted_lineage`], which is reachable
    /// only through [`ObjectStore::seal_promoted_projection`]. While it is
    /// `None`, a bootstrap-bound history stays read-only.
    promoted_lineage: Option<PromotedRuntimeStateV1>,
    /// Store-private, process-local memo of insertion-only transitions *this
    /// exact open* already authenticated. See
    /// [`Self::authenticate_current_history_extension`]; it is an accelerator
    /// for the walk, never an authority of its own, it never shortens the
    /// live-endpoint checks, and it is discarded permanently once
    /// [`EngineHistoryStore::storage_fault`] latches.
    authenticated_transitions: Mutex<Vec<AuthenticatedEngineHistoryTransition>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableEngineHistoryRoot {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    generation: u64,
    index_root: ContentDigest,
    latest_batch_id: Option<BatchId>,
    binding: DurableEngineHistoryBinding,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableEngineHistoryBinding {
    engine: EngineHistoryBinding,
    bootstrap: Option<BootstrapAggregateHistoryBindingV1>,
}

impl DurableEngineHistoryBinding {
    fn ordinary(engine: EngineHistoryBinding) -> Self {
        Self {
            engine,
            bootstrap: None,
        }
    }
}

/// Exact portable bootstrap authority retained by the schema-v8 durable
/// history root. The later hot-engine lane mirrors this value in each cold
/// record before calling `publish_many_exact`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapAggregateHistoryBindingV1 {
    publication_id: BootstrapPublicationIdV1,
    aggregate_digest: BootstrapAggregateDigestV1,
    part_count: u32,
    final_frontier: ArchiveLocalFrontierBindingV1,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapAggregateHistoryBindingWireV1 {
    publication_id: [u8; 32],
    aggregate_digest: [u8; 32],
    part_count: u32,
    final_frontier: Vec<u8>,
}

impl BootstrapAggregateHistoryBindingV1 {
    pub(crate) fn for_aggregate(
        aggregate: &BootstrapAggregateManifestV1,
    ) -> Result<Self, StoreError> {
        Self::new(
            aggregate.publication_id(),
            aggregate.aggregate_digest(),
            aggregate.parts().len() as u32,
            aggregate.final_frontier(),
        )
    }

    pub(crate) fn new(
        publication_id: BootstrapPublicationIdV1,
        aggregate_digest: BootstrapAggregateDigestV1,
        part_count: u32,
        final_frontier: ArchiveLocalFrontierBindingV1,
    ) -> Result<Self, StoreError> {
        if final_frontier.accepted_count() != part_count {
            return Err(StoreError::MalformedHistoryIndex);
        }
        Ok(Self {
            publication_id,
            aggregate_digest,
            part_count,
            final_frontier,
        })
    }

    pub(crate) const fn publication_id(self) -> BootstrapPublicationIdV1 {
        self.publication_id
    }

    pub(crate) const fn aggregate_digest(self) -> BootstrapAggregateDigestV1 {
        self.aggregate_digest
    }

    pub(crate) const fn part_count(self) -> u32 {
        self.part_count
    }

    pub(crate) const fn final_frontier(self) -> ArchiveLocalFrontierBindingV1 {
        self.final_frontier
    }
}

impl Serialize for BootstrapAggregateHistoryBindingV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        BootstrapAggregateHistoryBindingWireV1 {
            publication_id: *self.publication_id.as_bytes(),
            aggregate_digest: *self.aggregate_digest.as_bytes(),
            part_count: self.part_count,
            final_frontier: self.final_frontier.encode(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BootstrapAggregateHistoryBindingV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = BootstrapAggregateHistoryBindingWireV1::deserialize(deserializer)?;
        let frontier = ArchiveLocalFrontierBindingV1::decode(&wire.final_frontier)
            .map_err(serde::de::Error::custom)?;
        Self::new(
            BootstrapPublicationIdV1::from_bytes(wire.publication_id),
            BootstrapAggregateDigestV1::from_bytes(wire.aggregate_digest),
            wire.part_count,
            frontier,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// How a promoted runtime is authorized to extend one durable history.
///
/// Publishing an ordinary local batch onto a promoted history keeps the exact
/// bootstrap aggregate binding the inactive publication installed, so every
/// cold and every later record carries the identical binding. That homogeneous
/// lineage is what this mode names and authorizes: the promoted state never
/// reinterprets an inactive root as ordinary, and mixed record bindings remain
/// unrepresentable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum PromotedLineageModeV1 {
    BootstrapAnchoredHomogeneous,
}

/// The device-local durable promotion state for one enrolled endpoint.
///
/// This record is inert evidence, exactly like [`EngineHistoryBinding`]: it
/// grants nothing by itself. Only [`DurableEngineHistoryStore::authorize_promoted_lineage`]
/// — reached solely through [`ObjectStore::seal_promoted_projection`] — turns a
/// durable state that authenticates the live archive, history, bootstrap
/// aggregate, and enrollment identities into write authorization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromotedRuntimeStateV1 {
    pub(crate) schema_version: u32,
    pub(crate) lineage_mode: PromotedLineageModeV1,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) lineage_digest: LineageDigest,
    pub(crate) catalog_document_id: DocumentId,
    pub(crate) endpoint_id: super::ProjectionEndpointId,
    pub(crate) device_id: DeviceId,
    pub(crate) graph_resource_id: super::CanonicalGraphResourceId,
    pub(crate) receipt_store_id: super::ProjectionReceiptStoreId,
    /// The canonical archive resource claim `VerifiedLocal` enrolled.
    pub(crate) archive_resource_id: CanonicalArchiveResourceId,
    /// Binding digest of the physical archive control directory identity the
    /// inactive accepted authority observed.
    pub(crate) archive_control_binding: ContentDigest,
    /// Exact bootstrap aggregate/publication identity of the anchored lineage.
    pub(crate) bootstrap: BootstrapAggregateHistoryBindingV1,
    pub(crate) bootstrap_import_id: ImportId,
    /// The authenticated bootstrap history generation and radix index root that
    /// every later promoted history must descend from.
    pub(crate) anchor_history_generation: u64,
    pub(crate) anchor_history_index_root: ContentDigest,
    /// The accepted frontier the bootstrap published.
    pub(crate) anchor_acceptance_sequence: u64,
    pub(crate) anchor_accepted_frontier_state_digest: ContentDigest,
    /// The original `LocalActive` verification digest and its enrollment
    /// binding, so a promoted archive can never be adopted by another
    /// enrollment.
    pub(crate) enrollment_verification_digest: ContentDigest,
    pub(crate) enrollment_binding_digest: ContentDigest,
    /// The session that performed the one-time promotion.
    pub(crate) promotion_session_id: SessionId,
    /// Exact immutable shadow publication retained as the bootstrap projection
    /// fallback until individual pages acquire ordinary durable receipts.
    pub(crate) bootstrap_projection: PromotedBootstrapProjectionBindingV1,
}

impl PromotedRuntimeStateV1 {
    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != PROMOTED_RUNTIME_STATE_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedPromotedRuntimeSchema(
                self.schema_version,
            ));
        }
        self.bootstrap_projection.validate().map_err(|_| {
            StoreError::PromotedRuntimeStateMismatch(
                "bootstrap projection authority binding is invalid",
            )
        })?;
        let parts = u64::from(self.bootstrap.part_count());
        if self.bootstrap.final_frontier().accepted_count() != self.bootstrap.part_count() {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "bootstrap aggregate frontier does not cover its part count",
            ));
        }
        if self.anchor_history_generation != parts || self.anchor_acceptance_sequence != parts {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "anchor generation and acceptance sequence must equal the bootstrap part count",
            ));
        }
        if (self.bootstrap.part_count() == 0)
            != (self.anchor_history_index_root == EngineHistoryStore::empty_root())
        {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "anchor index root does not agree with the bootstrap part count",
            ));
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, StoreError> {
        self.validate()?;
        let bytes =
            postcard::to_allocvec(self).map_err(|_| StoreError::MalformedPromotedRuntimeState)?;
        if bytes.len() as u64 > MAX_PROMOTED_RUNTIME_STATE_BYTES {
            return Err(StoreError::MalformedPromotedRuntimeState);
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        let state = postcard::from_bytes::<Self>(bytes)
            .map_err(|_| StoreError::MalformedPromotedRuntimeState)?;
        state.validate()?;
        // Reject any residue that decodes but is not the exact canonical
        // encoding of what it decoded to.
        if postcard::to_allocvec(&state).map_err(|_| StoreError::MalformedPromotedRuntimeState)?
            != bytes
        {
            return Err(StoreError::MalformedPromotedRuntimeState);
        }
        Ok(state)
    }

    /// Digest of this state's exact canonical encoding.
    ///
    /// One field a resume point can carry instead of restating thirteen
    /// identities that could drift apart. Because the state itself is only ever
    /// read through [`DurableEngineHistoryStore::require_promoted_state_binding`],
    /// matching this digest transitively binds the endpoint, device, graph
    /// resource, receipt store, archive resource claim, physical archive control
    /// identity, lineage, catalog document, bootstrap aggregate and import
    /// identity, the bootstrap anchor authority, the enrollment
    /// verification/binding digests, and the promotion session.
    pub(crate) fn state_digest(&self) -> Result<ContentDigest, StoreError> {
        Ok(ContentDigest::of(&self.encode()?))
    }

    pub(crate) const fn bootstrap(&self) -> BootstrapAggregateHistoryBindingV1 {
        self.bootstrap
    }

    /// The authenticated bootstrap anchor every promoted history transition is
    /// proved from.
    pub(crate) const fn anchor_authority(&self) -> EngineHistoryAuthority {
        EngineHistoryAuthority {
            generation: self.anchor_history_generation,
            index_root: self.anchor_history_index_root,
        }
    }
}

/// Bounded inert evidence from an existing promoted archive.
///
/// It carries no archive capability, store, history handle, transition lock,
/// lease, or publication method. A later runtime open must independently
/// reopen and authenticate all archive state before acquiring writer authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArchiveDiscoveryEvidence {
    pub(crate) bootstrap_import_id: ImportId,
    pub(crate) anchor_history_generation: u64,
    pub(crate) anchor_history_index_root: ContentDigest,
    pub(crate) anchor_acceptance_sequence: u64,
    pub(crate) anchor_accepted_frontier_state_digest: ContentDigest,
    pub(crate) enrollment_verification_digest: ContentDigest,
    pub(crate) promotion_session_id: SessionId,
    pub(crate) state_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArchiveDiscoveryInspection {
    Absent,
    Residue,
    Present(ArchiveDiscoveryEvidence),
}

/// Inspect one explicit existing archive root without constructing an
/// [`ObjectStore`] or any writer/runtime authority.
///
/// With no expected enrollment binding this is intentionally only a presence
/// probe, used to distinguish true absence from unexplained archive residue.
/// With a binding it opens the exact existing engine-history control no-follow,
/// validates its canonical claim and live root, strictly decodes the promoted
/// state, and checks the graph/archive/resource/control identities.
pub(crate) fn inspect_existing_archive_at(
    archive_root: &Path,
    expected_binding: Option<&EnrollmentBindingV1>,
) -> Result<ArchiveDiscoveryInspection, StoreError> {
    let Some(archive) = open_existing_archive_root_nofollow(archive_root)? else {
        return Ok(ArchiveDiscoveryInspection::Absent);
    };
    let Some(binding) = expected_binding else {
        return Ok(ArchiveDiscoveryInspection::Residue);
    };

    CanonicalArchiveResourceId::open_enrolled_in_retained_directory(
        &archive,
        binding.archive_resource_id(),
    )
    .map_err(|_| {
        StoreError::PromotedRuntimeStateMismatch(
            "archive resource claim does not authenticate the enrollment binding",
        )
    })?;
    for name in [OBJECTS_DIR, BATCHES_DIR] {
        open_existing_dir_nofollow(&archive, name)?.ok_or(StoreError::MalformedHistoryIndex)?;
    }
    let lineage = read_optional_regular(&archive, LINEAGE_CLAIM_FILE, 32, Some(32))?
        .ok_or(StoreError::MalformedHistoryIndex)?;
    require_lineage_bytes(binding.lineage_digest(), &lineage)?;

    let Some(histories) = open_existing_dir_nofollow(&archive, ENGINE_HISTORY_DIR)? else {
        return Ok(ArchiveDiscoveryInspection::Residue);
    };
    let endpoint_name = binding.endpoint_id().to_string();
    let Some(control) = open_existing_dir_nofollow(&histories, &endpoint_name)? else {
        return Ok(ArchiveDiscoveryInspection::Residue);
    };
    let head = read_optional_regular(&control, ENGINE_HISTORY_HEAD_FILE, 64, None)?;
    let claim = read_optional_regular(&control, ENGINE_HISTORY_CLAIM_FILE, 256, None)?;
    let (head, claim) = match (head, claim) {
        (None, None) => return Ok(ArchiveDiscoveryInspection::Residue),
        (Some(head), Some(claim)) => (head, claim),
        _ => return Err(StoreError::MalformedHistoryIndex),
    };
    validate_engine_history_claim(
        &claim,
        binding.workspace_id(),
        binding.endpoint_id(),
        binding.graph_resource_id(),
        binding.receipt_store_id(),
    )?;
    open_existing_dir_nofollow(&control, ENGINE_HISTORY_NODES_DIR)?
        .ok_or(StoreError::MalformedHistoryIndex)?;
    let roots = open_existing_dir_nofollow(&control, ENGINE_HISTORY_ROOTS_DIR)?
        .ok_or(StoreError::MalformedHistoryIndex)?;
    let head_text = std::str::from_utf8(&head).map_err(|_| StoreError::MalformedHistoryIndex)?;
    let head_digest = parse_digest(head_text)
        .map(ContentDigest::from_bytes)
        .map_err(|_| StoreError::MalformedHistoryIndex)?;
    if head_digest.to_string().as_bytes() != head {
        return Err(StoreError::MalformedHistoryIndex);
    }
    let root_bytes = read_optional_regular(
        &roots,
        &engine_history_root_filename(head_digest),
        MAX_ENGINE_HISTORY_INDEX_BYTES,
        None,
    )?
    .ok_or(StoreError::MalformedHistoryIndex)?;
    if ContentDigest::of(&root_bytes) != head_digest {
        return Err(StoreError::HistoryIndexPathMismatch(head_digest));
    }
    let root: DurableEngineHistoryRoot =
        postcard::from_bytes(&root_bytes).map_err(|_| StoreError::MalformedHistoryIndex)?;
    if postcard::to_allocvec(&root).map_err(|_| StoreError::MalformedHistoryIndex)? != root_bytes {
        return Err(StoreError::MalformedHistoryIndex);
    }
    validate_engine_history_root(
        &root,
        binding.workspace_id(),
        binding.endpoint_id(),
        binding.graph_resource_id(),
        binding.receipt_store_id(),
    )?;

    let Some(state_bytes) = read_optional_regular(
        &control,
        PROMOTED_RUNTIME_STATE_FILE,
        MAX_PROMOTED_RUNTIME_STATE_BYTES,
        None,
    )?
    else {
        return Ok(ArchiveDiscoveryInspection::Residue);
    };
    let state = PromotedRuntimeStateV1::decode(&state_bytes)?;
    let expected_binding_digest = binding
        .binding_digest()
        .map_err(|_| StoreError::MalformedPromotedRuntimeState)?;
    if state.workspace_id != binding.workspace_id()
        || state.lineage_digest != binding.lineage_digest()
        || state.catalog_document_id != binding.catalog_document_id()
        || state.endpoint_id != binding.endpoint_id()
        || state.device_id != binding.device_id()
        || state.graph_resource_id != binding.graph_resource_id()
        || state.receipt_store_id != binding.receipt_store_id()
        || state.archive_resource_id != binding.archive_resource_id()
        || state.enrollment_binding_digest != expected_binding_digest
    {
        return Err(StoreError::PromotedRuntimeStateMismatch(
            "promoted runtime state is bound to another enrollment",
        ));
    }
    if state.archive_control_binding != control_directory_identity(&archive)?.binding_digest() {
        return Err(StoreError::PromotedRuntimeStateMismatch(
            "promoted runtime state is bound to another physical archive directory",
        ));
    }
    if root.binding.bootstrap != Some(state.bootstrap) {
        return Err(StoreError::PromotedRuntimeStateMismatch(
            "durable history bootstrap binding is not the promoted lineage",
        ));
    }
    Ok(ArchiveDiscoveryInspection::Present(
        ArchiveDiscoveryEvidence {
            bootstrap_import_id: state.bootstrap_import_id,
            anchor_history_generation: state.anchor_history_generation,
            anchor_history_index_root: state.anchor_history_index_root,
            anchor_acceptance_sequence: state.anchor_acceptance_sequence,
            anchor_accepted_frontier_state_digest: state.anchor_accepted_frontier_state_digest,
            enrollment_verification_digest: state.enrollment_verification_digest,
            promotion_session_id: state.promotion_session_id,
            state_digest: state.state_digest()?,
        },
    ))
}

fn open_existing_archive_root_nofollow(root: &Path) -> Result<Option<Dir>, StoreError> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::UnsafeEntry(
            "archive root is not a real no-follow directory".into(),
        ));
    }
    let name = root
        .file_name()
        .ok_or_else(|| StoreError::UnsafeEntry("archive root has no final component".into()))?;
    if !matches!(root.components().next_back(), Some(Component::Normal(_))) {
        return Err(StoreError::UnsafeEntry(
            "archive root must end in a normal path component".into(),
        ));
    }
    let name = name.to_str().ok_or_else(|| {
        StoreError::UnsafeEntry("archive root final component is not UTF-8".into())
    })?;
    let parent = root.parent().ok_or_else(|| {
        StoreError::UnsafeEntry("archive root must have an existing parent".into())
    })?;
    let canonical_parent = fs::canonicalize(parent)?;
    let parent = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
    open_existing_dir_nofollow(&parent, name)
}

/// Publish the smallest valid zero-part promoted archive used by discovery
/// tests. Production callers cannot reach this writer seam.
#[cfg(test)]
pub(crate) fn create_discovery_promoted_archive_for_test(
    archive_root: &Path,
    binding: &EnrollmentBindingV1,
    active: &super::enrollment::EnrollmentDiscoveryLocalActive,
    promotion_session_id: SessionId,
) -> Result<ContentDigest, StoreError> {
    let store = ObjectStore::open(archive_root, binding.workspace_id())?;
    store
        .validate_enrolled_archive_resource_id(binding.archive_resource_id())
        .map_err(StoreError::Io)?;
    let bootstrap_import_id = ImportId::from_digest(*active.bootstrap_import_id.as_bytes());
    let aggregate = BootstrapAggregateManifestV1::empty(
        binding.workspace_id(),
        binding.lineage_digest(),
        binding.graph_resource_id(),
        bootstrap_import_id,
    )
    .map_err(|error| StoreError::Bootstrap(error.to_string()))?;
    store.publish_bootstrap_aggregate_prefix(&aggregate)?;
    let publication_id = store.commit_bootstrap_aggregate(&aggregate)?;
    let publication = store.load_bootstrap_publication(publication_id)?;
    let history_binding = super::hot_engine::ProjectionStorageBinding {
        endpoint: super::ProjectionEndpointBinding {
            endpoint_id: binding.endpoint_id(),
            device_id: binding.device_id(),
            graph_resource_id: binding.graph_resource_id(),
        },
        receipt_store_id: binding.receipt_store_id(),
    };
    let history = store.open_engine_history(history_binding)?;
    history.publish_many_exact(&[], &publication, EngineHistoryBinding::empty())?;
    let state = PromotedRuntimeStateV1 {
        schema_version: PROMOTED_RUNTIME_STATE_SCHEMA_VERSION,
        lineage_mode: PromotedLineageModeV1::BootstrapAnchoredHomogeneous,
        workspace_id: binding.workspace_id(),
        lineage_digest: binding.lineage_digest(),
        catalog_document_id: binding.catalog_document_id(),
        endpoint_id: binding.endpoint_id(),
        device_id: binding.device_id(),
        graph_resource_id: binding.graph_resource_id(),
        receipt_store_id: binding.receipt_store_id(),
        archive_resource_id: binding.archive_resource_id(),
        archive_control_binding: control_directory_identity(&store.capability)?.binding_digest(),
        bootstrap: BootstrapAggregateHistoryBindingV1::for_aggregate(&aggregate)?,
        bootstrap_projection: PromotedBootstrapProjectionBindingV1::synthetic_for_object_store_test(
            binding.workspace_id(),
            binding.lineage_digest(),
            binding.endpoint_id(),
            binding.device_id(),
            binding.graph_resource_id(),
            binding.receipt_store_id(),
            control_directory_identity(&store.capability)?.binding_digest(),
            ContentDigest::from_bytes(*aggregate.publication_id().as_bytes()),
            ContentDigest::from_bytes(*aggregate.aggregate_digest().as_bytes()),
            active.bootstrap_import_id,
            aggregate.parts().len() as u32,
            active.anchor_accepted_frontier_state_digest,
            active.anchor_history_generation,
            active.anchor_history_index_root,
        ),
        bootstrap_import_id,
        anchor_history_generation: active.anchor_history_generation,
        anchor_history_index_root: active.anchor_history_index_root,
        anchor_acceptance_sequence: active.anchor_acceptance_sequence,
        anchor_accepted_frontier_state_digest: active.anchor_accepted_frontier_state_digest,
        enrollment_verification_digest: active.verification_digest,
        enrollment_binding_digest: binding
            .binding_digest()
            .map_err(|_| StoreError::MalformedPromotedRuntimeState)?,
        promotion_session_id,
    };
    history.publish_promoted_runtime_state(&state)?;
    state.state_digest()
}

#[cfg(test)]
mod discovery_inspector_tests {
    use super::*;

    #[test]
    fn explicit_archive_presence_probe_creates_nothing() {
        let parent =
            std::env::temp_dir().join(format!("tine-archive-discovery-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&parent).unwrap();
        let archive = parent.join("archive");
        assert_eq!(
            inspect_existing_archive_at(&archive, None).unwrap(),
            ArchiveDiscoveryInspection::Absent
        );
        assert!(!archive.exists());

        std::fs::create_dir(&archive).unwrap();
        assert_eq!(
            inspect_existing_archive_at(&archive, None).unwrap(),
            ArchiveDiscoveryInspection::Residue
        );
        assert!(std::fs::read_dir(&archive).unwrap().next().is_none());
        crate::test_support::remove_dir_all(parent);
    }
}

pub(crate) struct PreparedBootstrapHistoryRecordV1<'a> {
    part: BootstrapPartDescriptorV1,
    bytes: &'a [u8],
    binding: BootstrapAggregateHistoryBindingV1,
    engine_binding: EngineHistoryBinding,
}

impl<'a> PreparedBootstrapHistoryRecordV1<'a> {
    pub(crate) fn new(
        part: BootstrapPartDescriptorV1,
        bytes: &'a [u8],
        binding: BootstrapAggregateHistoryBindingV1,
    ) -> Result<Self, StoreError> {
        let engine_binding =
            super::hot_engine::validate_bootstrap_history_record(part, bytes, binding)
                .map_err(|error| StoreError::Bootstrap(error.to_string()))?;
        Ok(Self {
            part,
            bytes,
            binding,
            engine_binding,
        })
    }

    #[cfg(test)]
    fn unchecked_for_history_index_test(
        part: BootstrapPartDescriptorV1,
        bytes: &'a [u8],
        binding: BootstrapAggregateHistoryBindingV1,
    ) -> Self {
        Self {
            part,
            bytes,
            binding,
            engine_binding: EngineHistoryBinding::empty(),
        }
    }
}

pub(crate) struct ExactBootstrapHistoryBuilderV1<'a> {
    store: &'a DurableEngineHistoryStore,
    expected_parts: &'a [BootstrapPartDescriptorV1],
    binding: BootstrapAggregateHistoryBindingV1,
    engine_binding: EngineHistoryBinding,
    index_root: ContentDigest,
    latest: Option<BatchId>,
    next_ordinal: usize,
    batch_ids: std::collections::BTreeSet<BatchId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EngineHistoryAuthority {
    pub generation: u64,
    pub index_root: ContentDigest,
}

/// Opaque proof that one authenticated durable history is either exact or an
/// insertion-only prefix of another. Only the history store can mint this
/// witness; projection authority must not move between raw generation/root
/// pairs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedEngineHistoryTransition {
    before: EngineHistoryAuthority,
    after: EngineHistoryAuthority,
}

impl AuthenticatedEngineHistoryTransition {
    pub(crate) const fn before(self) -> EngineHistoryAuthority {
        self.before
    }

    pub(crate) const fn after(self) -> EngineHistoryAuthority {
        self.after
    }

    #[cfg(test)]
    pub(crate) const fn for_test(
        before: EngineHistoryAuthority,
        after: EngineHistoryAuthority,
    ) -> Self {
        Self { before, after }
    }
}

/// How many distinct anchors one open memoizes transitions for. A promoted
/// runtime revalidates from exactly one immutable bootstrap anchor, so this
/// only needs headroom for an incidental second caller; the memo must stay a
/// couple of pointer-sized pairs, not a history cache.
const MAX_AUTHENTICATED_TRANSITION_ANCHORS: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineHistoryBinding {
    pub portable_path_key_version: u32,
    pub portable_path_root: ContentDigest,
    pub catalog_checkpoint_binding: ContentDigest,
    pub portable_path_conflicts: Vec<super::PortablePathConflict>,
    pub terminal_evidence: Option<EngineTerminalEvidenceBinding>,
    pub page_names: PageNameDurableBinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineTerminalEvidenceBinding {
    pub conflict_root: ContentDigest,
    pub conflict_count: u64,
    pub participant_count: u64,
    pub canonical_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageNameDurableBinding {
    pub ownership_root: super::page_name_index::PageNameOwnershipRootV1,
    pub conflicts: Vec<super::page_name_index::PageNameConflictEvidenceV1>,
}

impl PageNameDurableBinding {
    pub(crate) fn empty() -> Self {
        Self {
            ownership_root: super::page_name_index::PageNameOwnershipRootV1::empty(),
            conflicts: Vec::new(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        self.ownership_root.encode()?;
        let digests = self
            .conflicts
            .iter()
            .map(super::page_name_index::PageNameConflictEvidenceV1::digest)
            .collect::<Result<Vec<_>, _>>()?;
        if digests.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(StoreError::MalformedHistoryIndex);
        }
        Ok(())
    }
}

impl EngineHistoryBinding {
    pub(crate) fn empty() -> Self {
        Self {
            portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
            portable_path_root: super::PortablePathIndexRoot::empty().digest(),
            catalog_checkpoint_binding: ContentDigest::of(
                b"tine/empty-catalog-checkpoint-binding/v1",
            ),
            portable_path_conflicts: Vec::new(),
            terminal_evidence: None,
            page_names: PageNameDurableBinding::empty(),
        }
    }

    /// Compare the replay-stable typed authority. The catalog checkpoint is
    /// intentionally omitted because it embeds fresh scratch-run page
    /// references; authenticated recovery applies the same rule while exact
    /// historical record bytes continue to protect the retained checkpoint.
    pub(crate) fn same_replay_authority(&self, other: &Self) -> bool {
        self.portable_path_key_version == other.portable_path_key_version
            && self.portable_path_root == other.portable_path_root
            && self.portable_path_conflicts == other.portable_path_conflicts
            && self.terminal_evidence == other.terminal_evidence
            && self.page_names == other.page_names
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct BlockClaimIndexRoot {
    next_generation: u64,
    global_filter: Option<BlockClaimPageRef>,
    levels:
        [[Option<BlockClaimSegmentRef>; BLOCK_CLAIM_SEGMENTS_PER_LEVEL]; BLOCK_CLAIM_INDEX_LEVELS],
}

/// Test-only saturation of the block-claim root, for the resume-point byte
/// ceiling proof.
///
/// Every member here is fixed-size — `BlockClaimPageRef` carries no key span —
/// so the whole root's width is decided by the two fixed array dimensions and
/// the widest encodable field values.
#[cfg(test)]
impl BlockClaimIndexRoot {
    pub(crate) fn saturated_for_test() -> Self {
        let page_ref = BlockClaimPageRef {
            offset: u64::MAX,
            encoded_len: u32::MAX,
            digest: ContentDigest::of(b"saturated block claim page"),
        };
        Self {
            next_generation: u64::MAX,
            global_filter: Some(page_ref),
            levels: [[Some(BlockClaimSegmentRef {
                generation: u64::MAX,
                entry_count: u64::MAX,
                page_ref,
                filter_ref: page_ref,
            }); BLOCK_CLAIM_SEGMENTS_PER_LEVEL]; BLOCK_CLAIM_INDEX_LEVELS],
        }
    }
}

#[derive(Debug)]
pub(crate) struct BlockClaimIndexStore {
    backing: BlockClaimIndexBacking,
    counters: Arc<StoreCounters>,
}

#[derive(Debug)]
enum BlockClaimIndexBacking {
    Scratch(Arc<super::scratch_store::ScratchStore>),
    #[cfg(test)]
    Standalone(Mutex<fs::File>),
}

impl BlockClaimIndexStore {
    /// A run-local block-claim point index over a caller-owned scratch store.
    ///
    /// The block-claim root is reconstructible run-local derived state — no
    /// accepted cold record binds it — so it belongs in whichever scratch run
    /// owns the engine. Detached bootstrap authoring owns its own disposable
    /// scratch run rather than the archive's, and builds its point index the
    /// same way an enrolled engine does instead of falling back to the bounded
    /// in-memory test map, whose fixed capacity would otherwise cap an
    /// importable graph at a few thousand blocks.
    pub(crate) fn for_scratch(
        scratch: Arc<super::scratch_store::ScratchStore>,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            backing: BlockClaimIndexBacking::Scratch(scratch),
            counters: Arc::new(StoreCounters::default()),
        })
    }
}

/// One run-local engine scratch pair over a **retained** scratch run.
///
/// This type is the retention capability. It is minted only by
/// [`ObjectStore::create_retained_engine_scratch`] and
/// [`ObjectStore::adopt_retained_engine_scratch`], has no public constructor,
/// no `Default`, and no `Clone`, so an engine that holds one can treat "this
/// run survives my death and may be named by a durable resume point" as a
/// structural fact rather than a re-read marker byte.
pub(crate) struct RetainedEngineScratch {
    scratch: Arc<super::scratch_store::ScratchStore>,
    claim_index: BlockClaimIndexStore,
    run_id: Uuid,
    binding_digest: ContentDigest,
}

impl fmt::Debug for RetainedEngineScratch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedEngineScratch")
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

impl RetainedEngineScratch {
    fn seal(
        store: &ObjectStore,
        scratch: super::scratch_store::ScratchStore,
    ) -> Result<Self, StoreError> {
        let scratch = Arc::new(scratch);
        let claim_index = store.engine_claim_index(Arc::clone(&scratch))?;
        let run_id = scratch.run_id();
        let binding_digest = scratch
            .binding_digest()
            .map_err(|error| StoreError::Scratch(error.to_string()))?;
        Ok(Self {
            scratch,
            claim_index,
            run_id,
            binding_digest,
        })
    }

    pub(crate) const fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub(crate) const fn binding_digest(&self) -> ContentDigest {
        self.binding_digest
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Arc<super::scratch_store::ScratchStore>,
        BlockClaimIndexStore,
        RetainedScratchIdentity,
    ) {
        let identity = RetainedScratchIdentity {
            run_id: self.run_id,
            binding_digest: self.binding_digest,
        };
        (self.scratch, self.claim_index, identity)
    }
}

/// The durable identity of the retained run an engine is running on.
///
/// Carried by the engine so a later quiescent snapshot can name its own run
/// without re-deriving retention, and so observability can report which run a
/// restart adopted or refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetainedScratchIdentity {
    run_id: Uuid,
    binding_digest: ContentDigest,
}

impl RetainedScratchIdentity {
    pub(crate) const fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub(crate) const fn binding_digest(&self) -> ContentDigest {
        self.binding_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct BlockClaimIndexValue(SmallVec<[u8; 64]>);

impl BlockClaimIndexValue {
    pub(crate) fn from_slice(bytes: &[u8]) -> Self {
        Self(SmallVec::from_slice(bytes))
    }

    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self(SmallVec::from_vec(bytes))
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.0
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BlockClaimPageRef {
    offset: u64,
    encoded_len: u32,
    digest: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BlockClaimSegmentRef {
    generation: u64,
    entry_count: u64,
    page_ref: BlockClaimPageRef,
    filter_ref: BlockClaimPageRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BlockClaimFilterPage {
    schema_version: u32,
    entry_count: u64,
    bit_len: u64,
    bits: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BlockClaimGlobalFilterPage {
    schema_version: u32,
    insertions: u64,
    bits: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum BlockClaimIndexPage {
    Branch {
        schema_version: u32,
        depth: u8,
        children: Vec<(u8, BlockClaimPageRef)>,
    },
    Leaf {
        schema_version: u32,
        depth: u8,
        entries: Vec<([u8; 16], BlockClaimIndexValue)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum HistoryIndexNode {
    Branch {
        schema_version: u32,
        depth: u8,
        children: Vec<(u8, ContentDigest)>,
    },
    Leaf {
        schema_version: u32,
        batch_id: BatchId,
        record: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchInspection {
    /// No manifest commit marker exists. Object-only residue remains invisible.
    Absent,
    /// The manifest is valid, but these canonical descriptors are not present.
    Staged {
        manifest: OperationBatch,
        missing: Vec<ObjectDescriptor>,
    },
    /// The manifest and its exact closed object set have been validated.
    Ready(ValidatedBatch),
}

#[derive(Debug)]
pub(crate) enum BootstrapPublicationInspectionV1 {
    Absent,
    Pending,
    Committed(ValidatedBootstrapPublicationV1),
    CorruptOrConflicting(StoreError),
}

#[derive(Clone, Debug)]
pub(crate) struct ValidatedBootstrapPublicationV1 {
    aggregate: BootstrapAggregateManifestV1,
}

impl ValidatedBootstrapPublicationV1 {
    pub(crate) fn aggregate(&self) -> &BootstrapAggregateManifestV1 {
        &self.aggregate
    }
}

enum DetachedBootstrapPublicationState {
    Open(tine_storage::ExactImmutablePublicationBatch),
    Poisoned,
    Finished,
}

struct DetachedBootstrapPublicationShared {
    state: Mutex<DetachedBootstrapPublicationState>,
}

/// Cloneable write-only handle shared by the authenticated index stores of one
/// detached bootstrap authoring session. Once the owning session finishes or
/// any publication fails, later writes fail closed.
#[derive(Clone)]
pub(crate) struct DetachedBootstrapImmutablePublisher {
    shared: Arc<DetachedBootstrapPublicationShared>,
}

impl fmt::Debug for DetachedBootstrapImmutablePublisher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DetachedBootstrapImmutablePublisher")
            .finish_non_exhaustive()
    }
}

impl DetachedBootstrapImmutablePublisher {
    pub(crate) fn publish(
        &self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
        kind: &'static str,
    ) -> Result<(), StoreError> {
        let mut state = self.shared.state.lock().map_err(|_| {
            StoreError::Bootstrap(
                "detached bootstrap immutable publication batch mutex is poisoned".into(),
            )
        })?;
        let result = match &mut *state {
            DetachedBootstrapPublicationState::Open(batch) => batch.publish(dir, filename, bytes),
            DetachedBootstrapPublicationState::Poisoned => {
                return Err(StoreError::Bootstrap(
                    "detached bootstrap immutable publication batch is poisoned".into(),
                ));
            }
            DetachedBootstrapPublicationState::Finished => {
                return Err(StoreError::Bootstrap(
                    "detached bootstrap immutable publication batch is closed".into(),
                ));
            }
        };
        if let Err(error) = result {
            *state = DetachedBootstrapPublicationState::Poisoned;
            return Err(publication_error(error, Collision::Exact(kind)));
        }
        Ok(())
    }
}

/// Unique completion authority for one archive-bound detached authoring batch.
/// Store writers receive only cloneable publication handles, never this token.
pub(crate) struct DetachedBootstrapPublicationSession {
    publisher: DetachedBootstrapImmutablePublisher,
    workspace_id: WorkspaceId,
    archive_identity: ControlDirectoryIdentity,
}

/// Non-serializable evidence that every immutable authenticated-index object
/// authored by one detached session is beneath its archive durability barrier.
pub(crate) struct CompletedDetachedBootstrapPublication {
    physical: tine_storage::CompletedExactImmutablePublicationBatch,
    packed_constructions: Option<[super::content_patricia::CompletedPatriciaConstruction; 4]>,
    workspace_id: WorkspaceId,
    archive_identity: ControlDirectoryIdentity,
}

impl CompletedDetachedBootstrapPublication {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn archive_identity(&self) -> ControlDirectoryIdentity {
        self.archive_identity
    }

    #[cfg(test)]
    pub(crate) const fn publication_count(&self) -> usize {
        self.physical.publication_count()
    }

    #[cfg(test)]
    pub(crate) const fn existing_publication_count(&self) -> usize {
        self.physical.existing_publication_count()
    }

    #[cfg(test)]
    pub(crate) fn packed_construction_stats(
        &self,
    ) -> Option<[tine_storage::PatriciaIndexConstructionStats; 4]> {
        self.packed_constructions.as_ref().map(|constructions| {
            constructions
                .each_ref()
                .map(super::content_patricia::CompletedPatriciaConstruction::stats)
        })
    }
}

impl DetachedBootstrapPublicationSession {
    fn new(
        archive: &Dir,
        workspace_id: WorkspaceId,
        archive_identity: ControlDirectoryIdentity,
    ) -> Result<Self, StoreError> {
        let physical = tine_storage::ExactImmutablePublicationBatch::new(archive)
            .map_err(filesystem_error_without_collision)?;
        Ok(Self {
            publisher: DetachedBootstrapImmutablePublisher {
                shared: Arc::new(DetachedBootstrapPublicationShared {
                    state: Mutex::new(DetachedBootstrapPublicationState::Open(physical)),
                }),
            },
            workspace_id,
            archive_identity,
        })
    }

    fn publisher(&self) -> DetachedBootstrapImmutablePublisher {
        self.publisher.clone()
    }

    pub(crate) fn finish(
        self,
        packed_constructions: [super::content_patricia::CompletedPatriciaConstruction; 4],
    ) -> Result<CompletedDetachedBootstrapPublication, StoreError> {
        self.finish_inner(Some(packed_constructions))
    }

    #[cfg(test)]
    fn finish_without_patricia_for_test(
        self,
    ) -> Result<CompletedDetachedBootstrapPublication, StoreError> {
        self.finish_inner(None)
    }

    fn finish_inner(
        self,
        packed_constructions: Option<[super::content_patricia::CompletedPatriciaConstruction; 4]>,
    ) -> Result<CompletedDetachedBootstrapPublication, StoreError> {
        let mut state = self.publisher.shared.state.lock().map_err(|_| {
            StoreError::Bootstrap(
                "detached bootstrap immutable publication batch mutex is poisoned".into(),
            )
        })?;
        let batch =
            match std::mem::replace(&mut *state, DetachedBootstrapPublicationState::Poisoned) {
                DetachedBootstrapPublicationState::Open(batch) => batch,
                DetachedBootstrapPublicationState::Poisoned => {
                    return Err(StoreError::Bootstrap(
                        "detached bootstrap immutable publication batch is poisoned".into(),
                    ));
                }
                DetachedBootstrapPublicationState::Finished => {
                    return Err(StoreError::Bootstrap(
                        "detached bootstrap immutable publication batch is already finished".into(),
                    ));
                }
            };
        detached_bootstrap_batch_finish_hook()?;
        let physical = batch.finish().map_err(filesystem_error_without_collision)?;
        note_detached_bootstrap_batch_finished();
        *state = DetachedBootstrapPublicationState::Finished;
        Ok(CompletedDetachedBootstrapPublication {
            physical,
            packed_constructions,
            workspace_id: self.workspace_id,
            archive_identity: self.archive_identity,
        })
    }
}

/// Uncommitted bootstrap-only immutable publication. Files are inserted under
/// their final content-addressed names without individual barriers; `finish`
/// authenticates the closed set and flushes the filesystem once. The ordinary
/// object publication path remains unchanged.
pub(crate) struct BootstrapPublicationBatch<'a> {
    store: &'a ObjectStore,
    physical: tine_storage::ExactImmutablePublicationBatch,
    inventory_root: Option<SourceInventoryRootV1>,
    inventory_pages: BTreeMap<u32, ()>,
    blob_root: Option<SourceBlobChunkRootV1>,
    blob_pages: BTreeMap<u32, ()>,
    part_packs: BTreeMap<super::identity::BootstrapPartId, ()>,
    parts: BTreeMap<super::identity::BootstrapPartId, ()>,
}

/// Non-serializable proof that every prefix byte named by one aggregate was
/// flushed before its commit-last marker can be published.
pub(crate) struct DurablyStagedBootstrapPrefix {
    workspace_id: WorkspaceId,
    archive_identity: ControlDirectoryIdentity,
    aggregate_digest: BootstrapAggregateDigestV1,
}

#[derive(Debug)]
pub(crate) struct LoadedBootstrapPartV1 {
    manifest: OperationBatch,
    objects: Vec<OperationObject>,
    spans: BootstrapPartSpanIndexV1,
}

impl LoadedBootstrapPartV1 {
    pub(crate) fn manifest(&self) -> &OperationBatch {
        &self.manifest
    }

    pub(crate) fn objects(&self) -> &[OperationObject] {
        &self.objects
    }

    pub(crate) fn spans(&self) -> &BootstrapPartSpanIndexV1 {
        &self.spans
    }

    /// Consume this loaded part into exactly its manifest and object payload.
    ///
    /// Recovery stages one bootstrap part at a time, so it must be able to move
    /// the payload into the prepared batch instead of cloning it beside the
    /// still-live loaded part.
    pub(crate) fn into_manifest_and_objects(self) -> (OperationBatch, Vec<OperationObject>) {
        (self.manifest, self.objects)
    }
}

/// The durable authenticated index capabilities of one exact archive, for
/// detached bootstrap authoring and replay.
///
/// Every accepted bootstrap cold record binds four authenticated roots — the
/// portable-path root, the page-name ownership root, the external UUID-claim
/// root, and the reference-catalog root. Each has exactly one construction:
/// the archive's durable content-addressed Patricia stores. A detached session that
/// used the run-local ephemeral backends instead would bind roots the promoted
/// runtime's durable stores can never open, so authoring takes this capability
/// over the archive the bootstrap is installed into and promoted from.
///
/// It carries nothing else: no object, manifest, engine-history,
/// projection-work, enrollment, scratch, or graph authority. Everything it
/// publishes is immutable and content-addressed, so a discarded preparation
/// leaves only unreachable nodes, never mutable archive state.
#[derive(Clone, Debug)]
pub(crate) struct BootstrapAuthoringCapability {
    workspace_id: WorkspaceId,
    archive_identity: ControlDirectoryIdentity,
    archive: Arc<Dir>,
    reference_catalog: Arc<super::reference_catalog::ReferenceCatalogStore>,
    portable_path_index: Arc<super::portable_path_index::PortablePathIndexStore>,
    logseq_claim_index: Arc<super::uuid_claim_index::LogseqClaimIndexStore>,
    page_name_index: Arc<super::page_name_index::PageNameOwnershipStore>,
}

pub(crate) struct DetachedBootstrapAuthoringIndexes {
    reference_catalog: Arc<super::reference_catalog::ReferenceCatalogStore>,
    portable_path_index: Arc<super::portable_path_index::PortablePathIndexStore>,
    logseq_claim_index: Arc<super::uuid_claim_index::LogseqClaimIndexStore>,
    page_name_index: Arc<super::page_name_index::PageNameOwnershipStore>,
    construction_resident_budget_bytes: usize,
}

impl DetachedBootstrapAuthoringIndexes {
    pub(crate) fn reference_catalog(&self) -> Arc<super::reference_catalog::ReferenceCatalogStore> {
        Arc::clone(&self.reference_catalog)
    }

    pub(crate) fn portable_path_index(
        &self,
    ) -> Arc<super::portable_path_index::PortablePathIndexStore> {
        Arc::clone(&self.portable_path_index)
    }

    pub(crate) fn logseq_claim_index(&self) -> Arc<super::uuid_claim_index::LogseqClaimIndexStore> {
        Arc::clone(&self.logseq_claim_index)
    }

    pub(crate) fn page_name_index(&self) -> Arc<super::page_name_index::PageNameOwnershipStore> {
        Arc::clone(&self.page_name_index)
    }

    pub(crate) const fn construction_resident_budget_bytes(&self) -> usize {
        self.construction_resident_budget_bytes
    }
}

fn parse_available_kib(meminfo: &str) -> Option<u64> {
    let value = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    value.checked_mul(1024)
}

fn finite_cgroup_available(maximum: &str, current: &str) -> Option<u64> {
    let maximum = maximum.trim().parse::<u64>().ok()?;
    if maximum >= (1_u64 << 60) {
        return None;
    }
    let current = current.trim().parse::<u64>().ok()?;
    Some(maximum.saturating_sub(current))
}

fn detached_bootstrap_available_memory_bytes() -> Option<u64> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let host = fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|meminfo| parse_available_kib(&meminfo));
        let cgroup_v2 = fs::read_to_string("/sys/fs/cgroup/memory.max")
            .ok()
            .zip(fs::read_to_string("/sys/fs/cgroup/memory.current").ok())
            .and_then(|(maximum, current)| finite_cgroup_available(&maximum, &current));
        let cgroup_v1 = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes")
            .ok()
            .zip(fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes").ok())
            .and_then(|(maximum, current)| finite_cgroup_available(&maximum, &current));
        return [host, cgroup_v2, cgroup_v1].into_iter().flatten().min();
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        None
    }
}

fn detached_bootstrap_construction_budget_for_available(available: Option<u64>) -> usize {
    const FALLBACK_BYTES: usize = 128 * 1024 * 1024;
    let target = available
        .and_then(|available| usize::try_from(available / 8).ok())
        .unwrap_or(FALLBACK_BYTES);
    target.clamp(
        tine_storage::DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
        tine_storage::MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
    )
}

fn detached_bootstrap_construction_resident_budget_bytes() -> usize {
    detached_bootstrap_construction_budget_for_available(detached_bootstrap_available_memory_bytes())
}

#[cfg(test)]
#[test]
fn detached_bootstrap_construction_budget_tracks_available_memory_with_bounds() {
    assert_eq!(
        parse_available_kib("MemAvailable: 1024 kB\n"),
        Some(1024 * 1024)
    );
    assert_eq!(finite_cgroup_available("1024\n", "256\n"), Some(768));
    assert_eq!(finite_cgroup_available("max\n", "256\n"), None);
    assert_eq!(
        detached_bootstrap_construction_budget_for_available(Some(128 * 1024 * 1024)),
        tine_storage::DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
    );
    assert_eq!(
        detached_bootstrap_construction_budget_for_available(Some(8 * 1024 * 1024 * 1024)),
        tine_storage::MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
    );
    assert_eq!(
        detached_bootstrap_construction_budget_for_available(None),
        128 * 1024 * 1024,
    );
}

impl BootstrapAuthoringCapability {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn archive_identity(&self) -> ControlDirectoryIdentity {
        self.archive_identity
    }

    pub(crate) fn reference_catalog(&self) -> Arc<super::reference_catalog::ReferenceCatalogStore> {
        Arc::clone(&self.reference_catalog)
    }

    pub(crate) fn portable_path_index(
        &self,
    ) -> Arc<super::portable_path_index::PortablePathIndexStore> {
        Arc::clone(&self.portable_path_index)
    }

    pub(crate) fn logseq_claim_index(&self) -> Arc<super::uuid_claim_index::LogseqClaimIndexStore> {
        Arc::clone(&self.logseq_claim_index)
    }

    pub(crate) fn page_name_index(&self) -> Arc<super::page_name_index::PageNameOwnershipStore> {
        Arc::clone(&self.page_name_index)
    }

    pub(crate) fn begin_detached_authoring(
        &self,
    ) -> Result<
        (
            DetachedBootstrapPublicationSession,
            DetachedBootstrapAuthoringIndexes,
        ),
        StoreError,
    > {
        let publication = DetachedBootstrapPublicationSession::new(
            &self.archive,
            self.workspace_id,
            self.archive_identity,
        )?;
        let publisher = publication.publisher();
        let construction_resident_budget_bytes =
            detached_bootstrap_construction_resident_budget_bytes();
        let indexes =
            DetachedBootstrapAuthoringIndexes {
                reference_catalog: Arc::new(
                    self.reference_catalog
                        .for_detached_bootstrap(publisher.clone())?,
                ),
                portable_path_index: Arc::new(self.portable_path_index.for_detached_bootstrap(
                    publisher.clone(),
                    construction_resident_budget_bytes,
                )?),
                logseq_claim_index: Arc::new(
                    self.logseq_claim_index
                        .for_detached_bootstrap_construction(
                            publisher.clone(),
                            construction_resident_budget_bytes,
                        )?,
                ),
                page_name_index: Arc::new(
                    self.page_name_index
                        .for_detached_bootstrap(publisher, construction_resident_budget_bytes)?,
                ),
                construction_resident_budget_bytes,
            };
        Ok((publication, indexes))
    }
}

impl ObjectStore {
    /// Open or create a store at an explicit root and retain the opened
    /// directory capability for all later operations.
    pub fn open(root: &Path, workspace_id: WorkspaceId) -> Result<Self, StoreError> {
        Self::open_with_lifecycle(root, workspace_id, ObjectStoreLifecycle::Authority)
    }

    /// Open the private archive while Markdown/Org is still the sole
    /// authority. On Android this uses ordinary app-private directory handles
    /// and accepts only unavailable durability capabilities; every content,
    /// type, collision, and ordinary I/O failure remains fatal.
    pub(crate) fn open_reconstructible_activation(
        root: &Path,
        workspace_id: WorkspaceId,
    ) -> Result<Self, StoreError> {
        Self::open_with_lifecycle(
            root,
            workspace_id,
            ObjectStoreLifecycle::ReconstructibleActivation,
        )
    }

    fn open_with_lifecycle(
        root: &Path,
        workspace_id: WorkspaceId,
        lifecycle: ObjectStoreLifecycle,
    ) -> Result<Self, StoreError> {
        let name = root
            .file_name()
            .ok_or_else(|| StoreError::UnsafeEntry("store root has no final component".into()))?;
        if !matches!(root.components().next_back(), Some(Component::Normal(_))) {
            return Err(StoreError::UnsafeEntry(
                "store root must end in a normal path component".into(),
            ));
        }
        let parent = root.parent().ok_or_else(|| {
            StoreError::UnsafeEntry("store root must have an existing parent".into())
        })?;
        let canonical_parent = fs::canonicalize(parent)?;
        let parent_capability = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
        let relative = Path::new(name);
        let name = name.to_str().ok_or_else(|| {
            StoreError::UnsafeEntry("store root final component is not UTF-8".into())
        })?;

        match parent_capability.symlink_metadata(relative) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(StoreError::UnsafeEntry(
                    "store root is not a real no-follow directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                parent_capability.create_dir(relative)?;
                sync_directory_for_lifecycle(&parent_capability, lifecycle)?;
            }
            Err(error) => return Err(error.into()),
        }

        let capability = open_directory_for_lifecycle(&parent_capability, name, lifecycle)?;
        ensure_directory_for_lifecycle(&capability, OBJECTS_DIR, lifecycle)?;
        ensure_directory_for_lifecycle(&capability, BATCHES_DIR, lifecycle)?;
        let store = Self {
            root_path: canonical_parent.join(name),
            workspace_id,
            capability,
            counters: Arc::new(StoreCounters::default()),
            lifecycle,
        };
        store.validate_namespace()?;
        Ok(store)
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// Duplicate the retained no-follow archive-root capability that roots the
    /// workspace runtime lease.
    ///
    /// The lease is deliberately archive-rooted rather than app-data-rooted:
    /// the returned handle is the same physical directory resource this store
    /// already authenticated, so two processes with different XDG, HOME, or
    /// Flatpak roots still contend on one lock, and renaming the archive
    /// pathname cannot split it.
    pub(crate) fn workspace_runtime_lease_capability(&self) -> std::io::Result<Dir> {
        self.capability.try_clone()
    }

    /// Duplicate this store directly from its retained no-follow archive-root
    /// capability.
    ///
    /// The duplicate is the *same* physical directory resource, never a fresh
    /// ambient pathname open, so a caller that already authenticated one exact
    /// archive can hand a consuming API (`seal_history_only`, `seal_enrolled_projection`)
    /// its own store value without ever reintroducing a pathname race. An
    /// archive renamed while retained open stays bound to the enrolled archive,
    /// and a look-alike directory that appears at the old pathname is not
    /// reachable through the duplicate at all.
    pub(crate) fn duplicate_retained_capability(&self) -> Result<Self, StoreError> {
        Ok(Self {
            root_path: self.root_path.clone(),
            workspace_id: self.workspace_id,
            capability: self.capability.try_clone()?,
            counters: Arc::clone(&self.counters),
            lifecycle: self.lifecycle,
        })
    }

    /// Prove this store's retained capability and its enrolled archive pathname
    /// still name one and the same physical directory.
    ///
    /// The retained capability remains the authority; this only refuses an
    /// *ambiguous* archive. If the archive was renamed while it stayed retained
    /// open and a look-alike directory now occupies the enrolled pathname, then
    /// two different directories both answer to "the enrolled archive": one by
    /// resource identity, one by pathname. A one-shot durable publication must
    /// block there rather than silently pick a winner, because the two
    /// candidates diverge immediately afterwards.
    ///
    /// Nothing is created, repaired, claimed, or written. The check is one
    /// ambient parent open, one no-follow child open, and one identity stat.
    pub(crate) fn authenticate_unambiguous_archive_pathname(&self) -> Result<(), StoreError> {
        let parent = self.root_path.parent().ok_or_else(|| {
            StoreError::UnsafeEntry("store root must have an existing parent".into())
        })?;
        let name = self
            .root_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                StoreError::UnsafeEntry("store root final component is not UTF-8".into())
            })?;
        let parent_capability = Dir::open_ambient_dir(parent, ambient_authority())?;
        let named = open_existing_dir_nofollow(&parent_capability, name)?.ok_or_else(|| {
            StoreError::UnsafeEntry(
                "enrolled archive pathname no longer names a real no-follow directory".into(),
            )
        })?;
        if control_directory_identity(&named)? != self.canonical_archive_identity()? {
            return Err(StoreError::UnsafeEntry(
                "enrolled archive pathname no longer names the retained archive capability".into(),
            ));
        }
        Ok(())
    }

    /// Validate and retain one object independently of any manifest delivery.
    pub fn stage_object_bytes(&self, bytes: &[u8]) -> Result<ContentDigest, StoreError> {
        let object = OperationObject::decode(bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        let digest = ContentDigest::of(bytes);
        let objects = self.open_namespace(OBJECTS_DIR)?;
        publish_immutable(
            &objects,
            &object_filename(digest),
            bytes,
            Collision::Object(digest),
        )?;
        Ok(digest)
    }

    /// Validate and publish the sole batch commit marker. Missing objects do
    /// not prevent staging the marker and remain invisible until complete.
    pub fn stage_manifest_bytes(&self, bytes: &[u8]) -> Result<BatchId, StoreError> {
        self.stage_manifest_bytes_impl(bytes, false)
    }

    /// Receive a manifest through one exact shared-enrollment descriptor.
    ///
    /// Historical bootstrap manifests are admitted only on this path. The
    /// descriptor authority is checked independently of the manifest before
    /// the ordinary immutable collision and lineage validation runs.
    pub(crate) fn stage_shared_provider_manifest_bytes(
        &self,
        ingress: &super::enrollment::SharedProviderIngressAuthority,
        bytes: &[u8],
    ) -> Result<BatchId, StoreError> {
        let manifest = OperationBatch::decode(bytes)?;
        if ingress.workspace_id() != self.workspace_id
            || manifest.workspace_id() != ingress.workspace_id()
        {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        if manifest.lineage_digest() != ingress.lineage_digest() {
            return Err(StoreError::LineageMismatch {
                expected: ingress.lineage_digest(),
                found: manifest.lineage_digest(),
            });
        }
        self.stage_manifest_bytes_impl(bytes, true)
    }

    /// Stage one canonical historical bootstrap manifest for deterministic
    /// simulator fixture ingress.
    ///
    /// The unforgeable safe-code authority is owned only by the simulator
    /// module. This is not an app-runtime, migration, provider-reconciliation,
    /// or enrollment API. It preserves normal decoding, workspace and lineage
    /// validation, size bounds, and immutable collision checks, and bypasses
    /// only the public bootstrap-origin admission guard.
    pub(super) fn stage_simulator_bootstrap_manifest_bytes(
        &self,
        _fixture_ingress: &SimulatorBootstrapFixtureIngress,
        bytes: &[u8],
    ) -> Result<BatchId, StoreError> {
        let manifest = OperationBatch::decode(bytes)?;
        assert_eq!(
            manifest.origin(),
            BatchOrigin::BootstrapImport,
            "simulator bootstrap fixture ingress requires BootstrapImport origin"
        );
        self.stage_manifest_bytes_impl(&manifest.encode()?, true)
    }

    fn stage_manifest_bytes_impl(
        &self,
        bytes: &[u8],
        allow_bootstrap: bool,
    ) -> Result<BatchId, StoreError> {
        let manifest = OperationBatch::decode(bytes)?;
        if !allow_bootstrap && manifest.origin() == BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapBatchRequiresDirectPublication);
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        let batch_id = manifest.batch_id();
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        if read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)?.is_some() {
            self.check_or_establish_lineage(manifest.lineage_digest())?;
            publish_immutable(&batches, &filename, bytes, Collision::Batch(batch_id))?;
            return Ok(batch_id);
        }
        self.check_or_establish_lineage(manifest.lineage_digest())?;
        publish_immutable(&batches, &filename, bytes, Collision::Batch(batch_id))?;
        Ok(batch_id)
    }

    /// Publish a prevalidated complete batch in the required order: every
    /// content-addressed object first, then the manifest commit marker.
    pub fn publish_prepared(&self, batch: &PreparedBatch) -> Result<(), StoreError> {
        if batch.manifest().origin() == BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapBatchRequiresDirectPublication);
        }
        self.publish_prepared_impl(batch, false)
    }

    /// Seed a bootstrap-origin archive fixture through the deterministic
    /// simulator's unforgeable ingress authority.
    pub(super) fn publish_simulator_bootstrap_prepared(
        &self,
        _fixture_ingress: &SimulatorBootstrapFixtureIngress,
        batch: &PreparedBatch,
    ) -> Result<(), StoreError> {
        self.publish_bootstrap_prepared_fixture(batch)
    }

    /// Seed a bootstrap-origin archive fixture without exposing a production
    /// publication bypass.
    #[cfg(test)]
    pub(crate) fn publish_bootstrap_prepared_for_test(
        &self,
        batch: &PreparedBatch,
    ) -> Result<(), StoreError> {
        self.publish_bootstrap_prepared_fixture(batch)
    }

    /// Seed only a bootstrap-origin manifest for an incomplete-store fixture.
    #[cfg(test)]
    pub(crate) fn stage_bootstrap_manifest_bytes_for_test(
        &self,
        bytes: &[u8],
    ) -> Result<BatchId, StoreError> {
        self.stage_manifest_bytes_impl(bytes, true)
    }

    fn publish_bootstrap_prepared_fixture(&self, batch: &PreparedBatch) -> Result<(), StoreError> {
        assert_eq!(
            batch.manifest().origin(),
            BatchOrigin::BootstrapImport,
            "bootstrap fixture publication requires BootstrapImport origin"
        );
        self.publish_prepared_impl(batch, true)
    }

    fn publish_prepared_impl(
        &self,
        batch: &PreparedBatch,
        allow_bootstrap: bool,
    ) -> Result<(), StoreError> {
        if batch.manifest().workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: batch.manifest().workspace_id(),
            });
        }
        for object in batch.objects() {
            self.stage_object_bytes(&object.encode()?)?;
        }
        publish_after_objects_hook()?;
        self.stage_manifest_bytes_impl(&batch.manifest().encode()?, allow_bootstrap)?;
        Ok(())
    }

    pub(crate) fn begin_bootstrap_publication_batch(
        &self,
    ) -> Result<BootstrapPublicationBatch<'_>, StoreError> {
        Ok(BootstrapPublicationBatch {
            store: self,
            physical: tine_storage::ExactImmutablePublicationBatch::new(&self.capability)
                .map_err(filesystem_error_without_collision)?,
            inventory_root: None,
            inventory_pages: BTreeMap::new(),
            blob_root: None,
            blob_pages: BTreeMap::new(),
            part_packs: BTreeMap::new(),
            parts: BTreeMap::new(),
        })
    }

    pub(crate) fn publish_bootstrap_source_inventory_page(
        &self,
        root: SourceInventoryRootV1,
        page: &SourceInventoryIndexPageV1,
    ) -> Result<(), StoreError> {
        let dir =
            self.bootstrap_index_root_dir(BOOTSTRAP_SOURCE_INVENTORY_DIR, root.digest(), true)?;
        let bytes = page.encode()?;
        publish_bootstrap_immutable(
            &dir,
            &bootstrap_page_filename(page.page_ordinal()),
            &bytes,
            "source inventory page",
            format!("{}/{}", hex_bytes(root.digest()), page.page_ordinal()),
        )
    }

    pub(crate) fn publish_bootstrap_source_blob_page(
        &self,
        root: SourceBlobChunkRootV1,
        page: &SourceBlobIndexPageV1,
    ) -> Result<(), StoreError> {
        let dir = self.bootstrap_index_root_dir(BOOTSTRAP_SOURCE_BLOB_DIR, root.digest(), true)?;
        let bytes = page.encode()?;
        publish_bootstrap_immutable(
            &dir,
            &bootstrap_page_filename(page.page_ordinal()),
            &bytes,
            "source blob page",
            format!("{}/{}", hex_bytes(root.digest()), page.page_ordinal()),
        )
    }

    pub(crate) fn publish_bootstrap_source_chunk(
        &self,
        digest: SourceBlobChunkDigestV1,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        if bytes.is_empty()
            || bytes.len() > MAX_SOURCE_BLOB_CHUNK_BYTES as usize
            || ContentDigest::of(bytes).as_bytes() != digest.as_bytes()
        {
            return Err(StoreError::BootstrapArtifactMismatch(
                "source chunk digest or length",
            ));
        }
        let dir = self.bootstrap_namespace(BOOTSTRAP_SOURCE_CHUNKS_DIR, true)?;
        let identity = hex_bytes(digest.as_bytes());
        publish_bootstrap_immutable(&dir, &identity, bytes, "source chunk", identity.clone())
    }

    pub(crate) fn publish_bootstrap_object_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<ContentDigest, StoreError> {
        let object = OperationObject::decode(bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        let digest = ContentDigest::of(bytes);
        let dir = self.bootstrap_namespace(BOOTSTRAP_OBJECTS_DIR, true)?;
        publish_bootstrap_immutable(
            &dir,
            &object_filename(digest),
            bytes,
            "bootstrap operation object",
            digest.to_string(),
        )?;
        Ok(digest)
    }

    #[cfg(test)]
    pub(crate) fn publish_bootstrap_part_pack_for_test(
        &self,
        descriptor: BootstrapPartDescriptorV1,
        objects: &[Vec<u8>],
    ) -> Result<(), StoreError> {
        let mut pack = Vec::new();
        for object in objects {
            let length = u32::try_from(object.len()).map_err(|_| {
                StoreError::BootstrapArtifactMismatch("bootstrap test part object length")
            })?;
            pack.extend_from_slice(&length.to_be_bytes());
            pack.extend_from_slice(object);
        }
        if pack.len() as u64 > MAX_BOOTSTRAP_PART_PACK_BYTES {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap test part object pack length",
            ));
        }
        let part_name = hex_bytes(descriptor.part_id().as_bytes());
        let dir = self.bootstrap_namespace(BOOTSTRAP_PART_PACKS_DIR, true)?;
        publish_bootstrap_immutable(
            &dir,
            &part_name,
            &pack,
            "bootstrap test part object pack",
            part_name.clone(),
        )
    }

    pub(crate) fn publish_bootstrap_part_artifacts(
        &self,
        descriptor: BootstrapPartDescriptorV1,
        manifest_bytes: &[u8],
        spans: &BootstrapPartSpanIndexV1,
    ) -> Result<(), StoreError> {
        let manifest = OperationBatch::decode(manifest_bytes)?;
        self.require_bootstrap_manifest(descriptor, &manifest)?;
        let manifest_digest = ContentDigest::of(manifest_bytes);
        let span_bytes = spans.encode()?;
        descriptor.validate_loaded_artifacts(
            BootstrapManifestFingerprintV1::from_bytes(*manifest_digest.as_bytes()),
            &manifest
                .required_objects()
                .iter()
                .map(|object| {
                    PayloadObjectDescriptorV1::new(
                        object.content_digest(),
                        object.encoded_byte_length(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            &[FullObjectDescriptorV1::manifest_defined(
                *ContentDigest::of(&span_bytes).as_bytes(),
                span_bytes.len() as u64,
            )?],
        )?;
        spans.validate_part(descriptor.evidence())?;

        let parts = self.bootstrap_namespace(BOOTSTRAP_PARTS_DIR, true)?;
        let part_name = hex_bytes(descriptor.part_id().as_bytes());
        publish_bootstrap_immutable(
            &parts,
            &part_name,
            manifest_bytes,
            "bootstrap part manifest",
            part_name.clone(),
        )?;

        let evidence = descriptor.evidence();
        let evidence_bytes = evidence.encode()?;
        let evidence_name = hex_bytes(evidence.evidence_digest().as_bytes());
        let evidence_dir = self.bootstrap_namespace(BOOTSTRAP_EVIDENCE_DIR, true)?;
        publish_bootstrap_immutable(
            &evidence_dir,
            &evidence_name,
            &evidence_bytes,
            "bootstrap part evidence",
            evidence_name.clone(),
        )?;

        let span_dir = self.bootstrap_namespace(BOOTSTRAP_PART_SPANS_DIR, true)?;
        publish_bootstrap_immutable(
            &span_dir,
            &part_name,
            &span_bytes,
            "bootstrap part span index",
            part_name.clone(),
        )
    }

    pub(crate) fn publish_bootstrap_aggregate_prefix(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
    ) -> Result<BootstrapAggregateDigestV1, StoreError> {
        self.require_bootstrap_aggregate_context(aggregate)?;
        let bytes = aggregate.encode()?;
        let digest = aggregate.aggregate_digest();
        let name = hex_bytes(digest.as_bytes());
        let dir = self.bootstrap_namespace(BOOTSTRAP_AGGREGATES_DIR, true)?;
        publish_bootstrap_immutable(&dir, &name, &bytes, "bootstrap aggregate", name.clone())?;
        Ok(digest)
    }

    /// Validate every direct prefix artifact, then publish the sole bootstrap
    /// authority marker last. Raw source chunks are checked here; reopen can
    /// skip rereading them because later engine replay needs only validated
    /// part artifacts.
    pub(crate) fn commit_bootstrap_aggregate(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
    ) -> Result<BootstrapPublicationIdV1, StoreError> {
        self.require_bootstrap_aggregate_context(aggregate)?;
        self.validate_bootstrap_aggregate_artifacts(aggregate, true)?;
        self.check_or_establish_lineage(aggregate.lineage_digest())?;
        let commit = BootstrapAggregateCommitV1::for_aggregate(aggregate)?;
        let bytes = commit.encode()?;
        let publication_id = aggregate.publication_id();
        let name = hex_bytes(publication_id.as_bytes());
        let dir = self.bootstrap_namespace(BOOTSTRAP_COMMITS_DIR, true)?;
        publish_bootstrap_immutable(&dir, &name, &bytes, "bootstrap commit", name.clone())?;
        Ok(publication_id)
    }

    pub(crate) fn commit_durably_staged_bootstrap_aggregate(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
        staged: DurablyStagedBootstrapPrefix,
    ) -> Result<BootstrapPublicationIdV1, StoreError> {
        self.require_bootstrap_aggregate_context(aggregate)?;
        if staged.workspace_id != self.workspace_id
            || staged.archive_identity != self.canonical_archive_identity()?
            || staged.aggregate_digest != aggregate.aggregate_digest()
        {
            return Err(StoreError::BootstrapArtifactMismatch(
                "durably staged bootstrap prefix binding",
            ));
        }
        self.check_or_establish_lineage(aggregate.lineage_digest())?;
        let commit = BootstrapAggregateCommitV1::for_aggregate(aggregate)?;
        let bytes = commit.encode()?;
        let publication_id = aggregate.publication_id();
        let name = hex_bytes(publication_id.as_bytes());
        let dir = self.bootstrap_namespace(BOOTSTRAP_COMMITS_DIR, true)?;
        publish_bootstrap_immutable(&dir, &name, &bytes, "bootstrap commit", name.clone())?;
        Ok(publication_id)
    }

    /// Direct reopen begins with the portable publication ID and never
    /// enumerates a bootstrap prefix.
    pub(crate) fn load_bootstrap_publication(
        &self,
        publication_id: BootstrapPublicationIdV1,
    ) -> Result<ValidatedBootstrapPublicationV1, StoreError> {
        self.load_bootstrap_publication_with_validation(publication_id, true)
    }

    /// Authenticate the compact commit/aggregate root while deferring old
    /// immutable part and source-index leaves to their ordinary verified reads.
    pub(crate) fn load_bootstrap_publication_deferred(
        &self,
        publication_id: BootstrapPublicationIdV1,
    ) -> Result<ValidatedBootstrapPublicationV1, StoreError> {
        self.load_bootstrap_publication_with_validation(publication_id, false)
    }

    fn load_bootstrap_publication_with_validation(
        &self,
        publication_id: BootstrapPublicationIdV1,
        validate_artifacts: bool,
    ) -> Result<ValidatedBootstrapPublicationV1, StoreError> {
        let commits = self.bootstrap_namespace(BOOTSTRAP_COMMITS_DIR, false)?;
        let commit_name = hex_bytes(publication_id.as_bytes());
        let commit_bytes = read_required_regular(
            &commits,
            &commit_name,
            MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES as u64,
            None,
        )?;
        let commit = BootstrapAggregateCommitV1::decode(&commit_bytes)?;
        if commit.publication_id() != publication_id {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap commit publication identity",
            ));
        }

        let aggregates = self.bootstrap_namespace(BOOTSTRAP_AGGREGATES_DIR, false)?;
        let aggregate_name = hex_bytes(commit.aggregate_digest().as_bytes());
        let aggregate_bytes = read_required_regular(
            &aggregates,
            &aggregate_name,
            MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES as u64,
            Some(commit.aggregate_byte_length()),
        )?;
        let aggregate = BootstrapAggregateManifestV1::decode(&aggregate_bytes)?;
        commit.validate_aggregate(&aggregate)?;
        if aggregate.publication_id() != publication_id
            || aggregate.aggregate_digest() != commit.aggregate_digest()
        {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap aggregate direct identity",
            ));
        }
        self.require_bootstrap_aggregate_context(&aggregate)?;
        self.require_lineage(aggregate.lineage_digest())?;
        if validate_artifacts {
            self.validate_bootstrap_aggregate_artifacts(&aggregate, false)?;
        }

        let final_commit = read_required_regular(
            &commits,
            &commit_name,
            MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES as u64,
            Some(commit_bytes.len() as u64),
        )?;
        let final_aggregate = read_required_regular(
            &aggregates,
            &aggregate_name,
            MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES as u64,
            Some(aggregate_bytes.len() as u64),
        )?;
        if final_commit != commit_bytes || final_aggregate != aggregate_bytes {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap commit or aggregate changed during direct validation",
            ));
        }
        Ok(ValidatedBootstrapPublicationV1 { aggregate })
    }

    pub(crate) fn validate_bootstrap_publication(
        &self,
        publication: &ValidatedBootstrapPublicationV1,
    ) -> Result<(), StoreError> {
        self.validate_bootstrap_aggregate_artifacts(publication.aggregate(), false)
    }

    pub(crate) fn load_bootstrap_part(
        &self,
        publication: &ValidatedBootstrapPublicationV1,
        ordinal: usize,
    ) -> Result<LoadedBootstrapPartV1, StoreError> {
        let descriptor = *publication.aggregate.parts().get(ordinal).ok_or(
            StoreError::BootstrapArtifactMismatch("bootstrap part ordinal"),
        )?;
        self.load_and_validate_bootstrap_part(&publication.aggregate, descriptor)
    }

    pub(crate) fn inspect_bootstrap_aggregate(
        &self,
        expected: &BootstrapAggregateManifestV1,
    ) -> BootstrapPublicationInspectionV1 {
        match self.inspect_bootstrap_aggregate_inner(expected) {
            Ok(inspection) => inspection,
            Err(error) => BootstrapPublicationInspectionV1::CorruptOrConflicting(error),
        }
    }

    /// Inspect a single manifest and validate every present required object.
    /// Missing objects stage the batch; corrupt or mismatched objects reject it.
    #[track_caller]
    pub fn inspect_batch(&self, batch_id: BatchId) -> Result<BatchInspection, StoreError> {
        INSPECT_BATCH_CALLS.fetch_add(1, Ordering::Relaxed);
        let site = super::inspect_site_trace_enabled().then(|| {
            let caller = std::panic::Location::caller();
            format!("{}:{}", caller.file(), caller.line())
        });
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        let manifest_bytes =
            match read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)? {
                None => return Ok(BatchInspection::Absent),
                Some(bytes) => bytes,
            };
        self.counters
            .inspected_manifest_operations
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .inspected_manifest_bytes
            .fetch_add(manifest_bytes.len(), Ordering::Relaxed);
        let manifest = OperationBatch::decode(&manifest_bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }

        if let Some(site) = site {
            if let Ok(mut sites) = INSPECT_BATCH_SITES.lock() {
                let entry = sites.entry(site).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += manifest.required_objects().len();
            }
        }
        let objects_dir = self.open_namespace(OBJECTS_DIR)?;
        let mut missing = Vec::new();
        let mut objects = Vec::with_capacity(manifest.required_objects().len());
        for descriptor in manifest.required_objects() {
            self.counters
                .inspected_object_operations
                .fetch_add(1, Ordering::Relaxed);
            let filename = object_filename(descriptor.content_digest());
            let Some(bytes) = read_optional_regular(
                &objects_dir,
                &filename,
                MAX_OBJECT_BYTES as u64,
                Some(descriptor.encoded_byte_length()),
            )?
            else {
                missing.push(descriptor.clone());
                continue;
            };
            self.counters
                .inspected_object_bytes
                .fetch_add(bytes.len(), Ordering::Relaxed);
            INSPECT_BATCH_OBJECT_READS.fetch_add(1, Ordering::Relaxed);
            INSPECT_BATCH_OBJECT_BYTES.fetch_add(bytes.len(), Ordering::Relaxed);
            INSPECT_BATCH_DIGEST_BYTES.fetch_add(bytes.len(), Ordering::Relaxed);
            let content_digest = ContentDigest::of(&bytes);
            if content_digest != descriptor.content_digest() {
                return Err(StoreError::ObjectPathMismatch(descriptor.content_digest()));
            }
            let object = OperationObject::decode(&bytes)?;
            if object.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: object.workspace_id(),
                });
            }
            let actual = ObjectDescriptor::new(
                object.document_id(),
                object.kind(),
                content_digest,
                bytes.len() as u64,
            )?;
            if actual != *descriptor {
                return Err(StoreError::Batch(BatchError::DescriptorMismatch {
                    expected: descriptor.clone(),
                    actual,
                }));
            }
            objects.push(object);
        }

        if !missing.is_empty() {
            return Ok(BatchInspection::Staged { manifest, missing });
        }
        // Exact lookup against the atomically established immutable lineage
        // claim keeps the Ready path independent of archive cardinality.
        // Store open and explicit `committed_manifests` remain full audits.
        self.require_lineage(manifest.lineage_digest())?;
        let prepared = PreparedBatch::new(manifest, objects)?;
        Ok(BatchInspection::Ready(ValidatedBatch::new(prepared)))
    }

    pub(crate) fn reload_accepted_document_object(
        &self,
        manifest: &OperationBatch,
        document_id: super::DocumentId,
    ) -> Result<OperationObject, StoreError> {
        let batch_id = manifest.batch_id();
        let descriptor = manifest
            .required_objects()
            .iter()
            .find(|descriptor| {
                descriptor.kind() == super::ObjectKind::CrdtUpdate
                    && descriptor.document_id() == document_id
            })
            .ok_or(StoreError::AcceptedDocumentUpdateMissing {
                batch_id,
                document_id,
            })?;
        let objects_dir = self.open_namespace(OBJECTS_DIR)?;
        let filename = object_filename(descriptor.content_digest());
        self.counters
            .accepted_object_reads
            .fetch_add(1, Ordering::Relaxed);
        crate::fast_commit::note_archive_object_read();
        let bytes = read_required_regular(
            &objects_dir,
            &filename,
            MAX_OBJECT_BYTES as u64,
            Some(descriptor.encoded_byte_length()),
        )?;
        if ContentDigest::of(&bytes) != descriptor.content_digest() {
            return Err(StoreError::ObjectPathMismatch(descriptor.content_digest()));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        let actual = object.descriptor()?;
        if actual != *descriptor {
            return Err(StoreError::Batch(BatchError::DescriptorMismatch {
                expected: descriptor.clone(),
                actual,
            }));
        }
        Ok(object)
    }

    pub(crate) fn reload_accepted_manifest(
        &self,
        batch_id: BatchId,
        expected_manifest_fingerprint: ContentDigest,
    ) -> Result<OperationBatch, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        self.counters
            .accepted_manifest_reads
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .dag_manifest_reads
            .fetch_add(1, Ordering::Relaxed);
        let bytes = read_required_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)?;
        let actual = ContentDigest::of(&bytes);
        if actual != expected_manifest_fingerprint {
            return Err(StoreError::AcceptedManifestMismatch {
                batch_id,
                expected: expected_manifest_fingerprint,
                actual,
            });
        }
        let manifest = OperationBatch::decode(&bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        Ok(manifest)
    }

    pub(crate) fn accepted_read_stats(&self) -> AcceptedReadStats {
        AcceptedReadStats {
            manifest_reads: self
                .counters
                .accepted_manifest_reads
                .load(Ordering::Relaxed),
            object_reads: self.counters.accepted_object_reads.load(Ordering::Relaxed),
        }
    }

    pub fn instrumentation(&self) -> ObjectStoreStats {
        self.counters.snapshot()
    }

    pub(crate) fn seal_enrolled_projection(
        self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<EnrolledProjectionOpen, (Self, StoreError)> {
        let history = match self.seal_existing_engine_history(binding) {
            Ok(history) => history,
            Err(error) => return Err((self, error)),
        };
        if let SealedControl::Existing(history) = &history {
            match history.current_bootstrap_binding() {
                Ok(Some(_)) => return Err((self, StoreError::InactiveBootstrapHistory)),
                Ok(None) => {}
                Err(error) => return Err((self, error)),
            }
            // A promoted archive is never an ordinary enrolled archive. Refusing
            // here keeps a promoted lineage from being silently reinterpreted as
            // an unanchored ordinary history.
            match history.read_promoted_runtime_state() {
                Ok(None) => {}
                Ok(Some(_)) => {
                    return Err((
                        self,
                        StoreError::PromotedRuntimeStateMismatch(
                            "ordinary enrolled open cannot consume a promoted runtime archive",
                        ),
                    ));
                }
                Err(error) => return Err((self, error)),
            }
        }
        self.finish_sealing_projection(binding, history)
    }

    /// Seal the enrolled projection controls for a promoted bootstrap-anchored
    /// runtime.
    ///
    /// This is the only construction that opens a bootstrap-bound durable
    /// history as a writable runtime. It requires an already published durable
    /// promotion state that is byte-equal to `expected`, claims this exact
    /// endpoint, and still binds the live authoritative root's bootstrap
    /// aggregate.
    pub(crate) fn seal_promoted_projection(
        self,
        binding: super::hot_engine::ProjectionStorageBinding,
        expected: &PromotedRuntimeStateV1,
    ) -> Result<EnrolledProjectionOpen, (Self, StoreError)> {
        let mut history = match self.seal_existing_engine_history(binding) {
            Ok(history) => history,
            Err(error) => return Err((self, error)),
        };
        let SealedControl::Existing(existing) = &mut history else {
            return Err((
                self,
                StoreError::PromotedRuntimeStateMismatch(
                    "promoted runtime open requires an existing durable bootstrap history",
                ),
            ));
        };
        if let Err(error) = existing.authorize_promoted_lineage(expected) {
            return Err((self, error));
        }
        self.finish_sealing_projection(binding, history)
    }

    fn finish_sealing_projection(
        self,
        binding: super::hot_engine::ProjectionStorageBinding,
        mut history: SealedControl<DurableEngineHistoryStore>,
    ) -> Result<EnrolledProjectionOpen, (Self, StoreError)> {
        let mut work = match self.seal_existing_projection_work(binding) {
            Ok(work) => work,
            Err(error) => return Err((self, error)),
        };
        let history_parent_created =
            match history.bind_absent_parent(&self.capability, self.lifecycle) {
                Ok(created) => created,
                Err(error) => return Err((self, error)),
            };
        if let Err(error) = work.bind_absent_parent(&self.capability, self.lifecycle) {
            if history_parent_created {
                history.release_empty_parent(&self.capability);
            }
            return Err((self, error));
        }
        Ok(EnrolledProjectionOpen {
            store: Some(self),
            binding,
            history: Some(history),
            work: Some(work),
        })
    }

    /// Seal the durable-history control for inactive bootstrap installation.
    /// This performs the same no-follow, absence, retained-resource, and
    /// substitution checks as enrolled open without touching projection-work.
    pub(crate) fn seal_history_only(
        self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<HistoryOnlyOpen, (Self, StoreError)> {
        let mut history = match self.seal_existing_engine_history(binding) {
            Ok(history) => history,
            Err(error) => return Err((self, error)),
        };
        if let Err(error) = history.bind_absent_parent(&self.capability, self.lifecycle) {
            return Err((self, error));
        }
        Ok(HistoryOnlyOpen {
            store: Some(self),
            binding,
            history: Some(history),
        })
    }

    fn seal_existing_engine_history(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<SealedControl<DurableEngineHistoryStore>, StoreError> {
        // Reject incompatible durable evidence before opening the durable lock
        // can create it. `open_sealed_existing` repeats the validation after
        // lock acquisition so a substitution in this window still fails shut.
        self.preflight_engine_history(binding)?;
        #[cfg(test)]
        sealed_history_after_preflight_hook();
        let Some(histories) = open_existing_directory_for_lifecycle(
            &self.capability,
            ENGINE_HISTORY_DIR,
            self.lifecycle,
        )?
        else {
            return Ok(SealedControl::Absent(AbsentControlName {
                namespace_name: ENGINE_HISTORY_DIR,
                namespace: None,
                namespace_identity: None,
                endpoint_name: binding.endpoint.endpoint_id.to_string(),
            }));
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) =
            open_existing_directory_for_lifecycle(&histories, &endpoint_name, self.lifecycle)?
        else {
            return Ok(SealedControl::Absent(AbsentControlName {
                namespace_name: ENGINE_HISTORY_DIR,
                namespace_identity: Some(control_directory_identity(&histories)?),
                namespace: Some(histories),
                endpoint_name,
            }));
        };
        let head = read_optional_regular(&control, ENGINE_HISTORY_HEAD_FILE, 64, None)?;
        let claim = read_optional_regular(&control, ENGINE_HISTORY_CLAIM_FILE, 256, None)?;
        match (head, claim) {
            (None, None) => Err(StoreError::MalformedHistoryIndex),
            (Some(_), Some(_)) => DurableEngineHistoryStore::open_sealed_existing(
                self.workspace_id,
                binding.endpoint.endpoint_id,
                binding.endpoint.graph_resource_id,
                binding.receipt_store_id,
                control,
                self.capability.try_clone()?,
                open_engine_history_transition_lock(&self.capability, self.lifecycle)?,
                Arc::clone(&self.counters),
            )
            .map(SealedControl::Existing),
            _ => Err(StoreError::MalformedHistoryIndex),
        }
    }

    fn seal_existing_projection_work(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<SealedControl<super::ProjectionWorkIndex>, StoreError> {
        let Some(root) = open_existing_directory_for_lifecycle(
            &self.capability,
            PROJECTION_WORK_DIR,
            self.lifecycle,
        )?
        else {
            return Ok(SealedControl::Absent(AbsentControlName {
                namespace_name: PROJECTION_WORK_DIR,
                namespace: None,
                namespace_identity: None,
                endpoint_name: binding.endpoint.endpoint_id.to_string(),
            }));
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) =
            open_existing_directory_for_lifecycle(&root, &endpoint_name, self.lifecycle)?
        else {
            return Ok(SealedControl::Absent(AbsentControlName {
                namespace_name: PROJECTION_WORK_DIR,
                namespace_identity: Some(control_directory_identity(&root)?),
                namespace: Some(root),
                endpoint_name,
            }));
        };
        let head = read_optional_regular(&control, "projection-work.head", 64, None)?;
        let claim = read_optional_regular(&control, "projection-work.claim", 256, None)?;
        match (head, claim) {
            (None, None) => Err(StoreError::MalformedHistoryIndex),
            (Some(_), Some(_)) => super::ProjectionWorkIndex::open_sealed_existing(
                control,
                self.workspace_id,
                binding.endpoint.endpoint_id,
                binding.endpoint.graph_resource_id,
                binding.receipt_store_id,
            )
            .map(SealedControl::Existing)
            .map_err(|error| StoreError::Scratch(error.to_string())),
            _ => Err(StoreError::MalformedHistoryIndex),
        }
    }

    #[cfg(test)]
    pub(crate) fn open_engine_history(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<DurableEngineHistoryStore, StoreError> {
        self.preflight_engine_history(binding)?;
        let endpoint = binding.endpoint;
        ensure_directory_for_lifecycle(&self.capability, ENGINE_HISTORY_DIR, self.lifecycle)?;
        let histories = open_dir_nofollow(&self.capability, ENGINE_HISTORY_DIR)?;
        let endpoint_name = endpoint.endpoint_id.to_string();
        ensure_directory_for_lifecycle(&histories, &endpoint_name, self.lifecycle)?;
        let control = open_dir_nofollow(&histories, &endpoint_name)?;
        for name in [ENGINE_HISTORY_NODES_DIR, ENGINE_HISTORY_ROOTS_DIR] {
            ensure_directory_for_lifecycle(&control, name, self.lifecycle)?;
        }
        DurableEngineHistoryStore::new(
            self.workspace_id,
            endpoint.endpoint_id,
            endpoint.graph_resource_id,
            binding.receipt_store_id,
            control.try_clone()?,
            self.capability.try_clone()?,
            open_directory_for_lifecycle(&control, ENGINE_HISTORY_ROOTS_DIR, self.lifecycle)?,
            EngineHistoryStore {
                capability: open_directory_for_lifecycle(
                    &control,
                    ENGINE_HISTORY_NODES_DIR,
                    self.lifecycle,
                )?,
                counters: Arc::clone(&self.counters),
                storage_fault: AtomicBool::new(false),
            },
            open_engine_history_transition_lock(&self.capability, self.lifecycle)?,
        )
    }

    fn open_absent_engine_history(
        &self,
        absence: AbsentControlName,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<DurableEngineHistoryStore, StoreError> {
        let control = absence.claim(&self.capability, self.lifecycle)?;
        for name in [ENGINE_HISTORY_NODES_DIR, ENGINE_HISTORY_ROOTS_DIR] {
            control.create_dir(name)?;
        }
        sync_directory_for_lifecycle(&control, self.lifecycle)?;
        DurableEngineHistoryStore::new(
            self.workspace_id,
            binding.endpoint.endpoint_id,
            binding.endpoint.graph_resource_id,
            binding.receipt_store_id,
            control.try_clone()?,
            self.capability.try_clone()?,
            open_directory_for_lifecycle(&control, ENGINE_HISTORY_ROOTS_DIR, self.lifecycle)?,
            EngineHistoryStore {
                capability: open_directory_for_lifecycle(
                    &control,
                    ENGINE_HISTORY_NODES_DIR,
                    self.lifecycle,
                )?,
                counters: Arc::clone(&self.counters),
                storage_fault: AtomicBool::new(false),
            },
            open_engine_history_transition_lock(&self.capability, self.lifecycle)?,
        )
    }

    #[cfg(test)]
    pub(crate) fn start_engine_history(&self) -> Result<EngineHistoryStore, StoreError> {
        ensure_directory_for_lifecycle(&self.capability, ENGINE_HISTORY_DIR, self.lifecycle)?;
        let histories = self.open_namespace(ENGINE_HISTORY_DIR)?;
        let run = format!("run-{}", Uuid::new_v4());
        ensure_directory_for_lifecycle(&histories, &run, self.lifecycle)?;
        Ok(EngineHistoryStore {
            capability: open_dir_nofollow(&histories, &run)?,
            counters: Arc::clone(&self.counters),
            storage_fault: AtomicBool::new(false),
        })
    }

    #[cfg(test)]
    pub(crate) fn start_block_claim_index(&self) -> Result<BlockClaimIndexStore, StoreError> {
        ensure_directory_for_lifecycle(&self.capability, BLOCK_CLAIM_INDEX_DIR, self.lifecycle)?;
        let indexes = self.open_namespace(BLOCK_CLAIM_INDEX_DIR)?;
        let run = format!("run-{}", Uuid::new_v4());
        ensure_directory_for_lifecycle(&indexes, &run, self.lifecycle)?;
        let run = open_dir_nofollow(&indexes, &run)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        let file = run.open_with(BLOCK_CLAIM_INDEX_FILE, &options)?.into_std();
        file.sync_all()?;
        sync_dir_required(&run)?;
        Ok(BlockClaimIndexStore {
            backing: BlockClaimIndexBacking::Standalone(Mutex::new(file)),
            counters: Arc::clone(&self.counters),
        })
    }

    /// Stable identity of the retained no-follow archive root capability.
    ///
    /// This is derived from the opened directory resource, never from an
    /// ambient path string, so two `ObjectStore` values opened over the same
    /// archive compare equal while a substituted directory does not.
    pub(crate) fn canonical_archive_identity(
        &self,
    ) -> Result<ControlDirectoryIdentity, StoreError> {
        control_directory_identity(&self.capability)
    }

    /// Authenticate the exact persisted canonical archive-resource claim
    /// retained inside this store's archive-root capability.
    ///
    /// This opens the already-enrolled archive-instance claim against the
    /// retained no-follow directory capability and confirms it derives to
    /// `expected`. It never derives, provisions, repairs, or overwrites the
    /// claim; a missing, substituted, or mismatched claim fails closed. The
    /// authenticated physical archive directory only proves its own control
    /// identity, so the persisted resource claim must be checked separately.
    pub(crate) fn validate_enrolled_archive_resource_id(
        &self,
        expected: super::CanonicalArchiveResourceId,
    ) -> std::io::Result<()> {
        super::CanonicalArchiveResourceId::open_enrolled_in_retained_directory(
            &self.capability,
            expected,
        )
        .map(|_| ())
    }

    /// Provision this store's canonical archive-resource claim exactly once and
    /// return its identity.
    ///
    /// The explicit local activation path uses this once to bind a newly
    /// created v2 archive to its enrollment. It goes through the same retained
    /// no-follow capability that
    /// [`Self::validate_enrolled_archive_resource_id`] later authenticates.
    pub(crate) fn provision_enrolled_archive_resource_id(
        &self,
    ) -> std::io::Result<super::CanonicalArchiveResourceId> {
        super::CanonicalArchiveResourceId::provision_in_retained_directory(&self.capability)
    }

    /// Publish or reopen the exact archive claim reserved in private
    /// application data before graph-local archive construction began.
    ///
    /// Publication uses the object store's immutable temp+sync+no-replace
    /// primitive. A crash may leave only a disposable temp, while retry always
    /// republishes the same canonical claim and refuses any different final
    /// claim instead of minting or adopting a replacement.
    pub(crate) fn provision_or_resume_local_activation_archive_resource_id(
        &self,
        instance_id: Uuid,
    ) -> Result<super::CanonicalArchiveResourceId, StoreError> {
        let claim = super::CanonicalArchiveResourceId::claim_bytes(instance_id)?;
        #[cfg(target_os = "android")]
        if self.lifecycle == ObjectStoreLifecycle::ReconstructibleActivation {
            match read_optional_regular(
                &self.capability,
                ARCHIVE_INSTANCE_CLAIM_FILE,
                claim.len() as u64,
                Some(claim.len() as u64),
            )? {
                Some(existing) if existing == claim => {}
                Some(_) => {
                    return Err(StoreError::ImmutableCollision(
                        "local activation archive claim",
                    ))
                }
                None => {
                    let mut options = OpenOptions::new();
                    options.write(true).create_new(true);
                    let mut file = self
                        .capability
                        .open_with(ARCHIVE_INSTANCE_CLAIM_FILE, &options)?
                        .into_std();
                    file.write_all(&claim)?;
                    crate::filesystem_durability::sync_reconstructible_file(&file)?;
                    drop(file);
                    let stored = read_required_regular(
                        &self.capability,
                        ARCHIVE_INSTANCE_CLAIM_FILE,
                        claim.len() as u64,
                        Some(claim.len() as u64),
                    )?;
                    if stored != claim {
                        return Err(StoreError::ImmutableCollision(
                            "local activation archive claim",
                        ));
                    }
                }
            }
            return super::CanonicalArchiveResourceId::open_exact_claim_in_retained_directory(
                &self.capability,
                &claim,
            )
            .map_err(StoreError::from);
        }
        publish_immutable_exact(
            &self.capability,
            ARCHIVE_INSTANCE_CLAIM_FILE,
            &claim,
            "local activation archive claim",
        )?;
        super::CanonicalArchiveResourceId::open_exact_claim_in_retained_directory(
            &self.capability,
            &claim,
        )
        .map_err(StoreError::from)
    }

    pub(crate) fn start_engine_scratch(
        &self,
    ) -> Result<
        (
            Arc<super::scratch_store::ScratchStore>,
            BlockClaimIndexStore,
        ),
        StoreError,
    > {
        let scratch = Arc::new(
            super::scratch_store::ScratchStore::open(&self.capability, self.workspace_id)
                .map_err(|error| StoreError::Scratch(error.to_string()))?,
        );
        Ok((
            Arc::clone(&scratch),
            self.engine_claim_index(Arc::clone(&scratch))?,
        ))
    }

    fn engine_claim_index(
        &self,
        scratch: Arc<super::scratch_store::ScratchStore>,
    ) -> Result<BlockClaimIndexStore, StoreError> {
        Ok(BlockClaimIndexStore {
            backing: BlockClaimIndexBacking::Scratch(scratch),
            counters: Arc::clone(&self.counters),
        })
    }

    /// Mint a fresh **retained** engine scratch pair beneath this archive.
    ///
    /// The only difference from [`Self::start_engine_scratch`] is the run's own
    /// durable retention marker, which makes the run survive its owner's death
    /// instead of being reclaimed as disposable sibling state. Because
    /// [`RetainedEngineScratch`] can be minted only here and by
    /// [`Self::adopt_retained_engine_scratch`], holding one is itself the proof
    /// that the run is retained — the engine never has to re-derive retention
    /// from an ambient marker read.
    pub(crate) fn create_retained_engine_scratch(
        &self,
    ) -> Result<RetainedEngineScratch, StoreError> {
        let scratch = super::scratch_store::ScratchStore::create_retained(
            &self.capability,
            self.workspace_id,
        )
        .map_err(|error| StoreError::Scratch(error.to_string()))?;
        RetainedEngineScratch::seal(self, scratch)
    }

    /// Mint a retained archive-local run containing the exact byte address
    /// space of one live detached scratch run.
    ///
    /// This is the one-way same-process bootstrap migration seam. The source
    /// remains owned and leased by its detached candidate, the destination is
    /// freshly created beneath this exact archive capability, and no caller can
    /// supply roots independently of the source bytes. The enrolled engine
    /// still has to restore and authenticate a `RuntimeResumeSnapshot` against
    /// durable history before these reconstructible bytes become usable.
    pub(crate) fn create_retained_engine_scratch_from(
        &self,
        source: &super::scratch_store::ScratchStore,
    ) -> Result<RetainedEngineScratch, StoreError> {
        match source.clone_retained_into(&self.capability) {
            Ok(retained) => RetainedEngineScratch::seal(self, retained),
            Err(copy_error) => self
                .adopt_retained_engine_scratch(
                    source.run_id(),
                    source
                        .binding_digest()
                        .map_err(|error| StoreError::Scratch(error.to_string()))?,
                )
                .map_err(|adoption_error| {
                    StoreError::Scratch(format!(
                        "retained scratch migration failed ({copy_error}); retry adoption failed ({adoption_error})"
                    ))
                }),
        }
    }

    /// Adopt exactly one already-published retained run.
    ///
    /// Four independent facts must hold before this returns, and every one of
    /// them is read from the run's own durable bytes rather than asserted by the
    /// caller:
    ///
    /// 1. the run directory is reachable no-follow under *this* archive
    ///    capability's scratch namespace, under the canonical `run-<uuid>`
    ///    spelling of `run_id`;
    /// 2. its own exclusive lease is acquired, so no live owner is mutating it;
    /// 3. its marker authenticates as schema-current, retained, owned by this
    ///    workspace, and carrying exactly `run_id`, with a complete regular
    ///    entry set — all inside [`ScratchStore::adopt_retained`];
    /// 4. its canonical marker digest equals `binding_digest`, which is what
    ///    catches a *re-created* run that reused the same UUID: the owner nonce
    ///    is fresh, so the digest cannot match.
    ///
    /// Any failure is returned as an ordinary error and **changes nothing**: no
    /// directory, marker, lease, or data file is created, truncated, or
    /// replaced, so the candidate run's bytes are exactly as they were. The
    /// caller's correct response is a fresh retained run plus a full replay,
    /// never a repair. Adoption authorizes reuse of reconstructible bytes and
    /// nothing else.
    pub(crate) fn adopt_retained_engine_scratch(
        &self,
        run_id: Uuid,
        binding_digest: ContentDigest,
    ) -> Result<RetainedEngineScratch, StoreError> {
        let scratch = super::scratch_store::ScratchStore::adopt_retained(
            &self.capability,
            self.workspace_id,
            run_id,
        )
        .map_err(|error| StoreError::Scratch(error.to_string()))?;
        let sealed = RetainedEngineScratch::seal(self, scratch)?;
        if sealed.run_id != run_id || sealed.binding_digest != binding_digest {
            return Err(StoreError::RetainedScratchBindingMismatch);
        }
        Ok(sealed)
    }

    pub(crate) fn open_logseq_claim_index(
        &self,
    ) -> Result<super::uuid_claim_index::LogseqClaimIndexStore, StoreError> {
        ensure_directory_for_lifecycle(&self.capability, LOGSEQ_CLAIM_INDEX_DIR, self.lifecycle)?;
        Ok(super::uuid_claim_index::LogseqClaimIndexStore::new(
            open_dir_nofollow(&self.capability, LOGSEQ_CLAIM_INDEX_DIR)?,
        ))
    }

    pub(crate) fn open_portable_path_index(
        &self,
    ) -> Result<super::portable_path_index::PortablePathIndexStore, StoreError> {
        ensure_directory_for_lifecycle(&self.capability, PORTABLE_PATH_INDEX_DIR, self.lifecycle)?;
        Ok(super::portable_path_index::PortablePathIndexStore::new(
            super::content_patricia::PatriciaIndexStore::new(open_dir_nofollow(
                &self.capability,
                PORTABLE_PATH_INDEX_DIR,
            )?),
        ))
    }

    pub(crate) fn open_page_name_ownership_index(
        &self,
    ) -> Result<super::page_name_index::PageNameOwnershipStore, StoreError> {
        ensure_directory_for_lifecycle(
            &self.capability,
            PAGE_NAME_OWNERSHIP_INDEX_DIR,
            self.lifecycle,
        )?;
        let index = open_dir_nofollow(&self.capability, PAGE_NAME_OWNERSHIP_INDEX_DIR)?;
        super::page_name_index::PageNameOwnershipStore::open(index)
    }

    pub(crate) fn open_reference_catalog(
        &self,
    ) -> Result<super::reference_catalog::ReferenceCatalogStore, StoreError> {
        ensure_directory_for_lifecycle(&self.capability, REFERENCE_CATALOG_DIR, self.lifecycle)?;
        let catalog = open_dir_nofollow(&self.capability, REFERENCE_CATALOG_DIR)?;
        for name in ["nodes", "postings"] {
            ensure_directory_for_lifecycle(&catalog, name, self.lifecycle)?;
        }
        Ok(super::reference_catalog::ReferenceCatalogStore::new(
            open_dir_nofollow(&catalog, "nodes")?,
            open_dir_nofollow(&catalog, "postings")?,
        ))
    }

    /// Mint the durable authenticated index capability detached bootstrap
    /// authoring and replay build their bound roots against.
    ///
    /// The bootstrap is authored detached from every runtime authority, but its
    /// accepted cold records bind this archive's authenticated portable-path,
    /// page-name, external UUID-claim, and reference-catalog roots. Handing
    /// authoring an explicit capability over *this* archive is what makes the
    /// promoted runtime later able to open the very roots its own bootstrap
    /// history names.
    pub(crate) fn bootstrap_authoring_capability(
        &self,
    ) -> Result<BootstrapAuthoringCapability, StoreError> {
        Ok(BootstrapAuthoringCapability {
            workspace_id: self.workspace_id,
            archive_identity: self.canonical_archive_identity()?,
            archive: Arc::new(self.capability.try_clone()?),
            reference_catalog: Arc::new(self.open_reference_catalog()?),
            portable_path_index: Arc::new(self.open_portable_path_index()?),
            logseq_claim_index: Arc::new(self.open_logseq_claim_index()?),
            page_name_index: Arc::new(self.open_page_name_ownership_index()?),
        })
    }

    #[cfg(test)]
    pub(crate) fn open_projection_work_index(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<super::ProjectionWorkIndex, StoreError> {
        self.preflight_projection_work_index(binding)?;
        let endpoint = binding.endpoint;
        ensure_directory_for_lifecycle(&self.capability, PROJECTION_WORK_DIR, self.lifecycle)?;
        let root = open_dir_nofollow(&self.capability, PROJECTION_WORK_DIR)?;
        let endpoint_name = endpoint.endpoint_id.to_string();
        ensure_directory_for_lifecycle(&root, &endpoint_name, self.lifecycle)?;
        let endpoint_dir = open_dir_nofollow(&root, &endpoint_name)?;
        for name in ["nodes", "roots", "prepared"] {
            ensure_directory_for_lifecycle(&endpoint_dir, name, self.lifecycle)?;
        }
        super::ProjectionWorkIndex::new(
            self.workspace_id,
            endpoint.endpoint_id,
            endpoint.graph_resource_id,
            binding.receipt_store_id,
            endpoint_dir.try_clone()?,
            open_dir_nofollow(&endpoint_dir, "nodes")?,
            open_dir_nofollow(&endpoint_dir, "roots")?,
            open_dir_nofollow(&endpoint_dir, "prepared")?,
        )
        .map_err(|error| StoreError::Scratch(error.to_string()))
    }

    fn open_absent_projection_work_index(
        &self,
        absence: AbsentControlName,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<super::ProjectionWorkIndex, StoreError> {
        let endpoint_dir = absence.claim(&self.capability, self.lifecycle)?;
        for name in ["nodes", "roots", "prepared"] {
            endpoint_dir.create_dir(name)?;
        }
        sync_directory_for_lifecycle(&endpoint_dir, self.lifecycle)?;
        super::ProjectionWorkIndex::new(
            self.workspace_id,
            binding.endpoint.endpoint_id,
            binding.endpoint.graph_resource_id,
            binding.receipt_store_id,
            endpoint_dir.try_clone()?,
            open_dir_nofollow(&endpoint_dir, "nodes")?,
            open_dir_nofollow(&endpoint_dir, "roots")?,
            open_dir_nofollow(&endpoint_dir, "prepared")?,
        )
        .map_err(|error| StoreError::Scratch(error.to_string()))
    }

    fn preflight_engine_history(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<(), StoreError> {
        let Some(histories) = open_existing_directory_for_lifecycle(
            &self.capability,
            ENGINE_HISTORY_DIR,
            self.lifecycle,
        )?
        else {
            return Ok(());
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) =
            open_existing_directory_for_lifecycle(&histories, &endpoint_name, self.lifecycle)?
        else {
            return Ok(());
        };
        let head = read_optional_regular(&control, ENGINE_HISTORY_HEAD_FILE, 64, None)?;
        let claim = read_optional_regular(&control, ENGINE_HISTORY_CLAIM_FILE, 256, None)?;
        match (head, claim) {
            (None, None) => Ok(()),
            (Some(head), Some(claim)) => {
                validate_engine_history_claim(
                    &claim,
                    self.workspace_id,
                    binding.endpoint.endpoint_id,
                    binding.endpoint.graph_resource_id,
                    binding.receipt_store_id,
                )?;
                let _nodes = open_existing_directory_for_lifecycle(
                    &control,
                    ENGINE_HISTORY_NODES_DIR,
                    self.lifecycle,
                )?
                .ok_or(StoreError::MalformedHistoryIndex)?;
                let roots = open_existing_directory_for_lifecycle(
                    &control,
                    ENGINE_HISTORY_ROOTS_DIR,
                    self.lifecycle,
                )?
                .ok_or(StoreError::MalformedHistoryIndex)?;
                let text =
                    std::str::from_utf8(&head).map_err(|_| StoreError::MalformedHistoryIndex)?;
                let digest = parse_digest(text)
                    .map(ContentDigest::from_bytes)
                    .map_err(|_| StoreError::MalformedHistoryIndex)?;
                if digest.to_string().as_bytes() != head {
                    return Err(StoreError::MalformedHistoryIndex);
                }
                let bytes = read_optional_regular(
                    &roots,
                    &engine_history_root_filename(digest),
                    MAX_ENGINE_HISTORY_INDEX_BYTES,
                    None,
                )?
                .ok_or(StoreError::MalformedHistoryIndex)?;
                if ContentDigest::of(&bytes) != digest {
                    return Err(StoreError::HistoryIndexPathMismatch(digest));
                }
                let root: DurableEngineHistoryRoot =
                    postcard::from_bytes(&bytes).map_err(|_| StoreError::MalformedHistoryIndex)?;
                if postcard::to_allocvec(&root).map_err(|_| StoreError::MalformedHistoryIndex)?
                    != bytes
                {
                    return Err(StoreError::MalformedHistoryIndex);
                }
                validate_engine_history_root(
                    &root,
                    self.workspace_id,
                    binding.endpoint.endpoint_id,
                    binding.endpoint.graph_resource_id,
                    binding.receipt_store_id,
                )
            }
            _ => Err(StoreError::MalformedHistoryIndex),
        }
    }

    #[cfg(test)]
    fn preflight_projection_work_index(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<(), StoreError> {
        let Some(root) = open_existing_directory_for_lifecycle(
            &self.capability,
            PROJECTION_WORK_DIR,
            self.lifecycle,
        )?
        else {
            return Ok(());
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) =
            open_existing_directory_for_lifecycle(&root, &endpoint_name, self.lifecycle)?
        else {
            return Ok(());
        };
        let head = read_optional_regular(&control, "projection-work.head", 64, None)?;
        let claim = read_optional_regular(&control, "projection-work.claim", 256, None)?;
        match (head, claim) {
            (None, None) => Ok(()),
            (Some(_), Some(_)) => super::ProjectionWorkIndex::preflight_existing(
                &control,
                self.workspace_id,
                binding.endpoint.endpoint_id,
                binding.endpoint.graph_resource_id,
                binding.receipt_store_id,
            )
            .map_err(|error| StoreError::Scratch(error.to_string())),
            _ => Err(StoreError::MalformedHistoryIndex),
        }
    }

    #[cfg(test)]
    fn preflight_enrolled_projection(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<(), StoreError> {
        self.preflight_engine_history(binding)?;
        self.preflight_projection_work_index(binding)
    }

    /// Enumerate all manifest commit markers in deterministic BatchId order.
    /// Staged manifests are included; readiness is determined by `inspect_batch`.
    pub fn committed_manifests(&self) -> Result<Vec<OperationBatch>, StoreError> {
        self.counters
            .directory_enumerations
            .fetch_add(1, Ordering::Relaxed);
        let batches = self.open_namespace(BATCHES_DIR)?;
        let mut manifests = Vec::new();
        for entry in batches.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| StoreError::MalformedPath("non-UTF-8 batch entry".into()))?;
            if is_temp_name(name) {
                require_regular_entry(&entry.file_type()?, name)?;
                continue;
            }
            require_regular_entry(&entry.file_type()?, name)?;
            let batch_id = parse_manifest_filename(name)?;
            let bytes = read_required_regular(&batches, name, MAX_MANIFEST_BYTES as u64, None)?;
            let manifest = OperationBatch::decode(&bytes)?;
            if manifest.batch_id() != batch_id {
                return Err(StoreError::ManifestPathMismatch {
                    expected: batch_id,
                    found: manifest.batch_id(),
                });
            }
            if manifest.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: manifest.workspace_id(),
                });
            }
            manifests.push(manifest);
        }
        manifests.sort_unstable_by_key(OperationBatch::batch_id);
        if let Some(first) = manifests.first() {
            for manifest in &manifests[1..] {
                if manifest.lineage_digest() != first.lineage_digest() {
                    return Err(StoreError::LineageMismatch {
                        expected: first.lineage_digest(),
                        found: manifest.lineage_digest(),
                    });
                }
            }
        }
        Ok(manifests)
    }

    /// Begin an incremental manifest enumeration without opening any manifest
    /// or object bytes.
    pub(crate) fn manifest_cursor(&self) -> Result<ObjectStoreManifestCursor, StoreError> {
        self.counters
            .directory_enumerations
            .fetch_add(1, Ordering::Relaxed);
        Ok(ObjectStoreManifestCursor {
            entries: self.open_namespace(BATCHES_DIR)?.entries()?,
        })
    }

    /// Visit at most one immutable manifest from a retained cursor.
    ///
    /// Temporary publication names are validated and skipped. A returned
    /// manifest is decoded and workspace-bound, but its objects are not opened.
    pub(crate) fn next_manifest(
        &self,
        cursor: &mut ObjectStoreManifestCursor,
    ) -> Result<Option<OperationBatch>, StoreError> {
        loop {
            let Some(entry) = cursor.entries.next() else {
                return Ok(None);
            };
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| StoreError::MalformedPath("non-UTF-8 batch entry".into()))?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            let batch_id = parse_manifest_filename(name)?;
            let bytes = self.read_manifest_bytes(batch_id)?;
            let manifest = OperationBatch::decode(&bytes)?;
            if manifest.batch_id() != batch_id {
                return Err(StoreError::ManifestPathMismatch {
                    expected: batch_id,
                    found: manifest.batch_id(),
                });
            }
            return Ok(Some(manifest));
        }
    }

    pub(crate) fn read_manifest_bytes(&self, batch_id: BatchId) -> Result<Vec<u8>, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let bytes = read_required_regular(
            &batches,
            &manifest_filename(batch_id),
            MAX_MANIFEST_BYTES as u64,
            None,
        )?;
        let manifest = OperationBatch::decode(&bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        Ok(bytes)
    }

    pub fn contains_object(&self, digest: ContentDigest) -> Result<bool, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let Some(bytes) = read_optional_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?
        else {
            return Ok(false);
        };
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(true)
    }

    /// Read and validate only a batch's manifest, without touching its objects.
    ///
    /// The manifest's descriptors already carry `document_id`, `kind`,
    /// `content_digest` and `encoded_byte_length` for every object, so a caller
    /// that needs object *metadata* -- which documents a batch updates, how many
    /// bytes it retains, which object holds a given kind -- never needs to read
    /// the objects themselves. Pair this with [`Self::read_object`] to fetch the
    /// one payload that is genuinely required.
    pub(crate) fn read_manifest(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<OperationBatch>, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        let Some(manifest_bytes) =
            read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)?
        else {
            return Ok(None);
        };
        self.counters
            .inspected_manifest_operations
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .inspected_manifest_bytes
            .fetch_add(manifest_bytes.len(), Ordering::Relaxed);
        let manifest = OperationBatch::decode(&manifest_bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        Ok(Some(manifest))
    }

    /// Read exactly the one content-addressed object a caller already has the
    /// digest for.
    ///
    /// This is the access path [`Self::inspect_batch`] is *not*: inspecting a
    /// batch to obtain a single object costs O(whole batch) — it reads,
    /// SHA-256s and decodes every object the manifest requires. Callers that
    /// hold a digest are asking for a file whose name already *is* that digest,
    /// so there is nothing to search and nothing to re-prove about the rest of
    /// the batch.
    ///
    /// The object's own integrity is still checked here (digest + workspace),
    /// because that is O(one object) and keeps the content-addressing contract.
    /// What is dropped is re-proving *batch completeness* per object, which is
    /// established once at acceptance: `hot_engine.rs:13120-13127` admits a
    /// batch to the archive only on `BatchInspection::Ready`, and projection
    /// work rows reach `Ready` only inside `accept_batch_at_history`.
    pub(crate) fn read_object(&self, digest: ContentDigest) -> Result<OperationObject, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        // Counted on the same counters as `inspect_batch`'s per-object reads.
        // These measure *object reads*, not one particular access path, and
        // tests use them as an oracle for how much an operation reconstructs
        // (`ordinary_drain_reconstructs_each_accepted_event_once`). A new path
        // that read objects silently would make that oracle lie.
        self.counters
            .inspected_object_operations
            .fetch_add(1, Ordering::Relaxed);
        let bytes = read_required_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?;
        self.counters
            .inspected_object_bytes
            .fetch_add(bytes.len(), Ordering::Relaxed);
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(object)
    }

    pub(crate) fn read_object_bytes(&self, digest: ContentDigest) -> Result<Vec<u8>, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let bytes = read_required_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?;
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(bytes)
    }

    fn validate_namespace(&self) -> Result<(), StoreError> {
        let mut manifests = Vec::new();
        for (directory, kind) in [
            (OBJECTS_DIR, NamespaceKind::Objects),
            (BATCHES_DIR, NamespaceKind::Batches),
        ] {
            self.counters
                .directory_enumerations
                .fetch_add(1, Ordering::Relaxed);
            let dir = self.open_namespace(directory)?;
            for entry in dir.entries()? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_str().ok_or_else(|| {
                    StoreError::MalformedPath(format!("non-UTF-8 entry under {directory}"))
                })?;
                require_regular_entry(&entry.file_type()?, name)?;
                if is_temp_name(name) {
                    let limit = match kind {
                        NamespaceKind::Objects => MAX_OBJECT_BYTES as u64,
                        NamespaceKind::Batches => MAX_MANIFEST_BYTES as u64,
                    };
                    read_required_regular(&dir, name, limit, None)?;
                    continue;
                }
                match kind {
                    NamespaceKind::Objects => {
                        let expected = parse_object_filename(name)?;
                        let bytes =
                            read_required_regular(&dir, name, MAX_OBJECT_BYTES as u64, None)?;
                        if ContentDigest::of(&bytes) != expected {
                            return Err(StoreError::ObjectPathMismatch(expected));
                        }
                        let object = OperationObject::decode(&bytes)?;
                        if object.workspace_id() != self.workspace_id {
                            return Err(StoreError::WorkspaceMismatch {
                                expected: self.workspace_id,
                                found: object.workspace_id(),
                            });
                        }
                        if object.encode()?.as_slice() != bytes {
                            return Err(StoreError::ObjectPathMismatch(expected));
                        }
                    }
                    NamespaceKind::Batches => {
                        let expected = parse_manifest_filename(name)?;
                        let bytes =
                            read_required_regular(&dir, name, MAX_MANIFEST_BYTES as u64, None)?;
                        let manifest = OperationBatch::decode(&bytes)?;
                        if manifest.batch_id() != expected {
                            return Err(StoreError::ManifestPathMismatch {
                                expected,
                                found: manifest.batch_id(),
                            });
                        }
                        if manifest.workspace_id() != self.workspace_id {
                            return Err(StoreError::WorkspaceMismatch {
                                expected: self.workspace_id,
                                found: manifest.workspace_id(),
                            });
                        }
                        manifests.push(manifest);
                    }
                }
            }
        }
        ensure_single_lineage(&manifests)?;
        if let Some(first) = manifests.first() {
            self.check_or_establish_lineage(first.lineage_digest())?;
        } else {
            let _ = read_optional_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?;
        }
        Ok(())
    }

    fn check_or_establish_lineage(&self, lineage: LineageDigest) -> Result<(), StoreError> {
        if let Some(bytes) =
            read_optional_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?
        {
            return require_lineage_bytes(lineage, &bytes);
        }
        match publish_immutable(
            &self.capability,
            LINEAGE_CLAIM_FILE,
            lineage.as_bytes(),
            Collision::Lineage(lineage),
        ) {
            Ok(()) => Ok(()),
            Err(StoreError::LineageClaimCollision(_)) => {
                let bytes =
                    read_required_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?;
                require_lineage_bytes(lineage, &bytes)
            }
            Err(error) => Err(error),
        }
    }

    fn require_lineage(&self, lineage: LineageDigest) -> Result<(), StoreError> {
        let bytes = read_required_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?;
        require_lineage_bytes(lineage, &bytes)
    }

    fn open_namespace(&self, name: &str) -> Result<Dir, StoreError> {
        let metadata = self.capability.symlink_metadata(name)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::UnsafeEntry(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        open_directory_for_lifecycle(&self.capability, name, self.lifecycle)
    }

    fn bootstrap_namespace(&self, name: &'static str, create: bool) -> Result<Dir, StoreError> {
        if create {
            ensure_directory_for_lifecycle(&self.capability, BOOTSTRAP_DIR, self.lifecycle)?;
        }
        let bootstrap =
            open_existing_directory_for_lifecycle(&self.capability, BOOTSTRAP_DIR, self.lifecycle)?
                .ok_or(StoreError::MissingBootstrapArtifact("bootstrap namespace"))?;
        if create {
            ensure_directory_for_lifecycle(&bootstrap, name, self.lifecycle)?;
        }
        open_existing_directory_for_lifecycle(&bootstrap, name, self.lifecycle)?
            .ok_or(StoreError::MissingBootstrapArtifact(name))
    }

    fn bootstrap_optional_namespace(&self, name: &str) -> Result<Option<Dir>, StoreError> {
        let Some(bootstrap) =
            open_existing_directory_for_lifecycle(&self.capability, BOOTSTRAP_DIR, self.lifecycle)?
        else {
            return Ok(None);
        };
        open_existing_directory_for_lifecycle(&bootstrap, name, self.lifecycle)
    }

    fn bootstrap_index_root_dir(
        &self,
        namespace: &'static str,
        root: &[u8; 32],
        create: bool,
    ) -> Result<Dir, StoreError> {
        let directory = self.bootstrap_namespace(namespace, create)?;
        let root_name = hex_bytes(root);
        if create {
            ensure_directory_for_lifecycle(&directory, &root_name, self.lifecycle)?;
        }
        open_existing_directory_for_lifecycle(&directory, &root_name, self.lifecycle)?
            .ok_or(StoreError::MissingBootstrapArtifact(namespace))
    }

    fn require_bootstrap_manifest(
        &self,
        descriptor: BootstrapPartDescriptorV1,
        manifest: &OperationBatch,
    ) -> Result<(), StoreError> {
        if manifest.origin() != BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap part origin",
            ));
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        if manifest.batch_id() != descriptor.batch_id() {
            return Err(StoreError::ManifestPathMismatch {
                expected: descriptor.batch_id(),
                found: manifest.batch_id(),
            });
        }
        Ok(())
    }

    fn require_bootstrap_aggregate_context(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
    ) -> Result<(), StoreError> {
        if aggregate.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: aggregate.workspace_id(),
            });
        }
        if let Some(bytes) =
            read_optional_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?
        {
            require_lineage_bytes(aggregate.lineage_digest(), &bytes)?;
        }
        Ok(())
    }

    fn validate_bootstrap_aggregate_artifacts(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
        verify_source_chunk_bytes: bool,
    ) -> Result<(), StoreError> {
        let aggregate_dir = self.bootstrap_namespace(BOOTSTRAP_AGGREGATES_DIR, false)?;
        let aggregate_name = hex_bytes(aggregate.aggregate_digest().as_bytes());
        let expected_aggregate = aggregate.encode()?;
        let stored_aggregate = read_required_regular(
            &aggregate_dir,
            &aggregate_name,
            MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES as u64,
            Some(expected_aggregate.len() as u64),
        )?;
        if stored_aggregate != expected_aggregate {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap aggregate bytes",
            ));
        }

        self.validate_bootstrap_source_indexes(aggregate, verify_source_chunk_bytes)?;
        for descriptor in aggregate.parts() {
            self.load_and_validate_bootstrap_part(aggregate, *descriptor)?;
        }
        Ok(())
    }

    fn validate_bootstrap_source_indexes(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
        verify_source_chunk_bytes: bool,
    ) -> Result<(), StoreError> {
        let inventory_root = aggregate.source_inventory_root();
        let inventory_pages = aggregate.source_inventory_page_count();
        let blob_root = aggregate.source_blob_root();
        let blob_pages = aggregate.source_blob_page_count();
        if inventory_pages == 0 {
            SourceInventoryIndexValidatorV1::new(inventory_root, 0)?.finish()?;
            SourceBlobIndexValidatorV1::new(blob_root, blob_pages)?.finish()?;
            if SourceBlobChunkRootBuilderV1::new().finish()? != blob_root {
                return Err(StoreError::BootstrapArtifactMismatch(
                    "empty source blob terminal coverage",
                ));
            }
            return Ok(());
        }
        let inventory_dir = self.bootstrap_index_root_dir(
            BOOTSTRAP_SOURCE_INVENTORY_DIR,
            inventory_root.digest(),
            false,
        )?;
        let mut inventory_validator =
            SourceInventoryIndexValidatorV1::new(inventory_root, inventory_pages)?;
        let mut scratch = BootstrapInventoryScratch::new(&self.capability)?;
        let mut runs = Vec::with_capacity(inventory_pages as usize);
        for ordinal in 0..inventory_pages {
            let bytes = read_required_regular(
                &inventory_dir,
                &bootstrap_page_filename(ordinal),
                MAX_SOURCE_INDEX_PAGE_BYTES as u64,
                None,
            )?;
            let page = SourceInventoryIndexPageV1::decode(&bytes)?;
            inventory_validator.push_page(&page)?;
            let mut leaves = page.entries().to_vec();
            leaves.sort_unstable_by_key(SourceLeafV1::digest);
            if leaves
                .windows(2)
                .any(|pair| pair[0].digest() == pair[1].digest())
            {
                return Err(StoreError::BootstrapArtifactMismatch(
                    "duplicate source leaf digest",
                ));
            }
            runs.push(scratch.write_run(&leaves)?);
        }
        inventory_validator.finish()?;
        let inventory_run = scratch.merge_all(runs)?;

        let blob_dir = if blob_pages == 0 {
            inventory_dir.try_clone()?
        } else {
            self.bootstrap_index_root_dir(BOOTSTRAP_SOURCE_BLOB_DIR, blob_root.digest(), false)?
        };
        let source_chunks = if verify_source_chunk_bytes && blob_root.chunk_count() != 0 {
            Some(self.bootstrap_namespace(BOOTSTRAP_SOURCE_CHUNKS_DIR, false)?)
        } else {
            None
        };
        let mut blob_cursor =
            BootstrapBlobCursor::new(blob_dir, source_chunks, blob_root, blob_pages)?;
        let mut source_builder = SourceBlobChunkRootBuilderV1::new();
        let mut inventory_reader = inventory_run
            .as_deref()
            .map(|name| BootstrapLeafRunReader::open(&scratch.dir, name))
            .transpose()?;
        while let Some(source) = inventory_reader
            .as_mut()
            .map(BootstrapLeafRunReader::next_leaf)
            .transpose()?
            .flatten()
        {
            source_builder.begin_source(&source)?;
            while blob_cursor
                .peek()?
                .is_some_and(|descriptor| descriptor.source_leaf() == source.digest())
            {
                source_builder.push(
                    blob_cursor
                        .next()?
                        .expect("peeked bootstrap blob descriptor remains present"),
                )?;
            }
            if blob_cursor
                .peek()?
                .is_some_and(|descriptor| descriptor.source_leaf() < source.digest())
            {
                return Err(StoreError::BootstrapArtifactMismatch(
                    "source blob descriptor has no inventory leaf",
                ));
            }
        }
        if blob_cursor.next()?.is_some() {
            return Err(StoreError::BootstrapArtifactMismatch(
                "source blob descriptor has no inventory leaf",
            ));
        }
        blob_cursor.finish()?;
        if source_builder.finish()? != blob_root {
            return Err(StoreError::BootstrapArtifactMismatch(
                "source blob terminal coverage",
            ));
        }
        Ok(())
    }

    fn load_and_validate_bootstrap_part(
        &self,
        aggregate: &BootstrapAggregateManifestV1,
        descriptor: BootstrapPartDescriptorV1,
    ) -> Result<LoadedBootstrapPartV1, StoreError> {
        let part_name = hex_bytes(descriptor.part_id().as_bytes());
        let parts = self.bootstrap_namespace(BOOTSTRAP_PARTS_DIR, false)?;
        let manifest_bytes =
            read_required_regular(&parts, &part_name, MAX_MANIFEST_BYTES as u64, None)?;
        let manifest = OperationBatch::decode(&manifest_bytes)?;
        self.require_bootstrap_manifest(descriptor, &manifest)?;
        if manifest.lineage_digest() != aggregate.lineage_digest() {
            return Err(StoreError::LineageMismatch {
                expected: aggregate.lineage_digest(),
                found: manifest.lineage_digest(),
            });
        }

        let evidence = descriptor.evidence();
        let evidence_name = hex_bytes(evidence.evidence_digest().as_bytes());
        let evidence_dir = self.bootstrap_namespace(BOOTSTRAP_EVIDENCE_DIR, false)?;
        let evidence_bytes = read_required_regular(
            &evidence_dir,
            &evidence_name,
            MAX_BOOTSTRAP_PART_EVIDENCE_BYTES as u64,
            None,
        )?;
        let loaded_evidence = BootstrapImportPartEvidenceV1::decode(&evidence_bytes)?;
        if loaded_evidence != evidence {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap part evidence",
            ));
        }

        let span_dir = self.bootstrap_namespace(BOOTSTRAP_PART_SPANS_DIR, false)?;
        let span_bytes = read_required_regular(
            &span_dir,
            &part_name,
            MAX_PART_SPAN_INDEX_BYTES as u64,
            None,
        )?;
        let spans = BootstrapPartSpanIndexV1::decode(&span_bytes)?;
        spans.validate_part(evidence)?;

        let pack_dir = self.bootstrap_namespace(BOOTSTRAP_PART_PACKS_DIR, false)?;
        let pack = open_file_nofollow(&pack_dir, &part_name)?;
        if pack.metadata()?.len() > MAX_BOOTSTRAP_PART_PACK_BYTES {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap part object pack exceeds its bound",
            ));
        }
        let mut pack = BufReader::with_capacity(64 * 1024, pack);
        let mut objects = Vec::with_capacity(manifest.required_objects().len());
        let mut payloads = Vec::with_capacity(manifest.required_objects().len());
        for expected in manifest.required_objects() {
            let mut length = [0; 4];
            pack.read_exact(&mut length)?;
            let length = u64::from(u32::from_be_bytes(length));
            if length != expected.encoded_byte_length() || length > MAX_OBJECT_BYTES as u64 {
                return Err(StoreError::BootstrapArtifactMismatch(
                    "bootstrap part object pack frame length",
                ));
            }
            let mut bytes = vec![0; length as usize];
            pack.read_exact(&mut bytes)?;
            if ContentDigest::of(&bytes) != expected.content_digest() {
                return Err(StoreError::ObjectPathMismatch(expected.content_digest()));
            }
            let object = OperationObject::decode(&bytes)?;
            if object.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: object.workspace_id(),
                });
            }
            if object.descriptor()? != *expected {
                return Err(StoreError::BootstrapArtifactMismatch(
                    "bootstrap operation object descriptor",
                ));
            }
            payloads.push(PayloadObjectDescriptorV1::new(
                expected.content_digest(),
                expected.encoded_byte_length(),
            )?);
            objects.push(object);
        }
        let mut trailing = [0; 1];
        if pack.read(&mut trailing)? != 0 {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap part object pack has trailing bytes",
            ));
        }
        let manifest_fingerprint = BootstrapManifestFingerprintV1::from_bytes(
            *ContentDigest::of(&manifest_bytes).as_bytes(),
        );
        let manifest_defined = [FullObjectDescriptorV1::manifest_defined(
            *ContentDigest::of(&span_bytes).as_bytes(),
            span_bytes.len() as u64,
        )?];
        descriptor.validate_loaded_artifacts(manifest_fingerprint, &payloads, &manifest_defined)?;
        Ok(LoadedBootstrapPartV1 {
            manifest,
            objects,
            spans,
        })
    }

    fn inspect_bootstrap_aggregate_inner(
        &self,
        expected: &BootstrapAggregateManifestV1,
    ) -> Result<BootstrapPublicationInspectionV1, StoreError> {
        self.require_bootstrap_aggregate_context(expected)?;
        let publication_id = expected.publication_id();
        if let Some(commits) = self.bootstrap_optional_namespace(BOOTSTRAP_COMMITS_DIR)? {
            let name = hex_bytes(publication_id.as_bytes());
            if read_optional_regular(
                &commits,
                &name,
                MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES as u64,
                None,
            )?
            .is_some()
            {
                let publication = self.load_bootstrap_publication(publication_id)?;
                if publication.aggregate() != expected {
                    return Err(StoreError::BootstrapArtifactMismatch(
                        "committed bootstrap aggregate",
                    ));
                }
                return Ok(BootstrapPublicationInspectionV1::Committed(publication));
            }
        }
        let Some(aggregates) = self.bootstrap_optional_namespace(BOOTSTRAP_AGGREGATES_DIR)? else {
            return Ok(BootstrapPublicationInspectionV1::Absent);
        };
        let name = hex_bytes(expected.aggregate_digest().as_bytes());
        let Some(bytes) = read_optional_regular(
            &aggregates,
            &name,
            MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES as u64,
            None,
        )?
        else {
            return Ok(BootstrapPublicationInspectionV1::Absent);
        };
        let found = BootstrapAggregateManifestV1::decode(&bytes)?;
        if found != *expected {
            return Err(StoreError::BootstrapArtifactMismatch(
                "pending bootstrap aggregate",
            ));
        }
        Ok(BootstrapPublicationInspectionV1::Pending)
    }
}

const BOOTSTRAP_INVENTORY_MERGE_FAN_IN: usize = 32;

struct BootstrapInventoryScratch {
    dir: Dir,
    names: Vec<String>,
}

impl BootstrapInventoryScratch {
    fn new(dir: &Dir) -> Result<Self, StoreError> {
        Ok(Self {
            dir: dir.try_clone()?,
            names: Vec::new(),
        })
    }

    fn create_run(&mut self) -> Result<(String, BufWriter<fs::File>), StoreError> {
        let name = format!(".tmp-bootstrap-inventory-{}", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let file = self.dir.open_with(&name, &options)?.into_std();
        self.names.push(name.clone());
        Ok((name, BufWriter::new(file)))
    }

    fn write_run(&mut self, leaves: &[SourceLeafV1]) -> Result<String, StoreError> {
        let (name, mut writer) = self.create_run()?;
        for leaf in leaves {
            write_bootstrap_leaf_record(&mut writer, leaf)?;
        }
        writer.flush()?;
        Ok(name)
    }

    fn merge_all(&mut self, mut runs: Vec<String>) -> Result<Option<String>, StoreError> {
        while runs.len() > 1 {
            let mut next =
                Vec::with_capacity(runs.len().div_ceil(BOOTSTRAP_INVENTORY_MERGE_FAN_IN));
            for group in runs.chunks(BOOTSTRAP_INVENTORY_MERGE_FAN_IN) {
                next.push(self.merge_group(group)?);
            }
            runs = next;
        }
        Ok(runs.pop())
    }

    fn merge_group(&mut self, runs: &[String]) -> Result<String, StoreError> {
        let mut readers = runs
            .iter()
            .map(|name| BootstrapLeafRunReader::open(&self.dir, name))
            .collect::<Result<Vec<_>, _>>()?;
        let mut heads = readers
            .iter_mut()
            .map(BootstrapLeafRunReader::next_leaf)
            .collect::<Result<Vec<_>, _>>()?;
        let mut heap = BinaryHeap::new();
        for (index, leaf) in heads.iter().enumerate() {
            if let Some(leaf) = leaf {
                heap.push(Reverse((*leaf.digest().as_bytes(), index)));
            }
        }
        let (name, mut writer) = self.create_run()?;
        let mut last = None;
        while let Some(Reverse((digest, index))) = heap.pop() {
            if last == Some(digest) {
                return Err(StoreError::BootstrapArtifactMismatch(
                    "duplicate source leaf digest",
                ));
            }
            let leaf = heads[index]
                .take()
                .ok_or(StoreError::BootstrapArtifactMismatch(
                    "inventory merge head",
                ))?;
            write_bootstrap_leaf_record(&mut writer, &leaf)?;
            last = Some(digest);
            heads[index] = readers[index].next_leaf()?;
            if let Some(next) = &heads[index] {
                heap.push(Reverse((*next.digest().as_bytes(), index)));
            }
        }
        writer.flush()?;
        Ok(name)
    }
}

impl Drop for BootstrapInventoryScratch {
    fn drop(&mut self) {
        for name in &self.names {
            let _ = self.dir.remove_file(name);
        }
    }
}

struct BootstrapLeafRunReader {
    reader: BufReader<fs::File>,
}

impl BootstrapLeafRunReader {
    fn open(dir: &Dir, name: &str) -> Result<Self, StoreError> {
        let file = read_regular_file_nofollow(dir, name)?;
        Ok(Self {
            reader: BufReader::new(file),
        })
    }

    fn next_leaf(&mut self) -> Result<Option<SourceLeafV1>, StoreError> {
        let mut length = [0_u8; 4];
        match self.reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(1) => self.reader.read_exact(&mut length[1..])?,
            Ok(_) => unreachable!("one-byte bootstrap run probe"),
            Err(error) => return Err(error.into()),
        }
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_SOURCE_INDEX_PAGE_BYTES {
            return Err(StoreError::BootstrapArtifactMismatch(
                "inventory scratch record length",
            ));
        }
        let mut bytes = vec![0; length];
        self.reader.read_exact(&mut bytes)?;
        Ok(Some(SourceLeafV1::decode(&bytes)?))
    }
}

fn write_bootstrap_leaf_record(
    writer: &mut impl Write,
    leaf: &SourceLeafV1,
) -> Result<(), StoreError> {
    let bytes = leaf.encode();
    let length = u32::try_from(bytes.len())
        .map_err(|_| StoreError::BootstrapArtifactMismatch("inventory scratch record length"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&bytes)?;
    Ok(())
}

struct BootstrapBlobCursor {
    pages: Dir,
    chunks: Option<Dir>,
    validator: Option<SourceBlobIndexValidatorV1>,
    expected_pages: u32,
    next_page: u32,
    entries: Vec<SourceBlobChunkDescriptorV1>,
    next_entry: usize,
}

impl BootstrapBlobCursor {
    fn new(
        pages: Dir,
        chunks: Option<Dir>,
        root: SourceBlobChunkRootV1,
        expected_pages: u32,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            pages,
            chunks,
            validator: Some(SourceBlobIndexValidatorV1::new(root, expected_pages)?),
            expected_pages,
            next_page: 0,
            entries: Vec::new(),
            next_entry: 0,
        })
    }

    fn peek(&mut self) -> Result<Option<SourceBlobChunkDescriptorV1>, StoreError> {
        self.fill()?;
        Ok(self.entries.get(self.next_entry).copied())
    }

    fn next(&mut self) -> Result<Option<SourceBlobChunkDescriptorV1>, StoreError> {
        self.fill()?;
        let value = self.entries.get(self.next_entry).copied();
        if value.is_some() {
            self.next_entry += 1;
        }
        Ok(value)
    }

    fn fill(&mut self) -> Result<(), StoreError> {
        while self.next_entry == self.entries.len() && self.next_page < self.expected_pages {
            let bytes = read_required_regular(
                &self.pages,
                &bootstrap_page_filename(self.next_page),
                MAX_SOURCE_INDEX_PAGE_BYTES as u64,
                None,
            )?;
            let page = SourceBlobIndexPageV1::decode(&bytes)?;
            self.validator
                .as_mut()
                .ok_or(StoreError::BootstrapArtifactMismatch(
                    "finished source blob validator",
                ))?
                .push_page(&page)?;
            self.entries = page.entries().to_vec();
            self.next_entry = 0;
            self.next_page += 1;
            if let Some(chunks) = &self.chunks {
                for descriptor in &self.entries {
                    let name = hex_bytes(descriptor.content_digest().as_bytes());
                    let chunk = read_required_regular(
                        chunks,
                        &name,
                        MAX_SOURCE_BLOB_CHUNK_BYTES as u64,
                        Some(u64::from(descriptor.byte_length())),
                    )?;
                    if ContentDigest::of(&chunk).as_bytes()
                        != descriptor.content_digest().as_bytes()
                    {
                        return Err(StoreError::BootstrapArtifactMismatch(
                            "source chunk content digest",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), StoreError> {
        self.fill()?;
        if self.next_entry != self.entries.len() || self.next_page != self.expected_pages {
            return Err(StoreError::BootstrapArtifactMismatch(
                "source blob index cursor",
            ));
        }
        self.validator
            .take()
            .ok_or(StoreError::BootstrapArtifactMismatch(
                "finished source blob validator",
            ))?
            .finish()?;
        Ok(())
    }
}

impl EnrolledProjectionOpen {
    pub(crate) const fn binding(&self) -> super::hot_engine::ProjectionStorageBinding {
        self.binding
    }

    pub(crate) fn into_runtime(
        mut self,
    ) -> Result<
        (
            ObjectStore,
            DurableEngineHistoryStore,
            super::ProjectionWorkIndex,
        ),
        (ObjectStore, StoreError),
    > {
        enrolled_open_use_hook();
        let validation = (|| {
            match self
                .history
                .as_ref()
                .expect("sealed history control is present")
            {
                SealedControl::Existing(history) => history.validate_sealed_open()?,
                SealedControl::Absent(_) => {}
            }
            match self.work.as_ref().expect("sealed work control is present") {
                SealedControl::Existing(work) => work
                    .validate_sealed_open()
                    .map_err(|error| StoreError::Scratch(error.to_string())),
                SealedControl::Absent(_) => Ok(()),
            }
        })();
        if let Err(error) = validation {
            return Err((self.store.take().expect("sealed store is present"), error));
        }
        enrolled_open_act_hook();

        let store = self.store.take().expect("sealed store is present");
        let post_hook_validation = (|| {
            match self
                .history
                .as_ref()
                .expect("sealed history control is present")
            {
                SealedControl::Existing(history) => history.validate_sealed_open()?,
                SealedControl::Absent(absence) => {
                    absence.validate_still_absent(&store.capability)?
                }
            }
            match self.work.as_ref().expect("sealed work control is present") {
                SealedControl::Existing(work) => work
                    .validate_sealed_open()
                    .map_err(|error| StoreError::Scratch(error.to_string())),
                SealedControl::Absent(absence) => absence.validate_still_absent(&store.capability),
            }
        })();
        if let Err(error) = post_hook_validation {
            return Err((store, error));
        }
        let history = match self
            .history
            .take()
            .expect("sealed history control is present")
        {
            SealedControl::Existing(history) => history,
            SealedControl::Absent(absence) => {
                match store.open_absent_engine_history(absence, self.binding) {
                    Ok(history) => history,
                    Err(error) => return Err((store, error)),
                }
            }
        };
        let work = match self.work.take().expect("sealed work control is present") {
            SealedControl::Existing(work) => work,
            SealedControl::Absent(absence) => {
                match store.open_absent_projection_work_index(absence, self.binding) {
                    Ok(work) => work,
                    Err(error) => return Err((store, error)),
                }
            }
        };
        Ok((store, history, work))
    }
}

impl HistoryOnlyOpen {
    pub(crate) const fn binding(&self) -> super::hot_engine::ProjectionStorageBinding {
        self.binding
    }

    pub(crate) fn into_history(
        mut self,
    ) -> Result<(ObjectStore, DurableEngineHistoryStore), (ObjectStore, StoreError)> {
        enrolled_open_use_hook();
        if let SealedControl::Existing(history) = self
            .history
            .as_ref()
            .expect("sealed history control is present")
        {
            if let Err(error) = history.validate_sealed_open() {
                return Err((self.store.take().expect("sealed store is present"), error));
            }
        }
        enrolled_open_act_hook();

        let store = self.store.take().expect("sealed store is present");
        let validation = match self
            .history
            .as_ref()
            .expect("sealed history control is present")
        {
            SealedControl::Existing(history) => history.validate_sealed_open(),
            SealedControl::Absent(absence) => absence.validate_still_absent(&store.capability),
        };
        if let Err(error) = validation {
            return Err((store, error));
        }
        let history = match self
            .history
            .take()
            .expect("sealed history control is present")
        {
            SealedControl::Existing(history) => history,
            SealedControl::Absent(absence) => {
                match store.open_absent_engine_history(absence, self.binding) {
                    Ok(history) => history,
                    Err(error) => return Err((store, error)),
                }
            }
        };
        Ok((store, history))
    }
}

impl<T> SealedControl<T> {
    fn bind_absent_parent(
        &mut self,
        store_root: &Dir,
        lifecycle: ObjectStoreLifecycle,
    ) -> Result<bool, StoreError> {
        let Self::Absent(absence) = self else {
            return Ok(false);
        };
        if absence.namespace.is_some() {
            return Ok(false);
        }
        store_root
            .create_dir(absence.namespace_name)
            .map_err(|error| {
                if error.kind() == ErrorKind::AlreadyExists {
                    StoreError::UnsafeEntry(format!(
                        "formerly absent {} was created while enrolled open was sealed",
                        absence.namespace_name
                    ))
                } else {
                    error.into()
                }
            })?;
        sync_directory_for_lifecycle(store_root, lifecycle)?;
        let namespace =
            open_directory_for_lifecycle(store_root, absence.namespace_name, lifecycle)?;
        absence.namespace_identity = Some(control_directory_identity(&namespace)?);
        absence.namespace = Some(namespace);
        Ok(true)
    }

    fn release_empty_parent(&mut self, store_root: &Dir) {
        let Self::Absent(absence) = self else {
            return;
        };
        let Some(namespace) = &absence.namespace else {
            return;
        };
        let Some(expected) = absence.namespace_identity else {
            return;
        };
        let is_unchanged_empty = control_directory_identity(namespace).ok() == Some(expected)
            && namespace
                .entries()
                .ok()
                .is_some_and(|mut entries| entries.next().is_none());
        if is_unchanged_empty {
            let _ = store_root.remove_dir(absence.namespace_name);
            let _ = sync_dir_required(store_root);
        }
    }
}

impl AbsentControlName {
    fn validate_still_absent(&self, store_root: &Dir) -> Result<(), StoreError> {
        let parent = match &self.namespace {
            Some(namespace) => {
                let live = open_existing_dir_nofollow(store_root, self.namespace_name)?
                    .ok_or_else(|| {
                        StoreError::UnsafeEntry(format!(
                            "enrolled-open parent {} disappeared",
                            self.namespace_name
                        ))
                    })?;
                let expected = self.namespace_identity.ok_or_else(|| {
                    StoreError::UnsafeEntry(format!(
                        "enrolled-open parent {} has no sealed identity",
                        self.namespace_name
                    ))
                })?;
                if control_directory_identity(&live)? != expected
                    || control_directory_identity(namespace)? != expected
                {
                    return Err(StoreError::UnsafeEntry(format!(
                        "enrolled-open parent {} was substituted",
                        self.namespace_name
                    )));
                }
                namespace
            }
            None => {
                return match store_root.symlink_metadata(self.namespace_name) {
                    Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                    Ok(_) => Err(StoreError::UnsafeEntry(format!(
                        "formerly absent {} was created before enrolled open consumed it",
                        self.namespace_name
                    ))),
                    Err(error) => Err(error.into()),
                };
            }
        };
        match parent.symlink_metadata(&self.endpoint_name) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(StoreError::UnsafeEntry(format!(
                "formerly absent {}/{} was created before enrolled open consumed it",
                self.namespace_name, self.endpoint_name
            ))),
            Err(error) => Err(error.into()),
        }
    }

    fn claim(self, store_root: &Dir, lifecycle: ObjectStoreLifecycle) -> Result<Dir, StoreError> {
        self.validate_still_absent(store_root)?;
        let namespace = match self.namespace {
            Some(namespace) => namespace,
            None => {
                store_root
                    .create_dir(self.namespace_name)
                    .map_err(|error| {
                        if error.kind() == ErrorKind::AlreadyExists {
                            StoreError::UnsafeEntry(format!(
                                "formerly absent {} was created before enrolled open consumed it",
                                self.namespace_name
                            ))
                        } else {
                            error.into()
                        }
                    })?;
                sync_directory_for_lifecycle(store_root, lifecycle)?;
                open_directory_for_lifecycle(store_root, self.namespace_name, lifecycle)?
            }
        };
        namespace.create_dir(&self.endpoint_name).map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                StoreError::UnsafeEntry(format!(
                    "formerly absent {}/{} was created before enrolled open consumed it",
                    self.namespace_name, self.endpoint_name
                ))
            } else {
                error.into()
            }
        })?;
        sync_directory_for_lifecycle(&namespace, lifecycle)?;
        open_directory_for_lifecycle(&namespace, &self.endpoint_name, lifecycle)
    }
}

impl StoreCounters {
    fn snapshot(&self) -> ObjectStoreStats {
        ObjectStoreStats {
            directory_enumerations: self.directory_enumerations.load(Ordering::Relaxed),
            accepted_manifest_reads: self.accepted_manifest_reads.load(Ordering::Relaxed),
            accepted_object_reads: self.accepted_object_reads.load(Ordering::Relaxed),
            dag_manifest_reads: self.dag_manifest_reads.load(Ordering::Relaxed),
            history_record_reads: self.history_record_reads.load(Ordering::Relaxed),
            history_index_reads: self.history_index_reads.load(Ordering::Relaxed),
            history_index_writes: self.history_index_writes.load(Ordering::Relaxed),
            history_decodes: self.history_decodes.load(Ordering::Relaxed),
            block_claim_index_reads: self.block_claim_index_reads.load(Ordering::Relaxed),
            block_claim_index_writes: self.block_claim_index_writes.load(Ordering::Relaxed),
            block_claim_index_syncs: self.block_claim_index_syncs.load(Ordering::Relaxed),
            inspected_manifest_operations: self
                .inspected_manifest_operations
                .load(Ordering::Relaxed),
            inspected_manifest_bytes: self.inspected_manifest_bytes.load(Ordering::Relaxed),
            inspected_object_operations: self.inspected_object_operations.load(Ordering::Relaxed),
            inspected_object_bytes: self.inspected_object_bytes.load(Ordering::Relaxed),
        }
    }
}

impl EngineHistoryStore {
    pub(crate) fn empty_root() -> ContentDigest {
        ContentDigest::of(b"tine/oplog-engine-history/radix-v1/empty")
    }

    pub(crate) fn lookup(
        &self,
        root: ContentDigest,
        batch_id: BatchId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        if root == Self::empty_root() {
            return Ok(None);
        }
        let batch_uuid = batch_id.as_uuid();
        let key = batch_uuid.as_bytes();
        let mut digest = root;
        for depth in 0..=ENGINE_HISTORY_RADIX_DEPTH {
            match self.read_node(digest)? {
                HistoryIndexNode::Branch {
                    depth: found_depth,
                    children,
                    ..
                } => {
                    if depth >= ENGINE_HISTORY_RADIX_DEPTH || found_depth != depth {
                        return Err(StoreError::MalformedHistoryIndex);
                    }
                    let nibble = history_key_nibble(key, depth);
                    let Some((_, child)) =
                        children.iter().find(|(candidate, _)| *candidate == nibble)
                    else {
                        return Ok(None);
                    };
                    digest = *child;
                }
                HistoryIndexNode::Leaf {
                    batch_id: found,
                    record,
                    ..
                } => {
                    if depth != ENGINE_HISTORY_RADIX_DEPTH || found != batch_id {
                        return Err(StoreError::MalformedHistoryIndex);
                    }
                    return Ok(Some(record));
                }
            }
        }
        Err(StoreError::MalformedHistoryIndex)
    }

    pub(crate) fn insert(
        &self,
        root: ContentDigest,
        batch_id: BatchId,
        bytes: &[u8],
    ) -> Result<ContentDigest, StoreError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_ENGINE_HISTORY_RECORD_BYTES {
            return Err(StoreError::StoredFileTooLarge {
                path: history_filename(batch_id),
                length: bytes.len() as u64,
                limit: MAX_ENGINE_HISTORY_RECORD_BYTES,
            });
        }
        self.insert_at(root, batch_id, bytes, 0)
    }

    pub(crate) fn materialize(
        &self,
        root: ContentDigest,
    ) -> Result<Vec<(BatchId, Vec<u8>)>, StoreError> {
        if root == Self::empty_root() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        let mut pending = vec![(root, 0_u8)];
        while let Some((digest, expected_depth)) = pending.pop() {
            match self.read_node(digest)? {
                HistoryIndexNode::Branch {
                    depth, children, ..
                } => {
                    if depth != expected_depth || depth >= ENGINE_HISTORY_RADIX_DEPTH {
                        return Err(StoreError::MalformedHistoryIndex);
                    }
                    pending.extend(
                        children
                            .into_iter()
                            .rev()
                            .map(|(_, child)| (child, depth + 1)),
                    );
                }
                HistoryIndexNode::Leaf {
                    batch_id, record, ..
                } => {
                    if expected_depth != ENGINE_HISTORY_RADIX_DEPTH {
                        return Err(StoreError::MalformedHistoryIndex);
                    }
                    records.push((batch_id, record));
                }
            }
        }
        records.sort_unstable_by_key(|(batch_id, _)| *batch_id);
        Ok(records)
    }

    pub(crate) fn note_history_decode(&self) {
        self.counters
            .history_decodes
            .fetch_add(1, Ordering::Relaxed);
    }

    fn insert_at(
        &self,
        root: ContentDigest,
        batch_id: BatchId,
        record: &[u8],
        depth: u8,
    ) -> Result<ContentDigest, StoreError> {
        if depth == ENGINE_HISTORY_RADIX_DEPTH {
            if root != Self::empty_root() {
                match self.read_node(root)? {
                    HistoryIndexNode::Leaf {
                        batch_id: existing_batch,
                        record: existing_record,
                        ..
                    } if existing_batch == batch_id && existing_record == record => {
                        return Ok(root);
                    }
                    _ => return Err(StoreError::HistoryIndexCollision(batch_id)),
                }
            }
            return self.publish_node(&HistoryIndexNode::Leaf {
                schema_version: ENGINE_HISTORY_INDEX_SCHEMA_VERSION,
                batch_id,
                record: record.to_vec(),
            });
        }

        let mut children = if root == Self::empty_root() {
            Vec::new()
        } else {
            match self.read_node(root)? {
                HistoryIndexNode::Branch {
                    depth: found_depth,
                    children,
                    ..
                } if found_depth == depth => children,
                _ => return Err(StoreError::MalformedHistoryIndex),
            }
        };
        let nibble = history_key_nibble(batch_id.as_uuid().as_bytes(), depth);
        let existing_child = children
            .iter()
            .find(|(candidate, _)| *candidate == nibble)
            .map(|(_, digest)| *digest)
            .unwrap_or_else(Self::empty_root);
        let child = self.insert_at(existing_child, batch_id, record, depth + 1)?;
        match children.binary_search_by_key(&nibble, |(candidate, _)| *candidate) {
            Ok(index) => children[index].1 = child,
            Err(index) => children.insert(index, (nibble, child)),
        }
        self.publish_node(&HistoryIndexNode::Branch {
            schema_version: ENGINE_HISTORY_INDEX_SCHEMA_VERSION,
            depth,
            children,
        })
    }

    /// Latch [`Self::storage_fault`]. Monotone, so a plain store is enough and
    /// the observation can never be lost by racing with another latch.
    fn note_storage_fault(&self) {
        self.storage_fault.store(true, Ordering::SeqCst);
    }

    fn storage_faulted(&self) -> bool {
        self.storage_fault.load(Ordering::SeqCst)
    }

    fn publish_node(&self, node: &HistoryIndexNode) -> Result<ContentDigest, StoreError> {
        self.publish_node_checked(node)
            .inspect_err(|_| self.note_storage_fault())
    }

    fn publish_node_checked(&self, node: &HistoryIndexNode) -> Result<ContentDigest, StoreError> {
        validate_history_node(node)?;
        let bytes = postcard::to_allocvec(node).map_err(|_| StoreError::MalformedHistoryIndex)?;
        if bytes.len() as u64 > MAX_ENGINE_HISTORY_INDEX_BYTES {
            return Err(StoreError::StoredFileTooLarge {
                path: "engine history index node".into(),
                length: bytes.len() as u64,
                limit: MAX_ENGINE_HISTORY_INDEX_BYTES,
            });
        }
        let digest = ContentDigest::of(&bytes);
        self.counters
            .history_index_writes
            .fetch_add(1, Ordering::Relaxed);
        publish_immutable(
            &self.capability,
            &history_index_filename(digest),
            &bytes,
            Collision::HistoryIndex(digest),
        )?;
        Ok(digest)
    }

    /// Read one immutable index node.
    ///
    /// Every failure here is a durable storage fault — the node is missing,
    /// oversized, stored under the wrong content address, undecodable,
    /// non-canonical or structurally invalid — so every failure latches
    /// [`Self::storage_fault`]. Structural *lineage* rejections are decided by
    /// the callers from successfully read nodes and never reach this latch.
    fn read_node(&self, digest: ContentDigest) -> Result<HistoryIndexNode, StoreError> {
        self.read_node_checked(digest)
            .inspect_err(|_| self.note_storage_fault())
    }

    fn read_node_checked(&self, digest: ContentDigest) -> Result<HistoryIndexNode, StoreError> {
        self.counters
            .history_index_reads
            .fetch_add(1, Ordering::Relaxed);
        let bytes = read_required_regular(
            &self.capability,
            &history_index_filename(digest),
            MAX_ENGINE_HISTORY_INDEX_BYTES,
            None,
        )?;
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::HistoryIndexPathMismatch(digest));
        }
        let node: HistoryIndexNode =
            postcard::from_bytes(&bytes).map_err(|_| StoreError::MalformedHistoryIndex)?;
        validate_history_node(&node)?;
        if postcard::to_allocvec(&node).map_err(|_| StoreError::MalformedHistoryIndex)? != bytes {
            return Err(StoreError::MalformedHistoryIndex);
        }
        if matches!(node, HistoryIndexNode::Leaf { .. }) {
            self.counters
                .history_record_reads
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(node)
    }
}

impl ExactBootstrapHistoryBuilderV1<'_> {
    /// Consume one exact cold record. The builder retains only radix roots,
    /// bounded identities, and the final engine binding; record bytes are
    /// released after their immutable node path is published.
    pub(crate) fn push(
        &mut self,
        record: &PreparedBootstrapHistoryRecordV1<'_>,
    ) -> Result<(), StoreError> {
        let expected = self
            .expected_parts
            .get(self.next_ordinal)
            .ok_or(StoreError::MalformedHistoryIndex)?;
        if record.binding != self.binding
            || record.part != *expected
            || record.part.acceptance_sequence() != self.next_ordinal as u32 + 1
            || record.part.evidence().ordinal() != self.next_ordinal as u32
            || record.part.evidence().part_count() != self.binding.part_count()
            || !self.batch_ids.insert(record.part.batch_id())
        {
            return Err(StoreError::MalformedHistoryIndex);
        }
        if self.next_ordinal + 1 == self.expected_parts.len()
            && record.engine_binding != self.engine_binding
        {
            return Err(StoreError::MalformedHistoryIndex);
        }
        let next_root =
            self.store
                .index
                .insert(self.index_root, record.part.batch_id(), record.bytes)?;
        if next_root == self.index_root {
            return Err(StoreError::MalformedHistoryIndex);
        }
        self.index_root = next_root;
        self.latest = Some(record.part.batch_id());
        self.next_ordinal += 1;
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<(u64, ContentDigest), StoreError> {
        if self.next_ordinal != self.expected_parts.len()
            || self.next_ordinal != self.binding.part_count() as usize
            || self
                .expected_parts
                .last()
                .map(|part| part.post_frontier())
                .unwrap_or_else(|| self.binding.final_frontier())
                != self.binding.final_frontier()
        {
            return Err(StoreError::MalformedHistoryIndex);
        }
        let generation =
            u64::try_from(self.next_ordinal).map_err(|_| StoreError::MalformedHistoryIndex)?;
        let candidate = DurableEngineHistoryRoot {
            schema_version: ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
            workspace_id: self.store.workspace_id,
            endpoint_id: self.store.endpoint_id,
            graph_resource_id: self.store.graph_resource_id,
            receipt_store_id: self.store.receipt_store_id,
            generation,
            index_root: self.index_root,
            latest_batch_id: self.latest,
            binding: DurableEngineHistoryBinding {
                engine: self.engine_binding,
                bootstrap: Some(self.binding),
            },
        };
        let candidate_digest = self.store.publish_root(&candidate)?;

        let _guard = self
            .store
            .transition
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?;
        let _workspace_guard = AdvisoryTransitionGuard::lock(&self.store.transition_lock)?;
        let (before_digest, before) = self.store.read_live_head_root()?;
        if before_digest == candidate_digest {
            if before != candidate {
                return Err(StoreError::MalformedHistoryIndex);
            }
            *self
                .store
                .authoritative_head
                .lock()
                .map_err(|_| StoreError::MalformedHistoryIndex)? = Some(candidate_digest);
            return Ok((candidate.generation, candidate.index_root));
        }
        if before.generation != 0
            || before.index_root != EngineHistoryStore::empty_root()
            || before.latest_batch_id.is_some()
            || before.binding.bootstrap.is_some()
        {
            return Err(StoreError::BootstrapHistoryRequiresEmptyAuthority);
        }
        self.store.replace_head(before_digest, candidate_digest)?;
        Ok((candidate.generation, candidate.index_root))
    }
}

/// The authenticated endpoint facts one resume-point publication is sealed
/// against.
///
/// Every field is private to this module, so the only route to a value is
/// [`DurableEngineHistoryStore::resume_point_endpoint_binding`]. That method
/// reads this endpoint's *durable* promoted runtime state through
/// [`DurableEngineHistoryStore::read_promoted_runtime_state`] — itself gated by
/// `require_promoted_state_binding`, which proves the state names this
/// workspace, this endpoint, this graph resource, this receipt store and this
/// exact physical archive directory — and derives the next sequence from an
/// actual survey rather than from a caller's belief.
///
/// This is the compile-time half of "the lifecycle caller cannot omit facts":
/// `RuntimeResumePointV2::seal` needs one of these, and nothing outside this
/// module can build one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResumePointEndpointBinding {
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    promoted_state_digest: ContentDigest,
    next_sequence: u64,
}

impl ResumePointEndpointBinding {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn endpoint_id(&self) -> super::ProjectionEndpointId {
        self.endpoint_id
    }

    pub(crate) const fn promoted_state_digest(&self) -> ContentDigest {
        self.promoted_state_digest
    }

    pub(crate) const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Hand-built binding for format-level tests that have no live endpoint.
    #[cfg(test)]
    pub(crate) const fn for_test(
        workspace_id: WorkspaceId,
        endpoint_id: super::ProjectionEndpointId,
        promoted_state_digest: ContentDigest,
        next_sequence: u64,
    ) -> Self {
        Self {
            workspace_id,
            endpoint_id,
            promoted_state_digest,
            next_sequence,
        }
    }
}

/// The live-open authority a published point must re-prove before it may be
/// offered to the engine as an adoption candidate.
///
/// Sealed for the same reason as [`ResumePointEndpointBinding`]: the digest, the
/// endpoint and the durable head all come from this store's own reads, so a
/// caller cannot weaken the comparison by supplying values it wishes were true.
/// The one caller-supplied member is the enrollment admission, because nothing
/// here can read the enrollment chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResumeAdoptionAuthority {
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    promoted_state_digest: ContentDigest,
    history_generation: u64,
    history_index_root: ContentDigest,
    history_latest_batch_id: Option<BatchId>,
    enrollment: ResumeEnrollmentAdmission,
}

impl ResumeAdoptionAuthority {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn endpoint_id(&self) -> super::ProjectionEndpointId {
        self.endpoint_id
    }

    pub(crate) const fn promoted_state_digest(&self) -> ContentDigest {
        self.promoted_state_digest
    }

    pub(crate) const fn history_generation(&self) -> u64 {
        self.history_generation
    }

    pub(crate) const fn history_index_root(&self) -> ContentDigest {
        self.history_index_root
    }

    pub(crate) const fn history_latest_batch_id(&self) -> Option<BatchId> {
        self.history_latest_batch_id
    }

    pub(crate) const fn enrollment(&self) -> ResumeEnrollmentAdmission {
        self.enrollment
    }

    /// Hand-built authority for format-level tests that have no live endpoint.
    #[cfg(test)]
    pub(crate) const fn for_test(
        workspace_id: WorkspaceId,
        endpoint_id: super::ProjectionEndpointId,
        promoted_state_digest: ContentDigest,
        history: (u64, ContentDigest, Option<BatchId>),
        enrollment: ResumeEnrollmentAdmission,
    ) -> Self {
        Self {
            workspace_id,
            endpoint_id,
            promoted_state_digest,
            history_generation: history.0,
            history_index_root: history.1,
            history_latest_batch_id: history.2,
            enrollment,
        }
    }
}

/// Proof that one exact replacement resume point reached durability.
///
/// Minted only by [`DurableEngineHistoryStore::publish_resume_point`], on its
/// success path. Retained-run reclamation consumes one, which is how "reclaim
/// only *after* a successful replacement publication" becomes a fact the type
/// system carries instead of a comment a later caller can reorder past: until
/// the replacement point is durable, the run a predecessor point names may still
/// hold the only resumable bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PublishedResumePoint {
    workspace_id: WorkspaceId,
    resume_sequence: u64,
    scratch_run_id: Uuid,
}

/// Sealed authority for the clear-before-Safe step.
///
/// This value can exist only while the exact graph reservation, watcher
/// quiesce proof, and archive-rooted workspace lease are all borrowed. The
/// lifecycle caller cannot construct one, and the clear operation cannot
/// outlive any of those barriers.
pub(crate) struct SafeTransitionCapability<'barriers, 'lease> {
    history: &'barriers DurableEngineHistoryStore,
    archive: &'barriers ObjectStore,
    _graph: &'barriers HandoffSafe,
    _watcher: &'barriers WatcherQuiescedProof,
    workspace: &'barriers WorkspaceRuntimeProof<'lease>,
}

impl SafeTransitionCapability<'_, '_> {
    fn revalidate_workspace(&self) -> Result<(), ProjectionError> {
        self.workspace
            .authorize_archive(self.archive, self.history.workspace_id)
    }

    /// Remove exactly the recognized Unsafe-bound points. Safe-bound evidence
    /// from an older lifecycle is preserved until a successfully published
    /// Safe successor makes it unreachable.
    ///
    /// The live lease proof is rerun inside this capability immediately before
    /// deletion. The initial proof that minted the capability is intentionally
    /// insufficient: a pathname can disappear or be replaced while the graph
    /// and watcher barriers remain continuously held.
    pub(crate) fn clear_unsafe_resume_points(
        &self,
    ) -> Result<ResumePointMaintenance, SafeTransitionError> {
        self.revalidate_workspace()
            .map_err(SafeTransitionError::Workspace)?;
        self.history
            .clear_unsafe_resume_points()
            .map_err(SafeTransitionError::Store)
    }

    /// Revalidate the same live lease immediately before the durable
    /// `Unsafe -> Safe` closure and keep every capability borrowed until that
    /// closure returns.
    pub(crate) fn commit_handoff<T, E>(
        &self,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, SafeTransitionCommitError<E>> {
        self.revalidate_workspace()
            .map_err(SafeTransitionCommitError::Workspace)?;
        commit().map_err(SafeTransitionCommitError::Commit)
    }
}

#[derive(Debug)]
pub(crate) enum SafeTransitionError {
    Workspace(ProjectionError),
    Store(StoreError),
}

#[derive(Debug)]
pub(crate) enum SafeTransitionCommitError<E> {
    Workspace(ProjectionError),
    Commit(E),
}

impl PublishedResumePoint {
    pub(crate) const fn resume_sequence(&self) -> u64 {
        self.resume_sequence
    }

    pub(crate) const fn scratch_run_id(&self) -> Uuid {
        self.scratch_run_id
    }
}

/// The adoption input one resuming open consumes.
///
/// `Unavailable` is never an error the caller has to recover from: it means
/// "reuse nothing, replay everything", which is always available and always
/// correct. It is carried as a value rather than an `Err` precisely so that a
/// caller cannot accidentally propagate it into a startup failure with `?`.
#[derive(Debug)]
pub(crate) enum ResumeAdoptionCandidate {
    /// Hand this to `ShardedHotEngine::open_enrolled_projection_resuming`. The
    /// engine still re-proves the run, the durable descent and every run-local
    /// root before it reuses a single byte.
    Available(Box<RuntimeResumeSnapshot>),
    Unavailable(ResumeAcceleratorUnavailable),
}

/// Why this open gets no accelerator. Diagnosable, never actionable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResumeAcceleratorUnavailable {
    /// No point has ever been published for this endpoint.
    NeverPublished,
    /// The strict complete-set proof was denied: unrecognizable provider or
    /// desktop residue, a surplus over the publication bound, a torn, renamed
    /// or oversize point, or an entry that could not be classified at all.
    /// Nothing was proved about reachability, so nothing is reclaimed either.
    ProofDenied(ResumePointError),
    /// The latest point decoded and validated but did not re-prove the live
    /// open's authority.
    BindingRefused(ResumePointError),
    /// The store could not be read, or the published set does not bind this
    /// endpoint at all.
    Unavailable(String),
}

/// What retention the next engine scratch run may use.
///
/// The `Ephemeral` arm is the leak bound. Once retention is flipped on, a
/// resuming open mints one retained run per restart, and the only pass that can
/// collect one needs the strict resume-point proof. A directory holding one
/// permanent `.sync-conflict-*` copy denies that proof *forever*, so without
/// this decision every restart would leak one archive directory, silently.
/// Choosing an ephemeral run costs exactly one full replay — always available,
/// always correct — and converts an unbounded disk leak into a bounded loss of
/// an accelerator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EngineScratchRetentionPlan {
    /// A retained run may be minted or adopted: either reachability is
    /// provable, so an unreachable predecessor can be collected later, or the
    /// census is still inside [`MAX_RETAINED_SCRATCH_RUNS`].
    Retained { retained_runs: usize },
    /// Reachability cannot be proved *and* the census is already at its bound.
    Ephemeral {
        retained_runs: usize,
        reason: ResumePointError,
    },
}

/// What one bounded retained-run maintenance pass proved, reclaimed and
/// preserved.
///
/// Maintenance is diagnosable but never a correctness or startup failure, so
/// this type has no `Err` sibling: every failure mode is a variant of
/// [`RetainedRunMaintenanceOutcome`] carried alongside the counts that are
/// still known.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetainedRunMaintenanceReport {
    /// Retained runs whose bytes this pass removed. The only member that
    /// describes deletion.
    pub(crate) reclaimed: usize,
    /// Authenticated retained runs of this workspace still on disk afterwards.
    pub(crate) retained_runs_remaining: usize,
    pub(crate) within_retained_run_bound: bool,
    /// Scratch siblings that could not be authenticated or classified —
    /// including a replicated conflict copy of a run directory. Preserved
    /// untouched, forever, by design.
    pub(crate) unclassified_preserved: usize,
    /// Resume-point directory entries this pass refused to interpret or
    /// remove. Non-empty means the strict proof is denied and retained runs are
    /// leaking, which is the only place that becomes visible.
    pub(crate) preserved_resume_residue: Vec<String>,
    pub(crate) outcome: RetainedRunMaintenanceOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RetainedRunMaintenanceOutcome {
    /// A complete strict proof authorized the pass and it ran.
    Reclaimed,
    /// The strict proof was denied. Every retained run was preserved.
    ProofDenied(ResumePointError),
    /// The pass could not run at all.
    Unavailable(String),
}

impl DurableEngineHistoryStore {
    fn open_sealed_existing(
        workspace_id: WorkspaceId,
        endpoint_id: super::ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
        control: Dir,
        archive_root: Dir,
        transition_lock: fs::File,
        counters: Arc<StoreCounters>,
    ) -> Result<Self, StoreError> {
        // Retain a duplicate handle for the returned store while the guard
        // borrows the original. This keeps one uninterrupted advisory lock
        // across every post-open claim/head/root check and construction, then
        // releases it before callers can invoke a transition method on the
        // returned store and acquire the same lock themselves.
        let retained_transition_lock = transition_lock.try_clone()?;
        let _workspace_guard = AdvisoryTransitionGuard::lock(&transition_lock)?;
        #[cfg(test)]
        sealed_history_authority_window_hook(SealedHistoryAuthorityWindowStage::Locked);
        let claim = read_optional_regular(&control, ENGINE_HISTORY_CLAIM_FILE, 256, None)?
            .ok_or(StoreError::MalformedHistoryIndex)?;
        validate_engine_history_claim(
            &claim,
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
        )?;
        let roots = open_existing_dir_nofollow(&control, ENGINE_HISTORY_ROOTS_DIR)?
            .ok_or(StoreError::MalformedHistoryIndex)?;
        let nodes = open_existing_dir_nofollow(&control, ENGINE_HISTORY_NODES_DIR)?
            .ok_or(StoreError::MalformedHistoryIndex)?;
        let store = Self {
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
            control,
            archive_root,
            roots,
            index: EngineHistoryStore {
                capability: nodes,
                counters,
                storage_fault: AtomicBool::new(false),
            },
            transition_lock: retained_transition_lock,
            transition: Mutex::new(()),
            authoritative_head: Mutex::new(None),
            promoted_lineage: None,
            authenticated_transitions: Mutex::new(Vec::new()),
        };
        let (digest, root) = store.read_live_head_root()?;
        store.require_root_binding(&root)?;
        *store
            .authoritative_head
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)? = Some(digest);
        #[cfg(test)]
        sealed_history_authority_window_hook(SealedHistoryAuthorityWindowStage::Validated);
        Ok(store)
    }

    fn new(
        workspace_id: WorkspaceId,
        endpoint_id: super::ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
        control: Dir,
        archive_root: Dir,
        roots: Dir,
        index: EngineHistoryStore,
        transition_lock: fs::File,
    ) -> Result<Self, StoreError> {
        let store = Self {
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
            control,
            archive_root,
            roots,
            index,
            transition_lock,
            transition: Mutex::new(()),
            authoritative_head: Mutex::new(None),
            promoted_lineage: None,
            authenticated_transitions: Mutex::new(Vec::new()),
        };
        store.initialize()?;
        Ok(store)
    }

    pub(crate) fn current(&self) -> Result<(u64, ContentDigest), StoreError> {
        let (_, root) = self.load_head_root()?;
        Ok((root.generation, root.index_root))
    }

    pub(crate) fn current_authority(&self) -> Result<EngineHistoryAuthority, StoreError> {
        let (_, root) = self.load_head_root()?;
        Ok(EngineHistoryAuthority {
            generation: root.generation,
            index_root: root.index_root,
        })
    }

    /// Authenticate an insertion-only transition directly from the shared
    /// immutable radix structure. Equal subtrees terminate immediately, so a
    /// normal point append is bounded by the changed radix paths rather than
    /// the lifetime history size.
    ///
    /// A promoted runtime proves every admission from one immutable bootstrap
    /// anchor, so the anchor falls further behind the head with each batch and
    /// the walk from that anchor grows with the post-anchor history. This open
    /// therefore memoizes the transitions it already proved and, when one of
    /// them starts at exactly this `before`, authenticates only the residual
    /// `middle -> current` step and composes.
    ///
    /// The memo is deliberately *transparent*: composition is attempted first
    /// and a failed residual step falls through to the complete
    /// `before -> current` walk, so the accepted/rejected outcome is exactly
    /// the one the uncached walk produces. That is what makes the accelerator
    /// safe rather than merely fast — see
    /// [`Self::compose_cached_history_extension`].
    ///
    /// A memo may only shorten the *walk*, never the availability and integrity
    /// facts the walk establishes about the live endpoints. Before any
    /// composition can run, [`Self::require_live_history_endpoint_nodes`]
    /// re-reads and re-authenticates from storage exactly the endpoint nodes
    /// the direct walk would have read, so a current root that has been
    /// deleted, truncated, substituted or digest-corrupted since this open
    /// warmed its memo is rejected identically warm and fresh. Faults *below*
    /// the endpoints stay a previously authenticated in-memory fact, guarded by
    /// the storage-fault latch described on [`EngineHistoryStore::storage_fault`]
    /// — see the residual note on [`Self::compose_cached_history_extension`].
    pub(crate) fn authenticate_current_history_extension(
        &self,
        before: EngineHistoryAuthority,
    ) -> Result<AuthenticatedEngineHistoryTransition, StoreError> {
        let after = self.current_authority()?;
        if (before.generation == 0) != (before.index_root == EngineHistoryStore::empty_root()) {
            return Err(StoreError::MalformedHistoryIndex);
        }
        let proof = match self.cached_history_extension(before) {
            Some(middle) => {
                self.require_live_history_endpoint_nodes(before.index_root, after.index_root)?;
                match self.compose_cached_history_extension(before, middle, after) {
                    Some(composed) => composed,
                    None => self.walk_history_extension(before, after)?,
                }
            }
            None => self.walk_history_extension(before, after)?,
        };
        self.remember_authenticated_history_extension(proof);
        Ok(proof)
    }

    /// The complete, unmemoized `before -> current` proof. This is the only
    /// thing that ever mints a transition out of raw storage, and it is exactly
    /// what a fresh open performs.
    fn walk_history_extension(
        &self,
        before: EngineHistoryAuthority,
        after: EngineHistoryAuthority,
    ) -> Result<AuthenticatedEngineHistoryTransition, StoreError> {
        let added = self.insertion_only_added_records(before.index_root, after.index_root, 0)?;
        if before
            .generation
            .checked_add(added)
            .filter(|generation| *generation == after.generation)
            .is_none()
        {
            return Err(StoreError::MalformedHistoryIndex);
        }
        Ok(AuthenticatedEngineHistoryTransition { before, after })
    }

    /// Re-establish, from storage, exactly the depth-0 availability and
    /// integrity facts [`Self::insertion_only_added_records`] would establish
    /// for this endpoint pair — no more, so a warm verdict stays byte-for-byte
    /// the fresh verdict, and no less, so a memo can never inherit them.
    ///
    /// The walk reads nothing when the two roots are identical (equal subtrees
    /// terminate immediately) and rejects a retreat to the empty root without
    /// reading anything, so those two cases are reproduced by reading nothing.
    /// Otherwise it reads `before` first and then `after`, requiring each to be
    /// an available, correctly addressed, canonical depth-0 branch; this
    /// mirrors the walk's own first step, including which endpoint reports the
    /// failure and with which error.
    fn require_live_history_endpoint_nodes(
        &self,
        before: ContentDigest,
        after: ContentDigest,
    ) -> Result<(), StoreError> {
        let empty = EngineHistoryStore::empty_root();
        if before == after || after == empty {
            return Ok(());
        }
        if before != empty {
            self.require_live_history_branch_root(before)?;
        }
        self.require_live_history_branch_root(after)
    }

    fn require_live_history_branch_root(&self, root: ContentDigest) -> Result<(), StoreError> {
        match self.index.read_node(root)? {
            HistoryIndexNode::Branch { depth: 0, .. } => Ok(()),
            // A node that reads cleanly but is not the radix root it is used as
            // is a substitution, not a lineage disagreement.
            _ => {
                self.index.note_storage_fault();
                Err(StoreError::MalformedHistoryIndex)
            }
        }
    }

    /// Compose a memoized `before -> middle` proof with a freshly walked
    /// `middle -> current` step.
    ///
    /// Soundness. A memo entry is only ever minted by
    /// [`Self::authenticate_current_history_extension`] on this exact store, so
    /// it carries this store's own proof that `middle`'s record set contains
    /// `before`'s with identical leaves on shared keys and that
    /// `before.generation + (|middle| - |before|) == middle.generation`. The
    /// residual step proves the same two facts for `middle -> current` with the
    /// identical walk and the identical exact-generation equality. Structural
    /// containment with agreeing leaves is transitive, and the two exact
    /// equalities telescope to
    /// `before.generation + (|current| - |before|) == current.generation`
    /// without overflow, because every intermediate sum is bounded by
    /// `current.generation`. Composition therefore establishes precisely what
    /// the direct `before -> current` walk establishes — including its
    /// rollback, divergence and missing-leaf rejections, which are exactly the
    /// walk's failures on the residual step.
    ///
    /// Lineage staleness. The memo records a fact about immutable
    /// content-addressed radix nodes, not a claim about the live head, so the
    /// *structural* fact cannot decay: no publish, failed publish or head
    /// replacement can make a once-true containment false. The live `current`
    /// is re-read on every call and the residual step is always freshly walked
    /// and digest-verified, so a memo that no longer lies on the live lineage
    /// can only fail to compose. Returning `None` on any such failure hands the
    /// decision back to the complete walk, so the memo can neither turn a
    /// rejection into an acceptance nor an acceptance into a rejection.
    ///
    /// Availability. What a memo *can* outlive is the storage the walk read.
    /// Two mechanisms bound that, because re-reading the whole authenticated
    /// region on every call is precisely the lifetime-sized work the memo
    /// exists to remove:
    ///
    /// 1. [`Self::require_live_history_endpoint_nodes`] re-reads and
    ///    re-authenticates the live endpoint nodes on every warm call, so
    ///    depth-0 loss, truncation, substitution and digest corruption are
    ///    rejected identically warm and fresh.
    /// 2. Deeper nodes stay a fact this same open authenticated earlier — every
    ///    node the direct `before -> current` walk would read was read and
    ///    digest-verified by this store when the memo entry was minted, by
    ///    induction over the composition chain. The compensating guarantee is
    ///    causal: the first operation that re-encounters damage down there —
    ///    any lookup, replay, rebuild or insertion that descends into it —
    ///    latches [`EngineHistoryStore::storage_fault`], which permanently
    ///    disarms this memo, so from that point the store decides exactly like
    ///    a fresh open. [`Self::publish`] latches it for an incomplete
    ///    publication too.
    ///
    /// The residual this leaves is narrow and deliberate: a node that this open
    /// already authenticated is destroyed by something outside Tine while Tine
    /// runs, and nothing touches it again before the next admission. Such an
    /// admission can extend the history along an undamaged radix path, which a
    /// fresh open would instead refuse; it cannot surface, project or replay
    /// the damaged region, because every path that reads it latches the fault
    /// first. A reopened store starts with an empty memo and pays the full walk
    /// once, so nothing here survives a restart.
    fn compose_cached_history_extension(
        &self,
        before: EngineHistoryAuthority,
        middle: EngineHistoryAuthority,
        after: EngineHistoryAuthority,
    ) -> Option<AuthenticatedEngineHistoryTransition> {
        let added = self
            .insertion_only_added_records(middle.index_root, after.index_root, 0)
            .ok()?;
        middle
            .generation
            .checked_add(added)
            .filter(|generation| *generation == after.generation)?;
        Some(AuthenticatedEngineHistoryTransition { before, after })
    }

    /// The furthest endpoint this store proved from exactly this anchor.
    ///
    /// Both anchor fields must match exactly; a substituted generation or index
    /// root simply misses the memo and is decided by the full walk. A latched
    /// storage fault discards the memo outright and keeps it discarded for the
    /// rest of this open.
    fn cached_history_extension(
        &self,
        before: EngineHistoryAuthority,
    ) -> Option<EngineHistoryAuthority> {
        let mut cache = self.authenticated_transitions.lock().ok()?;
        if self.index.storage_faulted() {
            cache.clear();
            return None;
        }
        cache
            .iter()
            .find(|entry| entry.before == before)
            .map(|entry| entry.after)
    }

    /// Retain the proof so the next admission from the same anchor only has to
    /// walk the records published after it.
    ///
    /// One entry per anchor and at most
    /// [`MAX_AUTHENTICATED_TRANSITION_ANCHORS`] anchors, evicted least-recently
    /// proved first. The recency order matters: the projection-work caller
    /// re-anchors on the head it just accepted, so it presents a *fresh* anchor
    /// every batch, while the promoted-runtime caller keeps proving from one
    /// immutable bootstrap anchor. Plain insertion order would let the moving
    /// anchor evict the fixed one within a few batches and restore exactly the
    /// growth this memo exists to remove; re-seating an anchor on every
    /// successful proof keeps the repeatedly used one resident. Only a proof
    /// this store just minted is recorded, so a rejected transition can neither
    /// enter the memo nor churn it. A poisoned memo lock degrades to no memo,
    /// never to a weaker proof, and so does a latched storage fault.
    fn remember_authenticated_history_extension(
        &self,
        proof: AuthenticatedEngineHistoryTransition,
    ) {
        let Ok(mut cache) = self.authenticated_transitions.lock() else {
            return;
        };
        if self.index.storage_faulted() {
            cache.clear();
            return;
        }
        cache.retain(|entry| entry.before != proof.before);
        if cache.len() >= MAX_AUTHENTICATED_TRANSITION_ANCHORS {
            cache.remove(0);
        }
        cache.push(proof);
    }

    fn insertion_only_added_records(
        &self,
        before: ContentDigest,
        after: ContentDigest,
        depth: u8,
    ) -> Result<u64, StoreError> {
        if before == after {
            return Ok(0);
        }
        if before == EngineHistoryStore::empty_root() {
            return self.history_record_count(after, depth);
        }
        if after == EngineHistoryStore::empty_root() {
            return Err(StoreError::MalformedHistoryIndex);
        }
        match (self.index.read_node(before)?, self.index.read_node(after)?) {
            (
                HistoryIndexNode::Branch {
                    depth: before_depth,
                    children: before_children,
                    ..
                },
                HistoryIndexNode::Branch {
                    depth: after_depth,
                    children: after_children,
                    ..
                },
            ) if before_depth == depth && after_depth == depth => {
                let mut added = 0_u64;
                for (nibble, before_child) in &before_children {
                    let after_child = after_children
                        .iter()
                        .find(|(candidate, _)| *candidate == *nibble)
                        .map(|(_, digest)| *digest)
                        .ok_or(StoreError::MalformedHistoryIndex)?;
                    added = added
                        .checked_add(self.insertion_only_added_records(
                            *before_child,
                            after_child,
                            depth + 1,
                        )?)
                        .ok_or(StoreError::MalformedHistoryIndex)?;
                }
                for (nibble, after_child) in after_children {
                    if !before_children
                        .iter()
                        .any(|(candidate, _)| *candidate == nibble)
                    {
                        added = added
                            .checked_add(self.history_record_count(after_child, depth + 1)?)
                            .ok_or(StoreError::MalformedHistoryIndex)?;
                    }
                }
                Ok(added)
            }
            (
                HistoryIndexNode::Leaf {
                    batch_id: before_batch,
                    record: before_record,
                    ..
                },
                HistoryIndexNode::Leaf {
                    batch_id: after_batch,
                    record: after_record,
                    ..
                },
            ) if depth == ENGINE_HISTORY_RADIX_DEPTH
                && before_batch == after_batch
                && before_record == after_record =>
            {
                Ok(0)
            }
            _ => Err(StoreError::MalformedHistoryIndex),
        }
    }

    fn history_record_count(&self, root: ContentDigest, depth: u8) -> Result<u64, StoreError> {
        if root == EngineHistoryStore::empty_root() {
            return Ok(0);
        }
        match self.index.read_node(root)? {
            HistoryIndexNode::Branch {
                depth: found,
                children,
                ..
            } if found == depth && depth < ENGINE_HISTORY_RADIX_DEPTH => {
                children.into_iter().try_fold(0_u64, |count, (_, child)| {
                    count
                        .checked_add(self.history_record_count(child, depth + 1)?)
                        .ok_or(StoreError::MalformedHistoryIndex)
                })
            }
            HistoryIndexNode::Leaf { .. } if depth == ENGINE_HISTORY_RADIX_DEPTH => Ok(1),
            _ => Err(StoreError::MalformedHistoryIndex),
        }
    }

    pub(crate) fn current_with_binding(
        &self,
    ) -> Result<(u64, ContentDigest, Option<BatchId>, EngineHistoryBinding), StoreError> {
        let (_, root) = self.load_head_root()?;
        Ok((
            root.generation,
            root.index_root,
            root.latest_batch_id,
            root.binding.engine.clone(),
        ))
    }

    pub(crate) fn current_bootstrap_binding(
        &self,
    ) -> Result<Option<BootstrapAggregateHistoryBindingV1>, StoreError> {
        let (_, root) = self.load_head_root()?;
        Ok(root.binding.bootstrap)
    }

    pub(crate) fn current_record_count(&self) -> Result<u64, StoreError> {
        let (_, root) = self.load_head_root()?;
        self.history_record_count(root.index_root, 0)
    }

    /// Read the device-local promoted-runtime state, if one was ever published.
    ///
    /// A present state must decode canonically at the supported schema version
    /// and must claim exactly this endpoint *and this exact physical archive*.
    /// Truncated, foreign, or divergent residue fails closed instead of being
    /// repaired.
    pub(crate) fn read_promoted_runtime_state(
        &self,
    ) -> Result<Option<PromotedRuntimeStateV1>, StoreError> {
        let Some(bytes) = read_optional_regular(
            &self.control,
            PROMOTED_RUNTIME_STATE_FILE,
            MAX_PROMOTED_RUNTIME_STATE_BYTES,
            None,
        )?
        else {
            return Ok(None);
        };
        let state = PromotedRuntimeStateV1::decode(&bytes)?;
        self.require_promoted_state_binding(&state)?;
        Ok(Some(state))
    }

    /// The one promoted-state authorization boundary.
    ///
    /// Every promoted-state read, publication, and live authorization goes
    /// through here, so no caller — present or future — can reach the state
    /// file of an archive the state does not bind. Endpoint identity alone is
    /// not enough: a byte-identical stale copy of an archive carries the same
    /// endpoint claim, the same durable history, and the same canonical
    /// archive-resource claim bytes, and is distinguishable only by its
    /// physical control-directory identity.
    fn require_promoted_state_binding(
        &self,
        state: &PromotedRuntimeStateV1,
    ) -> Result<(), StoreError> {
        if state.workspace_id != self.workspace_id
            || state.endpoint_id != self.endpoint_id
            || state.graph_resource_id != self.graph_resource_id
            || state.receipt_store_id != self.receipt_store_id
        {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "promoted runtime state is bound to another endpoint",
            ));
        }
        if control_directory_identity(&self.archive_root)?.binding_digest()
            != state.archive_control_binding
        {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "promoted runtime state is bound to another physical archive directory",
            ));
        }
        super::CanonicalArchiveResourceId::open_enrolled_in_retained_directory(
            &self.archive_root,
            state.archive_resource_id,
        )
        .map_err(|_| {
            StoreError::PromotedRuntimeStateMismatch(
                "promoted runtime state archive resource claim does not authenticate",
            )
        })?;
        Ok(())
    }

    /// Publish the one-time promoted-runtime state for this endpoint.
    ///
    /// The publication is a single immutable exact file, so every crash cut
    /// reopens as either the unchanged inactive bootstrap (no file) or the one
    /// exact resumable promoted state (complete file). Repeating the call with
    /// byte-identical state resumes; any divergent competing promotion fails
    /// closed and preserves the committed state as evidence.
    pub(crate) fn publish_promoted_runtime_state(
        &self,
        state: &PromotedRuntimeStateV1,
    ) -> Result<(), StoreError> {
        let _guard = self
            .transition
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?;
        let _workspace_guard = AdvisoryTransitionGuard::lock(&self.transition_lock)?;
        state.validate()?;
        self.require_promoted_state_binding(state)?;
        if let Some(existing) = self.read_promoted_runtime_state()? {
            return if &existing == state {
                Ok(())
            } else {
                Err(StoreError::CompetingRuntimePromotion)
            };
        }
        let (_, root) = self.read_live_head_root()?;
        match root.binding.bootstrap {
            Some(bootstrap) if bootstrap == state.bootstrap => {}
            _ => {
                return Err(StoreError::PromotedRuntimeStateMismatch(
                    "promoted runtime state does not bind this durable history's bootstrap aggregate",
                ));
            }
        }
        if root.generation != state.anchor_history_generation
            || root.index_root != state.anchor_history_index_root
        {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "first promotion requires the exact unadvanced bootstrap history anchor",
            ));
        }
        let bytes = state.encode()?;
        publish_immutable_exact(
            &self.control,
            PROMOTED_RUNTIME_STATE_FILE,
            &bytes,
            "promoted runtime state",
        )
    }

    /// The one resume-point authorization boundary, mirroring
    /// [`Self::require_promoted_state_binding`].
    ///
    /// A resume point must claim this workspace and must carry the digest of
    /// *this* endpoint's durable promoted-runtime state. The promoted state was
    /// itself read through `require_promoted_state_binding`, so matching its
    /// digest transitively proves the point belongs to this endpoint, this
    /// physical archive directory, this archive resource claim, and this
    /// bootstrap-anchored lineage. A stale byte-identical copy of another
    /// archive carries a different control-directory identity and therefore a
    /// different promoted state, so its resume points cannot bind here.
    fn require_resume_point_binding(
        &self,
        point: &RuntimeResumePointV2,
        promoted_state_digest: ContentDigest,
    ) -> Result<(), StoreError> {
        if point.workspace_id() != self.workspace_id {
            return Err(StoreError::ResumePointBindingMismatch(
                "runtime resume point is bound to another workspace",
            ));
        }
        if point.promoted_state_digest() != promoted_state_digest {
            return Err(StoreError::ResumePointBindingMismatch(
                "runtime resume point is bound to another promoted runtime state",
            ));
        }
        Ok(())
    }

    /// Survey this endpoint's resume-point directory and authenticate every
    /// point it recognized.
    ///
    /// This is the shared substrate of the strict proof and of publication.
    /// Unrecognizable residue is *carried*, not raised: the caller decides
    /// whether its own operation may proceed beside it.
    fn scan_resume_points(&self) -> Result<ResumePointScan, StoreError> {
        let scan = ResumePointScan::survey(&self.control)?;
        if scan.points().is_empty() {
            return Ok(scan);
        }
        // A published point without a promoted state is residue, not evidence:
        // there is nothing that could have authorized it.
        let promoted =
            self.read_promoted_runtime_state()?
                .ok_or(StoreError::ResumePointBindingMismatch(
                    "a runtime resume point exists without a promoted runtime state",
                ))?;
        let promoted_state_digest = promoted.state_digest()?;
        for point in scan.points() {
            self.require_resume_point_binding(point, promoted_state_digest)?;
        }
        Ok(scan)
    }

    /// Read the complete validated resume-point set of this endpoint.
    ///
    /// This is the strict adoption/reclamation proof. It fails closed on any
    /// residue and on a point surplus, and never returns a partial view: an
    /// `Err` here proves nothing about reachability, so the caller must
    /// preserve every candidate retained run. An absent directory is the
    /// ordinary "never published" shape and is not an error.
    ///
    /// **Deliberately private.** It used to be the crate-wide entry point, and
    /// that is exactly the shape `cf7dbe0b` teaches to remove: a `ResumePointSet`
    /// in a lifecycle caller's hands is one `.reachable_runs()` away from
    /// deletion authority that was never ordered behind a publication. The
    /// three sealed entry points below —
    /// [`Self::read_resume_adoption_candidate`],
    /// [`Self::plan_engine_scratch_retention`] and
    /// [`Self::reclaim_retained_runs_after_publication`] — are the whole
    /// supported surface, and none of them hands the strict proof out.
    fn read_resume_point_set(&self) -> Result<ResumePointSet, StoreError> {
        Ok(self.scan_resume_points()?.into_set()?)
    }

    /// The sealed endpoint binding one publication is minted against.
    ///
    /// Fails closed when this endpoint has no durable promoted runtime state
    /// (nothing could have authorized a point) or when the directory holds
    /// residue the survey could not classify (publishing beside an
    /// unrecognizable entry risks mistaking a provider conflict copy for
    /// authority).
    fn resume_point_endpoint_binding(&self) -> Result<ResumePointEndpointBinding, StoreError> {
        let promoted = self
            .read_promoted_runtime_state()?
            .ok_or(StoreError::PromotedRuntimeStateAbsent)?;
        let scan = self.scan_resume_points()?;
        scan.require_recognizable()?;
        Ok(ResumePointEndpointBinding {
            workspace_id: self.workspace_id,
            endpoint_id: self.endpoint_id,
            promoted_state_digest: promoted.state_digest()?,
            next_sequence: next_resume_sequence(scan.points())?,
        })
    }

    /// Build this endpoint's next resume point from a quiescent live engine.
    ///
    /// This is the one construction API. The run-local facts can only come from
    /// `snapshot`, which `ShardedHotEngine::runtime_resume_snapshot` mints only
    /// for a retained, quiescent, conflict-free, non-terminal engine whose
    /// durable head it re-read and whose head record is itself adoptable; the
    /// identity facts can only come from this store. The caller supplies just
    /// the enrollment evidence it authenticated, which is the one thing neither
    /// side can see.
    ///
    /// Record construction is a quiescent lifecycle read plus one encode. It is
    /// not on, and must never be moved onto, the keystroke, admission,
    /// authoring or acceptance path.
    pub(crate) fn mint_resume_point(
        &self,
        snapshot: &RuntimeResumeSnapshot,
        enrollment: ResumePointEnrollmentBinding,
    ) -> Result<RuntimeResumePointV2, StoreError> {
        let binding = self.resume_point_endpoint_binding()?;
        Ok(RuntimeResumePointV2::seal(&binding, enrollment, snapshot)?)
    }

    /// The sealed authority a published point must re-prove at a resuming open.
    fn resume_adoption_authority(
        &self,
        enrollment: ResumeEnrollmentAdmission,
    ) -> Result<ResumeAdoptionAuthority, StoreError> {
        let promoted = self
            .read_promoted_runtime_state()?
            .ok_or(StoreError::PromotedRuntimeStateAbsent)?;
        let (_, root) = self.read_live_head_root()?;
        Ok(ResumeAdoptionAuthority {
            workspace_id: self.workspace_id,
            endpoint_id: self.endpoint_id,
            promoted_state_digest: promoted.state_digest()?,
            history_generation: root.generation,
            history_index_root: root.index_root,
            history_latest_batch_id: root.latest_batch_id,
            enrollment,
        })
    }

    /// The strict latest-point read a resuming open consumes.
    ///
    /// The smallest surface that does the whole job: survey the directory,
    /// authenticate every recognized point against this endpoint's promoted
    /// state, mint the strict complete-set proof, take its highest sequence,
    /// re-prove the live open's authority against it, and hand back the exact
    /// snapshot the emitting engine produced.
    ///
    /// It never returns `Err`, never writes, and never removes a byte. Every
    /// doubt — an absent directory, unrecognizable provider residue, a surplus
    /// over the publication bound, a torn/renamed/oversize point, a foreign
    /// workspace or endpoint, a substituted durable history authority, or
    /// enrollment evidence the live record contradicts — becomes
    /// [`ResumeAdoptionCandidate::Unavailable`], i.e. a fresh retained run and
    /// a full replay. A *still-leased* run is refused one layer further in, by
    /// `adopt_retained_engine_scratch`, which is where the exclusive lease
    /// lives; that refusal likewise costs one full replay and leaves the
    /// candidate's bytes untouched.
    pub(crate) fn read_resume_adoption_candidate(
        &self,
        enrollment: ResumeEnrollmentAdmission,
    ) -> ResumeAdoptionCandidate {
        let scan = match self.scan_resume_points() {
            Ok(scan) => scan,
            Err(error) => {
                return ResumeAdoptionCandidate::Unavailable(
                    ResumeAcceleratorUnavailable::Unavailable(error.to_string()),
                );
            }
        };
        let set = match scan.into_set() {
            Ok(set) => set,
            Err(reason) => {
                return ResumeAdoptionCandidate::Unavailable(
                    ResumeAcceleratorUnavailable::ProofDenied(reason),
                );
            }
        };
        let Some(point) = set.latest() else {
            return ResumeAdoptionCandidate::Unavailable(
                ResumeAcceleratorUnavailable::NeverPublished,
            );
        };
        let authority = match self.resume_adoption_authority(enrollment) {
            Ok(authority) => authority,
            Err(error) => {
                return ResumeAdoptionCandidate::Unavailable(
                    ResumeAcceleratorUnavailable::Unavailable(error.to_string()),
                );
            }
        };
        match point.authenticate(&authority) {
            Ok(authenticated) => {
                ResumeAdoptionCandidate::Available(Box::new(authenticated.into_adoption_snapshot()))
            }
            Err(reason) => ResumeAdoptionCandidate::Unavailable(
                ResumeAcceleratorUnavailable::BindingRefused(reason),
            ),
        }
    }

    /// Decide, before a run is minted, whether this open may take a retained
    /// run at all.
    ///
    /// A retained run is safe to mint whenever it can later be *proved*
    /// unreachable — that is, whenever the strict set is available. When it is
    /// not, the census decides: below the bound, one more retained run is an
    /// acceptable accelerator; at or above it, minting another would add one
    /// permanently uncollectable directory per restart, so this returns
    /// [`EngineScratchRetentionPlan::Ephemeral`] and the open pays a full
    /// replay instead.
    ///
    /// A census that cannot be taken is treated as "at the bound" for the same
    /// reason: an unknown population must not authorize growth.
    pub(crate) fn plan_engine_scratch_retention(&self) -> EngineScratchRetentionPlan {
        let census =
            super::scratch_store::census_retained_runs(&self.archive_root, self.workspace_id);
        let reason = match self
            .scan_resume_points()
            .map_err(|error| ResumePointError::Io(error.to_string()))
            .and_then(ResumePointScan::into_set)
        {
            Ok(_) => {
                return EngineScratchRetentionPlan::Retained {
                    retained_runs: census.map(|census| census.retained).unwrap_or_default(),
                };
            }
            Err(reason) => reason,
        };
        // An uncountable namespace must not authorize growth either.
        let Ok(census) = census else {
            return EngineScratchRetentionPlan::Ephemeral {
                retained_runs: MAX_RETAINED_SCRATCH_RUNS,
                reason,
            };
        };
        if census.retained >= MAX_RETAINED_SCRATCH_RUNS {
            EngineScratchRetentionPlan::Ephemeral {
                retained_runs: census.retained,
                reason,
            }
        } else {
            EngineScratchRetentionPlan::Retained {
                retained_runs: census.retained,
            }
        }
    }

    /// Reclaim every retained run the published point set no longer reaches.
    ///
    /// The [`PublishedResumePoint`] witness is the ordering fence, and this
    /// method adds the one check the witness cannot carry: the published point
    /// must still be present in the strict set it is about to derive
    /// reachability from. A replacement that is durable but not in the proof
    /// means the proof is not describing the state the caller published, so the
    /// pass preserves everything.
    ///
    /// Nothing here is fatal. A denied proof, an unreadable namespace, or a
    /// per-sibling I/O error all preserve bytes and report; unclassified
    /// residue — including a replicated conflict copy of a run directory — is
    /// never deleted, and neither is a run whose own exclusive lease is held.
    pub(crate) fn reclaim_retained_runs_after_publication(
        &self,
        published: &PublishedResumePoint,
    ) -> RetainedRunMaintenanceReport {
        if published.workspace_id != self.workspace_id {
            return self.preserving_report(RetainedRunMaintenanceOutcome::Unavailable(
                "the published resume point belongs to another workspace".to_owned(),
            ));
        }
        let scan = match self.scan_resume_points() {
            Ok(scan) => scan,
            Err(error) => {
                return self.preserving_report(RetainedRunMaintenanceOutcome::Unavailable(
                    error.to_string(),
                ));
            }
        };
        let residue: Vec<String> = scan
            .residue()
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let set = match scan.into_set() {
            Ok(set) => set,
            Err(reason) => {
                let mut report =
                    self.preserving_report(RetainedRunMaintenanceOutcome::ProofDenied(reason));
                report.preserved_resume_residue = residue;
                return report;
            }
        };
        if !set.points().iter().any(|point| {
            point.resume_sequence() == published.resume_sequence
                && point.scratch_run_id() == published.scratch_run_id
        }) {
            return self.preserving_report(RetainedRunMaintenanceOutcome::ProofDenied(
                ResumePointError::Malformed(
                    "the published replacement resume point is not in the complete set",
                ),
            ));
        }
        let reachable = set.reachable_runs();
        match super::scratch_store::reclaim_unreachable_retained_runs(
            &self.archive_root,
            self.workspace_id,
            &reachable,
        ) {
            Ok(reclamation) => RetainedRunMaintenanceReport {
                reclaimed: reclamation.retained_reclaimed,
                retained_runs_remaining: reclamation.retained_runs_remaining(),
                within_retained_run_bound: reclamation.within_retained_run_bound(),
                unclassified_preserved: reclamation.unclassified_preserved,
                preserved_resume_residue: residue,
                outcome: RetainedRunMaintenanceOutcome::Reclaimed,
            },
            Err(error) => {
                let mut report = self.preserving_report(
                    RetainedRunMaintenanceOutcome::Unavailable(error.to_string()),
                );
                report.preserved_resume_residue = residue;
                report
            }
        }
    }

    /// A report for a pass that deleted nothing, with whatever census is still
    /// obtainable so the caller can still see an accumulating population.
    fn preserving_report(
        &self,
        outcome: RetainedRunMaintenanceOutcome,
    ) -> RetainedRunMaintenanceReport {
        let census =
            super::scratch_store::census_retained_runs(&self.archive_root, self.workspace_id)
                .unwrap_or_default();
        RetainedRunMaintenanceReport {
            reclaimed: 0,
            retained_runs_remaining: census.retained,
            within_retained_run_bound: census.retained <= MAX_RETAINED_SCRATCH_RUNS,
            unclassified_preserved: census.unclassified,
            preserved_resume_residue: Vec::new(),
            outcome,
        }
    }

    /// Publish one resume point, keeping the durable set bounded at every cut.
    ///
    /// The ordering rule is that **no cut may have zero valid points while a
    /// retained run holds the only resumable bytes**, and the shape that
    /// satisfies it is a bounded three-step sequence:
    ///
    /// 1. if the recognized set has already reached
    ///    [`MAX_RETAINED_RESUME_POINTS`], durably prune every recognized point
    ///    *below the current latest*. A crash here still leaves that latest
    ///    point, which is durable, valid, and already the newest evidence, so
    ///    nothing that was resumable stops being resumable;
    /// 2. publish the successor. This is the commit point;
    /// 3. prune every recognized point below the successor.
    ///
    /// Step 1 is what makes the bound self-restoring instead of a trap. Without
    /// it, one crash between steps 2 and 3 left `{n, n+1}` on disk, and the
    /// next honest publication — which after a crash takeover is a *different*
    /// session at a *later* enrollment generation, so it can never be a
    /// byte-identical retry of `n+1` — committed a third point and then failed
    /// its own prune, permanently bricking read, publish and clear. With it,
    /// the widest durable cut is two points and any pre-existing surplus
    /// converges on the next publication.
    ///
    /// Step 1 is also the only place in this packet that deletes a durable
    /// point *before* committing its replacement, so both of its cuts are named
    /// in [`ResumePublishBoundary`] and driven by deterministic fault injection
    /// rather than left to the doc claim above.
    ///
    /// The publication is immutable-exact, so repeating the call with
    /// byte-identical bytes resumes and re-runs the prune, while divergent
    /// bytes at the same sequence fail closed as
    /// [`StoreError::ImmutableCollision`]. Under the archive-rooted workspace
    /// runtime lease that collision is impossible in honest operation, which is
    /// exactly why it is a corruption signal rather than a retry.
    ///
    /// This records evidence only. It grants no write, frontier, projection, or
    /// import authority, and it deliberately performs no scratch-run
    /// reclamation: proving a retained run unreachable is a separate step the
    /// caller takes with [`Self::reclaim_retained_runs_after_publication`],
    /// which consumes the [`PublishedResumePoint`] this returns.
    pub(crate) fn publish_resume_point(
        &self,
        point: &RuntimeResumePointV2,
    ) -> Result<PublishedResumePoint, StoreError> {
        let _guard = self
            .transition
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?;
        let _workspace_guard = AdvisoryTransitionGuard::lock(&self.transition_lock)?;
        point.validate()?;
        let promoted = self
            .read_promoted_runtime_state()?
            .ok_or(StoreError::PromotedRuntimeStateAbsent)?;
        self.require_resume_point_binding(point, promoted.state_digest()?)?;

        // The recorded run-local roots correspond to one exact durable history
        // authority. Proving that here, from this store's own live head, is
        // what keeps the history binding real rather than caller-asserted.
        let (_, root) = self.read_live_head_root()?;
        if root.generation != point.history_generation()
            || root.index_root != point.history_index_root()
        {
            return Err(StoreError::ResumePointBindingMismatch(
                "runtime resume point does not name this endpoint's live durable history",
            ));
        }

        let bytes = point.encode()?;
        // Publication is bound-tolerant but poison-intolerant. A surplus of
        // recognized points is a state this call converges; an unrecognizable
        // entry means the directory is not fully understood, and publishing
        // beside it could mistake a provider conflict copy for authority, so it
        // fails closed to a full replay. Maintenance stays available either
        // way, so failing closed here can never make clearing impossible.
        let scan = self.scan_resume_points()?;
        scan.require_recognizable()?;
        let recognized = scan.points();
        let next = next_resume_sequence(recognized)?;
        let latest = recognized.last().map(|latest| latest.resume_sequence());
        // Either the next fresh sequence, or a retry of the last publication
        // whose byte identity `publish_immutable_exact` then proves.
        if point.resume_sequence() != next && Some(point.resume_sequence()) != latest {
            return Err(StoreError::ResumePointSequenceRegression {
                expected: next,
                found: point.resume_sequence(),
            });
        }

        ensure_directory_nofollow(&self.control, RESUME_POINT_DIR)?;
        let directory = open_dir_nofollow(&self.control, RESUME_POINT_DIR)?;
        // ---- Step 1: make room, without ever dropping the latest point. ----
        if recognized.len() >= MAX_RETAINED_RESUME_POINTS {
            if let Some(latest) = latest {
                prune_resume_points_below(&directory, latest)?;
            }
        }
        #[cfg(test)]
        inject_resume_publish_fault(ResumePublishBoundary::AfterPrePrune)?;
        publish_immutable_exact(
            &directory,
            &point.file_name(),
            &bytes,
            "runtime resume point",
        )?;
        // ---- COMMIT POINT: the new resume point is durable. ----
        #[cfg(test)]
        inject_resume_publish_fault(ResumePublishBoundary::AfterCommit)?;
        prune_resume_points_below(&directory, point.resume_sequence())?;
        Ok(PublishedResumePoint {
            workspace_id: self.workspace_id,
            resume_sequence: point.resume_sequence(),
            scratch_run_id: point.scratch_run_id(),
        })
    }

    /// Mint the only capability that may clear points for a Safe transition.
    ///
    /// All three authorities are checked against this exact sealed endpoint.
    /// Borrowing them into the result makes dropping either barrier before the
    /// clear a compile error.
    pub(crate) fn begin_safe_transition<'barriers, 'lease>(
        &'barriers self,
        archive: &'barriers ObjectStore,
        workspace: &'barriers WorkspaceRuntimeProof<'lease>,
        graph: &'barriers HandoffSafe,
        watcher: &'barriers WatcherQuiescedProof,
    ) -> Result<SafeTransitionCapability<'barriers, 'lease>, SafeTransitionError> {
        workspace
            .authorize_archive(archive, self.workspace_id)
            .map_err(SafeTransitionError::Workspace)?;
        let graph_binding = graph.binding();
        if graph_binding.workspace_id() != self.workspace_id
            || graph_binding.endpoint().endpoint_id() != self.endpoint_id
            || graph_binding.graph_resource_id() != self.graph_resource_id
        {
            return Err(SafeTransitionError::Store(
                StoreError::ResumePointBindingMismatch(
                    "Safe transition graph reservation is bound to another endpoint",
                ),
            ));
        }
        let watcher_binding = watcher.binding();
        if watcher_binding.endpoint != graph_binding.endpoint()
            || watcher_binding.receipt_store_id != self.receipt_store_id
        {
            return Err(SafeTransitionError::Store(
                StoreError::ResumePointBindingMismatch(
                    "Safe transition watcher proof is bound to another endpoint",
                ),
            ));
        }
        Ok(SafeTransitionCapability {
            history: self,
            archive,
            _graph: graph,
            _watcher: watcher,
            workspace,
        })
    }

    /// Remove every recognized Unsafe-bound resume point of this endpoint.
    ///
    /// This is the `Unsafe -> Safe` drain step: afterwards no recognized point
    /// names a retained scratch run. It is ordered before the handoff record
    /// moves to `Safe` on purpose — a crash in between leaves `Unsafe` with no
    /// resume point, which is the conservative full-replay state, whereas the
    /// reverse order would leave a `Safe` record pointing at a stale run.
    ///
    /// It is deliberately **conservative rather than strict**. A `.DS_Store`, a
    /// Syncthing conflict copy, a Dropbox duplicate, an editor backup or a torn
    /// point must not be deleted as if it were authoritative — but it must also
    /// not make the drain permanently impossible, which would pin the endpoint
    /// at `HandoffUnsafe` forever and visibly block handing the graph back to
    /// OG Logseq. So this removes what it fully recognized, preserves every
    /// other byte, and reports the residue in
    /// [`ResumePointMaintenance::preserved`]. While that residue exists no
    /// [`ResumePointSet`] can be minted, so no retained run is reclaimed: the
    /// run leaks, which is the correct trade.
    fn clear_unsafe_resume_points(&self) -> Result<ResumePointMaintenance, StoreError> {
        let _guard = self
            .transition
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?;
        let _workspace_guard = AdvisoryTransitionGuard::lock(&self.transition_lock)?;
        #[cfg(test)]
        inject_resume_clear_fault()?;
        let Some(directory) = open_existing_dir_nofollow(&self.control, RESUME_POINT_DIR)? else {
            return Ok(ResumePointMaintenance::default());
        };
        Ok(clear_resume_points_in(&directory)?)
    }

    #[cfg(test)]
    pub(crate) fn clear_resume_points_for_test(
        &self,
    ) -> Result<ResumePointMaintenance, StoreError> {
        self.clear_unsafe_resume_points()
    }

    /// Turn a durable promoted-runtime state into live write authorization for
    /// this exact bootstrap-anchored lineage.
    ///
    /// This is the only path that unfences a bootstrap-bound durable history.
    /// It requires the caller's expected state to be byte-equal to the durable
    /// state, that state to claim this endpoint, and the live authoritative
    /// root to still carry the exact same bootstrap aggregate binding.
    pub(crate) fn authorize_promoted_lineage(
        &mut self,
        expected: &PromotedRuntimeStateV1,
    ) -> Result<(), StoreError> {
        expected.validate()?;
        self.require_promoted_state_binding(expected)?;
        let durable = self
            .read_promoted_runtime_state()?
            .ok_or(StoreError::PromotedRuntimeStateAbsent)?;
        if &durable != expected {
            return Err(StoreError::PromotedRuntimeStateMismatch(
                "durable promoted runtime state is not the authorized state",
            ));
        }
        let (_, root) = self.read_live_head_root()?;
        match root.binding.bootstrap {
            Some(bootstrap) if bootstrap == durable.bootstrap => {}
            _ => {
                return Err(StoreError::PromotedRuntimeStateMismatch(
                    "durable history bootstrap binding is not the promoted lineage",
                ));
            }
        }
        self.promoted_lineage = Some(durable);
        Ok(())
    }

    /// The promoted lineage this open authorized, if any.
    pub(crate) const fn promoted_lineage(&self) -> Option<&PromotedRuntimeStateV1> {
        self.promoted_lineage.as_ref()
    }

    fn validate_sealed_open(&self) -> Result<(), StoreError> {
        let claim = read_optional_regular(&self.control, ENGINE_HISTORY_CLAIM_FILE, 256, None)?
            .ok_or(StoreError::MalformedHistoryIndex)?;
        validate_engine_history_claim(
            &claim,
            self.workspace_id,
            self.endpoint_id,
            self.graph_resource_id,
            self.receipt_store_id,
        )?;
        let expected = self
            .authoritative_head
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?
            .ok_or(StoreError::MalformedHistoryIndex)?;
        let (live, root) = self.read_live_head_root()?;
        if live != expected {
            return Err(StoreError::MalformedHistoryIndex);
        }
        self.require_root_binding(&root)?;
        // A promoted open stays authorized only while the exact durable
        // promotion state and the exact bootstrap-anchored root binding are
        // both still committed.
        if let Some(authorized) = &self.promoted_lineage {
            match self.read_promoted_runtime_state()? {
                Some(live_state) if &live_state == authorized => {}
                _ => {
                    return Err(StoreError::PromotedRuntimeStateMismatch(
                        "promoted runtime state changed while the enrolled open was sealed",
                    ));
                }
            }
            if root.binding.bootstrap != Some(authorized.bootstrap) {
                return Err(StoreError::PromotedRuntimeStateMismatch(
                    "promoted durable history is no longer the authorized bootstrap lineage",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn lookup(
        &self,
        index_root: ContentDigest,
        batch_id: BatchId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.index.lookup(index_root, batch_id)
    }

    pub(crate) fn materialize(
        &self,
        index_root: ContentDigest,
    ) -> Result<Vec<(BatchId, Vec<u8>)>, StoreError> {
        self.index.materialize(index_root)
    }

    pub(crate) fn note_history_decode(&self) {
        self.index.note_history_decode();
    }

    /// Extend the durable history by one record.
    ///
    /// A publication that starts and does not complete may have failed anywhere
    /// between the head read and the head swap, including on damaged index or
    /// root storage. Nothing this open proved earlier may survive such a
    /// failure as a shortcut, so any outcome other than success or the
    /// read-only bootstrap refusal — which is decided before any storage is
    /// touched — latches the storage fault and disarms the
    /// authenticated-transition memo for the rest of this open.
    pub(crate) fn publish(
        &self,
        batch_id: BatchId,
        bytes: &[u8],
        binding: EngineHistoryBinding,
    ) -> Result<(u64, ContentDigest), StoreError> {
        let _guard = self
            .transition
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?;
        let _workspace_guard = AdvisoryTransitionGuard::lock(&self.transition_lock)?;
        let published = self.publish_locked(batch_id, bytes, binding);
        if !matches!(published, Ok(_) | Err(StoreError::InactiveBootstrapHistory)) {
            self.index.note_storage_fault();
            if let Ok(mut cache) = self.authenticated_transitions.lock() {
                cache.clear();
            }
        }
        published
    }

    fn publish_locked(
        &self,
        batch_id: BatchId,
        bytes: &[u8],
        binding: EngineHistoryBinding,
    ) -> Result<(u64, ContentDigest), StoreError> {
        let (before_digest, before) = self.load_head_root()?;
        // An inactive bootstrap history is read-only. A promoted open may extend
        // exactly the bootstrap lineage its durable promotion state authorized,
        // and the successor below carries that identical binding forward, so the
        // promoted history stays one homogeneous bootstrap-anchored lineage.
        if let Some(bootstrap) = before.binding.bootstrap {
            match &self.promoted_lineage {
                Some(authorized) if authorized.bootstrap == bootstrap => {}
                _ => return Err(StoreError::InactiveBootstrapHistory),
            }
        }
        let index_root = self.index.insert(before.index_root, batch_id, bytes)?;
        if index_root == before.index_root {
            return Ok((before.generation, before.index_root));
        }
        let after = DurableEngineHistoryRoot {
            schema_version: ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            endpoint_id: self.endpoint_id,
            graph_resource_id: self.graph_resource_id,
            receipt_store_id: self.receipt_store_id,
            generation: before
                .generation
                .checked_add(1)
                .ok_or(StoreError::MalformedHistoryIndex)?,
            index_root,
            latest_batch_id: Some(batch_id),
            binding: DurableEngineHistoryBinding {
                engine: binding,
                bootstrap: before.binding.bootstrap,
            },
        };
        let after_digest = self.publish_root(&after)?;
        self.replace_head(before_digest, after_digest)?;
        Ok((after.generation, after.index_root))
    }

    pub(crate) fn begin_publish_many_exact<'a>(
        &'a self,
        publication: &'a ValidatedBootstrapPublicationV1,
        engine_binding: EngineHistoryBinding,
    ) -> Result<ExactBootstrapHistoryBuilderV1<'a>, StoreError> {
        let aggregate = publication.aggregate();
        if aggregate.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: aggregate.workspace_id(),
            });
        }
        let binding = BootstrapAggregateHistoryBindingV1::for_aggregate(aggregate)?;
        if aggregate.parts().len() != binding.part_count() as usize {
            return Err(StoreError::MalformedHistoryIndex);
        }
        if aggregate.parts().is_empty() && engine_binding != EngineHistoryBinding::empty() {
            return Err(StoreError::MalformedHistoryIndex);
        }
        Ok(ExactBootstrapHistoryBuilderV1 {
            store: self,
            expected_parts: aggregate.parts(),
            binding,
            engine_binding,
            index_root: EngineHistoryStore::empty_root(),
            latest: None,
            next_ordinal: 0,
            batch_ids: std::collections::BTreeSet::new(),
        })
    }

    #[cfg(test)]
    pub(crate) fn publish_many_exact(
        &self,
        records: &[PreparedBootstrapHistoryRecordV1<'_>],
        publication: &ValidatedBootstrapPublicationV1,
        engine_binding: EngineHistoryBinding,
    ) -> Result<(u64, ContentDigest), StoreError> {
        let mut builder = self.begin_publish_many_exact(publication, engine_binding)?;
        for record in records {
            builder.push(record)?;
        }
        builder.finish()
    }

    fn initialize(&self) -> Result<(), StoreError> {
        let head = read_optional_regular(&self.control, ENGINE_HISTORY_HEAD_FILE, 64, None)?;
        let claim = read_optional_regular(&self.control, ENGINE_HISTORY_CLAIM_FILE, 256, None)?;
        match (head, claim) {
            (None, None) => {
                let empty = DurableEngineHistoryRoot {
                    schema_version: ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
                    workspace_id: self.workspace_id,
                    endpoint_id: self.endpoint_id,
                    graph_resource_id: self.graph_resource_id,
                    receipt_store_id: self.receipt_store_id,
                    generation: 0,
                    index_root: EngineHistoryStore::empty_root(),
                    latest_batch_id: None,
                    binding: DurableEngineHistoryBinding::ordinary(EngineHistoryBinding::empty()),
                };
                let empty_digest = self.publish_root(&empty)?;
                publish_immutable_exact(
                    &self.control,
                    ENGINE_HISTORY_HEAD_FILE,
                    empty_digest.to_string().as_bytes(),
                    "engine history head",
                )?;
                let expected_claim = postcard::to_allocvec(&(
                    ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
                    self.workspace_id,
                    self.endpoint_id,
                    self.graph_resource_id,
                    self.receipt_store_id,
                ))
                .map_err(|_| StoreError::MalformedHistoryIndex)?;
                publish_immutable_exact(
                    &self.control,
                    ENGINE_HISTORY_CLAIM_FILE,
                    &expected_claim,
                    "engine history claim",
                )?;
            }
            (Some(_), Some(claim)) => validate_engine_history_claim(
                &claim,
                self.workspace_id,
                self.endpoint_id,
                self.graph_resource_id,
                self.receipt_store_id,
            )?,
            _ => return Err(StoreError::MalformedHistoryIndex),
        }
        self.read_live_head_root()?;
        Ok(())
    }

    fn publish_root(&self, root: &DurableEngineHistoryRoot) -> Result<ContentDigest, StoreError> {
        self.require_root_binding(root)?;
        let bytes = postcard::to_allocvec(root).map_err(|_| StoreError::MalformedHistoryIndex)?;
        let digest = ContentDigest::of(&bytes);
        publish_immutable_exact(
            &self.roots,
            &engine_history_root_filename(digest),
            &bytes,
            "engine history authenticated root",
        )?;
        Ok(digest)
    }

    fn load_head_root(&self) -> Result<(ContentDigest, DurableEngineHistoryRoot), StoreError> {
        let sealed = self
            .authoritative_head
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)?
            .to_owned();
        match sealed {
            Some(expected) => {
                let (live, root) = self.read_live_head_root()?;
                if live != expected {
                    return Err(StoreError::MalformedHistoryIndex);
                }
                Ok((live, root))
            }
            None => self.read_live_head_root(),
        }
    }

    fn read_live_head_root(&self) -> Result<(ContentDigest, DurableEngineHistoryRoot), StoreError> {
        let head = read_optional_regular(&self.control, ENGINE_HISTORY_HEAD_FILE, 64, None)?
            .ok_or(StoreError::MalformedHistoryIndex)?;
        let text = std::str::from_utf8(&head).map_err(|_| StoreError::MalformedHistoryIndex)?;
        let digest = parse_digest(text)
            .map(ContentDigest::from_bytes)
            .map_err(|_| StoreError::MalformedHistoryIndex)?;
        if digest.to_string().as_bytes() != head {
            return Err(StoreError::MalformedHistoryIndex);
        }
        Ok((digest, self.load_root(digest)?))
    }

    fn load_root(&self, digest: ContentDigest) -> Result<DurableEngineHistoryRoot, StoreError> {
        let bytes = read_optional_regular(
            &self.roots,
            &engine_history_root_filename(digest),
            MAX_ENGINE_HISTORY_INDEX_BYTES,
            None,
        )?
        .ok_or(StoreError::MalformedHistoryIndex)?;
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::HistoryIndexPathMismatch(digest));
        }
        let root: DurableEngineHistoryRoot =
            postcard::from_bytes(&bytes).map_err(|_| StoreError::MalformedHistoryIndex)?;
        if postcard::to_allocvec(&root).map_err(|_| StoreError::MalformedHistoryIndex)? != bytes {
            return Err(StoreError::MalformedHistoryIndex);
        }
        self.require_root_binding(&root)?;
        Ok(root)
    }

    fn require_root_binding(&self, root: &DurableEngineHistoryRoot) -> Result<(), StoreError> {
        validate_engine_history_root(
            root,
            self.workspace_id,
            self.endpoint_id,
            self.graph_resource_id,
            self.receipt_store_id,
        )
    }

    fn replace_head(
        &self,
        expected: ContentDigest,
        replacement: ContentDigest,
    ) -> Result<(), StoreError> {
        let (current, _) = self.read_live_head_root()?;
        if current != expected {
            return Err(StoreError::MalformedHistoryIndex);
        }
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut temp = self.control.open_with(&temp_name, &options)?;
        let result = (|| {
            temp.write_all(replacement.to_string().as_bytes())?;
            temp.sync_all()?;
            drop(temp);
            #[cfg(test)]
            ENGINE_HISTORY_FAIL_BEFORE_HEAD_SWAP.with(|fail| {
                if fail.replace(false) {
                    return Err(StoreError::Io(std::io::Error::other(
                        "injected engine history failure before authenticated head swap",
                    )));
                }
                Ok(())
            })?;
            self.control
                .rename(&temp_name, &self.control, ENGINE_HISTORY_HEAD_FILE)?;
            #[cfg(test)]
            ENGINE_HISTORY_FAIL_AFTER_HEAD_SWAP.with(|fail| {
                if fail.replace(false) {
                    return Err(StoreError::Io(std::io::Error::other(
                        "injected engine history failure after authenticated head swap",
                    )));
                }
                Ok(())
            })?;
            sync_dir_required(&self.control)?;
            Ok::<_, StoreError>(())
        })();
        let cleanup = self.control.remove_file(&temp_name);
        if let Err(error) = result {
            let _ = cleanup;
            return Err(error);
        }
        if cleanup
            .as_ref()
            .is_err_and(|error| error.kind() != ErrorKind::NotFound)
        {
            cleanup?;
        }
        *self
            .authoritative_head
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)? = Some(replacement);
        Ok(())
    }
}

/// Named durable boundaries of one resume-point publication, after the
/// resume-point directory exists.
///
/// These are the two cuts [`DurableEngineHistoryStore::publish_resume_point`]
/// can leave behind that no other test route can reach: the survey/publish
/// primitives are callable directly, but the *pre-prune* is only ever executed
/// from inside the publication, so a crash between it and the commit point —
/// the only window in which this packet deletes a durable point *before*
/// committing its replacement — is otherwise unobservable. Deterministic
/// injection at each of them proves at least one fully valid point survives
/// every cut.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResumePublishBoundary {
    /// After step 1's pre-prune, before the immutable commit point.
    AfterPrePrune,
    /// After the commit point, before step 3's prune.
    AfterCommit,
}

#[cfg(test)]
impl ResumePublishBoundary {
    /// Every durable boundary of the publication, in publication order.
    pub(crate) const ALL: [Self; 2] = [Self::AfterPrePrune, Self::AfterCommit];
}

#[cfg(test)]
thread_local! {
    /// One-shot publication fault. Thread-local and deterministic: no
    /// process-global resource limit or signal is involved, so parallel tests
    /// in other threads are unaffected.
    static RESUME_PUBLISH_FAULT: std::cell::Cell<Option<ResumePublishBoundary>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_resume_publication_at(boundary: ResumePublishBoundary) {
    RESUME_PUBLISH_FAULT.with(|fault| fault.set(Some(boundary)));
}

#[cfg(test)]
thread_local! {
    static RESUME_CLEAR_FAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn fail_next_resume_clear() {
    RESUME_CLEAR_FAULT.with(|fault| fault.set(true));
}

#[cfg(test)]
fn inject_resume_clear_fault() -> Result<(), StoreError> {
    RESUME_CLEAR_FAULT.with(|fault| {
        if fault.replace(false) {
            Err(StoreError::Io(std::io::Error::other(
                "injected resume-point clear failure before the first removal",
            )))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
fn inject_resume_publish_fault(boundary: ResumePublishBoundary) -> Result<(), StoreError> {
    RESUME_PUBLISH_FAULT.with(|fault| {
        if fault.get() == Some(boundary) {
            fault.set(None);
            return Err(StoreError::Io(std::io::Error::other(format!(
                "injected resume-point publication failure at {boundary:?}"
            ))));
        }
        Ok(())
    })
}

fn validate_engine_history_root(
    root: &DurableEngineHistoryRoot,
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
) -> Result<(), StoreError> {
    if root.schema_version < ENGINE_HISTORY_ROOT_SCHEMA_VERSION {
        return Err(StoreError::UpgradeRequired {
            store: "engine history",
            found: root.schema_version,
            current: ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
        });
    }
    if root.schema_version > ENGINE_HISTORY_ROOT_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedStoreVersion {
            store: "engine history",
            version: root.schema_version,
        });
    }
    if root.workspace_id != workspace_id
        || root.endpoint_id != endpoint_id
        || root.graph_resource_id != graph_resource_id
        || root.receipt_store_id != receipt_store_id
        || root.binding.engine.portable_path_key_version != super::PORTABLE_PATH_KEY_VERSION
        || (root.generation == 0) != root.latest_batch_id.is_none()
        // A bootstrap-anchored root's generation may never be behind the parts
        // its aggregate installed. It is *equal* while the bootstrap is still
        // inactive, and grows past it once a promoted runtime extends the same
        // lineage. Exact-equality remains enforced where it is the actual
        // requirement: bootstrap installation, inactive accepted-authority
        // reopen, and the first promotion's unadvanced-anchor gate. Ordinary
        // enrolled opens refuse every bootstrap-anchored history outright, and
        // `publish` refuses to extend one without an authorized promoted
        // lineage, so a relaxed root shape grants no write path.
        || root.binding.bootstrap.is_some_and(|binding| {
            u64::from(binding.part_count()) > root.generation
                || binding.final_frontier().accepted_count() != binding.part_count()
                || (root.generation == 0 && root.binding.engine != EngineHistoryBinding::empty())
        })
        || root
            .binding
            .engine
            .portable_path_conflicts
            .windows(2)
            .any(|pair| pair[0].key_digest() >= pair[1].key_digest())
        || root
            .binding
            .engine
            .portable_path_conflicts
            .iter()
            .any(|conflict| {
                conflict.key_version() != super::PORTABLE_PATH_KEY_VERSION
                    || conflict.participants().len() < 2
                    || conflict
                        .participants()
                        .windows(2)
                        .any(|pair| pair[0] >= pair[1])
            })
        || (!root.binding.engine.portable_path_conflicts.is_empty()
            && root.binding.engine.terminal_evidence.is_none())
    {
        return Err(StoreError::MalformedHistoryIndex);
    }
    root.binding.engine.page_names.validate()?;
    Ok(())
}

impl BlockClaimIndexStore {
    fn with_file<T>(
        &self,
        operation: impl FnOnce(&mut fs::File) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        match &self.backing {
            BlockClaimIndexBacking::Scratch(scratch) => scratch
                .with_pages(operation)
                .map_err(|error| StoreError::Scratch(error.to_string()))?,
            #[cfg(test)]
            BlockClaimIndexBacking::Standalone(file) => {
                let mut file = file
                    .lock()
                    .map_err(|_| StoreError::MalformedBlockClaimIndex)?;
                operation(&mut file)
            }
        }
    }

    pub(crate) fn lookup_many(
        &self,
        root: BlockClaimIndexRoot,
        keys: &[[u8; 16]],
    ) -> Result<BTreeMap<[u8; 16], BlockClaimIndexValue>, StoreError> {
        if keys.is_empty() || root.levels.iter().flatten().all(Option::is_none) {
            return Ok(BTreeMap::new());
        }
        if !keys.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(StoreError::MalformedBlockClaimIndex);
        }
        self.with_file(|file| {
            let mut segments: Vec<_> = root.levels.into_iter().flatten().flatten().collect();
            segments.sort_unstable_by_key(|segment| std::cmp::Reverse(segment.generation));
            let mut remaining: Vec<_> = keys
                .iter()
                .copied()
                .map(|key| {
                    let (first, second) = block_claim_filter_hashes(&key);
                    (key, first, second)
                })
                .collect();
            let global_filter = self.read_claim_global_filter(
                file,
                root.global_filter
                    .ok_or(StoreError::MalformedBlockClaimIndex)?,
            )?;
            remaining.retain(|(_, first, second)| {
                block_claim_global_filter_might_contain(&global_filter, *first, *second)
            });
            if remaining.is_empty() {
                return Ok(BTreeMap::new());
            }
            let mut found = BTreeMap::new();
            for segment in segments {
                let filter = self.read_claim_filter(file, segment.filter_ref)?;
                if filter.entry_count != segment.entry_count {
                    return Err(StoreError::MalformedBlockClaimIndex);
                }
                let selected: Vec<_> = remaining
                    .iter()
                    .filter(|(_, first, second)| {
                        block_claim_filter_might_contain(&filter, *first, *second)
                    })
                    .map(|(key, _, _)| *key)
                    .collect();
                if selected.is_empty() {
                    continue;
                }
                let mut segment_found = BTreeMap::new();
                self.lookup_many_at(file, segment.page_ref, 0, &selected, &mut segment_found)?;
                found.extend(segment_found);
                remaining.retain(|(key, _, _)| !found.contains_key(key));
                if remaining.is_empty() {
                    break;
                }
            }
            Ok(found)
        })
    }

    pub(crate) fn insert_many(
        &self,
        root: BlockClaimIndexRoot,
        records: &[([u8; 16], BlockClaimIndexValue)],
    ) -> Result<BlockClaimIndexRoot, StoreError> {
        if records.is_empty() {
            return Ok(root);
        }
        if !records.windows(2).all(|pair| pair[0].0 < pair[1].0)
            || records
                .iter()
                .any(|(_, record)| record.is_empty() || record.len() > MAX_BLOCK_CLAIM_RECORD_BYTES)
        {
            return Err(StoreError::MalformedBlockClaimIndex);
        }
        self.with_file(|file| {
            let generation = root
                .next_generation
                .checked_add(1)
                .ok_or(StoreError::MalformedBlockClaimIndex)?;
            let mut global_filter = match root.global_filter {
                Some(page_ref) => self.read_claim_global_filter(file, page_ref)?,
                None => new_block_claim_global_filter(),
            };
            update_block_claim_global_filter(&mut global_filter, records)?;
            let mut next = root;
            next.next_generation = generation;
            let mut merged = records.to_vec();
            let mut installed = false;
            for level in &mut next.levels {
                if let Some(empty) = level.iter().position(Option::is_none) {
                    let entry_count = u64::try_from(merged.len())
                        .map_err(|_| StoreError::MalformedBlockClaimIndex)?;
                    let filter_ref = self.append_claim_filter(file, &merged)?;
                    let page_ref = self.build_claim_subtree(file, 0, merged)?;
                    level[empty] = Some(BlockClaimSegmentRef {
                        generation,
                        entry_count,
                        page_ref,
                        filter_ref,
                    });
                    installed = true;
                    break;
                }
                let mut existing: Vec<_> = level.iter_mut().filter_map(Option::take).collect();
                existing.sort_unstable_by_key(|segment| segment.generation);
                let capacity = existing.iter().try_fold(merged.len(), |capacity, segment| {
                    usize::try_from(segment.entry_count)
                        .ok()
                        .and_then(|entries| capacity.checked_add(entries))
                });
                let mut combined =
                    AHashMap::with_capacity(capacity.ok_or(StoreError::MalformedBlockClaimIndex)?);
                for segment in existing {
                    let mut older = Vec::with_capacity(
                        usize::try_from(segment.entry_count)
                            .map_err(|_| StoreError::MalformedBlockClaimIndex)?,
                    );
                    self.materialize_claim_segment(file, segment.page_ref, 0, &mut older)?;
                    if older.len() as u64 != segment.entry_count {
                        return Err(StoreError::MalformedBlockClaimIndex);
                    }
                    combined.extend(older);
                }
                combined.extend(merged);
                merged = combined.into_iter().collect();
            }
            if !installed {
                return Err(StoreError::MalformedBlockClaimIndex);
            }
            next.global_filter = Some(self.append_claim_global_filter(file, &global_filter)?);
            Ok(next)
        })
    }

    fn lookup_many_at(
        &self,
        file: &mut fs::File,
        page_ref: BlockClaimPageRef,
        expected_depth: u8,
        keys: &[[u8; 16]],
        found: &mut BTreeMap<[u8; 16], BlockClaimIndexValue>,
    ) -> Result<(), StoreError> {
        match self.read_claim_page(file, page_ref, expected_depth)? {
            BlockClaimIndexPage::Leaf { entries, .. } => {
                for key in keys {
                    if let Ok(index) =
                        entries.binary_search_by_key(key, |(candidate, _)| *candidate)
                    {
                        found.insert(*key, entries[index].1.clone());
                    }
                }
            }
            BlockClaimIndexPage::Branch {
                depth, children, ..
            } => {
                let mut grouped = BTreeMap::<u8, Vec<[u8; 16]>>::new();
                for key in keys {
                    grouped
                        .entry(block_claim_key_nibble(key, depth))
                        .or_default()
                        .push(*key);
                }
                for (nibble, selected) in grouped {
                    if let Ok(index) =
                        children.binary_search_by_key(&nibble, |(candidate, _)| *candidate)
                    {
                        self.lookup_many_at(file, children[index].1, depth + 1, &selected, found)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn build_claim_subtree(
        &self,
        file: &mut fs::File,
        depth: u8,
        mut entries: Vec<([u8; 16], BlockClaimIndexValue)>,
    ) -> Result<BlockClaimPageRef, StoreError> {
        let estimated_encoded_bytes = entries.iter().try_fold(32_usize, |total, (_, record)| {
            total.checked_add(26)?.checked_add(record.len())
        });
        if (entries.len() <= BLOCK_CLAIM_LEAF_ENTRIES
            && estimated_encoded_bytes.is_some_and(|bytes| bytes <= MAX_BLOCK_CLAIM_PAGE_BYTES))
            || depth == BLOCK_CLAIM_RADIX_DEPTH
        {
            entries.sort_unstable_by_key(|entry| entry.0);
            return self.append_claim_page(
                file,
                &BlockClaimIndexPage::Leaf {
                    schema_version: BLOCK_CLAIM_INDEX_SCHEMA_VERSION,
                    depth,
                    entries,
                },
            );
        }
        let mut grouped = BTreeMap::<u8, Vec<([u8; 16], BlockClaimIndexValue)>>::new();
        for entry in entries {
            grouped
                .entry(block_claim_key_nibble(&entry.0, depth))
                .or_default()
                .push(entry);
        }
        let mut children = Vec::with_capacity(grouped.len());
        for (nibble, selected) in grouped {
            children.push((nibble, self.build_claim_subtree(file, depth + 1, selected)?));
        }
        self.append_claim_page(
            file,
            &BlockClaimIndexPage::Branch {
                schema_version: BLOCK_CLAIM_INDEX_SCHEMA_VERSION,
                depth,
                children,
            },
        )
    }

    fn append_claim_page(
        &self,
        file: &mut fs::File,
        page: &BlockClaimIndexPage,
    ) -> Result<BlockClaimPageRef, StoreError> {
        validate_block_claim_page(page)?;
        let bytes =
            postcard::to_allocvec(page).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        self.append_claim_bytes(file, &bytes)
    }

    fn append_claim_filter(
        &self,
        file: &mut fs::File,
        entries: &[([u8; 16], BlockClaimIndexValue)],
    ) -> Result<BlockClaimPageRef, StoreError> {
        let filter = new_block_claim_filter(entries)?;
        let bytes =
            postcard::to_allocvec(&filter).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        self.append_claim_bytes(file, &bytes)
    }

    fn append_claim_global_filter(
        &self,
        file: &mut fs::File,
        filter: &BlockClaimGlobalFilterPage,
    ) -> Result<BlockClaimPageRef, StoreError> {
        validate_block_claim_global_filter(filter)?;
        let bytes =
            postcard::to_allocvec(filter).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        self.append_claim_bytes(file, &bytes)
    }

    fn append_claim_bytes(
        &self,
        file: &mut fs::File,
        bytes: &[u8],
    ) -> Result<BlockClaimPageRef, StoreError> {
        if bytes.len() > MAX_BLOCK_CLAIM_PAGE_BYTES {
            return Err(StoreError::StoredFileTooLarge {
                path: BLOCK_CLAIM_INDEX_FILE.into(),
                length: bytes.len() as u64,
                limit: MAX_BLOCK_CLAIM_PAGE_BYTES as u64,
            });
        }
        let encoded_len =
            u32::try_from(bytes.len()).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        let offset = file.seek(SeekFrom::End(0))?;
        file.write_all(&encoded_len.to_be_bytes())?;
        file.write_all(bytes)?;
        self.counters
            .block_claim_index_writes
            .fetch_add(1, Ordering::Relaxed);
        Ok(BlockClaimPageRef {
            offset,
            encoded_len,
            digest: ContentDigest::of(bytes),
        })
    }

    fn materialize_claim_segment(
        &self,
        file: &mut fs::File,
        page_ref: BlockClaimPageRef,
        expected_depth: u8,
        entries: &mut Vec<([u8; 16], BlockClaimIndexValue)>,
    ) -> Result<(), StoreError> {
        match self.read_claim_page(file, page_ref, expected_depth)? {
            BlockClaimIndexPage::Leaf {
                entries: selected, ..
            } => entries.extend(selected),
            BlockClaimIndexPage::Branch {
                depth, children, ..
            } => {
                for (_, child) in children {
                    self.materialize_claim_segment(file, child, depth + 1, entries)?;
                }
            }
        }
        Ok(())
    }

    fn read_claim_page(
        &self,
        file: &mut fs::File,
        page_ref: BlockClaimPageRef,
        expected_depth: u8,
    ) -> Result<BlockClaimIndexPage, StoreError> {
        let bytes = self.read_claim_bytes(file, page_ref)?;
        let page: BlockClaimIndexPage =
            postcard::from_bytes(&bytes).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        validate_block_claim_page(&page)?;
        if block_claim_page_depth(&page) != expected_depth
            || postcard::to_allocvec(&page).map_err(|_| StoreError::MalformedBlockClaimIndex)?
                != bytes
        {
            return Err(StoreError::MalformedBlockClaimIndex);
        }
        Ok(page)
    }

    fn read_claim_filter(
        &self,
        file: &mut fs::File,
        page_ref: BlockClaimPageRef,
    ) -> Result<BlockClaimFilterPage, StoreError> {
        let bytes = self.read_claim_bytes(file, page_ref)?;
        let filter: BlockClaimFilterPage =
            postcard::from_bytes(&bytes).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        validate_block_claim_filter(&filter)?;
        if postcard::to_allocvec(&filter).map_err(|_| StoreError::MalformedBlockClaimIndex)?
            != bytes
        {
            return Err(StoreError::MalformedBlockClaimIndex);
        }
        Ok(filter)
    }

    fn read_claim_global_filter(
        &self,
        file: &mut fs::File,
        page_ref: BlockClaimPageRef,
    ) -> Result<BlockClaimGlobalFilterPage, StoreError> {
        let bytes = self.read_claim_bytes(file, page_ref)?;
        let filter: BlockClaimGlobalFilterPage =
            postcard::from_bytes(&bytes).map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        validate_block_claim_global_filter(&filter)?;
        if postcard::to_allocvec(&filter).map_err(|_| StoreError::MalformedBlockClaimIndex)?
            != bytes
        {
            return Err(StoreError::MalformedBlockClaimIndex);
        }
        Ok(filter)
    }

    fn read_claim_bytes(
        &self,
        file: &mut fs::File,
        page_ref: BlockClaimPageRef,
    ) -> Result<Vec<u8>, StoreError> {
        file.seek(SeekFrom::Start(page_ref.offset))?;
        let mut length = [0_u8; 4];
        file.read_exact(&mut length)?;
        let found_len = u32::from_be_bytes(length);
        if found_len != page_ref.encoded_len
            || usize::try_from(found_len)
                .ok()
                .is_none_or(|length| length == 0 || length > MAX_BLOCK_CLAIM_PAGE_BYTES)
        {
            return Err(StoreError::MalformedBlockClaimIndex);
        }
        let mut bytes = vec![0_u8; found_len as usize];
        file.read_exact(&mut bytes)?;
        if ContentDigest::of(&bytes) != page_ref.digest {
            return Err(StoreError::BlockClaimIndexPathMismatch(page_ref.digest));
        }
        self.counters
            .block_claim_index_reads
            .fetch_add(1, Ordering::Relaxed);
        Ok(bytes)
    }
}

#[derive(Clone, Copy)]
enum NamespaceKind {
    Objects,
    Batches,
}

#[derive(Clone)]
enum Collision {
    Object(ContentDigest),
    Batch(BatchId),
    HistoryIndex(ContentDigest),
    Lineage(LineageDigest),
    Exact(&'static str),
    Bootstrap(&'static str, String),
}

fn ensure_single_lineage(manifests: &[OperationBatch]) -> Result<(), StoreError> {
    if let Some(first) = manifests.first() {
        for manifest in &manifests[1..] {
            if manifest.lineage_digest() != first.lineage_digest() {
                return Err(StoreError::LineageMismatch {
                    expected: first.lineage_digest(),
                    found: manifest.lineage_digest(),
                });
            }
        }
    }
    Ok(())
}

fn require_lineage_bytes(expected: LineageDigest, bytes: &[u8]) -> Result<(), StoreError> {
    let found_bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| StoreError::MalformedPath(LINEAGE_CLAIM_FILE.into()))?;
    let found = LineageDigest::from_bytes(found_bytes);
    if found != expected {
        return Err(StoreError::LineageMismatch { expected, found });
    }
    Ok(())
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Batch(BatchError),
    Bootstrap(String),
    UnsafeEntry(String),
    MalformedPath(String),
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    LineageMismatch {
        expected: LineageDigest,
        found: LineageDigest,
    },
    ObjectCollision(ContentDigest),
    BatchCollision(BatchId),
    ObjectPathMismatch(ContentDigest),
    ManifestPathMismatch {
        expected: BatchId,
        found: BatchId,
    },
    AcceptedManifestMismatch {
        batch_id: BatchId,
        expected: ContentDigest,
        actual: ContentDigest,
    },
    AcceptedDocumentUpdateMissing {
        batch_id: BatchId,
        document_id: super::DocumentId,
    },
    HistoryIndexCollision(BatchId),
    HistoryIndexPathMismatch(ContentDigest),
    MalformedHistoryIndex,
    UpgradeRequired {
        store: &'static str,
        found: u32,
        current: u32,
    },
    UnsupportedStoreVersion {
        store: &'static str,
        version: u32,
    },
    BlockClaimIndexPathMismatch(ContentDigest),
    MalformedBlockClaimIndex,
    MissingLogseqClaimIndexNode(ContentDigest),
    LogseqClaimIndexPathMismatch(ContentDigest),
    MalformedLogseqClaimIndex,
    MissingExactLogicalPageNameBlob(ContentDigest),
    ExactLogicalPageNameBlobPathMismatch(ContentDigest),
    MalformedPageNameIndex,
    PageNamePointBatchTooLarge {
        actual: usize,
        limit: usize,
    },
    NonCanonicalPageNamePointKeys,
    MissingPageNameCatalogFrontier,
    MisboundPageNameCatalogFrontier,
    Scratch(String),
    LineageClaimCollision(LineageDigest),
    ImmutableCollision(&'static str),
    BootstrapArtifactCollision {
        kind: &'static str,
        identity: String,
    },
    BootstrapArtifactMismatch(&'static str),
    MissingBootstrapArtifact(&'static str),
    BootstrapBatchRequiresDirectPublication,
    BootstrapHistoryRequiresEmptyAuthority,
    InactiveBootstrapHistory,
    PromotedRuntimeStateAbsent,
    PromotedRuntimeStateMismatch(&'static str),
    MalformedPromotedRuntimeState,
    UnsupportedPromotedRuntimeSchema(u32),
    CompetingRuntimePromotion,
    /// One resume point, or one complete resume-point scan, was refused. Every
    /// shape means the same thing to a caller: do not adopt, do not prune, do
    /// not reclaim, preserve every candidate retained run.
    ResumePoint(String),
    ResumePointBindingMismatch(&'static str),
    ResumePointSequenceRegression {
        expected: u64,
        found: u64,
    },
    /// An adopted retained run authenticated as a real retained run of this
    /// workspace, but is not the exact run the caller named: its canonical
    /// marker digest differs, which is what a re-created run reusing the same
    /// UUID looks like. Nothing was changed; the caller must replay instead.
    RetainedScratchBindingMismatch,
    StoredLengthMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    StoredFileTooLarge {
        path: String,
        length: u64,
        limit: u64,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Batch(error) => error.fmt(f),
            Self::Bootstrap(error) => error.fmt(f),
            Self::UnsafeEntry(message) => write!(f, "unsafe store entry: {message}"),
            Self::MalformedPath(path) => write!(f, "malformed store path: {path}"),
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::LineageMismatch { expected, found } => {
                write!(f, "lineage mismatch: expected {expected}, found {found}")
            }
            Self::ObjectCollision(digest) => write!(f, "content-address collision at {digest}"),
            Self::BatchCollision(batch_id) => {
                write!(f, "fatal manifest collision for batch {batch_id}")
            }
            Self::ObjectPathMismatch(digest) => {
                write!(f, "stored object bytes do not match path {digest}")
            }
            Self::ManifestPathMismatch { expected, found } => write!(
                f,
                "manifest path names batch {expected}, but bytes name {found}"
            ),
            Self::AcceptedManifestMismatch {
                batch_id,
                expected,
                actual,
            } => write!(
                f,
                "accepted manifest {batch_id} fingerprint mismatch: expected {expected}, found {actual}"
            ),
            Self::AcceptedDocumentUpdateMissing {
                batch_id,
                document_id,
            } => write!(
                f,
                "accepted manifest {batch_id} has no CRDT update for document {document_id}"
            ),
            Self::HistoryIndexCollision(batch_id) => {
                write!(
                    f,
                    "authenticated history index collision for batch {batch_id}"
                )
            }
            Self::HistoryIndexPathMismatch(digest) => {
                write!(
                    f,
                    "authenticated history index bytes do not match path {digest}"
                )
            }
            Self::MalformedHistoryIndex => {
                f.write_str("authenticated history index is malformed or non-canonical")
            }
            Self::UpgradeRequired {
                store,
                found,
                current,
            } => write!(f, "{store} version {found} requires upgrade to {current}"),
            Self::UnsupportedStoreVersion { store, version } => {
                write!(f, "{store} version {version} is unsupported")
            }
            Self::BlockClaimIndexPathMismatch(digest) => write!(
                f,
                "authenticated block-claim index bytes do not match page {digest}"
            ),
            Self::MalformedBlockClaimIndex => {
                f.write_str("authenticated block-claim index is malformed or non-canonical")
            }
            Self::MissingLogseqClaimIndexNode(digest) => {
                write!(
                    f,
                    "authenticated Logseq claim index node {digest} is missing"
                )
            }
            Self::LogseqClaimIndexPathMismatch(digest) => write!(
                f,
                "authenticated Logseq claim index bytes do not match path {digest}"
            ),
            Self::MalformedLogseqClaimIndex => {
                f.write_str("authenticated Logseq claim index is malformed or non-canonical")
            }
            Self::MissingExactLogicalPageNameBlob(digest) => {
                write!(f, "exact logical page-name blob {digest} is missing")
            }
            Self::ExactLogicalPageNameBlobPathMismatch(digest) => {
                write!(
                    f,
                    "exact logical page-name blob bytes do not match path {digest}"
                )
            }
            Self::MalformedPageNameIndex => {
                f.write_str("authenticated page-name ownership index is malformed or non-canonical")
            }
            Self::PageNamePointBatchTooLarge { actual, limit } => write!(
                f,
                "page-name point batch has {actual} entries, exceeding {limit}"
            ),
            Self::NonCanonicalPageNamePointKeys => {
                f.write_str("page-name point keys are not strictly sorted and unique")
            }
            Self::MissingPageNameCatalogFrontier => {
                f.write_str("exact page-name catalog-frontier binding is missing")
            }
            Self::MisboundPageNameCatalogFrontier => {
                f.write_str("exact page-name catalog-frontier binding is misbound")
            }
            Self::Scratch(error) => write!(f, "engine scratch failed: {error}"),
            Self::LineageClaimCollision(lineage) => {
                write!(f, "immutable lineage claim collision for {lineage}")
            }
            Self::ImmutableCollision(kind) => {
                write!(f, "immutable {kind} collision")
            }
            Self::BootstrapArtifactCollision { kind, identity } => {
                write!(f, "immutable bootstrap {kind} collision at {identity}")
            }
            Self::BootstrapArtifactMismatch(kind) => {
                write!(f, "bootstrap {kind} does not match its direct authority")
            }
            Self::MissingBootstrapArtifact(kind) => {
                write!(f, "required bootstrap {kind} is missing")
            }
            Self::BootstrapBatchRequiresDirectPublication => {
                f.write_str("bootstrap batches require bootstrap-specific direct publication")
            }
            Self::BootstrapHistoryRequiresEmptyAuthority => {
                f.write_str("bootstrap history installation requires empty durable authority")
            }
            Self::InactiveBootstrapHistory => {
                f.write_str("inactive bootstrap history cannot be opened as ordinary runtime")
            }
            Self::PromotedRuntimeStateAbsent => {
                f.write_str("no durable promoted runtime state authorizes this archive")
            }
            Self::PromotedRuntimeStateMismatch(detail) => {
                write!(f, "promoted runtime state mismatch: {detail}")
            }
            Self::MalformedPromotedRuntimeState => {
                f.write_str("promoted runtime state is malformed, truncated, or non-canonical")
            }
            Self::UnsupportedPromotedRuntimeSchema(version) => write!(
                f,
                "unsupported promoted runtime state schema version {version}"
            ),
            Self::CompetingRuntimePromotion => f.write_str(
                "a different promoted runtime state is already committed for this archive",
            ),
            Self::ResumePoint(error) => write!(f, "{error}"),
            Self::ResumePointBindingMismatch(reason) => {
                write!(f, "runtime resume-point binding mismatch: {reason}")
            }
            Self::ResumePointSequenceRegression { expected, found } => write!(
                f,
                "runtime resume point {found} does not extend the published sequence {expected}"
            ),
            Self::RetainedScratchBindingMismatch => f.write_str(
                "retained scratch run does not carry the named canonical marker binding",
            ),
            Self::StoredLengthMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "stored file length mismatch at {path}: expected {expected}, found {actual}"
            ),
            Self::StoredFileTooLarge {
                path,
                length,
                limit,
            } => write!(
                f,
                "stored file at {path} is {length} bytes, exceeding limit {limit}"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Batch(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BatchError> for StoreError {
    fn from(error: BatchError) -> Self {
        Self::Batch(error)
    }
}

impl From<BootstrapImportError> for StoreError {
    fn from(error: BootstrapImportError) -> Self {
        Self::Bootstrap(error.to_string())
    }
}

impl From<ResumePointError> for StoreError {
    fn from(error: ResumePointError) -> Self {
        Self::ResumePoint(error.to_string())
    }
}

fn validate_engine_history_claim(
    bytes: &[u8],
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
) -> Result<(), StoreError> {
    type CurrentClaim = (
        u32,
        WorkspaceId,
        super::ProjectionEndpointId,
        super::CanonicalGraphResourceId,
        super::ProjectionReceiptStoreId,
    );
    if let Ok(claim) = postcard::from_bytes::<CurrentClaim>(bytes) {
        if postcard::to_allocvec(&claim).ok().as_deref() != Some(bytes) {
            return Err(StoreError::MalformedHistoryIndex);
        }
        if claim.0 < ENGINE_HISTORY_ROOT_SCHEMA_VERSION {
            return Err(StoreError::UpgradeRequired {
                store: "engine history",
                found: claim.0,
                current: ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
            });
        }
        if claim.0 > ENGINE_HISTORY_ROOT_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedStoreVersion {
                store: "engine history",
                version: claim.0,
            });
        }
        if claim.1 != workspace_id
            || claim.2 != endpoint_id
            || claim.3 != graph_resource_id
            || claim.4 != receipt_store_id
        {
            return Err(StoreError::MalformedHistoryIndex);
        }
        return Ok(());
    }
    type PriorClaim = (
        u32,
        WorkspaceId,
        super::ProjectionEndpointId,
        super::CanonicalGraphResourceId,
    );
    if let Ok(claim) = postcard::from_bytes::<PriorClaim>(bytes) {
        if postcard::to_allocvec(&claim).ok().as_deref() == Some(bytes)
            && claim.0 == ENGINE_HISTORY_ROOT_SCHEMA_VERSION - 1
        {
            return Err(StoreError::UpgradeRequired {
                store: "engine history",
                found: claim.0,
                current: ENGINE_HISTORY_ROOT_SCHEMA_VERSION,
            });
        }
    }
    Err(StoreError::MalformedHistoryIndex)
}

pub(crate) fn open_existing_dir_nofollow(
    root: &Dir,
    name: &str,
) -> Result<Option<Dir>, StoreError> {
    tine_storage::open_existing_dir_nofollow(root, name).map_err(filesystem_error_without_collision)
}

#[cfg(unix)]
pub(crate) fn control_directory_identity(
    dir: &Dir,
) -> Result<ControlDirectoryIdentity, StoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = dir.try_clone()?.into_std_file().metadata()?;
    Ok(ControlDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
pub(crate) fn control_directory_identity(
    dir: &Dir,
) -> Result<ControlDirectoryIdentity, StoreError> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let file = dir.try_clone()?.into_std_file();
    let mut information = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(StoreError::Io(std::io::Error::last_os_error()));
    }
    Ok(ControlDirectoryIdentity {
        volume: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn control_directory_identity(
    _dir: &Dir,
) -> Result<ControlDirectoryIdentity, StoreError> {
    Err(StoreError::Io(std::io::Error::new(
        ErrorKind::Unsupported,
        "directory identity is unavailable on this platform",
    )))
}

pub(crate) fn ensure_directory_nofollow(root: &Dir, name: &str) -> Result<(), StoreError> {
    tine_storage::ensure_directory_nofollow(root, name).map_err(filesystem_error_without_collision)
}

fn ensure_directory_for_lifecycle(
    root: &Dir,
    name: &str,
    lifecycle: ObjectStoreLifecycle,
) -> Result<(), StoreError> {
    let _ = lifecycle;
    #[cfg(target_os = "android")]
    if lifecycle == ObjectStoreLifecycle::ReconstructibleActivation {
        let component = Path::new(name);
        if !matches!(component.components().next(), Some(Component::Normal(_)))
            || component.components().count() != 1
        {
            return Err(StoreError::UnsafeEntry(format!(
                "managed private directory name is not one normal component: {name}"
            )));
        }
        match root.symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(StoreError::UnsafeEntry(format!(
                    "managed private directory is not a real no-follow directory: {name}"
                )));
            }
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => match root.create_dir(component) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(StoreError::Io(error)),
            },
            Err(error) => return Err(StoreError::Io(error)),
        }
        let metadata = root.symlink_metadata(component)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::UnsafeEntry(format!(
                "managed private directory is not a real no-follow directory: {name}"
            )));
        }
        // The object store lives in Android's app-private area. Some devices
        // permit the create and every file fsync but reject directory fsync.
        // Before promotion the whole tree is reconstructible from Markdown;
        // accepting only that platform capability refusal avoids treating a
        // missing filesystem primitive as a permission/ownership failure.
        crate::filesystem_durability::sync_reconstructible_directory(root)
            .map_err(StoreError::Io)?;
        return Ok(());
    }

    ensure_directory_nofollow(root, name)
}

/// Create only the immediate parent of an explicitly bound object-store root.
/// The grandparent must already exist; the final parent component is opened
/// no-follow and its creation is durability-synced before store construction.
pub(crate) fn prepare_object_store_parent_nofollow(root: &Path) -> Result<(), StoreError> {
    prepare_object_store_parent(root, ObjectStoreLifecycle::Authority)
}

pub(crate) fn prepare_reconstructible_object_store_parent(root: &Path) -> Result<(), StoreError> {
    prepare_object_store_parent(root, ObjectStoreLifecycle::ReconstructibleActivation)
}

fn prepare_object_store_parent(
    root: &Path,
    lifecycle: ObjectStoreLifecycle,
) -> Result<(), StoreError> {
    let parent = root
        .parent()
        .ok_or_else(|| StoreError::UnsafeEntry("store root has no parent".into()))?;
    let name = parent
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StoreError::UnsafeEntry("store parent is not UTF-8".into()))?;
    if !matches!(parent.components().next_back(), Some(Component::Normal(_))) {
        return Err(StoreError::UnsafeEntry(
            "store parent must end in a normal path component".into(),
        ));
    }
    let grandparent = parent
        .parent()
        .ok_or_else(|| StoreError::UnsafeEntry("store parent has no grandparent".into()))?;
    let canonical_grandparent = fs::canonicalize(grandparent)?;
    let grandparent = Dir::open_ambient_dir(&canonical_grandparent, ambient_authority())?;
    ensure_directory_for_lifecycle(&grandparent, name, lifecycle)
}

impl BootstrapPublicationBatch<'_> {
    fn stage(
        &mut self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
        collision: Collision,
    ) -> Result<(), StoreError> {
        self.physical
            .publish(dir, filename, bytes)
            .map_err(|error| publication_error(error, collision))
    }

    #[cfg(test)]
    const fn retained_artifact_handle_count(&self) -> usize {
        0
    }

    pub(crate) fn publish_source_inventory_page(
        &mut self,
        root: SourceInventoryRootV1,
        page: &SourceInventoryIndexPageV1,
    ) -> Result<(), StoreError> {
        if self.inventory_root.is_some_and(|bound| bound != root) {
            return Err(StoreError::BootstrapArtifactMismatch(
                "source inventory batch root",
            ));
        }
        let dir = self.store.bootstrap_index_root_dir(
            BOOTSTRAP_SOURCE_INVENTORY_DIR,
            root.digest(),
            true,
        )?;
        let bytes = page.encode()?;
        let filename = bootstrap_page_filename(page.page_ordinal());
        self.stage(
            &dir,
            &filename,
            &bytes,
            Collision::Bootstrap(
                "source inventory page",
                format!("{}/{}", hex_bytes(root.digest()), page.page_ordinal()),
            ),
        )?;
        self.inventory_root = Some(root);
        self.inventory_pages.insert(page.page_ordinal(), ());
        Ok(())
    }

    pub(crate) fn publish_source_blob_page(
        &mut self,
        root: SourceBlobChunkRootV1,
        page: &SourceBlobIndexPageV1,
    ) -> Result<(), StoreError> {
        if self.blob_root.is_some_and(|bound| bound != root) {
            return Err(StoreError::BootstrapArtifactMismatch(
                "source blob batch root",
            ));
        }
        let dir =
            self.store
                .bootstrap_index_root_dir(BOOTSTRAP_SOURCE_BLOB_DIR, root.digest(), true)?;
        let bytes = page.encode()?;
        let filename = bootstrap_page_filename(page.page_ordinal());
        self.stage(
            &dir,
            &filename,
            &bytes,
            Collision::Bootstrap(
                "source blob page",
                format!("{}/{}", hex_bytes(root.digest()), page.page_ordinal()),
            ),
        )?;
        self.blob_root = Some(root);
        self.blob_pages.insert(page.page_ordinal(), ());
        Ok(())
    }

    pub(crate) fn publish_part_pack(
        &mut self,
        descriptor: BootstrapPartDescriptorV1,
        source: &mut (impl Read + Seek),
        exact_length: u64,
    ) -> Result<(), StoreError> {
        if exact_length > MAX_BOOTSTRAP_PART_PACK_BYTES {
            return Err(StoreError::BootstrapArtifactMismatch(
                "bootstrap part object pack length",
            ));
        }
        let dir = self
            .store
            .bootstrap_namespace(BOOTSTRAP_PART_PACKS_DIR, true)?;
        let part_name = hex_bytes(descriptor.part_id().as_bytes());
        let final_name = part_name.clone();
        source.seek(SeekFrom::Start(0))?;
        let staged = tine_storage::StagedExactImmutablePublication::construct(&dir, |target| {
            let copied = std::io::copy(source, target)?;
            if copied != exact_length {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "bootstrap part object pack changed while being published",
                ));
            }
            Ok((final_name, copied))
        })
        .map_err(|error| {
            publication_error(
                error,
                Collision::Bootstrap("bootstrap part object pack", part_name.clone()),
            )
        })?;
        staged.commit().map_err(|error| {
            publication_error(
                error,
                Collision::Bootstrap("bootstrap part object pack", part_name.clone()),
            )
        })?;
        self.part_packs.insert(descriptor.part_id(), ());
        Ok(())
    }

    pub(crate) fn publish_part_artifacts(
        &mut self,
        descriptor: BootstrapPartDescriptorV1,
        manifest_bytes: &[u8],
        spans: &BootstrapPartSpanIndexV1,
    ) -> Result<(), StoreError> {
        let manifest = OperationBatch::decode(manifest_bytes)?;
        self.store
            .require_bootstrap_manifest(descriptor, &manifest)?;
        let manifest_digest = ContentDigest::of(manifest_bytes);
        let span_bytes = spans.encode()?;
        descriptor.validate_loaded_artifacts(
            BootstrapManifestFingerprintV1::from_bytes(*manifest_digest.as_bytes()),
            &manifest
                .required_objects()
                .iter()
                .map(|object| {
                    PayloadObjectDescriptorV1::new(
                        object.content_digest(),
                        object.encoded_byte_length(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            &[FullObjectDescriptorV1::manifest_defined(
                *ContentDigest::of(&span_bytes).as_bytes(),
                span_bytes.len() as u64,
            )?],
        )?;
        spans.validate_part(descriptor.evidence())?;

        let parts = self.store.bootstrap_namespace(BOOTSTRAP_PARTS_DIR, true)?;
        let part_name = hex_bytes(descriptor.part_id().as_bytes());
        self.stage(
            &parts,
            &part_name,
            manifest_bytes,
            Collision::Bootstrap("bootstrap part manifest", part_name.clone()),
        )?;
        let evidence = descriptor.evidence();
        let evidence_bytes = evidence.encode()?;
        let evidence_name = hex_bytes(evidence.evidence_digest().as_bytes());
        let evidence_dir = self
            .store
            .bootstrap_namespace(BOOTSTRAP_EVIDENCE_DIR, true)?;
        self.stage(
            &evidence_dir,
            &evidence_name,
            &evidence_bytes,
            Collision::Bootstrap("bootstrap part evidence", evidence_name.clone()),
        )?;
        let span_dir = self
            .store
            .bootstrap_namespace(BOOTSTRAP_PART_SPANS_DIR, true)?;
        self.stage(
            &span_dir,
            &part_name,
            &span_bytes,
            Collision::Bootstrap("bootstrap part span index", part_name.clone()),
        )?;
        self.parts.insert(descriptor.part_id(), ());
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        aggregate: &BootstrapAggregateManifestV1,
    ) -> Result<DurablyStagedBootstrapPrefix, StoreError> {
        self.store.require_bootstrap_aggregate_context(aggregate)?;
        let inventory_ordinals = (0..aggregate.source_inventory_page_count())
            .map(|ordinal| (ordinal, ()))
            .collect::<BTreeMap<_, _>>();
        let blob_ordinals = (0..aggregate.source_blob_page_count())
            .map(|ordinal| (ordinal, ()))
            .collect::<BTreeMap<_, _>>();
        let parts = aggregate
            .parts()
            .iter()
            .map(|part| (part.part_id(), ()))
            .collect::<BTreeMap<_, _>>();
        let expected_inventory_root = (aggregate.source_inventory_page_count() != 0)
            .then_some(aggregate.source_inventory_root());
        let expected_blob_root =
            (aggregate.source_blob_page_count() != 0).then_some(aggregate.source_blob_root());
        if self.inventory_root != expected_inventory_root
            || self.inventory_pages != inventory_ordinals
            || self.blob_root != expected_blob_root
            || self.blob_pages != blob_ordinals
            || self.part_packs != parts
            || self.parts != parts
        {
            return Err(StoreError::BootstrapArtifactMismatch(
                "batched bootstrap prefix closed set",
            ));
        }
        let bytes = aggregate.encode()?;
        let digest = aggregate.aggregate_digest();
        let name = hex_bytes(digest.as_bytes());
        let dir = self
            .store
            .bootstrap_namespace(BOOTSTRAP_AGGREGATES_DIR, true)?;
        self.stage(
            &dir,
            &name,
            &bytes,
            Collision::Bootstrap("bootstrap aggregate", name.clone()),
        )?;
        let _completed = self
            .physical
            .finish()
            .map_err(filesystem_error_without_collision)?;
        Ok(DurablyStagedBootstrapPrefix {
            workspace_id: self.store.workspace_id,
            archive_identity: self.store.canonical_archive_identity()?,
            aggregate_digest: digest,
        })
    }
}

fn publish_immutable(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    collision: Collision,
) -> Result<(), StoreError> {
    tine_storage::publish_immutable_exact(dir, filename, bytes)
        .map_err(|error| publication_error(error, collision))
}

pub(crate) fn publish_immutable_exact(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    kind: &'static str,
) -> Result<(), StoreError> {
    publish_immutable(dir, filename, bytes, Collision::Exact(kind))
}

fn publication_error(error: tine_storage::FilesystemError, collision: Collision) -> StoreError {
    match error {
        tine_storage::FilesystemError::ByteCollision => collision_error(collision),
        error => filesystem_error_without_collision(error),
    }
}

pub(crate) fn filesystem_error_without_collision(
    error: tine_storage::FilesystemError,
) -> StoreError {
    match error {
        tine_storage::FilesystemError::Io(error) => StoreError::Io(error),
        tine_storage::FilesystemError::DurableNameOperationUnavailable(message) => {
            StoreError::Io(std::io::Error::new(ErrorKind::Unsupported, message))
        }
        tine_storage::FilesystemError::UnsafeEntry(message) => StoreError::UnsafeEntry(message),
        tine_storage::FilesystemError::StoredLengthMismatch {
            path,
            expected,
            actual,
        } => StoreError::StoredLengthMismatch {
            path,
            expected,
            actual,
        },
        tine_storage::FilesystemError::StoredFileTooLarge {
            path,
            length,
            limit,
        } => StoreError::StoredFileTooLarge {
            path,
            length,
            limit,
        },
        tine_storage::FilesystemError::ByteCollision => {
            StoreError::ImmutableCollision("immutable publication")
        }
    }
}

fn collision_error(collision: Collision) -> StoreError {
    match collision {
        Collision::Object(digest) => StoreError::ObjectCollision(digest),
        Collision::Batch(batch_id) => StoreError::BatchCollision(batch_id),
        Collision::HistoryIndex(digest) => StoreError::HistoryIndexPathMismatch(digest),
        Collision::Lineage(lineage) => StoreError::LineageClaimCollision(lineage),
        Collision::Exact(kind) => StoreError::ImmutableCollision(kind),
        Collision::Bootstrap(kind, identity) => {
            StoreError::BootstrapArtifactCollision { kind, identity }
        }
    }
}

fn publish_bootstrap_immutable(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    kind: &'static str,
    identity: String,
) -> Result<(), StoreError> {
    publish_immutable(dir, filename, bytes, Collision::Bootstrap(kind, identity))
}

fn bootstrap_page_filename(ordinal: u32) -> String {
    ordinal.to_string()
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn read_regular_file_nofollow(dir: &Dir, name: &str) -> Result<fs::File, StoreError> {
    let file = open_file_nofollow(dir, name)?;
    if !file.metadata()?.is_file() {
        return Err(StoreError::UnsafeEntry(format!(
            "{name} is not a regular no-follow file"
        )));
    }
    Ok(file)
}

struct AdvisoryTransitionGuard<'a>(&'a fs::File);

impl<'a> AdvisoryTransitionGuard<'a> {
    fn lock(file: &'a fs::File) -> Result<Self, StoreError> {
        #[cfg(test)]
        {
            let contention_hook =
                ADVISORY_TRANSITION_CONTENTION_HOOK.with(|slot| slot.borrow_mut().take());
            if let Some(contention_hook) = contention_hook {
                match fs2::FileExt::try_lock_exclusive(file) {
                    Ok(()) => return Ok(Self(file)),
                    Err(error) if tine_storage::nonblocking_lock_is_contended(&error) => {
                        contention_hook()
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        fs2::FileExt::lock_exclusive(file)?;
        Ok(Self(file))
    }
}

impl Drop for AdvisoryTransitionGuard<'_> {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(self.0);
    }
}

#[cfg(unix)]
fn open_engine_history_transition_lock(
    root: &Dir,
    lifecycle: ObjectStoreLifecycle,
) -> Result<fs::File, StoreError> {
    let name = CString::new(ENGINE_HISTORY_TRANSITION_LOCK_FILE).map_err(|_| {
        std::io::Error::new(ErrorKind::InvalidInput, "invalid transition lock name")
    })?;
    // SAFETY: the name is live and relative to the retained workspace
    // capability. O_NOFOLLOW rejects a final-component symlink atomically.
    #[cfg(target_os = "android")]
    let flags = if lifecycle == ObjectStoreLifecycle::ReconstructibleActivation {
        libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC
    } else {
        libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW
    };
    #[cfg(not(target_os = "android"))]
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    let fd = unsafe { libc::openat(root.as_fd().as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: openat returned one newly owned descriptor.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(StoreError::UnsafeEntry(
            "engine history transition lock is not a regular no-follow file".into(),
        ));
    }
    match file.sync_all() {
        Ok(()) => {}
        #[cfg(target_os = "android")]
        Err(error)
            if lifecycle == ObjectStoreLifecycle::ReconstructibleActivation
                && crate::filesystem_durability::android_durability_capability_refusal(
                    error.kind(),
                ) => {}
        Err(error) => return Err(error.into()),
    }
    sync_directory_for_lifecycle(root, lifecycle)?;
    Ok(file)
}

#[cfg(windows)]
fn open_engine_history_transition_lock(
    root: &Dir,
    lifecycle: ObjectStoreLifecycle,
) -> Result<fs::File, StoreError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    let file = root
        .open_with(ENGINE_HISTORY_TRANSITION_LOCK_FILE, &options)?
        .into_std();
    let metadata = file.metadata()?;
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
        || !metadata.is_file()
    {
        return Err(StoreError::UnsafeEntry(
            "engine history transition lock is not a regular no-follow file".into(),
        ));
    }
    file.sync_all()?;
    sync_directory_for_lifecycle(root, lifecycle)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_engine_history_transition_lock(
    _root: &Dir,
    _lifecycle: ObjectStoreLifecycle,
) -> Result<fs::File, StoreError> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "workspace advisory transition locks are unsupported on this target",
    )
    .into())
}

pub(crate) fn open_file_nofollow(dir: &Dir, path: &str) -> std::io::Result<fs::File> {
    tine_storage::open_file_nofollow(dir, path)
}

pub(crate) fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
    tine_storage::open_dir_nofollow(dir, path).map_err(filesystem_error_without_collision)
    // SAFETY: `openat` returned a newly owned directory descriptor.
}

#[cfg(target_os = "android")]
fn open_android_private_archive_directory(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
    let component = Path::new(path);
    if !matches!(component.components().next(), Some(Component::Normal(_)))
        || component.components().count() != 1
    {
        return Err(StoreError::UnsafeEntry(format!(
            "private archive directory name is not one normal component: {path}"
        )));
    }
    let metadata = dir.symlink_metadata(component)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::UnsafeEntry(format!(
            "private archive entry is not a real directory: {path}"
        )));
    }
    let name = CString::new(path)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid archive directory"))?;
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
        )
    };
    if fd < 0 {
        return Err(StoreError::Io(std::io::Error::last_os_error()));
    }
    // SAFETY: openat returned one newly owned descriptor.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    if !file.metadata()?.is_dir() {
        return Err(StoreError::UnsafeEntry(
            "private archive handle is not a directory".into(),
        ));
    }
    Ok(Dir::from_std_file(file))
}

fn open_directory_for_lifecycle(
    dir: &Dir,
    path: &str,
    lifecycle: ObjectStoreLifecycle,
) -> Result<Dir, StoreError> {
    let _ = lifecycle;
    #[cfg(target_os = "android")]
    if lifecycle == ObjectStoreLifecycle::ReconstructibleActivation {
        let component = Path::new(path);
        if !matches!(component.components().next(), Some(Component::Normal(_)))
            || component.components().count() != 1
        {
            return Err(StoreError::UnsafeEntry(format!(
                "activation archive directory name is not one normal component: {path}"
            )));
        }
        let name = CString::new(path).map_err(|_| {
            std::io::Error::new(
                ErrorKind::InvalidInput,
                "invalid activation archive directory",
            )
        })?;
        // This is Android app-private, single-writer, pre-promotion state. An
        // ordinary directory open is the platform contract; the returned
        // handle is still checked before it is retained.
        let fd = unsafe {
            libc::openat(
                dir.as_fd().as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(StoreError::Io(std::io::Error::last_os_error()));
        }
        // SAFETY: openat returned one newly owned descriptor.
        let file = unsafe { fs::File::from_raw_fd(fd) };
        if !file.metadata()?.is_dir() {
            return Err(StoreError::UnsafeEntry(
                "activation archive handle is not a directory".into(),
            ));
        }
        return Ok(Dir::from_std_file(file));
    }

    open_dir_nofollow(dir, path)
}

fn open_existing_directory_for_lifecycle(
    dir: &Dir,
    path: &str,
    lifecycle: ObjectStoreLifecycle,
) -> Result<Option<Dir>, StoreError> {
    let metadata = match dir.symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::UnsafeEntry(format!(
            "private archive entry is not a real directory: {path}"
        )));
    }
    open_directory_for_lifecycle(dir, path, lifecycle).map(Some)
}

fn sync_directory_for_lifecycle(
    dir: &Dir,
    lifecycle: ObjectStoreLifecycle,
) -> Result<(), StoreError> {
    let _ = lifecycle;
    #[cfg(target_os = "android")]
    if lifecycle == ObjectStoreLifecycle::ReconstructibleActivation {
        return crate::filesystem_durability::sync_reconstructible_directory(dir)
            .map_err(StoreError::Io);
    }
    sync_dir_required(dir)
}

pub(crate) fn read_optional_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Option<Vec<u8>>, StoreError> {
    tine_storage::read_optional_regular(dir, path, limit, expected_length)
        .map_err(filesystem_error_without_collision)
}

fn read_required_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Vec<u8>, StoreError> {
    tine_storage::read_required_regular(dir, path, limit, expected_length)
        .map_err(filesystem_error_without_collision)
}

fn object_filename(digest: ContentDigest) -> String {
    format!("{digest}.object")
}

fn manifest_filename(batch_id: BatchId) -> String {
    format!("{batch_id}.manifest")
}

fn history_filename(batch_id: BatchId) -> String {
    format!("{batch_id}.status")
}

fn history_index_filename(digest: ContentDigest) -> String {
    format!("{digest}.index")
}

fn engine_history_root_filename(digest: ContentDigest) -> String {
    format!("{digest}{ENGINE_HISTORY_ROOT_SUFFIX}")
}

fn history_key_nibble(key: &[u8; 16], depth: u8) -> u8 {
    let byte = key[usize::from(depth / 2)];
    if depth.is_multiple_of(2) {
        byte >> 4
    } else {
        byte & 0x0f
    }
}

fn block_claim_key_nibble(key: &[u8; 16], depth: u8) -> u8 {
    let digest = ContentDigest::of(key);
    let byte = digest.as_bytes()[usize::from(depth / 2)];
    if depth.is_multiple_of(2) {
        byte >> 4
    } else {
        byte & 0x0f
    }
}

fn block_claim_page_depth(page: &BlockClaimIndexPage) -> u8 {
    match page {
        BlockClaimIndexPage::Branch { depth, .. } | BlockClaimIndexPage::Leaf { depth, .. } => {
            *depth
        }
    }
}

fn new_block_claim_filter(
    entries: &[([u8; 16], BlockClaimIndexValue)],
) -> Result<BlockClaimFilterPage, StoreError> {
    let bit_len = entries
        .len()
        .checked_mul(BLOCK_CLAIM_FILTER_BITS_PER_ENTRY)
        .ok_or(StoreError::MalformedBlockClaimIndex)?;
    let byte_len = bit_len
        .checked_add(7)
        .ok_or(StoreError::MalformedBlockClaimIndex)?
        / 8;
    let mut filter = BlockClaimFilterPage {
        schema_version: BLOCK_CLAIM_INDEX_SCHEMA_VERSION,
        entry_count: u64::try_from(entries.len())
            .map_err(|_| StoreError::MalformedBlockClaimIndex)?,
        bit_len: u64::try_from(bit_len).map_err(|_| StoreError::MalformedBlockClaimIndex)?,
        bits: vec![0; byte_len],
    };
    for (key, _) in entries {
        let (first, second) = block_claim_filter_hashes(key);
        for position in block_claim_filter_positions(first, second, filter.bit_len) {
            filter.bits[position as usize / 8] |= 1 << (position % 8);
        }
    }
    validate_block_claim_filter(&filter)?;
    Ok(filter)
}

fn new_block_claim_global_filter() -> BlockClaimGlobalFilterPage {
    BlockClaimGlobalFilterPage {
        schema_version: BLOCK_CLAIM_INDEX_SCHEMA_VERSION,
        insertions: 0,
        bits: vec![0; BLOCK_CLAIM_GLOBAL_FILTER_BYTES],
    }
}

fn update_block_claim_global_filter(
    filter: &mut BlockClaimGlobalFilterPage,
    records: &[([u8; 16], BlockClaimIndexValue)],
) -> Result<(), StoreError> {
    filter.insertions = filter
        .insertions
        .checked_add(
            u64::try_from(records.len()).map_err(|_| StoreError::MalformedBlockClaimIndex)?,
        )
        .ok_or(StoreError::MalformedBlockClaimIndex)?;
    let bit_len = u64::try_from(filter.bits.len())
        .ok()
        .and_then(|bytes| bytes.checked_mul(8))
        .ok_or(StoreError::MalformedBlockClaimIndex)?;
    for (key, _) in records {
        let (first, second) = block_claim_filter_hashes(key);
        for position in block_claim_filter_positions(first, second, bit_len) {
            filter.bits[position as usize / 8] |= 1 << (position % 8);
        }
    }
    Ok(())
}

fn validate_block_claim_global_filter(
    filter: &BlockClaimGlobalFilterPage,
) -> Result<(), StoreError> {
    if filter.schema_version != BLOCK_CLAIM_INDEX_SCHEMA_VERSION
        || filter.insertions == 0
        || filter.bits.len() != BLOCK_CLAIM_GLOBAL_FILTER_BYTES
    {
        return Err(StoreError::MalformedBlockClaimIndex);
    }
    Ok(())
}

fn block_claim_global_filter_might_contain(
    filter: &BlockClaimGlobalFilterPage,
    first: u64,
    second: u64,
) -> bool {
    let bit_len = (filter.bits.len() as u64) * 8;
    block_claim_filter_positions(first, second, bit_len)
        .into_iter()
        .all(|position| filter.bits[position as usize / 8] & (1 << (position % 8)) != 0)
}

fn validate_block_claim_filter(filter: &BlockClaimFilterPage) -> Result<(), StoreError> {
    let expected_bits = usize::try_from(filter.entry_count)
        .ok()
        .and_then(|entries| entries.checked_mul(BLOCK_CLAIM_FILTER_BITS_PER_ENTRY))
        .ok_or(StoreError::MalformedBlockClaimIndex)?;
    let expected_bytes = expected_bits
        .checked_add(7)
        .ok_or(StoreError::MalformedBlockClaimIndex)?
        / 8;
    if filter.schema_version != BLOCK_CLAIM_INDEX_SCHEMA_VERSION
        || filter.entry_count == 0
        || filter.bit_len != expected_bits as u64
        || filter.bits.len() != expected_bytes
    {
        return Err(StoreError::MalformedBlockClaimIndex);
    }
    let unused_bits = expected_bytes * 8 - expected_bits;
    if unused_bits != 0
        && filter.bits.last().is_some_and(|last| {
            let used_mask = u8::MAX >> unused_bits;
            *last & !used_mask != 0
        })
    {
        return Err(StoreError::MalformedBlockClaimIndex);
    }
    Ok(())
}

fn block_claim_filter_might_contain(
    filter: &BlockClaimFilterPage,
    first: u64,
    second: u64,
) -> bool {
    block_claim_filter_positions(first, second, filter.bit_len)
        .into_iter()
        .all(|position| filter.bits[position as usize / 8] & (1 << (position % 8)) != 0)
}

fn block_claim_filter_hashes(key: &[u8; 16]) -> (u64, u64) {
    let high = u64::from_be_bytes(key[..8].try_into().expect("fixed block key"));
    let low = u64::from_be_bytes(key[8..].try_into().expect("fixed block key"));
    let first = splitmix64(high ^ low.rotate_left(23));
    let second = splitmix64(low ^ high.rotate_right(17) ^ 0x9e37_79b9_7f4a_7c15) | 1;
    (first, second)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn block_claim_filter_positions(
    first: u64,
    second: u64,
    bit_len: u64,
) -> [u64; BLOCK_CLAIM_FILTER_HASHES as usize] {
    std::array::from_fn(|index| {
        first
            .wrapping_add((index as u64).wrapping_mul(second))
            .wrapping_rem(bit_len)
    })
}

fn validate_block_claim_page(page: &BlockClaimIndexPage) -> Result<(), StoreError> {
    match page {
        BlockClaimIndexPage::Branch {
            schema_version,
            depth,
            children,
        } => {
            if *schema_version != BLOCK_CLAIM_INDEX_SCHEMA_VERSION
                || *depth >= BLOCK_CLAIM_RADIX_DEPTH
                || children.is_empty()
                || children.iter().any(|(nibble, _)| *nibble >= 16)
                || !children.windows(2).all(|pair| pair[0].0 < pair[1].0)
            {
                return Err(StoreError::MalformedBlockClaimIndex);
            }
        }
        BlockClaimIndexPage::Leaf {
            schema_version,
            depth,
            entries,
        } => {
            if *schema_version != BLOCK_CLAIM_INDEX_SCHEMA_VERSION
                || *depth > BLOCK_CLAIM_RADIX_DEPTH
                || entries.is_empty()
                || (*depth < BLOCK_CLAIM_RADIX_DEPTH && entries.len() > BLOCK_CLAIM_LEAF_ENTRIES)
                || !entries.windows(2).all(|pair| pair[0].0 < pair[1].0)
                || entries.iter().any(|(_, record)| {
                    record.is_empty() || record.len() > MAX_BLOCK_CLAIM_RECORD_BYTES
                })
            {
                return Err(StoreError::MalformedBlockClaimIndex);
            }
        }
    }
    Ok(())
}

fn validate_history_node(node: &HistoryIndexNode) -> Result<(), StoreError> {
    match node {
        HistoryIndexNode::Branch {
            schema_version,
            depth,
            children,
        } => {
            if *schema_version != ENGINE_HISTORY_INDEX_SCHEMA_VERSION
                || *depth >= ENGINE_HISTORY_RADIX_DEPTH
                || children.is_empty()
                || children.iter().any(|(nibble, _)| *nibble >= 16)
                || !children.windows(2).all(|pair| pair[0].0 < pair[1].0)
            {
                return Err(StoreError::MalformedHistoryIndex);
            }
        }
        HistoryIndexNode::Leaf {
            schema_version,
            record,
            ..
        } => {
            if *schema_version != ENGINE_HISTORY_INDEX_SCHEMA_VERSION
                || record.is_empty()
                || record.len() as u64 > MAX_ENGINE_HISTORY_RECORD_BYTES
            {
                return Err(StoreError::MalformedHistoryIndex);
            }
        }
    }
    Ok(())
}

fn parse_object_filename(name: &str) -> Result<ContentDigest, StoreError> {
    let Some(digest) = name.strip_suffix(".object") else {
        return Err(StoreError::MalformedPath(name.into()));
    };
    if digest.len() != 64
        || digest
            .bytes()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(StoreError::MalformedPath(name.into()));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in digest.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]).expect("validated hex") << 4)
            | hex_nibble(pair[1]).expect("validated hex");
    }
    Ok(ContentDigest::from_bytes(bytes))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_manifest_filename(name: &str) -> Result<BatchId, StoreError> {
    let Some(batch_id) = name.strip_suffix(".manifest") else {
        return Err(StoreError::MalformedPath(name.into()));
    };
    let parsed = batch_id
        .parse::<BatchId>()
        .map_err(|_| StoreError::MalformedPath(name.into()))?;
    if parsed.to_string() != batch_id {
        return Err(StoreError::MalformedPath(name.into()));
    }
    Ok(parsed)
}

pub(crate) fn is_temp_name(name: &str) -> bool {
    name.strip_prefix(".tmp-")
        .and_then(|value| Uuid::parse_str(value).ok())
        .is_some()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod history_index_tests {
    use super::*;

    fn test_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("tine-history-index-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn snapshot_tree(path: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut result = BTreeMap::new();
        let mut pending = vec![path.to_path_buf()];
        while let Some(entry_path) = pending.pop() {
            let relative = entry_path.strip_prefix(path).unwrap().to_path_buf();
            if entry_path.is_dir() {
                result.insert(relative, None);
                for entry in std::fs::read_dir(&entry_path).unwrap() {
                    pending.push(entry.unwrap().path());
                }
            } else {
                result.insert(relative, Some(std::fs::read(entry_path).unwrap()));
            }
        }
        result
    }

    fn snapshot_tree_with_identity(path: &Path) -> BTreeMap<PathBuf, (Vec<u8>, Option<Vec<u8>>)> {
        fn identity(path: &Path) -> Vec<u8> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;

                let metadata = std::fs::symlink_metadata(path).unwrap();
                let mut identity = Vec::with_capacity(16);
                identity.extend_from_slice(&metadata.dev().to_be_bytes());
                identity.extend_from_slice(&metadata.ino().to_be_bytes());
                identity
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt as _;
                use windows_sys::Win32::Storage::FileSystem::{
                    FileIdInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS,
                    FILE_ID_INFO,
                };

                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(path)
                    .unwrap();
                let mut information = FILE_ID_INFO::default();
                let result = unsafe {
                    GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileIdInfo,
                        (&mut information as *mut FILE_ID_INFO).cast(),
                        std::mem::size_of::<FILE_ID_INFO>() as u32,
                    )
                };
                assert_ne!(result, 0, "test filesystem identity");
                let mut identity = Vec::with_capacity(24);
                identity.extend_from_slice(&information.VolumeSerialNumber.to_be_bytes());
                identity.extend_from_slice(&information.FileId.Identifier);
                identity
            }
            #[cfg(not(any(unix, windows)))]
            {
                Vec::new()
            }
        }

        let mut result = BTreeMap::new();
        let mut pending = vec![path.to_path_buf()];
        while let Some(entry_path) = pending.pop() {
            let relative = entry_path.strip_prefix(path).unwrap().to_path_buf();
            if entry_path.is_dir() {
                result.insert(relative, (identity(&entry_path), None));
                for entry in std::fs::read_dir(&entry_path).unwrap() {
                    pending.push(entry.unwrap().path());
                }
            } else {
                result.insert(
                    relative,
                    (
                        identity(&entry_path),
                        Some(std::fs::read(&entry_path).unwrap()),
                    ),
                );
            }
        }
        result
    }

    fn enrolled_binding(endpoint: u128) -> crate::oplog::hot_engine::ProjectionStorageBinding {
        crate::oplog::hot_engine::ProjectionStorageBinding {
            endpoint: crate::oplog::ProjectionEndpointBinding {
                endpoint_id: crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(
                    endpoint,
                )),
                device_id: crate::oplog::DeviceId::from_uuid(Uuid::from_u128(endpoint + 1)),
                graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"test",
                    &endpoint.to_be_bytes(),
                ),
            },
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"test",
                &(endpoint + 2).to_be_bytes(),
            ),
        }
    }

    #[test]
    fn absent_enrolled_controls_are_not_adopted_after_last_validation() {
        #[derive(Clone, Copy)]
        enum Attack {
            Create,
            Substitute,
        }

        for (label, control_name, attack) in [
            ("history-create", ENGINE_HISTORY_DIR, Attack::Create),
            ("work-create", PROJECTION_WORK_DIR, Attack::Create),
            ("history-substitute", ENGINE_HISTORY_DIR, Attack::Substitute),
            ("work-substitute", PROJECTION_WORK_DIR, Attack::Substitute),
        ] {
            let root = test_root(&format!("absent-enrolled-{label}"));
            let archive = root.join("archive");
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(100));
            let binding = enrolled_binding(110);
            let store = ObjectStore::open(&archive, workspace).unwrap();
            let open = store.seal_enrolled_projection(binding).unwrap();
            let control = archive
                .join(control_name)
                .join(binding.endpoint.endpoint_id.to_string());
            let snapshot = Arc::new(Mutex::new(None));
            let snapshot_hook = Arc::clone(&snapshot);
            let archive_hook = archive.clone();
            set_enrolled_open_act_hook(move || {
                match attack {
                    Attack::Create => std::fs::create_dir_all(&control).unwrap(),
                    Attack::Substitute => {
                        std::fs::create_dir_all(control.parent().unwrap()).unwrap();
                        let foreign = archive_hook.join(format!("foreign-{label}"));
                        std::fs::create_dir(&foreign).unwrap();
                        std::fs::rename(foreign, &control).unwrap();
                    }
                }
                std::fs::write(control.join("foreign-owner"), b"foreign archive").unwrap();
                *snapshot_hook.lock().unwrap() = Some(snapshot_tree_with_identity(&archive_hook));
            });

            assert!(
                open.into_runtime().is_err(),
                "formerly absent {label} control was adopted"
            );
            assert_eq!(
                snapshot_tree_with_identity(&archive),
                snapshot.lock().unwrap().clone().expect("attack hook ran"),
                "rejection mutated the foreign {label} archive"
            );
            crate::test_support::remove_dir_all(root);
        }
    }

    #[test]
    fn absent_endpoint_rejects_sealed_parent_namespace_substitution() {
        for (label, namespace_name) in [
            ("history-parent", ENGINE_HISTORY_DIR),
            ("work-parent", PROJECTION_WORK_DIR),
        ] {
            let root = test_root(&format!("absent-parent-{label}"));
            let archive = root.join("archive");
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(105));
            let binding = enrolled_binding(115);
            let store = ObjectStore::open(&archive, workspace).unwrap();
            let namespace = archive.join(namespace_name);
            std::fs::create_dir(&namespace).unwrap();
            std::fs::create_dir(namespace.join("unrelated-endpoint")).unwrap();
            let open = store.seal_enrolled_projection(binding).unwrap();
            let moved = archive.join(format!("{namespace_name}-moved"));
            let endpoint = namespace.join(binding.endpoint.endpoint_id.to_string());
            let snapshot = Arc::new(Mutex::new(None));
            let snapshot_hook = Arc::clone(&snapshot);
            let archive_hook = archive.clone();
            set_enrolled_open_act_hook(move || {
                std::fs::rename(&namespace, &moved).unwrap();
                std::fs::create_dir(&namespace).unwrap();
                std::fs::create_dir(&endpoint).unwrap();
                std::fs::write(endpoint.join("foreign-owner"), b"foreign archive").unwrap();
                *snapshot_hook.lock().unwrap() = Some(snapshot_tree_with_identity(&archive_hook));
            });

            assert!(open.into_runtime().is_err());
            assert_eq!(
                snapshot_tree_with_identity(&archive),
                snapshot.lock().unwrap().clone().expect("attack hook ran")
            );
            crate::test_support::remove_dir_all(root);
        }
    }

    #[test]
    fn enrolled_history_head_rollback_after_validation_is_rejected() {
        let root = test_root("enrolled-head-rollback-at-act");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(120));
        let binding = enrolled_binding(130);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        let control = archive
            .join(ENGINE_HISTORY_DIR)
            .join(binding.endpoint.endpoint_id.to_string());
        let original = std::fs::read(control.join(ENGINE_HISTORY_HEAD_FILE)).unwrap();
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(140)),
                b"accepted history",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        drop(history);
        drop(store.open_projection_work_index(binding).unwrap());
        drop(store);

        let open = ObjectStore::open(&archive, workspace)
            .unwrap()
            .seal_enrolled_projection(binding)
            .unwrap();
        let attacked = Arc::new(Mutex::new(None));
        let attacked_hook = Arc::clone(&attacked);
        let archive_hook = archive.clone();
        set_enrolled_open_act_hook(move || {
            std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), original).unwrap();
            *attacked_hook.lock().unwrap() = Some(snapshot_tree_with_identity(&archive_hook));
        });

        assert!(open.into_runtime().is_err());
        assert_eq!(
            snapshot_tree_with_identity(&archive),
            attacked.lock().unwrap().clone().expect("attack hook ran"),
            "rollback rejection mutated the archive"
        );
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_history_baseline_survives_reads_until_an_anchored_transition() {
        let root = test_root("enrolled-head-rollback-subsequent-read");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(150));
        let binding = enrolled_binding(160);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        let control = archive
            .join(ENGINE_HISTORY_DIR)
            .join(binding.endpoint.endpoint_id.to_string());
        let original = std::fs::read(control.join(ENGINE_HISTORY_HEAD_FILE)).unwrap();
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(170)),
                b"accepted history",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        let accepted = std::fs::read(control.join(ENGINE_HISTORY_HEAD_FILE)).unwrap();
        drop(history);
        drop(store.open_projection_work_index(binding).unwrap());
        drop(store);

        let (_, history, _) = ObjectStore::open(&archive, workspace)
            .unwrap()
            .seal_enrolled_projection(binding)
            .unwrap()
            .into_runtime()
            .unwrap();
        assert_eq!(history.current().unwrap().0, 1);
        std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), &original).unwrap();
        let attacked = snapshot_tree(&archive);
        assert!(
            history.current().is_err(),
            "rollback was accepted on reread"
        );
        assert_eq!(snapshot_tree(&archive), attacked);

        std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), accepted).unwrap();
        assert_eq!(
            history.current().unwrap().0,
            1,
            "the sealed baseline was forgotten after a rejected rollback"
        );
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn authenticated_history_point_lookup_tamper_and_collision_fail_closed() {
        let root = test_root("integrity");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let history = store.start_engine_history().unwrap();
        let batch_id = BatchId::from_uuid(Uuid::from_u128(2));
        let before_insert = store.instrumentation();
        let index_root = history
            .insert(EngineHistoryStore::empty_root(), batch_id, b"record")
            .unwrap();
        let after_insert = store.instrumentation();
        assert_eq!(
            after_insert.directory_enumerations - before_insert.directory_enumerations,
            0
        );
        assert_eq!(
            after_insert.history_index_reads - before_insert.history_index_reads,
            0
        );
        assert_eq!(
            after_insert.history_index_writes - before_insert.history_index_writes,
            33
        );

        let before = store.instrumentation();
        assert_eq!(
            history.lookup(index_root, batch_id).unwrap(),
            Some(b"record".to_vec())
        );
        let after = store.instrumentation();
        assert_eq!(
            after.directory_enumerations - before.directory_enumerations,
            0
        );
        assert!(after.history_index_reads - before.history_index_reads <= 33);
        assert_eq!(
            history
                .lookup(index_root, BatchId::from_uuid(Uuid::from_u128(3)))
                .unwrap(),
            None
        );

        let run = std::fs::read_dir(root.join("archive/engine-history"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let child_digest = match history.read_node(index_root).unwrap() {
            HistoryIndexNode::Branch { children, .. } => children[0].1,
            HistoryIndexNode::Leaf { .. } => panic!("radix root must be a branch"),
        };
        let child_path = run.join(history_index_filename(child_digest));
        let child_bytes = std::fs::read(&child_path).unwrap();
        let mut replaced_child = child_bytes.clone();
        let child_middle = replaced_child.len() / 2;
        replaced_child[child_middle] ^= 1;
        std::fs::write(&child_path, replaced_child).unwrap();
        assert!(matches!(
            history.lookup(index_root, batch_id),
            Err(StoreError::HistoryIndexPathMismatch(found)) if found == child_digest
        ));
        std::fs::write(&child_path, child_bytes).unwrap();

        let root_path = run.join(history_index_filename(index_root));
        let mut bytes = std::fs::read(&root_path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        std::fs::write(&root_path, bytes).unwrap();
        assert!(matches!(
            history.lookup(index_root, batch_id),
            Err(StoreError::HistoryIndexPathMismatch(_))
        ));

        let collision_batch = BatchId::from_uuid(Uuid::from_u128(4));
        let collision_node = HistoryIndexNode::Leaf {
            schema_version: ENGINE_HISTORY_INDEX_SCHEMA_VERSION,
            batch_id: collision_batch,
            record: b"collision".to_vec(),
        };
        let collision_bytes = postcard::to_allocvec(&collision_node).unwrap();
        let collision_digest = ContentDigest::of(&collision_bytes);
        std::fs::write(
            run.join(history_index_filename(collision_digest)),
            b"different immutable bytes",
        )
        .unwrap();
        assert!(matches!(
            history.publish_node(&collision_node),
            Err(StoreError::HistoryIndexPathMismatch(found)) if found == collision_digest
        ));
        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn durable_history_head_and_root_fail_closed() {
        let root = test_root("durable-root");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(5));
        let endpoint = crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(7));
        let endpoint_binding = crate::oplog::ProjectionEndpointBinding {
            endpoint_id: endpoint,
            device_id: crate::oplog::DeviceId::from_uuid(Uuid::from_u128(8)),
            graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                b"test",
                b"durable-root",
            ),
        };
        let store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let history = store
            .open_engine_history(crate::oplog::hot_engine::ProjectionStorageBinding {
                endpoint: endpoint_binding,
                receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                    b"test",
                    b"engine-history",
                ),
            })
            .unwrap();
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(6)),
                b"bound durable record",
                EngineHistoryBinding::empty(),
            )
            .unwrap();

        let control = root
            .join("archive")
            .join(ENGINE_HISTORY_DIR)
            .join(endpoint.to_string());
        let head = std::fs::read_to_string(control.join(ENGINE_HISTORY_HEAD_FILE)).unwrap();
        let root_path = control
            .join(ENGINE_HISTORY_ROOTS_DIR)
            .join(format!("{head}{ENGINE_HISTORY_ROOT_SUFFIX}"));
        let original = std::fs::read(&root_path).unwrap();
        let mut tampered = original.clone();
        tampered[0] ^= 0x80;
        std::fs::write(&root_path, tampered).unwrap();
        assert!(matches!(
            history.current(),
            Err(StoreError::HistoryIndexPathMismatch(_))
        ));

        std::fs::write(&root_path, original).unwrap();
        std::fs::remove_file(control.join(ENGINE_HISTORY_HEAD_FILE)).unwrap();
        assert!(matches!(
            history.current(),
            Err(StoreError::MalformedHistoryIndex)
        ));
        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn authenticated_history_transition_accepts_only_exact_or_insertion_only_lineage() {
        let root = test_root("authenticated-transition-lineage");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(45_000));
        let binding = enrolled_binding(45_010);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        let control = archive
            .join(ENGINE_HISTORY_DIR)
            .join(binding.endpoint.endpoint_id.to_string());
        let empty_head = std::fs::read(control.join(ENGINE_HISTORY_HEAD_FILE)).unwrap();
        let empty = history.current_authority().unwrap();

        let exact = history
            .authenticate_current_history_extension(empty)
            .unwrap();
        assert_eq!(exact.before(), empty);
        assert_eq!(exact.after(), empty);

        let first_batch = BatchId::from_uuid(Uuid::from_u128(45_020));
        let first_bytes = b"authenticated first record".to_vec();
        history
            .publish(first_batch, &first_bytes, EngineHistoryBinding::empty())
            .unwrap();
        let first = history.current_authority().unwrap();
        let extension = history
            .authenticate_current_history_extension(empty)
            .unwrap();
        assert_eq!(extension.before(), empty);
        assert_eq!(extension.after(), first);

        let second_batch = BatchId::from_uuid(Uuid::from_u128(45_021));
        let second_bytes = b"authenticated unrelated record".to_vec();
        history
            .publish(second_batch, &second_bytes, EngineHistoryBinding::empty())
            .unwrap();
        let forward = history.current_authority().unwrap();
        assert_eq!(
            history
                .authenticate_current_history_extension(first)
                .unwrap()
                .after(),
            forward
        );
        drop(history);
        drop(store);

        std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), &empty_head).unwrap();
        let divergent_store = ObjectStore::open(&archive, workspace).unwrap();
        let divergent_history = divergent_store.open_engine_history(binding).unwrap();
        let divergent_batch = BatchId::from_uuid(Uuid::from_u128(45_030));
        divergent_history
            .publish(
                divergent_batch,
                b"equal-generation divergent record",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        assert_eq!(divergent_history.current_authority().unwrap().generation, 1);
        assert!(matches!(
            divergent_history.authenticate_current_history_extension(first),
            Err(StoreError::MalformedHistoryIndex)
        ));

        let divergent_later = BatchId::from_uuid(Uuid::from_u128(45_031));
        divergent_history
            .publish(
                divergent_later,
                b"higher-generation divergent record",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        assert_eq!(divergent_history.current_authority().unwrap().generation, 2);
        assert!(matches!(
            divergent_history.authenticate_current_history_extension(first),
            Err(StoreError::MalformedHistoryIndex)
        ));
        drop(divergent_history);
        drop(divergent_store);

        std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), empty_head).unwrap();
        let rollback_store = ObjectStore::open(&archive, workspace).unwrap();
        let rollback_history = rollback_store.open_engine_history(binding).unwrap();
        assert!(matches!(
            rollback_history.authenticate_current_history_extension(first),
            Err(StoreError::MalformedHistoryIndex)
        ));

        drop(rollback_history);
        drop(rollback_store);
        crate::test_support::remove_dir_all(root);
    }

    /// Deterministic, well-spread batch identifiers. Multiplying by an odd
    /// constant is a bijection modulo 2^128, so every index yields a distinct
    /// key, and the high bits vary so the radix keys branch like real batch
    /// identifiers instead of sharing one long synthetic prefix.
    fn spread_history_batch_id(index: usize) -> BatchId {
        BatchId::from_uuid(Uuid::from_u128(
            0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835_u128.wrapping_mul(index as u128 + 1),
        ))
    }

    /// The constant live-endpoint revalidation every warm call performs: the
    /// two radix roots the direct walk itself would read, and never more.
    const LIVE_ENDPOINT_REVALIDATION_BOUND: usize = 2;

    /// One radix insertion touches `ENGINE_HISTORY_RADIX_DEPTH + 1` nodes. A
    /// residual `middle -> current` diff walk reads at most one such path on
    /// each side before it either terminates on an equal subtree or counts the
    /// single newly inserted record, so a memoized incremental step can never
    /// exceed twice one insertion path plus the constant live-endpoint
    /// revalidation — whatever the post-anchor history size.
    const INCREMENTAL_STEP_BOUND: usize =
        LIVE_ENDPOINT_REVALIDATION_BOUND + 2 * (ENGINE_HISTORY_RADIX_DEPTH as usize + 1);

    #[test]
    fn authenticated_history_extension_revalidation_is_bounded_per_step() {
        fn node_reads(store: &ObjectStore) -> usize {
            store.instrumentation().history_index_reads
        }

        // Post-anchor history sizes. Every assertion below is a statement about
        // *node reads*, and the memoized step is constant by construction, so
        // these only have to be large enough for each comparison to be decided
        // with real margin -- see the measured table on the closing assertion.
        // They used to be 1/1,000/10,000, which published 11,001 durable
        // records to decide those same comparisons and made this the slowest
        // test in the suite by an order of magnitude.
        let mut full_walks = Vec::new();
        for (run, size) in [1_usize, 64, 512].into_iter().enumerate() {
            let root = test_root(&format!("bounded-revalidation-{size}"));
            let archive = root.join("archive");
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(46_000 + run as u128));
            let binding = enrolled_binding(46_100 + run as u128 * 10);
            let store = ObjectStore::open(&archive, workspace).unwrap();
            let history = store.open_engine_history(binding).unwrap();
            let anchor = history.current_authority().unwrap();

            let mut bootstrap_step = 0_usize;
            let mut worst_incremental_step = 0_usize;
            for index in 0..size {
                history
                    .publish(
                        spread_history_batch_id(index),
                        b"bounded revalidation record",
                        EngineHistoryBinding::empty(),
                    )
                    .unwrap();
                let before = node_reads(&store);
                let proof = history
                    .authenticate_current_history_extension(anchor)
                    .unwrap();
                let step = node_reads(&store) - before;
                assert_eq!(proof.before(), anchor);
                assert_eq!(proof.after().generation, index as u64 + 1);
                if index == 0 {
                    bootstrap_step = step;
                } else {
                    worst_incremental_step = worst_incremental_step.max(step);
                    assert!(
                        step <= INCREMENTAL_STEP_BOUND,
                        "post-anchor record {index} of {size} revalidated with {step} node reads"
                    );
                }
            }

            // The very first proof from the immutable anchor is the one full
            // walk, and at post-anchor size 1 it is literally one radix
            // insertion path. The memo is cold, so it costs no revalidation.
            assert_eq!(bootstrap_step, ENGINE_HISTORY_RADIX_DEPTH as usize + 1);

            // Re-proving an unchanged head from the same anchor is composition
            // against an already-proved endpoint, so the residual walk is
            // empty. What it still costs — and must cost — is the live current
            // root, freshly read and authenticated. The anchor here is the
            // empty authority, which names no node.
            let before = node_reads(&store);
            let repeated = history
                .authenticate_current_history_extension(anchor)
                .unwrap();
            assert_eq!(node_reads(&store) - before, 1);
            assert_eq!(repeated.after().generation, size as u64);

            // A fresh open holds no memo and must pay the complete anchor ->
            // head walk, which visits every post-anchor record.
            drop(history);
            drop(store);
            let reopened_store = ObjectStore::open(&archive, workspace).unwrap();
            let reopened = reopened_store.open_engine_history(binding).unwrap();
            let before = node_reads(&reopened_store);
            let full = reopened
                .authenticate_current_history_extension(anchor)
                .unwrap();
            let full_walk = node_reads(&reopened_store) - before;
            assert_eq!(full.before(), anchor);
            assert_eq!(full.after().generation, size as u64);
            assert!(
                full_walk >= size,
                "a fresh full proof of {size} post-anchor records read only {full_walk} nodes"
            );
            let before = node_reads(&reopened_store);
            reopened
                .authenticate_current_history_extension(anchor)
                .unwrap();
            assert_eq!(node_reads(&reopened_store) - before, 1);

            if size >= 512 {
                assert!(
                    full_walk >= 100 * worst_incremental_step,
                    "full walk {full_walk} is not dominated by the {worst_incremental_step}-read \
                     incremental step at size {size}"
                );
            }
            full_walks.push(full_walk);

            drop(reopened);
            drop(reopened_store);
            crate::test_support::remove_dir_all(root);
        }

        // The unmemoized proof cost tracks the post-anchor history — which is
        // exactly the growth the memo removes from every step above.
        //
        // Measured here, one row per post-anchor size (worst memoized step /
        // full walk): 2 -> 35/65, 32 -> 36/1009, 64 -> 36/2001, 128 -> 36/3985,
        // 256 -> 37/7919, 512 -> 37/15633. The full walk is 31*size, the
        // memoized step is flat, and the separation is already two orders of
        // magnitude at 512. Growing the history further re-measures those same
        // two shapes at greater cost; it does not make either claim stronger.
        // Detection is immediate rather than asymptotic: with the memo removed
        // every step becomes its own full walk, and the per-step bound above is
        // breached by the third record (97 reads against 68), so the size that
        // catches a regression is small even though the size that makes the
        // contrast *legible* is 512.
        assert!(
            full_walks[2] >= 5 * full_walks[1],
            "full-walk cost {full_walks:?} did not scale with the post-anchor history"
        );
    }

    /// The accidental single-user damage a live immutable index node has to be
    /// re-checked against: it vanishes, it is cut short, or it is replaced by
    /// same-length bytes that no longer hash to the name it is stored under.
    #[derive(Clone, Copy, Debug)]
    enum HistoryNodeFault {
        Deleted,
        Truncated,
        DigestCorrupted,
    }

    fn history_node_path(
        archive: &Path,
        binding: crate::oplog::hot_engine::ProjectionStorageBinding,
        digest: ContentDigest,
    ) -> PathBuf {
        archive
            .join(ENGINE_HISTORY_DIR)
            .join(binding.endpoint.endpoint_id.to_string())
            .join(ENGINE_HISTORY_NODES_DIR)
            .join(history_index_filename(digest))
    }

    fn damage_history_node(path: &Path, fault: HistoryNodeFault) {
        let pristine = std::fs::read(path).unwrap();
        assert!(pristine.len() > 2);
        match fault {
            HistoryNodeFault::Deleted => std::fs::remove_file(path).unwrap(),
            HistoryNodeFault::Truncated => {
                std::fs::write(path, &pristine[..pristine.len() / 2]).unwrap();
            }
            HistoryNodeFault::DigestCorrupted => {
                let mut substituted = pristine.clone();
                let last = substituted.len() - 1;
                substituted[last] ^= 0xFF;
                assert_eq!(substituted.len(), pristine.len());
                std::fs::write(path, &substituted).unwrap();
            }
        }
    }

    /// The content address of the leaf `publish` stores for one record, so a
    /// test can name an individual deep node without walking the index.
    fn history_leaf_digest(batch_id: BatchId, record: &[u8]) -> ContentDigest {
        ContentDigest::of(
            &postcard::to_allocvec(&HistoryIndexNode::Leaf {
                schema_version: ENGINE_HISTORY_INDEX_SCHEMA_VERSION,
                batch_id,
                record: record.to_vec(),
            })
            .unwrap(),
        )
    }

    /// A memo may shorten the *walk*, never the availability and integrity
    /// facts the walk establishes about the live current endpoint. Losing the
    /// node named by `after.index_root` must be rejected identically whether or
    /// not this open already proved a transition from the same anchor.
    #[test]
    fn authenticated_history_extension_revalidates_the_live_current_root() {
        let root = test_root("live-current-root-revalidation");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(49_000));
        let binding = enrolled_binding(49_010);
        let open = || {
            let store = ObjectStore::open(&archive, workspace).unwrap();
            let history = store.open_engine_history(binding).unwrap();
            (store, history)
        };

        let (store, history) = open();
        let anchor = history.current_authority().unwrap();
        for index in 0..4_usize {
            history
                .publish(
                    spread_history_batch_id(index),
                    b"live current root record",
                    EngineHistoryBinding::empty(),
                )
                .unwrap();
        }
        let current = history.current_authority().unwrap();
        drop(history);
        drop(store);

        let node = history_node_path(&archive, binding, current.index_root);
        let pristine = std::fs::read(&node).unwrap();

        for fault in [
            HistoryNodeFault::Deleted,
            HistoryNodeFault::Truncated,
            HistoryNodeFault::DigestCorrupted,
        ] {
            // A warm store: the memo already holds `anchor -> current`, so the
            // residual step is empty and nothing below the head is walked.
            let (warm_store, warm) = open();
            warm.authenticate_current_history_extension(anchor).unwrap();
            warm.authenticate_current_history_extension(anchor).unwrap();

            damage_history_node(&node, fault);

            let warm_error = warm
                .authenticate_current_history_extension(anchor)
                .expect_err(&format!("a warm store accepted the {fault:?} current root"));
            drop(warm);
            drop(warm_store);

            let (fresh_store, fresh) = open();
            let fresh_error = fresh
                .authenticate_current_history_extension(anchor)
                .expect_err(&format!(
                    "a fresh store accepted the {fault:?} current root"
                ));
            assert_eq!(
                std::mem::discriminant(&warm_error),
                std::mem::discriminant(&fresh_error),
                "the {fault:?} current root was rejected differently warm ({warm_error:?}) and \
                 fresh ({fresh_error:?})"
            );
            drop(fresh);
            drop(fresh_store);

            // Repairing the exact immutable bytes restores the exact verdict.
            std::fs::write(&node, &pristine).unwrap();
            let (repaired_store, repaired) = open();
            assert_eq!(
                repaired
                    .authenticate_current_history_extension(anchor)
                    .unwrap()
                    .after(),
                current
            );
            drop(repaired);
            drop(repaired_store);
        }

        crate::test_support::remove_dir_all(root);
    }

    /// A publication that failed on a missing index node must not leave behind
    /// a memo that can authorize a later mutation against that storage.
    #[test]
    fn incomplete_publication_on_a_lost_index_node_disarms_the_memo() {
        let root = test_root("publication-failure-disarms-memo");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(50_000));
        let binding = enrolled_binding(50_010);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        let node_reads = || store.instrumentation().history_index_reads;

        let anchor = history.current_authority().unwrap();
        for index in 0..4_usize {
            history
                .publish(
                    spread_history_batch_id(index),
                    b"publication failure record",
                    EngineHistoryBinding::empty(),
                )
                .unwrap();
        }
        let current = history.current_authority().unwrap();
        history
            .authenticate_current_history_extension(anchor)
            .unwrap();
        let before = node_reads();
        history
            .authenticate_current_history_extension(anchor)
            .unwrap();
        let warm_step = node_reads() - before;

        // The publication fails because the live root node is gone, which is
        // exactly a detected history/index I/O failure.
        let node = history_node_path(&archive, binding, current.index_root);
        let pristine = std::fs::read(&node).unwrap();
        std::fs::remove_file(&node).unwrap();
        assert!(history
            .publish(
                spread_history_batch_id(4),
                b"never committed",
                EngineHistoryBinding::empty(),
            )
            .is_err());

        // While the damage stands, the warm store must reject exactly like a
        // fresh one, and it may not authorize anything.
        assert!(history
            .authenticate_current_history_extension(anchor)
            .is_err());

        // Even after the exact immutable bytes come back, nothing this open
        // proved before the failure may be reused as a shortcut: the proof is
        // re-derived by the complete walk a fresh open would perform.
        std::fs::write(&node, &pristine).unwrap();
        let before = node_reads();
        assert_eq!(
            history
                .authenticate_current_history_extension(anchor)
                .unwrap()
                .after(),
            current
        );
        let disarmed_step = node_reads() - before;
        assert!(
            disarmed_step > warm_step,
            "a failed publication left a {disarmed_step}-read shortcut over the {warm_step}-read \
             warm path instead of disarming the memo"
        );

        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    /// Deeper immutable nodes stay a *previously authenticated* in-memory fact:
    /// re-reading them on every call is exactly the lifetime-sized work the
    /// memo exists to remove. The compensating contract is causal — the first
    /// operation that re-encounters the damage disarms the memo permanently, so
    /// from that point the warm store decides exactly like a fresh one.
    #[test]
    fn deeper_history_node_loss_disarms_the_memo_when_it_is_re_encountered() {
        let root = test_root("deep-node-loss");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(51_000));
        let binding = enrolled_binding(51_010);
        const RECORD: &[u8] = b"deep node loss record";
        let open = || {
            let store = ObjectStore::open(&archive, workspace).unwrap();
            let history = store.open_engine_history(binding).unwrap();
            (store, history)
        };

        let (store, history) = open();
        let node_reads = || store.instrumentation().history_index_reads;
        let anchor = history.current_authority().unwrap();
        for index in 0..4_usize {
            history
                .publish(
                    spread_history_batch_id(index),
                    RECORD,
                    EngineHistoryBinding::empty(),
                )
                .unwrap();
        }
        // Warm the memo on the first four records, then extend once more so the
        // residual `middle -> current` step provably cannot revisit them.
        history
            .authenticate_current_history_extension(anchor)
            .unwrap();
        history
            .publish(
                spread_history_batch_id(4),
                RECORD,
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        history
            .authenticate_current_history_extension(anchor)
            .unwrap();
        let current = history.current_authority().unwrap();

        // A record whose radix path leaves the last insertion's path at the
        // root, so its leaf is outside the residual step by construction.
        let last_nibble = history_key_nibble(spread_history_batch_id(4).as_uuid().as_bytes(), 0);
        let doomed = (0..4_usize)
            .find(|index| {
                history_key_nibble(spread_history_batch_id(*index).as_uuid().as_bytes(), 0)
                    != last_nibble
            })
            .expect("a record diverging from the last insertion at the root nibble");
        let leaf = history_node_path(
            &archive,
            binding,
            history_leaf_digest(spread_history_batch_id(doomed), RECORD),
        );
        let pristine = std::fs::read(&leaf).unwrap();
        std::fs::remove_file(&leaf).unwrap();

        // The warm store still accepts: the missing leaf is a fact this open
        // authenticated earlier and nothing has re-encountered it yet. This is
        // the documented, bounded residual, and it costs only the constant
        // live-endpoint revalidation.
        let before = node_reads();
        assert_eq!(
            history
                .authenticate_current_history_extension(anchor)
                .unwrap()
                .after(),
            current
        );
        let warm_step = node_reads() - before;
        assert!(
            warm_step <= 2,
            "the warm step cost {warm_step} reads instead of the constant endpoint revalidation"
        );

        // Any replay, rebuild or lookup that reaches the damaged region — the
        // only way its bytes can reach a user or a projection — fails and
        // latches the fault.
        let replay_error = history
            .materialize(current.index_root)
            .expect_err("a replay over a missing leaf must fail");
        let warm_error = history
            .authenticate_current_history_extension(anchor)
            .expect_err("a re-encountered fault must disarm the memo");

        // A fresh open walks the whole post-anchor history and rejects the same
        // way, with no memo involved at all.
        drop(history);
        drop(store);
        let (fresh_store, fresh) = open();
        let fresh_error = fresh
            .authenticate_current_history_extension(anchor)
            .expect_err("a fresh store must reject a history with a missing leaf");
        assert_eq!(
            std::mem::discriminant(&warm_error),
            std::mem::discriminant(&fresh_error),
            "disarmed warm rejection {warm_error:?} differs from the fresh rejection \
             {fresh_error:?} (replay reported {replay_error:?})"
        );
        drop(fresh);
        drop(fresh_store);

        std::fs::write(&leaf, &pristine).unwrap();
        let (repaired_store, repaired) = open();
        assert_eq!(
            repaired
                .authenticate_current_history_extension(anchor)
                .unwrap()
                .after(),
            current
        );
        drop(repaired);
        drop(repaired_store);

        crate::test_support::remove_dir_all(root);
    }

    /// The projection-work caller re-anchors on the head it just accepted, so
    /// it presents a different anchor every batch while the promoted-runtime
    /// caller keeps proving from one immutable bootstrap anchor. The bounded
    /// memo must not let the moving anchor evict the fixed one.
    #[test]
    fn authenticated_history_extension_memo_keeps_the_reused_anchor_resident() {
        let root = test_root("memo-anchor-residency");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(48_000));
        let binding = enrolled_binding(48_010);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        let node_reads = || store.instrumentation().history_index_reads;

        let fixed_anchor = history.current_authority().unwrap();
        let mut moving_anchor = fixed_anchor;
        for index in 0..200_usize {
            history
                .publish(
                    spread_history_batch_id(index),
                    b"anchor residency record",
                    EngineHistoryBinding::empty(),
                )
                .unwrap();
            // The moving anchor introduces a brand-new memo entry every batch.
            history
                .authenticate_current_history_extension(moving_anchor)
                .unwrap();
            moving_anchor = history.current_authority().unwrap();

            let before = node_reads();
            let fixed = history
                .authenticate_current_history_extension(fixed_anchor)
                .unwrap();
            let step = node_reads() - before;
            assert_eq!(fixed.after(), moving_anchor);
            if index > 0 {
                assert!(
                    step <= INCREMENTAL_STEP_BOUND,
                    "the reused anchor was evicted at batch {index}: {step} node reads"
                );
            }
        }

        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn authenticated_history_extension_memo_preserves_every_rejection() {
        let root = test_root("memo-preserves-rejection");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(47_000));
        let binding = enrolled_binding(47_010);
        let head_file = archive
            .join(ENGINE_HISTORY_DIR)
            .join(binding.endpoint.endpoint_id.to_string())
            .join(ENGINE_HISTORY_HEAD_FILE);
        let read_head = || std::fs::read(&head_file).unwrap();
        let open = || {
            let store = ObjectStore::open(&archive, workspace).unwrap();
            let history = store.open_engine_history(binding).unwrap();
            (store, history)
        };
        let node_reads = |store: &ObjectStore| store.instrumentation().history_index_reads;

        // Lineage A, built on one store whose memo is warmed from two anchors
        // by the ordinary publish-then-revalidate loop.
        let (store, history) = open();
        let empty_head = read_head();
        let anchor = history.current_authority().unwrap();
        let mut lineage_a = Vec::new();
        for index in 0..4_usize {
            history
                .publish(
                    spread_history_batch_id(index),
                    b"lineage a record",
                    EngineHistoryBinding::empty(),
                )
                .unwrap();
            lineage_a.push(history.current_authority().unwrap());
            history
                .authenticate_current_history_extension(anchor)
                .unwrap();
            history
                .authenticate_current_history_extension(lineage_a[0])
                .unwrap();
        }
        let (first, third, fourth) = (lineage_a[0], lineage_a[2], lineage_a[3]);
        let head_fourth = read_head();

        // An exact self-transition against a warm memo stays exact.
        let exact = history
            .authenticate_current_history_extension(fourth)
            .unwrap();
        assert_eq!(exact.before(), fourth);
        assert_eq!(exact.after(), fourth);

        // Failed publish, crash cut *before* the head swap: the head does not
        // move, so the verdict is unchanged — but an incomplete publication
        // disarms the memo, so the same verdict is re-derived by the complete
        // walk instead of inherited from anything this open proved earlier.
        fail_next_engine_history_head_swap();
        assert!(history
            .publish(
                spread_history_batch_id(4),
                b"never committed",
                EngineHistoryBinding::empty(),
            )
            .is_err());
        assert_eq!(read_head(), head_fourth);
        assert_eq!(history.current_authority().unwrap(), fourth);
        let before = node_reads(&store);
        assert_eq!(
            history
                .authenticate_current_history_extension(first)
                .unwrap()
                .after(),
            fourth
        );
        assert!(
            node_reads(&store) - before > LIVE_ENDPOINT_REVALIDATION_BOUND,
            "an incomplete publication left the memo armed"
        );
        drop(history);
        drop(store);

        // Failed publish, crash cut *after* the head swap: the record is
        // durable even though the call returned an error. The next open must
        // authenticate the advanced head from the same anchor.
        let (store, history) = open();
        fail_next_engine_history_after_head_swap();
        assert!(history
            .publish(
                spread_history_batch_id(4),
                b"committed under a failed publish",
                EngineHistoryBinding::empty(),
            )
            .is_err());
        let head_fifth = read_head();
        assert_ne!(head_fifth, head_fourth);
        drop(history);
        drop(store);

        let (store, history) = open();
        let fifth = history.current_authority().unwrap();
        assert_eq!(fifth.generation, 5);
        let advanced = history
            .authenticate_current_history_extension(first)
            .unwrap();
        assert_eq!(advanced.before(), first);
        assert_eq!(advanced.after(), fifth);
        drop(history);
        drop(store);

        // A divergent lineage B of the same and then greater length, published
        // over a head that was replaced back to the empty authority.
        let (store, history) = open();
        std::fs::write(&head_file, &empty_head).unwrap();
        let mut lineage_b = Vec::new();
        let mut heads_b = Vec::new();
        for index in 0..6_usize {
            history
                .publish(
                    spread_history_batch_id(1_000 + index),
                    b"lineage b record",
                    EngineHistoryBinding::empty(),
                )
                .unwrap();
            lineage_b.push(history.current_authority().unwrap());
            heads_b.push(read_head());
        }
        let divergent_equal = lineage_b[4];
        let divergent_longer = lineage_b[5];
        assert_eq!(divergent_equal.generation, fifth.generation);
        assert_eq!(divergent_longer.generation, fifth.generation + 1);
        drop(history);
        drop(store);

        // The adversarial phase runs on one store that never publishes, so it
        // follows the live head while keeping the memo it warmed.
        let (store, history) = open();
        std::fs::write(&head_file, &head_fifth).unwrap();
        assert_eq!(history.current_authority().unwrap(), fifth);
        let before = node_reads(&store);
        assert_eq!(
            history
                .authenticate_current_history_extension(first)
                .unwrap()
                .after(),
            fifth
        );
        assert!(
            node_reads(&store) - before > 0,
            "a fresh open must pay the full walk once"
        );
        assert_eq!(
            history
                .authenticate_current_history_extension(third)
                .unwrap()
                .after(),
            fifth
        );

        // Head replacement: rollback. The memo holds `first -> fifth` and
        // `third -> fifth`, and neither may survive the retreat as a proof
        // about `fifth` itself.
        std::fs::write(&head_file, &head_fourth).unwrap();
        assert!(matches!(
            history.authenticate_current_history_extension(fifth),
            Err(StoreError::MalformedHistoryIndex)
        ));
        // A still-valid ancestor anchor is still accepted, even though its warm
        // memo endpoint is now ahead of the live head: a stale memo may only
        // fail to compose, never turn an acceptance into a rejection.
        assert_eq!(
            history
                .authenticate_current_history_extension(first)
                .unwrap()
                .after(),
            fourth
        );

        // Head replacement: equal-generation divergence.
        std::fs::write(&head_file, &heads_b[4]).unwrap();
        assert_eq!(history.current_authority().unwrap(), divergent_equal);
        assert!(matches!(
            history.authenticate_current_history_extension(fifth),
            Err(StoreError::MalformedHistoryIndex)
        ));
        // Cached-middle substitution: `fourth` and `fifth` are exactly the
        // endpoints the memo holds for `first` and `third`, and they are what
        // an attacker would want spliced in to reach the divergent head.
        // Neither composition nor the fallback walk can manufacture that proof.
        assert!(matches!(
            history.authenticate_current_history_extension(first),
            Err(StoreError::MalformedHistoryIndex)
        ));
        assert!(matches!(
            history.authenticate_current_history_extension(third),
            Err(StoreError::MalformedHistoryIndex)
        ));

        // Head replacement: higher-generation non-descendant.
        std::fs::write(&head_file, &heads_b[5]).unwrap();
        assert_eq!(history.current_authority().unwrap(), divergent_longer);
        assert!(matches!(
            history.authenticate_current_history_extension(fifth),
            Err(StoreError::MalformedHistoryIndex)
        ));
        assert!(matches!(
            history.authenticate_current_history_extension(first),
            Err(StoreError::MalformedHistoryIndex)
        ));

        // Substituted anchors — a real generation paired with another real
        // index root — miss the memo and are rejected by the full walk.
        for substituted in [
            EngineHistoryAuthority {
                generation: third.generation,
                index_root: first.index_root,
            },
            EngineHistoryAuthority {
                generation: first.generation,
                index_root: fifth.index_root,
            },
            EngineHistoryAuthority {
                generation: fifth.generation,
                index_root: lineage_b[0].index_root,
            },
        ] {
            assert!(matches!(
                history.authenticate_current_history_extension(substituted),
                Err(StoreError::MalformedHistoryIndex)
            ));
        }
        // A `before` whose generation and root disagree about emptiness is
        // rejected outright, memo or not.
        assert!(matches!(
            history.authenticate_current_history_extension(EngineHistoryAuthority {
                generation: 0,
                index_root: first.index_root,
            }),
            Err(StoreError::MalformedHistoryIndex)
        ));

        // Head replacement: rollback below a non-empty anchor.
        std::fs::write(&head_file, &empty_head).unwrap();
        assert!(matches!(
            history.authenticate_current_history_extension(first),
            Err(StoreError::MalformedHistoryIndex)
        ));
        drop(history);
        drop(store);

        // Fresh store: no memo survives the open, the first proof pays the
        // complete walk, and every verdict above is reproduced without it.
        std::fs::write(&head_file, &heads_b[5]).unwrap();
        let (store, history) = open();
        let before = node_reads(&store);
        let fresh = history
            .authenticate_current_history_extension(lineage_b[0])
            .unwrap();
        assert!(node_reads(&store) - before > 0);
        assert_eq!(fresh.before(), lineage_b[0]);
        assert_eq!(fresh.after(), divergent_longer);
        for rejected in [first, third, fourth, fifth] {
            assert!(matches!(
                history.authenticate_current_history_extension(rejected),
                Err(StoreError::MalformedHistoryIndex)
            ));
        }
        drop(history);
        drop(store);

        std::fs::write(&head_file, &head_fourth).unwrap();
        let (store, history) = open();
        let before = node_reads(&store);
        let reproved = history
            .authenticate_current_history_extension(first)
            .unwrap();
        assert!(node_reads(&store) - before > 0);
        assert_eq!(reproved.after(), fourth);

        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn prior_version_durable_history_requires_upgrade_without_writes() {
        fn snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
            let mut result = BTreeMap::new();
            let mut pending = vec![path.to_path_buf()];
            while let Some(directory) = pending.pop() {
                for entry in std::fs::read_dir(&directory).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_type().unwrap().is_dir() {
                        pending.push(entry.path());
                    } else {
                        result.insert(
                            entry.path().strip_prefix(path).unwrap().to_path_buf(),
                            std::fs::read(entry.path()).unwrap(),
                        );
                    }
                }
            }
            result
        }

        let root = test_root("prior-durable-root");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(50));
        let endpoint = crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(51));
        let binding = crate::oplog::hot_engine::ProjectionStorageBinding {
            endpoint: crate::oplog::ProjectionEndpointBinding {
                endpoint_id: endpoint,
                device_id: crate::oplog::DeviceId::from_uuid(Uuid::from_u128(52)),
                graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"test",
                    b"prior-durable-root",
                ),
            },
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"test",
                b"prior-durable-receipts",
            ),
        };
        let archive_path = root.join("archive");
        let store = ObjectStore::open(&archive_path, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(53)),
                b"preserved accepted history",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        let control = archive_path
            .join(ENGINE_HISTORY_DIR)
            .join(endpoint.to_string());
        let prior_version = ENGINE_HISTORY_ROOT_SCHEMA_VERSION - 1;
        let prior_claim = postcard::to_allocvec(&(
            prior_version,
            workspace,
            endpoint,
            binding.endpoint.graph_resource_id,
        ))
        .unwrap();
        std::fs::write(control.join(ENGINE_HISTORY_CLAIM_FILE), prior_claim).unwrap();
        let before = snapshot(&archive_path);

        let reopened = ObjectStore::open(&archive_path, workspace).unwrap();
        assert!(matches!(
            reopened.open_engine_history(binding),
            Err(StoreError::UpgradeRequired {
                store: "engine history",
                found,
                current
            }) if found == prior_version && current == ENGINE_HISTORY_ROOT_SCHEMA_VERSION
        ));
        assert_eq!(snapshot(&archive_path), before);
        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn engine_history_failure_before_head_swap_keeps_prior_authority() {
        let root = test_root("history-pre-head-swap-failure");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(55_000));
        let endpoint = crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(55_001));
        let binding = crate::oplog::hot_engine::ProjectionStorageBinding {
            endpoint: crate::oplog::ProjectionEndpointBinding {
                endpoint_id: endpoint,
                device_id: crate::oplog::DeviceId::from_uuid(Uuid::from_u128(55_002)),
                graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"test",
                    b"history-pre-head-swap-failure",
                ),
            },
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"test",
                b"history-pre-head-swap-failure-receipts",
            ),
        };
        let archive_path = root.join("archive");
        let store = ObjectStore::open(&archive_path, workspace).unwrap();
        let history = store.open_engine_history(binding).unwrap();
        let before = history.current_with_binding().unwrap();
        fail_next_engine_history_head_swap();
        assert!(history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(55_003)),
                b"unpublished record-v8 candidate",
                EngineHistoryBinding::empty(),
            )
            .is_err());
        assert_eq!(history.current_with_binding().unwrap(), before);
        drop(history);
        drop(store);

        let reopened = ObjectStore::open(&archive_path, workspace).unwrap();
        let reopened_history = reopened.open_engine_history(binding).unwrap();
        assert_eq!(reopened_history.current_with_binding().unwrap(), before);
        drop(reopened_history);
        drop(reopened);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn synthetic_future_durable_history_rejects_before_creating_layout() {
        let root = test_root("future-durable-root");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(60));
        let endpoint = crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(61));
        let binding = crate::oplog::hot_engine::ProjectionStorageBinding {
            endpoint: crate::oplog::ProjectionEndpointBinding {
                endpoint_id: endpoint,
                device_id: crate::oplog::DeviceId::from_uuid(Uuid::from_u128(62)),
                graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"test",
                    b"future-durable-root",
                ),
            },
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"test",
                b"future-durable-receipts",
            ),
        };
        let archive_path = root.join("archive");
        let store = ObjectStore::open(&archive_path, workspace).unwrap();
        let control = archive_path
            .join(ENGINE_HISTORY_DIR)
            .join(endpoint.to_string());
        std::fs::create_dir_all(&control).unwrap();
        std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), b"future-head").unwrap();
        let future_claim = postcard::to_allocvec(&(
            ENGINE_HISTORY_ROOT_SCHEMA_VERSION + 1,
            workspace,
            endpoint,
            binding.endpoint.graph_resource_id,
            binding.receipt_store_id,
        ))
        .unwrap();
        std::fs::write(control.join(ENGINE_HISTORY_CLAIM_FILE), future_claim).unwrap();
        let before = snapshot_tree(&archive_path);

        assert!(matches!(
            store.open_engine_history(binding),
            Err(StoreError::UnsupportedStoreVersion {
                store: "engine history",
                version
            }) if version == ENGINE_HISTORY_ROOT_SCHEMA_VERSION + 1
        ));
        assert_eq!(snapshot_tree(&archive_path), before);
        assert!(!control.join(ENGINE_HISTORY_NODES_DIR).exists());
        assert!(!control.join(ENGINE_HISTORY_ROOTS_DIR).exists());
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_history_claim_rejections_are_mutation_free_for_every_open_mode() {
        #[derive(Clone, Copy)]
        enum ClaimKind {
            Prior,
            Future,
            Synthetic,
        }

        #[derive(Clone, Copy)]
        enum OpenMode {
            Ordinary,
            Promoted,
            HistoryOnly,
        }

        fn promoted_state(
            store: &ObjectStore,
            workspace: WorkspaceId,
            binding: crate::oplog::hot_engine::ProjectionStorageBinding,
            seed: u8,
        ) -> PromotedRuntimeStateV1 {
            let lineage = LineageDigest::from_bytes([seed; 32]);
            let import_id = ImportId::from_digest([seed.wrapping_add(1); 32]);
            let aggregate = BootstrapAggregateManifestV1::empty(
                workspace,
                lineage,
                binding.endpoint.graph_resource_id,
                import_id,
            )
            .unwrap();
            PromotedRuntimeStateV1 {
                schema_version: PROMOTED_RUNTIME_STATE_SCHEMA_VERSION,
                lineage_mode: PromotedLineageModeV1::BootstrapAnchoredHomogeneous,
                workspace_id: workspace,
                lineage_digest: lineage,
                catalog_document_id: DocumentId::from_uuid(Uuid::from_u128(u128::from(seed))),
                endpoint_id: binding.endpoint.endpoint_id,
                device_id: binding.endpoint.device_id,
                graph_resource_id: binding.endpoint.graph_resource_id,
                receipt_store_id: binding.receipt_store_id,
                archive_resource_id: store.provision_enrolled_archive_resource_id().unwrap(),
                archive_control_binding: control_directory_identity(&store.capability)
                    .unwrap()
                    .binding_digest(),
                bootstrap: BootstrapAggregateHistoryBindingV1::for_aggregate(&aggregate).unwrap(),
                bootstrap_projection:
                    PromotedBootstrapProjectionBindingV1::synthetic_for_object_store_test(
                        workspace,
                        lineage,
                        binding.endpoint.endpoint_id,
                        binding.endpoint.device_id,
                        binding.endpoint.graph_resource_id,
                        binding.receipt_store_id,
                        control_directory_identity(&store.capability)
                            .unwrap()
                            .binding_digest(),
                        ContentDigest::from_bytes(*aggregate.publication_id().as_bytes()),
                        ContentDigest::from_bytes(*aggregate.aggregate_digest().as_bytes()),
                        ContentDigest::from_bytes(*import_id.as_bytes()),
                        aggregate.parts().len() as u32,
                        ContentDigest::of(b"synthetic frontier"),
                        0,
                        EngineHistoryStore::empty_root(),
                    ),
                bootstrap_import_id: import_id,
                anchor_history_generation: 0,
                anchor_history_index_root: EngineHistoryStore::empty_root(),
                anchor_acceptance_sequence: 0,
                anchor_accepted_frontier_state_digest: ContentDigest::of(b"synthetic frontier"),
                enrollment_verification_digest: ContentDigest::of(b"synthetic verification"),
                enrollment_binding_digest: ContentDigest::of(b"synthetic enrollment"),
                promotion_session_id: SessionId::from_uuid(Uuid::from_u128(u128::from(seed) + 1)),
            }
        }

        for (mode_label, mode) in [
            ("ordinary", OpenMode::Ordinary),
            ("promoted", OpenMode::Promoted),
            ("history-only", OpenMode::HistoryOnly),
        ] {
            for (claim_label, claim_kind) in [
                ("prior", ClaimKind::Prior),
                ("future", ClaimKind::Future),
                ("synthetic", ClaimKind::Synthetic),
            ] {
                let root = test_root(&format!("sealed-{mode_label}-{claim_label}"));
                let workspace = WorkspaceId::from_uuid(Uuid::new_v4());
                let binding = enrolled_binding(Uuid::new_v4().as_u128());
                let archive_path = root.join("archive");
                let store = ObjectStore::open(&archive_path, workspace).unwrap();
                let expected = matches!(mode, OpenMode::Promoted)
                    .then(|| promoted_state(&store, workspace, binding, claim_label.len() as u8));
                let control = archive_path
                    .join(ENGINE_HISTORY_DIR)
                    .join(binding.endpoint.endpoint_id.to_string());
                std::fs::create_dir_all(&control).unwrap();
                std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), b"synthetic-head").unwrap();
                let claim = match claim_kind {
                    ClaimKind::Prior => postcard::to_allocvec(&(
                        ENGINE_HISTORY_ROOT_SCHEMA_VERSION - 1,
                        workspace,
                        binding.endpoint.endpoint_id,
                        binding.endpoint.graph_resource_id,
                    ))
                    .unwrap(),
                    ClaimKind::Future => postcard::to_allocvec(&(
                        ENGINE_HISTORY_ROOT_SCHEMA_VERSION + 1,
                        workspace,
                        binding.endpoint.endpoint_id,
                        binding.endpoint.graph_resource_id,
                        binding.receipt_store_id,
                    ))
                    .unwrap(),
                    ClaimKind::Synthetic => b"synthetic history claim".to_vec(),
                };
                std::fs::write(control.join(ENGINE_HISTORY_CLAIM_FILE), claim).unwrap();
                let before = snapshot_tree(&archive_path);

                let rejected = match mode {
                    OpenMode::Ordinary => store.seal_enrolled_projection(binding).is_err(),
                    OpenMode::Promoted => store
                        .seal_promoted_projection(
                            binding,
                            expected.as_ref().expect("promoted state"),
                        )
                        .is_err(),
                    OpenMode::HistoryOnly => store.seal_history_only(binding).is_err(),
                };
                assert!(rejected, "{mode_label} open accepted a {claim_label} claim");
                assert_eq!(
                    snapshot_tree(&archive_path),
                    before,
                    "{mode_label} rejection mutated the {claim_label} archive"
                );
                assert!(
                    !archive_path
                        .join(ENGINE_HISTORY_TRANSITION_LOCK_FILE)
                        .exists(),
                    "{mode_label} rejection created the transition lock for a {claim_label} claim"
                );
                crate::test_support::remove_dir_all(root);
            }
        }
    }

    #[test]
    fn sealed_history_substitution_after_preflight_fails_closed() {
        let root = test_root("sealed-history-preflight-substitution");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(65));
        let binding = enrolled_binding(66);
        let substitute = enrolled_binding(67);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        drop(store.open_engine_history(binding).unwrap());
        drop(store.open_engine_history(substitute).unwrap());
        std::fs::remove_file(archive.join(ENGINE_HISTORY_TRANSITION_LOCK_FILE)).unwrap();

        let histories = archive.join(ENGINE_HISTORY_DIR);
        let target = histories.join(binding.endpoint.endpoint_id.to_string());
        let displaced = histories.join("displaced-after-preflight");
        let substitute_control = histories.join(substitute.endpoint.endpoint_id.to_string());
        let target_hook = target.clone();
        let displaced_hook = displaced.clone();
        set_sealed_history_after_preflight_hook(move || {
            std::fs::rename(target_hook, displaced_hook).unwrap();
            std::fs::rename(substitute_control, target).unwrap();
        });

        let error = store
            .seal_history_only(binding)
            .err()
            .expect("substituted history must be rejected")
            .1;
        assert!(matches!(error, StoreError::MalformedHistoryIndex));
        assert!(displaced.is_dir(), "the original control was not displaced");
        assert!(
            archive.join(ENGINE_HISTORY_TRANSITION_LOCK_FILE).is_file(),
            "compatible preflight must still reach the durable lock"
        );
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn valid_sealed_history_open_recreates_and_uses_transition_lock() {
        let root = test_root("valid-sealed-history-lock");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(68));
        let binding = enrolled_binding(69);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        drop(store.open_engine_history(binding).unwrap());
        let lock_path = archive.join(ENGINE_HISTORY_TRANSITION_LOCK_FILE);
        std::fs::remove_file(&lock_path).unwrap();

        let open = store.seal_history_only(binding).unwrap();
        assert!(
            lock_path.is_file(),
            "valid sealed open did not create the lock"
        );
        let (store, history) = open.into_history().unwrap();
        let guard = AdvisoryTransitionGuard::lock(&history.transition_lock).unwrap();
        drop(guard);
        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    #[ignore = "subprocess helper invoked by sealed_history_validation_serializes_a_concurrent_valid_transition"]
    fn sealed_history_transition_subprocess_helper() {
        let Ok(archive) = std::env::var("TINE_SEALED_OPEN_HELPER_ARCHIVE") else {
            return;
        };
        let contended = std::env::var("TINE_SEALED_OPEN_HELPER_CONTENDED").unwrap();
        let store = ObjectStore::open(
            Path::new(&archive),
            WorkspaceId::from_uuid(Uuid::from_u128(0x7e00)),
        )
        .unwrap();
        let history = store.open_engine_history(enrolled_binding(0x7e01)).unwrap();
        set_advisory_transition_contention_hook(move || {
            std::fs::write(contended, b"contended").unwrap();
        });
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(0x7e02)),
                b"serialized valid history transition",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
    }

    #[test]
    fn sealed_history_validation_serializes_a_concurrent_valid_transition() {
        let root = test_root("sealed-history-transition-serialization");
        let archive = root.join("archive");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x7e00));
        let binding = enrolled_binding(0x7e01);
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let publishing_history = store.open_engine_history(binding).unwrap();
        let initial_head = publishing_history.read_live_head_root().unwrap().0;
        let contended = root.join("child-contended");
        let child = Arc::new(Mutex::new(None));
        let child_for_hook = Arc::clone(&child);
        let archive_for_hook = archive.clone();
        let contended_for_hook = contended.clone();

        set_sealed_history_authority_window_hook(move |stage| match stage {
            SealedHistoryAuthorityWindowStage::Locked => {
                let spawned = std::process::Command::new(std::env::current_exe().unwrap())
                    .arg("sealed_history_transition_subprocess_helper")
                    .arg("--ignored")
                    .arg("--nocapture")
                    .env(
                        "TINE_SEALED_OPEN_HELPER_ARCHIVE",
                        archive_for_hook.as_os_str(),
                    )
                    .env(
                        "TINE_SEALED_OPEN_HELPER_CONTENDED",
                        contended_for_hook.as_os_str(),
                    )
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                *child_for_hook.lock().unwrap() = Some(spawned);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while !contended_for_hook.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert!(
                    contended_for_hook.exists(),
                    "valid subprocess transition did not contend with sealed validation"
                );
            }
            SealedHistoryAuthorityWindowStage::Validated => {
                assert!(
                    child_for_hook
                        .lock()
                        .unwrap()
                        .as_mut()
                        .expect("transition subprocess")
                        .try_wait()
                        .unwrap()
                        .is_none(),
                    "valid subprocess transition interleaved with sealed authority validation"
                );
            }
        });

        let sealed = store.seal_existing_engine_history(binding).unwrap();
        let opened = match sealed {
            SealedControl::Existing(history) => history,
            SealedControl::Absent(_) => panic!("initialized history reopened as absent"),
        };
        assert_eq!(
            *opened.authoritative_head.lock().unwrap(),
            Some(initial_head),
            "sealed validation did not pin the pre-transition authority"
        );
        let output = child
            .lock()
            .unwrap()
            .take()
            .expect("transition subprocess")
            .wait_with_output()
            .unwrap();
        assert!(
            output.status.success(),
            "serialized subprocess transition failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            publishing_history
                .read_live_head_root()
                .unwrap()
                .1
                .generation,
            1
        );
        drop(opened);
        drop(publishing_history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn authenticated_durable_root_version_matrix_rejects_without_writes() {
        let root = test_root("durable-root-version-matrix");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(70));
        let endpoint = crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(71));
        let binding = crate::oplog::hot_engine::ProjectionStorageBinding {
            endpoint: crate::oplog::ProjectionEndpointBinding {
                endpoint_id: endpoint,
                device_id: crate::oplog::DeviceId::from_uuid(Uuid::from_u128(72)),
                graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"test",
                    b"durable-root-version-matrix",
                ),
            },
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"test",
                b"durable-root-version-matrix-receipts",
            ),
        };
        let archive_path = root.join("archive");
        let store = ObjectStore::open(&archive_path, workspace).unwrap();
        drop(store.open_engine_history(binding).unwrap());
        let control = archive_path
            .join(ENGINE_HISTORY_DIR)
            .join(endpoint.to_string());
        let roots = control.join(ENGINE_HISTORY_ROOTS_DIR);

        for version in [
            ENGINE_HISTORY_ROOT_SCHEMA_VERSION - 1,
            ENGINE_HISTORY_ROOT_SCHEMA_VERSION + 1,
        ] {
            let authenticated_root = DurableEngineHistoryRoot {
                schema_version: version,
                workspace_id: workspace,
                endpoint_id: endpoint,
                graph_resource_id: binding.endpoint.graph_resource_id,
                receipt_store_id: binding.receipt_store_id,
                generation: 0,
                index_root: EngineHistoryStore::empty_root(),
                latest_batch_id: None,
                binding: DurableEngineHistoryBinding::ordinary(EngineHistoryBinding::empty()),
            };
            let bytes = postcard::to_allocvec(&authenticated_root).unwrap();
            let digest = ContentDigest::of(&bytes);
            std::fs::write(roots.join(engine_history_root_filename(digest)), &bytes).unwrap();
            std::fs::write(control.join(ENGINE_HISTORY_HEAD_FILE), digest.to_string()).unwrap();
            let before = snapshot_tree(&archive_path);

            let error = store.preflight_enrolled_projection(binding).unwrap_err();
            if version < ENGINE_HISTORY_ROOT_SCHEMA_VERSION {
                assert!(matches!(
                    error,
                    StoreError::UpgradeRequired {
                        store: "engine history",
                        found,
                        current,
                    } if found == version && current == ENGINE_HISTORY_ROOT_SCHEMA_VERSION
                ));
            } else {
                assert!(matches!(
                    error,
                    StoreError::UnsupportedStoreVersion {
                        store: "engine history",
                        version: found,
                    } if found == version
                ));
            }
            assert_eq!(snapshot_tree(&archive_path), before);
            assert!(!archive_path
                .join(super::super::scratch_store::SCRATCH_DIR)
                .exists());
            assert!(!archive_path.join(PROJECTION_WORK_DIR).exists());
        }

        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        windows
    ))]
    #[test]
    fn authenticated_history_publication_is_concurrent_canonical_and_missing_safe() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let root = test_root("concurrent");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(10));
        let store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let history = Arc::new(store.start_engine_history().unwrap());
        let batch_id = BatchId::from_uuid(Uuid::from_u128(11));
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let history = Arc::clone(&history);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    history.insert(
                        EngineHistoryStore::empty_root(),
                        batch_id,
                        b"same immutable record",
                    )
                })
            })
            .collect();
        let roots: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect();
        assert!(roots.iter().all(|candidate| *candidate == roots[0]));
        assert_eq!(
            history.lookup(roots[0], batch_id).unwrap(),
            Some(b"same immutable record".to_vec())
        );

        let malformed = HistoryIndexNode::Branch {
            schema_version: ENGINE_HISTORY_INDEX_SCHEMA_VERSION,
            depth: 0,
            children: vec![(1, roots[0]), (1, roots[0])],
        };
        assert!(matches!(
            history.publish_node(&malformed),
            Err(StoreError::MalformedHistoryIndex)
        ));

        let run = std::fs::read_dir(root.join("archive/engine-history"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::remove_file(run.join(history_index_filename(roots[0]))).unwrap();
        assert!(matches!(
            history.lookup(roots[0], batch_id),
            Err(StoreError::Io(error)) if error.kind() == ErrorKind::NotFound
        ));
        drop(history);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn authenticated_block_claim_point_index_is_bounded_and_fails_closed() {
        let root = test_root("block-claim-integrity");
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(20));
        let store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let index = store.start_block_claim_index().unwrap();
        let records: Vec<_> = (0_u128..256)
            .map(|value| {
                (
                    Uuid::from_u128(10_000 + value).into_bytes(),
                    BlockClaimIndexValue::from_slice(&value.to_be_bytes()),
                )
            })
            .collect();
        let before_insert = store.instrumentation();
        let mut index_root = index
            .insert_many(BlockClaimIndexRoot::default(), &records)
            .unwrap();
        let after_insert = store.instrumentation();
        assert_eq!(
            after_insert.directory_enumerations - before_insert.directory_enumerations,
            0
        );
        assert!(after_insert.block_claim_index_writes > before_insert.block_claim_index_writes);
        assert_eq!(
            after_insert.block_claim_index_syncs - before_insert.block_claim_index_syncs,
            0,
            "the reconstructible run-local index must not enter the authoritative durability path"
        );

        let requested = [
            records[0].0,
            records[127].0,
            records[255].0,
            Uuid::from_u128(99_999).into_bytes(),
        ];
        let before_lookup = store.instrumentation();
        let found = index.lookup_many(index_root, &requested).unwrap();
        let after_lookup = store.instrumentation();
        assert_eq!(found.len(), 3);
        assert_eq!(found[&records[127].0], records[127].1);
        assert_eq!(
            after_lookup.directory_enumerations - before_lookup.directory_enumerations,
            0
        );
        assert!(
            after_lookup.block_claim_index_reads - before_lookup.block_claim_index_reads <= 16,
            "point lookup escaped the requested radix paths"
        );

        assert!(matches!(
            index.lookup_many(index_root, &[records[1].0, records[0].0]),
            Err(StoreError::MalformedBlockClaimIndex)
        ));
        assert!(matches!(
            index.insert_many(
                index_root,
                &[
                    (records[1].0, BlockClaimIndexValue::from_slice(&[1])),
                    (records[0].0, BlockClaimIndexValue::from_slice(&[2]))
                ]
            ),
            Err(StoreError::MalformedBlockClaimIndex)
        ));

        let replacement = BlockClaimIndexValue::from_slice(b"newest canonical value");
        index_root = index
            .insert_many(index_root, &[(records[0].0, replacement.clone())])
            .unwrap();
        assert_eq!(
            index.lookup_many(index_root, &requested[..1]).unwrap()[&records[0].0],
            replacement,
            "newest authenticated segment must deterministically shadow an older value"
        );

        let run = std::fs::read_dir(root.join("archive/block-claim-index"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let page_path = run.join(BLOCK_CLAIM_INDEX_FILE);
        let original = std::fs::read(&page_path).unwrap();
        let global_ref = index_root.global_filter.unwrap();
        let global_payload_offset = usize::try_from(global_ref.offset).unwrap() + 4;
        let mut tampered_global = original.clone();
        tampered_global[global_payload_offset] ^= 1;
        std::fs::write(&page_path, &tampered_global).unwrap();
        assert!(matches!(
            index.lookup_many(index_root, &requested[..1]),
            Err(StoreError::BlockClaimIndexPathMismatch(found)) if found == global_ref.digest
        ));
        std::fs::write(&page_path, &original).unwrap();

        let root_segment = *index_root
            .levels
            .iter()
            .flatten()
            .flatten()
            .max_by_key(|segment| segment.generation)
            .unwrap();
        let root_ref = root_segment.page_ref;
        let payload_offset = usize::try_from(root_ref.offset).unwrap() + 4;
        let mut tampered = original.clone();
        tampered[payload_offset] ^= 1;
        std::fs::write(&page_path, &tampered).unwrap();
        assert!(matches!(
            index.lookup_many(index_root, &requested[..1]),
            Err(StoreError::BlockClaimIndexPathMismatch(found)) if found == root_ref.digest
        ));

        std::fs::write(&page_path, &original[..original.len() - 1]).unwrap();
        assert!(matches!(
            index.lookup_many(index_root, &requested[..1]),
            Err(StoreError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof
        ));
        std::fs::write(&page_path, &original).unwrap();

        let malformed = BlockClaimIndexPage::Branch {
            schema_version: BLOCK_CLAIM_INDEX_SCHEMA_VERSION,
            depth: 0,
            children: vec![(0, root_ref), (0, root_ref)],
        };
        let malformed_bytes = postcard::to_allocvec(&malformed).unwrap();
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&page_path)
            .unwrap();
        let offset = file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(&(malformed_bytes.len() as u32).to_be_bytes())
            .unwrap();
        file.write_all(&malformed_bytes).unwrap();
        file.sync_all().unwrap();
        let mut malformed_root = BlockClaimIndexRoot {
            next_generation: 1,
            global_filter: index_root.global_filter,
            ..BlockClaimIndexRoot::default()
        };
        malformed_root.levels[0][0] = Some(BlockClaimSegmentRef {
            generation: 1,
            entry_count: root_segment.entry_count,
            page_ref: BlockClaimPageRef {
                offset,
                encoded_len: malformed_bytes.len() as u32,
                digest: ContentDigest::of(&malformed_bytes),
            },
            filter_ref: root_segment.filter_ref,
        });
        assert!(matches!(
            index.lookup_many(malformed_root, &requested[..1]),
            Err(StoreError::MalformedBlockClaimIndex)
        ));

        let mut full_level = index_root;
        full_level.next_generation = BLOCK_CLAIM_SEGMENTS_PER_LEVEL as u64;
        for (slot, segment) in full_level.levels[0].iter_mut().enumerate() {
            let mut selected = root_segment;
            selected.generation = slot as u64 + 1;
            *segment = Some(selected);
        }
        let compacted_key = Uuid::from_u128(200_000).into_bytes();
        let compacted_value = BlockClaimIndexValue::from_slice(b"level carry");
        let compacted = index
            .insert_many(full_level, &[(compacted_key, compacted_value.clone())])
            .unwrap();
        assert!(compacted.levels[0].iter().all(Option::is_none));
        assert_eq!(compacted.levels[1].iter().flatten().count(), 1);
        let compacted_lookup = index
            .lookup_many(compacted, &[records[0].0, compacted_key])
            .unwrap();
        assert_eq!(compacted_lookup[&records[0].0], replacement);
        assert_eq!(compacted_lookup[&compacted_key], compacted_value);

        drop(index);
        drop(store);
        crate::test_support::remove_dir_all(root);
    }
}

pub(crate) fn require_regular_entry(
    file_type: &cap_std::fs::FileType,
    name: &str,
) -> Result<(), StoreError> {
    tine_storage::require_regular_entry(file_type, name).map_err(filesystem_error_without_collision)
}

pub(crate) fn sync_dir_required(dir: &Dir) -> Result<(), StoreError> {
    tine_storage::sync_dir_required(dir).map_err(Into::into)
}

#[cfg(test)]
mod bootstrap_store_tests {
    use super::*;
    use crate::oplog::bootstrap_import::{
        BootstrapPartitionProfileV1, OperationDigestV1, OperationLeafV1, OperationRootV1,
        PayloadObjectRootV1, SourceBlobIndexBuilderV1, SourceContentDigestV1,
        SourceInventoryIndexBuilderV1, SourceSpanRootV1, SourceSpanV1,
    };
    use crate::oplog::{
        BatchCausalDot, CausalPeerId, DeviceId, DocumentId, FrontierV2, ImportId, ManagedPath,
        ManagedTextKind, ObjectKind, ProjectionEndpointBinding, ProjectionEndpointId,
        ProjectionReceiptStoreId, SemanticEffectDigest, SessionId,
    };
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    struct EmptyBootstrapFixture {
        root: PathBuf,
        archive: PathBuf,
        workspace: WorkspaceId,
        aggregate: BootstrapAggregateManifestV1,
    }

    impl EmptyBootstrapFixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("tine-bootstrap-store-{label}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let archive = root.join("archive");
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x7000));
            let aggregate = BootstrapAggregateManifestV1::empty(
                workspace,
                LineageDigest::from_bytes([0x71; 32]),
                crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"bootstrap-store-test",
                    label.as_bytes(),
                ),
                ImportId::from_digest([0x72; 32]),
            )
            .unwrap();
            Self {
                root,
                archive,
                workspace,
                aggregate,
            }
        }

        fn store(&self) -> ObjectStore {
            ObjectStore::open(&self.archive, self.workspace).unwrap()
        }

        fn history_binding(
            &self,
            endpoint: u128,
        ) -> crate::oplog::hot_engine::ProjectionStorageBinding {
            crate::oplog::hot_engine::ProjectionStorageBinding {
                endpoint: ProjectionEndpointBinding {
                    endpoint_id: ProjectionEndpointId::from_uuid(Uuid::from_u128(endpoint)),
                    device_id: DeviceId::from_uuid(Uuid::from_u128(endpoint + 1)),
                    graph_resource_id:
                        crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                            b"bootstrap-history-test",
                            &endpoint.to_be_bytes(),
                        ),
                },
                receipt_store_id: ProjectionReceiptStoreId::from_capability_identity(
                    b"bootstrap-history-test",
                    &(endpoint + 2).to_be_bytes(),
                ),
            }
        }
    }

    impl Drop for EmptyBootstrapFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    struct BootstrapPartFixture {
        descriptor: BootstrapPartDescriptorV1,
        manifest_bytes: Vec<u8>,
        object_bytes: Vec<Vec<u8>>,
        spans: BootstrapPartSpanIndexV1,
        history_record: Vec<u8>,
    }

    struct BootstrapFixture {
        root: PathBuf,
        archive: PathBuf,
        workspace: WorkspaceId,
        aggregate: BootstrapAggregateManifestV1,
        inventory_root: SourceInventoryRootV1,
        inventory_pages: Vec<SourceInventoryIndexPageV1>,
        blob_root: SourceBlobChunkRootV1,
        blob_pages: Vec<SourceBlobIndexPageV1>,
        source_chunks: Vec<(SourceBlobChunkDigestV1, Vec<u8>)>,
        parts: Vec<BootstrapPartFixture>,
    }

    impl BootstrapFixture {
        fn new(label: &str, part_count: u32) -> Self {
            assert!(part_count > 0);
            let root = std::env::temp_dir()
                .join(format!("tine-bootstrap-store-{label}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let archive = root.join("archive");
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x7600));
            let lineage = LineageDigest::from_bytes([0x76; 32]);
            let import_id = ImportId::from_digest(*ContentDigest::of(label.as_bytes()).as_bytes());
            let profile = BootstrapPartitionProfileV1::v1().digest();
            let source_bytes = b"one bounded bootstrap source".to_vec();
            let source_digest = ContentDigest::of(&source_bytes);
            let source = SourceLeafV1::new(
                ManagedTextKind::Page,
                ManagedPath::parse("pages/bootstrap.md").unwrap(),
                SourceContentDigestV1::from_bytes(*source_digest.as_bytes()),
                source_bytes.len() as u64,
            )
            .unwrap();
            let inventory_root =
                SourceInventoryRootV1::from_leaves(std::slice::from_ref(&source)).unwrap();
            let mut inventory = SourceInventoryIndexBuilderV1::new(inventory_root);
            assert!(inventory.push(source.clone()).unwrap().is_none());
            let inventory_pages = vec![inventory.finish().unwrap().unwrap()];

            let chunk_digest = SourceBlobChunkDigestV1::from_bytes(*source_digest.as_bytes());
            let blob_descriptor = SourceBlobChunkDescriptorV1::new(
                source.digest(),
                0,
                1,
                0,
                source_bytes.len() as u32,
                chunk_digest,
            )
            .unwrap();
            let blob_root = SourceBlobChunkRootV1::from_descriptors(
                std::slice::from_ref(&source),
                std::slice::from_ref(&blob_descriptor),
            )
            .unwrap();
            let mut blob = SourceBlobIndexBuilderV1::new(blob_root);
            assert!(blob.push(blob_descriptor).unwrap().is_none());
            let blob_pages = vec![blob.finish().unwrap().unwrap()];

            let mut parts = Vec::with_capacity(part_count as usize);
            let mut descriptors = Vec::with_capacity(part_count as usize);
            let mut predecessor = None;
            let mut frontier = ArchiveLocalFrontierBindingV1::initial(import_id, profile);
            for ordinal in 0..part_count {
                let payload = format!("bootstrap semantic effect {ordinal}").into_bytes();
                let object = OperationObject::new(
                    workspace,
                    DocumentId::from_uuid(Uuid::from_u128(0x7700 + u128::from(ordinal))),
                    ObjectKind::SemanticEffect,
                    payload.clone(),
                )
                .unwrap();
                let object_bytes = object.encode().unwrap();
                let object_descriptor = object.descriptor().unwrap();
                let payload_descriptor = PayloadObjectDescriptorV1::new(
                    object_descriptor.content_digest(),
                    object_descriptor.encoded_byte_length(),
                )
                .unwrap();
                let span =
                    SourceSpanV1::new(source.digest(), 0, source_bytes.len() as u64).unwrap();
                let evidence = BootstrapImportPartEvidenceV1::new(
                    import_id,
                    profile,
                    ordinal,
                    part_count,
                    SourceSpanRootV1::from_spans(std::slice::from_ref(&span)).unwrap(),
                    OperationRootV1::from_operations(&[OperationLeafV1::new(
                        OperationDigestV1::from_bytes([ordinal as u8 + 1; 32]),
                        payload.len() as u64,
                    )
                    .unwrap()])
                    .unwrap(),
                    PayloadObjectRootV1::from_objects(std::slice::from_ref(&payload_descriptor))
                        .unwrap(),
                    predecessor,
                )
                .unwrap();
                let device = DeviceId::from_uuid(Uuid::from_u128(0x7800 + u128::from(ordinal)));
                let manifest = OperationBatch::new_with_causality(
                    workspace,
                    lineage,
                    evidence.batch_id(),
                    device,
                    SessionId::from_uuid(Uuid::from_u128(0x7900 + u128::from(ordinal))),
                    BatchOrigin::BootstrapImport,
                    BatchCausalDot::new(CausalPeerId::from_device_id(device), 1).unwrap(),
                    Vec::new(),
                    FrontierV2::new(Vec::new()).unwrap(),
                    SemanticEffectDigest::of(&payload),
                    vec![object_descriptor],
                )
                .unwrap();
                let manifest_bytes = manifest.encode().unwrap();
                let spans = BootstrapPartSpanIndexV1::new(evidence.part_id(), vec![span]).unwrap();
                let span_bytes = spans.encode().unwrap();
                let descriptor = BootstrapPartDescriptorV1::accepted(
                    evidence,
                    BootstrapManifestFingerprintV1::from_bytes(
                        *ContentDigest::of(&manifest_bytes).as_bytes(),
                    ),
                    std::slice::from_ref(&payload_descriptor),
                    &[FullObjectDescriptorV1::manifest_defined(
                        *ContentDigest::of(&span_bytes).as_bytes(),
                        span_bytes.len() as u64,
                    )
                    .unwrap()],
                    frontier,
                )
                .unwrap();
                predecessor = Some(descriptor.part_id());
                frontier = descriptor.post_frontier();
                descriptors.push(descriptor);
                parts.push(BootstrapPartFixture {
                    descriptor,
                    manifest_bytes,
                    object_bytes: vec![object_bytes],
                    spans,
                    history_record: format!("cold bootstrap history record {ordinal}").into_bytes(),
                });
            }
            let aggregate = BootstrapAggregateManifestV1::new_for_import(
                workspace,
                lineage,
                crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"bootstrap-store-test",
                    label.as_bytes(),
                ),
                import_id,
                1,
                inventory_root,
                inventory_pages.len() as u32,
                blob_root,
                blob_pages.len() as u32,
                profile,
                descriptors,
                ArchiveLocalFrontierBindingV1::initial(import_id, profile),
                frontier,
            )
            .unwrap();
            Self {
                root,
                archive,
                workspace,
                aggregate,
                inventory_root,
                inventory_pages,
                blob_root,
                blob_pages,
                source_chunks: vec![(chunk_digest, source_bytes)],
                parts,
            }
        }

        fn store(&self) -> ObjectStore {
            ObjectStore::open(&self.archive, self.workspace).unwrap()
        }

        fn history_binding(
            &self,
            endpoint: u128,
        ) -> crate::oplog::hot_engine::ProjectionStorageBinding {
            history_storage_binding(endpoint)
        }

        fn publish_replay_prefix(&self, store: &ObjectStore) {
            for page in &self.inventory_pages {
                store
                    .publish_bootstrap_source_inventory_page(self.inventory_root, page)
                    .unwrap();
            }
            for page in &self.blob_pages {
                store
                    .publish_bootstrap_source_blob_page(self.blob_root, page)
                    .unwrap();
            }
            for (digest, bytes) in &self.source_chunks {
                store
                    .publish_bootstrap_source_chunk(*digest, bytes)
                    .unwrap();
            }
            for part in &self.parts {
                for object in &part.object_bytes {
                    store.publish_bootstrap_object_bytes(object).unwrap();
                }
                store
                    .publish_bootstrap_part_pack_for_test(part.descriptor, &part.object_bytes)
                    .unwrap();
                store
                    .publish_bootstrap_part_artifacts(
                        part.descriptor,
                        &part.manifest_bytes,
                        &part.spans,
                    )
                    .unwrap();
            }
        }

        fn publish_committed(&self, store: &ObjectStore) -> BootstrapPublicationIdV1 {
            self.publish_replay_prefix(store);
            store
                .publish_bootstrap_aggregate_prefix(&self.aggregate)
                .unwrap();
            store.commit_bootstrap_aggregate(&self.aggregate).unwrap()
        }

        fn prepared_history(
            &self,
            binding: BootstrapAggregateHistoryBindingV1,
        ) -> Vec<PreparedBootstrapHistoryRecordV1<'_>> {
            self.parts
                .iter()
                .map(|part| {
                    PreparedBootstrapHistoryRecordV1::unchecked_for_history_index_test(
                        part.descriptor,
                        &part.history_record,
                        binding,
                    )
                })
                .collect()
        }
    }

    impl Drop for BootstrapFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn history_storage_binding(
        endpoint: u128,
    ) -> crate::oplog::hot_engine::ProjectionStorageBinding {
        crate::oplog::hot_engine::ProjectionStorageBinding {
            endpoint: ProjectionEndpointBinding {
                endpoint_id: ProjectionEndpointId::from_uuid(Uuid::from_u128(endpoint)),
                device_id: DeviceId::from_uuid(Uuid::from_u128(endpoint + 1)),
                graph_resource_id: crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"bootstrap-history-test",
                    &endpoint.to_be_bytes(),
                ),
            },
            receipt_store_id: ProjectionReceiptStoreId::from_capability_identity(
                b"bootstrap-history-test",
                &(endpoint + 2).to_be_bytes(),
            ),
        }
    }

    fn bootstrap_prepared_batch(workspace: WorkspaceId) -> PreparedBatch {
        let payload = b"bootstrap semantic effect".to_vec();
        let object = OperationObject::new(
            workspace,
            DocumentId::from_uuid(Uuid::from_u128(0x7300)),
            ObjectKind::SemanticEffect,
            payload.clone(),
        )
        .unwrap();
        let descriptor = object.descriptor().unwrap();
        let device = DeviceId::from_uuid(Uuid::from_u128(0x7301));
        let manifest = OperationBatch::new_with_causality(
            workspace,
            LineageDigest::from_bytes([0x73; 32]),
            BatchId::from_uuid(Uuid::from_u128(0x7302)),
            device,
            SessionId::from_uuid(Uuid::from_u128(0x7303)),
            BatchOrigin::BootstrapImport,
            BatchCausalDot::new(CausalPeerId::from_device_id(device), 1).unwrap(),
            Vec::new(),
            FrontierV2::new(Vec::new()).unwrap(),
            SemanticEffectDigest::of(&payload),
            vec![descriptor],
        )
        .unwrap();
        PreparedBatch::new(manifest, vec![object]).unwrap()
    }

    #[test]
    fn generic_publication_rejects_bootstrap_batches_without_prefix_writes() {
        let fixture = EmptyBootstrapFixture::new("generic-rejection");
        let store = fixture.store();
        let prepared = bootstrap_prepared_batch(fixture.workspace);
        assert!(matches!(
            store.stage_manifest_bytes(&prepared.manifest().encode().unwrap()),
            Err(StoreError::BootstrapBatchRequiresDirectPublication)
        ));
        assert!(matches!(
            store.publish_prepared(&prepared),
            Err(StoreError::BootstrapBatchRequiresDirectPublication)
        ));
        assert!(store.committed_manifests().unwrap().is_empty());
        assert!(!fixture.archive.join(BOOTSTRAP_DIR).exists());
    }

    #[test]
    fn detached_publisher_closes_without_changing_ordinary_index_publication() {
        let fixture = EmptyBootstrapFixture::new("detached-publisher-closed");
        let store = fixture.store();
        let capability = store.bootstrap_authoring_capability().unwrap();
        let (publication, indexes) = capability.begin_detached_authoring().unwrap();
        DETACHED_BOOTSTRAP_BATCH_FINISH_COUNT.with(|count| count.set(0));
        let detached_root = indexes
            .logseq_claim_index()
            .insert_many(
                super::super::uuid_claim_index::LogseqClaimIndexRoot::empty(),
                &BTreeMap::from([(b"detached key".to_vec(), b"detached value".to_vec())]),
            )
            .unwrap();
        let construction = indexes
            .logseq_claim_index()
            .finish_detached_construction(detached_root)
            .unwrap()
            .expect("detached construction completed");
        let stats = construction.stats();
        assert_eq!(stats.loose_publication_calls, 0);
        assert!(stats.pack_publication_calls > 0);
        assert!(stats.catalog_publication_calls > 0);
        assert_eq!(stats.head_transitions, 1);
        assert_eq!(stats.capacity_fallbacks, 0);
        let completed = publication.finish_without_patricia_for_test().unwrap();
        assert_eq!(completed.publication_count(), 0);
        assert_eq!(completed.existing_publication_count(), 0);
        DETACHED_BOOTSTRAP_BATCH_FINISH_COUNT.with(|count| assert_eq!(count.get(), 1));

        assert!(matches!(
            indexes.logseq_claim_index().insert_many(
                detached_root,
                &BTreeMap::from([(b"post-finish key".to_vec(), b"value".to_vec())]),
            ),
            Err(StoreError::Bootstrap(message)) if message.contains("closed")
        ));

        let ordinary_root = capability
            .logseq_claim_index()
            .insert_many(
                super::super::uuid_claim_index::LogseqClaimIndexRoot::empty(),
                &BTreeMap::from([(b"ordinary key".to_vec(), b"ordinary value".to_vec())]),
            )
            .unwrap();
        assert_eq!(
            capability
                .logseq_claim_index()
                .lookup(ordinary_root, b"ordinary key")
                .unwrap(),
            Some(b"ordinary value".to_vec())
        );
    }

    fn stage_fixture_bootstrap_batch<'a>(
        fixture: &BootstrapFixture,
        store: &'a ObjectStore,
    ) -> BootstrapPublicationBatch<'a> {
        let mut publication = store.begin_bootstrap_publication_batch().unwrap();
        for page in &fixture.inventory_pages {
            publication
                .publish_source_inventory_page(fixture.inventory_root, page)
                .unwrap();
        }
        for page in &fixture.blob_pages {
            publication
                .publish_source_blob_page(fixture.blob_root, page)
                .unwrap();
        }
        for part in &fixture.parts {
            let mut pack = Vec::new();
            for object in &part.object_bytes {
                let length = u32::try_from(object.len()).unwrap();
                pack.extend_from_slice(&length.to_be_bytes());
                pack.extend_from_slice(object);
            }
            let pack_length = pack.len() as u64;
            publication
                .publish_part_pack(
                    part.descriptor,
                    &mut std::io::Cursor::new(pack),
                    pack_length,
                )
                .unwrap();
            publication
                .publish_part_artifacts(part.descriptor, &part.manifest_bytes, &part.spans)
                .unwrap();
        }
        publication
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn bootstrap_batch_retains_no_per_artifact_handles() {
        let fixture = BootstrapFixture::new("batch-no-retained-handles", 8);
        let store = fixture.store();
        let publication = stage_fixture_bootstrap_batch(&fixture, &store);
        assert_eq!(publication.retained_artifact_handle_count(), 0);
        publication.finish(&fixture.aggregate).unwrap();
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[test]
    fn portable_bootstrap_batch_republishes_exact_residue_durably() {
        let fixture = BootstrapFixture::new("portable-batch-exact-residue", 2);
        let store = fixture.store();
        stage_fixture_bootstrap_batch(&fixture, &store)
            .finish(&fixture.aggregate)
            .unwrap();
        let retry = stage_fixture_bootstrap_batch(&fixture, &store);
        assert_eq!(retry.retained_artifact_handle_count(), 0);
        retry.finish(&fixture.aggregate).unwrap();
    }

    #[test]
    fn multipart_publication_cuts_remain_invisible_until_the_exact_commit() {
        let fixture = BootstrapFixture::new("multipart-cuts", 2);
        let store = fixture.store();
        let enumerations = store.instrumentation().directory_enumerations;
        let assert_absent = || {
            assert!(matches!(
                store.inspect_bootstrap_aggregate(&fixture.aggregate),
                BootstrapPublicationInspectionV1::Absent
            ));
            assert!(store
                .load_bootstrap_publication(fixture.aggregate.publication_id())
                .is_err());
        };
        assert_absent();

        for page in &fixture.inventory_pages {
            store
                .publish_bootstrap_source_inventory_page(fixture.inventory_root, page)
                .unwrap();
            store
                .publish_bootstrap_source_inventory_page(fixture.inventory_root, page)
                .unwrap();
            assert_absent();
        }
        for page in &fixture.blob_pages {
            store
                .publish_bootstrap_source_blob_page(fixture.blob_root, page)
                .unwrap();
            store
                .publish_bootstrap_source_blob_page(fixture.blob_root, page)
                .unwrap();
            assert_absent();
        }
        for (digest, bytes) in &fixture.source_chunks {
            store
                .publish_bootstrap_source_chunk(*digest, bytes)
                .unwrap();
            store
                .publish_bootstrap_source_chunk(*digest, bytes)
                .unwrap();
            assert_absent();
        }
        for part in &fixture.parts {
            store
                .publish_bootstrap_part_pack_for_test(part.descriptor, &part.object_bytes)
                .unwrap();
            store
                .publish_bootstrap_part_pack_for_test(part.descriptor, &part.object_bytes)
                .unwrap();
            assert_absent();
            store
                .publish_bootstrap_part_artifacts(
                    part.descriptor,
                    &part.manifest_bytes,
                    &part.spans,
                )
                .unwrap();
            store
                .publish_bootstrap_part_artifacts(
                    part.descriptor,
                    &part.manifest_bytes,
                    &part.spans,
                )
                .unwrap();
            assert_absent();
        }

        store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&fixture.aggregate),
            BootstrapPublicationInspectionV1::Pending
        ));
        let publication_id = store
            .commit_bootstrap_aggregate(&fixture.aggregate)
            .unwrap();
        let publication = store.load_bootstrap_publication(publication_id).unwrap();
        assert_eq!(publication.aggregate(), &fixture.aggregate);
        for (ordinal, expected) in fixture.parts.iter().enumerate() {
            let loaded = store.load_bootstrap_part(&publication, ordinal).unwrap();
            assert_eq!(loaded.manifest().encode().unwrap(), expected.manifest_bytes);
            assert_eq!(loaded.spans(), &expected.spans);
            assert_eq!(
                loaded
                    .objects()
                    .iter()
                    .map(|object| object.encode().unwrap())
                    .collect::<Vec<_>>(),
                expected.object_bytes
            );
        }
        assert_eq!(
            store.instrumentation().directory_enumerations,
            enumerations,
            "direct bootstrap publication and reopen must not enumerate prefixes"
        );
    }

    #[test]
    fn empty_publication_is_pending_invisible_committed_and_directly_reopenable() {
        let fixture = EmptyBootstrapFixture::new("empty-lifecycle");
        let store = fixture.store();
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&fixture.aggregate),
            BootstrapPublicationInspectionV1::Absent
        ));
        let enumerations = store.instrumentation().directory_enumerations;
        store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&fixture.aggregate),
            BootstrapPublicationInspectionV1::Pending
        ));
        assert!(store.committed_manifests().unwrap().is_empty());
        let publication_id = store
            .commit_bootstrap_aggregate(&fixture.aggregate)
            .unwrap();
        let loaded = store.load_bootstrap_publication(publication_id).unwrap();
        assert_eq!(loaded.aggregate(), &fixture.aggregate);
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&fixture.aggregate),
            BootstrapPublicationInspectionV1::Committed(_)
        ));
        assert_eq!(
            store.instrumentation().directory_enumerations,
            enumerations + 1,
            "only the explicit ordinary batch enumeration may scan"
        );
    }

    #[test]
    fn direct_identity_conflict_and_truncated_committed_aggregate_fail_closed() {
        let fixture = EmptyBootstrapFixture::new("conflict-truncation");
        let store = fixture.store();
        store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        let aggregates = store
            .bootstrap_namespace(BOOTSTRAP_AGGREGATES_DIR, false)
            .unwrap();
        let name = hex_bytes(fixture.aggregate.aggregate_digest().as_bytes());
        assert!(matches!(
            publish_bootstrap_immutable(
                &aggregates,
                &name,
                b"different bytes",
                "bootstrap aggregate",
                name.clone(),
            ),
            Err(StoreError::BootstrapArtifactCollision { .. })
        ));
        let publication_id = store
            .commit_bootstrap_aggregate(&fixture.aggregate)
            .unwrap();
        let aggregate_path = fixture
            .archive
            .join(BOOTSTRAP_DIR)
            .join(BOOTSTRAP_AGGREGATES_DIR)
            .join(name);
        let bytes = std::fs::read(&aggregate_path).unwrap();
        std::fs::write(&aggregate_path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(store.load_bootstrap_publication(publication_id).is_err());
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&fixture.aggregate),
            BootstrapPublicationInspectionV1::CorruptOrConflicting(_)
        ));
    }

    #[test]
    fn committed_same_publication_id_with_different_aggregate_is_a_typed_conflict() {
        let fixture = BootstrapFixture::new("publication-conflict", 1);
        let store = fixture.store();
        fixture.publish_committed(&store);
        let profile = fixture.aggregate.profile_digest();
        let initial =
            ArchiveLocalFrontierBindingV1::initial(fixture.aggregate.import_id(), profile);
        let conflicting = BootstrapAggregateManifestV1::new_for_import(
            fixture.workspace,
            fixture.aggregate.lineage_digest(),
            crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                b"bootstrap-store-conflict",
                b"different aggregate, same portable identity",
            ),
            fixture.aggregate.import_id(),
            fixture.inventory_root.source_count(),
            fixture.inventory_root,
            fixture.inventory_pages.len() as u32,
            fixture.blob_root,
            fixture.blob_pages.len() as u32,
            profile,
            fixture.parts.iter().map(|part| part.descriptor).collect(),
            initial,
            fixture.parts.last().unwrap().descriptor.post_frontier(),
        )
        .unwrap();
        assert_eq!(
            conflicting.publication_id(),
            fixture.aggregate.publication_id()
        );
        assert_ne!(
            conflicting.aggregate_digest(),
            fixture.aggregate.aggregate_digest()
        );
        store
            .publish_bootstrap_aggregate_prefix(&conflicting)
            .unwrap();
        assert!(matches!(
            store.commit_bootstrap_aggregate(&conflicting),
            Err(StoreError::BootstrapArtifactCollision { .. })
        ));
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&conflicting),
            BootstrapPublicationInspectionV1::CorruptOrConflicting(
                StoreError::BootstrapArtifactMismatch("committed bootstrap aggregate")
            )
        ));
    }

    #[test]
    fn missing_and_truncated_direct_replay_artifacts_fail_closed() {
        let missing_chunk = BootstrapFixture::new("missing-chunk", 1);
        let store = missing_chunk.store();
        missing_chunk.publish_replay_prefix(&store);
        store
            .publish_bootstrap_aggregate_prefix(&missing_chunk.aggregate)
            .unwrap();
        let chunk_name = hex_bytes(missing_chunk.source_chunks[0].0.as_bytes());
        std::fs::remove_file(
            missing_chunk
                .archive
                .join(BOOTSTRAP_DIR)
                .join(BOOTSTRAP_SOURCE_CHUNKS_DIR)
                .join(chunk_name),
        )
        .unwrap();
        assert!(store
            .commit_bootstrap_aggregate(&missing_chunk.aggregate)
            .is_err());

        let missing_pack = BootstrapFixture::new("missing-pack", 1);
        let store = missing_pack.store();
        missing_pack.publish_replay_prefix(&store);
        store
            .publish_bootstrap_aggregate_prefix(&missing_pack.aggregate)
            .unwrap();
        std::fs::remove_file(
            missing_pack
                .archive
                .join(BOOTSTRAP_DIR)
                .join(BOOTSTRAP_PART_PACKS_DIR)
                .join(hex_bytes(
                    missing_pack.parts[0].descriptor.part_id().as_bytes(),
                )),
        )
        .unwrap();
        assert!(store
            .commit_bootstrap_aggregate(&missing_pack.aggregate)
            .is_err());

        let truncated = BootstrapFixture::new("truncated-span", 1);
        let store = truncated.store();
        let publication_id = truncated.publish_committed(&store);
        let span_path = truncated
            .archive
            .join(BOOTSTRAP_DIR)
            .join(BOOTSTRAP_PART_SPANS_DIR)
            .join(hex_bytes(
                truncated.parts[0].descriptor.part_id().as_bytes(),
            ));
        let bytes = std::fs::read(&span_path).unwrap();
        std::fs::write(&span_path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(store.load_bootstrap_publication(publication_id).is_err());
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&truncated.aggregate),
            BootstrapPublicationInspectionV1::CorruptOrConflicting(_)
        ));
    }

    #[test]
    fn source_pages_with_valid_individual_roots_still_require_terminal_inventory_coverage() {
        let fixture = EmptyBootstrapFixture::new("source-terminal");
        let store = fixture.store();
        let first_bytes = b"uncovered source".to_vec();
        let second_bytes = b"covered source".to_vec();
        let first = SourceLeafV1::new(
            ManagedTextKind::Page,
            ManagedPath::parse("pages/uncovered.md").unwrap(),
            SourceContentDigestV1::from_bytes(*ContentDigest::of(&first_bytes).as_bytes()),
            first_bytes.len() as u64,
        )
        .unwrap();
        let second = SourceLeafV1::new(
            ManagedTextKind::Page,
            ManagedPath::parse("pages/covered.md").unwrap(),
            SourceContentDigestV1::from_bytes(*ContentDigest::of(&second_bytes).as_bytes()),
            second_bytes.len() as u64,
        )
        .unwrap();
        let inventory_root =
            SourceInventoryRootV1::from_leaves(&[first.clone(), second.clone()]).unwrap();
        let mut inventory_builder = SourceInventoryIndexBuilderV1::new(inventory_root);
        let mut leaves = vec![first, second.clone()];
        leaves.sort_unstable_by(|left, right| {
            left.path()
                .as_str()
                .as_bytes()
                .cmp(right.path().as_str().as_bytes())
        });
        for leaf in leaves {
            assert!(inventory_builder.push(leaf).unwrap().is_none());
        }
        let inventory_page = inventory_builder.finish().unwrap().unwrap();

        let chunk_digest =
            SourceBlobChunkDigestV1::from_bytes(*ContentDigest::of(&second_bytes).as_bytes());
        let blob_descriptor = SourceBlobChunkDescriptorV1::new(
            second.digest(),
            0,
            1,
            0,
            second_bytes.len() as u32,
            chunk_digest,
        )
        .unwrap();
        let blob_root = SourceBlobChunkRootV1::from_descriptors(
            std::slice::from_ref(&second),
            std::slice::from_ref(&blob_descriptor),
        )
        .unwrap();
        let mut blob_builder = SourceBlobIndexBuilderV1::new(blob_root);
        assert!(blob_builder.push(blob_descriptor).unwrap().is_none());
        let blob_page = blob_builder.finish().unwrap().unwrap();

        let mut inventory_validator =
            SourceInventoryIndexValidatorV1::new(inventory_root, 1).unwrap();
        inventory_validator.push_page(&inventory_page).unwrap();
        inventory_validator.finish().unwrap();
        let mut blob_validator = SourceBlobIndexValidatorV1::new(blob_root, 1).unwrap();
        blob_validator.push_page(&blob_page).unwrap();
        blob_validator.finish().unwrap();

        let profile = BootstrapPartitionProfileV1::v1().digest();
        let import_id = ImportId::from_digest([0x7a; 32]);
        let frontier = ArchiveLocalFrontierBindingV1::initial(import_id, profile);
        let aggregate = BootstrapAggregateManifestV1::new_for_import(
            fixture.workspace,
            LineageDigest::from_bytes([0x7b; 32]),
            crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                b"bootstrap-store-test",
                b"source terminal coverage",
            ),
            import_id,
            2,
            inventory_root,
            1,
            blob_root,
            1,
            profile,
            Vec::new(),
            frontier,
            frontier,
        )
        .unwrap();
        store
            .publish_bootstrap_source_inventory_page(inventory_root, &inventory_page)
            .unwrap();
        store
            .publish_bootstrap_source_blob_page(blob_root, &blob_page)
            .unwrap();
        store
            .publish_bootstrap_source_chunk(chunk_digest, &second_bytes)
            .unwrap();
        store
            .publish_bootstrap_aggregate_prefix(&aggregate)
            .unwrap();
        let error = store.commit_bootstrap_aggregate(&aggregate).unwrap_err();
        assert!(matches!(
            error,
            StoreError::Bootstrap(ref message) if message.contains("BlobContinuityMismatch")
        ));
        assert!(matches!(
            store.inspect_bootstrap_aggregate(&aggregate),
            BootstrapPublicationInspectionV1::Pending
        ));
    }

    #[test]
    fn bootstrap_history_zero_part_install_is_one_atomic_generation_zero_binding() {
        let fixture = EmptyBootstrapFixture::new("zero-history");
        let store = fixture.store();
        store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        let publication_id = store
            .commit_bootstrap_aggregate(&fixture.aggregate)
            .unwrap();
        let publication = store.load_bootstrap_publication(publication_id).unwrap();
        let history = store
            .open_engine_history(fixture.history_binding(0x7400))
            .unwrap();
        let binding =
            BootstrapAggregateHistoryBindingV1::for_aggregate(&fixture.aggregate).unwrap();
        assert_eq!(
            history
                .publish_many_exact(&[], &publication, EngineHistoryBinding::empty())
                .unwrap(),
            (0, EngineHistoryStore::empty_root())
        );
        assert_eq!(history.current_bootstrap_binding().unwrap(), Some(binding));
        drop(history);
        drop(store);

        let reopened = fixture.store();
        let history = reopened
            .open_engine_history(fixture.history_binding(0x7400))
            .unwrap();
        assert_eq!(history.current_bootstrap_binding().unwrap(), Some(binding));
        assert_eq!(
            history.current().unwrap(),
            (0, EngineHistoryStore::empty_root())
        );
    }

    #[test]
    fn bootstrap_history_multipart_install_publishes_every_exact_record_with_one_head_swap() {
        let fixture = BootstrapFixture::new("multipart-history", 2);
        let store = fixture.store();
        let publication_id = fixture.publish_committed(&store);
        let publication = store.load_bootstrap_publication(publication_id).unwrap();
        let history = store
            .open_engine_history(fixture.history_binding(0x7c00))
            .unwrap();
        let binding =
            BootstrapAggregateHistoryBindingV1::for_aggregate(&fixture.aggregate).unwrap();
        let prepared = fixture.prepared_history(binding);
        let (generation, index_root) = history
            .publish_many_exact(&prepared, &publication, EngineHistoryBinding::empty())
            .unwrap();
        assert_eq!(generation, fixture.parts.len() as u64);
        assert_ne!(index_root, EngineHistoryStore::empty_root());
        assert_eq!(history.current_bootstrap_binding().unwrap(), Some(binding));
        for part in &fixture.parts {
            assert_eq!(
                history
                    .lookup(index_root, part.descriptor.batch_id())
                    .unwrap()
                    .as_deref(),
                Some(part.history_record.as_slice())
            );
        }
        drop(history);
        drop(store);

        let reopened = fixture.store();
        let publication = reopened
            .load_bootstrap_publication(binding.publication_id())
            .unwrap();
        assert_eq!(
            publication.aggregate().aggregate_digest(),
            binding.aggregate_digest()
        );
        let history = reopened
            .open_engine_history(fixture.history_binding(0x7c00))
            .unwrap();
        assert_eq!(history.current_bootstrap_binding().unwrap(), Some(binding));
        assert_eq!(history.current().unwrap(), (generation, index_root));
    }

    #[test]
    fn bootstrap_history_multipart_crashes_on_both_sides_of_head_swap_have_atomic_authority() {
        let fixture = BootstrapFixture::new("multipart-head-cuts", 2);
        let store = fixture.store();
        let publication_id = fixture.publish_committed(&store);
        let publication = store.load_bootstrap_publication(publication_id).unwrap();
        let history = store
            .open_engine_history(fixture.history_binding(0x7c10))
            .unwrap();
        let binding =
            BootstrapAggregateHistoryBindingV1::for_aggregate(&fixture.aggregate).unwrap();
        let prepared = fixture.prepared_history(binding);

        fail_next_engine_history_head_swap();
        assert!(history
            .publish_many_exact(&prepared, &publication, EngineHistoryBinding::empty())
            .is_err());
        assert_eq!(history.current_bootstrap_binding().unwrap(), None);
        assert_eq!(
            history.current().unwrap(),
            (0, EngineHistoryStore::empty_root())
        );

        fail_next_engine_history_after_head_swap();
        assert!(history
            .publish_many_exact(&prepared, &publication, EngineHistoryBinding::empty())
            .is_err());
        drop(history);
        drop(store);

        let reopened = fixture.store();
        let publication = reopened
            .load_bootstrap_publication(binding.publication_id())
            .unwrap();
        assert_eq!(publication.aggregate(), &fixture.aggregate);
        let history = reopened
            .open_engine_history(fixture.history_binding(0x7c10))
            .unwrap();
        let (generation, index_root) = history.current().unwrap();
        assert_eq!(generation, 2);
        assert_ne!(index_root, EngineHistoryStore::empty_root());
        assert_eq!(history.current_bootstrap_binding().unwrap(), Some(binding));
        for part in &fixture.parts {
            assert_eq!(
                history
                    .lookup(index_root, part.descriptor.batch_id())
                    .unwrap()
                    .as_deref(),
                Some(part.history_record.as_slice())
            );
        }
        let prepared = fixture.prepared_history(binding);
        assert_eq!(
            history
                .publish_many_exact(&prepared, &publication, EngineHistoryBinding::empty(),)
                .unwrap(),
            (generation, index_root),
            "fresh reopen must adjudicate an after-swap retry as the exact same install"
        );
    }

    #[test]
    fn publish_many_exact_rejects_records_not_reaching_the_bound_aggregate_frontier() {
        let fixture = BootstrapFixture::new("history-frontier-mismatch", 1);
        let other = BootstrapFixture::new("history-frontier-mismatch-other", 1);
        let store = fixture.store();
        let history = store
            .open_engine_history(fixture.history_binding(0x7c20))
            .unwrap();
        let binding = BootstrapAggregateHistoryBindingV1::for_aggregate(&other.aggregate).unwrap();
        let publication = ValidatedBootstrapPublicationV1 {
            aggregate: other.aggregate.clone(),
        };
        let prepared = vec![
            PreparedBootstrapHistoryRecordV1::unchecked_for_history_index_test(
                fixture.parts[0].descriptor,
                &fixture.parts[0].history_record,
                binding,
            ),
        ];
        assert!(matches!(
            history.publish_many_exact(&prepared, &publication, EngineHistoryBinding::empty()),
            Err(StoreError::MalformedHistoryIndex)
        ));
        assert_eq!(
            history.current().unwrap(),
            (0, EngineHistoryStore::empty_root())
        );
    }

    #[test]
    fn publish_many_exact_rejects_duplicate_or_out_of_order_records_without_head_swap() {
        let fixture = BootstrapFixture::new("history-record-order", 2);
        let store = fixture.store();
        let publication_id = fixture.publish_committed(&store);
        let publication = store.load_bootstrap_publication(publication_id).unwrap();
        let history = store
            .open_engine_history(fixture.history_binding(0x7c25))
            .unwrap();
        let binding =
            BootstrapAggregateHistoryBindingV1::for_aggregate(&fixture.aggregate).unwrap();
        let duplicate = vec![
            PreparedBootstrapHistoryRecordV1::unchecked_for_history_index_test(
                fixture.parts[0].descriptor,
                &fixture.parts[0].history_record,
                binding,
            ),
            PreparedBootstrapHistoryRecordV1::unchecked_for_history_index_test(
                fixture.parts[0].descriptor,
                &fixture.parts[0].history_record,
                binding,
            ),
        ];
        assert!(matches!(
            history.publish_many_exact(&duplicate, &publication, EngineHistoryBinding::empty(),),
            Err(StoreError::MalformedHistoryIndex)
        ));
        assert_eq!(
            history.current().unwrap(),
            (0, EngineHistoryStore::empty_root())
        );
    }

    #[test]
    fn bootstrap_history_pre_head_crash_and_stale_second_writer_keep_exact_authority() {
        let fixture = EmptyBootstrapFixture::new("history-crash-stale");
        let first_store = fixture.store();
        let second_store = fixture.store();
        first_store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        let publication_id = first_store
            .commit_bootstrap_aggregate(&fixture.aggregate)
            .unwrap();
        let publication = first_store
            .load_bootstrap_publication(publication_id)
            .unwrap();
        let first = first_store
            .open_engine_history(fixture.history_binding(0x7500))
            .unwrap();
        let second = second_store
            .open_engine_history(fixture.history_binding(0x7500))
            .unwrap();
        let binding =
            BootstrapAggregateHistoryBindingV1::for_aggregate(&fixture.aggregate).unwrap();

        fail_next_engine_history_head_swap();
        assert!(first
            .publish_many_exact(&[], &publication, EngineHistoryBinding::empty())
            .is_err());
        assert_eq!(first.current_bootstrap_binding().unwrap(), None);
        first
            .publish_many_exact(&[], &publication, EngineHistoryBinding::empty())
            .unwrap();
        assert_eq!(first.current_bootstrap_binding().unwrap(), Some(binding));
        assert!(matches!(
            second.publish_many_exact(&[], &publication, EngineHistoryBinding::empty()),
            Ok((0, root)) if root == EngineHistoryStore::empty_root()
        ));
    }

    #[test]
    fn inactive_bootstrap_history_only_open_creates_no_projection_work() {
        let fixture = EmptyBootstrapFixture::new("history-only-open");
        let binding = fixture.history_binding(0x7c30);
        let open = fixture.store().seal_history_only(binding).unwrap();
        assert_eq!(open.binding(), binding);
        let (store, history) = open.into_history().unwrap();
        assert_eq!(
            history.current().unwrap(),
            (0, EngineHistoryStore::empty_root())
        );
        assert!(
            fixture
                .archive
                .join(PROJECTION_WORK_DIR)
                .symlink_metadata()
                .is_err(),
            "history-only open must not create projection-work"
        );
        drop(history);
        drop(store);
    }

    #[test]
    fn inactive_bootstrap_history_only_namespace_substitution_is_rejected_without_adoption() {
        let fixture = EmptyBootstrapFixture::new("history-only-substitution");
        let binding = fixture.history_binding(0x7c40);
        let open = fixture.store().seal_history_only(binding).unwrap();
        let endpoint = fixture
            .archive
            .join(ENGINE_HISTORY_DIR)
            .join(binding.endpoint.endpoint_id.to_string());
        set_enrolled_open_act_hook(move || {
            std::fs::create_dir(&endpoint).unwrap();
            std::fs::write(endpoint.join("foreign-owner"), b"foreign history").unwrap();
        });
        assert!(open.into_history().is_err());
        assert_eq!(
            std::fs::read(
                fixture
                    .archive
                    .join(ENGINE_HISTORY_DIR)
                    .join(binding.endpoint.endpoint_id.to_string())
                    .join("foreign-owner")
            )
            .unwrap(),
            b"foreign history"
        );
        assert!(fixture
            .archive
            .join(PROJECTION_WORK_DIR)
            .symlink_metadata()
            .is_err());
    }

    #[test]
    fn inactive_bootstrap_history_blocks_ordinary_enrolled_open_before_projection_creation() {
        let fixture = EmptyBootstrapFixture::new("ordinary-open-refusal");
        let store = fixture.store();
        store
            .publish_bootstrap_aggregate_prefix(&fixture.aggregate)
            .unwrap();
        let publication_id = store
            .commit_bootstrap_aggregate(&fixture.aggregate)
            .unwrap();
        let publication = store.load_bootstrap_publication(publication_id).unwrap();
        let storage = fixture.history_binding(0x7c50);
        let (_, history) = store
            .seal_history_only(storage)
            .unwrap()
            .into_history()
            .unwrap();
        history
            .publish_many_exact(&[], &publication, EngineHistoryBinding::empty())
            .unwrap();
        drop(history);

        let error = fixture
            .store()
            .seal_enrolled_projection(storage)
            .err()
            .expect("inactive authority must fail enrolled open")
            .1;
        assert!(matches!(error, StoreError::InactiveBootstrapHistory));
        assert!(fixture
            .archive
            .join(PROJECTION_WORK_DIR)
            .symlink_metadata()
            .is_err());
    }

    #[test]
    fn bootstrap_history_refuses_different_publication_and_ordinary_nonempty_authority() {
        let fixture = EmptyBootstrapFixture::new("authority-refusal");
        let storage = fixture.history_binding(0x7c60);
        let store = fixture.store();
        let history = store.open_engine_history(storage).unwrap();
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(0x7c61)),
                b"ordinary history record",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
        let publication = ValidatedBootstrapPublicationV1 {
            aggregate: fixture.aggregate.clone(),
        };
        assert!(matches!(
            history.publish_many_exact(&[], &publication, EngineHistoryBinding::empty()),
            Err(StoreError::BootstrapHistoryRequiresEmptyAuthority)
        ));
        assert_eq!(history.current().unwrap().0, 1);
        drop(history);
        drop(store);

        let first = EmptyBootstrapFixture::new("different-publication-first");
        let store = first.store();
        let history = store.open_engine_history(storage).unwrap();
        let second_aggregate = BootstrapAggregateManifestV1::empty(
            first.workspace,
            first.aggregate.lineage_digest(),
            crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                b"bootstrap-history-test",
                b"different publication",
            ),
            ImportId::from_digest([0x99; 32]),
        )
        .unwrap();
        assert_ne!(
            first.aggregate.publication_id(),
            second_aggregate.publication_id()
        );
        let first_publication = ValidatedBootstrapPublicationV1 {
            aggregate: first.aggregate.clone(),
        };
        let second_publication = ValidatedBootstrapPublicationV1 {
            aggregate: second_aggregate,
        };
        history
            .publish_many_exact(&[], &first_publication, EngineHistoryBinding::empty())
            .unwrap();
        assert!(matches!(
            history.publish_many_exact(&[], &second_publication, EngineHistoryBinding::empty(),),
            Err(StoreError::BootstrapHistoryRequiresEmptyAuthority)
        ));
    }

    #[test]
    #[ignore = "subprocess helper invoked by engine_history_lock_serializes_processes"]
    fn engine_history_lock_subprocess_helper() {
        let Ok(root) = std::env::var("TINE_BOOTSTRAP_LOCK_HELPER_ROOT") else {
            return;
        };
        let ready = std::env::var("TINE_BOOTSTRAP_LOCK_HELPER_READY").unwrap();
        let store = ObjectStore::open(
            Path::new(&root),
            WorkspaceId::from_uuid(Uuid::from_u128(0x7600)),
        )
        .unwrap();
        let history = store
            .open_engine_history(history_storage_binding(0x7d00))
            .unwrap();
        std::fs::write(&ready, b"ready").unwrap();
        history
            .publish(
                BatchId::from_uuid(Uuid::from_u128(0x7d10)),
                b"subprocess cold history record",
                EngineHistoryBinding::empty(),
            )
            .unwrap();
    }

    #[test]
    fn engine_history_lock_serializes_processes() {
        let fixture = BootstrapFixture::new("process-lock", 1);
        let store = fixture.store();
        let history = store
            .open_engine_history(fixture.history_binding(0x7d00))
            .unwrap();
        let guard = AdvisoryTransitionGuard::lock(&history.transition_lock).unwrap();
        let ready = fixture.root.join("child-ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("engine_history_lock_subprocess_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env(
                "TINE_BOOTSTRAP_LOCK_HELPER_ROOT",
                fixture.archive.as_os_str(),
            )
            .env("TINE_BOOTSTRAP_LOCK_HELPER_READY", ready.as_os_str())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "subprocess did not reach the transition");
        thread::sleep(Duration::from_millis(100));
        assert!(
            child.try_wait().unwrap().is_none(),
            "subprocess crossed the held workspace transition lock"
        );
        drop(guard);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "subprocess publication failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        drop(history);
        drop(store);

        let reopened = fixture.store();
        let history = reopened
            .open_engine_history(fixture.history_binding(0x7d00))
            .unwrap();
        let (generation, root) = history.current().unwrap();
        assert_eq!(generation, 1);
        assert_eq!(
            history
                .lookup(root, BatchId::from_uuid(Uuid::from_u128(0x7d10)))
                .unwrap()
                .as_deref(),
            Some(b"subprocess cold history record".as_slice())
        );
    }
}

#[cfg(test)]
mod resume_point_store_tests {
    use super::*;
    use crate::oplog::resume_point::RuntimeResumePointV2;
    use crate::oplog::{
        DeviceId, DocumentId, ImportId, ProjectionEndpointBinding, ProjectionEndpointId,
        ProjectionReceiptStoreId, SessionId,
    };

    /// One promoted, bootstrap-anchored endpoint: the smallest archive shape in
    /// which a resume point is admissible at all.
    ///
    /// A zero-part bootstrap aggregate installs the anchor binding at
    /// generation 0 with the empty index root, which is exactly the durable
    /// history authority a first resume point must name.
    struct PromotedHistoryFixture {
        root: PathBuf,
        archive: PathBuf,
        workspace: WorkspaceId,
        binding: crate::oplog::hot_engine::ProjectionStorageBinding,
        state: PromotedRuntimeStateV1,
    }

    impl PromotedHistoryFixture {
        fn new(label: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("tine-resume-point-{label}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let archive = root.join("archive");
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x8100));
            let lineage = LineageDigest::from_bytes([0x81; 32]);
            let import_id = ImportId::from_digest([0x82; 32]);
            let graph_resource_id =
                crate::oplog::CanonicalGraphResourceId::from_capability_identity(
                    b"resume-point-test",
                    label.as_bytes(),
                );
            let aggregate = BootstrapAggregateManifestV1::empty(
                workspace,
                lineage,
                graph_resource_id,
                import_id,
            )
            .unwrap();
            let binding = crate::oplog::hot_engine::ProjectionStorageBinding {
                endpoint: ProjectionEndpointBinding {
                    endpoint_id: ProjectionEndpointId::from_uuid(Uuid::from_u128(0x8200)),
                    device_id: DeviceId::from_uuid(Uuid::from_u128(0x8201)),
                    graph_resource_id,
                },
                receipt_store_id: ProjectionReceiptStoreId::from_capability_identity(
                    b"resume-point-test",
                    label.as_bytes(),
                ),
            };

            let store = ObjectStore::open(&archive, workspace).unwrap();
            store
                .publish_bootstrap_aggregate_prefix(&aggregate)
                .unwrap();
            let publication_id = store.commit_bootstrap_aggregate(&aggregate).unwrap();
            let publication = store.load_bootstrap_publication(publication_id).unwrap();
            let history = store.open_engine_history(binding).unwrap();
            assert_eq!(
                history
                    .publish_many_exact(&[], &publication, EngineHistoryBinding::empty())
                    .unwrap(),
                (0, EngineHistoryStore::empty_root())
            );
            let archive_resource_id = store.provision_enrolled_archive_resource_id().unwrap();
            let archive_capability = Dir::open_ambient_dir(&archive, ambient_authority()).unwrap();
            let state = PromotedRuntimeStateV1 {
                schema_version: PROMOTED_RUNTIME_STATE_SCHEMA_VERSION,
                lineage_mode: PromotedLineageModeV1::BootstrapAnchoredHomogeneous,
                workspace_id: workspace,
                lineage_digest: lineage,
                catalog_document_id: DocumentId::from_uuid(Uuid::from_u128(0x8300)),
                endpoint_id: binding.endpoint.endpoint_id,
                device_id: binding.endpoint.device_id,
                graph_resource_id,
                receipt_store_id: binding.receipt_store_id,
                archive_resource_id,
                archive_control_binding: control_directory_identity(&archive_capability)
                    .unwrap()
                    .binding_digest(),
                bootstrap: BootstrapAggregateHistoryBindingV1::for_aggregate(&aggregate).unwrap(),
                bootstrap_projection:
                    PromotedBootstrapProjectionBindingV1::synthetic_for_object_store_test(
                        workspace,
                        lineage,
                        binding.endpoint.endpoint_id,
                        binding.endpoint.device_id,
                        graph_resource_id,
                        binding.receipt_store_id,
                        control_directory_identity(&archive_capability)
                            .unwrap()
                            .binding_digest(),
                        ContentDigest::from_bytes(*aggregate.publication_id().as_bytes()),
                        ContentDigest::from_bytes(*aggregate.aggregate_digest().as_bytes()),
                        ContentDigest::from_bytes(*import_id.as_bytes()),
                        aggregate.parts().len() as u32,
                        ContentDigest::of(b"anchor frontier"),
                        0,
                        EngineHistoryStore::empty_root(),
                    ),
                bootstrap_import_id: import_id,
                anchor_history_generation: 0,
                anchor_history_index_root: EngineHistoryStore::empty_root(),
                anchor_acceptance_sequence: 0,
                anchor_accepted_frontier_state_digest: ContentDigest::of(b"anchor frontier"),
                enrollment_verification_digest: ContentDigest::of(b"enrollment verification"),
                enrollment_binding_digest: ContentDigest::of(b"enrollment binding"),
                promotion_session_id: SessionId::from_uuid(Uuid::from_u128(0x8400)),
            };
            history.publish_promoted_runtime_state(&state).unwrap();
            drop(history);
            drop(store);

            Self {
                root,
                archive,
                workspace,
                binding,
                state,
            }
        }

        fn history(&self) -> DurableEngineHistoryStore {
            ObjectStore::open(&self.archive, self.workspace)
                .unwrap()
                .open_engine_history(self.binding)
                .unwrap()
        }

        fn resume_point_path(&self) -> PathBuf {
            self.archive
                .join(ENGINE_HISTORY_DIR)
                .join(self.binding.endpoint.endpoint_id.to_string())
                .join(RESUME_POINT_DIR)
        }

        fn binding(&self, sequence: u64) -> ResumePointEndpointBinding {
            ResumePointEndpointBinding::for_test(
                self.workspace,
                self.binding.endpoint.endpoint_id,
                self.state.state_digest().unwrap(),
                sequence,
            )
        }

        fn enrollment(&self) -> ResumePointEnrollmentBinding {
            ResumePointEnrollmentBinding::unsafe_for_test(
                4,
                ContentDigest::of(b"enrollment head"),
                SessionId::from_uuid(Uuid::from_u128(0x8500)),
            )
        }

        /// The live durable head of this fixture: an unadvanced bootstrap
        /// anchor, so the fixture's points name generation zero.
        fn live_history(&self) -> (u64, ContentDigest, BatchId) {
            (
                0,
                EngineHistoryStore::empty_root(),
                BatchId::from_uuid(Uuid::from_u128(0x8550)),
            )
        }

        fn point(&self, sequence: u64, run: u128) -> RuntimeResumePointV2 {
            RuntimeResumePointV2::empty_rooted_for_test(
                &self.binding(sequence),
                self.enrollment(),
                self.live_history(),
                (Uuid::from_u128(run), ContentDigest::of(b"scratch marker")),
            )
        }

        /// Exact bytes of the resume-point directory, so a refusal can be shown
        /// to have changed nothing at all.
        fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
            let directory = self.resume_point_path();
            if !directory.is_dir() {
                return BTreeMap::new();
            }
            std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        std::fs::read(entry.path()).unwrap(),
                    )
                })
                .collect()
        }
    }

    impl Drop for PromotedHistoryFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_published_resume_point_reads_back_under_its_promoted_state_binding() {
        let fixture = PromotedHistoryFixture::new("publish-read");
        let history = fixture.history();
        assert!(history.read_resume_point_set().unwrap().points().is_empty());

        let point = fixture.point(1, 0x8601);
        history.publish_resume_point(&point).unwrap();

        let set = history.read_resume_point_set().unwrap();
        assert_eq!(set.points(), &[point.clone()]);
        assert_eq!(set.next_sequence().unwrap(), 2);
        assert!(set.reachable_runs().contains(Uuid::from_u128(0x8601)));

        // A fresh process observes the identical durable evidence.
        drop(history);
        assert_eq!(
            fixture.history().read_resume_point_set().unwrap().points(),
            &[point]
        );
    }

    #[test]
    fn republishing_identical_bytes_at_the_same_sequence_resumes() {
        let fixture = PromotedHistoryFixture::new("idempotent");
        let history = fixture.history();
        let point = fixture.point(1, 0x8601);
        history.publish_resume_point(&point).unwrap();
        let published = fixture.snapshot();

        history.publish_resume_point(&point).unwrap();
        assert_eq!(fixture.snapshot(), published);
    }

    #[test]
    fn divergent_bytes_at_the_same_sequence_fail_closed() {
        let fixture = PromotedHistoryFixture::new("divergent");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        let published = fixture.snapshot();

        assert!(matches!(
            history.publish_resume_point(&fixture.point(1, 0x8602)),
            Err(StoreError::ImmutableCollision("runtime resume point"))
        ));
        assert_eq!(fixture.snapshot(), published);
    }

    #[test]
    fn publication_prunes_only_lower_sequences_after_the_commit_point() {
        let fixture = PromotedHistoryFixture::new("supersede");
        let history = fixture.history();
        let first = fixture.point(1, 0x8601);
        let second = fixture.point(2, 0x8601);
        history.publish_resume_point(&first).unwrap();
        history.publish_resume_point(&second).unwrap();

        assert_eq!(
            history.read_resume_point_set().unwrap().points(),
            &[second.clone()]
        );
        assert_eq!(
            fixture.snapshot().keys().cloned().collect::<Vec<_>>(),
            vec![second.file_name()]
        );
    }

    /// Cut the process exactly between the commit point and the prune, the way
    /// a power loss does.
    fn cut_before_prune(fixture: &PromotedHistoryFixture, point: &RuntimeResumePointV2) {
        let directory =
            Dir::open_ambient_dir(fixture.resume_point_path(), ambient_authority()).unwrap();
        publish_immutable_exact(
            &directory,
            &point.file_name(),
            &point.encode().unwrap(),
            "runtime resume point",
        )
        .unwrap();
    }

    #[test]
    fn a_crash_between_publish_and_prune_leaves_two_points_and_a_retry_converges() {
        let fixture = PromotedHistoryFixture::new("crash-before-prune");
        let history = fixture.history();
        let first = fixture.point(1, 0x8601);
        let second = fixture.point(2, 0x8601);
        history.publish_resume_point(&first).unwrap();

        // Exactly the durable cut between B4 (the successor is durable) and B5
        // (the predecessor is removed): the successor is published through the
        // same immutable-exact primitive, and the process dies before pruning.
        cut_before_prune(&fixture, &second);

        // The cut is readable, bounded, and still names the retained run.
        let cut = history.read_resume_point_set().unwrap();
        assert_eq!(cut.points(), &[first, second.clone()]);
        assert_eq!(cut.latest().unwrap(), &second);
        assert_eq!(cut.reachable_runs().len(), 1);

        // Retrying the interrupted publication converges without republishing
        // divergent bytes. This is the route the *same* session can take; the
        // takeover route is the next test, and it is the one that matters.
        history.publish_resume_point(&second).unwrap();
        assert_eq!(history.read_resume_point_set().unwrap().points(), &[second]);
    }

    /// The causal B1 regression: the crash-takeover restart.
    ///
    /// After a crash the endpoint is reopened by a *different* session at a
    /// *later* enrollment generation, so its resume point can never be a
    /// byte-identical retry of the interrupted one — `publish_immutable_exact`
    /// refuses it as an `ImmutableCollision`, and the store additionally
    /// refuses any point that does not name the live durable history. The only
    /// available route is the next fresh sequence, and it must converge the cut
    /// instead of committing a third point that nothing can ever remove.
    #[test]
    fn a_takeover_session_converges_the_two_point_cut() {
        let fixture = PromotedHistoryFixture::new("takeover-convergence");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        let interrupted = fixture.point(2, 0x8601);
        cut_before_prune(&fixture, &interrupted);

        let takeover = |sequence: u64| {
            fixture.point(sequence, 0x8601).with_enrollment_for_test(
                ResumePointEnrollmentBinding::unsafe_for_test(
                    5,
                    ContentDigest::of(b"takeover enrollment head"),
                    SessionId::from_uuid(Uuid::from_u128(0x8501)),
                ),
            )
        };
        // The byte-identical retry is genuinely unavailable to this session.
        assert!(matches!(
            history.publish_resume_point(&takeover(2)),
            Err(StoreError::ImmutableCollision("runtime resume point"))
        ));

        let third = takeover(3);
        history.publish_resume_point(&third).unwrap();
        assert_eq!(
            fixture.snapshot().keys().cloned().collect::<Vec<_>>(),
            vec![third.file_name()]
        );
        assert_eq!(
            history.read_resume_point_set().unwrap().points(),
            &[third.clone()]
        );
        // Every downstream capability is still available afterwards.
        assert_eq!(
            history
                .read_resume_point_set()
                .unwrap()
                .next_sequence()
                .unwrap(),
            4
        );
        history.publish_resume_point(&takeover(4)).unwrap();
        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 1);
        assert!(history.read_resume_point_set().unwrap().points().is_empty());
    }

    /// Repeated crashes and restarts stay convergent and bounded.
    ///
    /// Each round crashes between the commit point and the prune, then restarts
    /// as a fresh takeover session. The durable set is never empty and never
    /// exceeds the publication bound at any observed cut.
    #[test]
    fn repeated_crash_takeover_rounds_stay_bounded_and_convergent() {
        let fixture = PromotedHistoryFixture::new("repeated-crash");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();

        let mut sequence = 1_u64;
        for round in 0..6_u64 {
            // Crash cut: the successor is durable, the prune never ran.
            sequence += 1;
            let interrupted = fixture.point(sequence, 0x8601).with_enrollment_for_test(
                ResumePointEnrollmentBinding::unsafe_for_test(
                    10 + round,
                    ContentDigest::of(b"interrupted enrollment head"),
                    SessionId::from_uuid(Uuid::from_u128(0x8600 + u128::from(round))),
                ),
            );
            cut_before_prune(&fixture, &interrupted);
            assert_eq!(
                fixture.snapshot().len(),
                2,
                "round {round}: the crash cut must hold exactly the two-point overlap"
            );

            // Restart as a takeover session: new session, later generation.
            sequence += 1;
            let restarted = fixture.point(sequence, 0x8601).with_enrollment_for_test(
                ResumePointEnrollmentBinding::unsafe_for_test(
                    100 + round,
                    ContentDigest::of(b"restarted enrollment head"),
                    SessionId::from_uuid(Uuid::from_u128(0x8700 + u128::from(round))),
                ),
            );
            history.publish_resume_point(&restarted).unwrap();

            let set = history.read_resume_point_set().unwrap();
            assert_eq!(set.points(), &[restarted], "round {round} did not converge");
            assert_eq!(set.reachable_runs().len(), 1);
        }
        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 1);
    }

    /// A fault at **every** durable boundary of one publication leaves at least
    /// one fully valid point, keeps the retained run provably reachable, and
    /// lets the restart converge.
    ///
    /// The starting shape is the valid two-point crash cut `{n, n+1}` on
    /// purpose: it is the widest set a publication may leave, and the only
    /// shape in which step 1's pre-prune actually deletes anything. That
    /// pre-prune is this packet's sole deletion *before* a commit point, and
    /// its whole safety argument rests on its watermark being the durable
    /// `latest` rather than the successor being published. Nothing else in the
    /// suite can observe that: at a retry the two watermarks are numerically
    /// identical, and the mid-call cut is unreachable from any black-box call
    /// sequence. Hence the named boundaries.
    #[test]
    fn a_fault_at_every_publication_boundary_leaves_a_valid_durable_point() {
        for boundary in ResumePublishBoundary::ALL {
            let fixture = PromotedHistoryFixture::new("publication-boundary");
            let history = fixture.history();
            let run = Uuid::from_u128(0x8601);
            history
                .publish_resume_point(&fixture.point(1, 0x8601))
                .unwrap();
            cut_before_prune(&fixture, &fixture.point(2, 0x8601));
            assert_eq!(
                fixture.snapshot().len(),
                2,
                "boundary {boundary:?}: the starting cut must hold both points"
            );

            // The restart is a takeover — different session, later enrollment
            // generation — so no byte-identical retry is available to it and
            // the publication really has to run the pre-prune.
            let takeover = |sequence: u64| {
                fixture.point(sequence, 0x8601).with_enrollment_for_test(
                    ResumePointEnrollmentBinding::unsafe_for_test(
                        5,
                        ContentDigest::of(b"takeover enrollment head"),
                        SessionId::from_uuid(Uuid::from_u128(0x8501)),
                    ),
                )
            };

            fail_next_resume_publication_at(boundary);
            let error = history.publish_resume_point(&takeover(3)).unwrap_err();
            assert!(
                error.to_string().contains(&format!("{boundary:?}")),
                "boundary {boundary:?} produced an unrelated error: {error}"
            );

            // Independently of the store: every surviving file decodes as a
            // sealed point bound to its own name, and at least one exists.
            let cut = fixture.snapshot();
            assert!(
                !cut.is_empty(),
                "boundary {boundary:?}: the cut has zero durable files"
            );
            let durable: Vec<RuntimeResumePointV2> = cut
                .iter()
                .map(|(name, bytes)| {
                    let point = RuntimeResumePointV2::decode(bytes).unwrap_or_else(|error| {
                        panic!("boundary {boundary:?}: {name} is not a valid point: {error}")
                    });
                    assert_eq!(
                        &point.file_name(),
                        name,
                        "boundary {boundary:?}: {name} is not bound to its payload"
                    );
                    point
                })
                .collect();

            // Authority remains valid: the strict proof still mints, so the
            // retained run is still provably reachable and unreclaimable.
            let set = history.read_resume_point_set().unwrap();
            assert_eq!(
                set.points(),
                durable.as_slice(),
                "boundary {boundary:?}: the store and the raw bytes disagree"
            );
            assert!(
                set.reachable_runs().contains(run),
                "boundary {boundary:?} lost the retained run's reachability"
            );

            // Restart/retry converges to exactly one point, and the drain is
            // still available afterwards.
            let resumed = takeover(set.next_sequence().unwrap());
            history.publish_resume_point(&resumed).unwrap();
            assert_eq!(
                fixture.snapshot().keys().cloned().collect::<Vec<_>>(),
                vec![resumed.file_name()],
                "boundary {boundary:?} did not converge on restart"
            );
            let converged = history.read_resume_point_set().unwrap();
            assert_eq!(converged.points(), &[resumed]);
            assert!(converged.reachable_runs().contains(run));
            assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 1);
            assert!(history.read_resume_point_set().unwrap().points().is_empty());
        }
    }

    /// A directory an older build already bricked — three recognized canonical
    /// points — must converge rather than stay unreadable forever.
    #[test]
    fn a_pre_existing_point_surplus_converges_on_the_next_publication() {
        let fixture = PromotedHistoryFixture::new("legacy-surplus");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        for sequence in [2_u64, 3] {
            cut_before_prune(&fixture, &fixture.point(sequence, 0x8601));
        }
        assert_eq!(fixture.snapshot().len(), 3);
        // The strict proof correctly refuses a surplus: nothing may be
        // reclaimed on the strength of a set that a prune never finished.
        assert!(matches!(
            history.read_resume_point_set(),
            Err(StoreError::ResumePoint(_))
        ));

        let fourth = fixture.point(4, 0x8601);
        history.publish_resume_point(&fourth).unwrap();
        assert_eq!(
            fixture.snapshot().keys().cloned().collect::<Vec<_>>(),
            vec![fourth.file_name()]
        );
        assert_eq!(history.read_resume_point_set().unwrap().points(), &[fourth]);
    }

    /// The same surplus is drainable without publishing anything at all.
    #[test]
    fn a_pre_existing_point_surplus_is_clearable() {
        let fixture = PromotedHistoryFixture::new("legacy-surplus-clear");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        for sequence in [2_u64, 3] {
            cut_before_prune(&fixture, &fixture.point(sequence, 0x8601));
        }
        assert!(history.read_resume_point_set().is_err());
        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 3);
        assert!(history.read_resume_point_set().unwrap().points().is_empty());
    }

    #[test]
    fn a_sequence_that_does_not_extend_the_published_set_is_refused() {
        let fixture = PromotedHistoryFixture::new("sequence-regression");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        history
            .publish_resume_point(&fixture.point(2, 0x8601))
            .unwrap();
        let published = fixture.snapshot();

        for sequence in [1_u64, 4] {
            assert!(
                matches!(
                    history.publish_resume_point(&fixture.point(sequence, 0x8601)),
                    Err(StoreError::ResumePointSequenceRegression {
                        expected: 3,
                        found,
                    }) if found == sequence
                ),
                "sequence {sequence} was not refused"
            );
        }
        assert_eq!(fixture.snapshot(), published);
    }

    #[test]
    fn a_resume_point_bound_to_another_endpoint_or_workspace_is_refused() {
        let fixture = PromotedHistoryFixture::new("foreign-binding");
        let history = fixture.history();

        let foreign_workspace = fixture
            .point(1, 0x8601)
            .with_workspace_id_for_test(WorkspaceId::from_uuid(Uuid::from_u128(0x8fff)));
        assert!(matches!(
            history.publish_resume_point(&foreign_workspace),
            Err(StoreError::ResumePointBindingMismatch(_))
        ));

        let foreign_state = fixture
            .point(1, 0x8601)
            .with_promoted_state_digest_for_test(ContentDigest::of(b"another endpoint"));
        assert!(matches!(
            history.publish_resume_point(&foreign_state),
            Err(StoreError::ResumePointBindingMismatch(_))
        ));

        assert!(fixture.snapshot().is_empty());
    }

    #[test]
    fn a_resume_point_that_does_not_name_the_live_durable_history_is_refused() {
        let fixture = PromotedHistoryFixture::new("history-binding");
        let history = fixture.history();

        let (_, live_root, live_batch) = fixture.live_history();
        let ahead = fixture
            .point(1, 0x8601)
            .with_history_for_test(1, live_root, live_batch);
        assert!(matches!(
            history.publish_resume_point(&ahead),
            Err(StoreError::ResumePointBindingMismatch(_))
        ));

        let wrong_root = fixture.point(1, 0x8601).with_history_for_test(
            0,
            ContentDigest::of(b"another index root"),
            live_batch,
        );
        assert!(matches!(
            history.publish_resume_point(&wrong_root),
            Err(StoreError::ResumePointBindingMismatch(_))
        ));

        assert!(fixture.snapshot().is_empty());
    }

    #[test]
    fn a_resume_point_without_a_promoted_runtime_state_is_residue() {
        let fixture = PromotedHistoryFixture::new("no-promotion");
        let history = fixture.history();
        let point = fixture.point(1, 0x8601);
        history.publish_resume_point(&point).unwrap();
        let published = fixture.snapshot();
        drop(history);

        // Removing the promoted state models the accidental loss of the only
        // authority that could have authorized this point.
        std::fs::remove_file(
            fixture
                .archive
                .join(ENGINE_HISTORY_DIR)
                .join(fixture.binding.endpoint.endpoint_id.to_string())
                .join(PROMOTED_RUNTIME_STATE_FILE),
        )
        .unwrap();

        assert!(matches!(
            fixture.history().read_resume_point_set(),
            Err(StoreError::ResumePointBindingMismatch(_))
        ));
        assert_eq!(fixture.snapshot(), published);
    }

    #[test]
    fn a_malformed_point_poisons_the_read_and_publishes_nothing() {
        let fixture = PromotedHistoryFixture::new("poison");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        std::fs::write(fixture.resume_point_path().join("stray-entry"), b"residue").unwrap();
        let poisoned = fixture.snapshot();

        // Adoption refuses, and publication fails closed to a full replay: a
        // directory that is not fully understood must not gain new authority,
        // and a conflict copy must never be mistaken for the canonical latest.
        assert!(matches!(
            history.read_resume_point_set(),
            Err(StoreError::ResumePoint(_))
        ));
        assert!(matches!(
            history.publish_resume_point(&fixture.point(2, 0x8601)),
            Err(StoreError::ResumePoint(_))
        ));
        // Neither refusal changed a single byte.
        assert_eq!(fixture.snapshot(), poisoned);
    }

    /// The causal B3 regression, at the store boundary.
    ///
    /// Every one of these is an ordinary accident of a filesystem sync provider
    /// or a desktop shell. None of them may be deleted as if it were
    /// authoritative, none of them may mint a reachability proof, and none of
    /// them may make the `Unsafe -> Safe` drain permanently impossible.
    #[test]
    fn provider_residue_never_blocks_the_safe_drain_or_mints_authority() {
        let canonical_name = "00000000000000000001.resume-point";
        for (label, stranger, bytes) in [
            ("desktop", ".DS_Store", b"\x00\x01Bud1 residue".to_vec()),
            (
                "backup",
                "00000000000000000001.resume-point.bak",
                Vec::new(),
            ),
            (
                "syncthing",
                "00000000000000000001.sync-conflict-20260728-120000-ABCDEFG.resume-point",
                Vec::new(),
            ),
            (
                "dropbox",
                "00000000000000000001 (1).resume-point",
                Vec::new(),
            ),
            ("torn", "00000000000000000002.resume-point", Vec::new()),
            ("unknown", "stray-entry", b"residue".to_vec()),
        ] {
            let fixture = PromotedHistoryFixture::new(&format!("drain-{label}"));
            let history = fixture.history();
            let point = fixture.point(1, 0x8601);
            history.publish_resume_point(&point).unwrap();
            assert_eq!(point.file_name(), canonical_name);

            // A copy of the real point under a residue name for the provider
            // shapes; a truncated one for `torn`; opaque bytes otherwise.
            let published = point.encode().unwrap();
            let residue_bytes = match label {
                "torn" => published[..published.len() - 3].to_vec(),
                _ if bytes.is_empty() => published.clone(),
                _ => bytes,
            };
            let residue_path = fixture.resume_point_path().join(stranger);
            std::fs::write(&residue_path, &residue_bytes).unwrap();

            // No proof, therefore no deletion authority over any retained run.
            assert!(
                history.read_resume_point_set().is_err(),
                "{label}: residue must not mint a reachability proof"
            );

            // The drain still progresses, removing only what it recognized.
            let maintenance = history.clear_resume_points_for_test().unwrap();
            assert_eq!(maintenance.removed, 1, "{label}");
            assert_eq!(maintenance.preserved, vec![stranger.to_owned()], "{label}");
            assert!(
                !fixture.resume_point_path().join(canonical_name).exists(),
                "{label}: the canonical point was not cleared"
            );
            assert_eq!(
                std::fs::read(&residue_path).unwrap(),
                residue_bytes,
                "{label}: residue was not preserved byte-for-byte"
            );
            // And it is still poison afterwards, so reclamation stays refused.
            assert!(
                history.read_resume_point_set().is_err(),
                "{label}: residue must still deny the proof after the drain"
            );
        }
    }

    /// The end-to-end consequence of B3's scoping: under poison the drain
    /// completes, the retained run is *not* reclaimed, and removing the residue
    /// is what restores deletion authority.
    #[test]
    fn a_retained_run_is_never_reclaimed_while_residue_denies_the_proof() {
        use crate::oplog::scratch_store::{
            reclaim_unreachable_retained_runs, ScratchStore, SCRATCH_DIR,
        };

        let fixture = PromotedHistoryFixture::new("residue-retains-run");
        let archive_capability =
            Dir::open_ambient_dir(&fixture.archive, ambient_authority()).unwrap();
        let retained =
            ScratchStore::create_retained(&archive_capability, fixture.workspace).unwrap();
        let run_id = retained.run_id();
        drop(retained);
        let run_path = fixture
            .archive
            .join(SCRATCH_DIR)
            .join(format!("run-{run_id}"));
        assert!(run_path.is_dir());

        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, run_id.as_u128()))
            .unwrap();
        let residue_path = fixture.resume_point_path().join(".DS_Store");
        std::fs::write(&residue_path, b"\x00\x01Bud1 residue").unwrap();

        // The drain progresses so the handoff can reach `Safe` ...
        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 1);
        // ... but the residue still denies the proof, so the composition the
        // lifecycle caller must use cannot even produce an argument for
        // reclamation, and the run's bytes survive.
        assert!(history
            .read_resume_point_set()
            .map(|set| set.reachable_runs())
            .is_err());
        assert!(run_path.is_dir());

        // Removing the residue is what restores deletion authority.
        std::fs::remove_file(&residue_path).unwrap();
        let reachable = history.read_resume_point_set().unwrap().reachable_runs();
        assert_eq!(reachable.len(), 0);
        let outcome =
            reclaim_unreachable_retained_runs(&archive_capability, fixture.workspace, &reachable)
                .unwrap();
        assert_eq!(outcome.retained_reclaimed, 1);
        assert!(!run_path.exists());
    }

    /// Mint one retained run, release its lease, and return its identity.
    fn retained_run(fixture: &PromotedHistoryFixture) -> (Uuid, PathBuf) {
        use crate::oplog::scratch_store::{ScratchStore, SCRATCH_DIR};

        let archive_capability =
            Dir::open_ambient_dir(&fixture.archive, ambient_authority()).unwrap();
        let retained =
            ScratchStore::create_retained(&archive_capability, fixture.workspace).unwrap();
        let run_id = retained.run_id();
        drop(retained);
        let path = fixture
            .archive
            .join(SCRATCH_DIR)
            .join(format!("run-{run_id}"));
        assert!(path.is_dir());
        (run_id, path)
    }

    fn adoption_candidate(
        fixture: &PromotedHistoryFixture,
        history: &DurableEngineHistoryStore,
    ) -> ResumeAdoptionCandidate {
        history.read_resume_adoption_candidate(ResumeEnrollmentAdmission::SameSession(
            fixture.enrollment(),
        ))
    }

    /// The strict latest-point read hands the resuming open exactly the
    /// snapshot the emitting engine produced.
    ///
    /// Sealed, published, re-read from durable bytes in a fresh store, re-proved
    /// against the live open's authority, and converted back — the round trip is
    /// an equality, not an approximation, because every member of the snapshot
    /// has a field of the record and `seal` filled every one of them.
    #[test]
    fn the_latest_point_reads_back_as_the_exact_snapshot_it_was_minted_from() {
        let fixture = PromotedHistoryFixture::new("adoption-candidate");
        let history = fixture.history();
        let (run_id, _) = retained_run(&fixture);
        let snapshot = RuntimeResumeSnapshot::empty_rooted_for_test(
            fixture.live_history(),
            (run_id, ContentDigest::of(b"scratch marker")),
        );

        assert!(matches!(
            adoption_candidate(&fixture, &history),
            ResumeAdoptionCandidate::Unavailable(ResumeAcceleratorUnavailable::NeverPublished)
        ));

        let point = history
            .mint_resume_point(&snapshot, fixture.enrollment())
            .unwrap();
        assert_eq!(point.resume_sequence(), 1);
        let published = history.publish_resume_point(&point).unwrap();
        assert_eq!(published.resume_sequence(), 1);
        assert_eq!(published.scratch_run_id(), run_id);

        // A fresh process reads the identical durable evidence.
        drop(history);
        let history = fixture.history();
        let ResumeAdoptionCandidate::Available(adopted) = adoption_candidate(&fixture, &history)
        else {
            panic!("a freshly published point must be adoptable");
        };
        assert_eq!(*adopted, snapshot);

        // The successor sequence is derived from the survey, never asserted.
        assert_eq!(
            history
                .mint_resume_point(&snapshot, fixture.enrollment())
                .unwrap()
                .resume_sequence(),
            2
        );
    }

    /// A torn candidate costs one full replay and not one byte.
    ///
    /// The three shapes are the ones this fault model actually produces: a
    /// truncated point, a provider conflict copy beside a valid point, and a
    /// point whose binding no longer matches the live enrollment record. All
    /// three are `Unavailable`, none is an `Err`, and the directory is
    /// byte-identical before and after — a refusal must never be a repair.
    #[test]
    fn a_torn_or_unbound_candidate_falls_back_without_changing_a_byte() {
        let fixture = PromotedHistoryFixture::new("candidate-fallback");
        let history = fixture.history();
        let (run_id, _) = retained_run(&fixture);
        let snapshot = RuntimeResumeSnapshot::empty_rooted_for_test(
            fixture.live_history(),
            (run_id, ContentDigest::of(b"scratch marker")),
        );
        let point = history
            .mint_resume_point(&snapshot, fixture.enrollment())
            .unwrap();
        history.publish_resume_point(&point).unwrap();
        let intact = fixture.snapshot();

        // 1. Torn bytes.
        let path = fixture.resume_point_path().join(point.file_name());
        let whole = std::fs::read(&path).unwrap();
        std::fs::write(&path, &whole[..whole.len() - 3]).unwrap();
        let torn = fixture.snapshot();
        assert!(matches!(
            adoption_candidate(&fixture, &history),
            ResumeAdoptionCandidate::Unavailable(ResumeAcceleratorUnavailable::ProofDenied(
                ResumePointError::Malformed(_)
            ))
        ));
        assert_eq!(fixture.snapshot(), torn, "a refusal must repair nothing");
        std::fs::write(&path, &whole).unwrap();
        assert_eq!(fixture.snapshot(), intact);

        // 2. A provider conflict copy carrying genuinely valid point bytes.
        // Unrecognized residue must never be promoted to authority, and it must
        // not be silently ignored either: it denies the whole proof.
        let conflict = fixture
            .resume_point_path()
            .join("00000000000000000001.sync-conflict-20260728-120000-ABCDEFG.resume-point");
        std::fs::write(&conflict, &whole).unwrap();
        let with_residue = fixture.snapshot();
        assert!(matches!(
            adoption_candidate(&fixture, &history),
            ResumeAdoptionCandidate::Unavailable(ResumeAcceleratorUnavailable::ProofDenied(_))
        ));
        assert_eq!(fixture.snapshot(), with_residue);
        std::fs::remove_file(&conflict).unwrap();
        assert_eq!(fixture.snapshot(), intact);

        // 3. Enrollment evidence the live record contradicts.
        let stranger = ResumePointEnrollmentBinding::unsafe_for_test(
            99,
            ContentDigest::of(b"a different enrollment head"),
            SessionId::from_uuid(Uuid::from_u128(0x8fff)),
        );
        assert!(matches!(
            history
                .read_resume_adoption_candidate(ResumeEnrollmentAdmission::SameSession(stranger)),
            ResumeAdoptionCandidate::Unavailable(ResumeAcceleratorUnavailable::BindingRefused(_))
        ));
        assert_eq!(fixture.snapshot(), intact);
        // And it is still adoptable for the session that actually published it.
        assert!(matches!(
            adoption_candidate(&fixture, &history),
            ResumeAdoptionCandidate::Available(_)
        ));
    }

    /// The ordering the whole maintenance design rests on.
    ///
    /// A retained run may be collected only once a *replacement* point naming
    /// its successor is durable — until then the predecessor's run may hold the
    /// only resumable bytes. The `PublishedResumePoint` witness carries that
    /// ordering in the type, and this proves the behaviour on both sides of it.
    #[test]
    fn a_predecessor_run_is_reclaimed_only_after_its_replacement_is_durable() {
        let fixture = PromotedHistoryFixture::new("reclaim-after-replacement");
        let history = fixture.history();
        let (predecessor, predecessor_path) = retained_run(&fixture);

        let first = history
            .publish_resume_point(
                &history
                    .mint_resume_point(
                        &RuntimeResumeSnapshot::empty_rooted_for_test(
                            fixture.live_history(),
                            (predecessor, ContentDigest::of(b"scratch marker")),
                        ),
                        fixture.enrollment(),
                    )
                    .unwrap(),
            )
            .unwrap();

        // Before the replacement exists, the predecessor is *reachable*: the
        // pass runs, proves it, and deletes nothing.
        let held = history.reclaim_retained_runs_after_publication(&first);
        assert_eq!(held.outcome, RetainedRunMaintenanceOutcome::Reclaimed);
        assert_eq!(held.reclaimed, 0);
        assert_eq!(held.retained_runs_remaining, 1);
        assert!(held.within_retained_run_bound);
        assert!(predecessor_path.is_dir());

        // The replacement publication prunes the predecessor's point, which is
        // what makes its run unreachable.
        let (successor, successor_path) = retained_run(&fixture);
        let second = history
            .publish_resume_point(
                &history
                    .mint_resume_point(
                        &RuntimeResumeSnapshot::empty_rooted_for_test(
                            fixture.live_history(),
                            (successor, ContentDigest::of(b"scratch marker")),
                        ),
                        fixture.enrollment(),
                    )
                    .unwrap(),
            )
            .unwrap();
        let collected = history.reclaim_retained_runs_after_publication(&second);
        assert_eq!(collected.outcome, RetainedRunMaintenanceOutcome::Reclaimed);
        assert_eq!(collected.reclaimed, 1);
        assert_eq!(collected.retained_runs_remaining, 1);
        assert!(collected.within_retained_run_bound);
        assert!(collected.preserved_resume_residue.is_empty());
        assert!(!predecessor_path.exists());
        assert!(successor_path.is_dir(), "the reachable run must survive");

        // A witness whose point has left the complete set proves nothing about
        // the state the caller published, so the pass preserves everything.
        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 1);
        let stale = history.reclaim_retained_runs_after_publication(&second);
        assert!(matches!(
            stale.outcome,
            RetainedRunMaintenanceOutcome::ProofDenied(_)
        ));
        assert_eq!(stale.reclaimed, 0);
        assert!(successor_path.is_dir());
    }

    /// Residue denies deletion, and at the retained-run bound it must also stop
    /// authorizing *growth*.
    ///
    /// This is the leak bound. One permanent conflict copy in the resume-point
    /// directory denies the strict proof forever, so without a pre-mint decision
    /// every restart would mint one more retained run that nothing can ever
    /// collect. Choosing ephemeral costs exactly one full replay.
    #[test]
    fn residue_denies_deletion_and_at_the_bound_chooses_ephemeral() {
        let fixture = PromotedHistoryFixture::new("bounded-minting");
        let history = fixture.history();
        let (first_run, first_path) = retained_run(&fixture);
        let published = history
            .publish_resume_point(
                &history
                    .mint_resume_point(
                        &RuntimeResumeSnapshot::empty_rooted_for_test(
                            fixture.live_history(),
                            (first_run, ContentDigest::of(b"scratch marker")),
                        ),
                        fixture.enrollment(),
                    )
                    .unwrap(),
            )
            .unwrap();

        // A provable directory authorizes a retained run at any census, because
        // an unreachable one can always be collected later.
        assert_eq!(
            history.plan_engine_scratch_retention(),
            EngineScratchRetentionPlan::Retained { retained_runs: 1 }
        );

        let residue = fixture.resume_point_path().join(".DS_Store");
        std::fs::write(&residue, b"\x00\x01Bud1 desktop residue").unwrap();

        // Unprovable but still below the bound: one more accelerator is an
        // acceptable trade.
        assert_eq!(
            history.plan_engine_scratch_retention(),
            EngineScratchRetentionPlan::Retained { retained_runs: 1 },
            "below the bound an unprovable directory still allows one more run"
        );

        // At the bound it does not.
        let (_, second_path) = retained_run(&fixture);
        let EngineScratchRetentionPlan::Ephemeral {
            retained_runs,
            reason,
        } = history.plan_engine_scratch_retention()
        else {
            panic!("an unprovable directory at the retained-run bound must choose ephemeral");
        };
        assert_eq!(retained_runs, MAX_RETAINED_SCRATCH_RUNS);
        assert!(matches!(reason, ResumePointError::UnexpectedEntry(_)));

        // Nor does residue authorize deletion. The witness is real — it was
        // minted by a successful publication — and the pass still preserves
        // every run, the residue, and the recognized point.
        let report = history.reclaim_retained_runs_after_publication(&published);
        assert!(matches!(
            report.outcome,
            RetainedRunMaintenanceOutcome::ProofDenied(_)
        ));
        assert_eq!(report.reclaimed, 0);
        assert_eq!(report.retained_runs_remaining, MAX_RETAINED_SCRATCH_RUNS);
        assert_eq!(
            report.preserved_resume_residue,
            vec![".DS_Store".to_owned()]
        );
        assert!(first_path.is_dir());
        assert!(second_path.is_dir());
        assert!(fixture
            .resume_point_path()
            .join("00000000000000000001.resume-point")
            .exists());
        assert_eq!(
            std::fs::read(&residue).unwrap(),
            b"\x00\x01Bud1 desktop residue"
        );

        // Removing the residue is what restores both authorities.
        std::fs::remove_file(&residue).unwrap();
        assert!(matches!(
            history.plan_engine_scratch_retention(),
            EngineScratchRetentionPlan::Retained { .. }
        ));
    }

    #[test]
    fn clearing_removes_every_unsafe_point_and_is_idempotent() {
        let fixture = PromotedHistoryFixture::new("clear");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();

        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 1);
        assert!(history.read_resume_point_set().unwrap().points().is_empty());
        assert_eq!(
            history
                .read_resume_point_set()
                .unwrap()
                .reachable_runs()
                .len(),
            0
        );
        assert_eq!(history.clear_resume_points_for_test().unwrap().removed, 0);

        // Clearing is not a terminal state. The publication sequence is derived
        // from the durable set rather than from a separate counter file, so a
        // cleared endpoint legitimately restarts at one; there is nothing on
        // disk left for it to be ambiguous against. A resurrected stale copy of
        // the old sequence-one file is still fenced, because immutable-exact
        // publication refuses divergent bytes under an existing name.
        let next = fixture.point(1, 0x8602);
        history.publish_resume_point(&next).unwrap();
        assert_eq!(
            history.read_resume_point_set().unwrap().points(),
            &[next.clone()]
        );
        assert!(matches!(
            history.publish_resume_point(&fixture.point(1, 0x8601)),
            Err(StoreError::ImmutableCollision("runtime resume point"))
        ));
        assert_eq!(history.read_resume_point_set().unwrap().points(), &[next]);
    }

    #[test]
    fn a_never_published_endpoint_has_an_empty_set_and_clears_nothing() {
        let fixture = PromotedHistoryFixture::new("never-published");
        let history = fixture.history();
        assert!(history.read_resume_point_set().unwrap().points().is_empty());
        assert_eq!(
            history.clear_resume_points_for_test().unwrap(),
            ResumePointMaintenance::default()
        );
        assert!(!fixture.resume_point_path().exists());
    }

    #[test]
    fn a_copied_archive_cannot_read_the_original_endpoint_resume_points() {
        let fixture = PromotedHistoryFixture::new("copied-archive");
        let history = fixture.history();
        history
            .publish_resume_point(&fixture.point(1, 0x8601))
            .unwrap();
        drop(history);

        let copy = fixture.root.join("archive-copy");
        copy_tree(&fixture.archive, &copy);
        let copied = ObjectStore::open(&copy, fixture.workspace)
            .unwrap()
            .open_engine_history(fixture.binding)
            .unwrap();

        // The copy is byte-identical, so only its physical control-directory
        // identity distinguishes it. That is exactly what the promoted-state
        // binding authenticates, and the resume point inherits it.
        assert!(copied.read_resume_point_set().is_err());
        assert!(copied
            .publish_resume_point(&fixture.point(2, 0x8601))
            .is_err());
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}
