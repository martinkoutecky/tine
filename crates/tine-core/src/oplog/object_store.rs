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
use std::collections::BTreeMap;
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ahash::AHashMap;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use uuid::Uuid;

use super::identity::parse_digest;
use super::{
    BatchError, BatchId, ContentDigest, LineageDigest, ObjectDescriptor, OperationBatch,
    OperationObject, PreparedBatch, ValidatedBatch, WorkspaceId, MAX_MANIFEST_BYTES,
    MAX_OBJECT_BYTES,
};

const OBJECTS_DIR: &str = "objects";
const BATCHES_DIR: &str = "batches";
const LINEAGE_CLAIM_FILE: &str = "lineage.claim";
const ENGINE_HISTORY_DIR: &str = "engine-history";
const ENGINE_HISTORY_NODES_DIR: &str = "nodes";
const ENGINE_HISTORY_ROOTS_DIR: &str = "roots";
const ENGINE_HISTORY_CLAIM_FILE: &str = "engine-history.claim";
const ENGINE_HISTORY_HEAD_FILE: &str = "engine-history.head";
const ENGINE_HISTORY_ROOT_SUFFIX: &str = ".history-root";
const ENGINE_HISTORY_ROOT_SCHEMA_VERSION: u32 = 5;
const MAX_ENGINE_HISTORY_RECORD_BYTES: u64 = 1024 * 1024;
const MAX_ENGINE_HISTORY_INDEX_BYTES: u64 = 2 * 1024 * 1024;
const ENGINE_HISTORY_INDEX_SCHEMA_VERSION: u32 = 1;
const ENGINE_HISTORY_RADIX_DEPTH: u8 = 32;
#[cfg(test)]
const BLOCK_CLAIM_INDEX_DIR: &str = "block-claim-index";
const BLOCK_CLAIM_INDEX_FILE: &str = "pages.index";
const LOGSEQ_CLAIM_INDEX_DIR: &str = "logseq-uuid-claim-index-v1";
const PORTABLE_PATH_INDEX_DIR: &str = "portable-path-index-v1";
#[allow(dead_code)] // opened by the intentionally unwired P2N2 foundation
const PAGE_NAME_OWNERSHIP_INDEX_DIR: &str = "page-name-ownership-index-v1";
const PROJECTION_WORK_DIR: &str = "projection-work-index-v1";
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

