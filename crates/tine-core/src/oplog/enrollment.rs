//! Authoritative device-local enrollment lifecycle journal.
//!
//! Enrollment records only persisted state. It deliberately exposes no graph
//! writer or projection authorization. The trusted private application-data
//! placement and no-follow checks match the project's baseline threat model:
//! accidental substitution is rejected, while malicious namespace races by
//! another process with the same user authority remain out of scope.
//!
//! Writable callers retain [`EnrollmentLease`] for their whole session. The
//! required global lock order is: enrollment lease, archive/engine lease, then
//! graph and process-local locks.

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
#[cfg(windows)]
use cap_std::fs::OpenOptions;
use cap_std::{ambient_authority, fs::Dir};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::identity::parse_digest;
use super::object_store::{ensure_directory_nofollow, open_dir_nofollow, sync_dir_required};
use super::{
    BatchId, CanonicalArchiveResourceId, CanonicalGraphResourceId, ContentDigest, DeviceId,
    DocumentId, GraphTextScopeBinding, ImportId, LineageDigest, ProjectionEndpointId,
    ProjectionReceiptStoreId, SessionId, WorkspaceId, DIFF_SCHEMA_VERSION,
    MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION,
    OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION, PROJECTION_POLICY_VERSION,
    PROJECTION_SCHEMA_VERSION, RECEIPT_SCHEMA_VERSION,
};

pub(crate) const ENROLLMENT_RECORD_SCHEMA_VERSION: u32 = 1;
pub(crate) const PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_ENROLLMENT_RECORD_BYTES: usize = 32 * 1024;
pub(crate) const MAX_ENROLLMENT_JSON_DEPTH: usize = 16;
pub(crate) const MAX_ENROLLMENT_JSON_TOKENS: usize = 256;
pub(crate) const MAX_ENROLLMENT_CHAIN_RECORDS: usize = 1024;
pub(crate) const MAX_ENROLLMENT_AUDIT_PAGE: usize = 64;
pub(crate) const MAX_ENROLLMENT_NAMESPACE_ENTRIES: usize = 2048;
pub(crate) const MAX_BLOCKED_REASON_CODE_BYTES: usize = 64;

const SPARSE_STORAGE_DIRECTORY: &str = "sparse-storage";
const STORAGE_VERSION_DIRECTORY: &str = "v2";
const LOCAL_DIRECTORY: &str = "local";
const ENROLLMENT_DIRECTORY: &str = "enrollment";
const RECORDS_DIRECTORY: &str = "records";
const LEASE_FILE: &str = "lease";
const HEAD_FILE: &str = "head";
const RECORD_SUFFIX: &str = ".enrollment";
const HEAD_BYTES: usize = 65;
const HEAD_TEMP_PREFIX: &str = ".head-tmp-";
const RECORD_TEMP_PREFIX: &str = ".record-tmp-";

/// A private application-data root selected by Tine, never a graph path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnrollmentApplicationRoot {
    path: PathBuf,
}

impl EnrollmentApplicationRoot {
    pub(crate) fn open() -> Result<Self, EnrollmentError> {
        let base = dirs::data_local_dir().ok_or_else(|| {
            EnrollmentError::UnsafeNamespace(
                "platform did not provide a per-user local-data directory".into(),
            )
        })?;
        let application_id = if cfg!(target_os = "android") {
            "page.tine.app"
        } else {
            "page.tine.Tine"
        };
        prepare_application_root(&base.join(application_id))
    }

    pub(crate) fn open_for_harness(path: &Path) -> Result<Self, EnrollmentError> {
        prepare_application_root(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn prepare_application_root(path: &Path) -> Result<EnrollmentApplicationRoot, EnrollmentError> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(EnrollmentError::UnsafeNamespace(
            "private enrollment application root is not a real directory".into(),
        ));
    }
    let path = fs::canonicalize(path)?;
    let directory = Dir::open_ambient_dir(&path, ambient_authority())?;
    validate_private_directory(&directory, "private enrollment application root")?;
    Ok(EnrollmentApplicationRoot { path })
}

/// Opaque identity of one non-mutating enrollment preparation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct PreparationId(Uuid);

impl PreparationId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[cfg(test)]
    const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnrollmentCompatibilityV1 {
    oplog_protocol_version: u32,
    operation_schema_version: u32,
    object_envelope_schema_version: u32,
    manifest_encoding_version: u32,
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
    diff_schema_version: u32,
}

impl EnrollmentCompatibilityV1 {
    pub(crate) const fn current() -> Self {
        Self {
            oplog_protocol_version: OPLOG_PROTOCOL_VERSION,
            operation_schema_version: OPERATION_SCHEMA_VERSION,
            object_envelope_schema_version: OBJECT_ENVELOPE_SCHEMA_VERSION,
            manifest_encoding_version: MANIFEST_ENCODING_VERSION,
            receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            projection_policy_version: PROJECTION_POLICY_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
            diff_schema_version: DIFF_SCHEMA_VERSION,
        }
    }

