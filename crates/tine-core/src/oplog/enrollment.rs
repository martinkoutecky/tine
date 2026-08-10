//! Authoritative device-local enrollment lifecycle journal.
//!
//! Enrollment records only persisted state. It deliberately exposes no graph
//! writer or projection authorization. Content addressing, retained
//! capabilities, no-follow opens, exact file identities, and an OS lease
//! reject corruption, accidental substitution, and cooperating-process
//! split-brain. Versioned integrity checkpoints bind bounded immutable history
//! to the exact authority identity, lease, binding, and lifecycle state, so
//! arbitrary record bytes cannot summarize an unvalidated prefix. Legacy v1
//! authorities retain their frozen verifier only for reopening old histories;
//! current records make a corruption-detection claim, not a secret-holder
//! security claim. Filesystem and directory-sync guarantees remain
//! platform/filesystem dependent.
//! Windows authoritative handles reject reparse points after open; writable
//! lease handles additionally deny delete/replacement sharing.
//!
//! Writable callers retain [`EnrollmentLease`] for their whole session. The
//! required global lock order is: enrollment lease, archive/engine lease, then
//! graph and process-local locks.

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
#[cfg(windows)]
use cap_std::fs::OpenOptions;
#[cfg(windows)]
use cap_std::fs::{MetadataExt as _, OpenOptionsExt as _};
use cap_std::{ambient_authority, fs::Dir};
use crc32fast::hash as crc32;
use fs2::FileExt as _;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
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
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::identity::parse_digest;
use super::import::{
    reopen_inactive_bootstrap_accepted_authority, BootstrapStreamingImportError,
    InactiveBootstrapAcceptedAuthority, InactiveBootstrapAcceptedAuthorityBinding,
    InactiveBootstrapPreparedPublication, InactiveBootstrapVerifiedPublication,
};
use super::legacy_enrollment_verifier::{
    self as legacy_checkpoint, LegacyAuthorityClaimV1 as EnrollmentAuthorityClaimV1,
};
use super::migration_backup::{
    verify_migration_source_backup, MigrationBackupError, MigrationBackupRoot, VerifiedSourceBackup,
};
use super::object_store::{
    ensure_directory_nofollow, open_dir_nofollow, publish_immutable_exact, sync_dir_required,
};
use super::shadow_projection::{
    verify_inactive_bootstrap_shadow_projection, ShadowProjectionError, VerifiedShadowProjection,
};
use super::sqlite::{
    OpenProjection, ProjectionError, VerifiedBootstrapSqliteProjection, WorkspaceRuntimeProof,
};
use super::{
    BatchId, BlobDescription, CanonicalArchiveResourceId, CanonicalGraphResourceId, ContentDigest,
    DeviceId, DocumentId, GraphTextScopeBinding, ImportId, LineageDigest, ObjectStore,
    ProjectionEndpointId, ProjectionReceiptStoreId, SessionId, WorkspaceId, DIFF_SCHEMA_VERSION,
    MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION,
    OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION, PROJECTION_POLICY_VERSION,
    PROJECTION_SCHEMA_VERSION, RECEIPT_SCHEMA_VERSION,
};
use crate::model::Graph;

pub(crate) const ENROLLMENT_RECORD_SCHEMA_VERSION: u32 = 6;
pub(crate) const PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION: u32 = 1;
pub(crate) const SHARED_ENROLLMENT_DESCRIPTOR_SCHEMA_VERSION: u32 = 1;
pub(crate) const JOINER_WORKSPACE_ARCHIVE_SCHEMA_VERSION: u32 = 1;
pub(crate) const LOCAL_ACTIVATION_RESERVATION_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_ENROLLMENT_RECORD_BYTES: usize = 32 * 1024;
pub(crate) const MAX_ENROLLMENT_JSON_DEPTH: usize = 16;
/// All lifecycle records remain bounded and read with a single fixed parser
/// budget.  Shared enrollment carries one exact descriptor plus local archive
/// proof, so its authenticated record is intentionally larger than the legacy
/// LocalActive handoff record while still far below the 32 KiB byte ceiling.
pub(crate) const MAX_ENROLLMENT_JSON_TOKENS: usize = 768;
pub(crate) const MAX_ENROLLMENT_OPEN_CHAIN_RECORDS: usize = 64;
pub(crate) const MAX_ENROLLMENT_AUDIT_PAGE: usize = 64;
pub(crate) const MAX_ENROLLMENT_NAMESPACE_ENTRIES: usize = 2048;
pub(crate) const MAX_BLOCKED_REASON_CODE_BYTES: usize = 64;

const SPARSE_STORAGE_DIRECTORY: &str = "sparse-storage";
const STORAGE_VERSION_DIRECTORY: &str = "v2";
const LOCAL_DIRECTORY: &str = "local";
const ENROLLMENT_DIRECTORY: &str = "enrollment";
const RECORDS_DIRECTORY: &str = "records";
const LEASE_FILE: &str = "lease";
const AUTHORITY_FILE: &str = "authority-v1.claim";
const HEAD_FILE: &str = "head";
const RECORD_SUFFIX: &str = ".enrollment";
const HEAD_BYTES: usize = 65;
const HEAD_TEMP_PREFIX: &str = ".head-tmp-";
const RECORD_TEMP_PREFIX: &str = ".record-tmp-";
const AUTHORITY_TEMP_PREFIX: &str = ".authority-tmp-";
const LOCAL_ACTIVATION_RESERVATION_FILE: &str = "local-activation-v1.reservation";
const MAX_LOCAL_ACTIVATION_RESERVATION_BYTES: usize = 4 * 1024;
const ENROLLMENT_AUTHORITY_SCHEMA_V1: u32 = 1;
const ENROLLMENT_AUTHORITY_SCHEMA_VERSION: u32 = 2;
const ENROLLMENT_RECORD_SCHEMA_V5: u32 = 5;
const ENROLLMENT_CHECKPOINT_SCHEMA_V2: u32 = 2;
const ENROLLMENT_CHECKPOINT_SCHEMA_VERSION: u32 = 3;
const MAX_ENROLLMENT_AUTHORITY_BYTES: usize = 4 * 1024;

#[cfg(test)]
thread_local! {
    static ENROLLMENT_RECORD_READS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static ENROLLMENT_HEAD_READS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static ENROLLMENT_NAMESPACE_SCANS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static ENROLLMENT_DIRECTORY_OPENS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static ENROLLMENT_LEASE_ACQUISITIONS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static ENROLLMENT_AUTHORITY_CLAIM_READS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FAIL_NEXT_ENROLLMENT_HEAD_READ: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
pub(crate) fn fail_next_enrollment_head_read() {
    FAIL_NEXT_ENROLLMENT_HEAD_READ.with(|fault| fault.set(true));
}

/// Exact causal accounting for the enrollment journal's filesystem work.
///
/// Every field is an operation count, never a duration, so a bounded-admission
/// assertion is deterministic and machine independent.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EnrollmentInstrumentation {
    /// Authenticated content-addressed record reads. This is the record-chain
    /// walk a per-mutation admission must never perform.
    pub(crate) record_reads: usize,
    /// Reads of the tiny fixed-size committed head file.
    pub(crate) head_reads: usize,
    /// Enrollment namespace enumerations.
    pub(crate) namespace_scans: usize,
    /// Enrollment directory-tree opens.
    pub(crate) directory_opens: usize,
    /// OS enrollment-lease acquisitions.
    pub(crate) lease_acquisitions: usize,
    /// Authority-claim file reads.
    pub(crate) authority_claim_reads: usize,
}

#[cfg(test)]
impl EnrollmentInstrumentation {
    pub(crate) fn capture() -> Self {
        Self {
            record_reads: ENROLLMENT_RECORD_READS.with(std::cell::Cell::get),
            head_reads: ENROLLMENT_HEAD_READS.with(std::cell::Cell::get),
            namespace_scans: ENROLLMENT_NAMESPACE_SCANS.with(std::cell::Cell::get),
            directory_opens: ENROLLMENT_DIRECTORY_OPENS.with(std::cell::Cell::get),
            lease_acquisitions: ENROLLMENT_LEASE_ACQUISITIONS.with(std::cell::Cell::get),
            authority_claim_reads: ENROLLMENT_AUTHORITY_CLAIM_READS.with(std::cell::Cell::get),
        }
    }

    /// The work performed since `self` was captured.
    pub(crate) fn since(self) -> Self {
        let now = Self::capture();
        Self {
            record_reads: now.record_reads - self.record_reads,
            head_reads: now.head_reads - self.head_reads,
            namespace_scans: now.namespace_scans - self.namespace_scans,
            directory_opens: now.directory_opens - self.directory_opens,
            lease_acquisitions: now.lease_acquisitions - self.lease_acquisitions,
            authority_claim_reads: now.authority_claim_reads - self.authority_claim_reads,
        }
    }
}

#[cfg(test)]
fn count(counter: &'static std::thread::LocalKey<std::cell::Cell<usize>>) {
    counter.with(|value| value.set(value.get().saturating_add(1)));
}

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

    #[cfg(test)]
    fn open_for_harness(path: &Path) -> Result<Self, EnrollmentError> {
        prepare_application_root(path)
    }

    /// Open an explicitly supplied, already caller-bound private application
    /// root for the one-shot local activation path.  The public activation
    /// facade validates that this path is outside the graph before reaching
    /// this constructor; this layer retains the no-follow/private-directory
    /// checks at the filesystem boundary.
    pub(crate) fn open_explicit_private(path: &Path) -> Result<Self, EnrollmentError> {
        prepare_application_root(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Retain one already-existing enrollment application root without creating
/// or repairing any namespace below it.
///
/// Runtime discovery has already classified this path, but the actor uses this
/// fresh capability so no advisory path value itself becomes writer authority.
pub(crate) fn open_existing_enrollment_application_root(
    path: &Path,
) -> Result<EnrollmentApplicationRoot, EnrollmentError> {
    open_existing_application_root(path)?.ok_or_else(|| {
        EnrollmentError::UnsafeNamespace(
            "discovered enrollment application root no longer exists".into(),
        )
    })
}

#[cfg(test)]
pub(crate) fn enrollment_application_root_for_test(
    path: &Path,
) -> Result<EnrollmentApplicationRoot, EnrollmentError> {
    EnrollmentApplicationRoot::open_for_harness(path)
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

    pub(crate) const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }
}

/// Exact caller identities fixed before any graph-local archive namespace is
/// opened. This private reservation identity is deliberately stricter than the
/// eventual runtime binding: an honest crash resume must use the same
/// preparation and activation session as the call that first reserved it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalActivationIdentityV1 {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    endpoint_id: ProjectionEndpointId,
    device_id: DeviceId,
    graph_resource_id: CanonicalGraphResourceId,
    preparation_id: PreparationId,
    session_id: SessionId,
}

impl LocalActivationIdentityV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
        endpoint_id: ProjectionEndpointId,
        device_id: DeviceId,
        graph_resource_id: CanonicalGraphResourceId,
        preparation_id: PreparationId,
        session_id: SessionId,
    ) -> Self {
        Self {
            workspace_id,
            lineage_digest,
            catalog_document_id,
            endpoint_id,
            device_id,
            graph_resource_id,
            preparation_id,
            session_id,
        }
    }
}

/// Complete private pre-enrollment reservation binding. Receipt-store,
/// graph-scope, and source-inventory evidence are freshly derived before this
/// is published, so archive construction never relies on a pathname-only
/// assertion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalActivationReservationBindingV1 {
    identity: LocalActivationIdentityV1,
    receipt_store_id: ProjectionReceiptStoreId,
    graph_text_scope_binding: GraphTextScopeBinding,
    source_inventory_digest: ContentDigest,
}

impl LocalActivationReservationBindingV1 {
    pub(crate) const fn new(
        identity: LocalActivationIdentityV1,
        receipt_store_id: ProjectionReceiptStoreId,
        graph_text_scope_binding: GraphTextScopeBinding,
        source_inventory_digest: ContentDigest,
    ) -> Self {
        Self {
            identity,
            receipt_store_id,
            graph_text_scope_binding,
            source_inventory_digest,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalActivationReservationV1 {
    schema_version: u32,
    binding: LocalActivationReservationBindingV1,
    archive_instance_id: Uuid,
}

/// Authenticated-by-private-root, bounded evidence that makes an archive
/// construction crash explicitly resumable before ShadowImport exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalActivationReservation {
    record: LocalActivationReservationV1,
}

impl LocalActivationReservation {
    pub(crate) const fn identity(&self) -> &LocalActivationIdentityV1 {
        &self.record.binding.identity
    }

    pub(crate) const fn archive_instance_id(&self) -> Uuid {
        self.record.archive_instance_id
    }
}

/// Read one existing private reservation without creating the application
/// root, enrollment namespace, archive, or writer lease.
pub(crate) fn inspect_local_activation_reservation_at(
    root_path: &Path,
) -> Result<Option<LocalActivationReservation>, EnrollmentError> {
    let Some(root) = open_existing_application_root(root_path)? else {
        return Ok(None);
    };
    open_local_activation_reservation(&root)
}

/// Publish or resume the exact private reservation before archive creation.
/// Immutable publication is head-last/no-replace; any abandoned temp remains
/// outside the graph and grants no authority.
pub(crate) fn begin_or_resume_local_activation_reservation(
    root: &EnrollmentApplicationRoot,
    binding: LocalActivationReservationBindingV1,
) -> Result<LocalActivationReservation, EnrollmentError> {
    if let Some(existing) = open_local_activation_reservation(root)? {
        if existing.record.binding != binding {
            return Err(EnrollmentError::LocalActivationReservationMismatch);
        }
        return Ok(existing);
    }
    let record = LocalActivationReservationV1 {
        schema_version: LOCAL_ACTIVATION_RESERVATION_SCHEMA_VERSION,
        binding,
        archive_instance_id: Uuid::new_v4(),
    };
    let bytes =
        serde_json::to_vec(&record).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    if bytes.len() > MAX_LOCAL_ACTIVATION_RESERVATION_BYTES {
        return Err(EnrollmentError::LocalActivationReservationTooLarge(
            bytes.len(),
        ));
    }
    let directory = Dir::open_ambient_dir(root.path(), ambient_authority())?;
    publish_immutable_exact(
        &directory,
        LOCAL_ACTIVATION_RESERVATION_FILE,
        &bytes,
        "local activation reservation",
    )
    .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    let published = open_local_activation_reservation(root)?.ok_or_else(|| {
        EnrollmentError::Io("published local activation reservation is absent".into())
    })?;
    if published.record != record {
        return Err(EnrollmentError::LocalActivationReservationMismatch);
    }
    Ok(published)
}

fn open_local_activation_reservation(
    root: &EnrollmentApplicationRoot,
) -> Result<Option<LocalActivationReservation>, EnrollmentError> {
    let directory = Dir::open_ambient_dir(root.path(), ambient_authority())?;
    match directory.symlink_metadata(LOCAL_ACTIVATION_RESERVATION_FILE) {
        Ok(metadata) if !cap_metadata_is_authoritative_file(&metadata) => {
            return Err(EnrollmentError::UnsafeNamespace(
                "local activation reservation is not a regular no-follow file".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let (bytes, _) = read_bounded_authoritative_file(
        &directory,
        LOCAL_ACTIVATION_RESERVATION_FILE,
        MAX_LOCAL_ACTIVATION_RESERVATION_BYTES,
        "local activation reservation",
        true,
    )?;
    let record: LocalActivationReservationV1 = serde_json::from_slice(&bytes)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
    if record.schema_version != LOCAL_ACTIVATION_RESERVATION_SCHEMA_VERSION {
        return Err(
            EnrollmentError::UnsupportedLocalActivationReservationSchema(record.schema_version),
        );
    }
    let canonical =
        serde_json::to_vec(&record).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    if canonical != bytes {
        return Err(EnrollmentError::NonCanonicalLocalActivationReservation);
    }
    Ok(Some(LocalActivationReservation { record }))
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

    /// Canonical digest of this exact enrollment binding.
    ///
    /// Device-local runtime metadata records this instead of copying the
    /// binding, so a promoted archive cannot be adopted by an enrollment that
    /// differs in any field.
    pub(crate) fn binding_digest(&self) -> Result<ContentDigest, EnrollmentError> {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/enrollment-binding-digest/v1\0");
        hasher.update(
            serde_json::to_vec(self).map_err(|error| EnrollmentError::Encode(error.to_string()))?,
        );
        Ok(ContentDigest::from_bytes(hasher.finalize().into()))
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

/// The current claim deliberately keeps the historical filename: its exact
/// filesystem identity is bound into the lease protocol.  The schema, rather
/// than the pathname, determines whether a legacy verifier is available.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentAuthorityClaimV2 {
    schema_version: u32,
    authority_id: Uuid,
    lease_resource_id: ContentDigest,
    binding: EnrollmentBindingV1,
    initial_preparation_id: PreparationId,
    initial_source_inventory_digest: ContentDigest,
}

#[derive(Clone)]
enum EnrollmentAuthorityClaim {
    LegacyV1(EnrollmentAuthorityClaimV1),
    CurrentV2(EnrollmentAuthorityClaimV2),
}

impl EnrollmentAuthorityClaim {
    const fn schema_version(&self) -> u32 {
        match self {
            Self::LegacyV1(claim) => claim.schema_version,
            Self::CurrentV2(claim) => claim.schema_version,
        }
    }

    const fn authority_id(&self) -> Uuid {
        match self {
            Self::LegacyV1(claim) => claim.authority_id,
            Self::CurrentV2(claim) => claim.authority_id,
        }
    }

    const fn lease_resource_id(&self) -> ContentDigest {
        match self {
            Self::LegacyV1(claim) => claim.lease_resource_id,
            Self::CurrentV2(claim) => claim.lease_resource_id,
        }
    }

    fn binding(&self) -> &EnrollmentBindingV1 {
        match self {
            Self::LegacyV1(claim) => &claim.binding,
            Self::CurrentV2(claim) => &claim.binding,
        }
    }

    const fn initial_preparation_id(&self) -> PreparationId {
        match self {
            Self::LegacyV1(claim) => claim.initial_preparation_id,
            Self::CurrentV2(claim) => claim.initial_preparation_id,
        }
    }

    const fn initial_source_inventory_digest(&self) -> ContentDigest {
        match self {
            Self::LegacyV1(claim) => claim.initial_source_inventory_digest,
            Self::CurrentV2(claim) => claim.initial_source_inventory_digest,
        }
    }

    fn validate_initial_intent(&self, shadow: &ShadowImportV1) -> Result<(), EnrollmentError> {
        if self.initial_preparation_id() != shadow.preparation_id
            || self.initial_source_inventory_digest() != shadow.source_inventory_digest
        {
            return Err(EnrollmentError::InitialPreparationMismatch);
        }
        Ok(())
    }

    fn legacy_key(&self) -> Option<&[u8; legacy_checkpoint::LEGACY_AUTHORITY_KEY_BYTES]> {
        match self {
            Self::LegacyV1(claim) => Some(claim.legacy_key()),
            Self::CurrentV2(_) => None,
        }
    }
}

struct EnrollmentAuthorityMaterial {
    claim: EnrollmentAuthorityClaim,
    resource_id: ContentDigest,
}

impl EnrollmentAuthorityMaterial {
    fn from_claim(
        claim: EnrollmentAuthorityClaim,
        resource_id: ContentDigest,
        expected_binding: &EnrollmentBindingV1,
        expected_lease_resource_id: ContentDigest,
    ) -> Result<Self, EnrollmentError> {
        if !matches!(
            claim.schema_version(),
            ENROLLMENT_AUTHORITY_SCHEMA_V1 | ENROLLMENT_AUTHORITY_SCHEMA_VERSION
        ) {
            return Err(EnrollmentError::UnsupportedAuthoritySchema(
                claim.schema_version(),
            ));
        }
        claim.binding().validate_exact(expected_binding)?;
        if claim.lease_resource_id() != expected_lease_resource_id {
            return Err(EnrollmentError::LeaseResourceMismatch);
        }
        Ok(Self { claim, resource_id })
    }

    fn checkpoint_for(
        &self,
        generation: u64,
        previous: Option<ContentDigest>,
        history_accumulator: ContentDigest,
        lease_resource_id: ContentDigest,
        binding: &EnrollmentBindingV1,
        lifecycle: &EnrollmentLifecycleV1,
    ) -> Result<EnrollmentCheckpoint, EnrollmentError> {
        let message = current_checkpoint_message_bytes(
            self.claim.authority_id(),
            self.resource_id,
            generation,
            previous,
            history_accumulator,
            lease_resource_id,
            binding,
            lifecycle,
        )?;
        Ok(EnrollmentCheckpoint::CurrentV3(IntegrityCheckpointV3 {
            schema_version: ENROLLMENT_CHECKPOINT_SCHEMA_VERSION,
            authority_id: self.claim.authority_id(),
            authority_resource_id: self.resource_id,
            integrity_tag: crc32(&message),
        }))
    }

    fn verify_checkpoint(&self, record: &EnrollmentRecordV1) -> Result<(), EnrollmentError> {
        let checkpoint = record
            .checkpoint
            .as_ref()
            .ok_or(EnrollmentError::MissingAuthenticatedCheckpoint)?;
        match (record.schema_version, checkpoint) {
            (ENROLLMENT_RECORD_SCHEMA_V5, EnrollmentCheckpoint::LegacyV2(checkpoint)) => {
                let Some(key) = self.claim.legacy_key() else {
                    return Err(EnrollmentError::IllegalCheckpointPair);
                };
                if checkpoint.schema_version != ENROLLMENT_CHECKPOINT_SCHEMA_V2 {
                    return Err(EnrollmentError::UnsupportedCheckpointSchema(
                        checkpoint.schema_version,
                    ));
                }
                if checkpoint.authority_id != self.claim.authority_id()
                    || checkpoint.authority_resource_id != self.resource_id
                {
                    return Err(EnrollmentError::AuthorityMismatch);
                }
                let message = legacy_checkpoint_message_bytes(
                    checkpoint.authority_id,
                    checkpoint.authority_resource_id,
                    record.generation,
                    record.previous,
                    record.history_accumulator,
                    record.lease_resource_id,
                    &record.binding,
                    &record.lifecycle,
                )?;
                if legacy_checkpoint::verify(key, &message, checkpoint.authentication_tag) {
                    Ok(())
                } else {
                    Err(EnrollmentError::CheckpointLegacyAuthenticationFailed)
                }
            }
            (ENROLLMENT_RECORD_SCHEMA_VERSION, EnrollmentCheckpoint::CurrentV3(checkpoint)) => {
                if checkpoint.schema_version != ENROLLMENT_CHECKPOINT_SCHEMA_VERSION {
                    return Err(EnrollmentError::UnsupportedCheckpointSchema(
                        checkpoint.schema_version,
                    ));
                }
                if checkpoint.authority_id != self.claim.authority_id()
                    || checkpoint.authority_resource_id != self.resource_id
                {
                    return Err(EnrollmentError::AuthorityMismatch);
                }
                let message = current_checkpoint_message_bytes(
                    checkpoint.authority_id,
                    checkpoint.authority_resource_id,
                    record.generation,
                    record.previous,
                    record.history_accumulator,
                    record.lease_resource_id,
                    &record.binding,
                    &record.lifecycle,
                )?;
                if crc32(&message) == checkpoint.integrity_tag {
                    Ok(())
                } else {
                    Err(EnrollmentError::CheckpointIntegrityFailed)
                }
            }
            _ => Err(EnrollmentError::IllegalCheckpointPair),
        }
    }

    #[cfg(test)]
    fn legacy_checkpoint_for_test(
        &self,
        generation: u64,
        previous: Option<ContentDigest>,
        history_accumulator: ContentDigest,
        lease_resource_id: ContentDigest,
        binding: &EnrollmentBindingV1,
        lifecycle: &EnrollmentLifecycleV1,
    ) -> Result<EnrollmentCheckpoint, EnrollmentError> {
        let key = self
            .claim
            .legacy_key()
            .ok_or(EnrollmentError::IllegalCheckpointPair)?;
        let message = legacy_checkpoint_message_bytes(
            self.claim.authority_id(),
            self.resource_id,
            generation,
            previous,
            history_accumulator,
            lease_resource_id,
            binding,
            lifecycle,
        )?;
        Ok(EnrollmentCheckpoint::LegacyV2(AuthenticatedCheckpointV1 {
            schema_version: ENROLLMENT_CHECKPOINT_SCHEMA_V2,
            authority_id: self.claim.authority_id(),
            authority_resource_id: self.resource_id,
            authentication_tag: legacy_checkpoint::sign_for_test(key, &message),
        }))
    }

    fn audit_cursor_tag(
        &self,
        head: ContentDigest,
        digest: ContentDigest,
        generation: u64,
        newer_digest: ContentDigest,
    ) -> u32 {
        crc32(&audit_cursor_message_bytes(
            self.claim.authority_id(),
            self.resource_id,
            head,
            digest,
            generation,
            newer_digest,
        ))
    }

    fn verify_audit_cursor(&self, cursor: &EnrollmentAuditCursor) -> Result<(), EnrollmentError> {
        if cursor.schema_version != 1
            || self.audit_cursor_tag(
                cursor.head,
                cursor.digest,
                cursor.generation,
                cursor.newer_digest,
            ) != cursor.integrity_tag
        {
            return Err(EnrollmentError::InvalidAuditCursor);
        }
        Ok(())
    }
}

fn audit_cursor_message_bytes(
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    head: ContentDigest,
    digest: ContentDigest,
    generation: u64,
    newer_digest: ContentDigest,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(4 + 16 + 32 * 4 + 8 + 48);
    message.extend_from_slice(b"tine/enrollment-audit-cursor-integrity/v1\0");
    message.extend_from_slice(&1_u32.to_be_bytes());
    message.extend_from_slice(authority_id.as_bytes());
    message.extend_from_slice(authority_resource_id.as_bytes());
    message.extend_from_slice(head.as_bytes());
    message.extend_from_slice(digest.as_bytes());
    message.extend_from_slice(&generation.to_be_bytes());
    message.extend_from_slice(newer_digest.as_bytes());
    message
}

struct EnrollmentAuthority {
    material: EnrollmentAuthorityMaterial,
    file: File,
    directory: Dir,
    identity: AuthoritativeFileIdentity,
}

impl EnrollmentAuthority {
    fn validate_current(&self) -> Result<(), EnrollmentError> {
        #[cfg(test)]
        count(&ENROLLMENT_AUTHORITY_CLAIM_READS);
        validate_authoritative_file(&self.file, "enrollment authority claim")?;
        if authoritative_file_identity(&self.file)? != self.identity {
            return Err(EnrollmentError::AuthorityMismatch);
        }
        let reopened = open_regular_readonly(&self.directory, AUTHORITY_FILE)
            .map_err(|_| EnrollmentError::AuthorityMismatch)?;
        validate_authoritative_file(&reopened, "enrollment authority claim")?;
        if authoritative_file_identity(&reopened)? != self.identity {
            return Err(EnrollmentError::AuthorityMismatch);
        }
        let expected = canonical_authority_claim_bytes(&self.material.claim)?;
        let mut bytes = Vec::with_capacity(expected.len());
        reopened
            .take((MAX_ENROLLMENT_AUTHORITY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes != expected {
            return Err(EnrollmentError::AuthorityMismatch);
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
struct AcceptedFrontierAnchorV1 {
    acceptance_sequence: u64,
    accepted_frontier_state_digest: ContentDigest,
    history_generation: u64,
    history_root: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShadowImportV1 {
    preparation_id: PreparationId,
    source_inventory_digest: ContentDigest,
}

impl ShadowImportV1 {
    pub(crate) const fn new(
        preparation_id: PreparationId,
        source_inventory_digest: ContentDigest,
    ) -> Self {
        Self {
            preparation_id,
            source_inventory_digest,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedLocalV1 {
    preparation_id: PreparationId,
    source_inventory_digest: ContentDigest,
    source_file_count: u64,
    source_chunk_count: u64,
    source_total_bytes: u64,
    backup_manifest: BlobDescription,
    backup_restore_proof: BlobDescription,
    backup_evidence_digest: ContentDigest,
    bootstrap_import_id: ContentDigest,
    bootstrap_part_count: u32,
    bootstrap_terminal_part_id: Option<ContentDigest>,
    bootstrap_batch_id: Option<BatchId>,
    accepted_frontier_anchor: AcceptedFrontierAnchorV1,
    accepted_history_record_count: u64,
    catalog_row_count: u64,
    sqlite_accepted_batch_count: u64,
    sqlite_semantic_projection_digest: ContentDigest,
    sqlite_materialized_row_digest: ContentDigest,
    staged_projection_manifest: BlobDescription,
    staged_projection_proof: BlobDescription,
    staged_file_count: u64,
    staged_total_bytes: u64,
    byte_compare_digest: ContentDigest,
    shadow_evidence_digest: ContentDigest,
    proof_binding_digest: ContentDigest,
}

impl VerifiedLocalV1 {
    fn validate_fields(&self) -> Result<(), EnrollmentError> {
        let part_count = u64::from(self.bootstrap_part_count);
        let zero = self.bootstrap_part_count == 0;
        if self.bootstrap_batch_id.is_none() != zero
            || self.bootstrap_terminal_part_id.is_none() != zero
            || (self.accepted_frontier_anchor.history_root
                == super::object_store::EngineHistoryStore::empty_root())
                != zero
            || self.accepted_frontier_anchor.acceptance_sequence != part_count
            || self.accepted_frontier_anchor.history_generation != part_count
            || self.accepted_history_record_count != part_count
            || self.sqlite_accepted_batch_count != part_count
            || (self.source_file_count == 0) != zero
            || (zero && (self.source_chunk_count != 0 || self.source_total_bytes != 0))
            || self.source_file_count != self.catalog_row_count
            || self.source_file_count != self.staged_file_count
            || self.source_total_bytes != self.staged_total_bytes
        {
            return Err(EnrollmentError::InvalidVerifiedLocalTerminal);
        }
        if self.bootstrap_batch_id
            == Some(BatchId::for_import(ImportId::from_digest(
                *self.bootstrap_import_id.as_bytes(),
            )))
        {
            return Err(EnrollmentError::InvalidVerifiedLocalTerminal);
        }
        Ok(())
    }

    fn verification_digest(&self) -> Result<ContentDigest, EnrollmentError> {
        self.validate_fields()?;
        let bytes =
            serde_json::to_vec(self).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
        Ok(ContentDigest::of(&bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum HandoffV1 {
    Safe,
    Unsafe { session_id: SessionId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedRecoveryPacketV1 {
    packet_schema_version: u32,
    batch_id: BatchId,
    import_id: ImportId,
    manifest_digest: ContentDigest,
    archive_resource_id: CanonicalArchiveResourceId,
    published_from: AcceptedFrontierAnchorV1,
}

impl PublishedRecoveryPacketV1 {
    fn new(
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
enum LocalExclusionV1 {
    Idle,
    Published { packet: PublishedRecoveryPacketV1 },
}

/// The immutable bootstrap anchor every `LocalActive` record carries.
///
/// It is derived exactly once, at the sole `VerifiedLocal -> LocalActive`
/// transition, from the committed predecessor record and that record's own
/// content digest; every later `LocalActive -> LocalActive` handoff must repeat
/// it byte-for-byte. It therefore holds the complete durable data needed to
/// reconstruct and revalidate the original `VerifiedLocal`/bootstrap anchor in
/// O(1) from the head record alone.
///
/// It is authenticated by exactly the mechanism that authenticates the head:
/// the anchor lives inside `lifecycle`, which the hash-linked record digest, the
/// history accumulator, and the periodic authority-keyed checkpoint all commit
/// to. A fresh reopen therefore needs only the existing bounded checkpoint/open
/// proof — never a backward search for the `VerifiedLocal` record, whose
/// distance from the head grows without bound over a graph's lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalActiveAnchorV1 {
    verified_local_record_digest: ContentDigest,
    preparation_id: PreparationId,
    bootstrap_import_id: ContentDigest,
    bootstrap_part_count: u32,
    bootstrap_batch_id: Option<BatchId>,
    accepted_history_record_count: u64,
    accepted_frontier_anchor: AcceptedFrontierAnchorV1,
}

impl LocalActiveAnchorV1 {
    /// Derive the anchor from the actual committed `VerifiedLocal` predecessor
    /// and the exact content digest of the record that carries it.
    const fn from_verified_local(
        verified: &VerifiedLocalV1,
        verified_local_record_digest: ContentDigest,
    ) -> Self {
        Self {
            verified_local_record_digest,
            preparation_id: verified.preparation_id,
            bootstrap_import_id: verified.bootstrap_import_id,
            bootstrap_part_count: verified.bootstrap_part_count,
            bootstrap_batch_id: verified.bootstrap_batch_id,
            accepted_history_record_count: verified.accepted_history_record_count,
            accepted_frontier_anchor: verified.accepted_frontier_anchor,
        }
    }

    /// The zero/nonzero/multipart bootstrap identity rules
    /// [`VerifiedLocalV1::validate_fields`] enforces, restated over exactly the
    /// fields the anchor retains, so an anchor is rejected on its own terms
    /// without reading the record it names.
    fn validate(&self) -> Result<(), EnrollmentError> {
        let part_count = u64::from(self.bootstrap_part_count);
        let zero = self.bootstrap_part_count == 0;
        if self.bootstrap_batch_id.is_none() != zero
            || (self.accepted_frontier_anchor.history_root
                == super::object_store::EngineHistoryStore::empty_root())
                != zero
            || self.accepted_frontier_anchor.acceptance_sequence != part_count
            || self.accepted_frontier_anchor.history_generation != part_count
            || self.accepted_history_record_count != part_count
        {
            return Err(EnrollmentError::InvalidLocalActiveAnchor);
        }
        if self.bootstrap_batch_id
            == Some(BatchId::for_import(ImportId::from_digest(
                *self.bootstrap_import_id.as_bytes(),
            )))
        {
            return Err(EnrollmentError::InvalidLocalActiveAnchor);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalActiveV1 {
    verification_digest: ContentDigest,
    anchor: LocalActiveAnchorV1,
    handoff: HandoffV1,
    exclusion: LocalExclusionV1,
}

/// Exact bootstrap and projection/base facts that two honest local enrollments
/// must share before they can enter one shared lineage.  This deliberately
/// excludes device-local paths and resource identities: those are separately
/// bound by each enrollment record and cannot be compared across devices.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedProjectionBaseEvidenceV1 {
    bootstrap_import_id: ContentDigest,
    bootstrap_part_count: u32,
    bootstrap_terminal_part_id: Option<ContentDigest>,
    staged_file_count: u64,
    staged_total_bytes: u64,
}

impl SharedProjectionBaseEvidenceV1 {
    fn from_verified_local(verified: &VerifiedLocalV1) -> Self {
        Self {
            bootstrap_import_id: verified.bootstrap_import_id,
            bootstrap_part_count: verified.bootstrap_part_count,
            bootstrap_terminal_part_id: verified.bootstrap_terminal_part_id,
            staged_file_count: verified.staged_file_count,
            staged_total_bytes: verified.staged_total_bytes,
        }
    }

    fn validate(&self) -> Result<(), EnrollmentError> {
        let zero = self.bootstrap_part_count == 0;
        if self.bootstrap_terminal_part_id.is_none() != zero {
            return Err(EnrollmentError::InvalidSharedProjectionBaseEvidence);
        }
        Ok(())
    }

    fn first_mismatch(&self, other: &Self) -> Option<&'static str> {
        if self.bootstrap_import_id != other.bootstrap_import_id {
            Some("bootstrap_import_id")
        } else if self.bootstrap_part_count != other.bootstrap_part_count {
            Some("bootstrap_part_count")
        } else if self.bootstrap_terminal_part_id != other.bootstrap_terminal_part_id {
            Some("bootstrap_terminal_part_id")
        } else if self.staged_file_count != other.staged_file_count {
            Some("staged_file_count")
        } else if self.staged_total_bytes != other.staged_total_bytes {
            Some("staged_total_bytes")
        } else {
            None
        }
    }
}

/// The one portable, commit-last enrollment descriptor an initiator may hand
/// to a peer.  Its digest is its identity; there is intentionally no mutable
/// descriptor registry or descriptor discovery scan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharedEnrollmentDescriptorV1 {
    schema_version: u32,
    compatibility: EnrollmentCompatibilityV1,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    initiator_graph_resource_id: CanonicalGraphResourceId,
    initiator_device_id: DeviceId,
    object_store_namespace: ContentDigest,
    initiator_local_active_head: ContentDigest,
    initiator_verification_digest: ContentDigest,
    initiator_handoff: HandoffV1,
    projection_base: SharedProjectionBaseEvidenceV1,
}

impl SharedEnrollmentDescriptorV1 {
    fn from_local_active(
        binding: &EnrollmentBindingV1,
        committed: &CommittedLocalActive,
        verified: &VerifiedLocalV1,
        object_store_namespace: ContentDigest,
    ) -> Result<Self, EnrollmentError> {
        let descriptor = Self {
            schema_version: SHARED_ENROLLMENT_DESCRIPTOR_SCHEMA_VERSION,
            compatibility: binding.compatibility,
            workspace_id: binding.workspace_id,
            lineage_digest: binding.lineage_digest,
            catalog_document_id: binding.catalog_document_id,
            initiator_graph_resource_id: binding.graph_resource_id,
            initiator_device_id: binding.device_id,
            object_store_namespace,
            initiator_local_active_head: committed.enrollment_head,
            initiator_verification_digest: committed.verification_digest,
            initiator_handoff: HandoffV1::Safe,
            projection_base: SharedProjectionBaseEvidenceV1::from_verified_local(verified),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub(crate) fn digest(&self) -> Result<ContentDigest, EnrollmentError> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
        Ok(ContentDigest::of(
            &[b"tine/shared-enrollment-descriptor/v1\0".as_slice(), &bytes].concat(),
        ))
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, EnrollmentError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| EnrollmentError::Encode(error.to_string()))
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, EnrollmentError> {
        if bytes.len() > MAX_ENROLLMENT_RECORD_BYTES {
            return Err(EnrollmentError::RecordTooLarge(bytes.len()));
        }
        validate_json_bounds(bytes)?;
        reject_duplicate_json_fields(bytes)?;
        let descriptor: Self = serde_json::from_slice(bytes)
            .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
        descriptor.validate()?;
        if descriptor.encode()? != bytes {
            return Err(EnrollmentError::NonCanonicalRecord);
        }
        Ok(descriptor)
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

    pub(crate) const fn object_store_namespace(&self) -> ContentDigest {
        self.object_store_namespace
    }

    fn validate(&self) -> Result<(), EnrollmentError> {
        if self.schema_version != SHARED_ENROLLMENT_DESCRIPTOR_SCHEMA_VERSION {
            return Err(
                EnrollmentError::UnsupportedSharedEnrollmentDescriptorSchema(self.schema_version),
            );
        }
        self.compatibility.validate_current()?;
        if !matches!(self.initiator_handoff, HandoffV1::Safe) {
            return Err(EnrollmentError::UnsafeSharedEnrollmentHandoff);
        }
        self.projection_base.validate()
    }

    fn is_compatible_with(&self, binding: &EnrollmentBindingV1) -> bool {
        self.workspace_id == binding.workspace_id
            && self.lineage_digest == binding.lineage_digest
            && self.catalog_document_id == binding.catalog_document_id
            && self.compatibility == binding.compatibility
    }

    pub(crate) fn provider_ingress(
        &self,
    ) -> Result<SharedProviderIngressAuthority, EnrollmentError> {
        Ok(SharedProviderIngressAuthority {
            workspace_id: self.workspace_id,
            lineage_digest: self.lineage_digest,
            descriptor_digest: self.digest()?,
        })
    }
}

/// Narrow authority for receiving the initiator's historical bootstrap
/// through the shared provider. It is minted only from a fully validated
/// descriptor and grants no enrollment or graph-write authority.
pub(crate) struct SharedProviderIngressAuthority {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    descriptor_digest: ContentDigest,
}

impl SharedProviderIngressAuthority {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn descriptor_digest(&self) -> ContentDigest {
        self.descriptor_digest
    }
}

/// Durable evidence that a joining device retired its former local workspace
/// only after proving it had no unique operation that was not projected.  The
/// archive bytes are retained by the caller's existing backup/archive path;
/// the journal stores the exact digest and pre-archive LocalActive witness.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinerWorkspaceArchiveV1 {
    schema_version: u32,
    archived_workspace_digest: ContentDigest,
    source_local_active_head: ContentDigest,
    source_verification_digest: ContentDigest,
    unique_unprojected_operation_count: u64,
    projection_base: SharedProjectionBaseEvidenceV1,
}

impl JoinerWorkspaceArchiveV1 {
    fn validate(&self) -> Result<(), EnrollmentError> {
        if self.schema_version != JOINER_WORKSPACE_ARCHIVE_SCHEMA_VERSION {
            return Err(EnrollmentError::UnsupportedJoinerWorkspaceArchiveSchema(
                self.schema_version,
            ));
        }
        if self.unique_unprojected_operation_count != 0 {
            return Err(EnrollmentError::DirtyUniqueLocalTail);
        }
        self.projection_base.validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SharedEnrollmentRoleV1 {
    Initiator,
    Joiner,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharePreparedV1 {
    descriptor: SharedEnrollmentDescriptorV1,
    descriptor_digest: ContentDigest,
    local_active: LocalActiveV1,
}

impl SharePreparedV1 {
    fn validate(&self, binding: &EnrollmentBindingV1) -> Result<(), EnrollmentError> {
        self.descriptor.validate()?;
        if !self.descriptor.is_compatible_with(binding)
            || self.descriptor.initiator_graph_resource_id != binding.graph_resource_id
            || self.descriptor.initiator_device_id != binding.device_id
        {
            return Err(EnrollmentError::SharedEnrollmentBindingMismatch);
        }
        if self.descriptor.digest()? != self.descriptor_digest {
            return Err(EnrollmentError::SharedEnrollmentDescriptorDigestMismatch);
        }
        self.local_active.validate_for_shared_runtime(binding)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JoiningV1 {
    descriptor: SharedEnrollmentDescriptorV1,
    descriptor_digest: ContentDigest,
    archived_local_workspace: JoinerWorkspaceArchiveV1,
    local_active: LocalActiveV1,
}

impl JoiningV1 {
    fn validate(&self, binding: &EnrollmentBindingV1) -> Result<(), EnrollmentError> {
        self.descriptor.validate()?;
        self.archived_local_workspace.validate()?;
        if !self.descriptor.is_compatible_with(binding)
            || self.descriptor.digest()? != self.descriptor_digest
            || self.archived_local_workspace.projection_base != self.descriptor.projection_base
        {
            return Err(EnrollmentError::SharedEnrollmentBindingMismatch);
        }
        self.local_active.validate_for_shared_runtime(binding)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedActiveV1 {
    descriptor: SharedEnrollmentDescriptorV1,
    descriptor_digest: ContentDigest,
    role: SharedEnrollmentRoleV1,
    archived_local_workspace: Option<JoinerWorkspaceArchiveV1>,
    local_active: LocalActiveV1,
}

impl SharedActiveV1 {
    fn validate(&self, binding: &EnrollmentBindingV1) -> Result<(), EnrollmentError> {
        self.descriptor.validate()?;
        if self.descriptor.digest()? != self.descriptor_digest
            || !self.descriptor.is_compatible_with(binding)
        {
            return Err(EnrollmentError::SharedEnrollmentBindingMismatch);
        }
        self.local_active.validate_for_shared_runtime(binding)?;
        match (self.role, &self.archived_local_workspace) {
            (SharedEnrollmentRoleV1::Initiator, None) => Ok(()),
            (SharedEnrollmentRoleV1::Joiner, Some(archive)) => {
                archive.validate()?;
                if archive.projection_base != self.descriptor.projection_base {
                    return Err(EnrollmentError::SharedEnrollmentBindingMismatch);
                }
                Ok(())
            }
            _ => Err(EnrollmentError::IllegalLifecycle(
                "shared enrollment role and joiner archive evidence disagree",
            )),
        }
    }
}

impl LocalActiveV1 {
    fn validate_for_shared_runtime(
        &self,
        binding: &EnrollmentBindingV1,
    ) -> Result<(), EnrollmentError> {
        self.anchor.validate()?;
        if matches!(self.handoff, HandoffV1::Safe)
            && matches!(self.exclusion, LocalExclusionV1::Published { .. })
        {
            return Err(EnrollmentError::IllegalLifecycle(
                "a shared published exclusion cannot be marked handoff-safe",
            ));
        }
        if let LocalExclusionV1::Published { packet } = &self.exclusion {
            packet.validate()?;
            if packet.archive_resource_id != binding.archive_resource_id {
                return Err(EnrollmentError::BindingMismatch(
                    EnrollmentBindingField::ArchiveResource,
                ));
            }
        }
        Ok(())
    }

    fn validate_for_shared_transition(&self) -> Result<(), EnrollmentError> {
        self.anchor.validate()?;
        if !matches!(self.handoff, HandoffV1::Safe)
            || !matches!(self.exclusion, LocalExclusionV1::Idle)
        {
            return Err(EnrollmentError::UnsafeSharedEnrollmentHandoff);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockedV1 {
    prior_record_digest: ContentDigest,
    reason_code: String,
    evidence_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum EnrollmentLifecycleV1 {
    ShadowImport(ShadowImportV1),
    VerifiedLocal(VerifiedLocalV1),
    LocalActive(LocalActiveV1),
    SharePrepared(SharePreparedV1),
    Joining(JoiningV1),
    SharedActive(SharedActiveV1),
    Blocked(BlockedV1),
}

impl EnrollmentLifecycleV1 {
    fn validate(
        &self,
        binding: &EnrollmentBindingV1,
        previous: Option<ContentDigest>,
    ) -> Result<(), EnrollmentError> {
        match self {
            Self::ShadowImport(_) => Ok(()),
            Self::VerifiedLocal(verified) => verified.validate_fields(),
            Self::LocalActive(active) => {
                active.anchor.validate()?;
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
            Self::SharePrepared(prepared) => prepared.validate(binding),
            Self::Joining(joining) => joining.validate(binding),
            Self::SharedActive(active) => active.validate(binding),
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
struct AuthenticatedCheckpointV1 {
    schema_version: u32,
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    authentication_tag: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegrityCheckpointV3 {
    schema_version: u32,
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    integrity_tag: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EnrollmentCheckpoint {
    LegacyV2(AuthenticatedCheckpointV1),
    CurrentV3(IntegrityCheckpointV3),
}

#[derive(Serialize)]
struct CheckpointMessageV1<'a> {
    domain: &'static str,
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: &'a EnrollmentBindingV1,
    lifecycle: &'a EnrollmentLifecycleV1,
}

fn legacy_checkpoint_message_bytes(
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: &EnrollmentBindingV1,
    lifecycle: &EnrollmentLifecycleV1,
) -> Result<Vec<u8>, EnrollmentError> {
    serde_json::to_vec(&CheckpointMessageV1 {
        domain: legacy_checkpoint::LEGACY_CHECKPOINT_DOMAIN,
        authority_id,
        authority_resource_id,
        generation,
        previous,
        history_accumulator,
        lease_resource_id,
        binding,
        lifecycle,
    })
    .map_err(|error| EnrollmentError::Encode(error.to_string()))
}

#[derive(Serialize)]
struct CheckpointMessageV3<'a> {
    domain: &'static str,
    record_schema_version: u32,
    checkpoint_schema_version: u32,
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: &'a EnrollmentBindingV1,
    lifecycle: &'a EnrollmentLifecycleV1,
}

fn current_checkpoint_message_bytes(
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: &EnrollmentBindingV1,
    lifecycle: &EnrollmentLifecycleV1,
) -> Result<Vec<u8>, EnrollmentError> {
    serde_json::to_vec(&CheckpointMessageV3 {
        domain: "tine/enrollment-checkpoint-integrity/v1",
        record_schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
        checkpoint_schema_version: ENROLLMENT_CHECKPOINT_SCHEMA_VERSION,
        authority_id,
        authority_resource_id,
        generation,
        previous,
        history_accumulator,
        lease_resource_id,
        binding,
        lifecycle,
    })
    .map_err(|error| EnrollmentError::Encode(error.to_string()))
}

const fn generation_requires_checkpoint(generation: u64) -> bool {
    generation > 0 && (generation - 1).is_multiple_of(MAX_ENROLLMENT_OPEN_CHAIN_RECORDS as u64)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EnrollmentRecordV1 {
    schema_version: u32,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: EnrollmentBindingV1,
    lifecycle: EnrollmentLifecycleV1,
    checkpoint: Option<EnrollmentCheckpoint>,
}

impl EnrollmentRecordV1 {
    fn initial(
        binding: EnrollmentBindingV1,
        shadow: ShadowImportV1,
        lease_resource_id: ContentDigest,
        authority: &EnrollmentAuthorityMaterial,
    ) -> Result<Self, EnrollmentError> {
        let lifecycle = EnrollmentLifecycleV1::ShadowImport(shadow);
        let history_accumulator = compute_history_accumulator(1, None, None, &binding, &lifecycle)?;
        let mut record = Self {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 1,
            previous: None,
            history_accumulator,
            lease_resource_id,
            binding,
            lifecycle,
            checkpoint: None,
        };
        record.checkpoint = Some(authority.checkpoint_for(
            record.generation,
            record.previous,
            record.history_accumulator,
            record.lease_resource_id,
            &record.binding,
            &record.lifecycle,
        )?);
        record.validate()?;
        Ok(record)
    }

    fn successor(
        current: &EnrollmentSnapshot,
        lifecycle: EnrollmentLifecycleV1,
        authority: &EnrollmentAuthorityMaterial,
    ) -> Result<Self, EnrollmentError> {
        validate_transition(&current.record.lifecycle, &lifecycle, current.digest)?;
        let generation = current
            .record
            .generation
            .checked_add(1)
            .ok_or(EnrollmentError::GenerationOverflow)?;
        let mut record = Self {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation,
            previous: Some(current.digest),
            history_accumulator: compute_history_accumulator(
                generation,
                Some(current.digest),
                Some(current.record.history_accumulator),
                &current.record.binding,
                &lifecycle,
            )?,
            lease_resource_id: current.record.lease_resource_id,
            binding: current.record.binding.clone(),
            lifecycle,
            checkpoint: None,
        };
        if generation_requires_checkpoint(generation) {
            record.checkpoint = Some(authority.checkpoint_for(
                record.generation,
                record.previous,
                record.history_accumulator,
                record.lease_resource_id,
                &record.binding,
                &record.lifecycle,
            )?);
        }
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), EnrollmentError> {
        if !matches!(
            self.schema_version,
            ENROLLMENT_RECORD_SCHEMA_V5 | ENROLLMENT_RECORD_SCHEMA_VERSION
        ) {
            return Err(EnrollmentError::UnsupportedRecordSchema(
                self.schema_version,
            ));
        }
        if self.generation == 0 || (self.generation == 1) != self.previous.is_none() {
            return Err(EnrollmentError::NonmonotonicGeneration);
        }
        if self.checkpoint.is_some() != generation_requires_checkpoint(self.generation) {
            return Err(EnrollmentError::MissingAuthenticatedCheckpoint);
        }
        if let Some(checkpoint) = &self.checkpoint {
            match (self.schema_version, checkpoint) {
                (ENROLLMENT_RECORD_SCHEMA_V5, EnrollmentCheckpoint::LegacyV2(checkpoint))
                    if checkpoint.schema_version == ENROLLMENT_CHECKPOINT_SCHEMA_V2 => {}
                (ENROLLMENT_RECORD_SCHEMA_VERSION, EnrollmentCheckpoint::CurrentV3(checkpoint))
                    if checkpoint.schema_version == ENROLLMENT_CHECKPOINT_SCHEMA_VERSION => {}
                (_, EnrollmentCheckpoint::LegacyV2(checkpoint))
                    if checkpoint.schema_version != ENROLLMENT_CHECKPOINT_SCHEMA_V2 =>
                {
                    return Err(EnrollmentError::UnsupportedCheckpointSchema(
                        checkpoint.schema_version,
                    ));
                }
                (_, EnrollmentCheckpoint::CurrentV3(checkpoint))
                    if checkpoint.schema_version != ENROLLMENT_CHECKPOINT_SCHEMA_VERSION =>
                {
                    return Err(EnrollmentError::UnsupportedCheckpointSchema(
                        checkpoint.schema_version,
                    ));
                }
                _ => return Err(EnrollmentError::IllegalCheckpointPair),
            }
        }
        self.binding.validate_internal()?;
        self.lifecycle.validate(&self.binding, self.previous)
    }

    const fn generation(&self) -> u64 {
        self.generation
    }

    const fn previous(&self) -> Option<ContentDigest> {
        self.previous
    }

    const fn binding(&self) -> &EnrollmentBindingV1 {
        &self.binding
    }

    const fn lifecycle(&self) -> &EnrollmentLifecycleV1 {
        &self.lifecycle
    }
}

fn compute_history_accumulator(
    generation: u64,
    previous: Option<ContentDigest>,
    previous_accumulator: Option<ContentDigest>,
    binding: &EnrollmentBindingV1,
    lifecycle: &EnrollmentLifecycleV1,
) -> Result<ContentDigest, EnrollmentError> {
    let binding_bytes =
        serde_json::to_vec(binding).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    let lifecycle_bytes = serde_json::to_vec(lifecycle)
        .map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"tine/enrollment-history-accumulator/v2\0");
    hasher.update(generation.to_be_bytes());
    match previous {
        Some(digest) => {
            hasher.update([1]);
            hasher.update(digest.as_bytes());
        }
        None => hasher.update([0]),
    }
    match previous_accumulator {
        Some(digest) => {
            hasher.update([1]);
            hasher.update(digest.as_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update((binding_bytes.len() as u64).to_be_bytes());
    hasher.update(binding_bytes);
    hasher.update((lifecycle_bytes.len() as u64).to_be_bytes());
    hasher.update(lifecycle_bytes);
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn validate_initial_record(record: &EnrollmentRecordV1) -> Result<(), EnrollmentError> {
    if record.generation != 1
        || record.previous.is_some()
        || !matches!(record.lifecycle, EnrollmentLifecycleV1::ShadowImport(_))
    {
        return Err(EnrollmentError::NonmonotonicGeneration);
    }
    let expected = compute_history_accumulator(1, None, None, &record.binding, &record.lifecycle)?;
    if record.history_accumulator != expected {
        return Err(EnrollmentError::HistoryAccumulatorMismatch);
    }
    Ok(())
}

fn validate_record_link(
    previous_digest: ContentDigest,
    previous: &EnrollmentRecordV1,
    current: &EnrollmentRecordV1,
) -> Result<(), EnrollmentError> {
    if current.previous != Some(previous_digest)
        || current.generation
            != previous
                .generation
                .checked_add(1)
                .ok_or(EnrollmentError::GenerationOverflow)?
    {
        return Err(EnrollmentError::NonmonotonicGeneration);
    }
    validate_transition(&previous.lifecycle, &current.lifecycle, previous_digest)?;
    let expected = compute_history_accumulator(
        current.generation,
        Some(previous_digest),
        Some(previous.history_accumulator),
        &current.binding,
        &current.lifecycle,
    )?;
    if current.history_accumulator != expected {
        return Err(EnrollmentError::HistoryAccumulatorMismatch);
    }
    Ok(())
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
            // The anchor is minted here and only here, from the actual
            // committed predecessor record and its exact content digest.
            verified
                .verification_digest()
                .is_ok_and(|digest| digest == active.verification_digest)
                && active.anchor
                    == LocalActiveAnchorV1::from_verified_local(verified, current_digest)
                && matches!(active.handoff, HandoffV1::Unsafe { .. })
                && matches!(active.exclusion, LocalExclusionV1::Idle)
        }
        (EnrollmentLifecycleV1::VerifiedLocal(_), EnrollmentLifecycleV1::Blocked(blocked)) => {
            blocked.prior_record_digest == current_digest
        }
        (EnrollmentLifecycleV1::LocalActive(current), EnrollmentLifecycleV1::LocalActive(next)) => {
            // The anchor is immutable for the whole `LocalActive` lifetime:
            // every handoff, session change, exclusion change and checkpoint
            // must repeat it exactly.
            current.verification_digest == next.verification_digest
                && current.anchor == next.anchor
                && legal_local_active_transition(current, next)
        }
        (EnrollmentLifecycleV1::LocalActive(_), EnrollmentLifecycleV1::Blocked(blocked)) => {
            blocked.prior_record_digest == current_digest
        }
        (
            EnrollmentLifecycleV1::LocalActive(current),
            EnrollmentLifecycleV1::SharePrepared(prepared),
        ) => {
            matches!(current.handoff, HandoffV1::Safe)
                && matches!(current.exclusion, LocalExclusionV1::Idle)
                && prepared.descriptor.initiator_verification_digest == current.verification_digest
                && prepared.local_active == *current
        }
        (EnrollmentLifecycleV1::LocalActive(current), EnrollmentLifecycleV1::Joining(joining)) => {
            matches!(current.handoff, HandoffV1::Safe)
                && matches!(current.exclusion, LocalExclusionV1::Idle)
                && joining.archived_local_workspace.source_local_active_head == current_digest
                && joining.archived_local_workspace.source_verification_digest
                    == current.verification_digest
                && joining.local_active == *current
        }
        (
            EnrollmentLifecycleV1::SharePrepared(prepared),
            EnrollmentLifecycleV1::SharedActive(active),
        ) => {
            active.role == SharedEnrollmentRoleV1::Initiator
                && active.archived_local_workspace.is_none()
                && active.descriptor == prepared.descriptor
                && active.descriptor_digest == prepared.descriptor_digest
                && active.local_active == prepared.local_active
        }
        (
            EnrollmentLifecycleV1::SharePrepared(current),
            EnrollmentLifecycleV1::SharePrepared(next),
        ) => {
            current.descriptor == next.descriptor
                && current.descriptor_digest == next.descriptor_digest
                && current.local_active.verification_digest == next.local_active.verification_digest
                && current.local_active.anchor == next.local_active.anchor
                && legal_local_active_transition(&current.local_active, &next.local_active)
        }
        (EnrollmentLifecycleV1::SharePrepared(_), EnrollmentLifecycleV1::Blocked(blocked))
        | (EnrollmentLifecycleV1::Joining(_), EnrollmentLifecycleV1::Blocked(blocked))
        | (EnrollmentLifecycleV1::SharedActive(_), EnrollmentLifecycleV1::Blocked(blocked)) => {
            blocked.prior_record_digest == current_digest
        }
        (EnrollmentLifecycleV1::Joining(joining), EnrollmentLifecycleV1::SharedActive(active)) => {
            active.role == SharedEnrollmentRoleV1::Joiner
                && active.descriptor == joining.descriptor
                && active.descriptor_digest == joining.descriptor_digest
                && active.archived_local_workspace.as_ref()
                    == Some(&joining.archived_local_workspace)
                && active.local_active == joining.local_active
        }
        (EnrollmentLifecycleV1::Joining(current), EnrollmentLifecycleV1::Joining(next)) => {
            current.descriptor == next.descriptor
                && current.descriptor_digest == next.descriptor_digest
                && current.archived_local_workspace == next.archived_local_workspace
                && current.local_active.verification_digest == next.local_active.verification_digest
                && current.local_active.anchor == next.local_active.anchor
                && legal_local_active_transition(&current.local_active, &next.local_active)
        }
        (
            EnrollmentLifecycleV1::SharedActive(current),
            EnrollmentLifecycleV1::SharedActive(next),
        ) => {
            current.descriptor == next.descriptor
                && current.descriptor_digest == next.descriptor_digest
                && current.role == next.role
                && current.archived_local_workspace == next.archived_local_workspace
                && current.local_active.verification_digest == next.local_active.verification_digest
                && current.local_active.anchor == next.local_active.anchor
                && legal_local_active_transition(&current.local_active, &next.local_active)
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
    digest: ContentDigest,
    record: EnrollmentRecordV1,
}

impl EnrollmentSnapshot {
    pub(crate) const fn digest(&self) -> ContentDigest {
        self.digest
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.record.generation
    }

    pub(crate) const fn binding(&self) -> &EnrollmentBindingV1 {
        &self.record.binding
    }
}

/// Bounded, inert evidence from one authenticated enrollment head.
///
/// This value deliberately contains no directory capability, authority key,
/// lease handle, reader, writer, or transition method. A later runtime open
/// must independently reopen and authenticate the enrollment and acquire its
/// writer lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnrollmentDiscoveryEvidence {
    pub(crate) binding: EnrollmentBindingV1,
    pub(crate) head_digest: ContentDigest,
    pub(crate) generation: u64,
    pub(crate) lifecycle: EnrollmentDiscoveryLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentDiscoveryLifecycle {
    ShadowImport,
    VerifiedLocal,
    LocalActive(EnrollmentDiscoveryLocalActive),
    Blocked {
        reason_code: String,
        evidence_digest: ContentDigest,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnrollmentDiscoveryLocalActive {
    pub(crate) verification_digest: ContentDigest,
    pub(crate) bootstrap_import_id: ContentDigest,
    pub(crate) bootstrap_part_count: u32,
    pub(crate) anchor_history_generation: u64,
    pub(crate) anchor_history_index_root: ContentDigest,
    pub(crate) anchor_acceptance_sequence: u64,
    pub(crate) anchor_accepted_frontier_state_digest: ContentDigest,
    pub(crate) handoff: EnrollmentDiscoveryHandoff,
    pub(crate) exclusion: EnrollmentDiscoveryExclusion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentDiscoveryHandoff {
    Safe,
    Unsafe { session_id: SessionId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentDiscoveryExclusion {
    Idle,
    Published,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentDiscoveryInspection {
    Absent,
    Residue,
    Present(EnrollmentDiscoveryEvidence),
}

/// Inspect one explicit device-local application root without creating or
/// repairing any enrollment state.
///
/// The path is opened as an existing no-follow directory. A present head is
/// decoded through the canonical authority-claim and record decoders and
/// authenticated through the same bounded checkpoint/chain validation as
/// [`EnrollmentReader::open_existing`]. The lease file is read only for its
/// physical identity and is never locked.
pub(crate) fn inspect_existing_enrollment_at(
    root_path: &Path,
    expected_graph_resource: CanonicalGraphResourceId,
) -> Result<EnrollmentDiscoveryInspection, EnrollmentError> {
    let Some(root) = open_existing_application_root(root_path)? else {
        return Ok(EnrollmentDiscoveryInspection::Absent);
    };
    let Some(directories) = open_directories(&root, expected_graph_resource, false)? else {
        return Ok(EnrollmentDiscoveryInspection::Absent);
    };
    validate_namespaces(&directories)?;
    let lease_resource_id = inspect_lease_resource_id(&directories)?;
    if read_head(&directories.enrollment)?.is_none() {
        return Ok(EnrollmentDiscoveryInspection::Residue);
    }
    let authority = open_discovered_enrollment_authority(
        &directories,
        expected_graph_resource,
        lease_resource_id,
    )?;
    let binding = authority.material.claim.binding().clone();
    let current = read_head_and_chain(
        &directories,
        &binding,
        lease_resource_id,
        &authority.material,
    )?
    .ok_or(EnrollmentError::MalformedHead)?;
    let lifecycle = match current.record.lifecycle() {
        EnrollmentLifecycleV1::ShadowImport(_) => EnrollmentDiscoveryLifecycle::ShadowImport,
        EnrollmentLifecycleV1::VerifiedLocal(_) => EnrollmentDiscoveryLifecycle::VerifiedLocal,
        EnrollmentLifecycleV1::LocalActive(active)
        | EnrollmentLifecycleV1::SharedActive(SharedActiveV1 {
            local_active: active,
            ..
        }) => EnrollmentDiscoveryLifecycle::LocalActive(EnrollmentDiscoveryLocalActive {
            verification_digest: active.verification_digest,
            bootstrap_import_id: active.anchor.bootstrap_import_id,
            bootstrap_part_count: active.anchor.bootstrap_part_count,
            anchor_history_generation: active.anchor.accepted_frontier_anchor.history_generation,
            anchor_history_index_root: active.anchor.accepted_frontier_anchor.history_root,
            anchor_acceptance_sequence: active.anchor.accepted_frontier_anchor.acceptance_sequence,
            anchor_accepted_frontier_state_digest: active
                .anchor
                .accepted_frontier_anchor
                .accepted_frontier_state_digest,
            handoff: match active.handoff {
                HandoffV1::Safe => EnrollmentDiscoveryHandoff::Safe,
                HandoffV1::Unsafe { session_id } => {
                    EnrollmentDiscoveryHandoff::Unsafe { session_id }
                }
            },
            exclusion: match active.exclusion {
                LocalExclusionV1::Idle => EnrollmentDiscoveryExclusion::Idle,
                LocalExclusionV1::Published { .. } => EnrollmentDiscoveryExclusion::Published,
            },
        }),
        EnrollmentLifecycleV1::SharePrepared(SharePreparedV1 {
            local_active: active,
            ..
        })
        | EnrollmentLifecycleV1::Joining(JoiningV1 {
            local_active: active,
            ..
        }) => EnrollmentDiscoveryLifecycle::LocalActive(EnrollmentDiscoveryLocalActive {
            verification_digest: active.verification_digest,
            bootstrap_import_id: active.anchor.bootstrap_import_id,
            bootstrap_part_count: active.anchor.bootstrap_part_count,
            anchor_history_generation: active.anchor.accepted_frontier_anchor.history_generation,
            anchor_history_index_root: active.anchor.accepted_frontier_anchor.history_root,
            anchor_acceptance_sequence: active.anchor.accepted_frontier_anchor.acceptance_sequence,
            anchor_accepted_frontier_state_digest: active
                .anchor
                .accepted_frontier_anchor
                .accepted_frontier_state_digest,
            handoff: match active.handoff {
                HandoffV1::Safe => EnrollmentDiscoveryHandoff::Safe,
                HandoffV1::Unsafe { session_id } => {
                    EnrollmentDiscoveryHandoff::Unsafe { session_id }
                }
            },
            exclusion: match active.exclusion {
                LocalExclusionV1::Idle => EnrollmentDiscoveryExclusion::Idle,
                LocalExclusionV1::Published { .. } => EnrollmentDiscoveryExclusion::Published,
            },
        }),
        EnrollmentLifecycleV1::Blocked(blocked) => EnrollmentDiscoveryLifecycle::Blocked {
            reason_code: blocked.reason_code.clone(),
            evidence_digest: blocked.evidence_digest,
        },
    };
    Ok(EnrollmentDiscoveryInspection::Present(
        EnrollmentDiscoveryEvidence {
            binding,
            head_digest: current.digest,
            generation: current.record.generation,
            lifecycle,
        },
    ))
}

fn open_existing_application_root(
    path: &Path,
) -> Result<Option<EnrollmentApplicationRoot>, EnrollmentError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(EnrollmentError::UnsafeNamespace(
            "private enrollment application root is not a real no-follow directory".into(),
        ));
    }
    let name = path.file_name().ok_or_else(|| {
        EnrollmentError::UnsafeNamespace(
            "private enrollment application root has no final component".into(),
        )
    })?;
    if !matches!(
        path.components().next_back(),
        Some(std::path::Component::Normal(_))
    ) {
        return Err(EnrollmentError::UnsafeNamespace(
            "private enrollment application root must end in a normal path component".into(),
        ));
    }
    let name = name.to_str().ok_or_else(|| {
        EnrollmentError::UnsafeNamespace(
            "private enrollment application root final component is not UTF-8".into(),
        )
    })?;
    let parent = path.parent().ok_or_else(|| {
        EnrollmentError::UnsafeNamespace(
            "private enrollment application root has no existing parent".into(),
        )
    })?;
    let canonical_parent = fs::canonicalize(parent)?;
    let parent = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
    let Some(directory) = open_component(&parent, name, false)? else {
        return Ok(None);
    };
    validate_private_directory(&directory, "private enrollment application root")?;
    Ok(Some(EnrollmentApplicationRoot {
        path: canonical_parent.join(name),
    }))
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentDiscoveryFixtureLifecycle {
    ShadowImport,
    VerifiedLocal,
    LocalActiveSafe,
    LocalActiveUnsafe { session_id: SessionId },
    Blocked,
}

/// Build the smallest valid authenticated enrollment used by the read-only
/// discovery tests. Production callers cannot reach this writer seam.
#[cfg(test)]
pub(crate) fn create_discovery_enrollment_for_test(
    root_path: &Path,
    binding: EnrollmentBindingV1,
    lifecycle: EnrollmentDiscoveryFixtureLifecycle,
) -> Result<EnrollmentDiscoveryEvidence, EnrollmentError> {
    let root = prepare_application_root(root_path)?;
    let preparation_id = PreparationId::from_uuid(Uuid::from_u128(0xd150));
    let source_inventory_digest = ContentDigest::of(b"discovery-fixture-source");
    let shadow = ShadowImportV1::new(preparation_id, source_inventory_digest);
    let mut writer = EnrollmentWriter::create(&root, binding.clone(), shadow)?;
    if lifecycle == EnrollmentDiscoveryFixtureLifecycle::ShadowImport {
        drop(writer);
    } else if lifecycle == EnrollmentDiscoveryFixtureLifecycle::Blocked {
        let current = writer.current().digest;
        writer.block_current(
            current,
            "discovery.blocked".into(),
            ContentDigest::of(b"discovery-fixture-blocked"),
        )?;
        drop(writer);
    } else {
        let verified = VerifiedLocalV1 {
            preparation_id,
            source_inventory_digest,
            source_file_count: 0,
            source_chunk_count: 0,
            source_total_bytes: 0,
            backup_manifest: BlobDescription::of(b"discovery-backup-manifest"),
            backup_restore_proof: BlobDescription::of(b"discovery-backup-restore"),
            backup_evidence_digest: ContentDigest::of(b"discovery-backup-evidence"),
            bootstrap_import_id: ContentDigest::of(b"discovery-bootstrap-import"),
            bootstrap_part_count: 0,
            bootstrap_terminal_part_id: None,
            bootstrap_batch_id: None,
            accepted_frontier_anchor: AcceptedFrontierAnchorV1 {
                acceptance_sequence: 0,
                accepted_frontier_state_digest: ContentDigest::of(b"discovery-accepted-frontier"),
                history_generation: 0,
                history_root: super::object_store::EngineHistoryStore::empty_root(),
            },
            accepted_history_record_count: 0,
            catalog_row_count: 0,
            sqlite_accepted_batch_count: 0,
            sqlite_semantic_projection_digest: ContentDigest::of(b"discovery-sqlite-semantic"),
            sqlite_materialized_row_digest: ContentDigest::of(b"discovery-sqlite-materialized"),
            staged_projection_manifest: BlobDescription::of(b"discovery-staged-manifest"),
            staged_projection_proof: BlobDescription::of(b"discovery-staged-proof"),
            staged_file_count: 0,
            staged_total_bytes: 0,
            byte_compare_digest: ContentDigest::of(b"discovery-byte-compare"),
            shadow_evidence_digest: ContentDigest::of(b"discovery-shadow-evidence"),
            proof_binding_digest: ContentDigest::of(b"discovery-proof-binding"),
        };
        let shadow_head = writer.current().digest;
        writer.transition(
            shadow_head,
            EnrollmentLifecycleV1::VerifiedLocal(verified.clone()),
        )?;
        if lifecycle == EnrollmentDiscoveryFixtureLifecycle::VerifiedLocal {
            drop(writer);
        } else {
            let verified_head = writer.current().digest;
            let session_id = match lifecycle {
                EnrollmentDiscoveryFixtureLifecycle::LocalActiveUnsafe { session_id } => session_id,
                EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe => {
                    SessionId::from_uuid(Uuid::from_u128(0xd151))
                }
                _ => unreachable!("non-active fixture states returned above"),
            };
            let anchor = LocalActiveAnchorV1::from_verified_local(&verified, verified_head);
            let active = LocalActiveV1 {
                verification_digest: verified.verification_digest()?,
                anchor,
                handoff: HandoffV1::Unsafe { session_id },
                exclusion: LocalExclusionV1::Idle,
            };
            writer.transition(
                verified_head,
                EnrollmentLifecycleV1::LocalActive(active.clone()),
            )?;
            if lifecycle == EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe {
                let active_head = writer.current().digest;
                writer.transition(
                    active_head,
                    EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
                        handoff: HandoffV1::Safe,
                        ..active
                    }),
                )?;
            }
            drop(writer);
        }
    }
    match inspect_existing_enrollment_at(root_path, binding.graph_resource_id())? {
        EnrollmentDiscoveryInspection::Present(evidence) => Ok(evidence),
        EnrollmentDiscoveryInspection::Absent | EnrollmentDiscoveryInspection::Residue => {
            Err(EnrollmentError::MalformedHead)
        }
    }
}

/// Install the frozen v1 authority / v5 generation-one bytes used by
/// cross-version recovery and discovery tests. The historical signer is
/// available only under `cfg(test)`; production can verify these bytes but can
/// never mint them.
#[cfg(test)]
pub(crate) fn create_legacy_initial_enrollment_for_test(
    root_path: &Path,
    binding: EnrollmentBindingV1,
) -> Result<(), EnrollmentError> {
    let root = prepare_application_root(root_path)?;
    let shadow = ShadowImportV1 {
        preparation_id: PreparationId::from_uuid(Uuid::from_u128(8)),
        source_inventory_digest: ContentDigest::from_bytes([9; 32]),
    };
    let writer = EnrollmentWriter::create(&root, binding.clone(), shadow.clone())?;
    let lease_resource_id = writer.lease.resource_id;
    drop(writer);

    let directories = open_directories(&root, binding.graph_resource_id, false)?
        .ok_or(EnrollmentError::AmbiguousInitialCreation)?;
    if let Some(head) = read_head(&directories.enrollment)? {
        directories.enrollment.remove_file(HEAD_FILE)?;
        directories
            .records
            .remove_file(&format!("{head}{RECORD_SUFFIX}"))?;
    }
    let legacy_claim =
        EnrollmentAuthorityClaim::LegacyV1(EnrollmentAuthorityClaimV1::new_for_test(
            ENROLLMENT_AUTHORITY_SCHEMA_V1,
            Uuid::from_u128(0x1e9ac),
            lease_resource_id,
            binding.clone(),
            shadow.preparation_id,
            shadow.source_inventory_digest,
            0x5a,
        ));
    let authority_bytes = canonical_authority_claim_bytes(&legacy_claim)?;
    directories.enrollment.remove_file(AUTHORITY_FILE)?;
    let mut authority_file = create_new_regular(&directories.enrollment, AUTHORITY_FILE)?;
    authority_file.write_all(&authority_bytes)?;
    authority_file.sync_all()?;
    let authority_identity = authoritative_file_identity(&authority_file)?;
    let material = EnrollmentAuthorityMaterial::from_claim(
        legacy_claim,
        authority_resource_id(&authority_identity),
        &binding,
        lease_resource_id,
    )?;
    let lifecycle = EnrollmentLifecycleV1::ShadowImport(shadow);
    let mut record = EnrollmentRecordV1 {
        schema_version: ENROLLMENT_RECORD_SCHEMA_V5,
        generation: 1,
        previous: None,
        history_accumulator: compute_history_accumulator(1, None, None, &binding, &lifecycle)?,
        lease_resource_id,
        binding,
        lifecycle,
        checkpoint: None,
    };
    record.checkpoint = Some(material.legacy_checkpoint_for_test(
        record.generation,
        record.previous,
        record.history_accumulator,
        record.lease_resource_id,
        &record.binding,
        &record.lifecycle,
    )?);
    let record_bytes = canonical_record_bytes(&record)?;
    let digest = ContentDigest::of(&record_bytes);
    let mut record_file =
        create_new_regular(&directories.records, &format!("{digest}{RECORD_SUFFIX}"))?;
    record_file.write_all(&record_bytes)?;
    record_file.sync_all()?;
    let mut head_file = create_new_regular(&directories.enrollment, HEAD_FILE)?;
    head_file.write_all(format!("{digest}\n").as_bytes())?;
    head_file.sync_all()?;
    sync_dir_required(&directories.records)
        .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    sync_dir_required(&directories.enrollment)
        .map_err(|error| EnrollmentError::Durability(error.to_string()))
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
    authority: EnrollmentAuthority,
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
        let lease_resource_id = inspect_lease_resource_id(&directories)?;
        if read_head(&directories.enrollment)?.is_none() {
            return Ok(EnrollmentOpen::Absent);
        }
        let authority =
            open_enrollment_authority(&directories, expected_binding, lease_resource_id)?;
        let current = read_head_and_chain(
            &directories,
            expected_binding,
            lease_resource_id,
            &authority.material,
        )?
        .ok_or(EnrollmentError::MalformedHead)?;
        Ok(EnrollmentOpen::Present(Self {
            directories,
            authority,
            current,
        }))
    }

    pub(crate) fn current(&self) -> &EnrollmentSnapshot {
        &self.current
    }

    pub(crate) fn audit_chain_page(
        &self,
        start: Option<EnrollmentAuditCursor>,
        limit: usize,
    ) -> Result<EnrollmentAuditPage, EnrollmentError> {
        if limit == 0 || limit > MAX_ENROLLMENT_AUDIT_PAGE {
            return Err(EnrollmentError::InvalidPageLimit(limit));
        }
        self.authority.validate_current()?;
        let (mut next, mut expected_generation, mut newer) = match start {
            Some(cursor) => {
                if cursor.head != self.current.digest {
                    return Err(EnrollmentError::InvalidAuditCursor);
                }
                self.authority.material.verify_audit_cursor(&cursor)?;
                let newer_record = read_record(&self.directories.records, cursor.newer_digest)?;
                validate_record_authority(
                    &newer_record,
                    &self.current.record.binding,
                    self.current.record.lease_resource_id,
                    &self.authority.material,
                )?;
                (
                    Some(cursor.digest),
                    cursor.generation,
                    Some((cursor.newer_digest, newer_record)),
                )
            }
            None => (
                Some(self.current.digest),
                self.current.record.generation,
                None,
            ),
        };
        let mut records = Vec::with_capacity(limit);
        while records.len() < limit {
            let Some(digest) = next else {
                break;
            };
            let record = read_record(&self.directories.records, digest)?;
            validate_record_authority(
                &record,
                &self.current.record.binding,
                self.current.record.lease_resource_id,
                &self.authority.material,
            )?;
            if record.generation != expected_generation {
                return Err(EnrollmentError::NonmonotonicGeneration);
            }
            if let Some((_, newer_record)) = &newer {
                validate_record_link(digest, &record, newer_record)?;
            }
            next = record.previous;
            expected_generation = expected_generation.saturating_sub(1);
            newer = Some((digest, record.clone()));
            records.push(EnrollmentSnapshot { digest, record });
        }
        if let Some(last) = records.last() {
            match last.record.previous {
                None => validate_initial_record(&last.record)?,
                Some(_) if expected_generation == 0 => {
                    return Err(EnrollmentError::NonmonotonicGeneration);
                }
                Some(_) => {}
            }
        }
        Ok(EnrollmentAuditPage {
            records,
            next: next.map(|digest| {
                let newer_digest = newer
                    .as_ref()
                    .expect("a continued page has a newer record")
                    .0;
                EnrollmentAuditCursor {
                    schema_version: 1,
                    head: self.current.digest,
                    digest,
                    generation: expected_generation,
                    newer_digest,
                    integrity_tag: self.authority.material.audit_cursor_tag(
                        self.current.digest,
                        digest,
                        expected_generation,
                        newer_digest,
                    ),
                }
            }),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnrollmentAuditCursor {
    schema_version: u32,
    head: ContentDigest,
    digest: ContentDigest,
    generation: u64,
    newer_digest: ContentDigest,
    integrity_tag: u32,
}

pub(crate) struct EnrollmentAuditPage {
    pub(crate) records: Vec<EnrollmentSnapshot>,
    pub(crate) next: Option<EnrollmentAuditCursor>,
}

/// OS-backed exclusive lease retained by one writable enrollment session.
pub(crate) struct EnrollmentLease {
    file: File,
    directory: Dir,
    identity: AuthoritativeFileIdentity,
    resource_id: ContentDigest,
}

impl EnrollmentLease {
    fn validate_current(&self) -> Result<(), EnrollmentError> {
        validate_authoritative_file(&self.file, "enrollment lease")?;
        if authoritative_file_identity(&self.file)? != self.identity {
            return Err(EnrollmentError::LeaseResourceMismatch);
        }
        let reopened = open_regular_readwrite_existing(&self.directory, LEASE_FILE)
            .map_err(|_| EnrollmentError::LeaseResourceMismatch)?;
        validate_authoritative_file(&reopened, "enrollment lease")?;
        if authoritative_file_identity(&reopened)? != self.identity {
            return Err(EnrollmentError::LeaseResourceMismatch);
        }
        Ok(())
    }
}

impl Drop for EnrollmentLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) struct EnrollmentWriter {
    reader: EnrollmentReader,
    lease: EnrollmentLease,
}

impl EnrollmentWriter {
    pub(crate) fn create(
        root: &EnrollmentApplicationRoot,
        binding: EnrollmentBindingV1,
        shadow: ShadowImportV1,
    ) -> Result<Self, EnrollmentError> {
        Self::create_at_cut(root, binding, shadow, CommitCut::None)
    }

    fn create_at_cut(
        root: &EnrollmentApplicationRoot,
        binding: EnrollmentBindingV1,
        shadow: ShadowImportV1,
        cut: CommitCut,
    ) -> Result<Self, EnrollmentError> {
        binding.validate_internal()?;
        let directories = open_directories(root, binding.graph_resource_id, true)?
            .expect("create mode returns enrollment directories");
        validate_namespaces(&directories)?;
        let lease = acquire_lease(&directories, true)?;
        let authority =
            provision_or_resume_enrollment_authority(&directories, &lease, &binding, &shadow)?;
        let current_record =
            EnrollmentRecordV1::initial(binding, shadow, lease.resource_id, &authority.material)?;
        let record = select_initial_record_for_recovery(&directories, &authority, &current_record)?;
        let snapshot =
            resume_or_persist_initial_record(&directories, &lease, &authority, &record, cut)?;
        Ok(Self {
            reader: EnrollmentReader {
                directories,
                authority,
                current: snapshot,
            },
            lease,
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
        if read_head(&directories.enrollment)?.is_none() {
            return Ok(EnrollmentOpen::Absent);
        }
        let authority =
            open_enrollment_authority(&directories, expected_binding, lease.resource_id)?;
        let current = read_head_and_chain(
            &directories,
            expected_binding,
            lease.resource_id,
            &authority.material,
        )?
        .ok_or(EnrollmentError::MalformedHead)?;
        Ok(EnrollmentOpen::Present(Self {
            reader: EnrollmentReader {
                directories,
                authority,
                current,
            },
            lease,
        }))
    }

    pub(crate) fn current(&self) -> &EnrollmentSnapshot {
        self.reader.current()
    }

    /// Read the committed head digest alone through the retained enrollment
    /// directory capability.
    ///
    /// This is one bounded read of the fixed 65-byte head file, with the same
    /// no-follow authoritative-file validation `read_head` always performs. It
    /// enumerates no namespace, reacquires no lease, rereads no authority
    /// claim, and reads no record.
    fn committed_head(&self) -> Result<Option<ContentDigest>, EnrollmentError> {
        read_head(&self.reader.directories.enrollment)
    }

    /// Repeat the complete authenticated open over the *retained* capabilities.
    ///
    /// This is exactly the proof `EnrollmentReader::open_existing` performs —
    /// namespace validation, lease identity, authority-claim identity and
    /// bytes, and the bounded authenticated head/checkpoint chain — except that
    /// it runs against the directory, lease, and authority handles this session
    /// already holds instead of resolving and reacquiring them. Reopening from
    /// the pathname would drop and retake the exclusive lease, which is the one
    /// thing a retained session must never do.
    fn reauthenticate(&mut self) -> Result<&EnrollmentSnapshot, EnrollmentError> {
        self.lease.validate_current()?;
        self.reader.authority.validate_current()?;
        validate_namespaces(&self.reader.directories)?;
        let binding = self.reader.current.record.binding.clone();
        let current = read_head_and_chain(
            &self.reader.directories,
            &binding,
            self.lease.resource_id,
            &self.reader.authority.material,
        )?
        .ok_or(EnrollmentError::MalformedHead)?;
        self.reader.current = current;
        Ok(&self.reader.current)
    }

    pub(crate) fn audit_chain_page(
        &self,
        start: Option<EnrollmentAuditCursor>,
        limit: usize,
    ) -> Result<EnrollmentAuditPage, EnrollmentError> {
        self.reader.audit_chain_page(start, limit)
    }

    fn transition(
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
        self.lease.validate_current()?;
        self.reader.authority.validate_current()?;
        if self.reader.current.digest != expected_current
            || read_head(&self.reader.directories.enrollment)? != Some(expected_current)
        {
            return Err(EnrollmentError::StaleCompareAndSwap);
        }
        let record = EnrollmentRecordV1::successor(
            &self.reader.current,
            lifecycle,
            &self.reader.authority.material,
        )?;
        let snapshot =
            persist_record_and_head(&self.reader.directories, &self.lease, &record, cut)?;
        self.reader.current = snapshot;
        Ok(&self.reader.current)
    }

    pub(crate) fn block_current(
        &mut self,
        expected_current: ContentDigest,
        reason_code: String,
        evidence_digest: ContentDigest,
    ) -> Result<&EnrollmentSnapshot, EnrollmentError> {
        self.transition(
            expected_current,
            EnrollmentLifecycleV1::Blocked(BlockedV1 {
                prior_record_digest: expected_current,
                reason_code,
                evidence_digest,
            }),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitCut {
    None,
    AfterRecordTempCreate,
    AfterRecordWrite,
    AfterRecordFileSync,
    AfterRecordLink,
    AfterRecordInsert,
    AfterRecordsDirectorySync,
    AfterHeadTempCreate,
    AfterHeadWrite,
    AfterHeadFileSync,
    AfterHeadReplace,
    AfterEnrollmentDirectorySync,
}

/// Exact retained proof set accepted by the sole `ShadowImport ->
/// VerifiedLocal` composition boundary. None of these types can be constructed
/// from enrollment digest fields.
pub(crate) struct VerifiedLocalProofSet<'a> {
    pub(crate) graph: &'a Graph,
    pub(crate) roots: &'a MigrationBackupRoot,
    pub(crate) prepared: &'a InactiveBootstrapPreparedPublication,
    pub(crate) verified_publication: &'a InactiveBootstrapVerifiedPublication,
    pub(crate) source_backup: &'a VerifiedSourceBackup,
    pub(crate) accepted_authority: &'a InactiveBootstrapAcceptedAuthority,
    pub(crate) sqlite: &'a OpenProjection,
    pub(crate) sqlite_projection: &'a VerifiedBootstrapSqliteProjection,
    pub(crate) shadow_projection: &'a VerifiedShadowProjection,
}

/// Opaque pre-activation authority. It is minted only after the committed
/// enrollment head and every retained proof have been freshly reopened.
///
/// This type deliberately exposes no graph writer, projection writer, watcher,
/// managed-sync mutation, or `LocalActive` transition.
pub(crate) struct VerifiedLocalEvidence {
    enrollment_head: ContentDigest,
    verification_digest: ContentDigest,
    binding: EnrollmentBindingV1,
    preparation_id: PreparationId,
    bootstrap_batch_id: Option<BatchId>,
    accepted_frontier_state_digest: ContentDigest,
}

impl VerifiedLocalEvidence {
    pub(crate) const fn enrollment_head(&self) -> ContentDigest {
        self.enrollment_head
    }

    pub(crate) const fn preparation_id(&self) -> PreparationId {
        self.preparation_id
    }

    pub(crate) const fn verification_digest(&self) -> ContentDigest {
        self.verification_digest
    }

    pub(crate) const fn binding(&self) -> &EnrollmentBindingV1 {
        &self.binding
    }

    pub(crate) const fn bootstrap_batch_id(&self) -> Option<BatchId> {
        self.bootstrap_batch_id
    }

    pub(crate) const fn accepted_frontier_state_digest(&self) -> ContentDigest {
        self.accepted_frontier_state_digest
    }
}

/// Process-local proof that the complete backup/SQLite/shadow set was freshly
/// validated for the exact `VerifiedLocal` record immediately before its
/// commit/readback sequence. It is intentionally neither cloneable nor
/// serializable; fresh-process reopen continues through the full proof path.
pub(crate) struct RetainedVerifiedLocalValidation {
    evidence: VerifiedLocalEvidence,
    expected: VerifiedLocalV1,
}

impl RetainedVerifiedLocalValidation {
    pub(crate) const fn evidence(&self) -> &VerifiedLocalEvidence {
        &self.evidence
    }

    pub(crate) fn into_evidence(self) -> VerifiedLocalEvidence {
        self.evidence
    }
}

#[derive(Debug)]
pub(crate) enum VerifiedLocalCompositionError {
    Enrollment(EnrollmentError),
    Bootstrap(BootstrapStreamingImportError),
    Backup(MigrationBackupError),
    Sqlite(ProjectionError),
    Shadow(ShadowProjectionError),
    ProofBinding(String),
    ProofMismatch(&'static str),
    WrongLifecycle(&'static str),
    StaleEvidence(&'static str),
    CompetingSession,
}

impl fmt::Display for VerifiedLocalCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enrollment(error) => error.fmt(formatter),
            Self::Bootstrap(error) => error.fmt(formatter),
            Self::Backup(error) => error.fmt(formatter),
            Self::Sqlite(error) => error.fmt(formatter),
            Self::Shadow(error) => error.fmt(formatter),
            Self::ProofBinding(detail) => {
                write!(formatter, "verified-local proof binding failed: {detail}")
            }
            Self::ProofMismatch(detail) => {
                write!(formatter, "verified-local proof mismatch: {detail}")
            }
            Self::WrongLifecycle(detail) => {
                write!(formatter, "verified-local lifecycle mismatch: {detail}")
            }
            Self::StaleEvidence(detail) => {
                write!(formatter, "stale enrollment activation evidence: {detail}")
            }
            Self::CompetingSession => formatter
                .write_str("a competing session already owns this LocalActive enrollment handoff"),
        }
    }
}

impl std::error::Error for VerifiedLocalCompositionError {}

impl From<EnrollmentError> for VerifiedLocalCompositionError {
    fn from(error: EnrollmentError) -> Self {
        Self::Enrollment(error)
    }
}

impl From<BootstrapStreamingImportError> for VerifiedLocalCompositionError {
    fn from(error: BootstrapStreamingImportError) -> Self {
        Self::Bootstrap(error)
    }
}

impl From<MigrationBackupError> for VerifiedLocalCompositionError {
    fn from(error: MigrationBackupError) -> Self {
        Self::Backup(error)
    }
}

impl From<ProjectionError> for VerifiedLocalCompositionError {
    fn from(error: ProjectionError) -> Self {
        Self::Sqlite(error)
    }
}

impl From<ShadowProjectionError> for VerifiedLocalCompositionError {
    fn from(error: ShadowProjectionError) -> Self {
        Self::Shadow(error)
    }
}

impl From<std::io::Error> for VerifiedLocalCompositionError {
    fn from(error: std::io::Error) -> Self {
        Self::Enrollment(EnrollmentError::from(error))
    }
}

/// Persist or resume the exact inactive enrollment composition. The only
/// caller-supplied enrollment datum is the opaque preparation identity; every
/// digest in `VerifiedLocalV1` is freshly derived from retained proof types.
pub(crate) fn compose_verified_local(
    root: &EnrollmentApplicationRoot,
    binding: EnrollmentBindingV1,
    preparation_id: PreparationId,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<VerifiedLocalEvidence, VerifiedLocalCompositionError> {
    compose_verified_local_at_cut(root, binding, preparation_id, proofs, CommitCut::None)
}

pub(crate) fn compose_verified_local_retaining_validation(
    root: &EnrollmentApplicationRoot,
    binding: EnrollmentBindingV1,
    preparation_id: PreparationId,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<RetainedVerifiedLocalValidation, VerifiedLocalCompositionError> {
    let shadow = ShadowImportV1::new(
        preparation_id,
        ContentDigest::from_bytes(
            *proofs
                .prepared
                .source_capture()
                .inventory_description()
                .sha256(),
        ),
    );
    let mut writer = match EnrollmentWriter::open_existing(root, &binding)? {
        EnrollmentOpen::Absent => EnrollmentWriter::create(root, binding.clone(), shadow.clone())?,
        EnrollmentOpen::Present(writer) => writer,
    };
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::ShadowImport(current) if current == &shadow => {}
        EnrollmentLifecycleV1::VerifiedLocal(current)
            if current.preparation_id == shadow.preparation_id
                && current.source_inventory_digest == shadow.source_inventory_digest => {}
        EnrollmentLifecycleV1::ShadowImport(_) | EnrollmentLifecycleV1::VerifiedLocal(_) => {
            return Err(EnrollmentError::InitialPreparationMismatch.into());
        }
        _ => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "only ShadowImport can enter retained VerifiedLocal activation",
            ));
        }
    }
    validate_verified_local_binding(&binding, proofs)?;
    proofs
        .accepted_authority
        .store()
        .validate_enrolled_archive_resource_id(binding.archive_resource_id())
        .map_err(|error| {
            VerifiedLocalCompositionError::ProofBinding(format!(
                "persisted archive resource claim does not authenticate retained validation: {error}"
            ))
        })?;
    let expected = verified_local_from_validated_proofs(
        &binding,
        preparation_id,
        proofs,
        proofs.accepted_authority,
        proofs.source_backup,
        proofs.shadow_projection,
    )?;
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::ShadowImport(_) => {
            let current = writer.current().digest;
            writer.transition(
                current,
                EnrollmentLifecycleV1::VerifiedLocal(expected.clone()),
            )?;
        }
        EnrollmentLifecycleV1::VerifiedLocal(current) if current == &expected => {}
        EnrollmentLifecycleV1::VerifiedLocal(_) => {
            return Err(VerifiedLocalCompositionError::ProofMismatch(
                "committed VerifiedLocal differs from retained validation",
            ));
        }
        _ => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "enrollment changed during retained VerifiedLocal composition",
            ));
        }
    }
    drop(writer);
    let evidence = reopen_verified_local_against_expected(root, &binding, &expected)?;
    Ok(RetainedVerifiedLocalValidation { evidence, expected })
}

/// Persist or resume only the durable `ShadowImport` predecessor of the
/// verified-local composition.  This is the first resumable marker the public
/// activation facade writes after it has captured exact source inventory
/// evidence.  It deliberately grants no graph, projection, or mutation
/// authority.
pub(crate) fn begin_or_resume_shadow_import(
    root: &EnrollmentApplicationRoot,
    binding: EnrollmentBindingV1,
    preparation_id: PreparationId,
    source_inventory_digest: ContentDigest,
) -> Result<(), VerifiedLocalCompositionError> {
    let shadow = ShadowImportV1::new(preparation_id, source_inventory_digest);
    let writer = match EnrollmentWriter::open_existing(root, &binding)? {
        EnrollmentOpen::Absent => EnrollmentWriter::create(root, binding, shadow.clone())?,
        EnrollmentOpen::Present(writer) => writer,
    };
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::ShadowImport(current) if current == &shadow => Ok(()),
        EnrollmentLifecycleV1::VerifiedLocal(current)
            if current.preparation_id == shadow.preparation_id
                && current.source_inventory_digest == shadow.source_inventory_digest =>
        {
            Ok(())
        }
        EnrollmentLifecycleV1::ShadowImport(_) | EnrollmentLifecycleV1::VerifiedLocal(_) => {
            Err(EnrollmentError::InitialPreparationMismatch.into())
        }
        EnrollmentLifecycleV1::LocalActive(_)
        | EnrollmentLifecycleV1::SharePrepared(_)
        | EnrollmentLifecycleV1::Joining(_)
        | EnrollmentLifecycleV1::SharedActive(_) => {
            Err(VerifiedLocalCompositionError::WrongLifecycle(
                "active or shared enrollment cannot be resumed as ShadowImport",
            ))
        }
        EnrollmentLifecycleV1::Blocked(_) => Err(VerifiedLocalCompositionError::WrongLifecycle(
            "blocked enrollment cannot resume ShadowImport",
        )),
    }
}

#[cfg(test)]
pub(crate) fn compose_verified_local_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    binding: EnrollmentBindingV1,
    preparation_id: PreparationId,
    proofs: &VerifiedLocalProofSet<'_>,
    cut: CommitCut,
) -> Result<VerifiedLocalEvidence, VerifiedLocalCompositionError> {
    compose_verified_local_at_cut(root, binding, preparation_id, proofs, cut)
}

fn compose_verified_local_at_cut(
    root: &EnrollmentApplicationRoot,
    binding: EnrollmentBindingV1,
    preparation_id: PreparationId,
    proofs: &VerifiedLocalProofSet<'_>,
    cut: CommitCut,
) -> Result<VerifiedLocalEvidence, VerifiedLocalCompositionError> {
    let shadow = ShadowImportV1::new(
        preparation_id,
        ContentDigest::from_bytes(
            *proofs
                .prepared
                .source_capture()
                .inventory_description()
                .sha256(),
        ),
    );
    let mut writer = match EnrollmentWriter::open_existing(root, &binding)? {
        EnrollmentOpen::Absent => EnrollmentWriter::create(root, binding.clone(), shadow.clone())?,
        EnrollmentOpen::Present(writer) => writer,
    };
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::ShadowImport(current) if current == &shadow => {}
        EnrollmentLifecycleV1::VerifiedLocal(current)
            if current.preparation_id == shadow.preparation_id
                && current.source_inventory_digest == shadow.source_inventory_digest => {}
        EnrollmentLifecycleV1::ShadowImport(_) | EnrollmentLifecycleV1::VerifiedLocal(_) => {
            return Err(EnrollmentError::InitialPreparationMismatch.into());
        }
        EnrollmentLifecycleV1::LocalActive(_)
        | EnrollmentLifecycleV1::SharePrepared(_)
        | EnrollmentLifecycleV1::Joining(_)
        | EnrollmentLifecycleV1::SharedActive(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "active or shared enrollment cannot be composed or reopened as VerifiedLocal",
            ));
        }
        EnrollmentLifecycleV1::Blocked(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "blocked enrollment cannot advance",
            ));
        }
    }

    let expected = freshly_validate_verified_local(&binding, preparation_id, proofs)?;
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::ShadowImport(_) => {
            let current = writer.current().digest;
            writer.transition_at_cut(
                current,
                EnrollmentLifecycleV1::VerifiedLocal(expected),
                cut,
            )?;
        }
        EnrollmentLifecycleV1::VerifiedLocal(current) if current == &expected => {}
        EnrollmentLifecycleV1::VerifiedLocal(_) => {
            return Err(VerifiedLocalCompositionError::ProofMismatch(
                "committed VerifiedLocal record differs from freshly validated proofs",
            ));
        }
        EnrollmentLifecycleV1::LocalActive(_)
        | EnrollmentLifecycleV1::SharePrepared(_)
        | EnrollmentLifecycleV1::Joining(_)
        | EnrollmentLifecycleV1::SharedActive(_)
        | EnrollmentLifecycleV1::Blocked(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "enrollment changed during VerifiedLocal composition",
            ));
        }
    }
    drop(writer);
    reopen_verified_local(root, &binding, proofs)
}

/// Bounded startup/reopen gate. Enrollment bytes alone never mint authority:
/// retained backup, accepted history, SQLite, live source, and shadow evidence
/// are all freshly revalidated, followed by a second enrollment-head reopen.
pub(crate) fn reopen_verified_local(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<VerifiedLocalEvidence, VerifiedLocalCompositionError> {
    let reader = match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "VerifiedLocal enrollment is absent",
            ));
        }
        EnrollmentOpen::Present(reader) => reader,
    };
    let committed = match reader.current().record.lifecycle() {
        EnrollmentLifecycleV1::VerifiedLocal(verified) => verified.clone(),
        EnrollmentLifecycleV1::ShadowImport(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "enrollment remains ShadowImport",
            ));
        }
        EnrollmentLifecycleV1::LocalActive(_)
        | EnrollmentLifecycleV1::SharePrepared(_)
        | EnrollmentLifecycleV1::Joining(_)
        | EnrollmentLifecycleV1::SharedActive(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "active or shared enrollment is not VerifiedLocal evidence",
            ));
        }
        EnrollmentLifecycleV1::Blocked(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "blocked enrollment has no VerifiedLocal authority",
            ));
        }
    };
    let expected = freshly_validate_verified_local(binding, committed.preparation_id, proofs)?;
    if committed != expected {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "enrollment head does not bind the freshly reopened proofs",
        ));
    }
    drop(reader);

    reopen_verified_local_against_expected(root, binding, &expected)
}

fn reopen_verified_local_against_expected(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected: &VerifiedLocalV1,
) -> Result<VerifiedLocalEvidence, VerifiedLocalCompositionError> {
    let reader = match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "VerifiedLocal enrollment disappeared before retained readback",
            ));
        }
        EnrollmentOpen::Present(reader) => reader,
    };
    let expected_head = reader.current().digest;
    if !matches!(
        reader.current().record.lifecycle(),
        EnrollmentLifecycleV1::VerifiedLocal(verified) if verified == expected
    ) {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "enrollment head does not bind retained VerifiedLocal validation",
        ));
    }
    drop(reader);
    let reopened = match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "VerifiedLocal enrollment disappeared during proof validation",
            ));
        }
        EnrollmentOpen::Present(reader) => reader,
    };
    let reopened_verified = match reopened.current().record.lifecycle() {
        EnrollmentLifecycleV1::VerifiedLocal(verified)
            if reopened.current().digest == expected_head && verified == expected =>
        {
            verified
        }
        _ => {
            return Err(VerifiedLocalCompositionError::ProofMismatch(
                "enrollment head changed during proof validation",
            ));
        }
    };
    Ok(VerifiedLocalEvidence {
        enrollment_head: reopened.current().digest,
        verification_digest: reopened_verified.verification_digest()?,
        binding: binding.clone(),
        preparation_id: reopened_verified.preparation_id,
        bootstrap_batch_id: reopened_verified.bootstrap_batch_id,
        accepted_frontier_state_digest: reopened_verified
            .accepted_frontier_anchor
            .accepted_frontier_state_digest,
    })
}

/// Durable handoff state of a committed `LocalActive` enrollment.
///
/// `Unsafe` is the only state a mutation may be admitted under, and it always
/// names the exact session that owns graph text. A crash therefore always
/// resumes conservatively unsafe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalActiveHandoff {
    Unsafe { session_id: SessionId },
    Safe,
}

/// Durable sync/exclusion state of a committed `LocalActive` enrollment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalActiveSync {
    Idle,
    Published,
}

/// Exact, freshly authenticated enrollment evidence carried by one runtime
/// resume point.
///
/// The fields and lifecycle representation are private to this module. A
/// lifecycle caller can obtain the value only from a [`CommittedLocalActive`]
/// that this module minted after an authenticated readback; it cannot invent a
/// generation, head, session, or a synthetic session for `Safe`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResumePointEnrollmentBinding {
    generation: u64,
    head: ContentDigest,
    lifecycle: ResumePointEnrollmentLifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResumePointEnrollmentLifecycle {
    Unsafe { session_id: SessionId },
    Safe,
}

impl ResumePointEnrollmentBinding {
    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) const fn head(self) -> ContentDigest {
        self.head
    }

    pub(crate) const fn unsafe_session_id(self) -> Option<SessionId> {
        match self.lifecycle {
            ResumePointEnrollmentLifecycle::Unsafe { session_id } => Some(session_id),
            ResumePointEnrollmentLifecycle::Safe => None,
        }
    }

    pub(crate) const fn is_safe(self) -> bool {
        matches!(self.lifecycle, ResumePointEnrollmentLifecycle::Safe)
    }

    #[cfg(test)]
    pub(crate) const fn unsafe_for_test(
        generation: u64,
        head: ContentDigest,
        session_id: SessionId,
    ) -> Self {
        Self {
            generation,
            head,
            lifecycle: ResumePointEnrollmentLifecycle::Unsafe { session_id },
        }
    }

    #[cfg(test)]
    pub(crate) const fn safe_for_test(generation: u64, head: ContentDigest) -> Self {
        Self {
            generation,
            head,
            lifecycle: ResumePointEnrollmentLifecycle::Safe,
        }
    }
}

/// Freshly reopened committed `LocalActive` enrollment state.
///
/// This type is minted only by this module, only after reading the committed
/// head back from disk. Its fields are private so no sibling module can
/// assemble one from record bytes, and it deliberately exposes no writer.
pub(crate) struct CommittedLocalActive {
    enrollment_head: ContentDigest,
    generation: u64,
    verification_digest: ContentDigest,
    anchor: LocalActiveAnchorV1,
    handoff: LocalActiveHandoff,
    sync: LocalActiveSync,
    binding: EnrollmentBindingV1,
}

impl CommittedLocalActive {
    pub(crate) const fn enrollment_head(&self) -> ContentDigest {
        self.enrollment_head
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) const fn verification_digest(&self) -> ContentDigest {
        self.verification_digest
    }

    pub(crate) const fn handoff(&self) -> LocalActiveHandoff {
        self.handoff
    }

    pub(crate) const fn sync(&self) -> LocalActiveSync {
        self.sync
    }

    pub(crate) const fn binding(&self) -> &EnrollmentBindingV1 {
        &self.binding
    }

    pub(crate) fn session_id(&self) -> Option<SessionId> {
        match self.handoff {
            LocalActiveHandoff::Unsafe { session_id } => Some(session_id),
            LocalActiveHandoff::Safe => None,
        }
    }

    /// Bind a resume point to this exact authenticated lifecycle record.
    ///
    /// `Safe` has no session identity. Keeping the construction here prevents
    /// a caller from encoding it with a sentinel or otherwise hand-building
    /// lifecycle evidence.
    pub(crate) const fn resume_point_binding(&self) -> ResumePointEnrollmentBinding {
        ResumePointEnrollmentBinding {
            generation: self.generation,
            head: self.enrollment_head,
            lifecycle: match self.handoff {
                LocalActiveHandoff::Unsafe { session_id } => {
                    ResumePointEnrollmentLifecycle::Unsafe { session_id }
                }
                LocalActiveHandoff::Safe => ResumePointEnrollmentLifecycle::Safe,
            },
        }
    }

    /// Rebuild the exact predecessor [`VerifiedLocalEvidence`] the original
    /// activation consumed, entirely from this record's immutable anchor.
    fn predecessor_evidence(&self) -> VerifiedLocalEvidence {
        VerifiedLocalEvidence {
            enrollment_head: self.anchor.verified_local_record_digest,
            verification_digest: self.verification_digest,
            binding: self.binding.clone(),
            preparation_id: self.anchor.preparation_id,
            bootstrap_batch_id: self.anchor.bootstrap_batch_id,
            accepted_frontier_state_digest: self
                .anchor
                .accepted_frontier_anchor
                .accepted_frontier_state_digest,
        }
    }
}

fn observe_local_active(
    snapshot: &EnrollmentSnapshot,
    binding: &EnrollmentBindingV1,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let active = match snapshot.record.lifecycle() {
        EnrollmentLifecycleV1::LocalActive(active) => active,
        EnrollmentLifecycleV1::ShadowImport(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "enrollment remains ShadowImport",
            ));
        }
        EnrollmentLifecycleV1::VerifiedLocal(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "enrollment remains VerifiedLocal",
            ));
        }
        EnrollmentLifecycleV1::SharePrepared(prepared) => &prepared.local_active,
        EnrollmentLifecycleV1::Joining(joining) => &joining.local_active,
        EnrollmentLifecycleV1::SharedActive(active) => &active.local_active,
        EnrollmentLifecycleV1::Blocked(_) => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "blocked enrollment has no LocalActive authority",
            ));
        }
    };
    Ok(CommittedLocalActive {
        enrollment_head: snapshot.digest,
        generation: snapshot.record.generation,
        verification_digest: active.verification_digest,
        anchor: active.anchor,
        handoff: match active.handoff {
            HandoffV1::Safe => LocalActiveHandoff::Safe,
            HandoffV1::Unsafe { session_id } => LocalActiveHandoff::Unsafe { session_id },
        },
        sync: match active.exclusion {
            LocalExclusionV1::Idle => LocalActiveSync::Idle,
            LocalExclusionV1::Published { .. } => LocalActiveSync::Published,
        },
        binding: binding.clone(),
    })
}

fn local_active_lifecycle(
    current: &EnrollmentLifecycleV1,
    verification_digest: ContentDigest,
    anchor: LocalActiveAnchorV1,
    handoff: LocalActiveHandoff,
) -> EnrollmentLifecycleV1 {
    let local_active = LocalActiveV1 {
        verification_digest,
        anchor,
        handoff: match handoff {
            LocalActiveHandoff::Safe => HandoffV1::Safe,
            LocalActiveHandoff::Unsafe { session_id } => HandoffV1::Unsafe { session_id },
        },
        exclusion: LocalExclusionV1::Idle,
    };
    match current {
        EnrollmentLifecycleV1::SharePrepared(prepared) => {
            let mut prepared = prepared.clone();
            prepared.local_active = local_active;
            EnrollmentLifecycleV1::SharePrepared(prepared)
        }
        EnrollmentLifecycleV1::Joining(joining) => {
            let mut joining = joining.clone();
            joining.local_active = local_active;
            EnrollmentLifecycleV1::Joining(joining)
        }
        EnrollmentLifecycleV1::SharedActive(active) => {
            let mut active = active.clone();
            active.local_active = local_active;
            EnrollmentLifecycleV1::SharedActive(active)
        }
        _ => EnrollmentLifecycleV1::LocalActive(local_active),
    }
}

/// The only externally observable core states for the still-inactive sharing
/// packet.  There is deliberately no runtime writer authority for either
/// `SharePrepared` or `SharedActive`; P3.2 must explicitly wire that later.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedEnrollmentPhase {
    SharePrepared,
    Joining,
    SharedActiveInitiator,
    SharedActiveJoiner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedEnrollmentRole {
    Initiator,
    Joiner,
}

pub(crate) fn inspect_shared_enrollment_descriptor(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
) -> Result<Option<(SharedEnrollmentDescriptorV1, SharedEnrollmentRole)>, EnrollmentError> {
    let EnrollmentOpen::Present(reader) = EnrollmentReader::open_existing(root, binding)? else {
        return Ok(None);
    };
    Ok(match reader.current().record.lifecycle() {
        EnrollmentLifecycleV1::SharePrepared(prepared) => {
            Some((prepared.descriptor.clone(), SharedEnrollmentRole::Initiator))
        }
        EnrollmentLifecycleV1::Joining(joining) => {
            Some((joining.descriptor.clone(), SharedEnrollmentRole::Joiner))
        }
        EnrollmentLifecycleV1::SharedActive(active) => Some((
            active.descriptor.clone(),
            match active.role {
                SharedEnrollmentRoleV1::Initiator => SharedEnrollmentRole::Initiator,
                SharedEnrollmentRoleV1::Joiner => SharedEnrollmentRole::Joiner,
            },
        )),
        _ => None,
    })
}

/// Read the durable sharing phase without creating an enrollment namespace or
/// acquiring a writer lease.  This is useful to a future inactive UI/runtime
/// facade, but does not enable any default flow.
pub(crate) fn inspect_shared_enrollment_phase(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
) -> Result<Option<SharedEnrollmentPhase>, EnrollmentError> {
    let EnrollmentOpen::Present(reader) = EnrollmentReader::open_existing(root, binding)? else {
        return Ok(None);
    };
    let phase = match reader.current().record.lifecycle() {
        EnrollmentLifecycleV1::SharePrepared(_) => Some(SharedEnrollmentPhase::SharePrepared),
        EnrollmentLifecycleV1::Joining(_) => Some(SharedEnrollmentPhase::Joining),
        EnrollmentLifecycleV1::SharedActive(active) => Some(match active.role {
            SharedEnrollmentRoleV1::Initiator => SharedEnrollmentPhase::SharedActiveInitiator,
            SharedEnrollmentRoleV1::Joiner => SharedEnrollmentPhase::SharedActiveJoiner,
        }),
        EnrollmentLifecycleV1::ShadowImport(_)
        | EnrollmentLifecycleV1::VerifiedLocal(_)
        | EnrollmentLifecycleV1::LocalActive(_)
        | EnrollmentLifecycleV1::Blocked(_) => None,
    };
    Ok(phase)
}

fn shared_descriptor_evidence_digest(descriptor: &SharedEnrollmentDescriptorV1) -> ContentDigest {
    // Evidence attribution must remain available even when the descriptor is
    // malformed or from a future protocol, so it intentionally does not call
    // `descriptor.validate()`.
    match serde_json::to_vec(descriptor) {
        Ok(bytes) => ContentDigest::of(&bytes),
        Err(_) => ContentDigest::of(b"shared-enrollment-descriptor-encode-failure"),
    }
}

fn block_shared_enrollment(
    writer: &mut EnrollmentWriter,
    reason_code: &str,
    evidence_digest: ContentDigest,
    error: EnrollmentError,
) -> Result<(), EnrollmentError> {
    let current = writer.current().digest;
    writer.block_current(current, reason_code.into(), evidence_digest)?;
    Err(error)
}

fn current_local_active_for_shared_enrollment(
    writer: &EnrollmentWriter,
) -> Result<CommittedLocalActive, EnrollmentError> {
    observe_local_active(writer.current(), writer.current().record.binding()).map_err(|error| {
        EnrollmentError::IllegalLifecycle(match error {
            VerifiedLocalCompositionError::WrongLifecycle(detail) => detail,
            _ => "LocalActive state could not be authenticated for shared enrollment",
        })
    })
}

fn require_safe_idle_shared_local_active(
    active: &CommittedLocalActive,
) -> Result<(), EnrollmentError> {
    if active.handoff != LocalActiveHandoff::Safe {
        return Err(EnrollmentError::UnsafeSharedEnrollmentHandoff);
    }
    if active.sync != LocalActiveSync::Idle {
        return Err(EnrollmentError::IllegalLifecycle(
            "shared enrollment requires an idle LocalActive record",
        ));
    }
    Ok(())
}

/// Persist the initiator's commit-last `LocalActive -> SharePrepared` record
/// and return the one exact descriptor a peer may use.  Repeating the call
/// after any enrollment commit cut returns the same descriptor; a changed
/// object-store namespace becomes a durable conflict rather than a second
/// genesis.
pub(crate) fn prepare_shared_enrollment(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    object_store_namespace: ContentDigest,
) -> Result<SharedEnrollmentDescriptorV1, EnrollmentError> {
    prepare_shared_enrollment_at_cut(root, binding, object_store_namespace, CommitCut::None)
}

#[cfg(test)]
fn prepare_shared_enrollment_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    object_store_namespace: ContentDigest,
    cut: CommitCut,
) -> Result<SharedEnrollmentDescriptorV1, EnrollmentError> {
    prepare_shared_enrollment_at_cut(root, binding, object_store_namespace, cut)
}

fn prepare_shared_enrollment_at_cut(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    object_store_namespace: ContentDigest,
    cut: CommitCut,
) -> Result<SharedEnrollmentDescriptorV1, EnrollmentError> {
    let EnrollmentOpen::Present(mut writer) = EnrollmentWriter::open_existing(root, binding)?
    else {
        return Err(EnrollmentError::IllegalLifecycle(
            "shared enrollment requires an existing LocalActive enrollment",
        ));
    };
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::LocalActive(_) => {
            let active = current_local_active_for_shared_enrollment(&writer)?;
            if let Err(error) = require_safe_idle_shared_local_active(&active) {
                return block_shared_enrollment(
                    &mut writer,
                    "shared.unsafe-handoff",
                    active.enrollment_head,
                    error,
                )
                .and(Err(EnrollmentError::IllegalTransition));
            }
            let verified = match read_anchored_verified_local(&writer.reader, &active) {
                Ok(verified) => verified,
                Err(error) => {
                    return block_shared_enrollment(
                        &mut writer,
                        "shared.local-proof-mismatch",
                        active.enrollment_head,
                        EnrollmentError::IllegalLifecycle(match error {
                            VerifiedLocalCompositionError::WrongLifecycle(detail) => detail,
                            _ => "LocalActive anchor could not prove shared enrollment evidence",
                        }),
                    )
                    .and(Err(EnrollmentError::IllegalTransition));
                }
            };
            let descriptor = match SharedEnrollmentDescriptorV1::from_local_active(
                binding,
                &active,
                &verified,
                object_store_namespace,
            ) {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    return block_shared_enrollment(
                        &mut writer,
                        "shared.incompatible-descriptor",
                        active.enrollment_head,
                        error,
                    )
                    .and(Err(EnrollmentError::IllegalTransition));
                }
            };
            let prepared = SharePreparedV1 {
                descriptor: descriptor.clone(),
                descriptor_digest: descriptor.digest()?,
                local_active: match writer.current().record.lifecycle() {
                    EnrollmentLifecycleV1::LocalActive(active) => active.clone(),
                    _ => unreachable!("the branch is LocalActive"),
                },
            };
            let expected = writer.current().digest;
            writer.transition_at_cut(
                expected,
                EnrollmentLifecycleV1::SharePrepared(prepared),
                cut,
            )?;
            Ok(descriptor)
        }
        EnrollmentLifecycleV1::SharePrepared(prepared) => {
            let existing = prepared.clone();
            if existing.descriptor.object_store_namespace == object_store_namespace {
                Ok(existing.descriptor)
            } else {
                block_shared_enrollment(
                    &mut writer,
                    "shared.descriptor-conflict",
                    existing.descriptor_digest,
                    EnrollmentError::SharedEnrollmentBindingMismatch,
                )
                .and(Err(EnrollmentError::IllegalTransition))
            }
        }
        EnrollmentLifecycleV1::SharedActive(active)
            if active.role == SharedEnrollmentRoleV1::Initiator
                && active.descriptor.object_store_namespace == object_store_namespace =>
        {
            Ok(active.descriptor.clone())
        }
        EnrollmentLifecycleV1::SharedActive(_) => block_shared_enrollment(
            &mut writer,
            "shared.descriptor-conflict",
            ContentDigest::of(b"shared-initiator-namespace-conflict"),
            EnrollmentError::SharedEnrollmentBindingMismatch,
        )
        .and(Err(EnrollmentError::IllegalTransition)),
        EnrollmentLifecycleV1::Joining(_)
        | EnrollmentLifecycleV1::ShadowImport(_)
        | EnrollmentLifecycleV1::VerifiedLocal(_)
        | EnrollmentLifecycleV1::Blocked(_) => Err(EnrollmentError::IllegalLifecycle(
            "enrollment cannot prepare an initiator descriptor from its current state",
        )),
    }
}

/// Commit the initiator side of `SharePrepared -> SharedActive`.  The only
/// data written is the authenticated enrollment journal; no graph or
/// projection bytes are opened or changed by this core boundary.
pub(crate) fn activate_shared_initiator(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
) -> Result<(), EnrollmentError> {
    activate_shared_initiator_at_cut(root, binding, descriptor, CommitCut::None)
}

#[cfg(test)]
fn activate_shared_initiator_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    activate_shared_initiator_at_cut(root, binding, descriptor, cut)
}

fn activate_shared_initiator_at_cut(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    let evidence = shared_descriptor_evidence_digest(descriptor);
    let EnrollmentOpen::Present(mut writer) = EnrollmentWriter::open_existing(root, binding)?
    else {
        return Err(EnrollmentError::IllegalLifecycle(
            "shared initiator enrollment is absent",
        ));
    };
    if let Err(error) = descriptor.validate() {
        return block_shared_enrollment(
            &mut writer,
            "shared.incompatible-descriptor",
            evidence,
            error,
        )
        .and(Err(EnrollmentError::IllegalTransition));
    }
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::SharePrepared(prepared) if prepared.descriptor == *descriptor => {
            let expected = writer.current().digest;
            writer.transition_at_cut(
                expected,
                EnrollmentLifecycleV1::SharedActive(SharedActiveV1 {
                    descriptor: descriptor.clone(),
                    descriptor_digest: prepared.descriptor_digest,
                    role: SharedEnrollmentRoleV1::Initiator,
                    archived_local_workspace: None,
                    local_active: prepared.local_active.clone(),
                }),
                cut,
            )?;
            Ok(())
        }
        EnrollmentLifecycleV1::SharedActive(active)
            if active.role == SharedEnrollmentRoleV1::Initiator
                && active.descriptor == *descriptor =>
        {
            Ok(())
        }
        EnrollmentLifecycleV1::SharePrepared(_) | EnrollmentLifecycleV1::SharedActive(_) => {
            block_shared_enrollment(
                &mut writer,
                "shared.descriptor-conflict",
                evidence,
                EnrollmentError::SharedEnrollmentBindingMismatch,
            )
            .and(Err(EnrollmentError::IllegalTransition))
        }
        _ => Err(EnrollmentError::IllegalLifecycle(
            "shared initiator activation requires SharePrepared",
        )),
    }
}

/// Commit `LocalActive -> Joining` after the caller has archived the joiner's
/// former local workspace.  The `unique_unprojected_operation_count` is a
/// deliberately explicit proof input: anything nonzero blocks *before* the
/// archive witness can be persisted, so a dirty unique tail cannot be
/// discarded or silently merged.
pub(crate) fn prepare_shared_join(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    archived_workspace_digest: ContentDigest,
    unique_unprojected_operation_count: u64,
) -> Result<(), EnrollmentError> {
    prepare_shared_join_at_cut(
        root,
        binding,
        descriptor,
        archived_workspace_digest,
        unique_unprojected_operation_count,
        CommitCut::None,
    )
}

#[cfg(test)]
fn prepare_shared_join_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    archived_workspace_digest: ContentDigest,
    unique_unprojected_operation_count: u64,
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    prepare_shared_join_at_cut(
        root,
        binding,
        descriptor,
        archived_workspace_digest,
        unique_unprojected_operation_count,
        cut,
    )
}

fn prepare_shared_join_at_cut(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    archived_workspace_digest: ContentDigest,
    unique_unprojected_operation_count: u64,
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    let evidence = shared_descriptor_evidence_digest(descriptor);
    let EnrollmentOpen::Present(mut writer) = EnrollmentWriter::open_existing(root, binding)?
    else {
        return Err(EnrollmentError::IllegalLifecycle(
            "shared join enrollment is absent",
        ));
    };
    if let Err(error) = descriptor.validate() {
        return block_shared_enrollment(
            &mut writer,
            "shared.incompatible-descriptor",
            evidence,
            error,
        )
        .and(Err(EnrollmentError::IllegalTransition));
    }
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::LocalActive(_) => {
            let active = current_local_active_for_shared_enrollment(&writer)?;
            if let Err(error) = require_safe_idle_shared_local_active(&active) {
                return block_shared_enrollment(
                    &mut writer,
                    "shared.unsafe-handoff",
                    active.enrollment_head,
                    error,
                )
                .and(Err(EnrollmentError::IllegalTransition));
            }
            let verified = match read_anchored_verified_local(&writer.reader, &active) {
                Ok(verified) => verified,
                Err(_) => {
                    return block_shared_enrollment(
                        &mut writer,
                        "shared.local-proof-mismatch",
                        active.enrollment_head,
                        EnrollmentError::SharedEnrollmentBindingMismatch,
                    )
                    .and(Err(EnrollmentError::IllegalTransition));
                }
            };
            let archive = JoinerWorkspaceArchiveV1 {
                schema_version: JOINER_WORKSPACE_ARCHIVE_SCHEMA_VERSION,
                archived_workspace_digest,
                source_local_active_head: active.enrollment_head,
                source_verification_digest: active.verification_digest,
                unique_unprojected_operation_count,
                projection_base: SharedProjectionBaseEvidenceV1::from_verified_local(&verified),
            };
            if let Err(error) = archive.validate() {
                return block_shared_enrollment(
                    &mut writer,
                    "shared.dirty-unique-tail",
                    archived_workspace_digest,
                    error,
                )
                .and(Err(EnrollmentError::IllegalTransition));
            }
            let projection_mismatch = archive
                .projection_base
                .first_mismatch(&descriptor.projection_base);
            if !descriptor.is_compatible_with(binding) || projection_mismatch.is_some() {
                return block_shared_enrollment(
                    &mut writer,
                    "shared.projection-base-mismatch",
                    evidence,
                    projection_mismatch.map_or(
                        EnrollmentError::SharedEnrollmentBindingMismatch,
                        EnrollmentError::SharedProjectionBaseMismatch,
                    ),
                )
                .and(Err(EnrollmentError::IllegalTransition));
            }
            let joining = JoiningV1 {
                descriptor: descriptor.clone(),
                descriptor_digest: descriptor.digest()?,
                archived_local_workspace: archive,
                local_active: match writer.current().record.lifecycle() {
                    EnrollmentLifecycleV1::LocalActive(active) => active.clone(),
                    _ => unreachable!("the branch is LocalActive"),
                },
            };
            let expected = writer.current().digest;
            writer.transition_at_cut(expected, EnrollmentLifecycleV1::Joining(joining), cut)?;
            Ok(())
        }
        EnrollmentLifecycleV1::Joining(joining) if joining.descriptor == *descriptor => Ok(()),
        EnrollmentLifecycleV1::SharedActive(active)
            if active.role == SharedEnrollmentRoleV1::Joiner
                && active.descriptor == *descriptor =>
        {
            Ok(())
        }
        EnrollmentLifecycleV1::Joining(_) | EnrollmentLifecycleV1::SharedActive(_) => {
            block_shared_enrollment(
                &mut writer,
                "shared.descriptor-conflict",
                evidence,
                EnrollmentError::SharedEnrollmentBindingMismatch,
            )
            .and(Err(EnrollmentError::IllegalTransition))
        }
        _ => Err(EnrollmentError::IllegalLifecycle(
            "shared join requires LocalActive or its exact resumable state",
        )),
    }
}

/// Commit the peer side of `Joining -> SharedActive`.  Retrying with the same
/// descriptor is idempotent; another descriptor is a durable split-genesis
/// conflict, never an automatic merge.
pub(crate) fn activate_shared_joiner(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
) -> Result<(), EnrollmentError> {
    activate_shared_joiner_at_cut(root, binding, descriptor, CommitCut::None)
}

#[cfg(test)]
fn activate_shared_joiner_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    activate_shared_joiner_at_cut(root, binding, descriptor, cut)
}

fn activate_shared_joiner_at_cut(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    descriptor: &SharedEnrollmentDescriptorV1,
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    let evidence = shared_descriptor_evidence_digest(descriptor);
    let EnrollmentOpen::Present(mut writer) = EnrollmentWriter::open_existing(root, binding)?
    else {
        return Err(EnrollmentError::IllegalLifecycle(
            "shared join enrollment is absent",
        ));
    };
    if let Err(error) = descriptor.validate() {
        return block_shared_enrollment(
            &mut writer,
            "shared.incompatible-descriptor",
            evidence,
            error,
        )
        .and(Err(EnrollmentError::IllegalTransition));
    }
    match writer.current().record.lifecycle() {
        EnrollmentLifecycleV1::Joining(joining) if joining.descriptor == *descriptor => {
            let expected = writer.current().digest;
            writer.transition_at_cut(
                expected,
                EnrollmentLifecycleV1::SharedActive(SharedActiveV1 {
                    descriptor: descriptor.clone(),
                    descriptor_digest: joining.descriptor_digest,
                    role: SharedEnrollmentRoleV1::Joiner,
                    archived_local_workspace: Some(joining.archived_local_workspace.clone()),
                    local_active: joining.local_active.clone(),
                }),
                cut,
            )?;
            Ok(())
        }
        EnrollmentLifecycleV1::SharedActive(active)
            if active.role == SharedEnrollmentRoleV1::Joiner
                && active.descriptor == *descriptor =>
        {
            Ok(())
        }
        EnrollmentLifecycleV1::Joining(_) | EnrollmentLifecycleV1::SharedActive(_) => {
            block_shared_enrollment(
                &mut writer,
                "shared.descriptor-conflict",
                evidence,
                EnrollmentError::SharedEnrollmentBindingMismatch,
            )
            .and(Err(EnrollmentError::IllegalTransition))
        }
        _ => Err(EnrollmentError::IllegalLifecycle(
            "shared join activation requires Joining",
        )),
    }
}

/// Persist or idempotently resume the exact `VerifiedLocal -> LocalActive`
/// transition for one activation session.
///
/// Every persisted field is derived here: the caller supplies only retained
/// evidence, the activation session identity, and the live proof set. The
/// retained proofs are freshly revalidated and `reopen_verified_local` is
/// called immediately before the transition, so a changed head, mixed proof
/// set, or stale evidence fails closed without advancing.
pub(crate) fn activate_verified_local_record(
    root: &EnrollmentApplicationRoot,
    evidence: &VerifiedLocalEvidence,
    session_id: SessionId,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    activate_verified_local_record_at_cut(root, evidence, session_id, proofs, CommitCut::None)
}

pub(crate) fn activate_verified_local_record_with_retained_validation(
    root: &EnrollmentApplicationRoot,
    validation: &RetainedVerifiedLocalValidation,
    session_id: SessionId,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let evidence = validation.evidence();
    let binding = evidence.binding();
    let already_active = {
        let reader = match EnrollmentReader::open_existing(root, binding)? {
            EnrollmentOpen::Absent => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "enrollment is absent",
                ));
            }
            EnrollmentOpen::Present(reader) => reader,
        };
        match reader.current().record.lifecycle() {
            EnrollmentLifecycleV1::VerifiedLocal(verified)
                if reader.current().digest == evidence.enrollment_head()
                    && verified == &validation.expected =>
            {
                false
            }
            EnrollmentLifecycleV1::LocalActive(_) => true,
            EnrollmentLifecycleV1::VerifiedLocal(_) => {
                return Err(VerifiedLocalCompositionError::StaleEvidence(
                    "VerifiedLocal head changed after retained proof validation",
                ));
            }
            _ => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "retained validation cannot activate this lifecycle",
                ));
            }
        }
    };
    if !already_active {
        let mut writer = match EnrollmentWriter::open_existing(root, binding)? {
            EnrollmentOpen::Absent => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "enrollment disappeared during activation",
                ));
            }
            EnrollmentOpen::Present(writer) => writer,
        };
        let current = writer.current().digest;
        let anchor = match writer.current().record.lifecycle() {
            EnrollmentLifecycleV1::VerifiedLocal(verified)
                if current == evidence.enrollment_head() && verified == &validation.expected =>
            {
                LocalActiveAnchorV1::from_verified_local(verified, current)
            }
            _ => {
                return Err(VerifiedLocalCompositionError::StaleEvidence(
                    "enrollment head changed before retained activation commit",
                ));
            }
        };
        writer.transition(
            current,
            local_active_lifecycle(
                writer.current().record.lifecycle(),
                evidence.verification_digest(),
                anchor,
                LocalActiveHandoff::Unsafe { session_id },
            ),
        )?;
        drop(writer);
    }
    reopen_local_active_record_against_evidence(root, evidence, session_id)
}

#[cfg(test)]
pub(crate) fn activate_verified_local_record_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    evidence: &VerifiedLocalEvidence,
    session_id: SessionId,
    proofs: &VerifiedLocalProofSet<'_>,
    cut: CommitCut,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    activate_verified_local_record_at_cut(root, evidence, session_id, proofs, cut)
}

fn activate_verified_local_record_at_cut(
    root: &EnrollmentApplicationRoot,
    evidence: &VerifiedLocalEvidence,
    session_id: SessionId,
    proofs: &VerifiedLocalProofSet<'_>,
    cut: CommitCut,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let binding = &evidence.binding;
    let already_active = {
        let reader = match EnrollmentReader::open_existing(root, binding)? {
            EnrollmentOpen::Absent => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "enrollment is absent",
                ));
            }
            EnrollmentOpen::Present(reader) => reader,
        };
        match reader.current().record.lifecycle() {
            EnrollmentLifecycleV1::VerifiedLocal(_) => {
                if reader.current().digest != evidence.enrollment_head {
                    return Err(VerifiedLocalCompositionError::StaleEvidence(
                        "committed VerifiedLocal head is not the retained evidence head",
                    ));
                }
                false
            }
            EnrollmentLifecycleV1::LocalActive(_) => true,
            EnrollmentLifecycleV1::SharePrepared(_)
            | EnrollmentLifecycleV1::Joining(_)
            | EnrollmentLifecycleV1::SharedActive(_) => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "shared enrollment cannot activate a LocalActive runtime",
                ));
            }
            EnrollmentLifecycleV1::ShadowImport(_) => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "enrollment remains ShadowImport",
                ));
            }
            EnrollmentLifecycleV1::Blocked(_) => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "blocked enrollment cannot activate",
                ));
            }
        }
    };

    if !already_active {
        // Freshly reopen the complete retained proof set and the committed
        // VerifiedLocal head immediately before the transition.
        let fresh = reopen_verified_local(root, binding, proofs)?;
        if fresh.enrollment_head != evidence.enrollment_head
            || fresh.verification_digest != evidence.verification_digest
            || fresh.preparation_id != evidence.preparation_id
            || fresh.bootstrap_batch_id != evidence.bootstrap_batch_id
            || fresh.accepted_frontier_state_digest != evidence.accepted_frontier_state_digest
            || fresh.binding != evidence.binding
        {
            return Err(VerifiedLocalCompositionError::StaleEvidence(
                "freshly reopened VerifiedLocal evidence differs from the retained evidence",
            ));
        }
        let mut writer = match EnrollmentWriter::open_existing(root, binding)? {
            EnrollmentOpen::Absent => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "enrollment disappeared during activation",
                ));
            }
            EnrollmentOpen::Present(writer) => writer,
        };
        let current = writer.current().digest;
        // The anchor persisted below is derived from the record this writer
        // just reread, and from that record's own committed content digest, so
        // it can only ever name the exact `VerifiedLocal` predecessor consumed.
        let anchor = match writer.current().record.lifecycle() {
            EnrollmentLifecycleV1::VerifiedLocal(verified)
                if current == fresh.enrollment_head
                    && verified.verification_digest()? == fresh.verification_digest =>
            {
                LocalActiveAnchorV1::from_verified_local(verified, current)
            }
            _ => {
                return Err(VerifiedLocalCompositionError::StaleEvidence(
                    "enrollment head changed between proof revalidation and activation",
                ));
            }
        };
        writer.transition_at_cut(
            current,
            local_active_lifecycle(
                writer.current().record.lifecycle(),
                fresh.verification_digest,
                anchor,
                LocalActiveHandoff::Unsafe { session_id },
            ),
            cut,
        )?;
        drop(writer);
    }

    reopen_local_active_record(root, evidence, session_id, proofs)
}

/// Bounded fresh-process reopen of the exact committed activation record.
///
/// Enrollment bytes alone never mint authority here either: the complete
/// retained proof set is revalidated and must reproduce the exact committed
/// verification digest.
pub(crate) fn reopen_local_active_record(
    root: &EnrollmentApplicationRoot,
    evidence: &VerifiedLocalEvidence,
    session_id: SessionId,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let binding = &evidence.binding;
    let fresh = freshly_validate_verified_local(binding, evidence.preparation_id, proofs)?;
    if fresh.verification_digest()? != evidence.verification_digest {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "freshly revalidated proofs do not reproduce the retained verification digest",
        ));
    }
    reopen_local_active_record_against_evidence(root, evidence, session_id)
}

fn reopen_local_active_record_against_evidence(
    root: &EnrollmentApplicationRoot,
    evidence: &VerifiedLocalEvidence,
    session_id: SessionId,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let binding = &evidence.binding;
    let reader = match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "LocalActive enrollment is absent",
            ));
        }
        EnrollmentOpen::Present(reader) => reader,
    };
    let committed = observe_local_active(reader.current(), binding)?;
    if reader.current().record.previous != Some(evidence.enrollment_head) {
        return Err(VerifiedLocalCompositionError::StaleEvidence(
            "committed LocalActive record does not succeed the retained VerifiedLocal evidence",
        ));
    }
    if committed.verification_digest != evidence.verification_digest {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "committed LocalActive record binds another verification digest",
        ));
    }
    if committed.sync != LocalActiveSync::Idle {
        return Err(VerifiedLocalCompositionError::WrongLifecycle(
            "LocalActive activation requires an Idle sync state",
        ));
    }
    match committed.handoff {
        LocalActiveHandoff::Unsafe {
            session_id: committed_session,
        } if committed_session == session_id => Ok(committed),
        LocalActiveHandoff::Unsafe { .. } => Err(VerifiedLocalCompositionError::CompetingSession),
        LocalActiveHandoff::Safe => Err(VerifiedLocalCompositionError::WrongLifecycle(
            "LocalActive activation requires the durable Unsafe handoff state",
        )),
    }
}

/// A freshly proof-revalidated fresh-process reopen of a committed
/// `LocalActive` enrollment.
///
/// It carries the reconstructed [`VerifiedLocalEvidence`] for the exact
/// `VerifiedLocal` predecessor the original activation consumed, so a restarted
/// process needs no retained in-memory evidence at all. Only this module can
/// mint one, and only after the complete retained proof set has reproduced the
/// exact committed verification digest.
pub(crate) struct ReopenedLocalActive {
    predecessor: VerifiedLocalEvidence,
    committed: CommittedLocalActive,
}

impl ReopenedLocalActive {
    pub(crate) const fn predecessor_evidence(&self) -> &VerifiedLocalEvidence {
        &self.predecessor
    }

    pub(crate) const fn committed(&self) -> &CommittedLocalActive {
        &self.committed
    }

    pub(crate) fn into_parts(self) -> (VerifiedLocalEvidence, CommittedLocalActive) {
        (self.predecessor, self.committed)
    }
}

/// Bounded fresh-process reopen of a committed `LocalActive` enrollment that
/// requires no retained in-memory [`VerifiedLocalEvidence`].
///
/// Enrollment bytes alone still never mint authority. The committed
/// `LocalActive` head's immutable anchor names the original `VerifiedLocal`
/// record by content address, so that record is reread directly, the complete
/// retained proof set is freshly revalidated against it, and the freshly
/// derived verification digest must reproduce the digest the committed
/// `LocalActive` record binds. The head is then reopened a second time, so a
/// head that moved during the proof pass fails closed.
///
/// Cost is independent of the enrollment's lifetime generation: the head's
/// bounded checkpoint/open proof plus exactly one anchored record read. Nothing
/// here assumes the committed `LocalActive` record directly succeeds
/// `VerifiedLocal`; any legal sequence of `Safe`/`Unsafe` handoff records is
/// accepted, however long.
pub(crate) fn reopen_local_active_from_durable_state(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<ReopenedLocalActive, VerifiedLocalCompositionError> {
    let reader = open_local_active_reader(root, binding)?;
    let committed = observe_idle_local_active(&reader, binding)?;
    let expected_head = committed.enrollment_head;
    let verified = read_anchored_verified_local(&reader, &committed)?;
    drop(reader);

    let expected = freshly_validate_verified_local(binding, verified.preparation_id, proofs)?;
    if expected != verified {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "committed VerifiedLocal predecessor does not bind the freshly reopened proofs",
        ));
    }
    if expected.verification_digest()? != committed.verification_digest {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "freshly revalidated proofs do not reproduce the committed verification digest",
        ));
    }

    // Reopen after the expensive proof pass. The head digest commits to the
    // whole authenticated hash-linked ancestry including the immutable anchor,
    // so an unchanged head keeps the anchored predecessor read above exact.
    let reopened = open_local_active_reader(root, binding)?;
    let reopened_committed = observe_idle_local_active(&reopened, binding)?;
    if reopened_committed.enrollment_head != expected_head
        || reopened_committed.verification_digest != committed.verification_digest
        || reopened_committed.anchor != committed.anchor
        || reopened_committed.handoff != committed.handoff
    {
        return Err(VerifiedLocalCompositionError::StaleEvidence(
            "committed LocalActive head changed during proof revalidation",
        ));
    }
    Ok(ReopenedLocalActive {
        predecessor: reopened_committed.predecessor_evidence(),
        committed: reopened_committed,
    })
}

/// The original `VerifiedLocal` bootstrap anchor, reconstructed from durable
/// enrollment state alone.
///
/// A promoted runtime has advanced its durable history past the bootstrap, so
/// [`reopen_local_active_from_durable_state`]'s full proof revalidation is no
/// longer reconstructible: the shadow projection compares against graph text
/// the user has since edited, and the inactive accepted authority requires an
/// unadvanced history head. What *is* still exactly reconstructible is the
/// anchor itself. Every committed `LocalActive` record carries the immutable
/// [`LocalActiveAnchorV1`] minted at activation, and the committed
/// `VerifiedLocalV1` record it names by content address is self-authenticating
/// — its verification digest is a pure function of its own bytes — and the
/// enrollment chain is hash-linked, so the head's anchor plus one
/// content-addressed record read reproduces the committed digest and proves the
/// exact original anchor, without reading one graph byte and without any
/// dependence on how many sessions the graph has since had.
///
/// This value binds only enrollment state. Binding it to the live archive,
/// retained immutable bootstrap publication, and authenticated history
/// transition is [`super::local_active`]'s job; nothing here grants authority.
pub(crate) struct PromotedBootstrapAnchor {
    committed: CommittedLocalActive,
}

impl PromotedBootstrapAnchor {
    const fn anchor(&self) -> &LocalActiveAnchorV1 {
        &self.committed.anchor
    }

    pub(crate) const fn binding(&self) -> &EnrollmentBindingV1 {
        self.committed.binding()
    }

    pub(crate) const fn committed(&self) -> &CommittedLocalActive {
        &self.committed
    }

    pub(crate) const fn verification_digest(&self) -> ContentDigest {
        self.committed.verification_digest
    }

    pub(crate) const fn bootstrap_import_id(&self) -> ContentDigest {
        self.anchor().bootstrap_import_id
    }

    pub(crate) const fn bootstrap_part_count(&self) -> u32 {
        self.anchor().bootstrap_part_count
    }

    pub(crate) const fn accepted_history_record_count(&self) -> u64 {
        self.anchor().accepted_history_record_count
    }

    pub(crate) const fn acceptance_sequence(&self) -> u64 {
        self.anchor().accepted_frontier_anchor.acceptance_sequence
    }

    pub(crate) const fn accepted_frontier_state_digest(&self) -> ContentDigest {
        self.anchor()
            .accepted_frontier_anchor
            .accepted_frontier_state_digest
    }

    pub(crate) const fn history_generation(&self) -> u64 {
        self.anchor().accepted_frontier_anchor.history_generation
    }

    pub(crate) const fn history_root(&self) -> ContentDigest {
        self.anchor().accepted_frontier_anchor.history_root
    }

    /// Rebuild the exact predecessor [`VerifiedLocalEvidence`] the original
    /// activation consumed, plus the committed `LocalActive` record.
    pub(crate) fn into_predecessor_evidence(self) -> (VerifiedLocalEvidence, CommittedLocalActive) {
        let predecessor = self.committed.predecessor_evidence();
        (predecessor, self.committed)
    }
}

/// Fresh-process reconstruction of the committed `LocalActive` record and its
/// exact original `VerifiedLocal` bootstrap anchor.
///
/// Cost is the existing bounded checkpoint/open proof plus exactly one
/// content-addressed record read, never the enrollment lifetime, the graph, the
/// archive, or SQLite. Absent, `ShadowImport`, `Blocked`, non-`Idle`,
/// malformed, cross-bound, forged-anchor, and changed-digest state all fail
/// closed.
pub(crate) fn reopen_promoted_bootstrap_anchor(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
) -> Result<PromotedBootstrapAnchor, VerifiedLocalCompositionError> {
    binding.validate_internal()?;
    let reader = open_local_active_reader(root, binding)?;
    let committed = observe_idle_local_active(&reader, binding)?;
    read_anchored_verified_local(&reader, &committed)?;
    drop(reader);
    Ok(PromotedBootstrapAnchor { committed })
}

/// Reread and revalidate the exact original `VerifiedLocal` record the
/// committed `LocalActive` head anchors.
///
/// The head's immutable anchor names that record by content address, so this is
/// one direct authenticated record read rather than a backward search, and its
/// cost does not grow with the number of handoff records the enrollment has
/// accumulated. The original record is never deleted or compacted, so it stays
/// available both for this revalidation and for forensic audit.
fn read_anchored_verified_local(
    reader: &EnrollmentReader,
    committed: &CommittedLocalActive,
) -> Result<VerifiedLocalV1, VerifiedLocalCompositionError> {
    let anchor = committed.anchor;
    let record = read_record(
        &reader.directories.records,
        anchor.verified_local_record_digest,
    )?;
    validate_record_authority(
        &record,
        &reader.current.record.binding,
        reader.current.record.lease_resource_id,
        &reader.authority.material,
    )?;
    let EnrollmentLifecycleV1::VerifiedLocal(verified) = record.lifecycle() else {
        return Err(VerifiedLocalCompositionError::WrongLifecycle(
            "the anchored enrollment record is not the original VerifiedLocal record",
        ));
    };
    // The committed VerifiedLocal record commits to itself: reproducing its
    // digest proves the retained anchor fields are the exact ones the original
    // activation bound.
    if verified.verification_digest()? != committed.verification_digest {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "the anchored VerifiedLocal record does not reproduce the committed verification digest",
        ));
    }
    if anchor
        != LocalActiveAnchorV1::from_verified_local(verified, anchor.verified_local_record_digest)
    {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "the committed LocalActive anchor diverges from the original VerifiedLocal record",
        ));
    }
    Ok(verified.clone())
}

fn open_local_active_reader(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
) -> Result<EnrollmentReader, VerifiedLocalCompositionError> {
    match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => Err(VerifiedLocalCompositionError::WrongLifecycle(
            "LocalActive enrollment is absent",
        )),
        EnrollmentOpen::Present(reader) => Ok(reader),
    }
}

fn observe_idle_local_active(
    reader: &EnrollmentReader,
    binding: &EnrollmentBindingV1,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let committed = observe_local_active(reader.current(), binding)?;
    if committed.sync != LocalActiveSync::Idle {
        return Err(VerifiedLocalCompositionError::WrongLifecycle(
            "LocalActive reopen requires an Idle sync state",
        ));
    }
    Ok(committed)
}

/// Durably move one committed `LocalActive` record between handoff states.
///
/// The compare-and-swap is narrow: the exact expected head, verification
/// digest, and `Idle` sync state must still be committed. The new state is
/// proved by a fresh committed-head reopen before it is returned.
pub(crate) fn transition_local_active_handoff(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected_head: ContentDigest,
    verification_digest: ContentDigest,
    target: LocalActiveHandoff,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    transition_local_active_handoff_at_cut(
        root,
        binding,
        expected_head,
        verification_digest,
        target,
        CommitCut::None,
    )
}

#[cfg(test)]
pub(crate) fn transition_local_active_handoff_at_cut_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected_head: ContentDigest,
    verification_digest: ContentDigest,
    target: LocalActiveHandoff,
    cut: CommitCut,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    transition_local_active_handoff_at_cut(
        root,
        binding,
        expected_head,
        verification_digest,
        target,
        cut,
    )
}

fn transition_local_active_handoff_at_cut(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected_head: ContentDigest,
    verification_digest: ContentDigest,
    target: LocalActiveHandoff,
    cut: CommitCut,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let mut writer = match EnrollmentWriter::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "LocalActive enrollment is absent",
            ));
        }
        EnrollmentOpen::Present(writer) => writer,
    };
    let current = observe_local_active(writer.current(), binding)?;
    if current.enrollment_head != expected_head
        || current.verification_digest != verification_digest
        || current.sync != LocalActiveSync::Idle
    {
        return Err(VerifiedLocalCompositionError::StaleEvidence(
            "committed LocalActive head is not the expected handoff predecessor",
        ));
    }
    if current.handoff == target {
        drop(writer);
        return reopen_committed_local_active(root, binding, expected_head, verification_digest);
    }
    // The anchor is carried through unchanged from the record this writer just
    // reread, so a handoff can never restate, advance, or drop it.
    writer.transition_at_cut(
        expected_head,
        local_active_lifecycle(
            writer.current().record.lifecycle(),
            verification_digest,
            current.anchor,
            target,
        ),
        cut,
    )?;
    let advanced = writer.current().digest;
    drop(writer);
    let reopened = reopen_committed_local_active(root, binding, advanced, verification_digest)?;
    if reopened.handoff != target {
        return Err(VerifiedLocalCompositionError::StaleEvidence(
            "committed LocalActive handoff state changed during the transition",
        ));
    }
    Ok(reopened)
}

/// A writable enrollment session retained for one promoted runtime lifetime.
///
/// The sparse-oplog runtime used to reopen the whole enrollment journal on
/// every single admission: resolve and open the directory tree, enumerate the
/// namespace, stat the lease, read and byte-compare the authority claim, and
/// walk the authenticated record chain back to its checkpoint — twice, because
/// the promoted binding proof and the mutation permit each did it. That is
/// per-keystroke work whose cost is set by the enrollment's checkpoint phase,
/// not by anything the keystroke changed.
///
/// This value acquires the [`EnrollmentLease`] exactly once and retains the
/// directory, lease, and authority capabilities for the whole promoted runtime.
/// The documented global lock order is unchanged and is why the session must be
/// acquired first: enrollment lease, then archive/engine lease, then graph and
/// process-local locks. Because the lease is exclusive and retained, every
/// journal mutation in the promoted path must borrow this session; a nested
/// [`EnrollmentWriter::open_existing`] would contend with its own process.
///
/// Admission cost is then split honestly:
///
/// * the *cheap* check is [`Self::revalidate`] with an unchanged head — one
///   bounded read of the fixed-size head file, and nothing else;
/// * the *full* check is the complete authenticated reopen above, performed
///   only when the committed head actually changed, and unconditionally at
///   every open, handoff, and recovery boundary.
///
/// An unchanged head is exact authority, not a heuristic: the head names one
/// content-addressed record whose bytes are verified against that digest on
/// read, and that record's digest commits to its complete hash-linked ancestry,
/// its binding, its lease resource, and its authenticated checkpoint. There is
/// no state the full reopen could observe that an identical head permits to
/// differ.
pub(crate) struct RetainedEnrollmentSession {
    writer: EnrollmentWriter,
    verification_digest: ContentDigest,
    committed: CommittedLocalActive,
    /// Bumped on every full authenticated revalidation and every journal
    /// mutation. An admission window captures it, so a window minted before a
    /// lifecycle change can never authorize work after it.
    binding_generation: u64,
    #[cfg(test)]
    full_revalidations: usize,
}

impl RetainedEnrollmentSession {
    /// Acquire the exclusive enrollment lease and perform the complete
    /// authenticated open for one committed `LocalActive` verification digest.
    ///
    /// This performs no proof revalidation: it is the runtime enrollment/session
    /// capability, not the activation gate. It still refuses every lifecycle
    /// other than `LocalActive`, every non-`Idle` sync state, and every other
    /// verification digest, so a blocked, published, rolled-back, or foreign
    /// enrollment can never retain a session.
    pub(crate) fn open(
        root: &EnrollmentApplicationRoot,
        binding: &EnrollmentBindingV1,
        verification_digest: ContentDigest,
    ) -> Result<Self, VerifiedLocalCompositionError> {
        binding.validate_internal()?;
        let writer = match EnrollmentWriter::open_existing(root, binding)? {
            EnrollmentOpen::Absent => {
                return Err(VerifiedLocalCompositionError::WrongLifecycle(
                    "LocalActive enrollment is absent",
                ));
            }
            EnrollmentOpen::Present(writer) => writer,
        };
        let committed = observe_local_active(writer.current(), binding)?;
        require_session_record(&committed, verification_digest)?;
        Ok(Self {
            writer,
            verification_digest,
            committed,
            binding_generation: 0,
            #[cfg(test)]
            full_revalidations: 0,
        })
    }

    pub(crate) const fn committed(&self) -> &CommittedLocalActive {
        &self.committed
    }

    pub(crate) const fn verification_digest(&self) -> ContentDigest {
        self.verification_digest
    }

    /// The session-local binding generation. It changes on every full
    /// authenticated revalidation and every journal mutation.
    pub(crate) const fn binding_generation(&self) -> u64 {
        self.binding_generation
    }

    /// The cheap exact head-digest check: one bounded read of the fixed-size
    /// committed head file, compared against the retained committed head.
    pub(crate) fn committed_head_is_unchanged(&self) -> Result<bool, EnrollmentError> {
        Ok(self.writer.committed_head()? == Some(self.committed.enrollment_head))
    }

    /// Revalidate the committed record for one admission.
    ///
    /// An unchanged head returns the retained record without one namespace
    /// enumeration, lease reacquisition, authority-claim reread, or record
    /// read. Any observed change forces the complete authenticated reopen.
    pub(crate) fn revalidate(
        &mut self,
    ) -> Result<&CommittedLocalActive, VerifiedLocalCompositionError> {
        if self.committed_head_is_unchanged()? {
            return Ok(&self.committed);
        }
        self.reauthenticate()
    }

    /// The complete authenticated reopen, over the retained capabilities.
    ///
    /// Every open, handoff, and recovery boundary calls this unconditionally,
    /// and so does [`Self::revalidate`] the moment the head is not exactly the
    /// retained one.
    pub(crate) fn reauthenticate(
        &mut self,
    ) -> Result<&CommittedLocalActive, VerifiedLocalCompositionError> {
        let committed = {
            let snapshot = self.writer.reauthenticate()?;
            let binding = snapshot.record.binding.clone();
            observe_local_active(snapshot, &binding)?
        };
        require_session_record(&committed, self.verification_digest)?;
        self.committed = committed;
        self.binding_generation = self.binding_generation.saturating_add(1);
        #[cfg(test)]
        {
            self.full_revalidations = self.full_revalidations.saturating_add(1);
        }
        Ok(&self.committed)
    }

    /// Durably move this session's committed `LocalActive` record between
    /// handoff states, on the retained lease.
    ///
    /// This is the retained-session form of
    /// [`transition_local_active_handoff`]: same narrow compare-and-swap, same
    /// carried-through immutable anchor, same fresh committed-head proof — but
    /// it borrows the session instead of opening a second writer, which would
    /// contend with the lease this session already holds.
    ///
    /// A handoff is a lifecycle boundary, so it is bracketed by full
    /// authenticated revalidations rather than by the cheap head check.
    pub(crate) fn transition_handoff(
        &mut self,
        target: LocalActiveHandoff,
    ) -> Result<&CommittedLocalActive, VerifiedLocalCompositionError> {
        self.reauthenticate()?;
        if self.committed.handoff == target {
            return Ok(&self.committed);
        }
        let expected_head = self.committed.enrollment_head;
        let lifecycle = local_active_lifecycle(
            self.writer.current().record.lifecycle(),
            self.verification_digest,
            self.committed.anchor,
            target,
        );
        self.writer.transition(expected_head, lifecycle)?;
        self.reauthenticate()?;
        if self.committed.handoff != target {
            return Err(VerifiedLocalCompositionError::StaleEvidence(
                "committed LocalActive handoff state changed during the transition",
            ));
        }
        Ok(&self.committed)
    }

    /// Durably replace one *other* session's committed `Unsafe` handoff with
    /// this process's own, on the retained lease.
    ///
    /// This is the enrollment half of an archive-lease-proved crash takeover.
    /// It is deliberately narrower than [`Self::transition_handoff`]: that one
    /// settles this session's own record, while this one adopts a record whose
    /// owner is a different session, so it must name the predecessor it proved
    /// and refuse anything else.
    ///
    /// The compare-and-swap is exact on *both* the head and the predecessor
    /// session, and it is bracketed by complete authenticated revalidations:
    ///
    /// * the reauthenticated record must still be exactly
    ///   `Unsafe { predecessor.session_id }` at exactly
    ///   `predecessor.enrollment_head`, so a newcomer that observed the crashed
    ///   owner and then lost the race to another newcomer fails here without
    ///   writing one byte, rather than overwriting the winner;
    /// * the successor is `Unsafe { new_session_id }` and never `Safe`: a crash
    ///   takeover recovers an unsafe runtime and may not synthesize a clean
    ///   handoff;
    /// * the immutable activation anchor is carried through from the record
    ///   this session just reread, exactly as an ordinary handoff does.
    ///
    /// Ownership of the archive-rooted workspace runtime lease is what proves
    /// the predecessor process is gone, so `workspace` is required rather than
    /// documented: [`WorkspaceRuntimeProof`] has no constructor outside the
    /// sealed lease module, cannot be cloned into a longer life, and cannot
    /// outlive the lease it borrows. A caller that does not hold the archive
    /// lease right now cannot call this function at all. Binding that proof to
    /// this exact archive and workspace is [`super::local_active`]'s job, which
    /// it does before it recovers one byte of runtime state; this function's own
    /// job is the exact enrollment compare-and-swap.
    pub(crate) fn take_over_unsafe_handoff(
        &mut self,
        authorization: AuthenticatedUnsafePredecessor<'_>,
        new_session_id: SessionId,
    ) -> Result<&CommittedLocalActive, VerifiedLocalCompositionError> {
        self.take_over_unsafe_handoff_at_cut(authorization, new_session_id, CommitCut::None)
    }

    #[cfg(test)]
    pub(crate) fn take_over_unsafe_handoff_at_cut_for_test(
        &mut self,
        authorization: AuthenticatedUnsafePredecessor<'_>,
        new_session_id: SessionId,
        cut: CommitCut,
    ) -> Result<&CommittedLocalActive, VerifiedLocalCompositionError> {
        self.take_over_unsafe_handoff_at_cut(authorization, new_session_id, cut)
    }

    /// Mint the sealed, borrowed authorization one crash-takeover
    /// compare-and-swap consumes.
    ///
    /// This is the only constructor of [`AuthenticatedUnsafePredecessor`], and
    /// it is deliberately a method on the *retained session*: minting one
    /// therefore already requires the exclusive device-local enrollment lease,
    /// a complete authenticated reread of the hash-linked chain, and a live
    /// archive-rooted workspace lease that authorizes this exact archive
    /// directory — not merely some lease whose workspace id happens to match.
    ///
    /// `observed` cannot widen anything. It can only be the record this process
    /// already authenticated from the chain (see
    /// [`UnsafeHandoffPredecessor::observed_in`]), and the freshly reread record
    /// must still be exactly that, so a newcomer that lost the race to another
    /// newcomer is refused here, before the swap.
    pub(crate) fn authenticate_unsafe_predecessor<'lease>(
        &mut self,
        workspace: &'lease WorkspaceRuntimeProof<'lease>,
        archive: &ObjectStore,
        observed: UnsafeHandoffPredecessor,
    ) -> Result<AuthenticatedUnsafePredecessor<'lease>, VerifiedLocalCompositionError> {
        workspace
            .authorize_archive(archive, archive.workspace_id())
            .map_err(|error| {
                VerifiedLocalCompositionError::ProofBinding(format!(
                    "the workspace runtime lease does not authorize this archive: {error}"
                ))
            })?;
        self.reauthenticate()?;
        if self.committed.enrollment_head != observed.enrollment_head
            || self.committed.handoff
                != (LocalActiveHandoff::Unsafe {
                    session_id: observed.session_id,
                })
        {
            return Err(VerifiedLocalCompositionError::StaleEvidence(
                "committed LocalActive record is not the authenticated unsafe predecessor this \
                 takeover proved",
            ));
        }
        Ok(AuthenticatedUnsafePredecessor {
            workspace,
            predecessor: observed,
        })
    }

    fn take_over_unsafe_handoff_at_cut(
        &mut self,
        authorization: AuthenticatedUnsafePredecessor<'_>,
        new_session_id: SessionId,
        cut: CommitCut,
    ) -> Result<&CommittedLocalActive, VerifiedLocalCompositionError> {
        let AuthenticatedUnsafePredecessor {
            workspace,
            predecessor,
        } = authorization;
        // The authorization proves the lease authorizes its own archive; this
        // proves that archive is *this enrollment's* workspace.
        if workspace.workspace_id() != self.committed.binding().workspace_id() {
            return Err(VerifiedLocalCompositionError::ProofMismatch(
                "the workspace runtime lease is not this enrollment's workspace",
            ));
        }
        if predecessor.session_id == new_session_id {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "a crash takeover must adopt a session other than the crashed one",
            ));
        }
        let target = LocalActiveHandoff::Unsafe {
            session_id: new_session_id,
        };
        // A takeover is a lifecycle boundary, so it is bracketed by complete
        // authenticated revalidations rather than by the cheap head check.
        self.reauthenticate()?;
        if self.committed.enrollment_head != predecessor.enrollment_head
            || self.committed.handoff
                != (LocalActiveHandoff::Unsafe {
                    session_id: predecessor.session_id,
                })
        {
            return Err(VerifiedLocalCompositionError::StaleEvidence(
                "committed LocalActive record is not the authenticated unsafe predecessor this \
                 takeover proved",
            ));
        }
        let lifecycle = local_active_lifecycle(
            self.writer.current().record.lifecycle(),
            self.verification_digest,
            self.committed.anchor,
            target,
        );
        self.writer
            .transition_at_cut(predecessor.enrollment_head, lifecycle, cut)?;
        self.reauthenticate()?;
        if self.committed.handoff != target {
            return Err(VerifiedLocalCompositionError::StaleEvidence(
                "committed LocalActive handoff state changed during the crash takeover",
            ));
        }
        Ok(&self.committed)
    }

    /// How many complete authenticated reopens this session has performed.
    #[cfg(test)]
    pub(crate) const fn full_revalidations(&self) -> usize {
        self.full_revalidations
    }
}

/// The exact committed `Unsafe` record one crash takeover replaces.
///
/// Neither field is caller-chosen. [`Self::observed_in`] is the only
/// constructor, and it reads both out of a [`PromotedBootstrapAnchor`], which
/// is itself self-authenticated from the hash-linked enrollment chain — so a
/// caller cannot name a predecessor it did not first authenticate. On its own
/// this value still authorizes nothing: the compare-and-swap consumes an
/// [`AuthenticatedUnsafePredecessor`], which only
/// [`RetainedEnrollmentSession::authenticate_unsafe_predecessor`] can mint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnsafeHandoffPredecessor {
    enrollment_head: ContentDigest,
    session_id: SessionId,
}

impl UnsafeHandoffPredecessor {
    /// The committed `Unsafe` owner this authenticated anchor names, if any.
    ///
    /// `None` means the record is `Safe`, which is not a crash predecessor and
    /// must never be taken over.
    pub(crate) fn observed_in(anchor: &PromotedBootstrapAnchor) -> Option<Self> {
        match anchor.committed().handoff() {
            LocalActiveHandoff::Unsafe { session_id } => Some(Self {
                enrollment_head: anchor.committed().enrollment_head(),
                session_id,
            }),
            LocalActiveHandoff::Safe => None,
        }
    }

    /// The crashed owner this predecessor names.
    pub(crate) const fn session_id(&self) -> SessionId {
        self.session_id
    }
}

/// Sealed, borrowed authorization for exactly one crash-takeover
/// compare-and-swap.
///
/// It has private fields and no constructor outside
/// [`RetainedEnrollmentSession::authenticate_unsafe_predecessor`], it is not
/// `Clone`, and it borrows the workspace proof — which itself borrows the live
/// archive lease — so it can neither be forged, copied, nor outlive the archive
/// ownership that justified it. Assembling one from plain identifiers plus any
/// same-workspace lease is not possible: the mint requires the exclusive
/// enrollment lease, a fresh authenticated reread, and a lease rooted at this
/// exact archive directory.
pub(crate) struct AuthenticatedUnsafePredecessor<'lease> {
    workspace: &'lease WorkspaceRuntimeProof<'lease>,
    predecessor: UnsafeHandoffPredecessor,
}

impl fmt::Debug for AuthenticatedUnsafePredecessor<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedUnsafePredecessor")
            .field("workspace_id", &self.workspace.workspace_id())
            .field("predecessor", &self.predecessor)
            .finish()
    }
}

/// Every invariant a retained runtime session requires of a committed record.
fn require_session_record(
    committed: &CommittedLocalActive,
    verification_digest: ContentDigest,
) -> Result<(), VerifiedLocalCompositionError> {
    if committed.verification_digest != verification_digest {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "committed LocalActive record binds another verification digest",
        ));
    }
    if committed.sync != LocalActiveSync::Idle {
        return Err(VerifiedLocalCompositionError::WrongLifecycle(
            "LocalActive runtime authority requires an Idle sync state",
        ));
    }
    Ok(())
}

/// Persist the exact `Unsafe { session } + Published` exclusion state for one
/// committed `LocalActive` record.
///
/// Test-only. The recovery packet contents are not what a runtime reopen
/// authenticates; the non-`Idle` sync state is, and every runtime boundary must
/// refuse it.
#[cfg(test)]
pub(crate) fn publish_local_active_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected_head: ContentDigest,
    verification_digest: ContentDigest,
    session_id: SessionId,
) -> Result<ContentDigest, VerifiedLocalCompositionError> {
    let import_id = ImportId::from_digest([31; 32]);
    let packet = PublishedRecoveryPacketV1::new(
        BatchId::for_import(import_id),
        import_id,
        ContentDigest::of(b"tine/local-active-published-test-manifest"),
        binding.archive_resource_id,
        AcceptedFrontierAnchorV1 {
            acceptance_sequence: 0,
            accepted_frontier_state_digest: ContentDigest::of(b"tine/published-test-frontier"),
            history_generation: 0,
            history_root: ContentDigest::of(b"tine/published-test-history-root"),
        },
    )?;
    let mut writer = match EnrollmentWriter::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "LocalActive enrollment is absent",
            ));
        }
        EnrollmentOpen::Present(writer) => writer,
    };
    let anchor = observe_local_active(writer.current(), binding)?.anchor;
    Ok(writer
        .transition(
            expected_head,
            EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
                verification_digest,
                anchor,
                handoff: HandoffV1::Unsafe { session_id },
                exclusion: LocalExclusionV1::Published { packet },
            }),
        )?
        .digest())
}

/// Fail the current enrollment closed at an exact prior head.
#[cfg(test)]
pub(crate) fn block_current_for_test(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected_current: ContentDigest,
    reason_code: String,
) -> Result<ContentDigest, EnrollmentError> {
    let mut writer = match EnrollmentWriter::open_existing(root, binding)? {
        EnrollmentOpen::Absent => return Err(EnrollmentError::MalformedHead),
        EnrollmentOpen::Present(writer) => writer,
    };
    let evidence = ContentDigest::of(reason_code.as_bytes());
    Ok(writer
        .block_current(expected_current, reason_code, evidence)?
        .digest())
}

/// Reopen the committed head for a live runtime authority.
///
/// This deliberately performs no proof revalidation: it is the per-admission
/// enrollment/session check, not the activation gate. It still refuses every
/// lifecycle other than `LocalActive` and every other verification digest, so a
/// blocked, rolled-back, or foreign enrollment can never admit work.
pub(crate) fn reopen_committed_local_active_for_session(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    verification_digest: ContentDigest,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let reader = match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "LocalActive enrollment is absent",
            ));
        }
        EnrollmentOpen::Present(reader) => reader,
    };
    let committed = observe_local_active(reader.current(), binding)?;
    if committed.verification_digest != verification_digest {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "committed LocalActive record binds another verification digest",
        ));
    }
    Ok(committed)
}

/// Reopen the committed head and require it to be exactly this `LocalActive`
/// record. This performs no proof revalidation and is only used to prove a
/// handoff transition this process just persisted.
fn reopen_committed_local_active(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    expected_head: ContentDigest,
    verification_digest: ContentDigest,
) -> Result<CommittedLocalActive, VerifiedLocalCompositionError> {
    let reader = match EnrollmentReader::open_existing(root, binding)? {
        EnrollmentOpen::Absent => {
            return Err(VerifiedLocalCompositionError::WrongLifecycle(
                "LocalActive enrollment is absent",
            ));
        }
        EnrollmentOpen::Present(reader) => reader,
    };
    let committed = observe_local_active(reader.current(), binding)?;
    if committed.enrollment_head != expected_head
        || committed.verification_digest != verification_digest
        || committed.sync != LocalActiveSync::Idle
    {
        return Err(VerifiedLocalCompositionError::StaleEvidence(
            "committed LocalActive head is not the record this session persisted",
        ));
    }
    Ok(committed)
}

fn freshly_validate_verified_local(
    enrollment_binding: &EnrollmentBindingV1,
    preparation_id: PreparationId,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<VerifiedLocalV1, VerifiedLocalCompositionError> {
    validate_verified_local_binding(enrollment_binding, proofs)?;
    let reopened_store = ObjectStore::open(
        proofs.accepted_authority.store().root_path(),
        proofs.verified_publication.workspace_id(),
    )
    .map_err(BootstrapStreamingImportError::from)?;
    let fresh_authority =
        reopen_inactive_bootstrap_accepted_authority(proofs.verified_publication, reopened_store)?;
    if fresh_authority.binding() != proofs.accepted_authority.binding() {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "fresh accepted authority differs from the supplied retained authority",
        ));
    }
    // The reopened store above only authenticates the physical archive control
    // identity. Independently authenticate the persisted archive-resource claim
    // so a binding carrying another archive's valid resource id cannot advance.
    fresh_authority
        .store()
        .validate_enrolled_archive_resource_id(enrollment_binding.archive_resource_id())
        .map_err(|error| {
            VerifiedLocalCompositionError::ProofBinding(format!(
                "persisted archive resource claim does not authenticate the enrollment binding: {error}"
            ))
        })?;
    let fresh_backup =
        verify_migration_source_backup(proofs.roots, proofs.prepared, proofs.verified_publication)?;
    if &fresh_backup != proofs.source_backup {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "fresh backup differs from the supplied backup proof",
        ));
    }
    proofs
        .sqlite
        .database
        .freshly_verify_inactive_bootstrap(proofs.accepted_authority, proofs.sqlite_projection)?;
    let fresh_shadow = verify_inactive_bootstrap_shadow_projection(
        proofs.graph,
        proofs.roots,
        proofs.prepared,
        proofs.verified_publication,
        &fresh_backup,
        proofs.accepted_authority,
        proofs.sqlite_projection,
    )?;
    if &fresh_shadow != proofs.shadow_projection {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "fresh shadow projection differs from the supplied shadow proof",
        ));
    }

    verified_local_from_validated_proofs(
        enrollment_binding,
        preparation_id,
        proofs,
        &fresh_authority,
        &fresh_backup,
        &fresh_shadow,
    )
}

#[allow(clippy::too_many_arguments)]
fn verified_local_from_validated_proofs(
    enrollment_binding: &EnrollmentBindingV1,
    preparation_id: PreparationId,
    proofs: &VerifiedLocalProofSet<'_>,
    validated_authority: &InactiveBootstrapAcceptedAuthority,
    validated_backup: &VerifiedSourceBackup,
    validated_shadow: &VerifiedShadowProjection,
) -> Result<VerifiedLocalV1, VerifiedLocalCompositionError> {
    let authority = validated_authority.binding();
    let frontier = authority.accepted_frontier();
    let aggregate = validated_authority.publication().aggregate();
    let bootstrap_batch_id = aggregate.parts().last().map(|part| part.batch_id());
    let bootstrap_terminal_part_id = authority
        .predecessor_terminal()
        .map(|part| ContentDigest::from_bytes(*part.as_bytes()));
    let reference_policy_digest = proofs
        .verified_publication
        .reference_catalog_policy()
        .digest()
        .map_err(|error| VerifiedLocalCompositionError::ProofBinding(error.to_string()))?;
    let proof_binding_digest = verified_local_proof_binding_digest(
        enrollment_binding,
        proofs,
        validated_authority,
        validated_backup,
        validated_shadow,
        reference_policy_digest,
        bootstrap_batch_id,
    )?;
    let source = proofs.prepared.source_capture();
    let verified = VerifiedLocalV1 {
        preparation_id,
        source_inventory_digest: ContentDigest::from_bytes(
            *source.inventory_description().sha256(),
        ),
        source_file_count: validated_shadow.file_count(),
        source_chunk_count: validated_shadow.chunk_count(),
        source_total_bytes: validated_shadow.total_bytes(),
        backup_manifest: validated_backup.manifest(),
        backup_restore_proof: validated_backup.restore_proof(),
        backup_evidence_digest: validated_backup.evidence_digest(),
        bootstrap_import_id: ContentDigest::from_bytes(*authority.import_id().as_bytes()),
        bootstrap_part_count: authority.part_count(),
        bootstrap_terminal_part_id,
        bootstrap_batch_id,
        accepted_frontier_anchor: AcceptedFrontierAnchorV1 {
            acceptance_sequence: frontier.acceptance_sequence(),
            accepted_frontier_state_digest: frontier.state_digest(),
            history_generation: authority.history_generation(),
            history_root: authority.history_root(),
        },
        accepted_history_record_count: authority.cold_record_count(),
        catalog_row_count: validated_shadow.catalog_binding().catalog_rows(),
        sqlite_accepted_batch_count: proofs.sqlite_projection.accepted_batch_count(),
        sqlite_semantic_projection_digest: proofs.sqlite_projection.semantic_projection_digest(),
        sqlite_materialized_row_digest: proofs.sqlite_projection.materialized_row_digest(),
        staged_projection_manifest: validated_shadow.manifest(),
        staged_projection_proof: validated_shadow.proof(),
        staged_file_count: validated_shadow.staged_file_count(),
        staged_total_bytes: validated_shadow.staged_total_bytes(),
        byte_compare_digest: validated_shadow.staged_inventory_digest(),
        shadow_evidence_digest: validated_shadow.evidence_digest(),
        proof_binding_digest,
    };
    verified.validate_fields()?;
    Ok(verified)
}

fn validate_verified_local_binding(
    enrollment: &EnrollmentBindingV1,
    proofs: &VerifiedLocalProofSet<'_>,
) -> Result<(), VerifiedLocalCompositionError> {
    enrollment.validate_internal()?;
    let accepted = proofs.accepted_authority.binding();
    let storage = accepted.storage_binding();
    let graph_resource = proofs.graph.canonical_resource_id()?;
    let scope = proofs.graph.graph_text_scope_binding()?;
    if enrollment.workspace_id != accepted.workspace_id()
        || enrollment.lineage_digest != accepted.lineage_digest()
        || enrollment.catalog_document_id != proofs.verified_publication.catalog_document_id()
        || enrollment.endpoint_id != storage.endpoint.endpoint_id()
        || enrollment.device_id != storage.endpoint.device_id()
        || enrollment.graph_resource_id != graph_resource
        || enrollment.graph_resource_id != accepted.graph_resource()
        || enrollment.receipt_store_id != storage.receipt_store_id
        || enrollment.graph_text_scope_binding != scope
        || proofs.roots.graph_resource() != graph_resource
        || proofs.source_backup.backup_root_identity() != proofs.roots.root_identity()
        || proofs.shadow_projection.physical_root_identity() != proofs.roots.root_identity()
    {
        return Err(VerifiedLocalCompositionError::ProofMismatch(
            "enrollment resources do not match the retained proof roots",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn verified_local_proof_binding_digest(
    enrollment: &EnrollmentBindingV1,
    proofs: &VerifiedLocalProofSet<'_>,
    authority: &InactiveBootstrapAcceptedAuthority,
    backup: &VerifiedSourceBackup,
    shadow: &VerifiedShadowProjection,
    reference_policy_digest: ContentDigest,
    bootstrap_batch_id: Option<BatchId>,
) -> Result<ContentDigest, VerifiedLocalCompositionError> {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/verified-local-proof-binding/v1\0");
    hash_variable(
        &mut hasher,
        &serde_json::to_vec(enrollment)
            .map_err(|error| VerifiedLocalCompositionError::ProofBinding(error.to_string()))?,
    );
    let binding = authority.binding();
    hash_authority_binding(&mut hasher, binding)?;
    hasher.update(proofs.roots.root_identity().as_bytes());
    hasher.update(proofs.graph.canonical_resource_id()?.as_bytes());

    let capture = proofs.prepared.source_capture();
    hash_description(&mut hasher, capture.capture_identity()?);
    hash_description(&mut hasher, capture.inventory_description());
    hash_description(&mut hasher, capture.entries_description());
    hash_description(&mut hasher, capture.chunks_description());
    hasher.update(capture.source_file_count().to_be_bytes());
    hasher.update(capture.source_chunk_count().to_be_bytes());

    hasher.update(backup.backup_root_identity().as_bytes());
    hasher.update(backup.publication_id());
    hasher.update(backup.aggregate_digest());
    hash_description(&mut hasher, backup.source_inventory());
    hasher.update(backup.file_count().to_be_bytes());
    hasher.update(backup.total_bytes().to_be_bytes());
    hash_description(&mut hasher, backup.manifest());
    hash_description(&mut hasher, backup.restore_proof());
    hasher.update(backup.evidence_digest().as_bytes());

    let frontier = binding.accepted_frontier();
    hasher.update(frontier.state_digest().as_bytes());
    hasher.update(frontier.acceptance_sequence().to_be_bytes());
    hasher.update(frontier.document_count().to_be_bytes());
    hasher.update(frontier.retained_bytes_total().to_be_bytes());
    hasher.update(frontier.document_map_root_digest().as_bytes());
    hasher.update(frontier.batch_map_root_digest().as_bytes());
    hasher.update(
        frontier
            .reference_catalog_root()
            .external_digest()
            .map_err(|error| VerifiedLocalCompositionError::ProofBinding(error.to_string()))?
            .as_bytes(),
    );
    hasher.update(reference_policy_digest.as_bytes());

    let sqlite = proofs.sqlite_projection;
    hasher.update(sqlite.claim().workspace_id().as_uuid().as_bytes());
    hasher.update(sqlite.claim().lineage_digest().as_bytes());
    hasher.update(sqlite.frontier_root().state_digest().as_bytes());
    hasher.update(sqlite.accepted_batch_count().to_be_bytes());
    hasher.update(sqlite.semantic_projection_digest().as_bytes());
    hasher.update(sqlite.materialized_row_digest().as_bytes());

    hasher.update(shadow.physical_root_identity().as_bytes());
    hasher.update(shadow.publication_id().as_bytes());
    hash_description(&mut hasher, shadow.source_capture());
    hasher.update(shadow.file_count().to_be_bytes());
    hasher.update(shadow.chunk_count().to_be_bytes());
    hasher.update(shadow.directory_count().to_be_bytes());
    hasher.update(shadow.total_bytes().to_be_bytes());
    let catalog = shadow.catalog_binding();
    hasher.update(catalog.accepted_frontier().as_bytes());
    hasher.update(catalog.history_generation().to_be_bytes());
    hasher.update(catalog.history_root().as_bytes());
    hasher.update(catalog.catalog_root().as_bytes());
    hasher.update(catalog.catalog_rows().to_be_bytes());
    hash_description(&mut hasher, shadow.manifest());
    hash_description(&mut hasher, shadow.proof());
    hasher.update(shadow.staged_inventory_digest().as_bytes());
    hasher.update(shadow.staged_file_count().to_be_bytes());
    hasher.update(shadow.staged_total_bytes().to_be_bytes());
    hasher.update(shadow.evidence_digest().as_bytes());
    hasher.update(shadow.schema_binding_digest().as_bytes());
    match bootstrap_batch_id {
        Some(batch_id) => {
            hasher.update([1]);
            hasher.update(batch_id.as_uuid().as_bytes());
        }
        None => hasher.update([0]),
    }
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn hash_authority_binding(
    hasher: &mut Sha256,
    binding: &InactiveBootstrapAcceptedAuthorityBinding,
) -> Result<(), VerifiedLocalCompositionError> {
    hasher.update(binding.workspace_id().as_uuid().as_bytes());
    hasher.update(binding.lineage_digest().as_bytes());
    hasher.update(binding.graph_resource().as_bytes());
    hasher.update(binding.publication_id().as_bytes());
    hasher.update(binding.aggregate_digest().as_bytes());
    hasher.update(binding.import_id().as_bytes());
    hasher.update(binding.part_count().to_be_bytes());
    match binding.predecessor_terminal() {
        Some(part) => {
            hasher.update([1]);
            hasher.update(part.as_bytes());
        }
        None => hasher.update([0]),
    }
    hash_variable(
        hasher,
        &postcard::to_allocvec(binding.engine_binding())
            .map_err(|error| VerifiedLocalCompositionError::ProofBinding(error.to_string()))?,
    );
    let storage = binding.storage_binding();
    hasher.update(storage.endpoint.endpoint_id().as_uuid().as_bytes());
    hasher.update(storage.endpoint.device_id().as_uuid().as_bytes());
    hasher.update(storage.endpoint.graph_resource_id().as_bytes());
    hasher.update(storage.receipt_store_id.as_bytes());
    hasher.update(binding.bootstrap_binding().publication_id().as_bytes());
    hasher.update(binding.bootstrap_binding().aggregate_digest().as_bytes());
    hasher.update(binding.bootstrap_binding().part_count().to_be_bytes());
    hash_variable(
        hasher,
        &binding.bootstrap_binding().final_frontier().encode(),
    );
    hasher.update(binding.archive_identity().binding_digest().as_bytes());
    hasher.update(binding.history_generation().to_be_bytes());
    hasher.update(binding.history_root().as_bytes());
    hasher.update(binding.cold_record_count().to_be_bytes());
    Ok(())
}

fn hash_description(hasher: &mut Sha256, description: BlobDescription) {
    hasher.update(description.sha256());
    hasher.update(description.byte_length().to_be_bytes());
}

fn hash_variable(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn persist_record_and_head(
    directories: &EnrollmentDirectories,
    lease: &EnrollmentLease,
    record: &EnrollmentRecordV1,
    cut: CommitCut,
) -> Result<EnrollmentSnapshot, EnrollmentError> {
    let bytes = canonical_record_bytes(record)?;
    let digest = ContentDigest::from_bytes(Sha256::digest(&bytes).into());
    publish_record(&directories.records, lease, digest, &bytes, cut)?;

    let temp_name = format!("{HEAD_TEMP_PREFIX}{}", Uuid::new_v4());
    lease.validate_current()?;
    let mut temp = create_new_regular(&directories.enrollment, &temp_name)?;
    validate_authoritative_file(&temp, "enrollment head temporary file")?;
    inject_crash_cut(
        cut,
        CommitCut::AfterHeadTempCreate,
        "after_head_temp_create",
    )?;
    lease.validate_current()?;
    temp.write_all(format!("{digest}\n").as_bytes())?;
    inject_crash_cut(cut, CommitCut::AfterHeadWrite, "after_head_write")?;
    lease.validate_current()?;
    temp.sync_all()?;
    inject_crash_cut(cut, CommitCut::AfterHeadFileSync, "after_head_file_sync")?;

    lease.validate_current()?;
    reject_unsafe_head_target(&directories.enrollment)?;
    lease.validate_current()?;
    directories
        .enrollment
        .rename(&temp_name, &directories.enrollment, HEAD_FILE)?;
    inject_crash_cut(cut, CommitCut::AfterHeadReplace, "after_head_replace")?;
    lease.validate_current()?;
    sync_dir_required(&directories.enrollment)
        .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    inject_crash_cut(
        cut,
        CommitCut::AfterEnrollmentDirectorySync,
        "after_enrollment_directory_sync",
    )?;
    Ok(EnrollmentSnapshot {
        digest,
        record: record.clone(),
    })
}

#[cfg(test)]
fn inject_crash_cut(
    actual: CommitCut,
    expected: CommitCut,
    label: &'static str,
) -> Result<(), EnrollmentError> {
    if actual == expected {
        Err(EnrollmentError::InjectedCrashCut(label))
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
fn inject_crash_cut(
    _actual: CommitCut,
    _expected: CommitCut,
    _label: &'static str,
) -> Result<(), EnrollmentError> {
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRecordWireV5 {
    schema_version: u32,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: EnrollmentBindingV1,
    lifecycle: EnrollmentLifecycleV1,
    checkpoint: Option<AuthenticatedCheckpointV1>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRecordWireV6 {
    schema_version: u32,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: EnrollmentBindingV1,
    lifecycle: EnrollmentLifecycleV1,
    checkpoint: Option<IntegrityCheckpointV3>,
}

// The normalized model is deliberately not deserializable: wire decoding must
// select exactly one strict schema before normalization.  Test mutation helpers
// still need an unchecked canonical-shaped serializer to manufacture malformed
// bytes for the decoder's negative cases.
impl Serialize for EnrollmentRecordV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.schema_version {
            ENROLLMENT_RECORD_SCHEMA_V5 => EnrollmentRecordWireV5 {
                schema_version: self.schema_version,
                generation: self.generation,
                previous: self.previous,
                history_accumulator: self.history_accumulator,
                lease_resource_id: self.lease_resource_id,
                binding: self.binding.clone(),
                lifecycle: self.lifecycle.clone(),
                checkpoint: match &self.checkpoint {
                    Some(EnrollmentCheckpoint::LegacyV2(checkpoint)) => Some(checkpoint.clone()),
                    None => None,
                    _ => return Err(serde::ser::Error::custom("illegal checkpoint pair")),
                },
            }
            .serialize(serializer),
            ENROLLMENT_RECORD_SCHEMA_VERSION => EnrollmentRecordWireV6 {
                schema_version: self.schema_version,
                generation: self.generation,
                previous: self.previous,
                history_accumulator: self.history_accumulator,
                lease_resource_id: self.lease_resource_id,
                binding: self.binding.clone(),
                lifecycle: self.lifecycle.clone(),
                checkpoint: match &self.checkpoint {
                    Some(EnrollmentCheckpoint::CurrentV3(checkpoint)) => Some(checkpoint.clone()),
                    None => None,
                    _ => return Err(serde::ser::Error::custom("illegal checkpoint pair")),
                },
            }
            .serialize(serializer),
            schema => Err(serde::ser::Error::custom(format!(
                "unsupported enrollment schema {schema}"
            ))),
        }
    }
}

fn canonical_record_bytes(record: &EnrollmentRecordV1) -> Result<Vec<u8>, EnrollmentError> {
    record.validate()?;
    let bytes = match record.schema_version {
        ENROLLMENT_RECORD_SCHEMA_V5 => {
            let checkpoint = match &record.checkpoint {
                Some(EnrollmentCheckpoint::LegacyV2(checkpoint)) => Some(checkpoint.clone()),
                None => None,
                _ => return Err(EnrollmentError::IllegalCheckpointPair),
            };
            serde_json::to_vec(&EnrollmentRecordWireV5 {
                schema_version: record.schema_version,
                generation: record.generation,
                previous: record.previous,
                history_accumulator: record.history_accumulator,
                lease_resource_id: record.lease_resource_id,
                binding: record.binding.clone(),
                lifecycle: record.lifecycle.clone(),
                checkpoint,
            })
        }
        ENROLLMENT_RECORD_SCHEMA_VERSION => {
            let checkpoint = match &record.checkpoint {
                Some(EnrollmentCheckpoint::CurrentV3(checkpoint)) => Some(checkpoint.clone()),
                None => None,
                _ => return Err(EnrollmentError::IllegalCheckpointPair),
            };
            serde_json::to_vec(&EnrollmentRecordWireV6 {
                schema_version: record.schema_version,
                generation: record.generation,
                previous: record.previous,
                history_accumulator: record.history_accumulator,
                lease_resource_id: record.lease_resource_id,
                binding: record.binding.clone(),
                lifecycle: record.lifecycle.clone(),
                checkpoint,
            })
        }
        schema => return Err(EnrollmentError::UnsupportedRecordSchema(schema)),
    }
    .map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    if bytes.len() > MAX_ENROLLMENT_RECORD_BYTES {
        return Err(EnrollmentError::RecordTooLarge(bytes.len()));
    }
    Ok(bytes)
}

fn decode_record(bytes: &[u8]) -> Result<EnrollmentRecordV1, EnrollmentError> {
    validate_json_bounds(bytes)?;
    reject_duplicate_json_fields(bytes)?;
    let probe: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
    let schema = probe
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| EnrollmentError::Decode("record schema_version is missing".into()))?;
    let schema = u32::try_from(schema).unwrap_or(u32::MAX);
    let lifecycle_state = probe
        .get("lifecycle")
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| EnrollmentError::Decode("record lifecycle state is missing".into()))?;
    if !matches!(
        lifecycle_state,
        "shadow_import"
            | "verified_local"
            | "local_active"
            | "share_prepared"
            | "joining"
            | "shared_active"
            | "blocked"
    ) {
        return Err(EnrollmentError::FutureUnsupportedLifecycle(
            lifecycle_state.to_owned(),
        ));
    }
    let record = match schema {
        ENROLLMENT_RECORD_SCHEMA_V5 => {
            let record: EnrollmentRecordWireV5 = serde_json::from_slice(bytes)
                .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
            EnrollmentRecordV1 {
                schema_version: record.schema_version,
                generation: record.generation,
                previous: record.previous,
                history_accumulator: record.history_accumulator,
                lease_resource_id: record.lease_resource_id,
                binding: record.binding,
                lifecycle: record.lifecycle,
                checkpoint: record.checkpoint.map(EnrollmentCheckpoint::LegacyV2),
            }
        }
        ENROLLMENT_RECORD_SCHEMA_VERSION => {
            let record: EnrollmentRecordWireV6 = serde_json::from_slice(bytes)
                .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
            EnrollmentRecordV1 {
                schema_version: record.schema_version,
                generation: record.generation,
                previous: record.previous,
                history_accumulator: record.history_accumulator,
                lease_resource_id: record.lease_resource_id,
                binding: record.binding,
                lifecycle: record.lifecycle,
                checkpoint: record.checkpoint.map(EnrollmentCheckpoint::CurrentV3),
            }
        }
        schema => return Err(EnrollmentError::UnsupportedRecordSchema(schema)),
    };
    record.validate()?;
    if canonical_record_bytes(&record)? != bytes {
        return Err(EnrollmentError::NonCanonicalRecord);
    }
    Ok(record)
}

#[derive(Clone, Copy)]
struct RejectDuplicateJsonFields;

impl<'de> DeserializeSeed<'de> for RejectDuplicateJsonFields {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for RejectDuplicateJsonFields {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded JSON without duplicate object fields")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON field {key:?}"
                )));
            }
            map.next_value_seed(self)?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(self)?.is_some() {}
        Ok(())
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }
}

fn reject_duplicate_json_fields(bytes: &[u8]) -> Result<(), EnrollmentError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    RejectDuplicateJsonFields
        .deserialize(&mut deserializer)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| EnrollmentError::Decode(error.to_string()))
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
    expected_lease_resource_id: ContentDigest,
    authority: &EnrollmentAuthorityMaterial,
) -> Result<Option<EnrollmentSnapshot>, EnrollmentError> {
    let Some(head) = read_head(&directories.enrollment)? else {
        return Ok(None);
    };
    let current = read_record(&directories.records, head)?;
    validate_record_authority(
        &current,
        expected_binding,
        expected_lease_resource_id,
        authority,
    )?;

    let mut seen = BTreeSet::new();
    let mut digest = head;
    let mut record = current.clone();
    for count in 0..MAX_ENROLLMENT_OPEN_CHAIN_RECORDS {
        if !seen.insert(digest) {
            return Err(EnrollmentError::ChainCycle);
        }
        if record.checkpoint.is_some() {
            authority.verify_checkpoint(&record)?;
            if record.previous.is_none() {
                validate_initial_record(&record)?;
            }
            return Ok(Some(EnrollmentSnapshot {
                digest: head,
                record: current,
            }));
        }
        match record.previous {
            None => return Err(EnrollmentError::MissingAuthenticatedCheckpoint),
            Some(previous_digest) => {
                let previous = read_record(&directories.records, previous_digest)?;
                validate_record_authority(
                    &previous,
                    expected_binding,
                    expected_lease_resource_id,
                    authority,
                )?;
                validate_record_link(previous_digest, &previous, &record)?;
                digest = previous_digest;
                record = previous;
            }
        }
        if count + 1 == MAX_ENROLLMENT_OPEN_CHAIN_RECORDS {
            return Err(EnrollmentError::MissingAuthenticatedCheckpoint);
        }
    }
    unreachable!("bounded chain loop returns at its limit")
}

fn validate_record_authority(
    record: &EnrollmentRecordV1,
    expected_binding: &EnrollmentBindingV1,
    expected_lease_resource_id: ContentDigest,
    authority: &EnrollmentAuthorityMaterial,
) -> Result<(), EnrollmentError> {
    record.binding.validate_exact(expected_binding)?;
    record.validate()?;
    if record.lease_resource_id != expected_lease_resource_id {
        return Err(EnrollmentError::LeaseResourceMismatch);
    }
    if record.checkpoint.is_some() {
        authority.verify_checkpoint(record)?;
    }
    Ok(())
}

fn read_head(directory: &Dir) -> Result<Option<ContentDigest>, EnrollmentError> {
    #[cfg(test)]
    count(&ENROLLMENT_HEAD_READS);
    #[cfg(test)]
    if FAIL_NEXT_ENROLLMENT_HEAD_READ.with(|fault| fault.replace(false)) {
        return Err(EnrollmentError::Io(
            "injected transient enrollment head read failure".into(),
        ));
    }
    let metadata = match directory.symlink_metadata(HEAD_FILE) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !cap_metadata_is_authoritative_file(&metadata) {
        return Err(EnrollmentError::UnsafeNamespace(
            "enrollment head is not a regular no-follow file".into(),
        ));
    }
    if metadata.len() != HEAD_BYTES as u64 {
        return Err(EnrollmentError::MalformedHead);
    }
    let file = open_regular_readonly(directory, HEAD_FILE)?;
    validate_authoritative_file(&file, "enrollment head")?;
    if file.metadata()?.len() != HEAD_BYTES as u64 {
        return Err(EnrollmentError::MalformedHead);
    }
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
    #[cfg(test)]
    ENROLLMENT_RECORD_READS.with(|reads| reads.set(reads.get().saturating_add(1)));
    let name = format!("{expected_digest}{RECORD_SUFFIX}");
    let metadata = match records.symlink_metadata(&name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(EnrollmentError::MissingChainRecord(expected_digest));
        }
        Err(error) => return Err(error.into()),
    };
    if !cap_metadata_is_authoritative_file(&metadata) {
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
    validate_authoritative_file(&file, "enrollment record")?;
    let opened_len = file.metadata()?.len();
    if opened_len > MAX_ENROLLMENT_RECORD_BYTES as u64 {
        return Err(EnrollmentError::RecordTooLarge(
            usize::try_from(opened_len).unwrap_or(usize::MAX),
        ));
    }
    let mut bytes = Vec::with_capacity(opened_len as usize);
    file.take((MAX_ENROLLMENT_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if ContentDigest::of(&bytes) != expected_digest {
        return Err(EnrollmentError::RecordDigestMismatch(expected_digest));
    }
    decode_record(&bytes)
}

fn publish_record(
    records: &Dir,
    lease: &EnrollmentLease,
    digest: ContentDigest,
    bytes: &[u8],
    cut: CommitCut,
) -> Result<(), EnrollmentError> {
    let target = format!("{digest}{RECORD_SUFFIX}");
    let temp_name = format!("{RECORD_TEMP_PREFIX}{digest}");
    recover_record_publication_temp(records, lease, &temp_name, &target, bytes)?;
    lease.validate_current()?;
    let mut temp = create_new_regular(records, &temp_name)?;
    validate_authoritative_file(&temp, "enrollment record temporary file")?;
    inject_crash_cut(
        cut,
        CommitCut::AfterRecordTempCreate,
        "after_record_temp_create",
    )?;
    lease.validate_current()?;
    temp.write_all(bytes)?;
    inject_crash_cut(cut, CommitCut::AfterRecordWrite, "after_record_write")?;
    lease.validate_current()?;
    temp.sync_all()?;
    inject_crash_cut(
        cut,
        CommitCut::AfterRecordFileSync,
        "after_record_file_sync",
    )?;
    lease.validate_current()?;
    #[cfg(test)]
    if cut == CommitCut::AfterRecordLink {
        records.hard_link(&temp_name, records, &target)?;
        inject_crash_cut(cut, CommitCut::AfterRecordLink, "after_record_link")?;
        records.remove_file(&temp_name)?;
    }
    #[cfg(not(test))]
    let _ = cut;
    #[cfg(test)]
    let inserted_at_test_link_seam = cut == CommitCut::AfterRecordLink;
    #[cfg(not(test))]
    let inserted_at_test_link_seam = false;
    match if inserted_at_test_link_seam {
        Ok(())
    } else {
        rename_noreplace(records, &temp_name, &target)
    } {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing = read_record(records, digest)?;
            if canonical_record_bytes(&existing)? != bytes {
                return Err(EnrollmentError::RecordDigestMismatch(digest));
            }
            lease.validate_current()?;
            let _ = records.remove_file(&temp_name);
        }
        Err(error) => return Err(error.into()),
    }
    inject_crash_cut(cut, CommitCut::AfterRecordInsert, "after_record_insert")?;
    lease.validate_current()?;
    sync_dir_required(records).map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    inject_crash_cut(
        cut,
        CommitCut::AfterRecordsDirectorySync,
        "after_records_directory_sync",
    )
}

fn recover_record_publication_temp(
    records: &Dir,
    lease: &EnrollmentLease,
    temp_name: &str,
    target: &str,
    expected_bytes: &[u8],
) -> Result<(), EnrollmentError> {
    let target_state = match records.symlink_metadata(target) {
        Ok(metadata) if cap_metadata_is_authoritative_file(&metadata) => {
            let (bytes, identity) = read_bounded_authoritative_file(
                records,
                target,
                MAX_ENROLLMENT_RECORD_BYTES,
                "enrollment record publication target",
                true,
            )?;
            if bytes != expected_bytes {
                return Err(EnrollmentError::AmbiguousRecordPublication);
            }
            let file = open_regular_readonly(records, target)?;
            Some((identity, authoritative_file_link_count(&file)?))
        }
        Ok(_) => {
            return Err(EnrollmentError::UnsafeNamespace(
                "enrollment record publication target is not a regular no-follow file".into(),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let temp_state = match records.symlink_metadata(temp_name) {
        Ok(metadata) if cap_metadata_is_authoritative_file(&metadata) => {
            let (bytes, identity) = read_bounded_authoritative_file(
                records,
                temp_name,
                MAX_ENROLLMENT_RECORD_BYTES,
                "enrollment record publication temporary file",
                true,
            )?;
            if !bytes.is_empty() && bytes != expected_bytes {
                return Err(EnrollmentError::AmbiguousRecordPublication);
            }
            let file = open_regular_readonly(records, temp_name)?;
            Some((bytes, identity, authoritative_file_link_count(&file)?))
        }
        Ok(_) => {
            return Err(EnrollmentError::UnsafeNamespace(
                "enrollment record publication temporary path is not a regular no-follow file"
                    .into(),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    match (&target_state, &temp_state) {
        (Some((target_identity, 2)), Some((bytes, temp_identity, 2)))
            if target_identity == temp_identity && bytes == expected_bytes => {}
        (Some((_, 1)), Some((_, _, 1))) | (None, Some((_, _, 1))) => {}
        (Some((_, 1)), None) | (None, None) => return Ok(()),
        _ => return Err(EnrollmentError::AmbiguousRecordPublication),
    }
    lease.validate_current()?;
    records.remove_file(temp_name)?;
    sync_dir_required(records).map_err(|error| EnrollmentError::Durability(error.to_string()))
}

fn resume_or_persist_initial_record(
    directories: &EnrollmentDirectories,
    lease: &EnrollmentLease,
    authority: &EnrollmentAuthority,
    record: &EnrollmentRecordV1,
    cut: CommitCut,
) -> Result<EnrollmentSnapshot, EnrollmentError> {
    lease.validate_current()?;
    authority.validate_current()?;
    let bytes = canonical_record_bytes(record)?;
    let digest = ContentDigest::of(&bytes);
    let target = format!("{digest}{RECORD_SUFFIX}");
    let expected_head = format!("{digest}\n").into_bytes();
    let head = read_head(&directories.enrollment)?;
    if head.is_some_and(|found| found != digest) {
        return Err(EnrollmentError::AlreadyExists);
    }

    let mut target_identity = None;
    let mut target_has_two_links = false;
    let mut record_temps = Vec::new();
    let mut count = 0usize;
    for entry in directories.records.entries()? {
        let entry = entry?;
        count += 1;
        if count > MAX_ENROLLMENT_NAMESPACE_ENTRIES {
            return Err(EnrollmentError::NamespaceBoundExceeded);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| EnrollmentError::UnsupportedArtifact("non-UTF-8 record name".into()))?;
        if name == target {
            let (found, identity) = read_bounded_authoritative_file(
                &directories.records,
                &name,
                MAX_ENROLLMENT_RECORD_BYTES,
                "initial enrollment record",
                true,
            )?;
            if found != bytes {
                return Err(EnrollmentError::AmbiguousInitialCreation);
            }
            target_has_two_links = authoritative_path_has_two_links(&directories.records, &name)?;
            target_identity = Some(identity);
            continue;
        }
        if name.starts_with(RECORD_TEMP_PREFIX) && regular_entry(&entry)? {
            let (found, identity) = read_bounded_authoritative_file(
                &directories.records,
                &name,
                MAX_ENROLLMENT_RECORD_BYTES,
                "initial enrollment record temporary file",
                true,
            )?;
            if !found.is_empty() && found != bytes {
                return Err(EnrollmentError::AmbiguousInitialCreation);
            }
            let has_two_links = authoritative_path_has_two_links(&directories.records, &name)?;
            record_temps.push((name, identity, has_two_links));
            continue;
        }
        return Err(EnrollmentError::AmbiguousInitialCreation);
    }

    for (_, identity, has_two_links) in &record_temps {
        if *has_two_links && target_identity.as_ref() != Some(identity) {
            return Err(EnrollmentError::AmbiguousInitialCreation);
        }
    }
    if target_has_two_links
        && !record_temps.iter().any(|(_, identity, has_two_links)| {
            *has_two_links && target_identity.as_ref() == Some(identity)
        })
    {
        return Err(EnrollmentError::AmbiguousInitialCreation);
    }

    let mut head_temps = Vec::new();
    for entry in directories.enrollment.entries()? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            EnrollmentError::UnsupportedArtifact("non-UTF-8 enrollment artifact".into())
        })?;
        if !name.starts_with(HEAD_TEMP_PREFIX) {
            continue;
        }
        let (found, _) = read_bounded_authoritative_file(
            &directories.enrollment,
            &name,
            HEAD_BYTES,
            "initial enrollment head temporary file",
            false,
        )?;
        if !found.is_empty() && found != expected_head {
            return Err(EnrollmentError::AmbiguousInitialCreation);
        }
        head_temps.push(name);
    }

    if head.is_some() && target_identity.is_none() {
        return Err(EnrollmentError::MissingChainRecord(digest));
    }

    let had_record_temps = !record_temps.is_empty();
    let had_head_temps = !head_temps.is_empty();
    for (name, _, _) in record_temps {
        lease.validate_current()?;
        authority.validate_current()?;
        directories.records.remove_file(&name)?;
    }
    for name in head_temps {
        lease.validate_current()?;
        authority.validate_current()?;
        directories.enrollment.remove_file(&name)?;
    }
    if had_record_temps {
        sync_dir_required(&directories.records)
            .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    }
    if had_head_temps {
        sync_dir_required(&directories.enrollment)
            .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    }

    if head.is_some() {
        let found = read_record(&directories.records, digest)?;
        if canonical_record_bytes(&found)? != bytes {
            return Err(EnrollmentError::AmbiguousInitialCreation);
        }
        validate_record_authority(
            &found,
            &record.binding,
            record.lease_resource_id,
            &authority.material,
        )?;
        return Ok(EnrollmentSnapshot {
            digest,
            record: found,
        });
    }
    persist_record_and_head(directories, lease, record, cut)
}

/// A current binary never mints a legacy checkpoint, but it must be able to
/// finish an old binary's initial publication after the v1 authority became
/// durable. Select an existing v5 generation-one candidate only when its
/// canonical bytes, deterministic name, binding, lease, initial intent, and
/// frozen HMAC all verify. The ordinary initial-publication recovery below
/// then applies the retained-capability and link-count rules to those exact
/// bytes. With no valid legacy candidate, the current v6 initial record remains
/// the only record this binary can publish.
fn select_initial_record_for_recovery(
    directories: &EnrollmentDirectories,
    authority: &EnrollmentAuthority,
    current_record: &EnrollmentRecordV1,
) -> Result<EnrollmentRecordV1, EnrollmentError> {
    if !matches!(
        &authority.material.claim,
        EnrollmentAuthorityClaim::LegacyV1(_)
    ) {
        return Ok(current_record.clone());
    }
    let EnrollmentLifecycleV1::ShadowImport(expected_shadow) = &current_record.lifecycle else {
        unreachable!("an initial record always carries ShadowImport")
    };

    let mut candidate: Option<(ContentDigest, EnrollmentRecordV1)> = None;
    let mut count = 0usize;
    for entry in directories.records.entries()? {
        let entry = entry?;
        count += 1;
        if count > MAX_ENROLLMENT_NAMESPACE_ENTRIES {
            return Err(EnrollmentError::NamespaceBoundExceeded);
        }
        if !regular_entry(&entry)? {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| EnrollmentError::UnsupportedArtifact("non-UTF-8 record name".into()))?;
        let (bytes, _) = read_bounded_authoritative_file(
            &directories.records,
            &name,
            MAX_ENROLLMENT_RECORD_BYTES,
            "legacy initial enrollment recovery candidate",
            true,
        )?;
        if bytes.is_empty() {
            continue;
        }
        let Ok(record) = decode_record(&bytes) else {
            continue;
        };
        if record.schema_version != ENROLLMENT_RECORD_SCHEMA_V5 {
            continue;
        }
        let digest = ContentDigest::of(&bytes);
        let target = format!("{digest}{RECORD_SUFFIX}");
        let temp = format!("{RECORD_TEMP_PREFIX}{digest}");
        if name != target && name != temp {
            return Err(EnrollmentError::AmbiguousInitialCreation);
        }
        validate_record_authority(
            &record,
            &current_record.binding,
            current_record.lease_resource_id,
            &authority.material,
        )?;
        validate_initial_record(&record)?;
        if !matches!(
            &record.lifecycle,
            EnrollmentLifecycleV1::ShadowImport(shadow) if shadow == expected_shadow
        ) {
            return Err(EnrollmentError::InitialPreparationMismatch);
        }
        if candidate
            .as_ref()
            .is_some_and(|(found_digest, _)| *found_digest != digest)
        {
            return Err(EnrollmentError::AmbiguousInitialCreation);
        }
        candidate = Some((digest, record));
    }
    Ok(candidate
        .map(|(_, record)| record)
        .unwrap_or_else(|| current_record.clone()))
}

fn reject_unsafe_head_target(directory: &Dir) -> Result<(), EnrollmentError> {
    match directory.symlink_metadata(HEAD_FILE) {
        Ok(metadata) if !cap_metadata_is_authoritative_file(&metadata) => {
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
    #[cfg(test)]
    count(&ENROLLMENT_NAMESPACE_SCANS);
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
            HEAD_FILE | LEASE_FILE | AUTHORITY_FILE => regular_entry(&entry)?,
            _ if name.starts_with(HEAD_TEMP_PREFIX) => regular_entry(&entry)?,
            _ if name.starts_with(AUTHORITY_TEMP_PREFIX) => {
                if !regular_entry(&entry)? {
                    return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
                }
                true
            }
            _ => false,
        };
        if !accepted {
            return Err(EnrollmentError::UnsupportedArtifact(name));
        }
    }

    // Immutable record history has no lifetime cardinality bound. Authoritative
    // records are classified and authenticated only when addressed by a head
    // or opaque audit cursor; unrelated artifacts are retained but inert.
    Ok(())
}

fn regular_entry(entry: &cap_std::fs::DirEntry) -> Result<bool, EnrollmentError> {
    Ok(cap_metadata_is_authoritative_file(&entry.metadata()?))
}

fn open_directories(
    root: &EnrollmentApplicationRoot,
    graph_resource: CanonicalGraphResourceId,
    create: bool,
) -> Result<Option<EnrollmentDirectories>, EnrollmentError> {
    #[cfg(test)]
    count(&ENROLLMENT_DIRECTORY_OPENS);
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
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || cap_metadata_is_windows_reparse(&metadata) =>
        {
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
    #[cfg(test)]
    count(&ENROLLMENT_LEASE_ACQUISITIONS);
    let file = if create {
        open_regular_readwrite_create(&directories.enrollment, LEASE_FILE)?
    } else {
        match directories.enrollment.symlink_metadata(LEASE_FILE) {
            Ok(metadata) if !cap_metadata_is_authoritative_file(&metadata) => {
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
    validate_authoritative_file(&file, "enrollment lease")?;
    if let Err(error) = file.try_lock_exclusive() {
        if error.kind() == ErrorKind::PermissionDenied
            || tine_storage::nonblocking_lock_is_contended(&error)
        {
            return Err(EnrollmentError::LeaseContended(
                directories.display_path.join(LEASE_FILE),
            ));
        }
        return Err(error.into());
    }
    let identity = authoritative_file_identity(&file)?;
    let resource_id = lease_resource_id(&identity);
    let lease = EnrollmentLease {
        file,
        directory: directories.enrollment.try_clone()?,
        identity,
        resource_id,
    };
    lease.validate_current()?;
    Ok(lease)
}

fn inspect_lease_resource_id(
    directories: &EnrollmentDirectories,
) -> Result<ContentDigest, EnrollmentError> {
    match directories.enrollment.symlink_metadata(LEASE_FILE) {
        Ok(metadata) if !cap_metadata_is_authoritative_file(&metadata) => {
            return Err(EnrollmentError::UnsafeNamespace(
                "enrollment lease is not a regular no-follow file".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(EnrollmentError::UnsafeNamespace(
                "existing enrollment has no lease authority".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let file = open_regular_readonly(&directories.enrollment, LEASE_FILE)?;
    validate_authoritative_file(&file, "enrollment lease")?;
    Ok(lease_resource_id(&authoritative_file_identity(&file)?))
}

fn provision_or_resume_enrollment_authority(
    directories: &EnrollmentDirectories,
    lease: &EnrollmentLease,
    binding: &EnrollmentBindingV1,
    shadow: &ShadowImportV1,
) -> Result<EnrollmentAuthority, EnrollmentError> {
    lease.validate_current()?;
    match directories.enrollment.symlink_metadata(AUTHORITY_FILE) {
        Ok(metadata) if !cap_metadata_is_authoritative_file(&metadata) => {
            return Err(EnrollmentError::UnsafeNamespace(
                "enrollment authority claim is not a regular no-follow file".into(),
            ));
        }
        Ok(_) => {
            let authority =
                open_enrollment_authority_for_recovery(directories, binding, lease.resource_id)?;
            authority.material.claim.validate_initial_intent(shadow)?;
            recover_authority_temps(directories, lease, binding, shadow, Some(&authority))?;
            drop(authority);
            return open_enrollment_authority(directories, binding, lease.resource_id);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    if let Some(temp_name) = recover_authority_temps(directories, lease, binding, shadow, None)? {
        lease.validate_current()?;
        rename_noreplace(&directories.enrollment, &temp_name, AUTHORITY_FILE)?;
        sync_dir_required(&directories.enrollment)
            .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
        return open_enrollment_authority(directories, binding, lease.resource_id);
    }

    let claim = EnrollmentAuthorityClaim::CurrentV2(EnrollmentAuthorityClaimV2 {
        schema_version: ENROLLMENT_AUTHORITY_SCHEMA_VERSION,
        authority_id: Uuid::new_v4(),
        lease_resource_id: lease.resource_id,
        binding: binding.clone(),
        initial_preparation_id: shadow.preparation_id,
        initial_source_inventory_digest: shadow.source_inventory_digest,
    });
    let bytes = canonical_authority_claim_bytes(&claim)?;
    let temp_name = format!("{AUTHORITY_TEMP_PREFIX}{}", Uuid::new_v4());
    lease.validate_current()?;
    let mut temp = create_new_regular(&directories.enrollment, &temp_name)?;
    validate_authoritative_file(&temp, "enrollment authority temporary file")?;
    temp.write_all(&bytes)?;
    temp.sync_all()?;
    lease.validate_current()?;
    rename_noreplace(&directories.enrollment, &temp_name, AUTHORITY_FILE)?;
    sync_dir_required(&directories.enrollment)
        .map_err(|error| EnrollmentError::Durability(error.to_string()))?;
    open_enrollment_authority(directories, binding, lease.resource_id)
}

fn recover_authority_temps(
    directories: &EnrollmentDirectories,
    lease: &EnrollmentLease,
    binding: &EnrollmentBindingV1,
    shadow: &ShadowImportV1,
    installed: Option<&EnrollmentAuthority>,
) -> Result<Option<String>, EnrollmentError> {
    if let Some(authority) = installed {
        recover_installed_authority_temp(directories, lease, authority)?;
        return Ok(None);
    }

    let mut resumable = None;
    for entry in directories.enrollment.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| EnrollmentError::AmbiguousAuthorityProvisioning)?;
        if !name.starts_with(AUTHORITY_TEMP_PREFIX) {
            continue;
        }
        if !regular_entry(&entry)? {
            return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
        }
        let (bytes, identity) = read_bounded_authoritative_file(
            &directories.enrollment,
            &name,
            MAX_ENROLLMENT_AUTHORITY_BYTES,
            "enrollment authority temporary file",
            true,
        )?;
        let claim = decode_authority_claim(&bytes)?;
        claim.validate_initial_intent(shadow)?;
        let resource_id = authority_resource_id(&identity);
        EnrollmentAuthorityMaterial::from_claim(claim, resource_id, binding, lease.resource_id)?;
        if resumable.replace(name).is_some() {
            return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
        }
    }
    Ok(resumable)
}

fn recover_installed_authority_temp(
    directories: &EnrollmentDirectories,
    lease: &EnrollmentLease,
    installed: &EnrollmentAuthority,
) -> Result<(), EnrollmentError> {
    let expected_bytes = canonical_authority_claim_bytes(&installed.material.claim)?;
    let mut temps = Vec::new();
    for entry in directories.enrollment.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| EnrollmentError::AmbiguousAuthorityProvisioning)?;
        if !name.starts_with(AUTHORITY_TEMP_PREFIX) {
            continue;
        }
        if !regular_entry(&entry)? {
            return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
        }
        let state = read_bounded_authoritative_file_for_recovery(
            &directories.enrollment,
            &name,
            MAX_ENROLLMENT_AUTHORITY_BYTES,
            "enrollment authority temporary file",
        )
        .map_err(|_| EnrollmentError::AmbiguousAuthorityProvisioning)?;
        temps.push((name, state));
    }

    if temps.is_empty() {
        if authoritative_file_link_count(&installed.file)? != 1 {
            return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
        }
        return Ok(());
    }
    if !authority_publication_uses_link_unlink() || temps.len() != 1 {
        return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
    }

    let (temp_name, (temp_bytes, temp_identity, temp_links)) = temps.pop().expect("one temp");
    if temp_bytes.is_empty()
        || temp_bytes != expected_bytes
        || temp_identity != installed.identity
        || temp_links != 2
    {
        return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
    }

    lease.validate_current()?;
    let (target_bytes, target_identity, target_links) =
        read_bounded_authoritative_file_for_recovery(
            &directories.enrollment,
            AUTHORITY_FILE,
            MAX_ENROLLMENT_AUTHORITY_BYTES,
            "enrollment authority claim",
        )
        .map_err(|_| EnrollmentError::AmbiguousAuthorityProvisioning)?;
    let (temp_bytes, temp_identity, temp_links) = read_bounded_authoritative_file_for_recovery(
        &directories.enrollment,
        &temp_name,
        MAX_ENROLLMENT_AUTHORITY_BYTES,
        "enrollment authority temporary file",
    )
    .map_err(|_| EnrollmentError::AmbiguousAuthorityProvisioning)?;
    if target_bytes != expected_bytes
        || target_identity != installed.identity
        || target_links != 2
        || temp_bytes.is_empty()
        || temp_bytes != expected_bytes
        || temp_identity != installed.identity
        || temp_links != 2
    {
        return Err(EnrollmentError::AmbiguousAuthorityProvisioning);
    }

    directories.enrollment.remove_file(&temp_name)?;
    sync_dir_required(&directories.enrollment)
        .map_err(|error| EnrollmentError::Durability(error.to_string()))
}

const fn authority_publication_uses_link_unlink() -> bool {
    cfg!(windows)
}

fn open_enrollment_authority(
    directories: &EnrollmentDirectories,
    binding: &EnrollmentBindingV1,
    lease_resource_id: ContentDigest,
) -> Result<EnrollmentAuthority, EnrollmentError> {
    let authority = open_enrollment_authority_internal(directories, binding, lease_resource_id)?;
    authority.validate_current()?;
    Ok(authority)
}

fn open_discovered_enrollment_authority(
    directories: &EnrollmentDirectories,
    expected_graph_resource: CanonicalGraphResourceId,
    lease_resource_id: ContentDigest,
) -> Result<EnrollmentAuthority, EnrollmentError> {
    #[cfg(test)]
    count(&ENROLLMENT_AUTHORITY_CLAIM_READS);
    let (bytes, identity) = read_bounded_authoritative_file(
        &directories.enrollment,
        AUTHORITY_FILE,
        MAX_ENROLLMENT_AUTHORITY_BYTES,
        "enrollment authority claim",
        false,
    )?;
    let file = open_regular_readonly(&directories.enrollment, AUTHORITY_FILE)?;
    validate_authoritative_file(&file, "enrollment authority claim")?;
    if authoritative_file_identity(&file)? != identity {
        return Err(EnrollmentError::AuthorityMismatch);
    }
    let claim = decode_authority_claim(&bytes)?;
    if claim.binding().graph_resource_id != expected_graph_resource {
        return Err(EnrollmentError::BindingMismatch(
            EnrollmentBindingField::GraphResource,
        ));
    }
    let binding = claim.binding().clone();
    let material = EnrollmentAuthorityMaterial::from_claim(
        claim,
        authority_resource_id(&identity),
        &binding,
        lease_resource_id,
    )?;
    let authority = EnrollmentAuthority {
        material,
        file,
        directory: directories.enrollment.try_clone()?,
        identity,
    };
    authority.validate_current()?;
    Ok(authority)
}

fn open_enrollment_authority_for_recovery(
    directories: &EnrollmentDirectories,
    binding: &EnrollmentBindingV1,
    lease_resource_id: ContentDigest,
) -> Result<EnrollmentAuthority, EnrollmentError> {
    let (bytes, identity, _) = read_bounded_authoritative_file_for_recovery(
        &directories.enrollment,
        AUTHORITY_FILE,
        MAX_ENROLLMENT_AUTHORITY_BYTES,
        "enrollment authority claim",
    )?;
    let file = open_regular_readonly(&directories.enrollment, AUTHORITY_FILE)?;
    validate_authoritative_file_without_link_count(&file, "enrollment authority claim")?;
    if authoritative_file_identity(&file)? != identity {
        return Err(EnrollmentError::AuthorityMismatch);
    }
    let material = EnrollmentAuthorityMaterial::from_claim(
        decode_authority_claim(&bytes)?,
        authority_resource_id(&identity),
        binding,
        lease_resource_id,
    )?;
    Ok(EnrollmentAuthority {
        material,
        file,
        directory: directories.enrollment.try_clone()?,
        identity,
    })
}

fn open_enrollment_authority_internal(
    directories: &EnrollmentDirectories,
    binding: &EnrollmentBindingV1,
    lease_resource_id: ContentDigest,
) -> Result<EnrollmentAuthority, EnrollmentError> {
    #[cfg(test)]
    count(&ENROLLMENT_AUTHORITY_CLAIM_READS);
    let (bytes, identity) = read_bounded_authoritative_file(
        &directories.enrollment,
        AUTHORITY_FILE,
        MAX_ENROLLMENT_AUTHORITY_BYTES,
        "enrollment authority claim",
        false,
    )?;
    let file = open_regular_readonly(&directories.enrollment, AUTHORITY_FILE)?;
    validate_authoritative_file(&file, "enrollment authority claim")?;
    if authoritative_file_identity(&file)? != identity {
        return Err(EnrollmentError::AuthorityMismatch);
    }
    let material = EnrollmentAuthorityMaterial::from_claim(
        decode_authority_claim(&bytes)?,
        authority_resource_id(&identity),
        binding,
        lease_resource_id,
    )?;
    Ok(EnrollmentAuthority {
        material,
        file,
        directory: directories.enrollment.try_clone()?,
        identity,
    })
}

fn canonical_authority_claim_bytes(
    claim: &EnrollmentAuthorityClaim,
) -> Result<Vec<u8>, EnrollmentError> {
    let bytes = match claim {
        EnrollmentAuthorityClaim::LegacyV1(claim) => serde_json::to_vec(claim),
        EnrollmentAuthorityClaim::CurrentV2(claim) => serde_json::to_vec(claim),
    }
    .map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    if bytes.len() > MAX_ENROLLMENT_AUTHORITY_BYTES {
        return Err(EnrollmentError::AuthorityClaimTooLarge(bytes.len()));
    }
    Ok(bytes)
}

fn decode_authority_claim(bytes: &[u8]) -> Result<EnrollmentAuthorityClaim, EnrollmentError> {
    if bytes.len() > MAX_ENROLLMENT_AUTHORITY_BYTES {
        return Err(EnrollmentError::AuthorityClaimTooLarge(bytes.len()));
    }
    reject_duplicate_json_fields(bytes)?;
    validate_json_bounds(bytes)?;
    let probe: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
    let schema = probe
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| EnrollmentError::Decode("authority schema_version is missing".into()))?;
    let schema = u32::try_from(schema).unwrap_or(u32::MAX);
    let claim = match schema {
        ENROLLMENT_AUTHORITY_SCHEMA_V1 => EnrollmentAuthorityClaim::LegacyV1(
            serde_json::from_slice(bytes)
                .map_err(|error| EnrollmentError::Decode(error.to_string()))?,
        ),
        ENROLLMENT_AUTHORITY_SCHEMA_VERSION => EnrollmentAuthorityClaim::CurrentV2(
            serde_json::from_slice(bytes)
                .map_err(|error| EnrollmentError::Decode(error.to_string()))?,
        ),
        schema => return Err(EnrollmentError::UnsupportedAuthoritySchema(schema)),
    };
    if canonical_authority_claim_bytes(&claim)? != bytes {
        return Err(EnrollmentError::NonCanonicalAuthorityClaim);
    }
    Ok(claim)
}

fn read_bounded_authoritative_file(
    directory: &Dir,
    name: &str,
    maximum: usize,
    description: &str,
    allow_link_gap: bool,
) -> Result<(Vec<u8>, AuthoritativeFileIdentity), EnrollmentError> {
    let metadata = directory.symlink_metadata(name)?;
    if !cap_metadata_is_authoritative_file(&metadata) || metadata.len() > maximum as u64 {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "{description} is not a bounded regular no-follow file"
        )));
    }
    let file = open_regular_readonly(directory, name)?;
    validate_authoritative_file_with_link_gap(&file, description, allow_link_gap)?;
    let identity = authoritative_file_identity(&file)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((maximum + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "{description} exceeds its byte bound"
        )));
    }
    Ok((bytes, identity))
}

fn read_bounded_authoritative_file_for_recovery(
    directory: &Dir,
    name: &str,
    maximum: usize,
    description: &str,
) -> Result<(Vec<u8>, AuthoritativeFileIdentity, u64), EnrollmentError> {
    let metadata = directory.symlink_metadata(name)?;
    if !cap_metadata_is_authoritative_file(&metadata) || metadata.len() > maximum as u64 {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "{description} is not a bounded regular no-follow file"
        )));
    }
    let file = open_regular_readonly(directory, name)?;
    validate_authoritative_file_without_link_count(&file, description)?;
    let identity = authoritative_file_identity(&file)?;
    let link_count = authoritative_file_link_count(&file)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((maximum + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "{description} exceeds its byte bound"
        )));
    }
    Ok((bytes, identity, link_count))
}

fn validate_authoritative_file(file: &File, name: &str) -> Result<(), EnrollmentError> {
    validate_authoritative_file_with_link_gap(file, name, false)
}

fn validate_authoritative_file_with_link_gap(
    file: &File,
    name: &str,
    allow_link_gap: bool,
) -> Result<(), EnrollmentError> {
    validate_authoritative_file_without_link_count(file, name)?;
    let link_count = authoritative_file_link_count(file)?;
    if link_count != 1 && !(allow_link_gap && link_count == 2) {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "opened {name} has unsafe ownership or links"
        )));
    }
    Ok(())
}

fn validate_authoritative_file_without_link_count(
    file: &File,
    name: &str,
) -> Result<(), EnrollmentError> {
    let metadata = file.metadata()?;
    if !authoritative_file_kind_allowed(
        metadata.is_file(),
        false,
        std_metadata_is_windows_reparse(&metadata),
    ) {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "opened {name} is not a regular file"
        )));
    }
    #[cfg(unix)]
    if metadata.uid() !=
        // SAFETY: geteuid has no arguments or memory-safety preconditions.
        unsafe { libc::geteuid() }
    {
        return Err(EnrollmentError::UnsafeNamespace(format!(
            "opened {name} has unsafe ownership or links"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn authoritative_file_link_count(file: &File) -> Result<u64, EnrollmentError> {
    Ok(file.metadata()?.nlink())
}

#[cfg(windows)]
fn authoritative_file_link_count(file: &File) -> Result<u64, EnrollmentError> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` retains the exact live handle and `information` is a
    // correctly sized writable result value.
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(u64::from(information.nNumberOfLinks))
}

#[cfg(not(any(unix, windows)))]
fn authoritative_file_link_count(_file: &File) -> Result<u64, EnrollmentError> {
    Err(unsupported_filesystem().into())
}

fn authoritative_path_has_two_links(directory: &Dir, name: &str) -> Result<bool, EnrollmentError> {
    let file = open_regular_readonly(directory, name)?;
    Ok(authoritative_file_link_count(&file)? == 2)
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthoritativeFileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthoritativeFileIdentity {
    volume: u64,
    file_id: [u8; 16],
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthoritativeFileIdentity;

#[cfg(unix)]
fn authoritative_file_identity(file: &File) -> Result<AuthoritativeFileIdentity, EnrollmentError> {
    let metadata = file.metadata()?;
    Ok(AuthoritativeFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn authoritative_file_identity(file: &File) -> Result<AuthoritativeFileIdentity, EnrollmentError> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut information = FILE_ID_INFO::default();
    // SAFETY: `file` retains the exact live handle and `information` is a
    // correctly sized writable FILE_ID_INFO value.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(AuthoritativeFileIdentity {
        volume: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(not(any(unix, windows)))]
fn authoritative_file_identity(_file: &File) -> Result<AuthoritativeFileIdentity, EnrollmentError> {
    Err(unsupported_filesystem().into())
}

#[cfg(unix)]
fn lease_resource_id(identity: &AuthoritativeFileIdentity) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/enrollment-lease-resource/v1\0unix-dev-inode\0");
    hasher.update(identity.device.to_be_bytes());
    hasher.update(identity.inode.to_be_bytes());
    ContentDigest::from_bytes(hasher.finalize().into())
}

#[cfg(windows)]
fn lease_resource_id(identity: &AuthoritativeFileIdentity) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/enrollment-lease-resource/v1\0windows-volume-file-id\0");
    hasher.update(identity.volume.to_be_bytes());
    hasher.update(identity.file_id);
    ContentDigest::from_bytes(hasher.finalize().into())
}

#[cfg(not(any(unix, windows)))]
fn lease_resource_id(_identity: &AuthoritativeFileIdentity) -> ContentDigest {
    ContentDigest::from_bytes([0; 32])
}

#[cfg(unix)]
fn authority_resource_id(identity: &AuthoritativeFileIdentity) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/enrollment-authority-resource/v1\0unix-dev-inode\0");
    hasher.update(identity.device.to_be_bytes());
    hasher.update(identity.inode.to_be_bytes());
    ContentDigest::from_bytes(hasher.finalize().into())
}

#[cfg(windows)]
fn authority_resource_id(identity: &AuthoritativeFileIdentity) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/enrollment-authority-resource/v1\0windows-volume-file-id\0");
    hasher.update(identity.volume.to_be_bytes());
    hasher.update(identity.file_id);
    ContentDigest::from_bytes(hasher.finalize().into())
}

#[cfg(not(any(unix, windows)))]
fn authority_resource_id(_identity: &AuthoritativeFileIdentity) -> ContentDigest {
    ContentDigest::from_bytes([0; 32])
}

#[cfg(windows)]
fn std_metadata_is_windows_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn std_metadata_is_windows_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn cap_metadata_is_windows_reparse(metadata: &cap_std::fs::Metadata) -> bool {
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn cap_metadata_is_windows_reparse(_metadata: &cap_std::fs::Metadata) -> bool {
    false
}

fn authoritative_file_kind_allowed(is_file: bool, is_symlink: bool, is_reparse: bool) -> bool {
    is_file && !is_symlink && !is_reparse
}

fn cap_metadata_is_authoritative_file(metadata: &cap_std::fs::Metadata) -> bool {
    authoritative_file_kind_allowed(
        metadata.is_file(),
        metadata.file_type().is_symlink(),
        cap_metadata_is_windows_reparse(metadata),
    )
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
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
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
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .follow(FollowSymlinks::No);
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

// Must exist wherever ANY caller does. Two different exclusion lists reach it:
// the `not(any(unix, windows))` helpers above, and `rename_noreplace`'s fallback
// below, which is `not(any(linux, macos, android, windows))` and therefore
// compiles on iOS/tvOS/BSD — all of which ARE `unix`. Gating this on
// `not(any(unix, windows))` made it vanish exactly where that fallback needed it,
// breaking the iOS build from 6162b381 (2026-07-26) until 2026-08-08. The list
// below is the union of both caller sets.
#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
)))]
fn unsupported_filesystem() -> std::io::Error {
    std::io::Error::new(
        ErrorKind::Unsupported,
        "durable no-follow enrollment files are unsupported on this target",
    )
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_noreplace(directory: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    let from = CString::new(from)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid temporary name"))?;
    let to = CString::new(to)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid record name"))?;
    // SAFETY: both names are live relative C strings beneath one retained dir.
    // Android's renameat2 wrapper is API-30-only, while syscall is available
    // across the supported Android range.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            directory.as_fd().as_raw_fd(),
            from.as_ptr(),
            directory.as_fd().as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE as libc::c_uint,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(directory: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    let from = CString::new(from)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid temporary name"))?;
    let to = CString::new(to)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid record name"))?;
    // SAFETY: both names are live relative C strings beneath one retained dir.
    let result = unsafe {
        libc::renameatx_np(
            directory.as_fd().as_raw_fd(),
            from.as_ptr(),
            directory.as_fd().as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_noreplace(directory: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    // cap-std does not expose a retained-directory, atomic no-replace rename on
    // Windows. Publication therefore uses the two-link fallback; the
    // deterministic temporary name lets retry validate both exact handles,
    // bytes, link count, and resource identity before unlinking only the temp.
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
    AmbiguousInitialCreation,
    AmbiguousRecordPublication,
    AmbiguousAuthorityProvisioning,
    InitialPreparationMismatch,
    LeaseContended(PathBuf),
    LeaseResourceMismatch,
    AuthorityMismatch,
    AuthorityClaimTooLarge(usize),
    NonCanonicalAuthorityClaim,
    UnsupportedAuthoritySchema(u32),
    LocalActivationReservationMismatch,
    LocalActivationReservationTooLarge(usize),
    NonCanonicalLocalActivationReservation,
    UnsupportedLocalActivationReservationSchema(u32),
    UnsupportedCheckpointSchema(u32),
    MissingAuthenticatedCheckpoint,
    /// An old v1 authority cannot verify the frozen v5 checkpoint HMAC.
    CheckpointLegacyAuthenticationFailed,
    /// A v6 CRC checkpoint does not bind its canonical record fields.
    CheckpointIntegrityFailed,
    /// Record and checkpoint codecs are version-paired and never interchangeable.
    IllegalCheckpointPair,
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
    InvalidVerifiedLocalTerminal,
    InvalidLocalActiveAnchor,
    InvalidSharedProjectionBaseEvidence,
    UnsupportedSharedEnrollmentDescriptorSchema(u32),
    UnsupportedJoinerWorkspaceArchiveSchema(u32),
    UnsafeSharedEnrollmentHandoff,
    SharedEnrollmentBindingMismatch,
    SharedProjectionBaseMismatch(&'static str),
    SharedEnrollmentDescriptorDigestMismatch,
    DirtyUniqueLocalTail,
    InvalidBlockedReason,
    IllegalLifecycle(&'static str),
    IllegalTransition,
    StaleCompareAndSwap,
    GenerationOverflow,
    NonmonotonicGeneration,
    HistoryAccumulatorMismatch,
    ChainCycle,
    InvalidPageLimit(usize),
    InvalidAuditCursor,
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
            Self::AmbiguousInitialCreation => {
                formatter.write_str("ambiguous stranded initial enrollment state")
            }
            Self::AmbiguousRecordPublication => {
                formatter.write_str("ambiguous enrollment record publication state")
            }
            Self::AmbiguousAuthorityProvisioning => {
                formatter.write_str("ambiguous enrollment authority provisioning state")
            }
            Self::InitialPreparationMismatch => {
                formatter.write_str("enrollment authority initial preparation mismatch")
            }
            Self::LeaseContended(path) => {
                write!(
                    formatter,
                    "enrollment lease is already held: {}",
                    path.display()
                )
            }
            Self::LeaseResourceMismatch => {
                formatter.write_str("enrollment lease resource was replaced or unlinked")
            }
            Self::AuthorityMismatch => {
                formatter.write_str("enrollment authority claim was replaced or substituted")
            }
            Self::AuthorityClaimTooLarge(bytes) => {
                write!(
                    formatter,
                    "enrollment authority claim is too large: {bytes} bytes"
                )
            }
            Self::NonCanonicalAuthorityClaim => {
                formatter.write_str("enrollment authority claim is not canonical")
            }
            Self::UnsupportedAuthoritySchema(schema) => {
                write!(
                    formatter,
                    "unsupported enrollment authority schema {schema}"
                )
            }
            Self::LocalActivationReservationMismatch => {
                formatter.write_str("local activation reservation binding mismatch")
            }
            Self::LocalActivationReservationTooLarge(bytes) => {
                write!(
                    formatter,
                    "local activation reservation is too large: {bytes} bytes"
                )
            }
            Self::NonCanonicalLocalActivationReservation => {
                formatter.write_str("local activation reservation is not canonical")
            }
            Self::UnsupportedLocalActivationReservationSchema(schema) => {
                write!(
                    formatter,
                    "unsupported local activation reservation schema {schema}"
                )
            }
            Self::UnsupportedCheckpointSchema(schema) => {
                write!(
                    formatter,
                    "unsupported enrollment checkpoint schema {schema}"
                )
            }
            Self::MissingAuthenticatedCheckpoint => {
                formatter.write_str("enrollment history suffix has no authenticated checkpoint")
            }
            Self::CheckpointLegacyAuthenticationFailed => {
                formatter.write_str("legacy enrollment checkpoint authentication failed")
            }
            Self::CheckpointIntegrityFailed => {
                formatter.write_str("enrollment checkpoint integrity check failed")
            }
            Self::IllegalCheckpointPair => {
                formatter.write_str("illegal enrollment record/checkpoint schema pair")
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
            Self::InvalidVerifiedLocalTerminal => formatter.write_str(
                "verified-local terminal bootstrap identity or proof counts are inconsistent",
            ),
            Self::InvalidLocalActiveAnchor => formatter.write_str(
                "local-active bootstrap anchor identity or proof counts are inconsistent",
            ),
            Self::InvalidSharedProjectionBaseEvidence => {
                formatter.write_str("shared enrollment projection/base evidence is inconsistent")
            }
            Self::UnsupportedSharedEnrollmentDescriptorSchema(schema) => {
                write!(
                    formatter,
                    "unsupported shared enrollment descriptor schema {schema}"
                )
            }
            Self::UnsupportedJoinerWorkspaceArchiveSchema(schema) => {
                write!(
                    formatter,
                    "unsupported joiner workspace archive schema {schema}"
                )
            }
            Self::UnsafeSharedEnrollmentHandoff => {
                formatter.write_str("shared enrollment requires a safe handoff")
            }
            Self::SharedEnrollmentBindingMismatch => formatter
                .write_str("shared enrollment binding or projection/base evidence mismatch"),
            Self::SharedProjectionBaseMismatch(field) => {
                write!(
                    formatter,
                    "shared enrollment projection/base mismatch: {field}"
                )
            }
            Self::SharedEnrollmentDescriptorDigestMismatch => {
                formatter.write_str("shared enrollment descriptor digest mismatch")
            }
            Self::DirtyUniqueLocalTail => {
                formatter.write_str("joiner has unique unprojected local operations")
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
            Self::HistoryAccumulatorMismatch => {
                formatter.write_str("enrollment history accumulator mismatch")
            }
            Self::ChainCycle => formatter.write_str("enrollment chain contains a cycle"),
            Self::InvalidPageLimit(limit) => {
                write!(formatter, "invalid enrollment audit page limit {limit}")
            }
            Self::InvalidAuditCursor => formatter.write_str("invalid enrollment audit cursor"),
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

    fn test_lease_resource() -> ContentDigest {
        digest(24)
    }

    fn test_authority(
        binding: EnrollmentBindingV1,
        lease_resource_id: ContentDigest,
    ) -> EnrollmentAuthorityMaterial {
        EnrollmentAuthorityMaterial::from_claim(
            EnrollmentAuthorityClaim::LegacyV1(EnrollmentAuthorityClaimV1 {
                schema_version: ENROLLMENT_AUTHORITY_SCHEMA_V1,
                authority_id: Uuid::from_u128(25),
                lease_resource_id,
                binding: binding.clone(),
                initial_preparation_id: shadow().preparation_id,
                initial_source_inventory_digest: shadow().source_inventory_digest,
                key: [26; legacy_checkpoint::LEGACY_AUTHORITY_KEY_BYTES],
            }),
            digest(27),
            &binding,
            lease_resource_id,
        )
        .unwrap()
    }

    /// Build one byte-exact old-format enrollment without exercising an old
    /// binary.  The test-only legacy signer makes the frozen v1/v5 bytes a
    /// compatibility fixture; production can only verify them.
    fn install_legacy_v1_v5_enrollment(
        root: &TestRoot,
        binding: EnrollmentBindingV1,
    ) -> (Vec<u8>, AuthoritativeFileIdentity) {
        create_legacy_initial_enrollment_for_test(&root.path, binding.clone()).unwrap();
        let enrollment = enrollment_directory(root, &binding);
        let authority_path = enrollment.join(AUTHORITY_FILE);
        let authority_bytes = fs::read(&authority_path).unwrap();
        let file = open_regular_readonly(
            &Dir::open_ambient_dir(&enrollment, ambient_authority()).unwrap(),
            AUTHORITY_FILE,
        )
        .unwrap();
        let identity = authoritative_file_identity(&file).unwrap();
        (authority_bytes, identity)
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

    fn shared_test_binding(device: u128, graph: u8, archive: u8) -> EnrollmentBindingV1 {
        let graph_resource = graph_resource(graph);
        EnrollmentBindingV1::new(
            WorkspaceId::from_uuid(Uuid::from_u128(1)),
            LineageDigest::from_bytes([2; 32]),
            DocumentId::from_uuid(Uuid::from_u128(3)),
            ProjectionEndpointId::from_uuid(Uuid::from_u128(0x7000 + device)),
            DeviceId::from_uuid(Uuid::from_u128(device)),
            graph_resource,
            receipt_store(0x40 + graph),
            archive_resource(archive),
            GraphTextScope::new(&[], false).bind_graph_resource(graph_resource),
        )
        .unwrap()
    }

    fn local_active_safe_for_shared_test(root: &TestRoot, binding: EnrollmentBindingV1) {
        create_discovery_enrollment_for_test(
            &root.path,
            binding,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
        )
        .unwrap();
    }

    fn assert_shared_blocked(root: &TestRoot, binding: &EnrollmentBindingV1) {
        let inspection =
            inspect_existing_enrollment_at(&root.path, binding.graph_resource_id()).unwrap();
        assert!(matches!(
            inspection,
            EnrollmentDiscoveryInspection::Present(EnrollmentDiscoveryEvidence {
                lifecycle: EnrollmentDiscoveryLifecycle::Blocked { .. },
                ..
            })
        ));
    }

    fn assert_shared_runtime_discoverable(root: &TestRoot, binding: &EnrollmentBindingV1) {
        let inspection =
            inspect_existing_enrollment_at(&root.path, binding.graph_resource_id()).unwrap();
        assert!(matches!(
            inspection,
            EnrollmentDiscoveryInspection::Present(EnrollmentDiscoveryEvidence {
                lifecycle: EnrollmentDiscoveryLifecycle::LocalActive(_),
                ..
            })
        ));
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
            source_file_count: 1,
            source_chunk_count: 1,
            source_total_bytes: 1,
            backup_manifest: BlobDescription::of(b"backup-manifest"),
            backup_restore_proof: BlobDescription::of(b"backup-restore-proof"),
            backup_evidence_digest: digest(10),
            bootstrap_import_id: digest(11),
            bootstrap_part_count: 1,
            bootstrap_terminal_part_id: Some(digest(12)),
            bootstrap_batch_id: Some(BatchId::from_uuid(Uuid::from_u128(12))),
            accepted_frontier_anchor: anchor(1),
            accepted_history_record_count: 1,
            catalog_row_count: 1,
            sqlite_accepted_batch_count: 1,
            sqlite_semantic_projection_digest: digest(13),
            sqlite_materialized_row_digest: digest(14),
            staged_projection_manifest: BlobDescription::of(b"shadow-manifest"),
            staged_projection_proof: BlobDescription::of(b"shadow-proof"),
            staged_file_count: 1,
            staged_total_bytes: 1,
            byte_compare_digest: digest(15),
            shadow_evidence_digest: digest(16),
            proof_binding_digest: digest(17),
        }
    }

    fn zero_verified() -> VerifiedLocalV1 {
        let mut value = verified();
        value.source_file_count = 0;
        value.source_chunk_count = 0;
        value.source_total_bytes = 0;
        value.bootstrap_part_count = 0;
        value.bootstrap_terminal_part_id = None;
        value.bootstrap_batch_id = None;
        value.accepted_frontier_anchor.acceptance_sequence = 0;
        value.accepted_frontier_anchor.history_generation = 0;
        value.accepted_frontier_anchor.history_root =
            super::super::object_store::EngineHistoryStore::empty_root();
        value.accepted_history_record_count = 0;
        value.catalog_row_count = 0;
        value.sqlite_accepted_batch_count = 0;
        value.staged_file_count = 0;
        value.staged_total_bytes = 0;
        value
    }

    fn multipart_verified() -> VerifiedLocalV1 {
        let mut value = verified();
        value.source_file_count = 7;
        value.source_chunk_count = 19;
        value.source_total_bytes = 4_096;
        value.bootstrap_part_count = 7;
        value.accepted_frontier_anchor.acceptance_sequence = 7;
        value.accepted_frontier_anchor.history_generation = 7;
        value.accepted_history_record_count = 7;
        value.catalog_row_count = 7;
        value.sqlite_accepted_batch_count = 7;
        value.staged_file_count = 7;
        value.staged_total_bytes = 4_096;
        value
    }

    /// The exact anchor a `VerifiedLocal -> LocalActive` transition out of the
    /// record at `predecessor` must mint.
    fn local_anchor(predecessor: ContentDigest) -> LocalActiveAnchorV1 {
        LocalActiveAnchorV1::from_verified_local(&verified(), predecessor)
    }

    fn active(
        predecessor: ContentDigest,
        handoff: HandoffV1,
        exclusion: LocalExclusionV1,
    ) -> EnrollmentLifecycleV1 {
        EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
            verification_digest: verified().verification_digest().unwrap(),
            anchor: local_anchor(predecessor),
            handoff,
            exclusion,
        })
    }

    fn unsafe_idle(predecessor: ContentDigest, session: u128) -> EnrollmentLifecycleV1 {
        active(
            predecessor,
            HandoffV1::Unsafe {
                session_id: SessionId::from_uuid(Uuid::from_u128(session)),
            },
            LocalExclusionV1::Idle,
        )
    }

    fn safe_idle(predecessor: ContentDigest) -> EnrollmentLifecycleV1 {
        active(predecessor, HandoffV1::Safe, LocalExclusionV1::Idle)
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

    fn published(
        predecessor: ContentDigest,
        session: u128,
        archive: CanonicalArchiveResourceId,
    ) -> EnrollmentLifecycleV1 {
        active(
            predecessor,
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
    fn current_enrollment_claim_record_and_checkpoint_are_keyless_canonical_crc_v2_v6_v3() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let root = TestRoot::new("current-crc-goldens");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let authority_bytes =
            fs::read(enrollment_directory(&root, &binding).join(AUTHORITY_FILE)).unwrap();
        let authority_value: serde_json::Value = serde_json::from_slice(&authority_bytes).unwrap();
        assert_eq!(authority_value["schema_version"], 2);
        assert!(authority_value.get("key").is_none());
        assert_eq!(
            canonical_authority_claim_bytes(&decode_authority_claim(&authority_bytes).unwrap())
                .unwrap(),
            authority_bytes
        );

        let record_bytes = fs::read(record_path(&root, &binding, writer.current().digest)).unwrap();
        let record_value: serde_json::Value = serde_json::from_slice(&record_bytes).unwrap();
        assert_eq!(record_value["schema_version"], 6);
        assert_eq!(record_value["checkpoint"]["schema_version"], 3);
        assert!(record_value["checkpoint"].get("integrity_tag").is_some());
        assert!(record_value["checkpoint"]
            .get("authentication_tag")
            .is_none());
        assert_eq!(
            decode_record(&record_bytes).unwrap(),
            writer.current().record
        );
    }

    #[test]
    fn legacy_v1_v5_reopens_byte_exactly_then_lazily_appends_v6_without_rewriting_authority() {
        let root = TestRoot::new("legacy-v1-v5-lazy-successor");
        let binding = test_binding();
        let (legacy_authority_bytes, legacy_identity) =
            install_legacy_v1_v5_enrollment(&root, binding.clone());

        let mut writer =
            expect_present(EnrollmentWriter::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(
            writer.current().record.schema_version,
            ENROLLMENT_RECORD_SCHEMA_V5
        );
        let old_head = writer.current().digest;
        let successor = writer
            .transition(old_head, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .clone();
        assert_eq!(
            successor.record.schema_version,
            ENROLLMENT_RECORD_SCHEMA_VERSION
        );
        assert_eq!(successor.record.previous, Some(old_head));
        assert!(
            matches!(successor.record.checkpoint, None),
            "generation two deliberately remains in the bounded suffix"
        );
        drop(writer);

        let authority_path = enrollment_directory(&root, &binding).join(AUTHORITY_FILE);
        assert_eq!(fs::read(&authority_path).unwrap(), legacy_authority_bytes);
        let directory =
            Dir::open_ambient_dir(enrollment_directory(&root, &binding), ambient_authority())
                .unwrap();
        let reopened_authority = open_regular_readonly(&directory, AUTHORITY_FILE).unwrap();
        assert_eq!(
            authoritative_file_identity(&reopened_authority).unwrap(),
            legacy_identity
        );

        let reopened =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(reopened.current().digest, successor.digest);
        let page = reopened
            .audit_chain_page(None, MAX_ENROLLMENT_AUDIT_PAGE)
            .unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(
            page.records[0].record.schema_version,
            ENROLLMENT_RECORD_SCHEMA_VERSION
        );
        assert_eq!(
            page.records[1].record.schema_version,
            ENROLLMENT_RECORD_SCHEMA_V5
        );
    }

    #[test]
    fn legacy_v1_v5_mixed_suffix_opens_at_every_boundary_and_full_audit_pages_verify_both_codecs() {
        let root = TestRoot::new("legacy-mixed-boundary-audit");
        let binding = test_binding();
        let (legacy_authority_bytes, legacy_identity) =
            install_legacy_v1_v5_enrollment(&root, binding.clone());
        let mut writer =
            expect_present(EnrollmentWriter::open_existing(&root.app(), &binding).unwrap());
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        for ordinal in 0..(MAX_ENROLLMENT_OPEN_CHAIN_RECORDS * 2) {
            let lifecycle = if ordinal.is_multiple_of(2) {
                unsafe_idle(verified_digest, 0x2_000 + ordinal as u128)
            } else {
                safe_idle(verified_digest)
            };
            let head = writer.current().digest;
            let successor = writer.transition(head, lifecycle).unwrap().clone();
            let reopened =
                expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
            assert_eq!(
                reopened.current().digest,
                successor.digest,
                "mixed suffix offset {ordinal} must retain a bounded open proof"
            );
        }
        drop(writer);

        let authority_path = enrollment_directory(&root, &binding).join(AUTHORITY_FILE);
        assert_eq!(fs::read(&authority_path).unwrap(), legacy_authority_bytes);
        let directory =
            Dir::open_ambient_dir(enrollment_directory(&root, &binding), ambient_authority())
                .unwrap();
        assert_eq!(
            authoritative_file_identity(
                &open_regular_readonly(&directory, AUTHORITY_FILE).unwrap()
            )
            .unwrap(),
            legacy_identity
        );

        let reader =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        let mut cursor = None;
        let mut total = 0;
        let mut saw_legacy_v5 = false;
        let mut saw_current_v6 = false;
        loop {
            let page = reader
                .audit_chain_page(cursor, MAX_ENROLLMENT_AUDIT_PAGE)
                .unwrap();
            for snapshot in &page.records {
                saw_legacy_v5 |= snapshot.record.schema_version == ENROLLMENT_RECORD_SCHEMA_V5;
                saw_current_v6 |=
                    snapshot.record.schema_version == ENROLLMENT_RECORD_SCHEMA_VERSION;
            }
            total += page.records.len();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(total, MAX_ENROLLMENT_OPEN_CHAIN_RECORDS * 2 + 2);
        assert!(saw_legacy_v5 && saw_current_v6);
    }

    #[test]
    fn legacy_v1_v5_initial_publication_resumes_every_durable_cross_version_cut() {
        for state in ["record-temp", "linked-final", "head-temp", "committed-head"] {
            let root = TestRoot::new(state);
            let binding = test_binding();
            let (authority_bytes, authority_identity) =
                install_legacy_v1_v5_enrollment(&root, binding.clone());
            let enrollment = enrollment_directory(&root, &binding);
            let records = enrollment.join(RECORDS_DIRECTORY);
            let head_bytes = fs::read(enrollment.join(HEAD_FILE)).unwrap();
            let digest = ContentDigest::from_bytes(
                parse_digest(std::str::from_utf8(&head_bytes[..64]).unwrap()).unwrap(),
            );
            let target = records.join(format!("{digest}{RECORD_SUFFIX}"));
            let record_bytes = fs::read(&target).unwrap();
            let temp = records.join(format!("{RECORD_TEMP_PREFIX}{digest}"));

            match state {
                "record-temp" => {
                    fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
                    fs::rename(&target, &temp).unwrap();
                }
                "linked-final" => {
                    fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
                    fs::hard_link(&target, &temp).unwrap();
                }
                "head-temp" => {
                    fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
                    fs::write(
                        enrollment.join(format!("{HEAD_TEMP_PREFIX}legacy")),
                        &head_bytes,
                    )
                    .unwrap();
                }
                "committed-head" => {}
                _ => unreachable!(),
            }

            let mut resumed =
                EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            assert_eq!(resumed.current().digest, digest, "state={state}");
            assert_eq!(
                resumed.current().record.schema_version,
                ENROLLMENT_RECORD_SCHEMA_V5,
                "state={state}"
            );
            assert_eq!(fs::read(&target).unwrap(), record_bytes, "state={state}");
            assert_eq!(
                fs::read(enrollment.join(AUTHORITY_FILE)).unwrap(),
                authority_bytes,
                "state={state}"
            );
            let directory = Dir::open_ambient_dir(&enrollment, ambient_authority()).unwrap();
            assert_eq!(
                authoritative_file_identity(
                    &open_regular_readonly(&directory, AUTHORITY_FILE).unwrap()
                )
                .unwrap(),
                authority_identity,
                "state={state}"
            );

            let successor = resumed
                .transition(digest, EnrollmentLifecycleV1::VerifiedLocal(verified()))
                .unwrap();
            assert_eq!(
                successor.record.schema_version, ENROLLMENT_RECORD_SCHEMA_VERSION,
                "the first post-recovery successor must migrate lazily; state={state}"
            );
            assert_eq!(fs::read(&target).unwrap(), record_bytes, "state={state}");
        }

        let fresh = TestRoot::new("fresh-v2-v6-only");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&fresh.app(), binding, shadow()).unwrap();
        assert!(matches!(
            writer.reader.authority.material.claim,
            EnrollmentAuthorityClaim::CurrentV2(_)
        ));
        assert_eq!(
            writer.current().record.schema_version,
            ENROLLMENT_RECORD_SCHEMA_VERSION
        );
    }

    #[test]
    fn legacy_initial_recovery_rejects_wrong_names_and_invalid_hmac_without_cleanup() {
        let ambiguous = TestRoot::new("legacy-initial-wrong-name");
        let binding = test_binding();
        install_legacy_v1_v5_enrollment(&ambiguous, binding.clone());
        let enrollment = enrollment_directory(&ambiguous, &binding);
        let records = enrollment.join(RECORDS_DIRECTORY);
        let head = fs::read_to_string(enrollment.join(HEAD_FILE)).unwrap();
        let legacy_digest = ContentDigest::from_bytes(parse_digest(head.trim()).unwrap());
        let target = records.join(format!("{legacy_digest}{RECORD_SUFFIX}"));
        let wrong = records.join(format!("{RECORD_TEMP_PREFIX}wrong-digest"));
        fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
        fs::rename(&target, &wrong).unwrap();
        let wrong_bytes = fs::read(&wrong).unwrap();
        assert_eq!(
            EnrollmentWriter::create(&ambiguous.app(), binding.clone(), shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousInitialCreation
        );
        assert_eq!(fs::read(&wrong).unwrap(), wrong_bytes);

        let corrupt = TestRoot::new("legacy-initial-invalid-hmac");
        install_legacy_v1_v5_enrollment(&corrupt, binding.clone());
        let enrollment = enrollment_directory(&corrupt, &binding);
        let records = enrollment.join(RECORDS_DIRECTORY);
        let head = fs::read_to_string(enrollment.join(HEAD_FILE)).unwrap();
        let legacy_digest = ContentDigest::from_bytes(parse_digest(head.trim()).unwrap());
        let old_target = records.join(format!("{legacy_digest}{RECORD_SUFFIX}"));
        let mut record = decode_record(&fs::read(&old_target).unwrap()).unwrap();
        let Some(EnrollmentCheckpoint::LegacyV2(checkpoint)) = record.checkpoint.as_mut() else {
            panic!("legacy fixture must carry its v2 HMAC checkpoint")
        };
        checkpoint.authentication_tag = digest(0xee);
        let corrupt_bytes = canonical_record_bytes(&record).unwrap();
        let corrupt_digest = ContentDigest::of(&corrupt_bytes);
        fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
        fs::remove_file(old_target).unwrap();
        let corrupt_target = records.join(format!("{corrupt_digest}{RECORD_SUFFIX}"));
        fs::write(&corrupt_target, &corrupt_bytes).unwrap();
        assert_eq!(
            EnrollmentWriter::create(&corrupt.app(), binding, shadow())
                .err()
                .unwrap(),
            EnrollmentError::CheckpointLegacyAuthenticationFailed
        );
        assert_eq!(fs::read(corrupt_target).unwrap(), corrupt_bytes);
    }

    #[test]
    fn frozen_legacy_v1_v5_codec_golden_digests_remain_exact() {
        let binding = test_binding();
        let material = test_authority(binding.clone(), test_lease_resource());
        let authority_bytes = canonical_authority_claim_bytes(&material.claim).unwrap();
        assert_eq!(
            ContentDigest::of(&authority_bytes).to_string(),
            "03abcb532ff1e270a5b41c6bf9d3b970cab69eb4360590bc17a006959ee586e4"
        );

        let lifecycle = EnrollmentLifecycleV1::ShadowImport(shadow());
        let mut record = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_V5,
            generation: 1,
            previous: None,
            history_accumulator: compute_history_accumulator(1, None, None, &binding, &lifecycle)
                .unwrap(),
            lease_resource_id: test_lease_resource(),
            binding,
            lifecycle,
            checkpoint: None,
        };
        record.checkpoint = Some(
            material
                .legacy_checkpoint_for_test(
                    record.generation,
                    record.previous,
                    record.history_accumulator,
                    record.lease_resource_id,
                    &record.binding,
                    &record.lifecycle,
                )
                .unwrap(),
        );
        let record_bytes = canonical_record_bytes(&record).unwrap();
        assert_eq!(
            ContentDigest::of(&record_bytes).to_string(),
            "b4e4b5b5b3f8b80f7fea9fb16ed5858c721f3d146b8834d3828bacc8b3d4858a"
        );
    }

    #[test]
    fn explicit_root_discovery_authenticates_without_taking_the_writer_lease() {
        let root = TestRoot::new("readonly-discovery");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let expected_head = writer.current().digest();
        drop(writer);

        let before = EnrollmentInstrumentation::capture();
        let inspection =
            inspect_existing_enrollment_at(&root.path, binding.graph_resource_id()).unwrap();
        let work = before.since();
        let EnrollmentDiscoveryInspection::Present(evidence) = inspection else {
            panic!("the authenticated enrollment should be discovered");
        };
        assert_eq!(evidence.head_digest, expected_head);
        assert_eq!(evidence.binding, binding);
        assert_eq!(
            evidence.lifecycle,
            EnrollmentDiscoveryLifecycle::ShadowImport
        );
        assert_eq!(work.lease_acquisitions, 0);
    }

    #[test]
    fn unknown_tampered_future_and_noncanonical_records_fail_closed() {
        let binding = test_binding();
        let authority = test_authority(binding.clone(), test_lease_resource());
        let record =
            EnrollmentRecordV1::initial(binding, shadow(), test_lease_resource(), &authority)
                .unwrap();
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
        schema["schema_version"] = serde_json::json!(ENROLLMENT_RECORD_SCHEMA_VERSION + 1);
        assert_eq!(
            decode_record(&serde_json::to_vec(&schema).unwrap()),
            Err(EnrollmentError::UnsupportedRecordSchema(
                ENROLLMENT_RECORD_SCHEMA_VERSION + 1
            ))
        );

        let mut future: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        future["lifecycle"]["state"] = serde_json::json!("future_shared_active");
        assert_eq!(
            decode_record(&serde_json::to_vec(&future).unwrap()),
            Err(EnrollmentError::FutureUnsupportedLifecycle(
                "future_shared_active".into()
            ))
        );

        let pretty = serde_json::to_string_pretty(&record).unwrap();
        assert_eq!(
            decode_record(pretty.as_bytes()),
            Err(EnrollmentError::NonCanonicalRecord)
        );

        let duplicate = format!(
            "{{\"schema_version\":{},{}",
            ENROLLMENT_RECORD_SCHEMA_VERSION,
            std::str::from_utf8(&canonical)
                .unwrap()
                .strip_prefix('{')
                .unwrap()
        );
        assert!(matches!(
            decode_record(duplicate.as_bytes()),
            Err(EnrollmentError::Decode(detail)) if detail.contains("duplicate JSON field")
        ));

        let mut compatibility: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        compatibility["binding"]["compatibility"]["operation_schema_version"] =
            serde_json::json!(OPERATION_SCHEMA_VERSION + 1);
        assert!(matches!(
            decode_record(&serde_json::to_vec(&compatibility).unwrap()),
            Err(EnrollmentError::UnsupportedCompatibility { .. })
        ));

        let mut old: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        old["schema_version"] = serde_json::json!(ENROLLMENT_RECORD_SCHEMA_VERSION - 1);
        assert!(matches!(
            decode_record(&serde_json::to_vec(&old).unwrap()),
            Err(EnrollmentError::Decode(_))
        ));
    }

    #[test]
    fn verified_local_terminal_identity_is_exact_optional_and_rejects_sentinels() {
        let zero = zero_verified();
        zero.validate_fields().unwrap();
        let zero_json = serde_json::to_value(&zero).unwrap();
        assert!(zero_json["bootstrap_batch_id"].is_null());
        assert!(zero_json["bootstrap_terminal_part_id"].is_null());

        let nonzero = verified();
        nonzero.validate_fields().unwrap();
        assert_eq!(
            serde_json::from_value::<VerifiedLocalV1>(serde_json::to_value(&nonzero).unwrap())
                .unwrap(),
            nonzero
        );

        let mut zero_with_batch = zero.clone();
        zero_with_batch.bootstrap_batch_id = Some(BatchId::from_uuid(Uuid::from_u128(0xfeed)));
        assert_eq!(
            zero_with_batch.validate_fields(),
            Err(EnrollmentError::InvalidVerifiedLocalTerminal)
        );

        let mut nonzero_without_batch = nonzero.clone();
        nonzero_without_batch.bootstrap_batch_id = None;
        assert_eq!(
            nonzero_without_batch.validate_fields(),
            Err(EnrollmentError::InvalidVerifiedLocalTerminal)
        );

        let mut sentinel = nonzero;
        sentinel.bootstrap_batch_id = Some(BatchId::for_import(ImportId::from_digest(
            *sentinel.bootstrap_import_id.as_bytes(),
        )));
        assert_eq!(
            sentinel.validate_fields(),
            Err(EnrollmentError::InvalidVerifiedLocalTerminal)
        );

        let binding = test_binding();
        let authority = test_authority(binding.clone(), test_lease_resource());
        let initial = EnrollmentRecordV1::initial(
            binding.clone(),
            shadow(),
            test_lease_resource(),
            &authority,
        )
        .unwrap();
        let initial_bytes = canonical_record_bytes(&initial).unwrap();
        let initial_digest = ContentDigest::of(&initial_bytes);
        let lifecycle = EnrollmentLifecycleV1::VerifiedLocal(sentinel);
        let malformed = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 2,
            previous: Some(initial_digest),
            history_accumulator: compute_history_accumulator(
                2,
                Some(initial_digest),
                Some(initial.history_accumulator),
                &binding,
                &lifecycle,
            )
            .unwrap(),
            lease_resource_id: test_lease_resource(),
            binding,
            lifecycle,
            checkpoint: None,
        };
        assert_eq!(
            decode_record(&serde_json::to_vec(&malformed).unwrap()),
            Err(EnrollmentError::InvalidVerifiedLocalTerminal)
        );
    }

    #[test]
    fn lifecycle_transition_matrix_and_local_substates_are_exact() {
        let current_digest = digest(30);
        let states = [
            EnrollmentLifecycleV1::ShadowImport(shadow()),
            EnrollmentLifecycleV1::VerifiedLocal(verified()),
            unsafe_idle(current_digest, 31),
            blocked(current_digest),
        ];
        for (from_index, from) in states.iter().enumerate() {
            for (to_index, to) in states.iter().enumerate() {
                let expected = matches!(
                    (from_index, to_index),
                    (0, 1) | (0, 3) | (1, 2) | (1, 3) | (2, 3)
                );
                assert_eq!(
                    validate_transition(from, to, current_digest).is_ok(),
                    expected,
                    "{from_index} -> {to_index}"
                );
            }
        }

        let unsafe_a = match unsafe_idle(current_digest, 40) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        let unsafe_b = match unsafe_idle(current_digest, 41) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        let safe = match safe_idle(current_digest) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            _ => unreachable!(),
        };
        let published = match published(current_digest, 40, test_binding().archive_resource_id) {
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

        // The anchor is part of the LocalActive -> LocalActive contract: an
        // otherwise legal handoff that restates the anchor is rejected.
        let mut divergent = unsafe_b.clone();
        divergent.anchor = local_anchor(digest(39));
        assert!(legal_local_active_transition(&unsafe_a, &divergent));
        assert_eq!(
            validate_transition(
                &EnrollmentLifecycleV1::LocalActive(unsafe_a.clone()),
                &EnrollmentLifecycleV1::LocalActive(divergent),
                current_digest,
            ),
            Err(EnrollmentError::IllegalTransition)
        );
        assert!(validate_transition(
            &EnrollmentLifecycleV1::LocalActive(unsafe_a.clone()),
            &EnrollmentLifecycleV1::LocalActive(unsafe_b),
            current_digest,
        )
        .is_ok());

        // ... and a VerifiedLocal -> LocalActive transition must mint the
        // anchor from the exact committed predecessor record digest.
        let mut foreign = unsafe_a;
        foreign.anchor = local_anchor(digest(38));
        assert_eq!(
            validate_transition(
                &EnrollmentLifecycleV1::VerifiedLocal(verified()),
                &EnrollmentLifecycleV1::LocalActive(foreign),
                current_digest,
            ),
            Err(EnrollmentError::IllegalTransition)
        );
    }

    #[test]
    fn verified_local_can_fail_closed_to_blocked_at_exact_prior_digest() {
        let current = EnrollmentLifecycleV1::VerifiedLocal(verified());
        let current_digest = digest(29);
        assert!(validate_transition(&current, &blocked(current_digest), current_digest).is_ok());
        assert_eq!(
            validate_transition(&current, &blocked(digest(28)), current_digest),
            Err(EnrollmentError::IllegalTransition)
        );
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
            writer.transition(stale, unsafe_idle(digest_verified, 50)),
            Err(EnrollmentError::StaleCompareAndSwap)
        );
        let active_digest = writer
            .transition(digest_verified, unsafe_idle(digest_verified, 50))
            .unwrap()
            .digest;
        let recovered_unclean_digest = writer
            .transition(active_digest, unsafe_idle(digest_verified, 53))
            .unwrap()
            .digest;
        let safe_digest = writer
            .transition(recovered_unclean_digest, safe_idle(digest_verified))
            .unwrap()
            .digest;
        let unsafe_digest = writer
            .transition(safe_digest, unsafe_idle(digest_verified, 51))
            .unwrap()
            .digest;
        let published_digest = writer
            .transition(
                unsafe_digest,
                published(digest_verified, 51, binding.archive_resource_id),
            )
            .unwrap()
            .digest;
        let recovered_digest = writer
            .transition(published_digest, unsafe_idle(digest_verified, 52))
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
        assert!(matches!(
            reader.audit_chain_page(None, MAX_ENROLLMENT_AUDIT_PAGE + 1),
            Err(EnrollmentError::InvalidPageLimit(limit))
                if limit == MAX_ENROLLMENT_AUDIT_PAGE + 1
        ));
    }

    #[test]
    fn verified_to_blocked_narrow_cas_survives_restart_and_rejects_stale_head() {
        let root = TestRoot::new("verified-blocked-restart");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let shadow_digest = writer.current().digest;
        let verified_digest = writer
            .transition(
                shadow_digest,
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
            )
            .unwrap()
            .digest;
        drop(writer);

        let mut reopened =
            expect_present(EnrollmentWriter::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(
            reopened.block_current(shadow_digest, "proof.failed".into(), digest(27)),
            Err(EnrollmentError::StaleCompareAndSwap)
        );
        let blocked_digest = reopened
            .block_current(verified_digest, "proof.failed".into(), digest(27))
            .unwrap()
            .digest;
        drop(reopened);

        let reader =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(reader.current().digest, blocked_digest);
        assert_eq!(reader.current().record.generation, 3);
        assert!(matches!(
            reader.current().record.lifecycle,
            EnrollmentLifecycleV1::Blocked(_)
        ));
    }

    #[test]
    fn crash_cuts_leave_old_or_new_valid_head() {
        for (cut, expect_new) in [
            (CommitCut::AfterRecordTempCreate, Some(false)),
            (CommitCut::AfterRecordWrite, Some(false)),
            (CommitCut::AfterRecordFileSync, Some(false)),
            (CommitCut::AfterRecordLink, Some(false)),
            (CommitCut::AfterRecordInsert, Some(false)),
            (CommitCut::AfterRecordsDirectorySync, Some(false)),
            (CommitCut::AfterHeadTempCreate, Some(false)),
            (CommitCut::AfterHeadWrite, Some(false)),
            (CommitCut::AfterHeadFileSync, Some(false)),
            (CommitCut::AfterHeadReplace, None),
            (CommitCut::AfterEnrollmentDirectorySync, Some(true)),
        ] {
            let root = TestRoot::new("crash");
            let binding = test_binding();
            let mut writer =
                EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            let old = writer.current().digest;
            let next = EnrollmentRecordV1::successor(
                writer.current(),
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
                &writer.reader.authority.material,
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
            match expect_new {
                Some(true) => assert_eq!(reader.current().digest, expected_new),
                Some(false) => assert_eq!(reader.current().digest, old),
                None => assert!(
                    matches!(reader.current().digest, found if found == old || found == expected_new)
                ),
            }
        }
    }

    #[test]
    fn abrupt_child_process_crash_cuts_reopen_old_or_new_valid_authority() {
        for (name, expect_new) in [
            ("record_temp_create", Some(false)),
            ("record_write", Some(false)),
            ("record_file_sync", Some(false)),
            ("record_link", Some(false)),
            ("record_insert", Some(false)),
            ("records_directory_sync", Some(false)),
            ("head_temp_create", Some(false)),
            ("head_write", Some(false)),
            ("head_file_sync", Some(false)),
            ("head_replace", None),
            ("enrollment_directory_sync", Some(true)),
        ] {
            let root = TestRoot::new("abrupt-crash");
            let binding = test_binding();
            let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            let old = writer.current().digest;
            let next = EnrollmentRecordV1::successor(
                writer.current(),
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
                &writer.reader.authority.material,
            )
            .unwrap();
            let expected_new = ContentDigest::of(&canonical_record_bytes(&next).unwrap());
            drop(writer);

            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "oplog::enrollment::tests::abrupt_crash_child_probe",
                    "--nocapture",
                ])
                .env("TINE_ENROLLMENT_CRASH_CHILD_ROOT", &root.path)
                .env("TINE_ENROLLMENT_CRASH_CHILD_CUT", name)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86), "child did not exit at {name}");

            let reader =
                expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
            match expect_new {
                Some(true) => assert_eq!(reader.current().digest, expected_new),
                Some(false) => assert_eq!(reader.current().digest, old),
                None => assert!(
                    matches!(reader.current().digest, found if found == old || found == expected_new)
                ),
            }
            drop(reader);
            if name == "record_link" {
                let mut retry =
                    expect_present(EnrollmentWriter::open_existing(&root.app(), &binding).unwrap());
                assert_eq!(
                    retry
                        .transition(
                            retry.current().digest,
                            EnrollmentLifecycleV1::VerifiedLocal(verified()),
                        )
                        .unwrap()
                        .digest,
                    expected_new
                );
            }
        }
    }

    #[test]
    fn abrupt_crash_child_probe() {
        let Some(path) = std::env::var_os("TINE_ENROLLMENT_CRASH_CHILD_ROOT") else {
            return;
        };
        let name = std::env::var("TINE_ENROLLMENT_CRASH_CHILD_CUT").unwrap();
        let cut = match name.as_str() {
            "record_temp_create" => CommitCut::AfterRecordTempCreate,
            "record_write" => CommitCut::AfterRecordWrite,
            "record_file_sync" => CommitCut::AfterRecordFileSync,
            "record_link" => CommitCut::AfterRecordLink,
            "record_insert" => CommitCut::AfterRecordInsert,
            "records_directory_sync" => CommitCut::AfterRecordsDirectorySync,
            "head_temp_create" => CommitCut::AfterHeadTempCreate,
            "head_write" => CommitCut::AfterHeadWrite,
            "head_file_sync" => CommitCut::AfterHeadFileSync,
            "head_replace" => CommitCut::AfterHeadReplace,
            "enrollment_directory_sync" => CommitCut::AfterEnrollmentDirectorySync,
            _ => panic!("unknown crash cut {name}"),
        };
        let app = EnrollmentApplicationRoot::open_for_harness(Path::new(&path)).unwrap();
        let mut writer =
            expect_present(EnrollmentWriter::open_existing(&app, &test_binding()).unwrap());
        let current = writer.current().digest;
        assert!(matches!(
            writer.transition_at_cut(
                current,
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
                cut
            ),
            Err(EnrollmentError::InjectedCrashCut(_))
        ));
        std::process::exit(86);
    }

    #[test]
    fn abrupt_initial_creation_cuts_resume_and_open_exact_authority() {
        for name in [
            "record_temp_create",
            "record_write",
            "record_file_sync",
            "record_link",
            "record_insert",
            "records_directory_sync",
            "head_temp_create",
            "head_write",
            "head_file_sync",
            "head_replace",
            "enrollment_directory_sync",
        ] {
            let root = TestRoot::new("abrupt-initial");
            let binding = test_binding();
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "oplog::enrollment::tests::abrupt_initial_creation_child_probe",
                    "--nocapture",
                ])
                .env("TINE_ENROLLMENT_INITIAL_CRASH_CHILD_ROOT", &root.path)
                .env("TINE_ENROLLMENT_INITIAL_CRASH_CHILD_CUT", name)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86), "child did not exit at {name}");

            let resumed = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            let initial = resumed.current().digest;
            assert_eq!(resumed.current().generation(), 1);
            drop(resumed);
            assert_eq!(
                expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap())
                    .current()
                    .digest,
                initial
            );
        }
    }

    #[test]
    fn abrupt_initial_creation_child_probe() {
        let Some(path) = std::env::var_os("TINE_ENROLLMENT_INITIAL_CRASH_CHILD_ROOT") else {
            return;
        };
        let name = std::env::var("TINE_ENROLLMENT_INITIAL_CRASH_CHILD_CUT").unwrap();
        let cut = match name.as_str() {
            "record_temp_create" => CommitCut::AfterRecordTempCreate,
            "record_write" => CommitCut::AfterRecordWrite,
            "record_file_sync" => CommitCut::AfterRecordFileSync,
            "record_link" => CommitCut::AfterRecordLink,
            "record_insert" => CommitCut::AfterRecordInsert,
            "records_directory_sync" => CommitCut::AfterRecordsDirectorySync,
            "head_temp_create" => CommitCut::AfterHeadTempCreate,
            "head_write" => CommitCut::AfterHeadWrite,
            "head_file_sync" => CommitCut::AfterHeadFileSync,
            "head_replace" => CommitCut::AfterHeadReplace,
            "enrollment_directory_sync" => CommitCut::AfterEnrollmentDirectorySync,
            _ => panic!("unknown initial crash cut {name}"),
        };
        let app = EnrollmentApplicationRoot::open_for_harness(Path::new(&path)).unwrap();
        assert!(matches!(
            EnrollmentWriter::create_at_cut(&app, test_binding(), shadow(), cut),
            Err(EnrollmentError::InjectedCrashCut(_))
        ));
        std::process::exit(86);
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
        bad.checkpoint = None;
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
    fn forged_accumulator_and_cyclic_pointer_attempts_fail_closed() {
        let root = TestRoot::new("accumulator-forgery");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let current = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .clone();
        drop(writer);

        let mut accumulator_forgery = current.record.clone();
        accumulator_forgery.history_accumulator = digest(92);
        let bytes = canonical_record_bytes(&accumulator_forgery).unwrap();
        let forged_digest = ContentDigest::of(&bytes);
        fs::write(record_path(&root, &binding, forged_digest), bytes).unwrap();
        write_head(&root, &binding, forged_digest);
        assert_eq!(
            EnrollmentReader::open_existing(&root.app(), &binding)
                .err()
                .unwrap(),
            EnrollmentError::HistoryAccumulatorMismatch
        );

        let mut cyclic_pointer = current.record;
        cyclic_pointer.previous = Some(current.digest);
        let bytes = serde_json::to_vec(&cyclic_pointer).unwrap();
        fs::write(record_path(&root, &binding, current.digest), bytes).unwrap();
        write_head(&root, &binding, current.digest);
        assert_eq!(
            EnrollmentReader::open_existing(&root.app(), &binding)
                .err()
                .unwrap(),
            EnrollmentError::RecordDigestMismatch(current.digest)
        );
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
        let resumed = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        assert_eq!(resumed.current().generation(), 1);
        drop(resumed);

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
        assert!(authoritative_file_kind_allowed(true, false, false));
        assert!(!authoritative_file_kind_allowed(true, false, true));
        assert!(!authoritative_file_kind_allowed(true, true, false));
        assert!(!authoritative_file_kind_allowed(false, false, false));
    }

    #[test]
    fn inert_history_artifacts_do_not_make_current_open_lifetime_dependent() {
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
            EnrollmentReader::open_existing(&root.app(), &binding).unwrap(),
            EnrollmentOpen::Present(_)
        ));

        let root = TestRoot::new("chain-bound");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let mut snapshot = writer.current().clone();
        let mut verified_digest = snapshot.digest;
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        for index in 0..MAX_ENROLLMENT_OPEN_CHAIN_RECORDS {
            let lifecycle = match index {
                0 => EnrollmentLifecycleV1::VerifiedLocal(verified()),
                1 => unsafe_idle(verified_digest, 200),
                _ if index % 2 == 0 => safe_idle(verified_digest),
                _ => unsafe_idle(verified_digest, 200 + index as u128),
            };
            let record = EnrollmentRecordV1::successor(
                &snapshot,
                lifecycle,
                &writer.reader.authority.material,
            )
            .unwrap();
            let bytes = canonical_record_bytes(&record).unwrap();
            let digest = ContentDigest::of(&bytes);
            fs::write(records.join(format!("{digest}{RECORD_SUFFIX}")), bytes).unwrap();
            if index == 0 {
                verified_digest = digest;
            }
            snapshot = EnrollmentSnapshot { digest, record };
        }
        drop(writer);
        write_head(&root, &binding, snapshot.digest);
        let reopened =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(reopened.current().digest, snapshot.digest);
    }

    #[test]
    fn legitimate_journal_remains_openable_and_page_auditable_after_2048_transitions() {
        let root = TestRoot::new("journal-longevity");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let shadow_digest = writer.current().digest;
        let verified_digest = writer
            .transition(
                shadow_digest,
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
            )
            .unwrap()
            .digest;
        let mut current = writer
            .transition(verified_digest, unsafe_idle(verified_digest, 700))
            .unwrap()
            .digest;
        for index in 0..2_049_u64 {
            let next = if index % 2 == 0 {
                safe_idle(verified_digest)
            } else {
                unsafe_idle(verified_digest, 701 + u128::from(index))
            };
            current = writer.transition(current, next).unwrap().digest;
        }
        drop(writer);

        ENROLLMENT_RECORD_READS.with(|reads| reads.set(0));
        let reader =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        let open_reads = ENROLLMENT_RECORD_READS.with(std::cell::Cell::get);
        assert_eq!(open_reads, 4, "head must reach generation-2049 checkpoint");
        assert_eq!(reader.current().digest, current);
        for page_size in [1, MAX_ENROLLMENT_AUDIT_PAGE] {
            let mut next = None;
            let mut audited = 0_u64;
            let mut audit_reads = 0_usize;
            let mut pages = 0_usize;
            loop {
                ENROLLMENT_RECORD_READS.with(|reads| reads.set(0));
                let page = reader.audit_chain_page(next, page_size).unwrap();
                let page_reads = ENROLLMENT_RECORD_READS.with(std::cell::Cell::get);
                assert!(
                    page_reads <= page_size + usize::from(next.is_some()),
                    "bounded audit page read {page_reads} records for limit {page_size}"
                );
                audit_reads += page_reads;
                pages += 1;
                audited += page.records.len() as u64;
                next = page.next;
                if next.is_none() {
                    break;
                }
            }
            assert_eq!(audited, reader.current().record.generation);
            assert_eq!(
                audit_reads,
                usize::try_from(audited).unwrap() + pages - 1,
                "each resumed page reads exactly one fixed-size successor proof"
            );
        }
    }

    /// Drive one enrollment to `LocalActive` and then append `handoffs`
    /// alternating `Safe`/`Unsafe` handoff records. Returns the committed
    /// `VerifiedLocal` record digest and the resulting head.
    fn local_active_journal(
        root: &TestRoot,
        binding: &EnrollmentBindingV1,
        handoffs: u64,
    ) -> (ContentDigest, ContentDigest) {
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        let mut current = writer
            .transition(verified_digest, unsafe_idle(verified_digest, 900))
            .unwrap()
            .digest;
        for index in 0..handoffs {
            let next = if index % 2 == 0 {
                safe_idle(verified_digest)
            } else {
                unsafe_idle(verified_digest, 901 + u128::from(index))
            };
            current = writer.transition(current, next).unwrap().digest;
        }
        // The journal must end on `Unsafe`+`Idle`, which is the only state a
        // promoted reopen accepts.
        assert!(handoffs % 2 == 0 && handoffs > 0);
        (verified_digest, current)
    }

    fn measured_promoted_reopen(
        root: &TestRoot,
        binding: &EnrollmentBindingV1,
    ) -> (PromotedBootstrapAnchor, usize) {
        ENROLLMENT_RECORD_READS.with(|reads| reads.set(0));
        let anchor = reopen_promoted_bootstrap_anchor(&root.app(), binding).unwrap();
        (anchor, ENROLLMENT_RECORD_READS.with(std::cell::Cell::get))
    }

    /// The bounded-lifetime defect this anchor exists to remove: the original
    /// `VerifiedLocal` record used to be found by walking back at most
    /// `MAX_ENROLLMENT_OPEN_CHAIN_RECORDS * MAX_ENROLLMENT_AUDIT_PAGE` = 4,096
    /// records, so a graph became permanently unopenable after roughly that many
    /// clean sessions.
    #[test]
    fn more_than_four_thousand_local_active_handoffs_stay_openable_at_bounded_cost() {
        let short_root = TestRoot::new("anchor-short");
        let binding = test_binding();
        let short_handoffs = 2_u64;
        let (short_verified, _) = local_active_journal(&short_root, &binding, short_handoffs);
        let (short_anchor, short_reads) = measured_promoted_reopen(&short_root, &binding);
        assert_eq!(short_anchor.committed().generation(), short_handoffs + 3);

        // 4,162 = 2 + 65 * 64, so the long head sits at exactly the same
        // distance from its authenticated checkpoint as the short one. Any
        // read-count difference is therefore lifetime dependence, not
        // checkpoint phase.
        let long_root = TestRoot::new("anchor-long");
        let handoffs = 4_162_u64;
        let (long_verified, long_head) = local_active_journal(&long_root, &binding, handoffs);
        let (long_anchor, long_reads) = measured_promoted_reopen(&long_root, &binding);
        assert_eq!(long_anchor.committed().generation(), handoffs + 3);
        assert!(
            handoffs + 1 > (MAX_ENROLLMENT_OPEN_CHAIN_RECORDS * MAX_ENROLLMENT_AUDIT_PAGE) as u64,
            "the journal must exceed the removed predecessor-walk limit"
        );
        assert_eq!(long_anchor.committed().enrollment_head(), long_head);

        // Reopen cost does not depend on the lifetime generation: the bounded
        // checkpoint/open proof plus exactly one anchored record read.
        let expected_reads = |generation: u64| {
            2 + usize::try_from((generation - 1) % MAX_ENROLLMENT_OPEN_CHAIN_RECORDS as u64)
                .unwrap()
        };
        assert_eq!(
            short_reads,
            expected_reads(short_anchor.committed().generation())
        );
        assert_eq!(
            long_reads,
            expected_reads(long_anchor.committed().generation())
        );
        assert_eq!(
            short_reads,
            long_reads,
            "anchored reopen must read the same number of records at generation \
             {} and generation {}",
            short_anchor.committed().generation(),
            long_anchor.committed().generation()
        );
        assert!(
            long_reads <= MAX_ENROLLMENT_OPEN_CHAIN_RECORDS + 1,
            "anchored reopen read {long_reads} records"
        );

        // The reconstructed anchor is exactly the original VerifiedLocal one.
        let expected = verified();
        for anchor in [&short_anchor, &long_anchor] {
            assert_eq!(
                anchor.verification_digest(),
                expected.verification_digest().unwrap()
            );
            assert_eq!(anchor.bootstrap_import_id(), expected.bootstrap_import_id);
            assert_eq!(anchor.bootstrap_part_count(), expected.bootstrap_part_count);
            assert_eq!(
                anchor.accepted_history_record_count(),
                expected.accepted_history_record_count
            );
            assert_eq!(
                anchor.acceptance_sequence(),
                expected.accepted_frontier_anchor.acceptance_sequence
            );
            assert_eq!(
                anchor.accepted_frontier_state_digest(),
                expected
                    .accepted_frontier_anchor
                    .accepted_frontier_state_digest
            );
            assert_eq!(
                anchor.history_generation(),
                expected.accepted_frontier_anchor.history_generation
            );
            assert_eq!(
                anchor.history_root(),
                expected.accepted_frontier_anchor.history_root
            );
            assert_eq!(anchor.binding(), &binding);
        }

        let (predecessor, committed) = long_anchor.into_predecessor_evidence();
        assert_eq!(predecessor.enrollment_head(), long_verified);
        assert_eq!(predecessor.preparation_id(), expected.preparation_id);
        assert_eq!(
            predecessor.bootstrap_batch_id(),
            expected.bootstrap_batch_id
        );
        assert_eq!(
            predecessor.accepted_frontier_state_digest(),
            expected
                .accepted_frontier_anchor
                .accepted_frontier_state_digest
        );
        assert_eq!(
            predecessor.verification_digest(),
            expected.verification_digest().unwrap()
        );
        assert_eq!(committed.enrollment_head(), long_head);
        assert_ne!(short_verified, long_verified);

        // Forensics: the original VerifiedLocal record is never deleted or
        // compacted, so it is still on disk and still auditable by walking the
        // complete chain, however long the journal has grown.
        assert!(record_path(&long_root, &binding, long_verified).exists());
        let stored =
            decode_record(&fs::read(record_path(&long_root, &binding, long_verified)).unwrap())
                .unwrap();
        match stored.lifecycle() {
            EnrollmentLifecycleV1::VerifiedLocal(stored_verified) => {
                assert_eq!(stored_verified, &expected);
            }
            other => panic!("anchored record is not VerifiedLocal: {other:?}"),
        }

        let reader =
            expect_present(EnrollmentReader::open_existing(&long_root.app(), &binding).unwrap());
        let mut cursor = None;
        let mut audited = 0_u64;
        let mut found_verified = None;
        loop {
            let page = reader
                .audit_chain_page(cursor, MAX_ENROLLMENT_AUDIT_PAGE)
                .unwrap();
            for snapshot in &page.records {
                audited += 1;
                if matches!(
                    snapshot.record.lifecycle(),
                    EnrollmentLifecycleV1::VerifiedLocal(_)
                ) {
                    found_verified = Some(snapshot.digest);
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(audited, handoffs + 3);
        assert_eq!(found_verified, Some(long_verified));
    }

    #[test]
    fn local_active_anchor_is_immutable_across_handoff_session_and_checkpoint_transitions() {
        let root = TestRoot::new("anchor-immutable");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        let minted = local_anchor(verified_digest);
        let mut current = writer
            .transition(verified_digest, unsafe_idle(verified_digest, 1_400))
            .unwrap()
            .digest;

        // Safe/Unsafe flips, session changes, a Published exclusion and its
        // recovery, and at least one authenticated checkpoint boundary.
        let mut checkpoints = 0_usize;
        while writer.current().generation() < 70 {
            let generation = writer.current().generation();
            let next = match generation % 4 {
                0 => safe_idle(verified_digest),
                1 => unsafe_idle(verified_digest, 1_400 + u128::from(generation)),
                2 => published(
                    verified_digest,
                    1_400 + u128::from(generation) - 1,
                    binding.archive_resource_id,
                ),
                _ => unsafe_idle(verified_digest, 1_500 + u128::from(generation)),
            };
            current = writer.transition(current, next).unwrap().digest;
            match writer.current().record.lifecycle() {
                EnrollmentLifecycleV1::LocalActive(active) => {
                    assert_eq!(
                        active.anchor, minted,
                        "anchor moved at generation {generation}"
                    );
                }
                other => panic!("unexpected lifecycle {other:?}"),
            }
            if writer.current().record.checkpoint.is_some() {
                checkpoints += 1;
            }
        }
        assert!(checkpoints > 0, "the journal must cross a checkpoint");

        // A handoff that restates the anchor is refused even though its
        // handoff/exclusion move is otherwise legal.
        let mut restated = match safe_idle(verified_digest) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            other => panic!("unexpected lifecycle {other:?}"),
        };
        restated
            .anchor
            .accepted_frontier_anchor
            .accepted_frontier_state_digest = digest(210);
        assert_eq!(
            writer.transition(current, EnrollmentLifecycleV1::LocalActive(restated)),
            Err(EnrollmentError::IllegalTransition)
        );
        drop(writer);

        // Every reopen path still reconstructs the originally minted anchor.
        let reopened = reopen_promoted_bootstrap_anchor(&root.app(), &binding).unwrap();
        assert_eq!(reopened.committed().anchor, minted);
        assert_eq!(reopened.committed().enrollment_head(), current);
        let committed = reopen_committed_local_active_for_session(
            &root.app(),
            &binding,
            verified().verification_digest().unwrap(),
        )
        .unwrap();
        assert_eq!(committed.anchor, minted);
    }

    #[test]
    fn local_active_handoff_crash_cuts_preserve_the_exact_anchor() {
        for (cut, expect_new) in [
            (CommitCut::AfterRecordTempCreate, Some(false)),
            (CommitCut::AfterRecordWrite, Some(false)),
            (CommitCut::AfterRecordFileSync, Some(false)),
            (CommitCut::AfterRecordLink, Some(false)),
            (CommitCut::AfterRecordInsert, Some(false)),
            (CommitCut::AfterRecordsDirectorySync, Some(false)),
            (CommitCut::AfterHeadTempCreate, Some(false)),
            (CommitCut::AfterHeadWrite, Some(false)),
            (CommitCut::AfterHeadFileSync, Some(false)),
            (CommitCut::AfterHeadReplace, None),
            (CommitCut::AfterEnrollmentDirectorySync, Some(true)),
        ] {
            let root = TestRoot::new("anchor-crash");
            let binding = test_binding();
            let mut writer =
                EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            let initial = writer.current().digest;
            let verified_digest = writer
                .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
                .unwrap()
                .digest;
            let minted = local_anchor(verified_digest);
            let old = writer
                .transition(verified_digest, unsafe_idle(verified_digest, 1_600))
                .unwrap()
                .digest;
            let next = EnrollmentRecordV1::successor(
                writer.current(),
                safe_idle(verified_digest),
                &writer.reader.authority.material,
            )
            .unwrap();
            let expected_new = ContentDigest::of(&canonical_record_bytes(&next).unwrap());
            assert!(matches!(
                writer.transition_at_cut(old, safe_idle(verified_digest), cut),
                Err(EnrollmentError::InjectedCrashCut(_))
            ));
            drop(writer);

            let reader =
                expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
            let head = reader.current().digest;
            match expect_new {
                Some(true) => assert_eq!(head, expected_new),
                Some(false) => assert_eq!(head, old),
                None => assert!(head == old || head == expected_new),
            }
            match reader.current().record.lifecycle() {
                EnrollmentLifecycleV1::LocalActive(active) => assert_eq!(active.anchor, minted),
                other => panic!("crash cut {cut:?} left lifecycle {other:?}"),
            }
        }
    }

    #[test]
    fn forged_and_divergent_local_active_anchors_fail_closed() {
        // 1. A forged anchor naming a foreign predecessor is refused at the
        //    VerifiedLocal -> LocalActive transition itself.
        let root = TestRoot::new("anchor-forged-transition");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        assert_eq!(
            writer.transition(verified_digest, unsafe_idle(initial, 1_700)),
            Err(EnrollmentError::IllegalTransition)
        );
        let head = writer
            .transition(verified_digest, unsafe_idle(verified_digest, 1_700))
            .unwrap()
            .digest;

        // 2. An anchor whose retained bootstrap identity is internally
        //    inconsistent is rejected when the record is decoded, so it cannot
        //    survive a round trip through the store either.
        let mut inconsistent = match unsafe_idle(verified_digest, 1_701) {
            EnrollmentLifecycleV1::LocalActive(value) => value,
            other => panic!("unexpected lifecycle {other:?}"),
        };
        inconsistent.anchor.accepted_history_record_count += 1;
        assert_eq!(
            EnrollmentLifecycleV1::LocalActive(inconsistent).validate(&binding, Some(head)),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );

        // 3. A record that authenticates under the real authority but whose
        //    anchor names a non-VerifiedLocal record fails the anchored reopen.
        let forge = |root: &TestRoot,
                     writer: &EnrollmentWriter,
                     head: ContentDigest,
                     anchor: LocalActiveAnchorV1| {
            let lifecycle = EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
                verification_digest: verified().verification_digest().unwrap(),
                anchor,
                handoff: HandoffV1::Unsafe {
                    session_id: SessionId::from_uuid(Uuid::from_u128(1_800)),
                },
                exclusion: LocalExclusionV1::Idle,
            });
            let mut record = EnrollmentRecordV1 {
                schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
                generation: 65,
                previous: Some(head),
                history_accumulator: compute_history_accumulator(
                    65,
                    Some(head),
                    Some(writer.current().record.history_accumulator),
                    &writer.current().record.binding,
                    &lifecycle,
                )
                .unwrap(),
                lease_resource_id: writer.current().record.lease_resource_id,
                binding: writer.current().record.binding.clone(),
                lifecycle,
                checkpoint: None,
            };
            record.checkpoint = Some(
                writer
                    .reader
                    .authority
                    .material
                    .checkpoint_for(
                        record.generation,
                        record.previous,
                        record.history_accumulator,
                        record.lease_resource_id,
                        &record.binding,
                        &record.lifecycle,
                    )
                    .unwrap(),
            );
            let bytes = canonical_record_bytes(&record).unwrap();
            let forged_digest = ContentDigest::of(&bytes);
            fs::write(
                record_path(root, &writer.current().record.binding, forged_digest),
                bytes,
            )
            .unwrap();
            forged_digest
        };

        let mut shadow_anchor = local_anchor(verified_digest);
        shadow_anchor.verified_local_record_digest = initial;
        let shadow_forged = forge(&root, &writer, head, shadow_anchor);

        let mut absent_anchor = local_anchor(verified_digest);
        absent_anchor.verified_local_record_digest = digest(211);
        let absent_forged = forge(&root, &writer, head, absent_anchor);

        let mut divergent_anchor = local_anchor(verified_digest);
        divergent_anchor.bootstrap_import_id = digest(212);
        let divergent_forged = forge(&root, &writer, head, divergent_anchor);
        drop(writer);

        write_head(&root, &binding, shadow_forged);
        assert_eq!(
            reopen_promoted_bootstrap_anchor(&root.app(), &binding)
                .err()
                .map(|error| error.to_string()),
            Some(
                VerifiedLocalCompositionError::WrongLifecycle(
                    "the anchored enrollment record is not the original VerifiedLocal record",
                )
                .to_string()
            )
        );

        write_head(&root, &binding, absent_forged);
        assert!(matches!(
            reopen_promoted_bootstrap_anchor(&root.app(), &binding),
            Err(VerifiedLocalCompositionError::Enrollment(
                EnrollmentError::MissingChainRecord(_)
            ))
        ));

        write_head(&root, &binding, divergent_forged);
        assert_eq!(
            reopen_promoted_bootstrap_anchor(&root.app(), &binding)
                .err()
                .map(|error| error.to_string()),
            Some(
                VerifiedLocalCompositionError::ProofMismatch(
                    "the committed LocalActive anchor diverges from the original VerifiedLocal record",
                )
                .to_string()
            )
        );

        // 4. A forged checkpoint over a tampered anchor fails authentication.
        write_head(&root, &binding, divergent_forged);
        let mut tampered =
            decode_record(&fs::read(record_path(&root, &binding, divergent_forged)).unwrap())
                .unwrap();
        let EnrollmentCheckpoint::CurrentV3(checkpoint) = tampered.checkpoint.as_mut().unwrap()
        else {
            panic!("fresh records use the v3 integrity checkpoint");
        };
        checkpoint.integrity_tag ^= 1;
        let bytes = canonical_record_bytes(&tampered).unwrap();
        let tampered_digest = ContentDigest::of(&bytes);
        fs::write(record_path(&root, &binding, tampered_digest), bytes).unwrap();
        write_head(&root, &binding, tampered_digest);
        assert!(matches!(
            reopen_promoted_bootstrap_anchor(&root.app(), &binding),
            Err(VerifiedLocalCompositionError::Enrollment(
                EnrollmentError::CheckpointIntegrityFailed
            ))
        ));
    }

    #[test]
    fn zero_nonzero_and_multipart_bootstrap_anchors_are_exact() {
        let predecessor = digest(214);
        for source in [verified(), zero_verified(), multipart_verified()] {
            source.validate_fields().unwrap();
            let anchor = LocalActiveAnchorV1::from_verified_local(&source, predecessor);
            anchor.validate().unwrap();
            assert_eq!(anchor.verified_local_record_digest, predecessor);
            assert_eq!(anchor.bootstrap_batch_id, source.bootstrap_batch_id);
            assert_eq!(
                anchor.accepted_frontier_anchor,
                source.accepted_frontier_anchor
            );
            let json = serde_json::to_value(anchor).unwrap();
            assert_eq!(
                json["bootstrap_batch_id"].is_null(),
                source.bootstrap_part_count == 0
            );
            assert_eq!(
                serde_json::from_value::<LocalActiveAnchorV1>(json.clone()).unwrap(),
                anchor
            );
            let mut unknown = json;
            unknown["unexpected"] = serde_json::json!(1);
            assert!(serde_json::from_value::<LocalActiveAnchorV1>(unknown).is_err());
        }

        let base = LocalActiveAnchorV1::from_verified_local(&verified(), predecessor);
        let zero = LocalActiveAnchorV1::from_verified_local(&zero_verified(), predecessor);

        let mut zero_with_batch = zero;
        zero_with_batch.bootstrap_batch_id = base.bootstrap_batch_id;
        assert_eq!(
            zero_with_batch.validate(),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );

        let mut nonzero_without_batch = base;
        nonzero_without_batch.bootstrap_batch_id = None;
        assert_eq!(
            nonzero_without_batch.validate(),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );

        let mut skewed = base;
        skewed.accepted_frontier_anchor.acceptance_sequence += 1;
        assert_eq!(
            skewed.validate(),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );

        let mut skewed_history = base;
        skewed_history.accepted_frontier_anchor.history_generation += 1;
        assert_eq!(
            skewed_history.validate(),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );

        let mut empty_root = base;
        empty_root.accepted_frontier_anchor.history_root =
            super::super::object_store::EngineHistoryStore::empty_root();
        assert_eq!(
            empty_root.validate(),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );

        let mut sentinel = base;
        sentinel.bootstrap_batch_id = Some(BatchId::for_import(ImportId::from_digest(
            *sentinel.bootstrap_import_id.as_bytes(),
        )));
        assert_eq!(
            sentinel.validate(),
            Err(EnrollmentError::InvalidLocalActiveAnchor)
        );
    }

    /// A zero-part bootstrap is a legal terminal identity, so it must also
    /// survive the full persist/reopen path.
    #[test]
    fn zero_bootstrap_enrollment_activates_and_reopens_through_its_anchor() {
        let root = TestRoot::new("anchor-zero-bootstrap");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(
                initial,
                EnrollmentLifecycleV1::VerifiedLocal(zero_verified()),
            )
            .unwrap()
            .digest;
        let anchor = LocalActiveAnchorV1::from_verified_local(&zero_verified(), verified_digest);
        let lifecycle = EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
            verification_digest: zero_verified().verification_digest().unwrap(),
            anchor,
            handoff: HandoffV1::Unsafe {
                session_id: SessionId::from_uuid(Uuid::from_u128(1_900)),
            },
            exclusion: LocalExclusionV1::Idle,
        });
        let head = writer
            .transition(verified_digest, lifecycle)
            .unwrap()
            .digest;
        drop(writer);

        let reopened = reopen_promoted_bootstrap_anchor(&root.app(), &binding).unwrap();
        assert_eq!(reopened.committed().enrollment_head(), head);
        assert_eq!(reopened.bootstrap_part_count(), 0);
        assert_eq!(reopened.accepted_history_record_count(), 0);
        assert_eq!(reopened.acceptance_sequence(), 0);
        assert_eq!(
            reopened.history_root(),
            super::super::object_store::EngineHistoryStore::empty_root()
        );
        let (predecessor, _) = reopened.into_predecessor_evidence();
        assert_eq!(predecessor.enrollment_head(), verified_digest);
        assert_eq!(predecessor.bootstrap_batch_id(), None);
    }

    /// The anchor grows every `LocalActive` record, and
    /// [`MAX_ENROLLMENT_JSON_TOKENS`] is a hard fail-closed decode bound: a
    /// record that exceeds it is unreadable, which would be the same class of
    /// permanent unopenability the anchor exists to remove. Pin the budget so a
    /// later field addition is a deliberate decision.
    #[test]
    fn the_largest_local_active_record_stays_inside_the_decode_budget() {
        fn json_tokens(bytes: &[u8]) -> usize {
            let (mut in_string, mut escaped, mut tokens) = (false, false, 0_usize);
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
                    b'{' | b'[' | b'}' | b']' | b',' | b':' => tokens += 1,
                    _ => {}
                }
            }
            tokens
        }

        let root = TestRoot::new("anchor-record-budget");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        let mut current = writer
            .transition(verified_digest, unsafe_idle(verified_digest, 2_000))
            .unwrap()
            .digest;
        // The widest LocalActive record: a Published exclusion carrying a
        // recovery packet, at a generation that also requires a checkpoint.
        while writer.current().generation() < 64 {
            current = writer
                .transition(
                    current,
                    if writer.current().generation() % 2 == 0 {
                        safe_idle(verified_digest)
                    } else {
                        unsafe_idle(
                            verified_digest,
                            2_000 + u128::from(writer.current().generation()),
                        )
                    },
                )
                .unwrap()
                .digest;
        }
        let session = match writer.current().record.lifecycle() {
            EnrollmentLifecycleV1::LocalActive(LocalActiveV1 {
                handoff: HandoffV1::Unsafe { session_id },
                ..
            }) => *session_id,
            other => panic!("unexpected lifecycle {other:?}"),
        };
        drop(writer);
        let head = publish_local_active_for_test(
            &root.app(),
            &binding,
            current,
            verified().verification_digest().unwrap(),
            session,
        )
        .unwrap();

        let bytes = fs::read(record_path(&root, &binding, head)).unwrap();
        let record = decode_record(&bytes).unwrap();
        assert_eq!(record.generation(), 65);
        assert!(record.checkpoint.is_some());
        assert!(matches!(
            record.lifecycle(),
            EnrollmentLifecycleV1::LocalActive(active)
                if matches!(active.exclusion, LocalExclusionV1::Published { .. })
        ));
        let tokens = json_tokens(&bytes);
        assert!(
            tokens <= MAX_ENROLLMENT_JSON_TOKENS,
            "the widest LocalActive record uses {tokens} of {MAX_ENROLLMENT_JSON_TOKENS} tokens"
        );
        assert_eq!(
            tokens, 224,
            "the LocalActive record token budget moved; confirm it still fits \
             {MAX_ENROLLMENT_JSON_TOKENS}"
        );
        assert!(bytes.len() < MAX_ENROLLMENT_RECORD_BYTES);
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
            EnrollmentReader::open_existing(&root.app(), &binding).unwrap(),
            EnrollmentOpen::Absent
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

    #[cfg(unix)]
    #[test]
    fn unlink_replacement_cannot_create_a_second_process_writer() {
        let root = TestRoot::new("lease-replacement");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let lease = enrollment_directory(&root, &binding).join(LEASE_FILE);
        fs::remove_file(&lease).unwrap();
        fs::write(&lease, b"replacement").unwrap();

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "oplog::enrollment::tests::lease_replacement_child_probe",
                "--nocapture",
            ])
            .env("TINE_ENROLLMENT_REPLACED_LEASE_CHILD_ROOT", &root.path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "replacement child acquired a second writer: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let current = writer.current().digest;
        assert!(
            writer
                .transition(current, EnrollmentLifecycleV1::VerifiedLocal(verified()))
                .is_err(),
            "the writer holding the unlinked lease published"
        );
    }

    #[test]
    fn lease_replacement_child_probe() {
        let Some(path) = std::env::var_os("TINE_ENROLLMENT_REPLACED_LEASE_CHILD_ROOT") else {
            return;
        };
        let app = EnrollmentApplicationRoot::open_for_harness(Path::new(&path)).unwrap();
        assert!(
            EnrollmentWriter::open_existing(&app, &test_binding()).is_err(),
            "replacement lease opened as independent authority"
        );
    }

    #[test]
    fn archive_claim_is_create_new_exact_and_detects_replacement_and_id_reuse() {
        let root = TestRoot::new("archive-identity");
        let first_path = root.path.join("archive-a");
        let second_path = root.path.join("archive-b");
        fs::create_dir_all(&first_path).unwrap();
        fs::create_dir_all(&second_path).unwrap();
        let first = Dir::open_ambient_dir(&first_path, ambient_authority()).unwrap();
        let reopened = Dir::open_ambient_dir(&first_path, ambient_authority()).unwrap();
        let second = Dir::open_ambient_dir(&second_path, ambient_authority()).unwrap();

        let first_id = CanonicalArchiveResourceId::provision_in_retained_directory(&first).unwrap();
        assert_eq!(
            first_id,
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&reopened, first_id)
                .unwrap()
        );
        assert_eq!(
            CanonicalArchiveResourceId::provision_in_retained_directory(&reopened)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );
        let second_id =
            CanonicalArchiveResourceId::provision_in_retained_directory(&second).unwrap();
        assert_ne!(first_id, second_id);
        assert!(
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&second, first_id)
                .is_err()
        );

        let copied_path = root.path.join("archive-copy");
        fs::create_dir_all(&copied_path).unwrap();
        fs::copy(
            first_path.join("archive-instance-v1.claim"),
            copied_path.join("archive-instance-v1.claim"),
        )
        .unwrap();
        let copied = Dir::open_ambient_dir(&copied_path, ambient_authority()).unwrap();
        assert!(
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&copied, first_id)
                .is_err(),
            "a copied exact claim must not authenticate a substituted directory"
        );

        fs::remove_file(first_path.join("archive-instance-v1.claim")).unwrap();
        let replacement =
            CanonicalArchiveResourceId::provision_in_retained_directory(&reopened).unwrap();
        assert_ne!(replacement, first_id);
        assert!(
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&reopened, first_id)
                .is_err()
        );

        let missing_path = root.path.join("archive-missing");
        fs::create_dir_all(&missing_path).unwrap();
        let missing = Dir::open_ambient_dir(&missing_path, ambient_authority()).unwrap();
        assert_eq!(
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&missing, first_id)
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert!(!missing_path.join("archive-instance-v1.claim").exists());

        let incompatible_bytes =
            br#"{"schema_version":2,"instance_id":"00000000-0000-0000-0000-000000000001"}"#;
        fs::write(
            first_path.join("archive-instance-v1.claim"),
            incompatible_bytes,
        )
        .unwrap();
        assert!(
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&reopened, replacement)
                .is_err()
        );
        assert_eq!(
            fs::read(first_path.join("archive-instance-v1.claim")).unwrap(),
            incompatible_bytes
        );

        let reused_os_identity_a =
            CanonicalArchiveResourceId::from_test_claim_and_capability_identity(
                b"injected-os",
                b"same-reused-id",
                b"claim-a",
            );
        let reused_os_identity_b =
            CanonicalArchiveResourceId::from_test_claim_and_capability_identity(
                b"injected-os",
                b"same-reused-id",
                b"claim-b",
            );
        assert_ne!(reused_os_identity_a, reused_os_identity_b);

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

    #[cfg(unix)]
    #[test]
    fn archive_claim_reparse_is_rejected_and_preserved() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("archive-claim-reparse");
        let archive_path = root.path.join("archive");
        fs::create_dir_all(&archive_path).unwrap();
        let archive = Dir::open_ambient_dir(&archive_path, ambient_authority()).unwrap();
        let expected = CanonicalArchiveResourceId::from_capability_identity(b"test", b"expected");
        symlink("/dev/null", archive_path.join("archive-instance-v1.claim")).unwrap();
        assert!(
            CanonicalArchiveResourceId::open_enrolled_in_retained_directory(&archive, expected)
                .is_err()
        );
        assert!(
            fs::symlink_metadata(archive_path.join("archive-instance-v1.claim"))
                .unwrap()
                .file_type()
                .is_symlink()
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

        let wrong_archive = published(digest(62), 60, archive_resource(61));
        let record = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 2,
            previous: Some(digest(62)),
            history_accumulator: digest(63),
            lease_resource_id: test_lease_resource(),
            binding: test_binding(),
            lifecycle: wrong_archive,
            checkpoint: None,
        };
        assert!(matches!(
            record.validate(),
            Err(EnrollmentError::BindingMismatch(
                EnrollmentBindingField::ArchiveResource
            ))
        ));

        let initial_binding = test_binding();
        let initial_authority = test_authority(initial_binding.clone(), test_lease_resource());
        let canonical = canonical_record_bytes(
            &EnrollmentRecordV1::initial(
                initial_binding,
                shadow(),
                test_lease_resource(),
                &initial_authority,
            )
            .unwrap(),
        )
        .unwrap();
        for state in ["future_shared_active", "future_joining", "future_active"] {
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

    #[test]
    fn stranded_exact_initial_record_is_resumed_idempotently() {
        let root = TestRoot::new("stranded-initial");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        drop(writer);
        fs::remove_file(enrollment_directory(&root, &binding).join(HEAD_FILE)).unwrap();

        let resumed = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        assert_eq!(resumed.current().digest, initial);
        drop(resumed);
        assert_eq!(
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap())
                .current()
                .digest,
            initial
        );
    }

    #[test]
    fn stranded_initial_recovery_preserves_foreign_and_ambiguous_records() {
        let root = TestRoot::new("stranded-initial-foreign");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        fs::remove_file(enrollment_directory(&root, &binding).join(HEAD_FILE)).unwrap();
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        let foreign = records.join(format!("{}{RECORD_SUFFIX}", digest(201)));
        fs::write(&foreign, b"foreign canonical-name bytes").unwrap();

        assert_eq!(
            EnrollmentWriter::create(&root.app(), binding.clone(), shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousInitialCreation
        );
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign canonical-name bytes");

        let suspicious_root = TestRoot::new("stranded-initial-suspicious-temp");
        let writer =
            EnrollmentWriter::create(&suspicious_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let enrollment = enrollment_directory(&suspicious_root, &binding);
        fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
        let suspicious = enrollment.join(format!("{HEAD_TEMP_PREFIX}suspicious"));
        fs::write(&suspicious, b"not a crash head").unwrap();
        assert_eq!(
            EnrollmentWriter::create(&suspicious_root.app(), binding, shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousInitialCreation
        );
        assert_eq!(fs::read(&suspicious).unwrap(), b"not a crash head");
    }

    #[test]
    fn authority_temp_provisioning_resumes_only_one_unambiguous_claim() {
        let root = TestRoot::new("authority-provision-resume");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        drop(writer);
        let enrollment = enrollment_directory(&root, &binding);
        fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
        fs::remove_file(record_path(&root, &binding, initial)).unwrap();
        let authority_temp = enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}resumable"));
        fs::rename(enrollment.join(AUTHORITY_FILE), &authority_temp).unwrap();

        let resumed = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        assert_eq!(resumed.current().generation(), 1);
        assert!(!authority_temp.exists());
        drop(resumed);

        let ambiguous_root = TestRoot::new("authority-provision-ambiguous");
        let writer =
            EnrollmentWriter::create(&ambiguous_root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        drop(writer);
        let enrollment = enrollment_directory(&ambiguous_root, &binding);
        fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
        fs::remove_file(record_path(&ambiguous_root, &binding, initial)).unwrap();
        let first = enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}first"));
        let second = enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}second"));
        fs::rename(enrollment.join(AUTHORITY_FILE), &first).unwrap();
        fs::copy(&first, &second).unwrap();
        assert_eq!(
            EnrollmentWriter::create(&ambiguous_root.app(), binding, shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousAuthorityProvisioning
        );
        assert!(first.exists());
        assert!(second.exists());

        let mismatched_root = TestRoot::new("authority-provision-mismatched-preparation");
        let writer =
            EnrollmentWriter::create(&mismatched_root.app(), test_binding(), shadow()).unwrap();
        let initial = writer.current().digest;
        drop(writer);
        let enrollment = enrollment_directory(&mismatched_root, &test_binding());
        fs::remove_file(enrollment.join(HEAD_FILE)).unwrap();
        fs::remove_file(record_path(&mismatched_root, &test_binding(), initial)).unwrap();
        let mismatched = ShadowImportV1::new(PreparationId::new(), digest(204));
        assert_eq!(
            EnrollmentWriter::create(&mismatched_root.app(), test_binding(), mismatched)
                .err()
                .unwrap(),
            EnrollmentError::InitialPreparationMismatch
        );
        assert!(enrollment.join(AUTHORITY_FILE).exists());
    }

    #[test]
    fn installed_authority_temp_recovery_rejects_copied_empty_and_multiple_temps() {
        let binding = test_binding();

        let copied_root = TestRoot::new("authority-installed-copied-temp");
        let writer =
            EnrollmentWriter::create(&copied_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let copied_enrollment = enrollment_directory(&copied_root, &binding);
        let copied_authority = copied_enrollment.join(AUTHORITY_FILE);
        let copied_temp = copied_enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}copied"));
        let authority_bytes = fs::read(&copied_authority).unwrap();
        fs::copy(&copied_authority, &copied_temp).unwrap();
        assert_eq!(
            EnrollmentWriter::create(&copied_root.app(), binding.clone(), shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousAuthorityProvisioning
        );
        assert_eq!(fs::read(&copied_authority).unwrap(), authority_bytes);
        assert_eq!(fs::read(&copied_temp).unwrap(), authority_bytes);

        let empty_root = TestRoot::new("authority-installed-empty-temp");
        let writer =
            EnrollmentWriter::create(&empty_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let empty_enrollment = enrollment_directory(&empty_root, &binding);
        let empty_temp = empty_enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}empty"));
        fs::write(&empty_temp, b"").unwrap();
        assert_eq!(
            EnrollmentWriter::create(&empty_root.app(), binding.clone(), shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousAuthorityProvisioning
        );
        assert_eq!(fs::read(&empty_temp).unwrap(), b"");

        let multiple_root = TestRoot::new("authority-installed-multiple-temps");
        let writer =
            EnrollmentWriter::create(&multiple_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let multiple_enrollment = enrollment_directory(&multiple_root, &binding);
        let multiple_authority = multiple_enrollment.join(AUTHORITY_FILE);
        let first_temp = multiple_enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}first"));
        let second_temp = multiple_enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}second"));
        fs::copy(&multiple_authority, &first_temp).unwrap();
        fs::copy(&multiple_authority, &second_temp).unwrap();
        assert_eq!(
            EnrollmentWriter::create(&multiple_root.app(), binding, shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousAuthorityProvisioning
        );
        assert!(first_temp.exists());
        assert!(second_temp.exists());
    }

    #[test]
    fn installed_authority_temp_recovery_rejects_foreign_link_state() {
        let root = TestRoot::new("authority-installed-foreign-link");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let enrollment = enrollment_directory(&root, &binding);
        let authority = enrollment.join(AUTHORITY_FILE);
        let temp = enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}link-gap"));
        let foreign = root.path.join("foreign-authority-link");
        fs::hard_link(&authority, &temp).unwrap();
        fs::hard_link(&authority, &foreign).unwrap();

        assert_eq!(
            EnrollmentWriter::create(&root.app(), binding, shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousAuthorityProvisioning
        );
        assert!(temp.exists());
        assert!(foreign.exists());
    }

    #[test]
    fn installed_authority_temp_recovers_only_the_platform_link_unlink_gap() {
        let root = TestRoot::new("authority-installed-link-gap");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let enrollment = enrollment_directory(&root, &binding);
        let authority = enrollment.join(AUTHORITY_FILE);
        let temp = enrollment.join(format!("{AUTHORITY_TEMP_PREFIX}link-gap"));
        fs::hard_link(&authority, &temp).unwrap();

        #[cfg(windows)]
        {
            let resumed = EnrollmentWriter::create(&root.app(), binding, shadow()).unwrap();
            assert_eq!(resumed.current().generation(), 1);
            assert!(!temp.exists());
        }
        #[cfg(not(windows))]
        {
            assert_eq!(
                EnrollmentWriter::create(&root.app(), binding, shadow())
                    .err()
                    .unwrap(),
                EnrollmentError::AmbiguousAuthorityProvisioning
            );
            assert!(temp.exists());
        }
    }

    #[test]
    fn missing_substituted_and_incompatible_authority_fail_closed_and_are_preserved() {
        let binding = test_binding();

        let missing_root = TestRoot::new("authority-missing");
        let writer =
            EnrollmentWriter::create(&missing_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let missing = enrollment_directory(&missing_root, &binding).join(AUTHORITY_FILE);
        fs::remove_file(&missing).unwrap();
        assert!(EnrollmentReader::open_existing(&missing_root.app(), &binding).is_err());
        assert!(!missing.exists());

        let substituted_root = TestRoot::new("authority-substituted");
        let writer =
            EnrollmentWriter::create(&substituted_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let substituted = enrollment_directory(&substituted_root, &binding).join(AUTHORITY_FILE);
        let mut claim: EnrollmentAuthorityClaimV2 =
            serde_json::from_slice(&fs::read(&substituted).unwrap()).unwrap();
        claim.authority_id = Uuid::from_u128(0xfeed);
        let bytes =
            canonical_authority_claim_bytes(&EnrollmentAuthorityClaim::CurrentV2(claim)).unwrap();
        fs::remove_file(&substituted).unwrap();
        fs::write(&substituted, &bytes).unwrap();
        let substituted_error = EnrollmentReader::open_existing(&substituted_root.app(), &binding)
            .err()
            .expect("a substituted current authority must fail closed");
        assert!(
            matches!(
                substituted_error,
                EnrollmentError::AuthorityMismatch | EnrollmentError::CheckpointIntegrityFailed
            ),
            "unexpected substituted-authority error: {substituted_error:?}"
        );
        assert_eq!(fs::read(&substituted).unwrap(), bytes);

        let incompatible_root = TestRoot::new("authority-incompatible");
        let writer =
            EnrollmentWriter::create(&incompatible_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let incompatible = enrollment_directory(&incompatible_root, &binding).join(AUTHORITY_FILE);
        let mut incompatible_claim: EnrollmentAuthorityClaimV2 =
            serde_json::from_slice(&fs::read(&incompatible).unwrap()).unwrap();
        incompatible_claim.schema_version = ENROLLMENT_AUTHORITY_SCHEMA_VERSION + 1;
        let incompatible_bytes = serde_json::to_vec(&incompatible_claim).unwrap();
        fs::write(&incompatible, &incompatible_bytes).unwrap();
        assert_eq!(
            EnrollmentReader::open_existing(&incompatible_root.app(), &binding)
                .err()
                .unwrap(),
            EnrollmentError::UnsupportedAuthoritySchema(ENROLLMENT_AUTHORITY_SCHEMA_VERSION + 1)
        );
        assert_eq!(fs::read(&incompatible).unwrap(), incompatible_bytes);
    }

    #[test]
    fn illegal_transition_immediately_beyond_open_window_fails_closed() {
        let root = TestRoot::new("illegal-beyond-open-window");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().clone();
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        let illegal_lifecycle = unsafe_idle(initial.digest, 900);
        let illegal_record = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 2,
            previous: Some(initial.digest),
            history_accumulator: compute_history_accumulator(
                2,
                Some(initial.digest),
                Some(initial.record.history_accumulator),
                &binding,
                &illegal_lifecycle,
            )
            .unwrap(),
            lease_resource_id: initial.record.lease_resource_id,
            binding: binding.clone(),
            lifecycle: illegal_lifecycle,
            checkpoint: None,
        };
        let bytes = canonical_record_bytes(&illegal_record).unwrap();
        let digest = ContentDigest::of(&bytes);
        fs::write(records.join(format!("{digest}{RECORD_SUFFIX}")), bytes).unwrap();
        let mut snapshot = EnrollmentSnapshot {
            digest,
            record: illegal_record,
        };
        for index in 0..MAX_ENROLLMENT_OPEN_CHAIN_RECORDS {
            let lifecycle = if index % 2 == 0 {
                safe_idle(initial.digest)
            } else {
                unsafe_idle(initial.digest, 901 + index as u128)
            };
            let forged_authority =
                test_authority(binding.clone(), initial.record.lease_resource_id);
            let record =
                EnrollmentRecordV1::successor(&snapshot, lifecycle, &forged_authority).unwrap();
            let bytes = canonical_record_bytes(&record).unwrap();
            let digest = ContentDigest::of(&bytes);
            fs::write(records.join(format!("{digest}{RECORD_SUFFIX}")), bytes).unwrap();
            snapshot = EnrollmentSnapshot { digest, record };
        }
        drop(writer);
        write_head(&root, &binding, snapshot.digest);

        assert!(EnrollmentReader::open_existing(&root.app(), &binding).is_err());
    }

    #[test]
    fn forged_checkpoint_immediately_at_open_boundary_fails_authentication() {
        let root = TestRoot::new("forged-checkpoint-boundary");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_digest = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        let mut current = writer
            .transition(verified_digest, unsafe_idle(verified_digest, 1_200))
            .unwrap()
            .digest;
        while writer.current().generation() < 65 {
            let next = if writer.current().generation() % 2 == 1 {
                safe_idle(verified_digest)
            } else {
                unsafe_idle(
                    verified_digest,
                    1_200 + u128::from(writer.current().generation()),
                )
            };
            current = writer.transition(current, next).unwrap().digest;
        }
        let mut forged = writer.current().record.clone();
        let EnrollmentCheckpoint::CurrentV3(checkpoint) = forged.checkpoint.as_mut().unwrap()
        else {
            panic!("fresh records use the v3 integrity checkpoint");
        };
        checkpoint.integrity_tag ^= 1;
        let bytes = canonical_record_bytes(&forged).unwrap();
        let forged_digest = ContentDigest::of(&bytes);
        fs::write(record_path(&root, &binding, forged_digest), bytes).unwrap();
        drop(writer);
        write_head(&root, &binding, forged_digest);

        assert_eq!(
            EnrollmentReader::open_existing(&root.app(), &binding)
                .err()
                .unwrap(),
            EnrollmentError::CheckpointIntegrityFailed
        );
    }

    #[test]
    fn cycle_and_nonmonotonic_claims_beyond_window_cannot_mint_a_checkpoint() {
        for cycle in [false, true] {
            let root = TestRoot::new(if cycle {
                "cycle-beyond-window"
            } else {
                "nonmonotonic-beyond-window"
            });
            let binding = test_binding();
            let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
            let initial = writer.current().clone();
            let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
            let previous = if cycle { digest(203) } else { initial.digest };
            let lifecycle = unsafe_idle(previous, 1_300);
            let forged_authority =
                test_authority(binding.clone(), initial.record.lease_resource_id);
            let mut checkpoint_record = EnrollmentRecordV1 {
                schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
                generation: 65,
                previous: Some(previous),
                history_accumulator: compute_history_accumulator(
                    65,
                    Some(previous),
                    Some(initial.record.history_accumulator),
                    &binding,
                    &lifecycle,
                )
                .unwrap(),
                lease_resource_id: initial.record.lease_resource_id,
                binding: binding.clone(),
                lifecycle,
                checkpoint: None,
            };
            checkpoint_record.checkpoint = Some(
                forged_authority
                    .checkpoint_for(
                        checkpoint_record.generation,
                        checkpoint_record.previous,
                        checkpoint_record.history_accumulator,
                        checkpoint_record.lease_resource_id,
                        &checkpoint_record.binding,
                        &checkpoint_record.lifecycle,
                    )
                    .unwrap(),
            );
            let bytes = canonical_record_bytes(&checkpoint_record).unwrap();
            let digest = ContentDigest::of(&bytes);
            fs::write(records.join(format!("{digest}{RECORD_SUFFIX}")), bytes).unwrap();
            let mut snapshot = EnrollmentSnapshot {
                digest,
                record: checkpoint_record,
            };
            for index in 0..63_u128 {
                let lifecycle = if index % 2 == 0 {
                    safe_idle(previous)
                } else {
                    unsafe_idle(previous, 1_301 + index)
                };
                let record =
                    EnrollmentRecordV1::successor(&snapshot, lifecycle, &forged_authority).unwrap();
                let bytes = canonical_record_bytes(&record).unwrap();
                let digest = ContentDigest::of(&bytes);
                fs::write(records.join(format!("{digest}{RECORD_SUFFIX}")), bytes).unwrap();
                snapshot = EnrollmentSnapshot { digest, record };
            }
            drop(writer);
            write_head(&root, &binding, snapshot.digest);
            assert!(
                EnrollmentReader::open_existing(&root.app(), &binding).is_err(),
                "a forged {} summary crossed the bounded-open checkpoint",
                if cycle { "cycle" } else { "nonmonotonic" }
            );
        }
    }

    #[test]
    fn audit_limit_one_validates_the_transition_across_page_boundary() {
        let root = TestRoot::new("audit-boundary");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_snapshot = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .clone();
        let illegal_lifecycle = EnrollmentLifecycleV1::VerifiedLocal(verified());
        let illegal_record = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 3,
            previous: Some(verified_snapshot.digest),
            history_accumulator: compute_history_accumulator(
                3,
                Some(verified_snapshot.digest),
                Some(verified_snapshot.record.history_accumulator),
                &binding,
                &illegal_lifecycle,
            )
            .unwrap(),
            lease_resource_id: verified_snapshot.record.lease_resource_id,
            binding,
            lifecycle: illegal_lifecycle,
            checkpoint: None,
        };
        let bytes = canonical_record_bytes(&illegal_record).unwrap();
        let digest = ContentDigest::of(&bytes);
        fs::write(
            writer
                .reader
                .directories
                .display_path
                .join(RECORDS_DIRECTORY)
                .join(format!("{digest}{RECORD_SUFFIX}")),
            bytes,
        )
        .unwrap();
        writer.reader.current = EnrollmentSnapshot {
            digest,
            record: illegal_record,
        };

        let first = writer.audit_chain_page(None, 1).unwrap();
        assert_eq!(
            writer.audit_chain_page(first.next, 1).err().unwrap(),
            EnrollmentError::IllegalTransition
        );
    }

    #[test]
    fn audit_limit_sixty_four_validates_the_transition_across_page_boundary() {
        let root = TestRoot::new("audit-boundary-64");
        let binding = test_binding();
        let mut writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        let verified_snapshot = writer
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .clone();
        let illegal_lifecycle = EnrollmentLifecycleV1::VerifiedLocal(verified());
        let illegal_record = EnrollmentRecordV1 {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            generation: 3,
            previous: Some(verified_snapshot.digest),
            history_accumulator: compute_history_accumulator(
                3,
                Some(verified_snapshot.digest),
                Some(verified_snapshot.record.history_accumulator),
                &binding,
                &illegal_lifecycle,
            )
            .unwrap(),
            lease_resource_id: verified_snapshot.record.lease_resource_id,
            binding: binding.clone(),
            lifecycle: illegal_lifecycle,
            checkpoint: None,
        };
        let bytes = canonical_record_bytes(&illegal_record).unwrap();
        let digest = ContentDigest::of(&bytes);
        fs::write(
            writer
                .reader
                .directories
                .display_path
                .join(RECORDS_DIRECTORY)
                .join(format!("{digest}{RECORD_SUFFIX}")),
            bytes,
        )
        .unwrap();
        let mut snapshot = EnrollmentSnapshot {
            digest,
            record: illegal_record,
        };
        for index in 0..63_u128 {
            let lifecycle = match index {
                0 => unsafe_idle(digest, 1_000),
                _ if index % 2 == 1 => safe_idle(digest),
                _ => unsafe_idle(digest, 1_000 + index),
            };
            let record = EnrollmentRecordV1::successor(
                &snapshot,
                lifecycle,
                &writer.reader.authority.material,
            )
            .unwrap();
            let bytes = canonical_record_bytes(&record).unwrap();
            let digest = ContentDigest::of(&bytes);
            fs::write(
                writer
                    .reader
                    .directories
                    .display_path
                    .join(RECORDS_DIRECTORY)
                    .join(format!("{digest}{RECORD_SUFFIX}")),
                bytes,
            )
            .unwrap();
            snapshot = EnrollmentSnapshot { digest, record };
        }
        assert_eq!(snapshot.generation(), 66);
        writer.reader.current = snapshot;

        let first = writer
            .audit_chain_page(None, MAX_ENROLLMENT_AUDIT_PAGE)
            .unwrap();
        assert_eq!(first.records.len(), MAX_ENROLLMENT_AUDIT_PAGE);
        assert_eq!(
            writer
                .audit_chain_page(first.next, MAX_ENROLLMENT_AUDIT_PAGE)
                .err()
                .unwrap(),
            EnrollmentError::IllegalTransition
        );
    }

    #[test]
    fn audit_cursor_rejects_wrong_schema_tag_message_stale_and_foreign_state() {
        let first_root = TestRoot::new("audit-cursor-first");
        let binding = test_binding();
        let mut first =
            EnrollmentWriter::create(&first_root.app(), binding.clone(), shadow()).unwrap();
        let initial = first.current().digest;
        let verified_digest = first
            .transition(initial, EnrollmentLifecycleV1::VerifiedLocal(verified()))
            .unwrap()
            .digest;
        first
            .transition(verified_digest, unsafe_idle(verified_digest, 1_100))
            .unwrap();
        let cursor = first.audit_chain_page(None, 1).unwrap().next.unwrap();

        let mut wrong_tag = cursor;
        wrong_tag.integrity_tag ^= 1;
        assert_eq!(
            first.audit_chain_page(Some(wrong_tag), 1).err().unwrap(),
            EnrollmentError::InvalidAuditCursor
        );

        let mut wrong_schema = cursor;
        wrong_schema.schema_version = 2;
        assert_eq!(
            first.audit_chain_page(Some(wrong_schema), 1).err().unwrap(),
            EnrollmentError::InvalidAuditCursor
        );

        let mut wrong_message = cursor;
        wrong_message.generation = wrong_message.generation.saturating_sub(1);
        assert_eq!(
            first
                .audit_chain_page(Some(wrong_message), 1)
                .err()
                .unwrap(),
            EnrollmentError::InvalidAuditCursor
        );

        let current = first.current().digest;
        first
            .transition(current, safe_idle(verified_digest))
            .unwrap();
        assert_eq!(
            first.audit_chain_page(Some(cursor), 1).err().unwrap(),
            EnrollmentError::InvalidAuditCursor
        );

        let second_root = TestRoot::new("audit-cursor-second");
        let mut second =
            EnrollmentWriter::create(&second_root.app(), binding.clone(), shadow()).unwrap();
        let second_initial = second.current().digest;
        let second_verified = second
            .transition(
                second_initial,
                EnrollmentLifecycleV1::VerifiedLocal(verified()),
            )
            .unwrap()
            .digest;
        second
            .transition(second_verified, unsafe_idle(second_verified, 1_100))
            .unwrap();
        assert_eq!(
            second.audit_chain_page(Some(cursor), 1).err().unwrap(),
            EnrollmentError::InvalidAuditCursor
        );
    }

    #[test]
    fn arbitrary_root_constructor_is_compiled_only_for_tests() {
        let source = include_str!("enrollment.rs");
        assert!(
            source.contains("#[cfg(test)]\n    fn open_for_harness"),
            "the arbitrary path constructor must not exist in production"
        );
    }

    #[test]
    fn enrollment_keyed_auth_source_guard_isolated_to_legacy_verification_and_simulator_formats() {
        fn visit(directory: &Path, files: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    visit(&path, files);
                } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/oplog");
        let mut files = Vec::new();
        visit(&root, &mut files);
        let mut keyed = Vec::new();
        for path in files {
            let source = fs::read_to_string(&path).unwrap();
            let production = source.split("#[cfg(test)]\nmod tests").next().unwrap();
            if production.contains("hmac::") || production.contains("hmac_") {
                keyed.push(path);
            }
        }
        keyed.sort();
        let names = keyed
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["enrollment_legacy_hmac.rs", "simulator.rs"],
            "keyed enrollment compatibility must stay isolated; simulator is a deterministic test format"
        );
        let legacy = fs::read_to_string(root.join("enrollment_legacy_hmac.rs")).unwrap();
        let legacy_production = legacy.split("#[cfg(test)]").next().unwrap();
        assert_eq!(legacy_production.matches("hmac::verify(").count(), 1);
        assert_eq!(legacy_production.matches("hmac::sign(").count(), 0);
        assert_eq!(legacy.matches("hmac::sign(").count(), 1);
        let current = fs::read_to_string(root.join("enrollment.rs")).unwrap();
        let current = current.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert!(!current.contains("hmac::"));
        assert!(!current.contains("ring::rand"));
        assert!(!current.contains(" key:"));
    }

    #[cfg(unix)]
    #[test]
    fn exact_record_link_gap_is_recovered_by_same_lease_retry() {
        let root = TestRoot::new("record-link-gap");
        let binding = test_binding();
        let writer = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        drop(writer);
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        let target = records.join(format!("{initial}{RECORD_SUFFIX}"));
        let temp = records.join(format!("{RECORD_TEMP_PREFIX}link-gap"));
        fs::hard_link(&target, &temp).unwrap();

        let resumed = EnrollmentWriter::create(&root.app(), binding.clone(), shadow()).unwrap();
        assert_eq!(resumed.current().digest, initial);
        assert!(!temp.exists());
        drop(resumed);

        let ambiguous_root = TestRoot::new("record-link-gap-ambiguous");
        let writer =
            EnrollmentWriter::create(&ambiguous_root.app(), binding.clone(), shadow()).unwrap();
        let initial = writer.current().digest;
        drop(writer);
        let records = enrollment_directory(&ambiguous_root, &binding).join(RECORDS_DIRECTORY);
        let target = records.join(format!("{initial}{RECORD_SUFFIX}"));
        let foreign = records.join("foreign-hardlink");
        fs::hard_link(&target, &foreign).unwrap();
        assert_eq!(
            EnrollmentWriter::create(&ambiguous_root.app(), binding, shadow())
                .err()
                .unwrap(),
            EnrollmentError::AmbiguousInitialCreation
        );
        assert!(target.exists());
        assert!(foreign.exists());
    }

    // -----------------------------------------------------------------------
    // Retained enrollment session: bounded per-mutation admission.
    // -----------------------------------------------------------------------

    /// The exact session the `handoffs`-long journal built by
    /// [`local_active_journal`] ends on.
    fn journal_terminal_session(handoffs: u64) -> SessionId {
        assert!(handoffs % 2 == 0 && handoffs > 0);
        SessionId::from_uuid(Uuid::from_u128(u128::from(900 + handoffs)))
    }

    fn open_retained_session(
        root: &TestRoot,
        binding: &EnrollmentBindingV1,
    ) -> RetainedEnrollmentSession {
        RetainedEnrollmentSession::open(
            &root.app(),
            binding,
            verified().verification_digest().unwrap(),
        )
        .unwrap()
    }

    /// Per-admission enrollment cost is constant and independent of how long
    /// the journal is.
    ///
    /// The three journal lengths are the packet's 1 / 1,000 / 10,000
    /// post-bootstrap scale points. At every one of them an unchanged-head
    /// admission must perform exactly one bounded read of the fixed-size head
    /// file and nothing else: no namespace enumeration, no lease
    /// reacquisition, no authority-claim reread, no directory-tree open, and
    /// no record-chain walk.
    ///
    /// Fail-before is executable and in the same test: the parent
    /// per-admission call, `reopen_committed_local_active_for_session`, is
    /// measured over the identical journal and must violate every one of those
    /// bounds. If the assertions could not discriminate, that control would
    /// pass too.
    #[test]
    fn retained_session_admissions_are_constant_cost_at_one_thousand_and_ten_thousand_records() {
        const ADMISSIONS: usize = 64;
        let mut measured = Vec::new();
        for handoffs in [2_u64, 1_000, 10_000] {
            let root = TestRoot::new(&format!("retained-session-{handoffs}"));
            let binding = test_binding();
            let (_verified_digest, head) = local_active_journal(&root, &binding, handoffs);

            let mut session = open_retained_session(&root, &binding);
            assert_eq!(session.committed().enrollment_head(), head);
            assert_eq!(
                session.committed().handoff(),
                LocalActiveHandoff::Unsafe {
                    session_id: journal_terminal_session(handoffs)
                }
            );

            let before = EnrollmentInstrumentation::capture();
            for _ in 0..ADMISSIONS {
                assert_eq!(session.revalidate().unwrap().enrollment_head(), head);
            }
            let admission = before.since();
            assert_eq!(
                session.full_revalidations(),
                0,
                "an unchanged head must never force a full authenticated reopen"
            );
            assert_eq!(
                admission,
                EnrollmentInstrumentation {
                    record_reads: 0,
                    head_reads: ADMISSIONS,
                    namespace_scans: 0,
                    directory_opens: 0,
                    lease_acquisitions: 0,
                    authority_claim_reads: 0,
                },
                "unchanged-head admissions at journal length {handoffs} were not bounded"
            );

            // Fail-before control: the parent per-admission enrollment call,
            // measured over this exact journal.
            let before = EnrollmentInstrumentation::capture();
            for _ in 0..ADMISSIONS {
                reopen_committed_local_active_for_session(
                    &root.app(),
                    &binding,
                    verified().verification_digest().unwrap(),
                )
                .unwrap();
            }
            let parent = before.since();
            assert_eq!(parent.directory_opens, ADMISSIONS);
            assert_eq!(parent.namespace_scans, ADMISSIONS);
            assert!(parent.authority_claim_reads >= ADMISSIONS);
            assert!(
                parent.record_reads >= ADMISSIONS,
                "the parent admission must walk the record chain, not just the head"
            );
            measured.push((handoffs, admission, parent));
        }

        // Constant, and constant across three orders of magnitude of journal.
        let (_, first_admission, _) = measured[0];
        for (handoffs, admission, parent) in &measured {
            assert_eq!(
                *admission, first_admission,
                "admission cost changed at journal length {handoffs}"
            );
            assert!(
                parent.record_reads > admission.record_reads
                    && parent.namespace_scans > admission.namespace_scans
                    && parent.directory_opens > admission.directory_opens,
                "the parent shape must fail the new bound at journal length {handoffs}"
            );
        }
    }

    /// The cheap check is exact, not approximate: any observed head change
    /// forces the complete authenticated reopen, and a head that is missing,
    /// malformed, or points at a non-`LocalActive` record rejects instead.
    #[test]
    fn a_changed_missing_or_divergent_head_forces_full_revalidation_or_rejects() {
        let root = TestRoot::new("retained-session-change");
        let binding = test_binding();
        let (verified_digest, head) = local_active_journal(&root, &binding, 4);
        let terminal = journal_terminal_session(4);

        // A genuine change: the session itself moves the record. A handoff is a
        // lifecycle boundary, so it is bracketed by full reauthentications.
        let mut session = open_retained_session(&root, &binding);
        assert_eq!(session.full_revalidations(), 0);
        let generation = session.binding_generation();
        let safe = session
            .transition_handoff(LocalActiveHandoff::Safe)
            .unwrap()
            .enrollment_head();
        assert_ne!(safe, head);
        assert!(session.full_revalidations() >= 2);
        assert!(session.binding_generation() > generation);
        assert_eq!(session.committed().handoff(), LocalActiveHandoff::Safe);
        // Repeating the same target is idempotent and does not advance a head.
        assert_eq!(
            session
                .transition_handoff(LocalActiveHandoff::Safe)
                .unwrap()
                .enrollment_head(),
            safe
        );
        let restored = session
            .transition_handoff(LocalActiveHandoff::Unsafe {
                session_id: terminal,
            })
            .unwrap()
            .enrollment_head();
        drop(session);

        // An externally moved head. The retained session holds the exclusive
        // lease, so this is a deliberately hostile raw write, not a legal
        // transition; the point is that the cheap check cannot miss it.
        let mut session = open_retained_session(&root, &binding);
        let before = session.full_revalidations();
        write_head(&root, &binding, head);
        assert_eq!(session.revalidate().unwrap().enrollment_head(), head);
        assert_eq!(
            session.full_revalidations(),
            before + 1,
            "a moved head must force exactly one full authenticated reopen"
        );
        // ...and settles back to cheap checks at the new head.
        let counted = EnrollmentInstrumentation::capture();
        session.revalidate().unwrap();
        assert_eq!(counted.since().record_reads, 0);
        assert_eq!(session.full_revalidations(), before + 1);
        write_head(&root, &binding, restored);
        session.revalidate().unwrap();
        drop(session);

        // Missing, malformed, and divergent heads all fail closed.
        let head_path = enrollment_directory(&root, &binding).join(HEAD_FILE);
        let committed_head_bytes = fs::read(&head_path).unwrap();
        for (label, mutate) in [
            (
                "missing",
                Box::new(|path: &Path| fs::remove_file(path).unwrap()) as Box<dyn Fn(&Path)>,
            ),
            (
                "truncated",
                Box::new(|path: &Path| fs::write(path, b"").unwrap()),
            ),
            (
                "malformed",
                Box::new(|path: &Path| fs::write(path, vec![b'z'; HEAD_BYTES]).unwrap()),
            ),
        ] {
            let mut session = open_retained_session(&root, &binding);
            mutate(&head_path);
            assert!(
                session.revalidate().is_err(),
                "a {label} head must never admit work"
            );
            drop(session);
            fs::write(&head_path, &committed_head_bytes).unwrap();
        }

        // A head that names a real, authenticated, but non-`LocalActive`
        // record is divergent, not adoptable.
        let mut session = open_retained_session(&root, &binding);
        write_head(&root, &binding, verified_digest);
        match session.revalidate() {
            Err(VerifiedLocalCompositionError::WrongLifecycle(_)) => {}
            Err(error) => panic!("a divergent head must fail closed: {error}"),
            Ok(_) => panic!("a divergent head must never be adopted"),
        }
        drop(session);
        fs::write(&head_path, &committed_head_bytes).unwrap();

        // The journal survived every refusal exactly.
        let reader =
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap());
        assert_eq!(reader.current().digest, restored);
    }

    /// Retaining the session takes the exclusive journal lease exactly once;
    /// dropping it releases the contention exactly.
    #[test]
    fn a_second_live_session_cannot_write_the_journal_and_dropping_one_releases_it() {
        let root = TestRoot::new("retained-session-lease");
        let binding = test_binding();
        let (_verified_digest, head) = local_active_journal(&root, &binding, 2);
        let terminal = journal_terminal_session(2);

        let before = EnrollmentInstrumentation::capture();
        let mut session = open_retained_session(&root, &binding);
        assert_eq!(
            before.since().lease_acquisitions,
            1,
            "a retained session acquires the lease exactly once"
        );

        // No second writer of any shape may exist while this session is live.
        assert!(matches!(
            EnrollmentWriter::open_existing(&root.app(), &binding),
            Err(EnrollmentError::LeaseContended(_))
        ));
        assert!(matches!(
            RetainedEnrollmentSession::open(
                &root.app(),
                &binding,
                verified().verification_digest().unwrap()
            ),
            Err(VerifiedLocalCompositionError::Enrollment(
                EnrollmentError::LeaseContended(_)
            ))
        ));
        assert!(matches!(
            transition_local_active_handoff(
                &root.app(),
                &binding,
                head,
                verified().verification_digest().unwrap(),
                LocalActiveHandoff::Safe,
            ),
            Err(VerifiedLocalCompositionError::Enrollment(
                EnrollmentError::LeaseContended(_)
            ))
        ));
        // Readers never contend, so audit and anchor reopen stay available.
        assert_eq!(
            expect_present(EnrollmentReader::open_existing(&root.app(), &binding).unwrap())
                .current()
                .digest,
            head
        );
        assert_eq!(
            reopen_promoted_bootstrap_anchor(&root.app(), &binding)
                .unwrap()
                .committed()
                .enrollment_head(),
            head
        );

        // The retained session is the one writer that can still move the
        // journal, and it does so without reacquiring anything.
        let before = EnrollmentInstrumentation::capture();
        let advanced = session
            .transition_handoff(LocalActiveHandoff::Safe)
            .unwrap()
            .enrollment_head();
        assert_eq!(before.since().lease_acquisitions, 0);
        assert_ne!(advanced, head);

        drop(session);
        // Contention is released exactly, with no residue.
        let mut resumed = open_retained_session(&root, &binding);
        assert_eq!(resumed.committed().enrollment_head(), advanced);
        assert_eq!(
            resumed
                .transition_handoff(LocalActiveHandoff::Unsafe {
                    session_id: terminal
                })
                .unwrap()
                .handoff(),
            LocalActiveHandoff::Unsafe {
                session_id: terminal
            }
        );
    }

    /// A retained session refuses every lifecycle a runtime authority may not
    /// be admitted under, and refuses a foreign verification digest.
    #[test]
    fn a_retained_session_refuses_blocked_published_and_foreign_enrollments() {
        let root = TestRoot::new("retained-session-lifecycle");
        let binding = test_binding();
        let (_verified_digest, head) = local_active_journal(&root, &binding, 2);

        // A foreign verification digest can never retain a session.
        assert!(matches!(
            RetainedEnrollmentSession::open(&root.app(), &binding, digest(200)),
            Err(VerifiedLocalCompositionError::ProofMismatch(_))
        ));

        // A published (non-`Idle`) record is refused.
        let published = publish_local_active_for_test(
            &root.app(),
            &binding,
            head,
            verified().verification_digest().unwrap(),
            journal_terminal_session(2),
        )
        .unwrap();
        assert!(matches!(
            RetainedEnrollmentSession::open(
                &root.app(),
                &binding,
                verified().verification_digest().unwrap()
            ),
            Err(VerifiedLocalCompositionError::WrongLifecycle(_))
        ));

        // So is a blocked one.
        block_current_for_test(&root.app(), &binding, published, "retained.test".into()).unwrap();
        assert!(matches!(
            RetainedEnrollmentSession::open(
                &root.app(),
                &binding,
                verified().verification_digest().unwrap()
            ),
            Err(VerifiedLocalCompositionError::WrongLifecycle(_))
        ));
    }

    /// Contract/fail-before: before P3.1 these two proven LocalActive journals
    /// had no legal shared path at all.  The smallest added path must retain
    /// the existing local proofs, keep the graph bytes untouched, and reach
    /// exactly one descriptor-bound shared lineage on both devices.
    #[test]
    fn shared_enrollment_clean_two_device_join_is_descriptor_bound_and_inactive() {
        fn token_count(bytes: &[u8]) -> usize {
            let (mut in_string, mut escaped, mut tokens) = (false, false, 0_usize);
            for byte in bytes.iter().copied() {
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        in_string = false;
                    }
                } else {
                    match byte {
                        b'"' => in_string = true,
                        b'{' | b'[' | b'}' | b']' | b',' | b':' => tokens += 1,
                        _ => {}
                    }
                }
            }
            tokens
        }

        let initiator = TestRoot::new("shared-clean-initiator");
        let joiner = TestRoot::new("shared-clean-joiner");
        let initiator_binding = shared_test_binding(0x801, 31, 41);
        let joiner_binding = shared_test_binding(0x802, 32, 42);
        local_active_safe_for_shared_test(&initiator, initiator_binding.clone());
        local_active_safe_for_shared_test(&joiner, joiner_binding.clone());

        let projection = initiator.path.join("projection-bytes.md");
        fs::write(&projection, b"- must not be touched by core enrollment\n").unwrap();
        let projection_before = fs::read(&projection).unwrap();

        let descriptor =
            prepare_shared_enrollment(&initiator.app(), &initiator_binding, digest(0xa1)).unwrap();
        assert_eq!(
            inspect_shared_enrollment_phase(&initiator.app(), &initiator_binding).unwrap(),
            Some(SharedEnrollmentPhase::SharePrepared)
        );
        activate_shared_initiator(&initiator.app(), &initiator_binding, &descriptor).unwrap();
        prepare_shared_join(&joiner.app(), &joiner_binding, &descriptor, digest(0xa2), 0).unwrap();
        assert_eq!(
            inspect_shared_enrollment_phase(&joiner.app(), &joiner_binding).unwrap(),
            Some(SharedEnrollmentPhase::Joining)
        );
        activate_shared_joiner(&joiner.app(), &joiner_binding, &descriptor).unwrap();

        assert_eq!(
            inspect_shared_enrollment_phase(&initiator.app(), &initiator_binding).unwrap(),
            Some(SharedEnrollmentPhase::SharedActiveInitiator)
        );
        assert_eq!(
            inspect_shared_enrollment_phase(&joiner.app(), &joiner_binding).unwrap(),
            Some(SharedEnrollmentPhase::SharedActiveJoiner)
        );
        assert_eq!(fs::read(&projection).unwrap(), projection_before);
        for (root, binding) in [(&initiator, &initiator_binding), (&joiner, &joiner_binding)] {
            let reader =
                expect_present(EnrollmentReader::open_existing(&root.app(), binding).unwrap());
            let bytes = fs::read(record_path(root, binding, reader.current().digest)).unwrap();
            assert!(bytes.len() < MAX_ENROLLMENT_RECORD_BYTES);
            assert!(
                token_count(&bytes) <= MAX_ENROLLMENT_JSON_TOKENS,
                "shared record exceeded its fixed bounded-open token budget"
            );
        }

        // The existing runtime discovery boundary must fail closed: P3.1 does
        // not silently grant SharedActive a LocalActive runtime or UI path.
        assert_shared_runtime_discoverable(&initiator, &initiator_binding);
        assert_shared_runtime_discoverable(&joiner, &joiner_binding);
    }

    #[test]
    fn shared_join_reordering_and_duplicate_descriptor_are_idempotent() {
        let initiator = TestRoot::new("shared-duplicate-initiator");
        let joiner = TestRoot::new("shared-duplicate-joiner");
        let initiator_binding = shared_test_binding(0x811, 33, 43);
        let joiner_binding = shared_test_binding(0x812, 34, 44);
        local_active_safe_for_shared_test(&initiator, initiator_binding.clone());
        local_active_safe_for_shared_test(&joiner, joiner_binding.clone());
        let descriptor =
            prepare_shared_enrollment(&initiator.app(), &initiator_binding, digest(0xb1)).unwrap();

        // Delivery may be reordered.  Completing an absent join is refused
        // without advancing the peer or touching projection bytes.
        assert!(activate_shared_joiner(&joiner.app(), &joiner_binding, &descriptor).is_err());
        assert_eq!(
            inspect_shared_enrollment_phase(&joiner.app(), &joiner_binding).unwrap(),
            None
        );
        prepare_shared_join(&joiner.app(), &joiner_binding, &descriptor, digest(0xb2), 0).unwrap();
        // Duplicate provider delivery repeats no state transition.
        prepare_shared_join(&joiner.app(), &joiner_binding, &descriptor, digest(0xb2), 0).unwrap();
        activate_shared_joiner(&joiner.app(), &joiner_binding, &descriptor).unwrap();
        activate_shared_joiner(&joiner.app(), &joiner_binding, &descriptor).unwrap();
        assert_eq!(
            inspect_shared_enrollment_phase(&joiner.app(), &joiner_binding).unwrap(),
            Some(SharedEnrollmentPhase::SharedActiveJoiner)
        );
    }

    #[test]
    fn shared_join_blocks_partial_bootstrap_split_genesis_dirty_tail_and_base_mismatch() {
        let initiator = TestRoot::new("shared-refusal-initiator");
        let initiator_binding = shared_test_binding(0x821, 35, 45);
        local_active_safe_for_shared_test(&initiator, initiator_binding.clone());
        let descriptor =
            prepare_shared_enrollment(&initiator.app(), &initiator_binding, digest(0xc1)).unwrap();

        let partial = TestRoot::new("shared-refusal-partial");
        let partial_binding = shared_test_binding(0x822, 36, 46);
        local_active_safe_for_shared_test(&partial, partial_binding.clone());
        let mut incomplete = descriptor.clone();
        incomplete.projection_base.bootstrap_part_count = 1;
        incomplete.projection_base.bootstrap_terminal_part_id = None;
        assert!(prepare_shared_join(
            &partial.app(),
            &partial_binding,
            &incomplete,
            digest(0xc2),
            0,
        )
        .is_err());
        assert_shared_blocked(&partial, &partial_binding);

        let dirty = TestRoot::new("shared-refusal-dirty-tail");
        let dirty_binding = shared_test_binding(0x823, 37, 47);
        local_active_safe_for_shared_test(&dirty, dirty_binding.clone());
        let bytes = dirty.path.join("local-projection.md");
        fs::write(&bytes, b"- untouched dirty-tail projection\n").unwrap();
        let before = fs::read(&bytes).unwrap();
        assert!(
            prepare_shared_join(&dirty.app(), &dirty_binding, &descriptor, digest(0xc3), 1,)
                .is_err()
        );
        assert_eq!(fs::read(&bytes).unwrap(), before);
        assert_shared_blocked(&dirty, &dirty_binding);

        let conflict = TestRoot::new("shared-refusal-conflicting-descriptor");
        let conflict_binding = shared_test_binding(0x824, 38, 48);
        local_active_safe_for_shared_test(&conflict, conflict_binding.clone());
        prepare_shared_join(
            &conflict.app(),
            &conflict_binding,
            &descriptor,
            digest(0xc4),
            0,
        )
        .unwrap();
        let mut competing = descriptor.clone();
        competing.object_store_namespace = digest(0xc5);
        assert!(prepare_shared_join(
            &conflict.app(),
            &conflict_binding,
            &competing,
            digest(0xc4),
            0,
        )
        .is_err());
        assert_shared_blocked(&conflict, &conflict_binding);

        let base_mismatch = TestRoot::new("shared-refusal-base-mismatch");
        let base_mismatch_binding = shared_test_binding(0x825, 39, 49);
        local_active_safe_for_shared_test(&base_mismatch, base_mismatch_binding.clone());
        let mut incompatible = descriptor.clone();
        incompatible.projection_base.bootstrap_import_id = digest(0xc6);
        assert!(prepare_shared_join(
            &base_mismatch.app(),
            &base_mismatch_binding,
            &incompatible,
            digest(0xc7),
            0,
        )
        .is_err());
        assert_shared_blocked(&base_mismatch, &base_mismatch_binding);

        let incompatible = TestRoot::new("shared-refusal-incompatible-version");
        let incompatible_binding = shared_test_binding(0x826, 40, 50);
        local_active_safe_for_shared_test(&incompatible, incompatible_binding.clone());
        let mut future_protocol = descriptor.clone();
        future_protocol.compatibility.oplog_protocol_version = future_protocol
            .compatibility
            .oplog_protocol_version
            .saturating_add(1);
        assert!(prepare_shared_join(
            &incompatible.app(),
            &incompatible_binding,
            &future_protocol,
            digest(0xc8),
            0,
        )
        .is_err());
        assert_shared_blocked(&incompatible, &incompatible_binding);
    }

    #[test]
    fn shared_enrollment_named_crash_cuts_resume_two_device_state_machine() {
        let cuts = [
            CommitCut::AfterRecordTempCreate,
            CommitCut::AfterRecordWrite,
            CommitCut::AfterRecordFileSync,
            CommitCut::AfterRecordLink,
            CommitCut::AfterRecordInsert,
            CommitCut::AfterRecordsDirectorySync,
            CommitCut::AfterHeadTempCreate,
            CommitCut::AfterHeadWrite,
            CommitCut::AfterHeadFileSync,
            CommitCut::AfterHeadReplace,
            CommitCut::AfterEnrollmentDirectorySync,
        ];

        for (index, cut) in cuts.into_iter().enumerate() {
            let initiator = TestRoot::new(&format!("shared-crash-initiator-{index}"));
            let joiner = TestRoot::new(&format!("shared-crash-joiner-{index}"));
            let initiator_binding = shared_test_binding(0x900 + index as u128, 50, 60);
            let joiner_binding = shared_test_binding(0xa00 + index as u128, 51, 61);
            local_active_safe_for_shared_test(&initiator, initiator_binding.clone());
            local_active_safe_for_shared_test(&joiner, joiner_binding.clone());

            assert!(matches!(
                prepare_shared_enrollment_at_cut_for_test(
                    &initiator.app(),
                    &initiator_binding,
                    digest(0xd1),
                    cut,
                ),
                Err(EnrollmentError::InjectedCrashCut(_))
            ));
            let descriptor =
                prepare_shared_enrollment(&initiator.app(), &initiator_binding, digest(0xd1))
                    .unwrap();

            assert!(matches!(
                activate_shared_initiator_at_cut_for_test(
                    &initiator.app(),
                    &initiator_binding,
                    &descriptor,
                    cut,
                ),
                Err(EnrollmentError::InjectedCrashCut(_))
            ));
            activate_shared_initiator(&initiator.app(), &initiator_binding, &descriptor).unwrap();

            assert!(matches!(
                prepare_shared_join_at_cut_for_test(
                    &joiner.app(),
                    &joiner_binding,
                    &descriptor,
                    digest(0xd2),
                    0,
                    cut,
                ),
                Err(EnrollmentError::InjectedCrashCut(_))
            ));
            prepare_shared_join(&joiner.app(), &joiner_binding, &descriptor, digest(0xd2), 0)
                .unwrap();

            assert!(matches!(
                activate_shared_joiner_at_cut_for_test(
                    &joiner.app(),
                    &joiner_binding,
                    &descriptor,
                    cut,
                ),
                Err(EnrollmentError::InjectedCrashCut(_))
            ));
            activate_shared_joiner(&joiner.app(), &joiner_binding, &descriptor).unwrap();
            assert_eq!(
                inspect_shared_enrollment_phase(&initiator.app(), &initiator_binding).unwrap(),
                Some(SharedEnrollmentPhase::SharedActiveInitiator)
            );
            assert_eq!(
                inspect_shared_enrollment_phase(&joiner.app(), &joiner_binding).unwrap(),
                Some(SharedEnrollmentPhase::SharedActiveJoiner)
            );
        }
    }
}