#[cfg(test)]
thread_local! {
    static ENROLLED_OPEN_USE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static ENROLLED_OPEN_ACT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
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
struct ControlDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControlDirectoryIdentity {
    volume: u64,
    file_id: [u8; 16],
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControlDirectoryIdentity;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AcceptedReadStats {
    pub manifest_reads: usize,
    pub object_reads: usize,
}

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
}

#[derive(Debug)]
pub(crate) struct EngineHistoryStore {
    capability: Dir,
    counters: Arc<StoreCounters>,
}

#[derive(Debug)]
pub(crate) struct DurableEngineHistoryStore {
    workspace_id: WorkspaceId,
    endpoint_id: super::ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    control: Dir,
    roots: Dir,
    index: EngineHistoryStore,
    transition: Mutex<()>,
    authoritative_head: Mutex<Option<ContentDigest>>,
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
    binding: EngineHistoryBinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EngineHistoryAuthority {
    pub generation: u64,
    pub index_root: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineHistoryBinding {
    pub portable_path_key_version: u32,
    pub portable_path_root: ContentDigest,
    pub catalog_checkpoint_binding: ContentDigest,
    pub portable_path_conflicts: Vec<super::PortablePathConflict>,
    pub terminal_evidence: Option<EngineTerminalEvidenceBinding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineTerminalEvidenceBinding {
    pub conflict_root: ContentDigest,
    pub conflict_count: u64,
    pub participant_count: u64,
    pub canonical_digest: ContentDigest,
}

impl EngineHistoryBinding {
    fn empty() -> Self {
        Self {
            portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
            portable_path_root: super::PortablePathIndexRoot::empty().digest(),
            catalog_checkpoint_binding: ContentDigest::of(
                b"tine/empty-catalog-checkpoint-binding/v1",
            ),
            portable_path_conflicts: Vec::new(),
            terminal_evidence: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct BlockClaimIndexRoot {
    next_generation: u64,
    global_filter: Option<BlockClaimPageRef>,
    levels:
        [[Option<BlockClaimSegmentRef>; BLOCK_CLAIM_SEGMENTS_PER_LEVEL]; BLOCK_CLAIM_INDEX_LEVELS],
}

#[derive(Debug)]
pub(crate) struct BlockClaimIndexStore {
    file: Mutex<fs::File>,
    counters: Arc<StoreCounters>,
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

impl ObjectStore {
    /// Open or create a store at an explicit root and retain the opened
    /// directory capability for all later operations.
    pub fn open(root: &Path, workspace_id: WorkspaceId) -> Result<Self, StoreError> {
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
                sync_dir_required(&parent_capability)?;
            }
            Err(error) => return Err(error.into()),
        }

        let capability = open_dir_nofollow(&parent_capability, name)?;
        ensure_directory(&capability, OBJECTS_DIR)?;
        ensure_directory(&capability, BATCHES_DIR)?;
        let store = Self {
            root_path: canonical_parent.join(name),
            workspace_id,
            capability,
            counters: Arc::new(StoreCounters::default()),
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

    pub(crate) fn sqlite_lease_capability(&self) -> std::io::Result<Dir> {
        self.capability.try_clone()
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
        let manifest = OperationBatch::decode(bytes)?;
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
        if batch.manifest().workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: batch.manifest().workspace_id(),
            });
        }
        for object in batch.objects() {
            self.stage_object_bytes(&object.encode()?)?;
        }
        self.stage_manifest_bytes(&batch.manifest().encode()?)?;
        Ok(())
    }

    /// Inspect a single manifest and validate every present required object.
    /// Missing objects stage the batch; corrupt or mismatched objects reject it.
    pub fn inspect_batch(&self, batch_id: BatchId) -> Result<BatchInspection, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        let manifest_bytes =
            match read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)? {
                None => return Ok(BatchInspection::Absent),
                Some(bytes) => bytes,
            };
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

        let objects_dir = self.open_namespace(OBJECTS_DIR)?;
        let mut missing = Vec::new();
        let mut objects = Vec::with_capacity(manifest.required_objects().len());
        for descriptor in manifest.required_objects() {
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
        let mut history = match self.seal_existing_engine_history(binding) {
            Ok(history) => history,
            Err(error) => return Err((self, error)),
        };
        let mut work = match self.seal_existing_projection_work(binding) {
            Ok(work) => work,
            Err(error) => return Err((self, error)),
        };
        let history_parent_created = match history.bind_absent_parent(&self.capability) {
            Ok(created) => created,
            Err(error) => return Err((self, error)),
        };
        if let Err(error) = work.bind_absent_parent(&self.capability) {
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

    fn seal_existing_engine_history(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<SealedControl<DurableEngineHistoryStore>, StoreError> {
        let Some(histories) = open_existing_dir_nofollow(&self.capability, ENGINE_HISTORY_DIR)?
        else {
            return Ok(SealedControl::Absent(AbsentControlName {
                namespace_name: ENGINE_HISTORY_DIR,
                namespace: None,
                namespace_identity: None,
                endpoint_name: binding.endpoint.endpoint_id.to_string(),
            }));
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) = open_existing_dir_nofollow(&histories, &endpoint_name)? else {
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
        let Some(root) = open_existing_dir_nofollow(&self.capability, PROJECTION_WORK_DIR)? else {
            return Ok(SealedControl::Absent(AbsentControlName {
                namespace_name: PROJECTION_WORK_DIR,
                namespace: None,
                namespace_identity: None,
                endpoint_name: binding.endpoint.endpoint_id.to_string(),
            }));
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) = open_existing_dir_nofollow(&root, &endpoint_name)? else {
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
        ensure_directory_nofollow(&self.capability, ENGINE_HISTORY_DIR)?;
        let histories = open_dir_nofollow(&self.capability, ENGINE_HISTORY_DIR)?;
        let endpoint_name = endpoint.endpoint_id.to_string();
        ensure_directory_nofollow(&histories, &endpoint_name)?;
        let control = open_dir_nofollow(&histories, &endpoint_name)?;
        for name in [ENGINE_HISTORY_NODES_DIR, ENGINE_HISTORY_ROOTS_DIR] {
            ensure_directory_nofollow(&control, name)?;
        }
        DurableEngineHistoryStore::new(
            self.workspace_id,
            endpoint.endpoint_id,
            endpoint.graph_resource_id,
            binding.receipt_store_id,
            control.try_clone()?,
            open_dir_nofollow(&control, ENGINE_HISTORY_ROOTS_DIR)?,
            EngineHistoryStore {
                capability: open_dir_nofollow(&control, ENGINE_HISTORY_NODES_DIR)?,
                counters: Arc::clone(&self.counters),
            },
        )
    }

    fn open_absent_engine_history(
        &self,
        absence: AbsentControlName,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<DurableEngineHistoryStore, StoreError> {
        let control = absence.claim(&self.capability)?;
        for name in [ENGINE_HISTORY_NODES_DIR, ENGINE_HISTORY_ROOTS_DIR] {
            control.create_dir(name)?;
        }
        sync_dir_required(&control)?;
        DurableEngineHistoryStore::new(
            self.workspace_id,
            binding.endpoint.endpoint_id,
            binding.endpoint.graph_resource_id,
            binding.receipt_store_id,
            control.try_clone()?,
            open_dir_nofollow(&control, ENGINE_HISTORY_ROOTS_DIR)?,
            EngineHistoryStore {
                capability: open_dir_nofollow(&control, ENGINE_HISTORY_NODES_DIR)?,
                counters: Arc::clone(&self.counters),
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn start_engine_history(&self) -> Result<EngineHistoryStore, StoreError> {
        ensure_directory(&self.capability, ENGINE_HISTORY_DIR)?;
        let histories = self.open_namespace(ENGINE_HISTORY_DIR)?;
        let run = format!("run-{}", Uuid::new_v4());
        ensure_directory(&histories, &run)?;
        Ok(EngineHistoryStore {
            capability: open_dir_nofollow(&histories, &run)?,
            counters: Arc::clone(&self.counters),
        })
    }

    #[cfg(test)]
    pub(crate) fn start_block_claim_index(&self) -> Result<BlockClaimIndexStore, StoreError> {
        ensure_directory(&self.capability, BLOCK_CLAIM_INDEX_DIR)?;
        let indexes = self.open_namespace(BLOCK_CLAIM_INDEX_DIR)?;
        let run = format!("run-{}", Uuid::new_v4());
        ensure_directory(&indexes, &run)?;
        let run = open_dir_nofollow(&indexes, &run)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        let file = run.open_with(BLOCK_CLAIM_INDEX_FILE, &options)?.into_std();
        file.sync_all()?;
        sync_dir_required(&run)?;
        Ok(BlockClaimIndexStore {
            file: Mutex::new(file),
            counters: Arc::clone(&self.counters),
        })
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
        let claim_index = BlockClaimIndexStore {
            file: Mutex::new(
                scratch
                    .clone_pages_file()
                    .map_err(|error| StoreError::Scratch(error.to_string()))?,
            ),
            counters: Arc::clone(&self.counters),
        };
        Ok((scratch, claim_index))
    }

    pub(crate) fn open_logseq_claim_index(
        &self,
    ) -> Result<super::uuid_claim_index::LogseqClaimIndexStore, StoreError> {
        ensure_directory_nofollow(&self.capability, LOGSEQ_CLAIM_INDEX_DIR)?;
        Ok(super::uuid_claim_index::LogseqClaimIndexStore::new(
            open_dir_nofollow(&self.capability, LOGSEQ_CLAIM_INDEX_DIR)?,
        ))
    }

    pub(crate) fn open_portable_path_index(
        &self,
    ) -> Result<super::portable_path_index::PortablePathIndexStore, StoreError> {
        ensure_directory_nofollow(&self.capability, PORTABLE_PATH_INDEX_DIR)?;
        Ok(super::portable_path_index::PortablePathIndexStore::new(
            super::authenticated_patricia::PatriciaIndexStore::new(open_dir_nofollow(
                &self.capability,
                PORTABLE_PATH_INDEX_DIR,
            )?),
        ))
    }

    #[allow(dead_code)] // activated only by later P2N2 acceptance wiring
    pub(crate) fn open_page_name_ownership_index(
        &self,
    ) -> Result<super::page_name_index::PageNameOwnershipStore, StoreError> {
        ensure_directory_nofollow(&self.capability, PAGE_NAME_OWNERSHIP_INDEX_DIR)?;
        let index = open_dir_nofollow(&self.capability, PAGE_NAME_OWNERSHIP_INDEX_DIR)?;
        super::page_name_index::PageNameOwnershipStore::open(index)
    }

    #[cfg(test)]
    pub(crate) fn open_projection_work_index(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<super::ProjectionWorkIndex, StoreError> {
        self.preflight_projection_work_index(binding)?;
        let endpoint = binding.endpoint;
        ensure_directory_nofollow(&self.capability, PROJECTION_WORK_DIR)?;
        let root = open_dir_nofollow(&self.capability, PROJECTION_WORK_DIR)?;
        let endpoint_name = endpoint.endpoint_id.to_string();
        ensure_directory_nofollow(&root, &endpoint_name)?;
        let endpoint_dir = open_dir_nofollow(&root, &endpoint_name)?;
        for name in ["nodes", "roots", "prepared"] {
            ensure_directory_nofollow(&endpoint_dir, name)?;
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
        let endpoint_dir = absence.claim(&self.capability)?;
        for name in ["nodes", "roots", "prepared"] {
            endpoint_dir.create_dir(name)?;
        }
        sync_dir_required(&endpoint_dir)?;
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

    #[cfg(test)]
    fn preflight_engine_history(
        &self,
        binding: super::hot_engine::ProjectionStorageBinding,
    ) -> Result<(), StoreError> {
        let Some(histories) = open_existing_dir_nofollow(&self.capability, ENGINE_HISTORY_DIR)?
        else {
            return Ok(());
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) = open_existing_dir_nofollow(&histories, &endpoint_name)? else {
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
                let _nodes = open_existing_dir_nofollow(&control, ENGINE_HISTORY_NODES_DIR)?
                    .ok_or(StoreError::MalformedHistoryIndex)?;
                let roots = open_existing_dir_nofollow(&control, ENGINE_HISTORY_ROOTS_DIR)?
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
        let Some(root) = open_existing_dir_nofollow(&self.capability, PROJECTION_WORK_DIR)? else {
            return Ok(());
        };
        let endpoint_name = binding.endpoint.endpoint_id.to_string();
        let Some(control) = open_existing_dir_nofollow(&root, &endpoint_name)? else {
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
        open_dir_nofollow(&self.capability, name)
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

impl<T> SealedControl<T> {
    fn bind_absent_parent(&mut self, store_root: &Dir) -> Result<bool, StoreError> {
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
        sync_dir_required(store_root)?;
        let namespace = open_dir_nofollow(store_root, absence.namespace_name)?;
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

    fn claim(self, store_root: &Dir) -> Result<Dir, StoreError> {
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
                sync_dir_required(store_root)?;
                open_dir_nofollow(store_root, self.namespace_name)?
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
        sync_dir_required(&namespace)?;
        open_dir_nofollow(&namespace, &self.endpoint_name)
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

    fn publish_node(&self, node: &HistoryIndexNode) -> Result<ContentDigest, StoreError> {
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

    fn read_node(&self, digest: ContentDigest) -> Result<HistoryIndexNode, StoreError> {
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

impl DurableEngineHistoryStore {
    fn open_sealed_existing(
        workspace_id: WorkspaceId,
        endpoint_id: super::ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
        control: Dir,
        counters: Arc<StoreCounters>,
    ) -> Result<Self, StoreError> {
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
            roots,
            index: EngineHistoryStore {
                capability: nodes,
                counters,
            },
            transition: Mutex::new(()),
            authoritative_head: Mutex::new(None),
        };
        let (digest, root) = store.read_live_head_root()?;
        store.require_root_binding(&root)?;
        *store
            .authoritative_head
            .lock()
            .map_err(|_| StoreError::MalformedHistoryIndex)? = Some(digest);
        Ok(store)
    }

    fn new(
        workspace_id: WorkspaceId,
        endpoint_id: super::ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
        control: Dir,
        roots: Dir,
        index: EngineHistoryStore,
    ) -> Result<Self, StoreError> {
        let store = Self {
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
            control,
            roots,
            index,
            transition: Mutex::new(()),
            authoritative_head: Mutex::new(None),
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

    pub(crate) fn current_with_binding(
        &self,
    ) -> Result<(u64, ContentDigest, Option<BatchId>, EngineHistoryBinding), StoreError> {
        let (_, root) = self.load_head_root()?;
        Ok((
            root.generation,
            root.index_root,
            root.latest_batch_id,
            root.binding.clone(),
        ))
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
        self.require_root_binding(&root)
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
        let (before_digest, before) = self.load_head_root()?;
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
            binding,
        };
        let after_digest = self.publish_root(&after)?;
        self.replace_head(before_digest, after_digest)?;
        Ok((after.generation, after.index_root))
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
                    binding: EngineHistoryBinding::empty(),
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
            self.control
                .rename(&temp_name, &self.control, ENGINE_HISTORY_HEAD_FILE)?;
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
        || root.binding.portable_path_key_version != super::PORTABLE_PATH_KEY_VERSION
        || (root.generation == 0) != root.latest_batch_id.is_none()
        || root
            .binding
            .portable_path_conflicts
            .windows(2)
            .any(|pair| pair[0].key_digest() >= pair[1].key_digest())
        || root.binding.portable_path_conflicts.iter().any(|conflict| {
            conflict.key_version() != super::PORTABLE_PATH_KEY_VERSION
                || conflict.participants().len() < 2
                || conflict
                    .participants()
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
        })
        || (!root.binding.portable_path_conflicts.is_empty()
            && root.binding.terminal_evidence.is_none())
    {
        return Err(StoreError::MalformedHistoryIndex);
    }
    Ok(())
}

impl BlockClaimIndexStore {
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
        let mut file = self
            .file
            .lock()
            .map_err(|_| StoreError::MalformedBlockClaimIndex)?;
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
            &mut file,
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
            let filter = self.read_claim_filter(&mut file, segment.filter_ref)?;
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
            self.lookup_many_at(
                &mut file,
                segment.page_ref,
                0,
                &selected,
                &mut segment_found,
            )?;
            found.extend(segment_found);
            remaining.retain(|(key, _, _)| !found.contains_key(key));
            if remaining.is_empty() {
                break;
            }
        }
        Ok(found)
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
        let mut file = self
            .file
            .lock()
            .map_err(|_| StoreError::MalformedBlockClaimIndex)?;
        let generation = root
            .next_generation
            .checked_add(1)
            .ok_or(StoreError::MalformedBlockClaimIndex)?;
        let mut global_filter = match root.global_filter {
            Some(page_ref) => self.read_claim_global_filter(&mut file, page_ref)?,
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
                let filter_ref = self.append_claim_filter(&mut file, &merged)?;
                let page_ref = self.build_claim_subtree(&mut file, 0, merged)?;
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
                self.materialize_claim_segment(&mut file, segment.page_ref, 0, &mut older)?;
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
        next.global_filter = Some(self.append_claim_global_filter(&mut file, &global_filter)?);
        Ok(next)
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

#[derive(Clone, Copy)]
enum Collision {
    Object(ContentDigest),
    Batch(BatchId),
    HistoryIndex(ContentDigest),
    Lineage(LineageDigest),
    Exact(&'static str),
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

fn open_existing_dir_nofollow(root: &Dir, name: &str) -> Result<Option<Dir>, StoreError> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            StoreError::UnsafeEntry(format!("{name} is not a real no-follow directory")),
        ),
        Ok(_) => open_dir_nofollow(root, name).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn control_directory_identity(dir: &Dir) -> Result<ControlDirectoryIdentity, StoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = dir.try_clone()?.into_std_file().metadata()?;
    Ok(ControlDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn control_directory_identity(dir: &Dir) -> Result<ControlDirectoryIdentity, StoreError> {
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
fn control_directory_identity(_dir: &Dir) -> Result<ControlDirectoryIdentity, StoreError> {
    Err(StoreError::Io(std::io::Error::new(
        ErrorKind::Unsupported,
        "directory identity is unavailable on this platform",
    )))
}

pub(crate) fn ensure_directory_nofollow(root: &Dir, name: &str) -> Result<(), StoreError> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(StoreError::UnsafeEntry(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    root.create_dir(name)?;
    sync_dir_required(root)
}

fn ensure_directory(root: &Dir, name: &str) -> Result<(), StoreError> {
    ensure_directory_nofollow(root, name)
}

fn publish_immutable(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    collision: Collision,
) -> Result<(), StoreError> {
    // Windows must establish that this directory supports the required
    // write-capable FlushFileBuffers operation before inserting an immutable
    // target name. The retained handle is reused for the post-insertion flush,
    // so namespace retargeting cannot redirect durability to another path.
    let publication_sync = PublicationDirSync::open(dir)?;
    publication_sync.preflight()?;
    let temp_name = format!(".tmp-{}", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut temp = dir.open_with(&temp_name, &options)?;
    let result = (|| {
        temp.write_all(bytes)?;
        temp.sync_all()?;
        drop(temp);
        match rename_noreplace(dir, &temp_name, filename) {
            // A post-insertion sync error can leave the correct immutable
            // target present. Retrying is safe: the AlreadyExists path below
            // verifies identical bytes and retries this same required sync.
            Ok(()) => publication_sync.sync(),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                verify_existing(dir, filename, bytes, collision)?;
                publication_sync.sync()
            }
            Err(error) => Err(error.into()),
        }
    })();
    let cleanup = dir.remove_file(&temp_name);
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
    Ok(())
}

pub(crate) fn publish_immutable_exact(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    kind: &'static str,
) -> Result<(), StoreError> {
    publish_immutable(dir, filename, bytes, Collision::Exact(kind))
}

fn verify_existing(
    dir: &Dir,
    filename: &str,
    expected: &[u8],
    collision: Collision,
) -> Result<(), StoreError> {
    let existing = match read_required_regular(
        dir,
        filename,
        expected.len() as u64,
        Some(expected.len() as u64),
    ) {
        Ok(existing) => existing,
        Err(StoreError::StoredLengthMismatch { .. } | StoreError::StoredFileTooLarge { .. }) => {
            return Err(collision_error(collision));
        }
        Err(error) => return Err(error),
    };
    if existing == expected {
        return Ok(());
    }
    Err(collision_error(collision))
}

fn collision_error(collision: Collision) -> StoreError {
    match collision {
        Collision::Object(digest) => StoreError::ObjectCollision(digest),
        Collision::Batch(batch_id) => StoreError::BatchCollision(batch_id),
        Collision::HistoryIndex(digest) => StoreError::HistoryIndexPathMismatch(digest),
        Collision::Lineage(lineage) => StoreError::LineageClaimCollision(lineage),
        Collision::Exact(kind) => StoreError::ImmutableCollision(kind),
    }
}

#[cfg(unix)]
fn open_file_nofollow(dir: &Dir, path: &str) -> std::io::Result<fs::File> {
    let path = CString::new(path)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid stored filename"))?;
    // SAFETY: `path` is a live NUL-terminated string and `dir` is an opened
    // directory capability. O_NOFOLLOW binds validation and reading to the
    // same opened regular-file handle.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a newly owned descriptor.
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
fn open_file_nofollow(dir: &Dir, path: &str) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(path, &options)?.into_std();
    reject_windows_reparse(&file, path)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_file_nofollow(_dir: &Dir, _path: &str) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow reads are unsupported on this target",
    ))
}

#[cfg(unix)]
pub(crate) fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
    let path = CString::new(path)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid directory name"))?;
    // SAFETY: as in `open_file_nofollow`; O_DIRECTORY rejects non-directories
    // and O_NOFOLLOW rejects a final-component symlink in the same operation.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `openat` returned a newly owned directory descriptor.
    Ok(Dir::from_std_file(unsafe { fs::File::from_raw_fd(fd) }))
}

#[cfg(windows)]
pub(crate) fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .maybe_dir(true);
    let file = dir.open_with(path, &options)?.into_std();
    let metadata = file.metadata()?;
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
        || !metadata.is_dir()
    {
        return Err(StoreError::UnsafeEntry(format!(
            "{path} is not a real no-follow directory"
        )));
    }
    Ok(Dir::from_std_file(file))
}

#[cfg(windows)]
fn reject_windows_reparse(file: &fs::File, path: &str) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("opened path is a reparse point: {path}"),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn open_dir_nofollow(_dir: &Dir, _path: &str) -> Result<Dir, StoreError> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow directory opens are unsupported on this target",
    )
    .into())
}

pub(crate) fn read_optional_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Option<Vec<u8>>, StoreError> {
    let mut file = match open_file_nofollow(dir, path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(StoreError::UnsafeEntry(format!(
            "stored path is not a regular no-follow file: {path}"
        )));
    }
    let length = metadata.len();
    if let Some(expected) = expected_length {
        if length != expected {
            return Err(StoreError::StoredLengthMismatch {
                path: path.into(),
                expected,
                actual: length,
            });
        }
    }
    if length > limit {
        return Err(StoreError::StoredFileTooLarge {
            path: path.into(),
            length,
            limit,
        });
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(StoreError::StoredFileTooLarge {
            path: path.into(),
            length: bytes.len() as u64,
            limit,
        });
    }
    if bytes.len() as u64 != length {
        return Err(StoreError::StoredLengthMismatch {
            path: path.into(),
            expected: length,
            actual: bytes.len() as u64,
        });
    }
    Ok(Some(bytes))
}

fn read_required_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Vec<u8>, StoreError> {
    read_optional_regular(dir, path, limit, expected_length)?.ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            ErrorKind::NotFound,
            format!("missing stored file {path}"),
        ))
    })
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
            std::fs::remove_dir_all(root).unwrap();
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
            std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
                binding: EngineHistoryBinding::empty(),
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::remove_dir_all(root).unwrap();
    }
}

pub(crate) fn require_regular_entry(
    file_type: &cap_std::fs::FileType,
    name: &str,
) -> Result<(), StoreError> {
    if file_type.is_symlink() || !file_type.is_file() {
        Err(StoreError::UnsafeEntry(format!(
            "namespace entry is not a regular no-follow file: {name}"
        )))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(dir: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    let from = CString::new(from)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid temporary name"))?;
    let to = CString::new(to)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid target name"))?;
    // SAFETY: both C strings are alive for the call, contain no interior NUL,
    // and both relative paths are resolved beneath the already-open directory.
    let result = unsafe {
        libc::renameat2(
            dir.as_fd().as_raw_fd(),
            from.as_ptr(),
            dir.as_fd().as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "android", windows))]
fn rename_noreplace(dir: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    // Hard-link creation is an atomic exclusive name insertion on these
    // platforms: it fails if `to` already exists. Both names are in the same
    // opened directory and the source is a private, synced regular file.
    dir.hard_link(from, dir, to)?;
    dir.remove_file(from)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
)))]
fn rename_noreplace(_dir: &Dir, _from: &str, _to: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-clobber publication is unsupported on this target",
    ))
}

#[cfg(unix)]
pub(crate) fn sync_dir_required(dir: &Dir) -> Result<(), StoreError> {
    // cap-std may retain an O_PATH directory capability, which is suitable for
    // openat but cannot itself be fsynced. Open the capability's `.` as a real
    // directory descriptor and propagate the result of syncing that handle.
    let dot = c".";
    // SAFETY: `dot` is a static C string and `dir` is an opened directory.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            dot.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `openat` returned a newly owned directory descriptor.
    unsafe { fs::File::from_raw_fd(fd) }.sync_all()?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn sync_dir_required(dir: &Dir) -> Result<(), StoreError> {
    PublicationDirSync::open(dir)?.sync()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn sync_dir_required(_dir: &Dir) -> Result<(), StoreError> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "directory durability is unsupported on this target",
    )
    .into())
}

#[cfg(windows)]
struct PublicationDirSync(fs::File);

#[cfg(windows)]
impl PublicationDirSync {
    fn open(dir: &Dir) -> Result<Self, StoreError> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .follow(FollowSymlinks::No)
            .maybe_dir(true);
        let file = dir.open_with(".", &options)?.into_std();
        let metadata = file.metadata()?;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
            || !metadata.is_dir()
        {
            return Err(StoreError::UnsafeEntry(
                "directory durability handle is not a real no-follow directory".into(),
            ));
        }
        Ok(Self(file))
    }

    fn preflight(&self) -> Result<(), StoreError> {
        self.sync()
    }

    fn sync(&self) -> Result<(), StoreError> {
        use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;

        // SAFETY: the handle remains owned by `self` for the call. `open`
        // requested GENERIC_WRITE, which FlushFileBuffers requires, together
        // with directory and no-follow semantics.
        if unsafe { FlushFileBuffers(self.0.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
struct PublicationDirSync<'a>(&'a Dir);

#[cfg(not(windows))]
impl<'a> PublicationDirSync<'a> {
    fn open(dir: &'a Dir) -> Result<Self, StoreError> {
        Ok(Self(dir))
    }

    fn preflight(&self) -> Result<(), StoreError> {
        Ok(())
    }

    fn sync(&self) -> Result<(), StoreError> {
        sync_dir_required(self.0)
    }
}