    fn validate_current(self) -> Result<(), EnrollmentError> {
        if self != Self::current() {
            return Err(EnrollmentError::UnsupportedCompatibility {
                expected: Self::current(),
                found: self,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnrollmentBindingV1 {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    endpoint_id: ProjectionEndpointId,
    device_id: DeviceId,
    graph_resource_id: CanonicalGraphResourceId,
    receipt_store_id: ProjectionReceiptStoreId,
    archive_resource_id: CanonicalArchiveResourceId,
    graph_text_scope_binding: GraphTextScopeBinding,
    compatibility: EnrollmentCompatibilityV1,
}

impl EnrollmentBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
        endpoint_id: ProjectionEndpointId,
        device_id: DeviceId,
        graph_resource_id: CanonicalGraphResourceId,
        receipt_store_id: ProjectionReceiptStoreId,
        archive_resource_id: CanonicalArchiveResourceId,
        graph_text_scope_binding: GraphTextScopeBinding,
    ) -> Result<Self, EnrollmentError> {
        let binding = Self {
            workspace_id,
            lineage_digest,
            catalog_document_id,
            endpoint_id,
            device_id,
            graph_resource_id,
            receipt_store_id,
            archive_resource_id,
            graph_text_scope_binding,
            compatibility: EnrollmentCompatibilityV1::current(),
        };
        binding.validate_internal()?;
        Ok(binding)
    }

    pub(crate) const fn graph_resource_id(&self) -> CanonicalGraphResourceId {
        self.graph_resource_id
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn catalog_document_id(&self) -> DocumentId {
        self.catalog_document_id
    }

    pub(crate) const fn endpoint_id(&self) -> ProjectionEndpointId {
        self.endpoint_id
    }

    pub(crate) const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub(crate) const fn receipt_store_id(&self) -> ProjectionReceiptStoreId {
        self.receipt_store_id
    }

    pub(crate) const fn archive_resource_id(&self) -> CanonicalArchiveResourceId {
        self.archive_resource_id
    }

    pub(crate) const fn graph_text_scope_binding(&self) -> GraphTextScopeBinding {
        self.graph_text_scope_binding
    }

    pub(crate) const fn compatibility(&self) -> EnrollmentCompatibilityV1 {
        self.compatibility
    }

    fn validate_internal(&self) -> Result<(), EnrollmentError> {
        self.compatibility.validate_current()?;
        if self.graph_text_scope_binding.graph_resource_id() != self.graph_resource_id {
            return Err(EnrollmentError::BindingMismatch(
                EnrollmentBindingField::GraphTextScope,
            ));
        }
        Ok(())
    }

    fn validate_exact(&self, expected: &Self) -> Result<(), EnrollmentError> {
        self.validate_internal()?;
        let mismatch = if self.workspace_id != expected.workspace_id {
            Some(EnrollmentBindingField::Workspace)
        } else if self.lineage_digest != expected.lineage_digest {
            Some(EnrollmentBindingField::Lineage)
        } else if self.catalog_document_id != expected.catalog_document_id {
            Some(EnrollmentBindingField::CatalogDocument)
        } else if self.endpoint_id != expected.endpoint_id {
            Some(EnrollmentBindingField::Endpoint)
        } else if self.device_id != expected.device_id {
            Some(EnrollmentBindingField::Device)
        } else if self.graph_resource_id != expected.graph_resource_id {
            Some(EnrollmentBindingField::GraphResource)
        } else if self.receipt_store_id != expected.receipt_store_id {
            Some(EnrollmentBindingField::ReceiptStore)
        } else if self.archive_resource_id != expected.archive_resource_id {
            Some(EnrollmentBindingField::ArchiveResource)
        } else if self.graph_text_scope_binding != expected.graph_text_scope_binding {
            Some(EnrollmentBindingField::GraphTextScope)
        } else if self.compatibility != expected.compatibility {
            Some(EnrollmentBindingField::Compatibility)
        } else {
            None
        };
        if let Some(field) = mismatch {
            return Err(EnrollmentError::BindingMismatch(field));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentBindingField {
    Workspace,
    Lineage,
    CatalogDocument,
    Endpoint,
    Device,
    GraphResource,
    ReceiptStore,
    ArchiveResource,
    GraphTextScope,
    Compatibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedFrontierAnchorV1 {
    pub(crate) acceptance_sequence: u64,
    pub(crate) accepted_frontier_state_digest: ContentDigest,
    pub(crate) history_generation: u64,
    pub(crate) history_root: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShadowImportV1 {
    pub(crate) preparation_id: PreparationId,
    pub(crate) source_inventory_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedLocalV1 {
    pub(crate) preparation_id: PreparationId,
    pub(crate) source_inventory_digest: ContentDigest,
    pub(crate) backup_manifest_digest: ContentDigest,
    pub(crate) backup_restore_proof_digest: ContentDigest,
    pub(crate) bootstrap_batch_id: BatchId,
    pub(crate) accepted_frontier_anchor: AcceptedFrontierAnchorV1,
    pub(crate) staged_projection_manifest_digest: ContentDigest,
    pub(crate) byte_compare_digest: ContentDigest,
}

impl VerifiedLocalV1 {
    pub(crate) fn verification_digest(&self) -> Result<ContentDigest, EnrollmentError> {
        let bytes =
            serde_json::to_vec(self).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
        Ok(ContentDigest::of(&bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HandoffV1 {
    Safe,
    Unsafe { session_id: SessionId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishedRecoveryPacketV1 {
    packet_schema_version: u32,
    pub(crate) batch_id: BatchId,
    pub(crate) import_id: ImportId,
    pub(crate) manifest_digest: ContentDigest,
    pub(crate) archive_resource_id: CanonicalArchiveResourceId,
    pub(crate) published_from: AcceptedFrontierAnchorV1,
}

impl PublishedRecoveryPacketV1 {
    pub(crate) fn new(
        batch_id: BatchId,
        import_id: ImportId,
        manifest_digest: ContentDigest,
        archive_resource_id: CanonicalArchiveResourceId,
        published_from: AcceptedFrontierAnchorV1,
    ) -> Result<Self, EnrollmentError> {
        let packet = Self {
            packet_schema_version: PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION,
            batch_id,
            import_id,
            manifest_digest,
            archive_resource_id,
            published_from,
        };
        packet.validate()?;
        Ok(packet)
    }

    fn validate(&self) -> Result<(), EnrollmentError> {
        if self.packet_schema_version != PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION {
            return Err(EnrollmentError::UnsupportedPacketSchema(
                self.packet_schema_version,
            ));
        }
        if self.batch_id != BatchId::for_import(self.import_id) {
            return Err(EnrollmentError::PublishedBatchMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LocalExclusionV1 {
    Idle,
    Published { packet: PublishedRecoveryPacketV1 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalActiveV1 {
    pub(crate) verification_digest: ContentDigest,
    pub(crate) handoff: HandoffV1,
    pub(crate) exclusion: LocalExclusionV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlockedV1 {
    pub(crate) prior_record_digest: ContentDigest,
    pub(crate) reason_code: String,
    pub(crate) evidence_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EnrollmentLifecycleV1 {
    ShadowImport(ShadowImportV1),
    VerifiedLocal(VerifiedLocalV1),
    LocalActive(LocalActiveV1),
    Blocked(BlockedV1),
}

impl EnrollmentLifecycleV1 {
    fn validate(
        &self,
        binding: &EnrollmentBindingV1,
        previous: Option<ContentDigest>,
    ) -> Result<(), EnrollmentError> {
        match self {
            Self::ShadowImport(_) | Self::VerifiedLocal(_) => Ok(()),
            Self::LocalActive(active) => {
                if matches!(active.handoff, HandoffV1::Safe)
                    && matches!(active.exclusion, LocalExclusionV1::Published { .. })
                {
                    return Err(EnrollmentError::IllegalLifecycle(
                        "a published exclusion cannot be marked handoff-safe",
                    ));
                }
                if let LocalExclusionV1::Published { packet } = &active.exclusion {
                    packet.validate()?;
                    if packet.archive_resource_id != binding.archive_resource_id {
                        return Err(EnrollmentError::BindingMismatch(
                            EnrollmentBindingField::ArchiveResource,
                        ));
                    }
                }
                Ok(())
            }
            Self::Blocked(blocked) => {
                if Some(blocked.prior_record_digest) != previous {
                    return Err(EnrollmentError::IllegalLifecycle(
                        "blocked evidence does not identify the immediately prior record",
                    ));
                }
                validate_reason_code(&blocked.reason_code)
            }
        }
    }
}

fn validate_reason_code(reason: &str) -> Result<(), EnrollmentError> {
    if reason.is_empty()
        || reason.len() > MAX_BLOCKED_REASON_CODE_BYTES
        || reason
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    {
        return Err(EnrollmentError::InvalidBlockedReason);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnrollmentRecordV1 {
    schema_version: u32,
    generation: u64,
    previous: Option<ContentDigest>,
    binding: EnrollmentBindingV1,
    lifecycle: EnrollmentLifecycleV1,
}

impl EnrollmentRecordV1 {
    fn initial(
        binding: EnrollmentBindingV1,
        shadow: ShadowImportV1,
    ) -> Result<Self, EnrollmentError> {
        let record = Self {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 1,
            previous: None,
            binding,
            lifecycle: EnrollmentLifecycleV1::ShadowImport(shadow),
        };
        record.validate()?;
        Ok(record)
    }

    fn successor(
        current: &EnrollmentSnapshot,
        lifecycle: EnrollmentLifecycleV1,
    ) -> Result<Self, EnrollmentError> {
        validate_transition(&current.record.lifecycle, &lifecycle, current.digest)?;
        let generation = current
            .record
            .generation
            .checked_add(1)
            .ok_or(EnrollmentError::GenerationOverflow)?;
        let record = Self {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation,
            previous: Some(current.digest),
            binding: current.record.binding.clone(),
            lifecycle,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), EnrollmentError> {
        if self.schema_version != ENROLLMENT_RECORD_SCHEMA_VERSION {
            return Err(EnrollmentError::UnsupportedRecordSchema(
                self.schema_version,
            ));
        }
        if self.generation == 0 || (self.generation == 1) != self.previous.is_none() {
            return Err(EnrollmentError::NonmonotonicGeneration);
        }
        self.binding.validate_internal()?;
        self.lifecycle.validate(&self.binding, self.previous)
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) const fn previous(&self) -> Option<ContentDigest> {
        self.previous
    }

    pub(crate) const fn binding(&self) -> &EnrollmentBindingV1 {
        &self.binding
    }

    pub(crate) const fn lifecycle(&self) -> &EnrollmentLifecycleV1 {
        &self.lifecycle
    }
}

fn validate_transition(
    current: &EnrollmentLifecycleV1,
    next: &EnrollmentLifecycleV1,
    current_digest: ContentDigest,
) -> Result<(), EnrollmentError> {
    let legal = match (current, next) {
        (
            EnrollmentLifecycleV1::ShadowImport(shadow),
            EnrollmentLifecycleV1::VerifiedLocal(verified),
        ) => {
            shadow.preparation_id == verified.preparation_id
                && shadow.source_inventory_digest == verified.source_inventory_digest
        }
        (EnrollmentLifecycleV1::ShadowImport(_), EnrollmentLifecycleV1::Blocked(blocked)) => {
            blocked.prior_record_digest == current_digest
        }
        (
            EnrollmentLifecycleV1::VerifiedLocal(verified),
            EnrollmentLifecycleV1::LocalActive(active),
        ) => {
            verified
                .verification_digest()
                .is_ok_and(|digest| digest == active.verification_digest)
                && matches!(active.handoff, HandoffV1::Unsafe { .. })
                && matches!(active.exclusion, LocalExclusionV1::Idle)
        }
        (EnrollmentLifecycleV1::LocalActive(current), EnrollmentLifecycleV1::LocalActive(next)) => {
            current.verification_digest == next.verification_digest
                && legal_local_active_transition(current, next)
        }
        (EnrollmentLifecycleV1::LocalActive(_), EnrollmentLifecycleV1::Blocked(blocked)) => {
            blocked.prior_record_digest == current_digest
        }
        _ => false,
    };
    if !legal {
        return Err(EnrollmentError::IllegalTransition);
    }
    Ok(())
}

fn legal_local_active_transition(current: &LocalActiveV1, next: &LocalActiveV1) -> bool {
    match (
        current.handoff,
        &current.exclusion,
        next.handoff,
        &next.exclusion,
    ) {
        (
            HandoffV1::Safe,
            LocalExclusionV1::Idle,
            HandoffV1::Unsafe { .. },
            LocalExclusionV1::Idle,
        )
        | (
            HandoffV1::Unsafe { .. },
            LocalExclusionV1::Idle,
            HandoffV1::Safe,
            LocalExclusionV1::Idle,
        )
        | (
            HandoffV1::Unsafe { .. },
            LocalExclusionV1::Published { .. },
            HandoffV1::Unsafe { .. },
            LocalExclusionV1::Idle,
        ) => true,
        (
            HandoffV1::Unsafe {
                session_id: current,
            },
            LocalExclusionV1::Idle,
            HandoffV1::Unsafe { session_id: next },
            LocalExclusionV1::Idle,
        ) => current != next,
        (
            HandoffV1::Unsafe {
                session_id: current,
            },
            LocalExclusionV1::Idle,
            HandoffV1::Unsafe { session_id: next },
            LocalExclusionV1::Published { .. },
        ) => current == next,
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnrollmentSnapshot {
    pub(crate) digest: ContentDigest,
    pub(crate) record: EnrollmentRecordV1,
}

#[derive(Debug)]
struct EnrollmentDirectories {
    enrollment: Dir,
    records: Dir,
    display_path: PathBuf,
}

pub(crate) enum EnrollmentOpen<T> {
    Absent,
    Present(T),
}

pub(crate) struct EnrollmentReader {
    directories: EnrollmentDirectories,
    current: EnrollmentSnapshot,
}

impl EnrollmentReader {
    pub(crate) fn open_existing(
        root: &EnrollmentApplicationRoot,
        expected_binding: &EnrollmentBindingV1,
    ) -> Result<EnrollmentOpen<Self>, EnrollmentError> {
        let Some(directories) = open_directories(root, expected_binding.graph_resource_id, false)?
        else {
            return Ok(EnrollmentOpen::Absent);
        };
        validate_namespaces(&directories)?;
        let Some(current) = read_head_and_chain(&directories, expected_binding)? else {
            return Ok(EnrollmentOpen::Absent);
        };
        Ok(EnrollmentOpen::Present(Self {
            directories,
            current,
        }))
    }

    pub(crate) fn current(&self) -> &EnrollmentSnapshot {
        &self.current
    }

    pub(crate) fn audit_chain_page(
        &self,
        start: Option<ContentDigest>,
        limit: usize,
    ) -> Result<EnrollmentAuditPage, EnrollmentError> {
        if limit == 0 || limit > MAX_ENROLLMENT_AUDIT_PAGE {
            return Err(EnrollmentError::InvalidPageLimit(limit));
        }
        let mut next = Some(start.unwrap_or(self.current.digest));
        let mut records = Vec::with_capacity(limit);
        while records.len() < limit {
            let Some(digest) = next else {
                break;
            };
            let record = read_record(&self.directories.records, digest)?;
            record
                .binding
                .validate_exact(&self.current.record.binding)?;
            record.validate()?;
            next = record.previous;
            records.push(EnrollmentSnapshot { digest, record });
        }
        Ok(EnrollmentAuditPage { records, next })
    }
}

pub(crate) struct EnrollmentAuditPage {
    pub(crate) records: Vec<EnrollmentSnapshot>,
    pub(crate) next: Option<ContentDigest>,
}

/// OS-backed exclusive lease retained by one writable enrollment session.
pub(crate) struct EnrollmentLease {
    file: File,
}

impl Drop for EnrollmentLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) struct EnrollmentWriter {
    reader: EnrollmentReader,
    _lease: EnrollmentLease,
}

impl EnrollmentWriter {
    pub(crate) fn create(
        root: &EnrollmentApplicationRoot,
        binding: EnrollmentBindingV1,
        shadow: ShadowImportV1,
    ) -> Result<Self, EnrollmentError> {
        binding.validate_internal()?;
        let directories = open_directories(root, binding.graph_resource_id, true)?
            .expect("create mode returns enrollment directories");
        validate_namespaces(&directories)?;
        let lease = acquire_lease(&directories, true)?;
        if read_head(&directories.enrollment)?.is_some() {
            return Err(EnrollmentError::AlreadyExists);
        }
        require_no_persisted_records(&directories.records)?;
        let record = EnrollmentRecordV1::initial(binding, shadow)?;
        let snapshot = persist_record_and_head(&directories, &record, CommitCut::None)?;
        Ok(Self {
            reader: EnrollmentReader {
                directories,
                current: snapshot,
            },
            _lease: lease,
        })
    }

    pub(crate) fn open_existing(
        root: &EnrollmentApplicationRoot,
        expected_binding: &EnrollmentBindingV1,
    ) -> Result<EnrollmentOpen<Self>, EnrollmentError> {
        let Some(directories) = open_directories(root, expected_binding.graph_resource_id, false)?
        else {
            return Ok(EnrollmentOpen::Absent);
        };
        validate_namespaces(&directories)?;
        let lease = acquire_lease(&directories, false)?;
        let Some(current) = read_head_and_chain(&directories, expected_binding)? else {
            return Ok(EnrollmentOpen::Absent);
        };
        Ok(EnrollmentOpen::Present(Self {
            reader: EnrollmentReader {
                directories,
                current,
            },
            _lease: lease,
        }))
    }

    pub(crate) fn current(&self) -> &EnrollmentSnapshot {
        self.reader.current()
    }

    pub(crate) fn audit_chain_page(
        &self,
        start: Option<ContentDigest>,
        limit: usize,
    ) -> Result<EnrollmentAuditPage, EnrollmentError> {
        self.reader.audit_chain_page(start, limit)
    }

    pub(crate) fn transition(
        &mut self,
        expected_current: ContentDigest,
        lifecycle: EnrollmentLifecycleV1,
    ) -> Result<&EnrollmentSnapshot, EnrollmentError> {
        self.transition_at_cut(expected_current, lifecycle, CommitCut::None)
    }

    fn transition_at_cut(
        &mut self,
        expected_current: ContentDigest,
        lifecycle: EnrollmentLifecycleV1,
        cut: CommitCut,
    ) -> Result<&EnrollmentSnapshot, EnrollmentError> {
        if self.reader.current.digest != expected_current
            || read_head(&self.reader.directories.enrollment)? != Some(expected_current)
        {
            return Err(EnrollmentError::StaleCompareAndSwap);
        }
        let record = EnrollmentRecordV1::successor(&self.reader.current, lifecycle)?;
        let snapshot = persist_record_and_head(&self.reader.directories, &record, cut)?;
        self.reader.current = snapshot;
        Ok(&self.reader.current)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitCut {
    None,
    #[cfg(test)]
    AfterRecordSync,
    #[cfg(test)]
    AfterHeadTempSync,
    #[cfg(test)]
    AfterHeadRename,
}

fn persist_record_and_head(
    directories: &EnrollmentDirectories,
    record: &EnrollmentRecordV1,
    cut: CommitCut,
) -> Result<EnrollmentSnapshot, EnrollmentError> {
    let bytes = canonical_record_bytes(record)?;
    let digest = ContentDigest::from_bytes(Sha256::digest(&bytes).into());
    publish_record(&directories.records, digest, &bytes)?;
    if cut_after_record_sync(cut) {
        return Err(EnrollmentError::InjectedCrashCut("after_record_sync"));
    }

    let temp_name = format!("{HEAD_TEMP_PREFIX}{}", Uuid::new_v4());
    let mut temp = create_new_regular(&directories.enrollment, &temp_name)?;
    temp.write_all(format!("{digest}\n").as_bytes())?;
    temp.sync_all()?;
    drop(temp);
    if cut_after_head_temp_sync(cut) {
        return Err(EnrollmentError::InjectedCrashCut("after_head_temp_sync"));
    }

    reject_unsafe_head_target(&directories.enrollment)?;
    directories
        .enrollment
        .rename(&temp_name, &directories.enrollment, HEAD_FILE)?;
    if cut_after_head_rename(cut) {
        return Err(EnrollmentError::InjectedCrashCut("after_head_rename"));
    }
    sync_dir_required(&directories.enrollment)
        .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    Ok(EnrollmentSnapshot {
        digest,
        record: record.clone(),
    })
}

#[cfg(test)]
fn cut_after_record_sync(cut: CommitCut) -> bool {
    cut == CommitCut::AfterRecordSync
}

#[cfg(not(test))]
fn cut_after_record_sync(_cut: CommitCut) -> bool {
    false
}

#[cfg(test)]
fn cut_after_head_temp_sync(cut: CommitCut) -> bool {
    cut == CommitCut::AfterHeadTempSync
}

#[cfg(not(test))]
fn cut_after_head_temp_sync(_cut: CommitCut) -> bool {
    false
}

#[cfg(test)]
fn cut_after_head_rename(cut: CommitCut) -> bool {
    cut == CommitCut::AfterHeadRename
}

#[cfg(not(test))]
fn cut_after_head_rename(_cut: CommitCut) -> bool {
    false
}

fn canonical_record_bytes(record: &EnrollmentRecordV1) -> Result<Vec<u8>, EnrollmentError> {
    record.validate()?;
    let bytes =
        serde_json::to_vec(record).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    if bytes.len() > MAX_ENROLLMENT_RECORD_BYTES {
        return Err(EnrollmentError::RecordTooLarge(bytes.len()));
    }
    Ok(bytes)
}

fn decode_record(bytes: &[u8]) -> Result<EnrollmentRecordV1, EnrollmentError> {
    validate_json_bounds(bytes)?;
    let probe: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
    let schema = probe
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| EnrollmentError::Decode("record schema_version is missing".into()))?;
    if schema != u64::from(ENROLLMENT_RECORD_SCHEMA_VERSION) {
        return Err(EnrollmentError::UnsupportedRecordSchema(
            u32::try_from(schema).unwrap_or(u32::MAX),
        ));
    }
    let lifecycle_state = probe
        .get("lifecycle")
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| EnrollmentError::Decode("record lifecycle state is missing".into()))?;
    if !matches!(
        lifecycle_state,
        "shadow_import" | "verified_local" | "local_active" | "blocked"
    ) {
        return Err(EnrollmentError::FutureUnsupportedLifecycle(
            lifecycle_state.to_owned(),
        ));
    }
    let record: EnrollmentRecordV1 = serde_json::from_slice(bytes)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
    record.validate()?;
    if canonical_record_bytes(&record)? != bytes {
        return Err(EnrollmentError::NonCanonicalRecord);
    }
    Ok(record)
}

fn validate_json_bounds(bytes: &[u8]) -> Result<(), EnrollmentError> {
    if bytes.len() > MAX_ENROLLMENT_RECORD_BYTES {
        return Err(EnrollmentError::RecordTooLarge(bytes.len()));
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(EnrollmentError::Decode("record is not UTF-8".into()));
    }
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut tokens = 0usize;
    for byte in bytes.iter().copied() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                tokens = tokens.saturating_add(1);
                if depth > MAX_ENROLLMENT_JSON_DEPTH {
                    return Err(EnrollmentError::JsonDepthExceeded);
                }
            }
            b'}' | b']' => {
                if depth == 0 {
                    return Err(EnrollmentError::Decode(
                        "record has unbalanced JSON delimiters".into(),
                    ));
                }
                depth -= 1;
                tokens = tokens.saturating_add(1);
            }
            b',' | b':' => tokens = tokens.saturating_add(1),
            _ => {}
        }
        if tokens > MAX_ENROLLMENT_JSON_TOKENS {
            return Err(EnrollmentError::JsonTokenBoundExceeded);
        }
    }
    if in_string || escaped || depth != 0 {
        return Err(EnrollmentError::Decode(
            "record has unterminated JSON structure".into(),
        ));
    }
    Ok(())
}

fn read_head_and_chain(
    directories: &EnrollmentDirectories,
    expected_binding: &EnrollmentBindingV1,
) -> Result<Option<EnrollmentSnapshot>, EnrollmentError> {
    let Some(head) = read_head(&directories.enrollment)? else {
        return Ok(None);
    };
    let current = read_record(&directories.records, head)?;
    current.binding.validate_exact(expected_binding)?;
    current.validate()?;

    let mut seen = BTreeSet::new();
    let mut digest = head;
    let mut record = current.clone();
    for count in 0..MAX_ENROLLMENT_CHAIN_RECORDS {
        if !seen.insert(digest) {
            return Err(EnrollmentError::ChainCycle);
        }
        match record.previous {
            None => {
                if record.generation != 1
                    || !matches!(record.lifecycle, EnrollmentLifecycleV1::ShadowImport(_))
                {
                    return Err(EnrollmentError::NonmonotonicGeneration);
                }
                return Ok(Some(EnrollmentSnapshot {
                    digest: head,
                    record: current,
                }));
            }
            Some(previous_digest) => {
                let previous = read_record(&directories.records, previous_digest)?;
                previous.binding.validate_exact(expected_binding)?;
                previous.validate()?;
                if record.generation != previous.generation.saturating_add(1) {
                    return Err(EnrollmentError::NonmonotonicGeneration);
                }
                validate_transition(&previous.lifecycle, &record.lifecycle, previous_digest)?;
                digest = previous_digest;
                record = previous;
            }
        }
        if count + 1 == MAX_ENROLLMENT_CHAIN_RECORDS {
            return Err(EnrollmentError::ChainBoundExceeded);
        }
    }
    unreachable!("bounded chain loop returns at its limit")
}

fn read_head(directory: &Dir) -> Result<Option<ContentDigest>, EnrollmentError> {
    let metadata = match directory.symlink_metadata(HEAD_FILE) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EnrollmentError::UnsafeNamespace(
            "enrollment head is not a regular no-follow file".into(),
        ));
    }
    if metadata.len() != HEAD_BYTES as u64 {
        return Err(EnrollmentError::MalformedHead);
    }
    let file = open_regular_readonly(directory, HEAD_FILE)?;
    let mut bytes = Vec::with_capacity(HEAD_BYTES);
    file.take((HEAD_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() != HEAD_BYTES || bytes[64] != b'\n' {
        return Err(EnrollmentError::MalformedHead);
    }
    let text = std::str::from_utf8(&bytes[..64]).map_err(|_| EnrollmentError::MalformedHead)?;
    let digest = parse_digest(text).map_err(|_| EnrollmentError::MalformedHead)?;
    Ok(Some(ContentDigest::from_bytes(digest)))
}

fn read_record(
    records: &Dir,
    expected_digest: ContentDigest,
) -> Result<EnrollmentRecordV1, EnrollmentError> {
    let name = format!("{expected_digest}{RECORD_SUFFIX}");
    let metadata = match records.symlink_metadata(&name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(EnrollmentError::MissingChainRecord(expected_digest));
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "enrollment record {name} is not a regular no-follow file"
        )));
    }
    if metadata.len() > MAX_ENROLLMENT_RECORD_BYTES as u64 {
        return Err(EnrollmentError::RecordTooLarge(
            usize::try_from(metadata.len()).unwrap_or(usize::MAX),
        ));
    }
    let file = open_regular_readonly(records, &name)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_ENROLLMENT_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if ContentDigest::of(&bytes) != expected_digest {
        return Err(EnrollmentError::RecordDigestMismatch(expected_digest));
    }
    decode_record(&bytes)
}

fn publish_record(
    records: &Dir,
    digest: ContentDigest,
    bytes: &[u8],
) -> Result<(), EnrollmentError> {
    let target = format!("{digest}{RECORD_SUFFIX}");
    let temp_name = format!("{RECORD_TEMP_PREFIX}{}", Uuid::new_v4());
    let mut temp = create_new_regular(records, &temp_name)?;
    temp.write_all(bytes)?;
    temp.sync_all()?;
    drop(temp);
    match rename_noreplace(records, &temp_name, &target) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing = read_record(records, digest)?;
            if canonical_record_bytes(&existing)? != bytes {
                return Err(EnrollmentError::RecordDigestMismatch(digest));
            }
            let _ = records.remove_file(&temp_name);
        }
        Err(error) => return Err(error.into()),
    }
    sync_dir_required(records).map_err(|error| EnrollmentError::Durability(error.to_string()))
}

fn require_no_persisted_records(records: &Dir) -> Result<(), EnrollmentError> {
    let mut count = 0usize;
    for entry in records.entries()? {
        let entry = entry?;
        count += 1;
        if count > MAX_ENROLLMENT_NAMESPACE_ENTRIES {
            return Err(EnrollmentError::NamespaceBoundExceeded);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| EnrollmentError::UnsupportedArtifact("non-UTF-8 record name".into()))?;
        if name.starts_with(RECORD_TEMP_PREFIX) && regular_entry(&entry)? {
            continue;
        }
        return Err(EnrollmentError::AlreadyExists);
    }
    Ok(())
}

fn reject_unsafe_head_target(directory: &Dir) -> Result<(), EnrollmentError> {
    match directory.symlink_metadata(HEAD_FILE) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(EnrollmentError::UnsafeNamespace(
                "enrollment head target is not a regular no-follow file".into(),
            ))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_namespaces(directories: &EnrollmentDirectories) -> Result<(), EnrollmentError> {
    validate_private_directory(&directories.enrollment, "enrollment directory")?;
    validate_private_directory(&directories.records, "enrollment records directory")?;

    let mut count = 0usize;
    for entry in directories.enrollment.entries()? {
        let entry = entry?;
        count += 1;
        if count > MAX_ENROLLMENT_NAMESPACE_ENTRIES {
            return Err(EnrollmentError::NamespaceBoundExceeded);
        }
        let name = entry.file_name().into_string().map_err(|_| {
            EnrollmentError::UnsupportedArtifact("non-UTF-8 enrollment artifact".into())
        })?;
        let accepted = match name.as_str() {
            RECORDS_DIRECTORY => entry.file_type()?.is_dir(),
            HEAD_FILE | LEASE_FILE => regular_entry(&entry)?,
            _ if name.starts_with(HEAD_TEMP_PREFIX) => regular_entry(&entry)?,
            _ => false,
        };
        if !accepted {
            return Err(EnrollmentError::UnsupportedArtifact(name));
        }
    }

    count = 0;
    for entry in directories.records.entries()? {
        let entry = entry?;
        count += 1;
        if count > MAX_ENROLLMENT_NAMESPACE_ENTRIES {
            return Err(EnrollmentError::NamespaceBoundExceeded);
        }
        let name = entry.file_name().into_string().map_err(|_| {
            EnrollmentError::UnsupportedArtifact("non-UTF-8 record artifact".into())
        })?;
        let accepted = (is_record_name(&name) || name.starts_with(RECORD_TEMP_PREFIX))
            && regular_entry(&entry)?;
        if !accepted {
            return Err(EnrollmentError::UnsupportedArtifact(name));
        }
    }
    Ok(())
}

fn regular_entry(entry: &cap_std::fs::DirEntry) -> Result<bool, EnrollmentError> {
    let kind = entry.file_type()?;
    Ok(!kind.is_symlink() && kind.is_file())
}

fn is_record_name(name: &str) -> bool {
    let Some(digest) = name.strip_suffix(RECORD_SUFFIX) else {
        return false;
    };
    parse_digest(digest).is_ok()
}

fn open_directories(
    root: &EnrollmentApplicationRoot,
    graph_resource: CanonicalGraphResourceId,
    create: bool,
) -> Result<Option<EnrollmentDirectories>, EnrollmentError> {
    let root_dir = Dir::open_ambient_dir(root.path(), ambient_authority())?;
    let sparse = open_component(&root_dir, SPARSE_STORAGE_DIRECTORY, create)?;
    let Some(sparse) = sparse else {
        return Ok(None);
    };
    let version = open_component(&sparse, STORAGE_VERSION_DIRECTORY, create)?;
    let Some(version) = version else {
        return Ok(None);
    };
    let local = open_component(&version, LOCAL_DIRECTORY, create)?;
    let Some(local) = local else {
        return Ok(None);
    };
    let graph_name = graph_resource.to_string();
    let graph = open_component(&local, &graph_name, create)?;
    let Some(graph) = graph else {
        return Ok(None);
    };
    let enrollment = open_component(&graph, ENROLLMENT_DIRECTORY, create)?;
    let Some(enrollment) = enrollment else {
        return Ok(None);
    };
    let records = open_component(&enrollment, RECORDS_DIRECTORY, create)?;
    let Some(records) = records else {
        return Err(EnrollmentError::UnsafeNamespace(
            "enrollment exists without its records directory".into(),
        ));
    };
    Ok(Some(EnrollmentDirectories {
        enrollment,
        records,
        display_path: root
            .path()
            .join(SPARSE_STORAGE_DIRECTORY)
            .join(STORAGE_VERSION_DIRECTORY)
            .join(LOCAL_DIRECTORY)
            .join(graph_name)
            .join(ENROLLMENT_DIRECTORY),
    }))
}

fn open_component(parent: &Dir, name: &str, create: bool) -> Result<Option<Dir>, EnrollmentError> {
    let created;
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(EnrollmentError::UnsafeNamespace(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        Ok(_) => created = false,
        Err(error) if error.kind() == ErrorKind::NotFound && !create => return Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            ensure_directory_nofollow(parent, name)
                .map_err(|error| EnrollmentError::UnsafeNamespace(error.to_string()))?;
            created = true;
        }
        Err(error) => return Err(error.into()),
    }
    let directory = open_dir_nofollow(parent, name)
        .map_err(|error| EnrollmentError::UnsafeNamespace(error.to_string()))?;
    #[cfg(unix)]
    if created {
        let descriptor = directory.try_clone()?.into_std_file();
        // SAFETY: this changes the exact retained directory descriptor.
        if unsafe { libc::fchmod(descriptor.as_raw_fd(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    validate_private_directory(&directory, name)?;
    Ok(Some(directory))
}

fn validate_private_directory(directory: &Dir, name: &str) -> Result<(), EnrollmentError> {
    let metadata = directory.try_clone()?.into_std_file().metadata()?;
    if !metadata.is_dir() {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "{name} is not an opened directory"
        )));
    }
    #[cfg(unix)]
    if metadata.uid() !=
        // SAFETY: geteuid has no arguments or memory-safety preconditions.
        unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "{name} is not exclusively writable by the current user"
        )));
    }
    Ok(())
}

fn acquire_lease(
    directories: &EnrollmentDirectories,
    create: bool,
) -> Result<EnrollmentLease, EnrollmentError> {
    let file = if create {
        open_regular_readwrite_create(&directories.enrollment, LEASE_FILE)?
    } else {
        match directories.enrollment.symlink_metadata(LEASE_FILE) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(EnrollmentError::UnsafeNamespace(
                    "enrollment lease is not a regular no-follow file".into(),
                ));
            }
            Ok(_) => open_regular_readwrite_existing(&directories.enrollment, LEASE_FILE)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(EnrollmentError::UnsafeNamespace(
                    "existing enrollment has no lease authority".into(),
                ));
            }
            Err(error) => return Err(error.into()),
        }
    };
    validate_lease_file(&file)?;
    if let Err(error) = file.try_lock_exclusive() {
        if matches!(
            error.kind(),
            ErrorKind::WouldBlock | ErrorKind::PermissionDenied
        ) {
            return Err(EnrollmentError::LeaseContended(
                directories.display_path.join(LEASE_FILE),
            ));
        }
        return Err(error.into());
    }
    Ok(EnrollmentLease { file })
}

fn validate_lease_file(file: &File) -> Result<(), EnrollmentError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(EnrollmentError::UnsafeNamespace(
            "opened enrollment lease is not a regular file".into(),
        ));
    }
    #[cfg(unix)]
    if metadata.uid() !=
        // SAFETY: geteuid has no arguments or memory-safety preconditions.
        unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(EnrollmentError::UnsafeNamespace(
            "opened enrollment lease has unsafe ownership or links".into(),
        ));
    }
    #[cfg(windows)]
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(EnrollmentError::UnsafeNamespace(
            "opened enrollment lease is a reparse point".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn open_regular_readonly(directory: &Dir, name: &str) -> std::io::Result<File> {
    openat_regular(directory, name, libc::O_RDONLY, 0)
}

#[cfg(windows)]
fn open_regular_readonly(directory: &Dir, name: &str) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    Ok(directory.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_regular_readonly(_directory: &Dir, _name: &str) -> std::io::Result<File> {
    Err(unsupported_filesystem())
}

#[cfg(unix)]
fn open_regular_readwrite_create(directory: &Dir, name: &str) -> std::io::Result<File> {
    openat_regular(directory, name, libc::O_RDWR | libc::O_CREAT, 0o600)
}

#[cfg(windows)]
fn open_regular_readwrite_create(directory: &Dir, name: &str) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    Ok(directory.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_regular_readwrite_create(_directory: &Dir, _name: &str) -> std::io::Result<File> {
    Err(unsupported_filesystem())
}

#[cfg(unix)]
fn open_regular_readwrite_existing(directory: &Dir, name: &str) -> std::io::Result<File> {
    openat_regular(directory, name, libc::O_RDWR, 0)
}

#[cfg(windows)]
fn open_regular_readwrite_existing(directory: &Dir, name: &str) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).follow(FollowSymlinks::No);
    Ok(directory.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_regular_readwrite_existing(_directory: &Dir, _name: &str) -> std::io::Result<File> {
    Err(unsupported_filesystem())
}

#[cfg(unix)]
fn create_new_regular(directory: &Dir, name: &str) -> std::io::Result<File> {
    openat_regular(
        directory,
        name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )
}

#[cfg(windows)]
fn create_new_regular(directory: &Dir, name: &str) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    Ok(directory.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn create_new_regular(_directory: &Dir, _name: &str) -> std::io::Result<File> {
    Err(unsupported_filesystem())
}

#[cfg(unix)]
fn openat_regular(directory: &Dir, name: &str, flags: i32, mode: u32) -> std::io::Result<File> {
    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid file name"))?;
    // SAFETY: name is a live relative C string and directory is retained.
    let fd = unsafe {
        libc::openat(
            directory.as_fd().as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: openat returned one newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(not(any(unix, windows)))]
fn unsupported_filesystem() -> std::io::Error {
    std::io::Error::new(
        ErrorKind::Unsupported,
        "durable no-follow enrollment files are unsupported on this target",
    )
}

#[cfg(target_os = "linux")]
fn rename_noreplace(directory: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    let from = CString::new(from)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid temporary name"))?;
    let to = CString::new(to)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid record name"))?;
    // SAFETY: both names are live relative C strings beneath one retained dir.
    let result = unsafe {
        libc::renameat2(
            directory.as_fd().as_raw_fd(),
            from.as_ptr(),
            directory.as_fd().as_raw_fd(),
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
fn rename_noreplace(directory: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    directory.hard_link(from, directory, to)?;
    directory.remove_file(from)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
)))]
fn rename_noreplace(_directory: &Dir, _from: &str, _to: &str) -> std::io::Result<()> {
    Err(unsupported_filesystem())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentError {
    Io(String),
    Durability(String),
    UnsafeNamespace(String),
    UnsupportedArtifact(String),
    NamespaceBoundExceeded,
    AlreadyExists,
    LeaseContended(PathBuf),
    MalformedHead,
    MissingChainRecord(ContentDigest),
    RecordDigestMismatch(ContentDigest),
    UnsupportedRecordSchema(u32),
    UnsupportedPacketSchema(u32),
    UnsupportedCompatibility {
        expected: EnrollmentCompatibilityV1,
        found: EnrollmentCompatibilityV1,
    },
    FutureUnsupportedLifecycle(String),
    Decode(String),
    Encode(String),
    NonCanonicalRecord,
    RecordTooLarge(usize),
    JsonDepthExceeded,
    JsonTokenBoundExceeded,
    BindingMismatch(EnrollmentBindingField),
    PublishedBatchMismatch,
    InvalidBlockedReason,
    IllegalLifecycle(&'static str),
    IllegalTransition,
    StaleCompareAndSwap,
    GenerationOverflow,
    NonmonotonicGeneration,
    ChainCycle,
    ChainBoundExceeded,
    InvalidPageLimit(usize),
    InjectedCrashCut(&'static str),
}

impl fmt::Display for EnrollmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "enrollment I/O failed: {error}"),
            Self::Durability(error) => write!(formatter, "enrollment durability failed: {error}"),
            Self::UnsafeNamespace(detail) => {
                write!(formatter, "unsafe enrollment namespace: {detail}")
            }
            Self::UnsupportedArtifact(name) => {
                write!(formatter, "unsupported enrollment artifact: {name}")
            }
            Self::NamespaceBoundExceeded => {
                formatter.write_str("enrollment namespace entry bound exceeded")
            }
            Self::AlreadyExists => formatter.write_str("enrollment already exists"),
            Self::LeaseContended(path) => {
                write!(
                    formatter,
                    "enrollment lease is already held: {}",
                    path.display()
                )
            }
            Self::MalformedHead => formatter.write_str("enrollment head is malformed"),
            Self::MissingChainRecord(digest) => {
                write!(formatter, "enrollment chain record is missing: {digest}")
            }
            Self::RecordDigestMismatch(digest) => {
                write!(formatter, "enrollment record digest mismatch: {digest}")
            }
            Self::UnsupportedRecordSchema(schema) => {
                write!(formatter, "unsupported enrollment schema {schema}")
            }
            Self::UnsupportedPacketSchema(schema) => {
                write!(formatter, "unsupported published packet schema {schema}")
            }
            Self::UnsupportedCompatibility { .. } => {
                formatter.write_str("unsupported enrollment compatibility bundle")
            }
            Self::FutureUnsupportedLifecycle(state) => {
                write!(formatter, "unsupported future/shared lifecycle {state}")
            }
            Self::Decode(error) => write!(formatter, "enrollment decode failed: {error}"),
            Self::Encode(error) => write!(formatter, "enrollment encode failed: {error}"),
            Self::NonCanonicalRecord => formatter.write_str("enrollment record is not canonical"),
            Self::RecordTooLarge(bytes) => {
                write!(formatter, "enrollment record is too large: {bytes} bytes")
            }
            Self::JsonDepthExceeded => formatter.write_str("enrollment JSON depth exceeded"),
            Self::JsonTokenBoundExceeded => {
                formatter.write_str("enrollment JSON token bound exceeded")
            }
            Self::BindingMismatch(field) => {
                write!(formatter, "enrollment binding mismatch: {field:?}")
            }
            Self::PublishedBatchMismatch => {
                formatter.write_str("published packet batch/import identity mismatch")
            }
            Self::InvalidBlockedReason => formatter.write_str("invalid blocked reason code"),
            Self::IllegalLifecycle(detail) => {
                write!(formatter, "illegal enrollment lifecycle: {detail}")
            }
            Self::IllegalTransition => formatter.write_str("illegal enrollment transition"),
            Self::StaleCompareAndSwap => formatter.write_str("stale enrollment compare-and-swap"),
            Self::GenerationOverflow => formatter.write_str("enrollment generation overflow"),
            Self::NonmonotonicGeneration => {
                formatter.write_str("nonmonotonic enrollment generation")
            }
            Self::ChainCycle => formatter.write_str("enrollment chain contains a cycle"),
            Self::ChainBoundExceeded => formatter.write_str("enrollment chain bound exceeded"),
            Self::InvalidPageLimit(limit) => {
                write!(formatter, "invalid enrollment audit page limit {limit}")
            }
            Self::InjectedCrashCut(cut) => write!(formatter, "injected crash cut: {cut}"),
        }
    }
}

impl std::error::Error for EnrollmentError {}

impl From<std::io::Error> for EnrollmentError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_text_scope::GraphTextScope;
    use pretty_assertions::assert_eq;
    use std::process::Command;

    struct TestRoot {
        path: PathBuf,
    }

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("tine-enrollment-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn app(&self) -> EnrollmentApplicationRoot {
            EnrollmentApplicationRoot::open_for_harness(&self.path).unwrap()
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_bytes([byte; 32])
    }

    fn graph_resource(byte: u8) -> CanonicalGraphResourceId {
        CanonicalGraphResourceId::from_capability_identity(b"test", &[byte])
    }

    fn archive_resource(byte: u8) -> CanonicalArchiveResourceId {
        CanonicalArchiveResourceId::from_capability_identity(b"test", &[byte])
    }

    fn receipt_store(byte: u8) -> ProjectionReceiptStoreId {
        ProjectionReceiptStoreId::from_capability_identity(b"test", &[byte])
    }

    fn test_binding() -> EnrollmentBindingV1 {
        let graph = graph_resource(1);
        EnrollmentBindingV1::new(
            WorkspaceId::from_uuid(Uuid::from_u128(1)),
            LineageDigest::from_bytes([2; 32]),
            DocumentId::from_uuid(Uuid::from_u128(3)),
            ProjectionEndpointId::from_uuid(Uuid::from_u128(4)),
            DeviceId::from_uuid(Uuid::from_u128(5)),
            graph,
            receipt_store(6),
            archive_resource(7),
            GraphTextScope::new(&[], false).bind_graph_resource(graph),
        )
        .unwrap()
    }

    fn shadow() -> ShadowImportV1 {
        ShadowImportV1 {
            preparation_id: PreparationId::from_uuid(Uuid::from_u128(8)),
            source_inventory_digest: digest(9),
        }
    }

    fn anchor(byte: u8) -> AcceptedFrontierAnchorV1 {
        AcceptedFrontierAnchorV1 {
            acceptance_sequence: u64::from(byte),
            accepted_frontier_state_digest: digest(byte),
            history_generation: u64::from(byte),
            history_root: digest(byte.wrapping_add(1)),
        }
    }

    fn verified() -> VerifiedLocalV1 {
        VerifiedLocalV1 {
            preparation_id: shadow().preparation_id,
            source_inventory_digest: shadow().source_inventory_digest,
            backup_manifest_digest: digest(10),
            backup_restore_proof_digest: digest(11),
            bootstrap_batch_id: BatchId::from_uuid(Uuid::from_u128(12)),
            accepted_frontier_anchor: anchor(13),
            staged_projection_manifest_digest: digest(14),
            byte_compare_digest: digest(15),
        }
    }

    fn active(handoff: HandoffV1, exclusion: LocalExclusionV1) -> EnrollmentLifecycleV1 {
        EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
            verification_digest: verified().verification_digest().unwrap(),
            handoff,
            exclusion,
        })
    }

    fn unsafe_idle(session: u128) -> EnrollmentLifecycleV1 {
        active(
            HandoffV1::Unsafe {
                session_id: SessionId::from_uuid(Uuid::from_u128(session)),
            },
            LocalExclusionV1::Idle,
        )
    }

    fn safe_idle() -> EnrollmentLifecycleV1 {
        active(HandoffV1::Safe, LocalExclusionV1::Idle)
    }

    fn packet(archive: CanonicalArchiveResourceId) -> PublishedRecoveryPacketV1 {
        let import = ImportId::from_digest([20; 32]);
        PublishedRecoveryPacketV1::new(
            BatchId::for_import(import),
            import,
            digest(21),
            archive,
            anchor(22),
        )
        .unwrap()
    }

    fn published(session: u128, archive: CanonicalArchiveResourceId) -> EnrollmentLifecycleV1 {
        active(
            HandoffV1::Unsafe {
                session_id: SessionId::from_uuid(Uuid::from_u128(session)),
            },
            LocalExclusionV1::Published {
                packet: packet(archive),
            },
        )
    }

    fn blocked(prior: ContentDigest) -> EnrollmentLifecycleV1 {
        EnrollmentLifecycleV1::Blocked(BlockedV1 {
            prior_record_digest: prior,
            reason_code: "proof.failed".into(),
            evidence_digest: digest(23),
        })
    }

    fn graph_directory(root: &TestRoot, binding: &EnrollmentBindingV1) -> PathBuf {
        root.path
            .join(SPARSE_STORAGE_DIRECTORY)
            .join(STORAGE_VERSION_DIRECTORY)
            .join(LOCAL_DIRECTORY)
            .join(binding.graph_resource_id.to_string())
    }

    fn enrollment_directory(root: &TestRoot, binding: &EnrollmentBindingV1) -> PathBuf {
        graph_directory(root, binding).join(ENROLLMENT_DIRECTORY)
    }

    fn record_path(
        root: &TestRoot,
        binding: &EnrollmentBindingV1,
        digest: ContentDigest,
    ) -> PathBuf {
        enrollment_directory(root, binding)
            .join(RECORDS_DIRECTORY)
            .join(format!("{digest}{RECORD_SUFFIX}"))
    }

    fn write_head(root: &TestRoot, binding: &EnrollmentBindingV1, digest: ContentDigest) {
        fs::write(
            enrollment_directory(root, binding).join(HEAD_FILE),
            format!("{digest}\n"),
        )
        .unwrap();
    }

    fn expect_present<T>(open: EnrollmentOpen<T>) -> T {
        match open {
            EnrollmentOpen::Present(value) => value,
            EnrollmentOpen::Absent => panic!("expected enrollment to be present"),
        }
    }

    #[test]
    fn canonical_record_bytes_digest_filename_and_round_trip_are_exact() {
        let root = TestRoot::new("canonical");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let snapshot = writer.current();
        let bytes = fs::read(record_path(&root, &binding, snapshot.digest)).unwrap();

        assert_eq!(ContentDigest::of(&bytes), snapshot.digest);
        assert_eq!(decode_record(&bytes).unwrap(), snapshot.record);
        assert_eq!(canonical_record_bytes(&snapshot.record).unwrap(), bytes);
        assert_eq!(
            record_path(&root, &binding, snapshot.digest)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            format!("{}{RECORD_SUFFIX}", snapshot.digest)
        );
    }

    #[test]
    fn unknown_tampered_future_and_noncanonical_records_fail_closed() {
        let record = EnrollmentRecordV1::initial(test_binding(), shadow()).unwrap();
        let canonical = canonical_record_bytes(&record).unwrap();

        let mut unknown: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("future".into(), serde_json::json!(true));
        assert!(matches!(
            decode_record(&serde_json::to_vec(&unknown).unwrap()),
            Err(EnrollmentError::Decode(_))
        ));

        let mut schema: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        schema["schema_version"] = serde_json::json!(2);
        assert_eq!(
            decode_record(&serde_json::to_vec(&schema).unwrap()),
            Err(EnrollmentError::UnsupportedRecordSchema(2))
        );

        let mut future: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        future["lifecycle"]["state"] = serde_json::json!("shared_active");
        assert_eq!(
            decode_record(&serde_json::to_vec(&future).unwrap()),
            Err(EnrollmentError::FutureUnsupportedLifecycle(
                "shared_active".into()
            ))
        );

        let pretty = serde_json::to_string_pretty(&record).unwrap();
        assert_eq!(
            decode_record(pretty.as_bytes()),
            Err(EnrollmentError::NonCanonicalRecord)
        );

        let mut compatibility: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        compatibility["binding"]["compatibility"]["operation_schema_version"] =
            serde_json::json!(OPERATION_SCHEMA_VERSION + 1);
        assert!(matches!(
            decode_record(&serde_json::to_vec(&compatibility).unwrap()),
            Err(EnrollmentError::UnsupportedCompatibility { .. })
        ));
    }

    #[test]
    fn lifecycle_transition_matrix_and_local_substates_are_exact() {
        let current_digest = digest(30);
        let states = [
            EnrollmentLifecycleV1::ShadowImport(shadow()),
            EnrollmentLifecycleV1::VerifiedLocal(verified()),
            unsafe_idle(31),
            blocked(current_digest),
        ];
        for (from_index, from) in states.iter().enumerate() {
            for (to_index, to) in states.iter().enumerate() {
                let expected = matches!((from_index, to_index), (0, 1) | (0, 3) | (1, 2) | (2, 3));
                assert_eq!(
                    validate_transition(from, to, current_digest).is_ok(),
                    expected,
                    "{from_index} -> {to_index}"
                );
            }
        }

        let unsafe_a = match unsafe_idle(40) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        let unsafe_b = match unsafe_idle(41) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        let safe = match safe_idle() {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        let published = match published(40, test_binding().archive_resource_id) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        assert!(legal_local_active_transition(&safe, &unsafe_a));
        assert!(legal_local_active_transition(&unsafe_a, &safe));
        assert!(legal_local_active_transition(&unsafe_a, &published));
        assert!(legal_local_active_transition(&unsafe_a, &unsafe_b));
        assert!(legal_local_active_transition(&published, &unsafe_b));
        assert!(!legal_local_active_transition(&unsafe_a, &unsafe_a));
        assert!(!legal_local_active_transition(&unsafe_b, &published));
        assert!(!legal_local_active_transition(&safe, &published));
        assert!(!legal_local_active_transition(&published, &published));
    }

    #[test]
    fn every_legal_cas_path_persists_and_stale_cas_is_rejected() {
        let root = TestRoot::new("cas");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let stale = writer.current().digest;

        let verified = EnrollmentLifecycleV1::VerifiedLocal(verified());
        let digest_verified = writer.transition(stale, verified).unwrap().digest;
        assert_eq!(
            writer.transition(stale, unsafe_idle(50)),
            Err(EnrollmentError::StaleCompareAndSwap)
        );
        let active_digest = writer
            .transition(digest_verified, unsafe_idle(50))
            .unwrap()
            .digest;
        let recovered_unclean_digest = writer
            .transition(active_digest, unsafe_idle(53))
            .unwrap()
            .digest;
        let safe_digest = writer
            .transition(recovered_unclean_digest, safe_idle())
            .unwrap()
            .digest;
        let unsafe_digest = writer
            .transition(safe_digest, unsafe_idle(51))
            .unwrap()
            .digest;
        let published_digest = writer
            .transition(unsafe_digest, published(51, binding.archive_resource_id))
            .unwrap()
            .digest;
        let recovered_digest = writer
            .transition(published_digest, unsafe_idle(52))
            .unwrap()
            .digest;
        let blocked_digest = writer
            .transition(recovered_digest, blocked(recovered_digest))
            .unwrap()
            .digest;
        drop(writer);

        let reader =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(reader.current().digest, blocked_digest);
        assert_eq!(reader.current().record.generation, 9);
        let page = reader.audit_chain_page(None, 3).unwrap();
        assert_eq!(page.records.len(), 3);
        assert!(page.next.is_some());
        assert!(matches!(
            reader.audit_chain_page(None, 0),
            Err(EnrollmentError::InvalidPageLimit(0))
        ));
    }

    #[test]
    fn crash_cuts_leave_old_or_new_valid_head() {
        for (cut, expect_new) in [
            (CommitCut::AfterRecordSync, false),
            (CommitCut::AfterHeadTempSync, false),
            (CommitCut::AfterHeadRename, true),
        ] {
            let root = TestRoot::new("crash");
            let binding = test_binding();
            let mut writer =
                EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            let old = writer.current().digest;
            let next = EnrollmentRecordV1::successor(
                writer.current(),
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
            )
            .unwrap();
            let expected_new = ContentDigest::of(&canonical_record_bytes(&next).unwrap());
            assert!(matches!(
                writer.transition_at_cut(
                    old,
                    EnrollmentLifecycleV1::VerifiedLocal(verified()),
                    cut
                ),
                Err(EnrollmentError::InjectedCrashCut(_))
            ));
            drop(writer);

            let reader =
                expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
            assert_eq!(
                reader.current().digest,
                if expect_new { expected_new } else { old }
            );
        }
    }

    #[test]
    fn exact_binding_substitution_is_rejected_without_replacing_bytes() {
        let mut variants = Vec::new();
        let canonical = test_binding();

        let mut endpoint = canonical.clone();
        endpoint.endpoint_id = ProjectionEndpointId::from_uuid(Uuid::from_u128(100));
        variants.push((endpoint, EnrollmentBindingField::Endpoint));

        let mut receipt = canonical.clone();
        receipt.receipt_store_id = receipt_store(101);
        variants.push((receipt, EnrollmentBindingField::ReceiptStore));

        let mut archive = canonical.clone();
        archive.archive_resource_id = archive_resource(102);
        variants.push((archive, EnrollmentBindingField::ArchiveResource));

        let mut scope = canonical.clone();
        scope.graph_text_scope_binding = GraphTextScope::new(&["hidden".into()], false)
            .bind_graph_resource(canonical.graph_resource_id);
        variants.push((scope, EnrollmentBindingField::GraphTextScope));

        for (expected, field) in variants {
            let root = TestRoot::new("binding");
            let writer =
                EnrollmentWriter::create(&root.app(), canonical.clone(), shadow()).unwrap();
            let head = writer.current().digest;
            drop(writer);
            assert!(matches!(
                EnrollmentReader::open_existing(&root.app(), &expected),
                Err(EnrollmentError::BindingMismatch(found)) if found == field
            ));
            assert_eq!(
                fs::read_to_string(enrollment_directory(&root, &canonical).join(HEAD_FILE))
                    .unwrap(),
                format!("{head}\n")
            );
        }
    }

    #[test]
    fn graph_resource_copy_and_device_substitution_fail_closed() {
        let root = TestRoot::new("resource-copy");
        let canonical = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), canonical.clone(), shadow()).unwrap();
        drop(writer);

        let mut substituted = canonical.clone();
        substituted.graph_resource_id = graph_resource(110);
        substituted.graph_text_scope_binding =
            GraphTextScope::new(&[], false).bind_graph_resource(substituted.graph_resource_id);
        copy_tree(
            &graph_directory(&root, &canonical),
            &graph_directory(&root, &substituted),
        );
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &substituted),
            Err(EnrollmentError::BindingMismatch(
                EnrollmentBindingField::GraphResource
            ))
        ));

        let mut device = canonical.clone();
        device.device_id = DeviceId::from_uuid(Uuid::from_u128(111));
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &device),
            Err(EnrollmentError::BindingMismatch(
                EnrollmentBindingField::Device
            ))
        ));
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn head_digest_chain_and_generation_corruption_fail_closed() {
        let root = TestRoot::new("head-corruption");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        fs::write(
            enrollment_directory(&root, &binding).join(HEAD_FILE),
            b"BAD\n",
        )
        .unwrap();
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::MalformedHead)
        ));

        let root = TestRoot::new("digest-corruption");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let head = writer.current().digest;
        drop(writer);
        let path = record_path(&root, &binding, head);
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(path, bytes).unwrap();
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::RecordDigestMismatch(found)) if found == head
        ));

        let root = TestRoot::new("missing-chain");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let old = writer.current().digest;
        let old_path = record_path(&root, &binding, old);
        let current = writer
            .transition(old, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        drop(writer);
        fs::remove_file(old_path).unwrap();
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::MissingChainRecord(found)) if found == old
        ));
        assert_eq!(
            fs::read_to_string(enrollment_directory(&root, &binding).join(HEAD_FILE)).unwrap(),
            format!("{current}\n")
        );

        let root = TestRoot::new("generation-corruption");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let previous = writer.current().digest;
        let mut bad = writer.current().record.clone();
        bad.generation = 4;
        bad.previous = Some(previous);
        bad.lifecycle = blocked(previous);
        drop(writer);
        let bad_bytes = serde_json::to_vec(&bad).unwrap();
        let bad_digest = ContentDigest::of(&bad_bytes);
        fs::write(record_path(&root, &binding, bad_digest), bad_bytes).unwrap();
        write_head(&root, &binding, bad_digest);
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::NonmonotonicGeneration)
        ));
    }

