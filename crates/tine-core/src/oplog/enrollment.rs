//! Authoritative device-local enrollment lifecycle journal.
//!
//! Enrollment records only persisted state. It deliberately exposes no graph
//! writer or projection authorization. Content addressing, retained
//! capabilities, no-follow opens, exact file identities, and an OS lease
//! reject corruption, accidental substitution, and cooperating-process
//! split-brain. A private enrollment-authority key authenticates bounded
//! immutable history checkpoints, so arbitrary record bytes cannot summarize
//! an unvalidated prefix. The key is protected only by the private application
//! directory: a process with the same user authority that can read the key and
//! rewrite the complete store remains outside this boundary. Filesystem and
//! directory-sync guarantees also remain platform/filesystem dependent.
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
use fs2::FileExt as _;
use ring::rand::SecureRandom as _;
use ring::{hmac, rand as ring_rand};
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
use super::object_store::{ensure_directory_nofollow, open_dir_nofollow, sync_dir_required};
use super::{
    BatchId, CanonicalArchiveResourceId, CanonicalGraphResourceId, ContentDigest, DeviceId,
    DocumentId, GraphTextScopeBinding, ImportId, LineageDigest, ProjectionEndpointId,
    ProjectionReceiptStoreId, SessionId, WorkspaceId, DIFF_SCHEMA_VERSION,
    MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION,
    OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION, PROJECTION_POLICY_VERSION,
    PROJECTION_SCHEMA_VERSION, RECEIPT_SCHEMA_VERSION,
};

pub(crate) const ENROLLMENT_RECORD_SCHEMA_VERSION: u32 = 3;
pub(crate) const PUBLISHED_RECOVERY_PACKET_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_ENROLLMENT_RECORD_BYTES: usize = 32 * 1024;
pub(crate) const MAX_ENROLLMENT_JSON_DEPTH: usize = 16;
pub(crate) const MAX_ENROLLMENT_JSON_TOKENS: usize = 256;
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
const ENROLLMENT_AUTHORITY_SCHEMA_VERSION: u32 = 1;
const ENROLLMENT_CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const MAX_ENROLLMENT_AUTHORITY_BYTES: usize = 4 * 1024;
const ENROLLMENT_AUTHORITY_KEY_BYTES: usize = 32;