    #[test]
    fn explicit_create_open_absence_and_namespace_artifacts_are_bounded() {
        let root = TestRoot::new("explicit");
        let binding = test_binding();
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding).unwrap(),
            EnrollmentOpen::Absent
        ));
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        assert!(matches!(
            EnrollmentWriter::create(&root.app(), binding.clone(), shadow()),
            Err(EnrollmentError::AlreadyExists)
        ));

        fs::write(
            enrollment_directory(&root, &binding).join("future.store"),
            b"preserve",
        )
        .unwrap();
        assert_eq!(
            EnrollmentReader::open_existing(&root.app(), &binding)
                .err()
                .unwrap(),
            EnrollmentError::UnsupportedArtifact("future.store".into())
        );
        assert_eq!(
            fs::read(enrollment_directory(&root, &binding).join("future.store")).unwrap(),
            b"preserve"
        );

        assert!(matches!(
            validate_json_bounds(&vec![b' '; MAX_ENROLLMENT_RECORD_BYTES + 1]),
            Err(EnrollmentError::RecordTooLarge(_))
        ));
        let deep = format!("{}0{}", "[".repeat(17), "]".repeat(17));
        assert_eq!(
            validate_json_bounds(deep.as_bytes()),
            Err(EnrollmentError::JsonDepthExceeded)
        );
        let tokens = format!("[{}0]", "0,".repeat(MAX_ENROLLMENT_JSON_TOKENS + 1));
        assert_eq!(
            validate_json_bounds(tokens.as_bytes()),
            Err(EnrollmentError::JsonTokenBoundExceeded)
        );
    }

    #[test]
    fn namespace_and_chain_audits_stop_at_hard_bounds() {
        let root = TestRoot::new("namespace-bound");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        for index in 0..MAX_ENROLLMENT_NAMESPACE_ENTRIES {
            let name = format!(
                "{}{RECORD_SUFFIX}",
                ContentDigest::of(&(index as u64).to_be_bytes())
            );
            let path = records.join(name);
            if !path.exists() {
                fs::write(path, b"orphan diagnostic").unwrap();
            }
        }
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::NamespaceBoundExceeded)
        ));

        let root = TestRoot::new("chain-bound");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let mut snapshot = writer.current().clone();
        drop(writer);
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        for index in 0..MAX_ENROLLMENT_CHAIN_RECORDS {
            let lifecycle = match index {
                0 => EnrollmentLifecycleV1::VerifiedLocal(verified()),
                1 => unsafe_idle(200),
                _ if index % 2 == 0 => safe_idle(),
                _ => unsafe_idle(200 + index as u128),
            };
            let record = EnrollmentRecordV1::successor(&snapshot, lifecycle).unwrap();
            let bytes = canonical_record_bytes(&record).unwrap();
            let digest = ContentDigest::of(&bytes);
            fs::write(records.join(format!("{digest}{RECORD_SUFFIX}")), bytes).unwrap();
            snapshot = EnrollmentSnapshot { digest, record };
        }
        write_head(&root, &binding, snapshot.digest);
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::ChainBoundExceeded)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_unsupported_record_artifacts_are_rejected_and_preserved() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("symlink");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let head = enrollment_directory(&root, &binding).join(HEAD_FILE);
        fs::remove_file(&head).unwrap();
        symlink("/dev/null", &head).unwrap();
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::UnsupportedArtifact(name)) if name == HEAD_FILE
        ));
        assert!(fs::symlink_metadata(&head)
            .unwrap()
            .file_type()
            .is_symlink());

        fs::remove_file(&head).unwrap();
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        let unsupported = records.join("future.record");
        fs::write(&unsupported, b"preserve").unwrap();
        assert!(matches!(
            EnrollmentReader::open_existing(&root.app(), &binding),
            Err(EnrollmentError::UnsupportedArtifact(name)) if name == "future.record"
        ));
        assert_eq!(fs::read(unsupported).unwrap(), b"preserve");
    }

    #[test]
    fn simultaneous_writable_handles_and_processes_contend_cleanly() {
        let root = TestRoot::new("lease");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        assert!(matches!(
            EnrollmentWriter::open_existing(&root.app(), &binding),
            Err(EnrollmentError::LeaseContended(_))
        ));

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "oplog::enrollment::tests::lease_child_probe",
                "--nocapture",
            ])
            .env("TINE_ENROLLMENT_LEASE_CHILD_ROOT", &root.path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        drop(writer);
        assert!(matches!(
            EnrollmentWriter::open_existing(&root.app(), &binding).unwrap(),
            EnrollmentOpen::Present(_)
        ));
    }

    #[test]
    fn lease_child_probe() {
        let Some(path) = std::env::var_os("TINE_ENROLLMENT_LEASE_CHILD_ROOT") else {
            return;
        };
        let app = EnrollmentApplicationRoot::open_for_harness(Path::new(&path)).unwrap();
        assert!(matches!(
            EnrollmentWriter::open_existing(&app, &test_binding()),
            Err(EnrollmentError::LeaseContended(_))
        ));
    }

    #[test]
    fn archive_identity_is_persistable_domain_separated_and_detects_substitution() {
        let root = TestRoot::new("archive-identity");
        let first_path = root.path.join("archive-a");
        let second_path = root.path.join("archive-b");
        fs::create_dir_all(&first_path).unwrap();
        fs::create_dir_all(&second_path).unwrap();
        let first = Dir::open_ambient_dir(&first_path, ambient_authority()).unwrap();
        let reopened = Dir::open_ambient_dir(&first_path, ambient_authority()).unwrap();
        let second = Dir::open_ambient_dir(&second_path, ambient_authority()).unwrap();

        let first_id = CanonicalArchiveResourceId::from_retained_directory(&first).unwrap();
        assert_eq!(
            first_id,
            CanonicalArchiveResourceId::from_retained_directory(&reopened).unwrap()
        );
        assert_ne!(
            first_id,
            CanonicalArchiveResourceId::from_retained_directory(&second).unwrap()
        );
        #[cfg(unix)]
        {
            let metadata = first
                .try_clone()
                .unwrap()
                .into_std_file()
                .metadata()
                .unwrap();
            let mut identity = [0_u8; 16];
            identity[..8].copy_from_slice(&metadata.dev().to_be_bytes());
            identity[8..].copy_from_slice(&metadata.ino().to_be_bytes());
            assert_ne!(
                first_id.as_bytes(),
                CanonicalGraphResourceId::from_capability_identity(b"unix-dev-inode", &identity)
                    .as_bytes()
            );
        }
        assert_eq!(
            first_id
                .to_string()
                .parse::<CanonicalArchiveResourceId>()
                .unwrap(),
            first_id
        );
    }

    #[test]
    fn published_packet_validation_and_shared_future_rejection_never_activate() {
        let import = ImportId::from_digest([50; 32]);
        assert_eq!(
            PublishedRecoveryPacketV1::new(
                BatchId::from_uuid(Uuid::from_u128(51)),
                import,
                digest(52),
                test_binding().archive_resource_id,
                anchor(53),
            ),
            Err(EnrollmentError::PublishedBatchMismatch)
        );
        let mut future_packet = packet(test_binding().archive_resource_id);
        future_packet.packet_schema_version = PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION + 1;
        assert_eq!(
            future_packet.validate(),
            Err(EnrollmentError::UnsupportedPacketSchema(
                PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION + 1
            ))
        );

        let wrong_archive = published(60, archive_resource(61));
        let record = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 2,
            previous: Some(digest(62)),
            binding: test_binding(),
            lifecycle: wrong_archive,
        };
        assert!(matches!(
            record.validate(),
            Err(EnrollmentError::BindingMismatch(
                EnrollmentBindingField::ArchiveResource
            ))
        ));

        let canonical =
            canonical_record_bytes(&EnrollmentRecordV1::initial(test_binding(), shadow()).unwrap())
                .unwrap();
        for state in [
            "shared_active",
            "joining",
            "share_prepared",
            "future_active",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
            value["lifecycle"]["state"] = serde_json::json!(state);
            assert_eq!(
                decode_record(&serde_json::to_vec(&value).unwrap()),
                Err(EnrollmentError::FutureUnsupportedLifecycle(state.into()))
            );
        }
    }

    #[test]
    fn enrollment_store_never_mutates_graph_bytes() {
        let root = TestRoot::new("no-graph-write");
        let graph = root.path.join("graph");
        fs::create_dir_all(&graph).unwrap();
        let page = graph.join("page.md");
        fs::write(&page, b"original graph bytes\n").unwrap();
        let before = fs::read(&page).unwrap();

        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding, shadow()).unwrap();
        let head = writer.current().digest;
        writer
            .transition(head, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap();
        assert_eq!(fs::read(page).unwrap(), before);
    }
}