#[cfg(test)]
thread_local! {
    static ENROLLMENT_RECORD_READS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentAuthorityClaimV1 {
    schema_version: u32,
    authority_id: Uuid,
    lease_resource_id: ContentDigest,
    binding: EnrollmentBindingV1,
    initial_preparation_id: PreparationId,
    initial_source_inventory_digest: ContentDigest,
    key: [u8; ENROLLMENT_AUTHORITY_KEY_BYTES],
}

impl EnrollmentAuthorityClaimV1 {
    fn validate_initial_intent(&self, shadow: &ShadowImportV1) -> Result<(), EnrollmentError> {
        if self.initial_preparation_id != shadow.preparation_id
            || self.initial_source_inventory_digest != shadow.source_inventory_digest
        {
            return Err(EnrollmentError::InitialPreparationMismatch);
        }
        Ok(())
    }
}

struct EnrollmentAuthorityMaterial {
    claim: EnrollmentAuthorityClaimV1,
    resource_id: ContentDigest,
    key: hmac::Key,
}

impl EnrollmentAuthorityMaterial {
    fn from_claim(
        claim: EnrollmentAuthorityClaimV1,
        resource_id: ContentDigest,
        expected_binding: &EnrollmentBindingV1,
        expected_lease_resource_id: ContentDigest,
    ) -> Result<Self, EnrollmentError> {
        if claim.schema_version != ENROLLMENT_AUTHORITY_SCHEMA_VERSION {
            return Err(EnrollmentError::UnsupportedAuthoritySchema(
                claim.schema_version,
            ));
        }
        claim.binding.validate_exact(expected_binding)?;
        if claim.lease_resource_id != expected_lease_resource_id {
            return Err(EnrollmentError::LeaseResourceMismatch);
        }
        let key = hmac::Key::new(hmac::HMAC_SHA256, &claim.key);
        Ok(Self {
            claim,
            resource_id,
            key,
        })
    }

    fn checkpoint_for(
        &self,
        generation: u64,
        previous: Option<ContentDigest>,
        history_accumulator: ContentDigest,
        lease_resource_id: ContentDigest,
        binding: &EnrollmentBindingV1,
        lifecycle: &EnrollmentLifecycleV1,
    ) -> Result<AuthenticatedCheckpointV1, EnrollmentError> {
        let message = checkpoint_message_bytes(
            self.claim.authority_id,
            self.resource_id,
            generation,
            previous,
            history_accumulator,
            lease_resource_id,
            binding,
            lifecycle,
        )?;
        Ok(AuthenticatedCheckpointV1 {
            schema_version: ENROLLMENT_CHECKPOINT_SCHEMA_VERSION,
            authority_id: self.claim.authority_id,
            authority_resource_id: self.resource_id,
            authentication_tag: ContentDigest::from_bytes(
                hmac::sign(&self.key, &message)
                    .as_ref()
                    .try_into()
                    .expect("SHA-256 tag"),
            ),
        })
    }

    fn verify_checkpoint(&self, record: &EnrollmentRecordV1) -> Result<(), EnrollmentError> {
        let checkpoint = record
            .checkpoint
            .as_ref()
            .ok_or(EnrollmentError::MissingAuthenticatedCheckpoint)?;
        if checkpoint.schema_version != ENROLLMENT_CHECKPOINT_SCHEMA_VERSION {
            return Err(EnrollmentError::UnsupportedCheckpointSchema(
                checkpoint.schema_version,
            ));
        }
        if checkpoint.authority_id != self.claim.authority_id
            || checkpoint.authority_resource_id != self.resource_id
        {
            return Err(EnrollmentError::AuthorityMismatch);
        }
        let message = checkpoint_message_bytes(
            checkpoint.authority_id,
            checkpoint.authority_resource_id,
            record.generation,
            record.previous,
            record.history_accumulator,
            record.lease_resource_id,
            &record.binding,
            &record.lifecycle,
        )?;
        hmac::verify(
            &self.key,
            &message,
            checkpoint.authentication_tag.as_bytes(),
        )
        .map_err(|_| EnrollmentError::CheckpointAuthenticationFailed)
    }

    fn audit_cursor_tag(
        &self,
        head: ContentDigest,
        digest: ContentDigest,
        generation: u64,
        newer_digest: ContentDigest,
    ) -> ContentDigest {
        let message = audit_cursor_message_bytes(
            self.claim.authority_id,
            self.resource_id,
            head,
            digest,
            generation,
            newer_digest,
        );
        ContentDigest::from_bytes(
            hmac::sign(&self.key, &message)
                .as_ref()
                .try_into()
                .expect("SHA-256 tag"),
        )
    }

    fn verify_audit_cursor(&self, cursor: &EnrollmentAuditCursor) -> Result<(), EnrollmentError> {
        let message = audit_cursor_message_bytes(
            self.claim.authority_id,
            self.resource_id,
            cursor.head,
            cursor.digest,
            cursor.generation,
            cursor.newer_digest,
        );
        hmac::verify(&self.key, &message, cursor.authentication_tag.as_bytes())
            .map_err(|_| EnrollmentError::InvalidAuditCursor)
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
    let mut message = Vec::with_capacity(32 * 4 + 8 + 48);
    message.extend_from_slice(b"tine/enrollment-audit-cursor/v1\0");
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
    backup_manifest_digest: ContentDigest,
    backup_restore_proof_digest: ContentDigest,
    bootstrap_batch_id: BatchId,
    accepted_frontier_anchor: AcceptedFrontierAnchorV1,
    staged_projection_manifest_digest: ContentDigest,
    byte_compare_digest: ContentDigest,
}

impl VerifiedLocalV1 {
    fn verification_digest(&self) -> Result<ContentDigest, EnrollmentError> {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalActiveV1 {
    verification_digest: ContentDigest,
    handoff: HandoffV1,
    exclusion: LocalExclusionV1,
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
struct AuthenticatedCheckpointV1 {
    schema_version: u32,
    authority_id: Uuid,
    authority_resource_id: ContentDigest,
    authentication_tag: ContentDigest,
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

fn checkpoint_message_bytes(
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
        domain: "tine/enrollment-checkpoint/v1",
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRecordV1 {
    schema_version: u32,
    generation: u64,
    previous: Option<ContentDigest>,
    history_accumulator: ContentDigest,
    lease_resource_id: ContentDigest,
    binding: EnrollmentBindingV1,
    lifecycle: EnrollmentLifecycleV1,
    checkpoint: Option<AuthenticatedCheckpointV1>,
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
        if self.schema_version != ENROLLMENT_RECORD_SCHEMA_VERSION {
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
    hasher.update(b"tine/enrollment-history-accumulator/v1\0");
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
            verified
                .verification_digest()
                .is_ok_and(|digest| digest == active.verification_digest)
                && matches!(active.handoff, HandoffV1::Unsafe { .. })
                && matches!(active.exclusion, LocalExclusionV1::Idle)
        }
        (EnrollmentLifecycleV1::VerifiedLocal(_), EnrollmentLifecycleV1::Blocked(blocked)) => {
            blocked.prior_record_digest == current_digest
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
                    head: self.current.digest,
                    digest,
                    generation: expected_generation,
                    newer_digest,
                    authentication_tag: self.authority.material.audit_cursor_tag(
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
    head: ContentDigest,
    digest: ContentDigest,
    generation: u64,
    newer_digest: ContentDigest,
    authentication_tag: ContentDigest,
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
        let record =
            EnrollmentRecordV1::initial(binding, shadow, lease.resource_id, &authority.material)?;
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
enum CommitCut {
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
    reject_duplicate_json_fields(bytes)?;
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

    let mut key = [0_u8; ENROLLMENT_AUTHORITY_KEY_BYTES];
    ring_rand::SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| EnrollmentError::AuthorityRandomness)?;
    let claim = EnrollmentAuthorityClaimV1 {
        schema_version: ENROLLMENT_AUTHORITY_SCHEMA_VERSION,
        authority_id: Uuid::new_v4(),
        lease_resource_id: lease.resource_id,
        binding: binding.clone(),
        initial_preparation_id: shadow.preparation_id,
        initial_source_inventory_digest: shadow.source_inventory_digest,
        key,
    };
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
    claim: &EnrollmentAuthorityClaimV1,
) -> Result<Vec<u8>, EnrollmentError> {
    let bytes =
        serde_json::to_vec(claim).map_err(|error| EnrollmentError::Encode(error.to_string()))?;
    if bytes.len() > MAX_ENROLLMENT_AUTHORITY_BYTES {
        return Err(EnrollmentError::AuthorityClaimTooLarge(bytes.len()));
    }
    Ok(bytes)
}

fn decode_authority_claim(bytes: &[u8]) -> Result<EnrollmentAuthorityClaimV1, EnrollmentError> {
    if bytes.len() > MAX_ENROLLMENT_AUTHORITY_BYTES {
        return Err(EnrollmentError::AuthorityClaimTooLarge(bytes.len()));
    }
    reject_duplicate_json_fields(bytes)?;
    let claim: EnrollmentAuthorityClaimV1 = serde_json::from_slice(bytes)
        .map_err(|error| EnrollmentError::Decode(error.to_string()))?;
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

#[cfg(not(any(unix, windows)))]
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
    AuthorityRandomness,
    AuthorityClaimTooLarge(usize),
    NonCanonicalAuthorityClaim,
    UnsupportedAuthoritySchema(u32),
    UnsupportedCheckpointSchema(u32),
    MissingAuthenticatedCheckpoint,
    CheckpointAuthenticationFailed,
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
            Self::AuthorityRandomness => {
                formatter.write_str("enrollment authority randomness was unavailable")
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
            Self::UnsupportedCheckpointSchema(schema) => {
                write!(
                    formatter,
                    "unsupported enrollment checkpoint schema {schema}"
                )
            }
            Self::MissingAuthenticatedCheckpoint => {
                formatter.write_str("enrollment history suffix has no authenticated checkpoint")
            }
            Self::CheckpointAuthenticationFailed => {
                formatter.write_str("enrollment checkpoint authentication failed")
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
            EnrollmentAuthorityClaimV1 {
                schema_version: ENROLLMENT_AUTHORITY_SCHEMA_VERSION,
                authority_id: Uuid::from_u128(25),
                lease_resource_id,
                binding: binding.clone(),
                initial_preparation_id: shadow().preparation_id,
                initial_source_inventory_digest: shadow().source_inventory_digest,
                key: [26; ENROLLMENT_AUTHORITY_KEY_BYTES],
            },
            digest(27),
            &binding,
            lease_resource_id,
        )
        .unwrap()
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
        let records = enrollment_directory(&root, &binding).join(RECORDS_DIRECTORY);
        for index in 0..MAX_ENROLLMENT_OPEN_CHAIN_RECORDS {
            let lifecycle = match index {
                0 => EnrollmentLifecycleV1::VerifiedLocal(verified()),
                1 => unsafe_idle(200),
                _ if index % 2 == 0 => safe_idle(),
                _ => unsafe_idle(200 + index as u128),
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
            .transition(verified_digest, unsafe_idle(700))
            .unwrap()
            .digest;
        for index in 0..2_049_u64 {
            let next = if index % 2 == 0 {
                safe_idle()
            } else {
                unsafe_idle(701 + u128::from(index))
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

        let wrong_archive = published(60, archive_resource(61));
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
        let mut claim: EnrollmentAuthorityClaimV1 =
            serde_json::from_slice(&fs::read(&substituted).unwrap()).unwrap();
        claim.key[0] ^= 1;
        let bytes = canonical_authority_claim_bytes(&claim).unwrap();
        fs::remove_file(&substituted).unwrap();
        fs::write(&substituted, &bytes).unwrap();
        assert!(matches!(
            EnrollmentReader::open_existing(&substituted_root.app(), &binding),
            Err(EnrollmentError::AuthorityMismatch
                | EnrollmentError::CheckpointAuthenticationFailed)
        ));
        assert_eq!(fs::read(&substituted).unwrap(), bytes);

        let incompatible_root = TestRoot::new("authority-incompatible");
        let writer =
            EnrollmentWriter::create(&incompatible_root.app(), binding.clone(), shadow()).unwrap();
        drop(writer);
        let incompatible = enrollment_directory(&incompatible_root, &binding).join(AUTHORITY_FILE);
        let mut claim: EnrollmentAuthorityClaimV1 =
            serde_json::from_slice(&fs::read(&incompatible).unwrap()).unwrap();
        claim.schema_version = ENROLLMENT_AUTHORITY_SCHEMA_VERSION + 1;
        let incompatible_bytes = canonical_authority_claim_bytes(&claim).unwrap();
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
        let illegal_lifecycle = unsafe_idle(900);
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
                safe_idle()
            } else {
                unsafe_idle(901 + index as u128)
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
            .transition(verified_digest, unsafe_idle(1_200))
            .unwrap()
            .digest;
        while writer.current().generation() < 65 {
            let next = if writer.current().generation() % 2 == 1 {
                safe_idle()
            } else {
                unsafe_idle(1_200 + u128::from(writer.current().generation()))
            };
            current = writer.transition(current, next).unwrap().digest;
        }
        let mut forged = writer.current().record.clone();
        forged.checkpoint.as_mut().unwrap().authentication_tag = digest(202);
        let bytes = canonical_record_bytes(&forged).unwrap();
        let forged_digest = ContentDigest::of(&bytes);
        fs::write(record_path(&root, &binding, forged_digest), bytes).unwrap();
        drop(writer);
        write_head(&root, &binding, forged_digest);

        assert_eq!(
            EnrollmentReader::open_existing(&root.app(), &binding)
                .err()
                .unwrap(),
            EnrollmentError::CheckpointAuthenticationFailed
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
            let lifecycle = unsafe_idle(1_300);
            let previous = if cycle { digest(203) } else { initial.digest };
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
                    safe_idle()
                } else {
                    unsafe_idle(1_301 + index)
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
                0 => unsafe_idle(1_000),
                _ if index % 2 == 1 => safe_idle(),
                _ => unsafe_idle(1_000 + index),
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
    fn audit_cursor_rejects_wrong_tag_key_message_stale_and_foreign_state() {
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
            .transition(verified_digest, unsafe_idle(1_100))
            .unwrap();
        let cursor = first.audit_chain_page(None, 1).unwrap().next.unwrap();

        let mut wrong_tag = cursor;
        wrong_tag.authentication_tag = ContentDigest::from_bytes([0; 32]);
        assert_eq!(
            first.audit_chain_page(Some(wrong_tag), 1).err().unwrap(),
            EnrollmentError::InvalidAuditCursor
        );

        let mut wrong_key = cursor;
        let message = audit_cursor_message_bytes(
            first.reader.authority.material.claim.authority_id,
            first.reader.authority.material.resource_id,
            wrong_key.head,
            wrong_key.digest,
            wrong_key.generation,
            wrong_key.newer_digest,
        );
        let forged_key = hmac::Key::new(hmac::HMAC_SHA256, &[0xa5; ENROLLMENT_AUTHORITY_KEY_BYTES]);
        wrong_key.authentication_tag = ContentDigest::from_bytes(
            hmac::sign(&forged_key, &message)
                .as_ref()
                .try_into()
                .expect("SHA-256 tag"),
        );
        assert_eq!(
            first.audit_chain_page(Some(wrong_key), 1).err().unwrap(),
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
        first.transition(current, safe_idle()).unwrap();
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
            .transition(second_verified, unsafe_idle(1_100))
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
}
