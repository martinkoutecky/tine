//! Durable, read-only backup proof for an inactive bootstrap source capture.
//!
//! This module owns no graph writer or enrollment capability. A returned
//! [`VerifiedSourceBackup`] can only be minted after exact staged publication,
//! fresh committed-byte readback, and a proof of the verified inventory.

use std::fmt;
use std::fs::{self, File, OpenOptions};
#[cfg(target_os = "android")]
use std::io::ErrorKind;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::bootstrap_import::{BOOTSTRAP_FRONTIER_SCHEMA_VERSION, BOOTSTRAP_IMPORT_SCHEMA_VERSION};
use super::import::{
    BootstrapStreamingImportError, InactiveBootstrapPreparedPublication,
    InactiveBootstrapVerifiedPublication,
};
use super::object_store::{
    control_directory_identity, open_dir_nofollow, open_file_nofollow,
    BootstrapAggregateHistoryBindingV1,
};
use super::sync_layout::{
    COMMIT_MARKER_FILE, COMMIT_MARKER_STAGE_FILE, MANIFEST_FILE,
    MIGRATION_BACKUP_ROOT_DIR as BACKUP_ROOT_DIRECTORY, MIGRATION_PAYLOAD_DIR as PAYLOAD_DIRECTORY,
    RESTORE_PROOF_FILE, RESTORE_PROOF_STAGE_FILE,
};
use super::{
    BlobDescription, CanonicalGraphResourceId, ContentDigest, ManagedPath, ManagedTextKind,
    WorkspaceId, DIFF_SCHEMA_VERSION, MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION,
    OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};
use crate::model::{
    canonical_graph_resource_id, move_file_noreplace, BootstrapSourceCapture,
    BootstrapSourceChunkCursor, BootstrapSourceEntry, BOOTSTRAP_SOURCE_CAPTURE_SCHEMA,
    BOOTSTRAP_SOURCE_CHUNK_BYTES, BOOTSTRAP_SOURCE_MAX_DIRECTORIES,
    BOOTSTRAP_SOURCE_MAX_DIRECTORY_DEPTH, BOOTSTRAP_SOURCE_MAX_FILES,
    BOOTSTRAP_SOURCE_MAX_FILE_BYTES, BOOTSTRAP_SOURCE_MAX_LOGICAL_NAME_BYTES,
    BOOTSTRAP_SOURCE_MAX_PATH_BYTES, BOOTSTRAP_SOURCE_MAX_TOTAL_BYTES,
};

const BACKUP_SCHEMA_VERSION: u32 = 1;
const RESTORE_PROOF_SCHEMA_VERSION: u32 = 1;
const COMMIT_MARKER_SCHEMA_VERSION: u32 = 1;
const MANIFEST_MAGIC: &[u8; 8] = b"TINEMB1\0";
const RESTORE_PROOF_MAGIC: &[u8; 8] = b"TINERP1\0";
const COMMIT_MARKER_MAGIC: &[u8; 8] = b"TINEMC1\0";
const IO_BUFFER_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_HEADER_BYTES: usize = 256 * 1024;
const MAX_RETAINED_EVIDENCE_BYTES: u64 = 1024 * 1024;
const SOURCE_BACKUP_BINDING_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
thread_local! {
    static MIGRATION_BACKUP_CRASH_CUT: std::cell::Cell<Option<MigrationBackupCrashCut>> =
        const { std::cell::Cell::new(None) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum MigrationBackupCrashCut {
    PartialPayloadWrite,
    PartialManifestHeaderWrite,
    PartialManifestEntryWrite,
    PartialRestoreWrite,
    PartialRestoreProofStageWrite,
    PartialCommitMarkerStageWrite,
    AfterStagingCreation,
    AfterPayloadWrite,
    AfterPayloadSync,
    AfterManifestPublication,
    AfterStagingSync,
    AfterStagingRename,
    AfterCommittedBackupReopen,
    AfterProofPublication,
    AfterCommitMarkerPublication,
}

fn inject_crash_cut(cut: MigrationBackupCrashCut) -> Result<(), MigrationBackupError> {
    if take_crash_cut(cut) {
        return Err(MigrationBackupError::InjectedCrashCut(cut.label()));
    }
    Ok(())
}

fn take_crash_cut(cut: MigrationBackupCrashCut) -> bool {
    #[cfg(test)]
    {
        return MIGRATION_BACKUP_CRASH_CUT
            .with(|pending| (pending.get() == Some(cut)).then(|| pending.set(None)))
            .is_some();
    }
    #[cfg(not(test))]
    {
        let _ = cut;
        false
    }
}

fn write_with_partial_crash_cut(
    output: &mut impl Write,
    bytes: &[u8],
    cut: MigrationBackupCrashCut,
) -> Result<(), MigrationBackupError> {
    if bytes.len() > 1 && take_crash_cut(cut) {
        let prefix = (bytes.len() / 2).clamp(1, bytes.len() - 1);
        output.write_all(&bytes[..prefix])?;
        output.flush()?;
        return Err(MigrationBackupError::InjectedCrashCut(cut.label()));
    }
    output.write_all(bytes)?;
    Ok(())
}

impl MigrationBackupCrashCut {
    const fn label(self) -> &'static str {
        match self {
            Self::PartialPayloadWrite => "partial_payload_write",
            Self::PartialManifestHeaderWrite => "partial_manifest_header_write",
            Self::PartialManifestEntryWrite => "partial_manifest_entry_write",
            Self::PartialRestoreWrite => "partial_restore_write",
            Self::PartialRestoreProofStageWrite => "partial_restore_proof_stage_write",
            Self::PartialCommitMarkerStageWrite => "partial_commit_marker_stage_write",
            Self::AfterStagingCreation => "after_staging_creation",
            Self::AfterPayloadWrite => "after_payload_write",
            Self::AfterPayloadSync => "after_payload_sync",
            Self::AfterManifestPublication => "after_manifest_publication",
            Self::AfterStagingSync => "after_staging_sync",
            Self::AfterStagingRename => "after_staging_rename",
            Self::AfterCommittedBackupReopen => "after_committed_backup_reopen",
            Self::AfterProofPublication => "after_proof_publication",
            Self::AfterCommitMarkerPublication => "after_commit_marker_publication",
        }
    }
}

#[derive(Debug)]
pub(crate) enum MigrationBackupError {
    Io(io::Error),
    InvalidRoot(&'static str),
    BindingMismatch(&'static str),
    CorruptOrConflicting(&'static str),
    ResourceLimit {
        resource: &'static str,
        observed: u64,
        limit: u64,
    },
    InjectedCrashCut(&'static str),
}

impl fmt::Display for MigrationBackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidRoot(detail)
            | Self::BindingMismatch(detail)
            | Self::CorruptOrConflicting(detail) => formatter.write_str(detail),
            Self::ResourceLimit {
                resource,
                observed,
                limit,
            } => write!(
                formatter,
                "{resource} limit exceeded: observed {observed}, limit {limit}"
            ),
            Self::InjectedCrashCut(label) => write!(formatter, "injected crash cut: {label}"),
        }
    }
}

impl std::error::Error for MigrationBackupError {}

impl From<io::Error> for MigrationBackupError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BootstrapStreamingImportError> for MigrationBackupError {
    fn from(error: BootstrapStreamingImportError) -> Self {
        Self::CorruptOrConflicting(match error {
            BootstrapStreamingImportError::ConflictingSeal => {
                "bootstrap preparation has a conflicting seal"
            }
            _ => "bootstrap preparation cannot be reopened",
        })
    }
}

/// Retained structural authority for a device-local backup root and the exact
/// canonical graph root it is forbidden to contain.
pub(crate) struct MigrationBackupRoot {
    canonical_root: PathBuf,
    canonical_graph_root: PathBuf,
    graph_resource: CanonicalGraphResourceId,
    backup_root_identity: ContentDigest,
    _root_capability: Dir,
    _graph_capability: Dir,
}

impl MigrationBackupRoot {
    pub(crate) fn open(
        device_local_backup_root: &Path,
        canonical_graph_root: &Path,
    ) -> Result<Self, MigrationBackupError> {
        require_real_directory(
            device_local_backup_root,
            "backup root is not a real directory",
        )?;
        require_real_directory(canonical_graph_root, "graph root is not a real directory")?;
        let canonical_root = fs::canonicalize(device_local_backup_root)?;
        let canonical_graph_root = fs::canonicalize(canonical_graph_root)?;
        if canonical_root == canonical_graph_root
            || canonical_root.starts_with(&canonical_graph_root)
            || canonical_graph_root.starts_with(&canonical_root)
        {
            return Err(MigrationBackupError::InvalidRoot(
                "backup and graph roots must be structurally disjoint",
            ));
        }
        let root_capability = open_directory_nofollow_ambient(&canonical_root)?;
        let graph_capability = open_directory_nofollow_ambient(&canonical_graph_root)?;
        let root_directory_identity = control_directory_identity(&root_capability)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        let graph_directory_identity = control_directory_identity(&graph_capability)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        if root_directory_identity == graph_directory_identity {
            return Err(MigrationBackupError::InvalidRoot(
                "backup and graph roots resolve to the same directory resource",
            ));
        }
        let graph_resource = canonical_graph_resource_id(&graph_capability)?;
        let backup_root_identity = root_directory_identity.migration_backup_root_binding_digest();
        Ok(Self {
            canonical_root,
            canonical_graph_root,
            graph_resource,
            backup_root_identity,
            _root_capability: root_capability,
            _graph_capability: graph_capability,
        })
    }

    #[cfg(test)]
    fn open_for_harness(
        device_local_backup_root: &Path,
        canonical_graph_root: &Path,
    ) -> Result<Self, MigrationBackupError> {
        Self::open(device_local_backup_root, canonical_graph_root)
    }

    pub(crate) fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub(crate) fn canonical_graph_root(&self) -> &Path {
        &self.canonical_graph_root
    }

    pub(crate) const fn graph_resource(&self) -> CanonicalGraphResourceId {
        self.graph_resource
    }

    pub(crate) const fn root_identity(&self) -> ContentDigest {
        self.backup_root_identity
    }

    pub(crate) fn freshly_validate_retained_roots(&self) -> Result<(), MigrationBackupError> {
        if self.canonical_root == self.canonical_graph_root
            || self.canonical_root.starts_with(&self.canonical_graph_root)
            || self.canonical_graph_root.starts_with(&self.canonical_root)
        {
            return Err(MigrationBackupError::InvalidRoot(
                "backup and graph roots are no longer structurally disjoint",
            ));
        }
        let retained_root = control_directory_identity(&self._root_capability)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        let retained_graph = control_directory_identity(&self._graph_capability)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        if retained_root == retained_graph
            || retained_root.migration_backup_root_binding_digest() != self.backup_root_identity
        {
            return Err(MigrationBackupError::InvalidRoot(
                "retained backup and graph directory resources alias or changed identity",
            ));
        }
        let rebound_root = open_directory_nofollow_ambient(&self.canonical_root)?;
        let rebound_graph = open_directory_nofollow_ambient(&self.canonical_graph_root)?;
        if control_directory_identity(&rebound_root)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?
            != retained_root
            || canonical_graph_resource_id(&rebound_graph)? != self.graph_resource
        {
            return Err(MigrationBackupError::InvalidRoot(
                "backup or graph root path no longer names its retained resource",
            ));
        }
        Ok(())
    }
}

/// Private evidence that can only be constructed by the final committed
/// readback and restore-verification path below.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedSourceBackup {
    directory: PathBuf,
    workspace_id: WorkspaceId,
    graph_resource: CanonicalGraphResourceId,
    backup_root_identity: ContentDigest,
    publication_id: [u8; 32],
    aggregate_digest: [u8; 32],
    source_inventory: BlobDescription,
    file_count: u64,
    total_bytes: u64,
    manifest: BlobDescription,
    restore_proof: BlobDescription,
    commit_marker: BlobDescription,
    evidence_digest: ContentDigest,
    instrumentation: MigrationBackupInstrumentation,
}

impl PartialEq for VerifiedSourceBackup {
    fn eq(&self, other: &Self) -> bool {
        self.workspace_id == other.workspace_id
            && self.graph_resource == other.graph_resource
            && self.backup_root_identity == other.backup_root_identity
            && self.publication_id == other.publication_id
            && self.aggregate_digest == other.aggregate_digest
            && self.source_inventory == other.source_inventory
            && self.file_count == other.file_count
            && self.total_bytes == other.total_bytes
            && self.manifest == other.manifest
            && self.restore_proof == other.restore_proof
            && self.commit_marker == other.commit_marker
            && self.evidence_digest == other.evidence_digest
    }
}

impl Eq for VerifiedSourceBackup {}

impl VerifiedSourceBackup {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn graph_resource(&self) -> CanonicalGraphResourceId {
        self.graph_resource
    }

    pub(crate) const fn backup_root_identity(&self) -> ContentDigest {
        self.backup_root_identity
    }

    pub(crate) const fn publication_id(&self) -> &[u8; 32] {
        &self.publication_id
    }

    pub(crate) const fn aggregate_digest(&self) -> &[u8; 32] {
        &self.aggregate_digest
    }

    pub(crate) const fn source_inventory(&self) -> BlobDescription {
        self.source_inventory
    }

    pub(crate) const fn file_count(&self) -> u64 {
        self.file_count
    }

    pub(crate) const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub(crate) const fn manifest(&self) -> BlobDescription {
        self.manifest
    }

    pub(crate) const fn restore_proof(&self) -> BlobDescription {
        self.restore_proof
    }

    pub(crate) const fn commit_marker(&self) -> BlobDescription {
        self.commit_marker
    }

    pub(crate) const fn evidence_digest(&self) -> ContentDigest {
        self.evidence_digest
    }

    pub(crate) const fn instrumentation(&self) -> &MigrationBackupInstrumentation {
        &self.instrumentation
    }

    pub(crate) fn retained_binding(&self) -> SourceBackupBindingV1 {
        SourceBackupBindingV1 {
            schema_version: SOURCE_BACKUP_BINDING_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            graph_resource: self.graph_resource,
            backup_root_identity: self.backup_root_identity,
            publication_id: ContentDigest::from_bytes(self.publication_id),
            aggregate_digest: ContentDigest::from_bytes(self.aggregate_digest),
            source_inventory: self.source_inventory,
            file_count: self.file_count,
            total_bytes: self.total_bytes,
            manifest: self.manifest,
            restore_proof: self.restore_proof,
            commit_marker: self.commit_marker,
            evidence_digest: self.evidence_digest,
        }
    }
}

/// Constant-size identity for the already verified migration backup that is
/// retained as both rollback authority and bootstrap baseline byte storage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceBackupBindingV1 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    graph_resource: CanonicalGraphResourceId,
    backup_root_identity: ContentDigest,
    publication_id: ContentDigest,
    aggregate_digest: ContentDigest,
    source_inventory: BlobDescription,
    file_count: u64,
    total_bytes: u64,
    manifest: BlobDescription,
    restore_proof: BlobDescription,
    commit_marker: BlobDescription,
    evidence_digest: ContentDigest,
}

impl SourceBackupBindingV1 {
    pub(crate) fn validate(&self) -> Result<(), MigrationBackupError> {
        if self.schema_version != SOURCE_BACKUP_BINDING_SCHEMA_VERSION
            || self.file_count > BOOTSTRAP_SOURCE_MAX_FILES
            || self.total_bytes > BOOTSTRAP_SOURCE_MAX_TOTAL_BYTES
            || self.restore_proof.byte_length() > MAX_RETAINED_EVIDENCE_BYTES
            || self.commit_marker.byte_length() > MAX_RETAINED_EVIDENCE_BYTES
        {
            return Err(MigrationBackupError::BindingMismatch(
                "retained source backup binding is invalid",
            ));
        }
        Ok(())
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn graph_resource(&self) -> CanonicalGraphResourceId {
        self.graph_resource
    }

    pub(crate) const fn backup_root_identity(&self) -> ContentDigest {
        self.backup_root_identity
    }

    pub(crate) const fn publication_id(&self) -> ContentDigest {
        self.publication_id
    }

    pub(crate) const fn aggregate_digest(&self) -> ContentDigest {
        self.aggregate_digest
    }

    pub(crate) const fn source_inventory(&self) -> BlobDescription {
        self.source_inventory
    }

    pub(crate) const fn file_count(&self) -> u64 {
        self.file_count
    }

    pub(crate) const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub(crate) const fn manifest(&self) -> BlobDescription {
        self.manifest
    }

    pub(crate) const fn restore_proof(&self) -> BlobDescription {
        self.restore_proof
    }

    pub(crate) const fn commit_marker(&self) -> BlobDescription {
        self.commit_marker
    }

    pub(crate) const fn evidence_digest(&self) -> ContentDigest {
        self.evidence_digest
    }

    pub(crate) fn binding_digest(&self) -> ContentDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/source-backup-binding/v1\0");
        hasher.update(self.schema_version.to_be_bytes());
        hasher.update(self.workspace_id.as_uuid().as_bytes());
        for digest in [
            ContentDigest::from_bytes(*self.graph_resource.as_bytes()),
            self.backup_root_identity,
            self.publication_id,
            self.aggregate_digest,
            self.evidence_digest,
        ] {
            hasher.update(digest.as_bytes());
        }
        for description in [
            self.source_inventory,
            self.manifest,
            self.restore_proof,
            self.commit_marker,
        ] {
            hasher.update(description.sha256());
            hasher.update(description.byte_length().to_be_bytes());
        }
        hasher.update(self.file_count.to_be_bytes());
        hasher.update(self.total_bytes.to_be_bytes());
        ContentDigest::from_bytes(hasher.finalize().into())
    }

    #[cfg(test)]
    pub(crate) fn synthetic_for_test(
        workspace_id: WorkspaceId,
        graph_resource: CanonicalGraphResourceId,
        backup_root_identity: ContentDigest,
        publication_id: ContentDigest,
        aggregate_digest: ContentDigest,
    ) -> Self {
        let empty = BlobDescription::of(&[]);
        Self {
            schema_version: SOURCE_BACKUP_BINDING_SCHEMA_VERSION,
            workspace_id,
            graph_resource,
            backup_root_identity,
            publication_id,
            aggregate_digest,
            source_inventory: empty,
            file_count: 0,
            total_bytes: 0,
            manifest: empty,
            restore_proof: empty,
            commit_marker: empty,
            evidence_digest: ContentDigest::of(b"synthetic source backup evidence"),
        }
    }
}

/// Read-only capability for exact point reads from the committed migration
/// backup. The compact binding is authenticated once at reopen; callers still
/// supply and verify the semantic manifest row for every byte read.
pub(crate) struct SourceBackupPayloadAuthority {
    payload: Dir,
}

impl SourceBackupPayloadAuthority {
    pub(crate) fn reopen(
        roots: &MigrationBackupRoot,
        binding: &SourceBackupBindingV1,
    ) -> Result<Self, MigrationBackupError> {
        binding.validate()?;
        roots.freshly_validate_retained_roots()?;
        if roots.root_identity() != binding.backup_root_identity()
            || roots.graph_resource() != binding.graph_resource()
        {
            return Err(MigrationBackupError::BindingMismatch(
                "retained source backup root or graph resource changed",
            ));
        }

        let root = Dir::open_ambient_dir(roots.canonical_root(), ambient_authority())?;
        let base = open_dir_nofollow(&root, BACKUP_ROOT_DIRECTORY)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        let workspace = open_dir_nofollow(&base, &binding.workspace_id().to_string())
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        let publication = open_dir_nofollow(&workspace, &hex(binding.publication_id().as_bytes()))
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;

        require_capability_description(&publication, MANIFEST_FILE, binding.manifest())?;
        require_capability_description(&publication, RESTORE_PROOF_FILE, binding.restore_proof())?;
        let marker = read_capability_file(
            &publication,
            COMMIT_MARKER_FILE,
            MAX_RETAINED_EVIDENCE_BYTES,
        )?;
        if BlobDescription::of(&marker) != binding.commit_marker()
            || marker
                .get(marker.len().saturating_sub(32)..)
                .is_none_or(|suffix| suffix != binding.evidence_digest().as_bytes())
        {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "retained source backup commit marker changed",
            ));
        }
        let payload = open_dir_nofollow(&publication, PAYLOAD_DIRECTORY)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))?;
        Ok(Self { payload })
    }

    pub(crate) fn read_at(
        &self,
        path: &ManagedPath,
        expected: BlobDescription,
    ) -> Result<Vec<u8>, MigrationBackupError> {
        validate_retained_managed_path(path)?;
        let mut components = path.as_str().split('/').peekable();
        let mut directory = self.payload.try_clone()?;
        let mut leaf = None;
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                leaf = Some(component);
            } else {
                directory = open_dir_nofollow(&directory, component).map_err(|error| {
                    MigrationBackupError::Io(io::Error::other(error.to_string()))
                })?;
            }
        }
        let leaf = leaf.ok_or(MigrationBackupError::CorruptOrConflicting(
            "retained source backup path is empty",
        ))?;
        let mut file = open_file_nofollow(&directory, leaf)?;
        let metadata = file.metadata()?;
        if !metadata_is_real_file(&metadata) || metadata.len() != expected.byte_length() {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "retained source backup payload has the wrong shape or length",
            ));
        }
        if metadata.len() > BOOTSTRAP_SOURCE_MAX_FILE_BYTES {
            return Err(MigrationBackupError::ResourceLimit {
                resource: "retained source backup payload bytes",
                observed: metadata.len(),
                limit: BOOTSTRAP_SOURCE_MAX_FILE_BYTES,
            });
        }
        let capacity =
            usize::try_from(metadata.len()).map_err(|_| MigrationBackupError::ResourceLimit {
                resource: "retained source backup payload allocation",
                observed: metadata.len(),
                limit: BOOTSTRAP_SOURCE_MAX_FILE_BYTES,
            })?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| MigrationBackupError::ResourceLimit {
                resource: "retained source backup payload allocation",
                observed: metadata.len(),
                limit: BOOTSTRAP_SOURCE_MAX_FILE_BYTES,
            })?;
        file.read_to_end(&mut bytes)?;
        if BlobDescription::of(&bytes) != expected {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "retained source backup payload digest changed",
            ));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MigrationBackupInstrumentation {
    pub(crate) payload_bytes_written: u64,
    pub(crate) payload_bytes_read: u64,
    pub(crate) restored_bytes_written: u64,
    pub(crate) restored_bytes_read: u64,
    pub(crate) peak_owned_payload_buffer_bytes: u64,
    pub(crate) manifest_entries: u64,
    pub(crate) manifest_chunks: u64,
}

#[derive(Clone, Copy)]
struct SourceSummary {
    file_count: u64,
    chunk_count: u64,
    directory_count: u64,
    total_bytes: u64,
    max_path_bytes: u64,
    max_depth: usize,
}

struct PublicationPaths {
    publication_parent: PathBuf,
    stage: PathBuf,
    final_directory: PathBuf,
}

/// Publish, reopen, and verify one immutable pre-activation source backup.
/// Proof digests are always derived internally from bytes freshly read from the
/// committed backup.
pub(crate) fn verify_migration_source_backup(
    roots: &MigrationBackupRoot,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
) -> Result<VerifiedSourceBackup, MigrationBackupError> {
    validate_bindings(roots, prepared, verified)?;
    let summary = summarize_source(prepared.source_capture())?;
    let paths = publication_paths(roots, verified)?;
    ensure_publication_parents(roots, &paths)?;
    let mut instrumentation = MigrationBackupInstrumentation::default();

    let final_exists = path_exists(&paths.final_directory)?;
    let stage_exists = path_exists(&paths.stage)?;
    if final_exists && stage_exists {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "both staged and final backup directories exist",
        ));
    }
    if !final_exists {
        ensure_real_directory_created(&paths.stage)?;
        ensure_real_directory_created(&paths.stage.join(PAYLOAD_DIRECTORY))?;
        inject_crash_cut(MigrationBackupCrashCut::AfterStagingCreation)?;
        populate_payload_from_capture(
            prepared.source_capture(),
            &paths.stage.join(PAYLOAD_DIRECTORY),
            &mut instrumentation,
        )?;
        inject_crash_cut(MigrationBackupCrashCut::AfterPayloadWrite)?;
        sync_tree(&paths.stage.join(PAYLOAD_DIRECTORY), summary)?;
        inject_crash_cut(MigrationBackupCrashCut::AfterPayloadSync)?;
        let manifest = resume_manifest(
            &paths.stage.join(MANIFEST_FILE),
            prepared,
            verified,
            summary,
            roots.backup_root_identity,
        )?;
        sync_file_and_parent(&paths.stage.join(MANIFEST_FILE))?;
        inject_crash_cut(MigrationBackupCrashCut::AfterManifestPublication)?;
        let _ = verify_backup_directory(
            &paths.stage,
            prepared,
            verified,
            summary,
            roots.backup_root_identity,
            manifest,
            &mut instrumentation,
            false,
        )?;
        sync_directory(&paths.stage)?;
        inject_crash_cut(MigrationBackupCrashCut::AfterStagingSync)?;
        move_reconstructible_noreplace(&paths.stage, &paths.final_directory).map_err(|_| {
            MigrationBackupError::CorruptOrConflicting(
                "final backup destination appeared or staged rename failed",
            )
        })?;
        sync_directory(&paths.publication_parent)?;
        inject_crash_cut(MigrationBackupCrashCut::AfterStagingRename)?;
    }

    require_real_directory(
        &paths.final_directory,
        "final backup is not a real directory",
    )?;
    let manifest = compare_expected_manifest(
        &paths.final_directory.join(MANIFEST_FILE),
        prepared,
        verified,
        summary,
        roots.backup_root_identity,
    )?;
    let verified_inventory = verify_backup_directory(
        &paths.final_directory,
        prepared,
        verified,
        summary,
        roots.backup_root_identity,
        manifest,
        &mut instrumentation,
        true,
    )?;
    inject_crash_cut(MigrationBackupCrashCut::AfterCommittedBackupReopen)?;

    let proof_bytes = restore_proof_bytes(
        prepared,
        verified,
        summary,
        roots.backup_root_identity,
        manifest,
        verified_inventory,
    );
    let restore_proof = publish_small_file_atomic(
        &paths.final_directory,
        RESTORE_PROOF_STAGE_FILE,
        RESTORE_PROOF_FILE,
        &proof_bytes,
        "restore proof conflicts with an existing backup",
    )?;
    inject_crash_cut(MigrationBackupCrashCut::AfterProofPublication)?;

    let (marker_bytes, evidence_digest) = commit_marker_bytes(
        prepared,
        verified,
        summary,
        roots.backup_root_identity,
        manifest,
        restore_proof,
    );
    let commit_marker = publish_small_file_atomic(
        &paths.final_directory,
        COMMIT_MARKER_STAGE_FILE,
        COMMIT_MARKER_FILE,
        &marker_bytes,
        "commit marker conflicts with an existing backup",
    )?;
    inject_crash_cut(MigrationBackupCrashCut::AfterCommitMarkerPublication)?;

    let final_manifest = compare_expected_manifest(
        &paths.final_directory.join(MANIFEST_FILE),
        prepared,
        verified,
        summary,
        roots.backup_root_identity,
    )?;
    if final_manifest != manifest {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "manifest changed after proof derivation",
        ));
    }
    let _ = verify_backup_directory(
        &paths.final_directory,
        prepared,
        verified,
        summary,
        roots.backup_root_identity,
        final_manifest,
        &mut instrumentation,
        true,
    )?;
    compare_exact_small_file(
        &paths.final_directory.join(RESTORE_PROOF_FILE),
        &proof_bytes,
        "restore proof changed after publication",
    )?;
    compare_exact_small_file(
        &paths.final_directory.join(COMMIT_MARKER_FILE),
        &marker_bytes,
        "commit marker changed after publication",
    )?;
    validate_final_root_entries(&paths.final_directory)?;
    sync_directory(&paths.final_directory)?;

    Ok(VerifiedSourceBackup {
        directory: paths.final_directory,
        workspace_id: verified.workspace_id(),
        graph_resource: verified.graph_resource(),
        backup_root_identity: roots.backup_root_identity,
        publication_id: *verified.publication_id().as_bytes(),
        aggregate_digest: *verified.aggregate_digest().as_bytes(),
        source_inventory: prepared.source_capture().inventory_description(),
        file_count: summary.file_count,
        total_bytes: summary.total_bytes,
        manifest,
        restore_proof,
        commit_marker,
        evidence_digest,
        instrumentation,
    })
}

fn validate_bindings(
    roots: &MigrationBackupRoot,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
) -> Result<(), MigrationBackupError> {
    let aggregate = prepared.aggregate();
    let capture = prepared.source_capture();
    let expected_frontier = prepared
        .candidate()
        .accepted_frontier_root()
        .map_err(|_| MigrationBackupError::BindingMismatch("prepared frontier is invalid"))?;
    let bootstrap_binding =
        BootstrapAggregateHistoryBindingV1::for_aggregate(aggregate).map_err(|_| {
            MigrationBackupError::BindingMismatch("prepared bootstrap history binding is invalid")
        })?;
    if aggregate.workspace_id() != verified.workspace_id()
        || aggregate.lineage_digest() != verified.lineage_digest()
        || aggregate.graph_resource() != verified.graph_resource()
        || aggregate.publication_id() != verified.publication_id()
        || aggregate.aggregate_digest() != verified.aggregate_digest()
        || aggregate.import_id() != verified.import_id()
        || aggregate.parts().len() as u32 != verified.part_count()
        || aggregate.final_frontier().last_part() != verified.predecessor_terminal()
        || &expected_frontier != verified.accepted_frontier()
        || !prepared
            .candidate()
            .durable_history_binding()
            .same_replay_authority(verified.engine_binding())
        || bootstrap_binding != verified.bootstrap_binding()
        || verified.storage_binding().endpoint.graph_resource_id() != verified.graph_resource()
        || verified.history_generation() != u64::from(verified.part_count())
        || verified.cold_record_count() != u64::from(verified.part_count())
    {
        return Err(MigrationBackupError::BindingMismatch(
            "prepared and verified bootstrap publications do not match",
        ));
    }
    if capture.graph_resource() != verified.graph_resource()
        || roots.graph_resource != verified.graph_resource()
    {
        return Err(MigrationBackupError::BindingMismatch(
            "backup root graph capability does not match source provenance",
        ));
    }
    roots.freshly_validate_retained_roots()
}

fn summarize_source(
    capture: &BootstrapSourceCapture,
) -> Result<SourceSummary, MigrationBackupError> {
    let mut entries = capture.entries_cursor()?;
    let mut file_count = 0_u64;
    let mut chunk_count = 0_u64;
    let mut directory_count = 0_u64;
    let mut total_bytes = 0_u64;
    let mut max_path_bytes = 0_u64;
    let mut max_depth = 0_usize;
    let mut previous_parents = Vec::<String>::new();
    while let Some(entry) = entries.next()? {
        validate_entry_bounds(&entry)?;
        file_count = checked_add(file_count, 1, "source files")?;
        enforce_limit("source files", file_count, BOOTSTRAP_SOURCE_MAX_FILES)?;
        chunk_count = checked_add(chunk_count, u64::from(entry.chunk_count()), "source chunks")?;
        total_bytes = checked_add(
            total_bytes,
            entry.description().byte_length(),
            "source bytes",
        )?;
        enforce_limit(
            "source bytes",
            total_bytes,
            BOOTSTRAP_SOURCE_MAX_TOTAL_BYTES,
        )?;
        max_path_bytes = max_path_bytes.max(entry.path().as_str().len() as u64);
        let depth = entry.path().as_str().split('/').count();
        enforce_limit(
            "source capture depth",
            depth as u64,
            BOOTSTRAP_SOURCE_MAX_DIRECTORY_DEPTH.saturating_add(1) as u64,
        )?;
        max_depth = max_depth.max(depth);
        let parents = entry
            .path()
            .as_str()
            .split('/')
            .rev()
            .skip(1)
            .collect::<Vec<_>>();
        let parents = parents.into_iter().rev().collect::<Vec<_>>();
        let common = parents
            .iter()
            .zip(&previous_parents)
            .take_while(|(left, right)| **left == right.as_str())
            .count();
        directory_count = checked_add(
            directory_count,
            (parents.len() - common) as u64,
            "source directories",
        )?;
        enforce_limit(
            "source directories",
            directory_count,
            BOOTSTRAP_SOURCE_MAX_DIRECTORIES,
        )?;
        previous_parents.clear();
        previous_parents.extend(parents.into_iter().map(ToOwned::to_owned));
    }
    if file_count != capture.source_file_count() || chunk_count != capture.source_chunk_count() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "sealed source summary differs from its cursors",
        ));
    }
    Ok(SourceSummary {
        file_count,
        chunk_count,
        directory_count,
        total_bytes,
        max_path_bytes,
        max_depth,
    })
}

fn publication_paths(
    roots: &MigrationBackupRoot,
    verified: &InactiveBootstrapVerifiedPublication,
) -> Result<PublicationPaths, MigrationBackupError> {
    let workspace = verified.workspace_id().to_string();
    let publication = hex(verified.publication_id().as_bytes());
    let publication_parent = roots
        .canonical_root
        .join(BACKUP_ROOT_DIRECTORY)
        .join(workspace.clone());
    let stage = publication_parent.join(format!(".{publication}.staging"));
    let final_directory = publication_parent.join(&publication);
    for path in [&stage, &final_directory] {
        if path == &roots.canonical_graph_root || path.starts_with(&roots.canonical_graph_root) {
            return Err(MigrationBackupError::InvalidRoot(
                "backup publication would be inside the graph",
            ));
        }
    }
    Ok(PublicationPaths {
        publication_parent,
        stage,
        final_directory,
    })
}

fn ensure_publication_parents(
    roots: &MigrationBackupRoot,
    paths: &PublicationPaths,
) -> Result<(), MigrationBackupError> {
    let backup_base = roots.canonical_root.join(BACKUP_ROOT_DIRECTORY);
    ensure_real_directory_created(&backup_base)?;
    ensure_real_directory_created(&paths.publication_parent)
}

fn validate_entry_bounds(entry: &BootstrapSourceEntry) -> Result<(), MigrationBackupError> {
    enforce_limit(
        "managed path bytes",
        entry.path().as_str().len() as u64,
        BOOTSTRAP_SOURCE_MAX_PATH_BYTES as u64,
    )?;
    enforce_limit(
        "logical name bytes",
        entry.logical_name().len() as u64,
        BOOTSTRAP_SOURCE_MAX_LOGICAL_NAME_BYTES,
    )?;
    enforce_limit(
        "source file bytes",
        entry.description().byte_length(),
        BOOTSTRAP_SOURCE_MAX_FILE_BYTES,
    )
}

fn checked_add(
    current: u64,
    growth: u64,
    resource: &'static str,
) -> Result<u64, MigrationBackupError> {
    current
        .checked_add(growth)
        .ok_or(MigrationBackupError::ResourceLimit {
            resource,
            observed: u64::MAX,
            limit: u64::MAX - 1,
        })
}

fn enforce_limit(
    resource: &'static str,
    observed: u64,
    limit: u64,
) -> Result<(), MigrationBackupError> {
    if observed > limit {
        Err(MigrationBackupError::ResourceLimit {
            resource,
            observed,
            limit,
        })
    } else {
        Ok(())
    }
}

fn manifest_header(
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
) -> Result<Vec<u8>, MigrationBackupError> {
    let aggregate = prepared.aggregate();
    let capture = prepared.source_capture();
    let aggregate_description = BlobDescription::of(&aggregate.encode().map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("prepared aggregate no longer encodes")
    })?);
    let commit_description = BlobDescription::of(&prepared.commit().encode().map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("prepared aggregate commit no longer encodes")
    })?);
    let capture_identity = capture.capture_identity()?;
    let frontier = verified.accepted_frontier();
    let storage = verified.storage_binding();
    let bootstrap = verified.bootstrap_binding();
    let mut header = Vec::with_capacity(1024 + aggregate.parts().len() * 128);
    header.extend_from_slice(MANIFEST_MAGIC);
    put_u32(&mut header, BACKUP_SCHEMA_VERSION);
    put_u32(&mut header, OPLOG_PROTOCOL_VERSION);
    put_u32(&mut header, OPERATION_SCHEMA_VERSION);
    put_u32(&mut header, OBJECT_ENVELOPE_SCHEMA_VERSION);
    put_u32(&mut header, MANIFEST_ENCODING_VERSION);
    put_u32(&mut header, DIFF_SCHEMA_VERSION);
    put_u32(&mut header, MANAGED_ENTITY_SET_VERSION);
    put_u32(&mut header, BOOTSTRAP_IMPORT_SCHEMA_VERSION);
    put_u32(&mut header, BOOTSTRAP_FRONTIER_SCHEMA_VERSION);
    put_u32(&mut header, BOOTSTRAP_SOURCE_CAPTURE_SCHEMA);
    header.extend_from_slice(verified.workspace_id().as_uuid().as_bytes());
    header.extend_from_slice(verified.lineage_digest().as_bytes());
    header.extend_from_slice(verified.graph_resource().as_bytes());
    header.extend_from_slice(backup_root_identity.as_bytes());
    header.extend_from_slice(verified.publication_id().as_bytes());
    header.extend_from_slice(verified.aggregate_digest().as_bytes());
    header.extend_from_slice(verified.import_id().as_bytes());
    put_description(&mut header, aggregate_description);
    put_description(&mut header, commit_description);
    put_description(&mut header, capture_identity);
    put_description(&mut header, capture.inventory_description());
    put_description(&mut header, capture.entries_description());
    put_description(&mut header, capture.chunks_description());
    put_u64(&mut header, summary.file_count);
    put_u64(&mut header, summary.chunk_count);
    put_u64(&mut header, summary.directory_count);
    put_u64(&mut header, summary.total_bytes);
    put_u32(&mut header, verified.part_count());
    let final_frontier = aggregate.final_frontier().encode();
    put_bytes(&mut header, &final_frontier)?;
    put_u64(&mut header, frontier.acceptance_sequence());
    put_u64(&mut header, frontier.document_count());
    put_u64(&mut header, frontier.retained_bytes_total());
    put_optional_16(&mut header, frontier.document_map_root_key());
    header.extend_from_slice(frontier.document_map_root_digest().as_bytes());
    put_optional_16(&mut header, frontier.batch_map_root_key());
    header.extend_from_slice(frontier.batch_map_root_digest().as_bytes());
    header.extend_from_slice(frontier.state_digest().as_bytes());
    header.extend_from_slice(storage.endpoint.endpoint_id().as_uuid().as_bytes());
    header.extend_from_slice(storage.endpoint.device_id().as_uuid().as_bytes());
    header.extend_from_slice(storage.endpoint.graph_resource_id().as_bytes());
    header.extend_from_slice(storage.receipt_store_id.as_bytes());
    header.extend_from_slice(bootstrap.publication_id().as_bytes());
    header.extend_from_slice(bootstrap.aggregate_digest().as_bytes());
    put_u32(&mut header, bootstrap.part_count());
    put_bytes(&mut header, &bootstrap.final_frontier().encode())?;
    header.extend_from_slice(verified.archive_identity().binding_digest().as_bytes());
    put_u64(&mut header, verified.history_generation());
    header.extend_from_slice(verified.history_root().as_bytes());
    put_u64(&mut header, verified.cold_record_count());
    put_u32(&mut header, aggregate.parts().len() as u32);
    for descriptor in aggregate.parts().iter().copied() {
        header.extend_from_slice(descriptor.part_id().as_bytes());
        header.extend_from_slice(descriptor.batch_id().as_uuid().as_bytes());
        put_bytes(&mut header, &descriptor.post_frontier().encode())?;
    }
    if header.len() > MAX_MANIFEST_HEADER_BYTES {
        return Err(MigrationBackupError::ResourceLimit {
            resource: "backup manifest header bytes",
            observed: header.len() as u64,
            limit: MAX_MANIFEST_HEADER_BYTES as u64,
        });
    }
    Ok(header)
}

fn emit_manifest(
    output: &mut impl Write,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
) -> Result<(), MigrationBackupError> {
    write_with_partial_crash_cut(
        output,
        &manifest_header(prepared, verified, summary, backup_root_identity)?,
        MigrationBackupCrashCut::PartialManifestHeaderWrite,
    )?;
    let capture = prepared.source_capture();
    let mut entries = capture.entries_cursor()?;
    let mut chunks = capture.chunks_cursor()?;
    let mut emitted_entries = 0_u64;
    let mut emitted_chunks = 0_u64;
    while let Some(entry) = entries.next()? {
        validate_entry_bounds(&entry)?;
        if emitted_entries == 0 {
            write_len_prefixed_with_cut(
                output,
                entry.path().as_str().as_bytes(),
                MigrationBackupCrashCut::PartialManifestEntryWrite,
            )?;
        } else {
            write_len_prefixed(output, entry.path().as_str().as_bytes())?;
        }
        output.write_all(&[match entry.kind() {
            ManagedTextKind::Page => 0,
            ManagedTextKind::Journal => 1,
        }])?;
        write_len_prefixed(output, entry.logical_name().as_bytes())?;
        write_description(output, entry.description())?;
        output.write_all(entry.file_resource().as_bytes())?;
        output.write_all(&entry.link_count().to_be_bytes())?;
        output.write_all(&entry.chunk_count().to_be_bytes())?;
        emitted_entries = checked_add(emitted_entries, 1, "manifest entries")?;
        for ordinal in 0..entry.chunk_count() {
            let chunk = chunks
                .next()?
                .ok_or(MigrationBackupError::CorruptOrConflicting(
                    "source chunk spool ended before its entry",
                ))?;
            if chunk.path() != entry.path() || chunk.ordinal() != ordinal {
                return Err(MigrationBackupError::CorruptOrConflicting(
                    "source chunks do not match canonical entry order",
                ));
            }
            output.write_all(&ordinal.to_be_bytes())?;
            write_description(output, chunk.description())?;
            emitted_chunks = checked_add(emitted_chunks, 1, "manifest chunks")?;
        }
    }
    if chunks.next()?.is_some()
        || emitted_entries != summary.file_count
        || emitted_chunks != summary.chunk_count
    {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "source manifest cursor counts do not match the sealed capture",
        ));
    }
    Ok(())
}

fn resume_manifest(
    path: &Path,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
) -> Result<BlobDescription, MigrationBackupError> {
    const CONFLICT: &str = "backup manifest conflicts with staging";
    let mut output = ResumableExactFile::open(path, CONFLICT)?;
    emit_manifest(
        &mut output,
        prepared,
        verified,
        summary,
        backup_root_identity,
    )
    .map_err(|error| match error {
        MigrationBackupError::Io(error) if error.kind() == io::ErrorKind::InvalidData => {
            MigrationBackupError::CorruptOrConflicting(CONFLICT)
        }
        other => other,
    })?;
    output.finish()
}

fn compare_expected_manifest(
    path: &Path,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
) -> Result<BlobDescription, MigrationBackupError> {
    const CONFLICT: &str = "backup manifest is corrupt or conflicting";
    let mut output = ExactFileComparator::open(path, CONFLICT)?;
    emit_manifest(
        &mut output,
        prepared,
        verified,
        summary,
        backup_root_identity,
    )
    .map_err(|error| match error {
        MigrationBackupError::Io(error) if error.kind() == io::ErrorKind::InvalidData => {
            MigrationBackupError::CorruptOrConflicting(CONFLICT)
        }
        other => other,
    })?;
    output.finish()
}

struct ResumableExactFile {
    existing: Option<File>,
    remaining_existing: u64,
    append: File,
    hasher: Sha256,
    expected_length: u64,
    conflict: &'static str,
}

impl ResumableExactFile {
    fn open(path: &Path, conflict: &'static str) -> Result<Self, MigrationBackupError> {
        let (existing, remaining_existing, append) = match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata_is_real_file(&metadata) => {
                return Err(MigrationBackupError::CorruptOrConflicting(conflict))
            }
            Ok(metadata) => (
                Some(open_regular_readonly_nofollow(path)?),
                metadata.len(),
                open_regular_append_nofollow(path)?,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                (None, 0, create_new_regular_nofollow(path)?)
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            existing,
            remaining_existing,
            append,
            hasher: Sha256::new(),
            expected_length: 0,
            conflict,
        })
    }

    fn finish(mut self) -> Result<BlobDescription, MigrationBackupError> {
        if self.remaining_existing != 0 {
            return Err(MigrationBackupError::CorruptOrConflicting(self.conflict));
        }
        self.append.flush()?;
        Ok(BlobDescription::from_parts(
            self.hasher.finalize().into(),
            self.expected_length,
        ))
    }
}

impl Write for ResumableExactFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let compare_len = usize::try_from(self.remaining_existing.min(bytes.len() as u64))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, self.conflict))?;
        if compare_len != 0 {
            let mut actual = vec![0_u8; compare_len];
            self.existing
                .as_mut()
                .expect("existing bytes have a reader")
                .read_exact(&mut actual)?;
            if actual != bytes[..compare_len] {
                return Err(io::Error::new(io::ErrorKind::InvalidData, self.conflict));
            }
            self.remaining_existing -= compare_len as u64;
        }
        if compare_len < bytes.len() {
            self.append.write_all(&bytes[compare_len..])?;
        }
        self.hasher.update(bytes);
        self.expected_length = self
            .expected_length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.append.flush()
    }
}

struct ExactFileComparator {
    file: File,
    hasher: Sha256,
    expected_length: u64,
    conflict: &'static str,
}

impl ExactFileComparator {
    fn open(path: &Path, conflict: &'static str) -> Result<Self, MigrationBackupError> {
        require_real_file(path, conflict)?;
        Ok(Self {
            file: open_regular_readonly_nofollow(path)?,
            hasher: Sha256::new(),
            expected_length: 0,
            conflict,
        })
    }

    fn finish(mut self) -> Result<BlobDescription, MigrationBackupError> {
        let mut trailing = [0_u8; 1];
        if self.file.read(&mut trailing)? != 0 {
            return Err(MigrationBackupError::CorruptOrConflicting(self.conflict));
        }
        Ok(BlobDescription::from_parts(
            self.hasher.finalize().into(),
            self.expected_length,
        ))
    }
}

impl Write for ExactFileComparator {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut actual = vec![0_u8; bytes.len()];
        self.file
            .read_exact(&mut actual)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, self.conflict))?;
        if actual != bytes {
            return Err(io::Error::new(io::ErrorKind::InvalidData, self.conflict));
        }
        self.hasher.update(bytes);
        self.expected_length = self
            .expected_length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_description(output: &mut Vec<u8>, description: BlobDescription) {
    output.extend_from_slice(description.sha256());
    put_u64(output, description.byte_length());
}

fn put_optional_16(output: &mut Vec<u8>, value: Option<[u8; 16]>) {
    match value {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&value);
        }
        None => {
            output.push(0);
            output.extend_from_slice(&[0_u8; 16]);
        }
    }
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), MigrationBackupError> {
    let length = u32::try_from(bytes.len()).map_err(|_| MigrationBackupError::ResourceLimit {
        resource: "manifest field bytes",
        observed: bytes.len() as u64,
        limit: u32::MAX as u64,
    })?;
    put_u32(output, length);
    output.extend_from_slice(bytes);
    Ok(())
}

fn write_len_prefixed(output: &mut impl Write, bytes: &[u8]) -> Result<(), MigrationBackupError> {
    let length = u32::try_from(bytes.len()).map_err(|_| MigrationBackupError::ResourceLimit {
        resource: "manifest string bytes",
        observed: bytes.len() as u64,
        limit: u32::MAX as u64,
    })?;
    output.write_all(&length.to_be_bytes())?;
    output.write_all(bytes)?;
    Ok(())
}

fn write_len_prefixed_with_cut(
    output: &mut impl Write,
    bytes: &[u8],
    cut: MigrationBackupCrashCut,
) -> Result<(), MigrationBackupError> {
    let length = u32::try_from(bytes.len()).map_err(|_| MigrationBackupError::ResourceLimit {
        resource: "manifest string bytes",
        observed: bytes.len() as u64,
        limit: u32::MAX as u64,
    })?;
    write_with_partial_crash_cut(output, &length.to_be_bytes(), cut)?;
    output.write_all(bytes)?;
    Ok(())
}

fn write_description(
    output: &mut impl Write,
    description: BlobDescription,
) -> Result<(), MigrationBackupError> {
    output.write_all(description.sha256())?;
    output.write_all(&description.byte_length().to_be_bytes())?;
    Ok(())
}

fn populate_payload_from_capture(
    capture: &BootstrapSourceCapture,
    payload: &Path,
    instrumentation: &mut MigrationBackupInstrumentation,
) -> Result<(), MigrationBackupError> {
    let mut entries = capture.entries_cursor()?;
    let mut chunks = capture.chunks_cursor()?;
    while let Some(entry) = entries.next()? {
        validate_entry_bounds(&entry)?;
        let destination = payload_path(payload, entry.path())?;
        ensure_managed_parent_directories(payload, entry.path())?;
        resume_payload_file_from_capture(
            capture,
            &entry,
            &mut chunks,
            &destination,
            instrumentation,
        )?;
    }
    if chunks.next()?.is_some() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "source capture has trailing chunks",
        ));
    }
    Ok(())
}

fn resume_payload_file_from_capture(
    capture: &BootstrapSourceCapture,
    entry: &BootstrapSourceEntry,
    chunks: &mut BootstrapSourceChunkCursor,
    destination: &Path,
    instrumentation: &mut MigrationBackupInstrumentation,
) -> Result<(), MigrationBackupError> {
    let existing_len = match fs::symlink_metadata(destination) {
        Ok(metadata) if !metadata_is_real_file(&metadata) => {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "staged payload target is not a regular file",
            ))
        }
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    if existing_len > entry.description().byte_length() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "staged payload file is longer than its source",
        ));
    }
    let mut existing = (existing_len != 0)
        .then(|| open_regular_readonly_nofollow(destination))
        .transpose()?;
    let mut output = if path_exists(destination)? {
        open_regular_append_nofollow(destination)?
    } else {
        create_new_regular_nofollow(destination)?
    };
    let mut remaining_existing = existing_len;
    let mut full_hasher = Sha256::new();
    let mut full_length = 0_u64;
    let mut buffer = [0_u8; IO_BUFFER_BYTES];
    instrumentation.peak_owned_payload_buffer_bytes = instrumentation
        .peak_owned_payload_buffer_bytes
        .max(buffer.len() as u64);
    for ordinal in 0..entry.chunk_count() {
        let chunk = chunks
            .next()?
            .ok_or(MigrationBackupError::CorruptOrConflicting(
                "source chunk spool ended while copying a file",
            ))?;
        if chunk.path() != entry.path() || chunk.ordinal() != ordinal {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "source chunk order differs from its entry",
            ));
        }
        let mut source = capture.open_chunk(&chunk)?;
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let compare = usize::try_from(remaining_existing.min(read as u64)).map_err(|_| {
                MigrationBackupError::CorruptOrConflicting(
                    "existing payload comparison length overflow",
                )
            })?;
            if compare != 0 {
                let mut actual = [0_u8; IO_BUFFER_BYTES];
                existing
                    .as_mut()
                    .expect("existing payload prefix has a reader")
                    .read_exact(&mut actual[..compare])?;
                if actual[..compare] != buffer[..compare] {
                    return Err(MigrationBackupError::CorruptOrConflicting(
                        "staged payload prefix conflicts with the source capture",
                    ));
                }
                remaining_existing -= compare as u64;
            }
            if compare < read {
                write_with_partial_crash_cut(
                    &mut output,
                    &buffer[compare..read],
                    MigrationBackupCrashCut::PartialPayloadWrite,
                )?;
                instrumentation.payload_bytes_written = checked_add(
                    instrumentation.payload_bytes_written,
                    (read - compare) as u64,
                    "payload bytes written",
                )?;
            }
            full_hasher.update(&buffer[..read]);
            full_length = checked_add(full_length, read as u64, "payload file bytes")?;
        }
        source.finish()?;
    }
    if remaining_existing != 0 {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "staged payload prefix exceeds emitted source bytes",
        ));
    }
    output.flush()?;
    let actual = BlobDescription::from_parts(full_hasher.finalize().into(), full_length);
    if actual != entry.description() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "source chunks do not reproduce their file description",
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct ManifestChunk {
    ordinal: u32,
    description: BlobDescription,
}

struct ManifestEntry {
    path: ManagedPath,
    kind: ManagedTextKind,
    logical_name: String,
    description: BlobDescription,
    file_resource: ContentDigest,
    link_count: u64,
    chunks: Vec<ManifestChunk>,
}

struct ManifestReader {
    file: File,
    remaining_entries: u64,
    remaining_chunks: u64,
    previous_path: Option<ManagedPath>,
}

impl ManifestReader {
    fn open(
        path: &Path,
        prepared: &InactiveBootstrapPreparedPublication,
        verified: &InactiveBootstrapVerifiedPublication,
        summary: SourceSummary,
        backup_root_identity: ContentDigest,
    ) -> Result<Self, MigrationBackupError> {
        require_real_file(path, "backup manifest is not a regular file")?;
        let mut file = open_regular_readonly_nofollow(path)?;
        let expected_header = manifest_header(prepared, verified, summary, backup_root_identity)?;
        let mut actual_header = vec![0_u8; expected_header.len()];
        file.read_exact(&mut actual_header).map_err(|_| {
            MigrationBackupError::CorruptOrConflicting("backup manifest header is truncated")
        })?;
        if actual_header != expected_header {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "backup manifest header differs from prepared and verified authority",
            ));
        }
        Ok(Self {
            file,
            remaining_entries: summary.file_count,
            remaining_chunks: summary.chunk_count,
            previous_path: None,
        })
    }

    fn next(&mut self) -> Result<Option<ManifestEntry>, MigrationBackupError> {
        if self.remaining_entries == 0 {
            return Ok(None);
        }
        let path_bytes = read_bounded_bytes(
            &mut self.file,
            BOOTSTRAP_SOURCE_MAX_PATH_BYTES,
            "manifest path",
        )?;
        let path_string = String::from_utf8(path_bytes).map_err(|_| {
            MigrationBackupError::CorruptOrConflicting("manifest path is not UTF-8")
        })?;
        let path = ManagedPath::parse(path_string)
            .map_err(|_| MigrationBackupError::CorruptOrConflicting("manifest path is unsafe"))?;
        if self
            .previous_path
            .as_ref()
            .is_some_and(|previous| previous >= &path)
        {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "manifest paths are duplicated or reordered",
            ));
        }
        let kind = match read_u8(&mut self.file)? {
            0 => ManagedTextKind::Page,
            1 => ManagedTextKind::Journal,
            _ => {
                return Err(MigrationBackupError::CorruptOrConflicting(
                    "manifest contains an unknown managed kind",
                ))
            }
        };
        let logical_name_bytes = read_bounded_bytes(
            &mut self.file,
            usize::try_from(BOOTSTRAP_SOURCE_MAX_LOGICAL_NAME_BYTES).map_err(|_| {
                MigrationBackupError::CorruptOrConflicting(
                    "logical-name bound does not fit this platform",
                )
            })?,
            "manifest logical name",
        )?;
        let logical_name = String::from_utf8(logical_name_bytes).map_err(|_| {
            MigrationBackupError::CorruptOrConflicting("manifest logical name is not UTF-8")
        })?;
        let description = read_description(&mut self.file)?;
        enforce_limit(
            "manifest source file bytes",
            description.byte_length(),
            BOOTSTRAP_SOURCE_MAX_FILE_BYTES,
        )?;
        let file_resource = ContentDigest::from_bytes(read_array_32(&mut self.file)?);
        let link_count = read_u64(&mut self.file)?;
        let chunk_count = read_u32(&mut self.file)?;
        if u64::from(chunk_count) > self.remaining_chunks
            || u64::from(chunk_count)
                > BOOTSTRAP_SOURCE_MAX_FILE_BYTES.div_ceil(BOOTSTRAP_SOURCE_CHUNK_BYTES as u64)
        {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "manifest chunk count exceeds its bound",
            ));
        }
        let mut chunks = Vec::with_capacity(chunk_count as usize);
        let mut chunk_bytes = 0_u64;
        for expected_ordinal in 0..chunk_count {
            let ordinal = read_u32(&mut self.file)?;
            let description = read_description(&mut self.file)?;
            if ordinal != expected_ordinal
                || description.byte_length() > BOOTSTRAP_SOURCE_CHUNK_BYTES as u64
                || (description.byte_length() == 0 && chunk_count != 0)
            {
                return Err(MigrationBackupError::CorruptOrConflicting(
                    "manifest chunks are malformed, duplicated, or reordered",
                ));
            }
            chunk_bytes = checked_add(
                chunk_bytes,
                description.byte_length(),
                "manifest chunk bytes",
            )?;
            chunks.push(ManifestChunk {
                ordinal,
                description,
            });
        }
        if chunk_bytes != description.byte_length()
            || (description.byte_length() == 0) != (chunk_count == 0)
        {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "manifest chunk lengths do not reproduce the file length",
            ));
        }
        self.remaining_entries -= 1;
        self.remaining_chunks -= u64::from(chunk_count);
        self.previous_path = Some(path.clone());
        Ok(Some(ManifestEntry {
            path,
            kind,
            logical_name,
            description,
            file_resource,
            link_count,
            chunks,
        }))
    }

    fn finish(mut self) -> Result<(), MigrationBackupError> {
        if self.remaining_entries != 0 || self.remaining_chunks != 0 {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "manifest ended before its declared counts",
            ));
        }
        let mut trailing = [0_u8; 1];
        if self.file.read(&mut trailing)? != 0 {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "manifest contains trailing or extra entries",
            ));
        }
        Ok(())
    }
}

fn verify_backup_directory(
    directory: &Path,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
    expected_manifest: BlobDescription,
    instrumentation: &mut MigrationBackupInstrumentation,
    final_directory: bool,
) -> Result<InventoryProof, MigrationBackupError> {
    let actual_manifest = compare_expected_manifest(
        &directory.join(MANIFEST_FILE),
        prepared,
        verified,
        summary,
        backup_root_identity,
    )?;
    if actual_manifest != expected_manifest {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "manifest description changed",
        ));
    }
    validate_backup_root_entries_for_state(directory, final_directory)?;
    let observed = verify_tree_from_manifest(
        &directory.join(MANIFEST_FILE),
        &directory.join(PAYLOAD_DIRECTORY),
        prepared,
        verified,
        summary,
        backup_root_identity,
        instrumentation,
        TreeReadKind::Backup,
    )?;
    if observed.file_count != summary.file_count || observed.total_bytes != summary.total_bytes {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "backup payload summary differs from the manifest",
        ));
    }
    Ok(observed)
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum TreeReadKind {
    Backup,
    Restore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InventoryProof {
    digest: ContentDigest,
    file_count: u64,
    total_bytes: u64,
}

fn verify_tree_from_manifest(
    manifest_path: &Path,
    tree: &Path,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
    instrumentation: &mut MigrationBackupInstrumentation,
    read_kind: TreeReadKind,
) -> Result<InventoryProof, MigrationBackupError> {
    require_real_directory(tree, "manifest payload tree is not a real directory")?;
    let counts = count_tree(tree, summary)?;
    if counts.files != summary.file_count
        || counts.directories != summary.directory_count
        || counts.bytes != summary.total_bytes
    {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "payload tree has missing or extra files or directories",
        ));
    }
    let mut reader = ManifestReader::open(
        manifest_path,
        prepared,
        verified,
        summary,
        backup_root_identity,
    )?;
    let mut inventory = Sha256::new();
    inventory.update(b"tine/migration-source-restored-inventory/v1\0");
    let mut file_count = 0_u64;
    let mut total_bytes = 0_u64;
    while let Some(entry) = reader.next()? {
        let path = payload_path(tree, &entry.path)?;
        let actual = verify_file_against_entry(&path, &entry, instrumentation, read_kind)?;
        if actual != entry.description {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "payload file digest or length differs from its manifest",
            ));
        }
        hash_inventory_entry(&mut inventory, &entry);
        file_count = checked_add(file_count, 1, "verified files")?;
        total_bytes = checked_add(total_bytes, actual.byte_length(), "verified payload bytes")?;
    }
    reader.finish()?;
    if file_count != summary.file_count || total_bytes != summary.total_bytes {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "verified payload counts differ from the source summary",
        ));
    }
    Ok(InventoryProof {
        digest: ContentDigest::from_bytes(inventory.finalize().into()),
        file_count,
        total_bytes,
    })
}

fn verify_file_against_entry(
    path: &Path,
    entry: &ManifestEntry,
    instrumentation: &mut MigrationBackupInstrumentation,
    read_kind: TreeReadKind,
) -> Result<BlobDescription, MigrationBackupError> {
    require_real_file(path, "manifest-listed payload is not a regular file")?;
    if fs::metadata(path)?.len() != entry.description.byte_length() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "manifest-listed payload length differs",
        ));
    }
    let mut file = open_regular_readonly_nofollow(path)?;
    let mut full = Sha256::new();
    let mut full_length = 0_u64;
    let mut buffer = [0_u8; IO_BUFFER_BYTES];
    instrumentation.peak_owned_payload_buffer_bytes = instrumentation
        .peak_owned_payload_buffer_bytes
        .max(buffer.len() as u64);
    for chunk in &entry.chunks {
        let mut chunk_hasher = Sha256::new();
        let mut remaining = chunk.description.byte_length();
        while remaining != 0 {
            let requested = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
                MigrationBackupError::CorruptOrConflicting("manifest chunk read length overflow")
            })?;
            let read = file.read(&mut buffer[..requested])?;
            if read == 0 {
                return Err(MigrationBackupError::CorruptOrConflicting(
                    "manifest-listed payload is truncated",
                ));
            }
            remaining -= read as u64;
            chunk_hasher.update(&buffer[..read]);
            full.update(&buffer[..read]);
            full_length = checked_add(full_length, read as u64, "verified file bytes")?;
            match read_kind {
                TreeReadKind::Backup => {
                    instrumentation.payload_bytes_read = checked_add(
                        instrumentation.payload_bytes_read,
                        read as u64,
                        "payload bytes read",
                    )?;
                }
                TreeReadKind::Restore => {
                    instrumentation.restored_bytes_read = checked_add(
                        instrumentation.restored_bytes_read,
                        read as u64,
                        "restored bytes read",
                    )?;
                }
            }
        }
        let actual_chunk = BlobDescription::from_parts(
            chunk_hasher.finalize().into(),
            chunk.description.byte_length(),
        );
        if actual_chunk != chunk.description {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "manifest-listed payload chunk digest differs",
            ));
        }
    }
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "manifest-listed payload has trailing bytes",
        ));
    }
    Ok(BlobDescription::from_parts(
        full.finalize().into(),
        full_length,
    ))
}

#[allow(dead_code)]
fn restore_from_manifest(
    backup: &Path,
    restore: &Path,
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
    instrumentation: &mut MigrationBackupInstrumentation,
) -> Result<(), MigrationBackupError> {
    let _ = traverse_tree_bounded(restore, summary, false)?;
    let manifest_path = backup.join(MANIFEST_FILE);
    let mut reader = ManifestReader::open(
        &manifest_path,
        prepared,
        verified,
        summary,
        backup_root_identity,
    )?;
    while let Some(entry) = reader.next()? {
        let source = payload_path(&backup.join(PAYLOAD_DIRECTORY), &entry.path)?;
        let destination = payload_path(restore, &entry.path)?;
        ensure_managed_parent_directories(restore, &entry.path)?;
        resume_restored_file(&source, &destination, &entry, instrumentation)?;
    }
    reader.finish()
}

fn resume_restored_file(
    source: &Path,
    destination: &Path,
    entry: &ManifestEntry,
    instrumentation: &mut MigrationBackupInstrumentation,
) -> Result<(), MigrationBackupError> {
    require_real_file(source, "restore source is not a regular backup payload")?;
    let existing_len = match fs::symlink_metadata(destination) {
        Ok(metadata) if !metadata_is_real_file(&metadata) => {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "restore target is not a regular file",
            ))
        }
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    if existing_len > entry.description.byte_length() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "restore target is longer than its manifest entry",
        ));
    }
    let mut input = open_regular_readonly_nofollow(source)?;
    let mut existing = (existing_len != 0)
        .then(|| open_regular_readonly_nofollow(destination))
        .transpose()?;
    let mut output = if path_exists(destination)? {
        open_regular_append_nofollow(destination)?
    } else {
        create_new_regular_nofollow(destination)?
    };
    let mut remaining_existing = existing_len;
    let mut full = Sha256::new();
    let mut full_length = 0_u64;
    let mut buffer = [0_u8; IO_BUFFER_BYTES];
    instrumentation.peak_owned_payload_buffer_bytes = instrumentation
        .peak_owned_payload_buffer_bytes
        .max(buffer.len() as u64);
    for chunk in &entry.chunks {
        let mut chunk_hasher = Sha256::new();
        let mut remaining = chunk.description.byte_length();
        while remaining != 0 {
            let requested = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
                MigrationBackupError::CorruptOrConflicting("restore read length overflow")
            })?;
            input.read_exact(&mut buffer[..requested])?;
            remaining -= requested as u64;
            let compare =
                usize::try_from(remaining_existing.min(requested as u64)).map_err(|_| {
                    MigrationBackupError::CorruptOrConflicting("restore comparison length overflow")
                })?;
            if compare != 0 {
                let mut actual = [0_u8; IO_BUFFER_BYTES];
                existing
                    .as_mut()
                    .expect("restore prefix has a reader")
                    .read_exact(&mut actual[..compare])?;
                if actual[..compare] != buffer[..compare] {
                    return Err(MigrationBackupError::CorruptOrConflicting(
                        "existing restore prefix conflicts with the backup",
                    ));
                }
                remaining_existing -= compare as u64;
            }
            if compare < requested {
                write_with_partial_crash_cut(
                    &mut output,
                    &buffer[compare..requested],
                    MigrationBackupCrashCut::PartialRestoreWrite,
                )?;
                instrumentation.restored_bytes_written = checked_add(
                    instrumentation.restored_bytes_written,
                    (requested - compare) as u64,
                    "restored bytes written",
                )?;
            }
            chunk_hasher.update(&buffer[..requested]);
            full.update(&buffer[..requested]);
            full_length = checked_add(full_length, requested as u64, "restored file bytes")?;
        }
        let actual = BlobDescription::from_parts(
            chunk_hasher.finalize().into(),
            chunk.description.byte_length(),
        );
        if actual != chunk.description {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "backup chunk changed during isolated restore",
            ));
        }
    }
    if remaining_existing != 0 {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "restore target prefix exceeds restored bytes",
        ));
    }
    let mut trailing = [0_u8; 1];
    if input.read(&mut trailing)? != 0 {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "backup payload has trailing bytes during restore",
        ));
    }
    output.flush()?;
    let restored = BlobDescription::from_parts(full.finalize().into(), full_length);
    if restored != entry.description {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "restored bytes differ from their manifest entry",
        ));
    }
    Ok(())
}

fn restore_proof_bytes(
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
    manifest: BlobDescription,
    verified_inventory: InventoryProof,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(256);
    bytes.extend_from_slice(RESTORE_PROOF_MAGIC);
    put_u32(&mut bytes, RESTORE_PROOF_SCHEMA_VERSION);
    put_u32(&mut bytes, BACKUP_SCHEMA_VERSION);
    bytes.extend_from_slice(verified.workspace_id().as_uuid().as_bytes());
    bytes.extend_from_slice(verified.lineage_digest().as_bytes());
    bytes.extend_from_slice(verified.graph_resource().as_bytes());
    bytes.extend_from_slice(backup_root_identity.as_bytes());
    bytes.extend_from_slice(verified.publication_id().as_bytes());
    bytes.extend_from_slice(verified.aggregate_digest().as_bytes());
    put_description(
        &mut bytes,
        prepared.source_capture().inventory_description(),
    );
    put_description(&mut bytes, manifest);
    put_u64(&mut bytes, summary.file_count);
    put_u64(&mut bytes, summary.total_bytes);
    bytes.extend_from_slice(verified_inventory.digest.as_bytes());
    put_u64(&mut bytes, verified_inventory.file_count);
    put_u64(&mut bytes, verified_inventory.total_bytes);
    bytes
}

fn commit_marker_bytes(
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    summary: SourceSummary,
    backup_root_identity: ContentDigest,
    manifest: BlobDescription,
    restore_proof: BlobDescription,
) -> (Vec<u8>, ContentDigest) {
    let mut body = Vec::with_capacity(320);
    body.extend_from_slice(COMMIT_MARKER_MAGIC);
    put_u32(&mut body, COMMIT_MARKER_SCHEMA_VERSION);
    put_u32(&mut body, BACKUP_SCHEMA_VERSION);
    put_u32(&mut body, RESTORE_PROOF_SCHEMA_VERSION);
    body.extend_from_slice(verified.workspace_id().as_uuid().as_bytes());
    body.extend_from_slice(verified.lineage_digest().as_bytes());
    body.extend_from_slice(verified.graph_resource().as_bytes());
    body.extend_from_slice(backup_root_identity.as_bytes());
    body.extend_from_slice(verified.publication_id().as_bytes());
    body.extend_from_slice(verified.aggregate_digest().as_bytes());
    put_description(&mut body, prepared.source_capture().inventory_description());
    put_u64(&mut body, summary.file_count);
    put_u64(&mut body, summary.total_bytes);
    put_description(&mut body, manifest);
    put_description(&mut body, restore_proof);
    let mut hasher = Sha256::new();
    hasher.update(b"tine/verified-migration-source-backup/v1\0");
    hasher.update(&body);
    let evidence_digest = ContentDigest::from_bytes(hasher.finalize().into());
    body.extend_from_slice(evidence_digest.as_bytes());
    (body, evidence_digest)
}

fn publish_small_file_atomic(
    directory: &Path,
    stage_name: &str,
    final_name: &str,
    expected: &[u8],
    conflict: &'static str,
) -> Result<BlobDescription, MigrationBackupError> {
    let stage = directory.join(stage_name);
    let final_path = directory.join(final_name);
    if path_exists(&final_path)? {
        if path_exists(&stage)? {
            return Err(MigrationBackupError::CorruptOrConflicting(conflict));
        }
        compare_exact_small_file(&final_path, expected, conflict)?;
        return Ok(BlobDescription::of(expected));
    }
    let mut output = ResumableExactFile::open(&stage, conflict)?;
    let cut = match stage_name {
        RESTORE_PROOF_STAGE_FILE => MigrationBackupCrashCut::PartialRestoreProofStageWrite,
        COMMIT_MARKER_STAGE_FILE => MigrationBackupCrashCut::PartialCommitMarkerStageWrite,
        _ => {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "unknown small-file staging class",
            ))
        }
    };
    write_with_partial_crash_cut(&mut output, expected, cut).map_err(|error| match error {
        MigrationBackupError::Io(_) => MigrationBackupError::CorruptOrConflicting(conflict),
        other => other,
    })?;
    let description = output.finish()?;
    sync_file_and_parent(&stage)?;
    move_reconstructible_noreplace(&stage, &final_path)
        .map_err(|_| MigrationBackupError::CorruptOrConflicting(conflict))?;
    sync_directory(directory)?;
    Ok(description)
}

fn compare_exact_small_file(
    path: &Path,
    expected: &[u8],
    conflict: &'static str,
) -> Result<(), MigrationBackupError> {
    let mut comparator = ExactFileComparator::open(path, conflict)?;
    comparator
        .write_all(expected)
        .map_err(|_| MigrationBackupError::CorruptOrConflicting(conflict))?;
    comparator.finish()?;
    Ok(())
}

fn hash_inventory_entry(hasher: &mut Sha256, entry: &ManifestEntry) {
    hasher.update((entry.path.as_str().len() as u64).to_be_bytes());
    hasher.update(entry.path.as_str().as_bytes());
    hasher.update([match entry.kind {
        ManagedTextKind::Page => 0,
        ManagedTextKind::Journal => 1,
    }]);
    hasher.update((entry.logical_name.len() as u64).to_be_bytes());
    hasher.update(entry.logical_name.as_bytes());
    hasher.update(entry.description.sha256());
    hasher.update(entry.description.byte_length().to_be_bytes());
    hasher.update(entry.file_resource.as_bytes());
    hasher.update(entry.link_count.to_be_bytes());
    hasher.update((entry.chunks.len() as u64).to_be_bytes());
    for chunk in &entry.chunks {
        hasher.update(chunk.ordinal.to_be_bytes());
        hasher.update(chunk.description.sha256());
        hasher.update(chunk.description.byte_length().to_be_bytes());
    }
}

#[derive(Default)]
struct TreeCounts {
    files: u64,
    directories: u64,
    bytes: u64,
}

struct TreeFrame {
    path: PathBuf,
    entries: fs::ReadDir,
}

fn traverse_tree_bounded(
    root: &Path,
    summary: SourceSummary,
    sync: bool,
) -> Result<TreeCounts, MigrationBackupError> {
    require_real_directory(root, "payload tree is not a real directory")?;
    let mut counts = TreeCounts::default();
    let mut stack = vec![TreeFrame {
        path: root.to_path_buf(),
        entries: fs::read_dir(root)?,
    }];
    while !stack.is_empty() {
        let next = stack
            .last_mut()
            .expect("bounded traversal has a frame")
            .entries
            .next();
        let Some(entry) = next else {
            let completed = stack.pop().expect("bounded traversal has a frame");
            if sync {
                sync_directory(&completed.path)?;
            }
            continue;
        };
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|_| {
            MigrationBackupError::CorruptOrConflicting("payload traversal escaped its root")
        })?;
        let relative = relative
            .to_str()
            .ok_or(MigrationBackupError::CorruptOrConflicting(
                "payload path is not UTF-8",
            ))?;
        enforce_limit(
            "tree path bytes",
            relative.len() as u64,
            summary.max_path_bytes,
        )?;
        let depth = relative.split(['/', '\\']).count();
        enforce_limit("tree capture depth", depth as u64, summary.max_depth as u64)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata_is_real_directory(&metadata) {
            counts.directories = checked_add(counts.directories, 1, "tree directories")?;
            enforce_limit(
                "tree directories",
                counts.directories,
                summary.directory_count,
            )?;
            stack.push(TreeFrame {
                entries: fs::read_dir(&path)?,
                path,
            });
        } else if metadata_is_real_file(&metadata) {
            counts.files = checked_add(counts.files, 1, "tree files")?;
            enforce_limit("tree files", counts.files, summary.file_count)?;
            counts.bytes = checked_add(counts.bytes, metadata.len(), "tree bytes")?;
            enforce_limit("tree bytes", counts.bytes, summary.total_bytes)?;
            if sync {
                let file = open_regular_for_sync(&path)?;
                crate::filesystem_durability::sync_reconstructible_file(&file)?;
            }
        } else {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "payload tree contains a symlink, reparse point, or special entry",
            ));
        }
    }
    Ok(counts)
}

fn count_tree(root: &Path, summary: SourceSummary) -> Result<TreeCounts, MigrationBackupError> {
    traverse_tree_bounded(root, summary, false)
}

fn validate_backup_root_entries_for_state(
    directory: &Path,
    final_directory: bool,
) -> Result<(), MigrationBackupError> {
    let mut payload = false;
    let mut manifest = false;
    let mut proof = false;
    let mut proof_stage = false;
    let mut marker = false;
    let mut marker_stage = false;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let metadata = fs::symlink_metadata(entry.path())?;
        match name.to_str() {
            Some(PAYLOAD_DIRECTORY) if metadata_is_real_directory(&metadata) && !payload => {
                payload = true
            }
            Some(MANIFEST_FILE) if metadata_is_real_file(&metadata) && !manifest => manifest = true,
            Some(RESTORE_PROOF_FILE)
                if final_directory && metadata_is_real_file(&metadata) && !proof =>
            {
                proof = true
            }
            Some(RESTORE_PROOF_STAGE_FILE)
                if final_directory && metadata_is_real_file(&metadata) && !proof_stage =>
            {
                proof_stage = true
            }
            Some(COMMIT_MARKER_FILE)
                if final_directory && metadata_is_real_file(&metadata) && !marker =>
            {
                marker = true
            }
            Some(COMMIT_MARKER_STAGE_FILE)
                if final_directory && metadata_is_real_file(&metadata) && !marker_stage =>
            {
                marker_stage = true
            }
            _ => {
                return Err(MigrationBackupError::CorruptOrConflicting(
                    "backup directory contains an extra or malformed entry",
                ))
            }
        }
    }
    if !payload
        || !manifest
        || (marker && !proof)
        || (proof && proof_stage)
        || (marker && marker_stage)
    {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "backup directory is missing required entries",
        ));
    }
    Ok(())
}

fn validate_final_root_entries(directory: &Path) -> Result<(), MigrationBackupError> {
    validate_backup_root_entries_for_state(directory, true)?;
    if !path_exists(&directory.join(RESTORE_PROOF_FILE))?
        || !path_exists(&directory.join(COMMIT_MARKER_FILE))?
        || path_exists(&directory.join(RESTORE_PROOF_STAGE_FILE))?
        || path_exists(&directory.join(COMMIT_MARKER_STAGE_FILE))?
    {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "committed backup is missing its proof or marker",
        ));
    }
    Ok(())
}

fn require_capability_description(
    directory: &Dir,
    name: &str,
    expected: BlobDescription,
) -> Result<(), MigrationBackupError> {
    let mut file = open_file_nofollow(directory, name)?;
    let metadata = file.metadata()?;
    if !metadata_is_real_file(&metadata) || metadata.len() != expected.byte_length() {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "retained source backup evidence has the wrong shape or length",
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; IO_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    if BlobDescription::from_parts(digest.finalize().into(), metadata.len()) != expected {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "retained source backup evidence digest changed",
        ));
    }
    Ok(())
}

fn read_capability_file(
    directory: &Dir,
    name: &str,
    maximum: u64,
) -> Result<Vec<u8>, MigrationBackupError> {
    let mut file = open_file_nofollow(directory, name)?;
    let metadata = file.metadata()?;
    if !metadata_is_real_file(&metadata) || metadata.len() > maximum {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "retained source backup evidence has the wrong shape or size",
        ));
    }
    let capacity =
        usize::try_from(metadata.len()).map_err(|_| MigrationBackupError::ResourceLimit {
            resource: "retained source backup evidence allocation",
            observed: metadata.len(),
            limit: maximum,
        })?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| MigrationBackupError::ResourceLimit {
            resource: "retained source backup evidence allocation",
            observed: metadata.len(),
            limit: maximum,
        })?;
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn validate_retained_managed_path(path: &ManagedPath) -> Result<(), MigrationBackupError> {
    if path.as_str().len() > BOOTSTRAP_SOURCE_MAX_PATH_BYTES {
        return Err(MigrationBackupError::ResourceLimit {
            resource: "managed path bytes",
            observed: path.as_str().len() as u64,
            limit: BOOTSTRAP_SOURCE_MAX_PATH_BYTES as u64,
        });
    }
    let depth = path.as_str().split('/').count() as u64;
    let maximum_depth = BOOTSTRAP_SOURCE_MAX_DIRECTORY_DEPTH.saturating_add(1) as u64;
    if depth > maximum_depth {
        return Err(MigrationBackupError::ResourceLimit {
            resource: "managed path depth",
            observed: depth,
            limit: maximum_depth,
        });
    }
    Ok(())
}

fn payload_path(root: &Path, path: &ManagedPath) -> Result<PathBuf, MigrationBackupError> {
    if path.as_str().len() > BOOTSTRAP_SOURCE_MAX_PATH_BYTES {
        return Err(MigrationBackupError::ResourceLimit {
            resource: "managed path bytes",
            observed: path.as_str().len() as u64,
            limit: BOOTSTRAP_SOURCE_MAX_PATH_BYTES as u64,
        });
    }
    let joined = root.join(path.as_str());
    if !joined.starts_with(root) {
        return Err(MigrationBackupError::CorruptOrConflicting(
            "managed path escapes its payload root",
        ));
    }
    Ok(joined)
}

fn ensure_managed_parent_directories(
    root: &Path,
    path: &ManagedPath,
) -> Result<(), MigrationBackupError> {
    let mut current = root.to_path_buf();
    let component_count = path.as_str().split('/').count();
    for component in path
        .as_str()
        .split('/')
        .take(component_count.saturating_sub(1))
    {
        current.push(component);
        ensure_real_directory_created(&current)?;
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_windows_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn metadata_is_windows_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

fn metadata_is_real_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && !metadata_is_windows_reparse(metadata)
}

fn metadata_is_real_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && !metadata.file_type().is_symlink()
        && !metadata_is_windows_reparse(metadata)
}

fn require_supported_exact_filesystem() -> io::Result<()> {
    #[cfg(any(unix, windows))]
    {
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "durable no-follow migration backups are unsupported on this platform",
        ))
    }
}

fn configure_file_nofollow(options: &mut OpenOptions) {
    #[cfg(all(unix, not(target_os = "android")))]
    {
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(any(target_os = "android", not(any(unix, windows))))]
    let _ = options;
}

fn validate_opened_regular(file: File) -> io::Result<File> {
    if metadata_is_real_file(&file.metadata()?) {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "opened migration backup path is not a regular no-follow file",
        ))
    }
}

fn open_regular_readonly_nofollow(path: &Path) -> io::Result<File> {
    require_supported_exact_filesystem()?;
    let mut options = OpenOptions::new();
    options.read(true);
    configure_file_nofollow(&mut options);
    validate_opened_regular(options.open(path)?)
}

fn open_regular_append_nofollow(path: &Path) -> io::Result<File> {
    require_supported_exact_filesystem()?;
    let mut options = OpenOptions::new();
    options.append(true);
    configure_file_nofollow(&mut options);
    validate_opened_regular(options.open(path)?)
}

fn create_new_regular_nofollow(path: &Path) -> io::Result<File> {
    require_supported_exact_filesystem()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_file_nofollow(&mut options);
    validate_opened_regular(options.open(path)?)
}

fn open_regular_for_sync(path: &Path) -> io::Result<File> {
    require_supported_exact_filesystem()?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.write(true);
    configure_file_nofollow(&mut options);
    validate_opened_regular(options.open(path)?)
}

fn open_directory_nofollow_ambient(path: &Path) -> Result<Dir, MigrationBackupError> {
    require_supported_exact_filesystem()?;
    #[cfg(target_os = "android")]
    {
        require_real_directory(path, "migration backup path is not a real directory")?;
        let file = File::open(path)?;
        if !file.metadata()?.is_dir() {
            return Err(MigrationBackupError::CorruptOrConflicting(
                "opened migration backup handle is not a directory",
            ));
        }
        return Ok(Dir::from_std_file(file));
    }

    #[cfg(not(target_os = "android"))]
    {
        let name = path.file_name().and_then(|name| name.to_str()).ok_or(
            MigrationBackupError::InvalidRoot("directory path has no UTF-8 leaf"),
        )?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = Dir::open_ambient_dir(parent, ambient_authority())?;
        open_dir_nofollow(&parent, name)
            .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))
    }
}

fn require_real_directory(path: &Path, detail: &'static str) -> Result<(), MigrationBackupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_real_directory(&metadata) => Ok(()),
        Ok(_) => Err(MigrationBackupError::CorruptOrConflicting(detail)),
        Err(error) => Err(error.into()),
    }
}

fn require_real_file(path: &Path, detail: &'static str) -> Result<(), MigrationBackupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_real_file(&metadata) => Ok(()),
        Ok(_) => Err(MigrationBackupError::CorruptOrConflicting(detail)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(MigrationBackupError::CorruptOrConflicting(detail))
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_real_directory_created(path: &Path) -> Result<(), MigrationBackupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_real_directory(&metadata) => Ok(()),
        Ok(_) => Err(MigrationBackupError::CorruptOrConflicting(
            "expected directory is not a real directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or(MigrationBackupError::InvalidRoot("directory has no parent"))?;
            fs::create_dir(path)?;
            sync_directory(parent)
        }
        Err(error) => Err(error.into()),
    }
}

fn path_exists(path: &Path) -> Result<bool, MigrationBackupError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn sync_file_and_parent(path: &Path) -> Result<(), MigrationBackupError> {
    let file = open_regular_for_sync(path)?;
    crate::filesystem_durability::sync_reconstructible_file(&file)?;
    sync_directory(
        path.parent()
            .ok_or(MigrationBackupError::InvalidRoot("file has no parent"))?,
    )
}

fn move_reconstructible_noreplace(src: &Path, dest: &Path) -> io::Result<()> {
    match move_file_noreplace(src, dest) {
        Ok(()) => Ok(()),
        #[cfg(target_os = "android")]
        Err(error)
            if crate::filesystem_durability::android_durability_capability_refusal(
                error.kind(),
            ) =>
        {
            // This tree is app-private, single-writer, and still disposable.
            // Preserve the semantic no-clobber check, then use Android's
            // ordinary rename when renameat2 is not exposed by the filesystem.
            match fs::symlink_metadata(dest) {
                Ok(_) => {
                    return Err(io::Error::new(
                        ErrorKind::AlreadyExists,
                        "reconstructible backup destination already exists",
                    ))
                }
                Err(missing) if missing.kind() == ErrorKind::NotFound => {}
                Err(other) => return Err(other),
            }
            fs::rename(src, dest)
        }
        Err(error) => Err(error),
    }
}

fn sync_directory(path: &Path) -> Result<(), MigrationBackupError> {
    let directory = open_directory_nofollow_ambient(path)?;
    crate::filesystem_durability::sync_reconstructible_directory(&directory)
        .map_err(|error| MigrationBackupError::Io(io::Error::other(error.to_string())))
}

fn sync_tree(path: &Path, summary: SourceSummary) -> Result<(), MigrationBackupError> {
    let _ = traverse_tree_bounded(path, summary, false)?;
    crate::filesystem_durability::sync_private_tree(path)?;
    Ok(())
}

fn read_bounded_bytes(
    reader: &mut impl Read,
    limit: usize,
    field: &'static str,
) -> Result<Vec<u8>, MigrationBackupError> {
    let length = read_u32(reader)? as usize;
    if length > limit {
        return Err(MigrationBackupError::ResourceLimit {
            resource: field,
            observed: length as u64,
            limit: limit as u64,
        });
    }
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes).map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("manifest string field is truncated")
    })?;
    Ok(bytes)
}

fn read_u8(reader: &mut impl Read) -> Result<u8, MigrationBackupError> {
    let mut bytes = [0_u8; 1];
    reader.read_exact(&mut bytes).map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("manifest integer field is truncated")
    })?;
    Ok(bytes[0])
}

fn read_u32(reader: &mut impl Read) -> Result<u32, MigrationBackupError> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes).map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("manifest integer field is truncated")
    })?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, MigrationBackupError> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes).map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("manifest integer field is truncated")
    })?;
    Ok(u64::from_be_bytes(bytes))
}

fn read_array_32(reader: &mut impl Read) -> Result<[u8; 32], MigrationBackupError> {
    let mut bytes = [0_u8; 32];
    reader.read_exact(&mut bytes).map_err(|_| {
        MigrationBackupError::CorruptOrConflicting("manifest digest field is truncated")
    })?;
    Ok(bytes)
}

fn read_description(reader: &mut impl Read) -> Result<BlobDescription, MigrationBackupError> {
    let digest = read_array_32(reader)?;
    let length = read_u64(reader)?;
    Ok(BlobDescription::from_parts(digest, length))
}

fn hex(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use super::*;
    use crate::model::Graph;
    use crate::oplog::hot_engine::{ProjectionEndpointBinding, ProjectionStorageBinding};
    use crate::oplog::import::{
        prepare_inactive_bootstrap_import, publish_install_verify_inactive_bootstrap,
    };
    use crate::oplog::{
        DeviceId, DocumentId, LineageDigest, ObjectStore, ProjectionEndpointId,
        ProjectionReceiptStoreId, ReferenceCatalogPolicyV1,
    };

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-migration-backup-{label}-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        root: TestRoot,
        graph_root: PathBuf,
        prepared: InactiveBootstrapPreparedPublication,
        verified: InactiveBootstrapVerifiedPublication,
        original_files: BTreeMap<String, Vec<u8>>,
    }

    impl Fixture {
        fn rich(label: &str, workspace_value: u128) -> Self {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            fs::create_dir(&graph_root).unwrap();
            fs::create_dir(graph_root.join("logseq")).unwrap();
            fs::write(
                graph_root.join("logseq/config.edn"),
                br#"{:pages-directory "notes"
                    :journals-directory "diary"
                    :file/name-format :triple-lowbar}"#,
            )
            .unwrap();
            let mut original_files = BTreeMap::new();
            let mut insert = |path: &str, bytes: Vec<u8>| {
                let target = graph_root.join(path);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(&target, &bytes).unwrap();
                original_files.insert(path.to_owned(), bytes);
            };
            insert("Root.md", b"title:: Root\r\n\r\n- CRLF\r\n".to_vec());
            insert(
                "notes/\u{6df1}\u{3044}/D\u{e9}j\u{e0}___\u{8a08}\u{753b}.markdown",
                "\u{feff}- Unicode caf\u{e9}\r\n".as_bytes().to_vec(),
            );
            insert("diary/nonstandard/empty.org", Vec::new());
            let mut large = b"title:: Multi chunk\r\n\r\n- ".to_vec();
            large.extend(std::iter::repeat_n(
                b'x',
                BOOTSTRAP_SOURCE_CHUNK_BYTES + 137,
            ));
            large.extend_from_slice(b"\r\n");
            insert("notes/nested/multi.md", large);

            let graph = Graph::open(&graph_root);
            let capture_scratch = root.path().join("capture");
            let preparation_scratch = root.path().join("preparation");
            fs::create_dir(&capture_scratch).unwrap();
            fs::create_dir(&preparation_scratch).unwrap();
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(workspace_value));
            let prepared = prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"migration-source-backup-test-lineage"),
                DocumentId::from_uuid(Uuid::from_u128(workspace_value + 1)),
                ReferenceCatalogPolicyV1::default(),
                &ObjectStore::open(&root.path().join("archive"), workspace)
                    .unwrap()
                    .bootstrap_authoring_capability()
                    .unwrap(),
                &preparation_scratch,
            )
            .unwrap();
            let binding = ProjectionStorageBinding {
                endpoint: ProjectionEndpointBinding {
                    endpoint_id: ProjectionEndpointId::from_uuid(Uuid::from_u128(
                        workspace_value + 2,
                    )),
                    device_id: DeviceId::from_uuid(Uuid::from_u128(workspace_value + 3)),
                    graph_resource_id: prepared.aggregate().graph_resource(),
                },
                receipt_store_id: ProjectionReceiptStoreId::from_capability_identity(
                    b"migration-source-backup-test",
                    &(workspace_value + 4).to_be_bytes(),
                ),
            };
            let verified = publish_install_verify_inactive_bootstrap(
                &prepared,
                ObjectStore::open(&root.path().join("archive"), workspace).unwrap(),
                binding,
            )
            .unwrap();
            Self {
                root,
                graph_root,
                prepared,
                verified,
                original_files,
            }
        }

        fn backup_root(&self, name: &str) -> PathBuf {
            let path = self.root.path().join(name);
            fs::create_dir(&path).unwrap();
            path
        }

        fn roots(&self, name: &str) -> MigrationBackupRoot {
            MigrationBackupRoot::open_for_harness(&self.backup_root(name), &self.graph_root)
                .unwrap()
        }
    }

    fn snapshot_tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        fn walk(root: &Path, current: &Path, output: &mut BTreeMap<String, Option<Vec<u8>>>) {
            let mut entries = fs::read_dir(current)
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if entry.file_type().unwrap().is_dir() {
                    output.insert(format!("{relative}/"), None);
                    walk(root, &entry.path(), output);
                } else {
                    output.insert(relative, Some(fs::read(entry.path()).unwrap()));
                }
            }
        }
        let mut output = BTreeMap::new();
        walk(root, root, &mut output);
        output
    }

    fn assert_roundtrip_files(root: &Path, expected: &BTreeMap<String, Vec<u8>>) {
        for (path, bytes) in expected {
            assert_eq!(fs::read(root.join(path)).unwrap(), *bytes, "{path}");
        }
    }

    fn reset_packet(roots: &MigrationBackupRoot) {
        let path = roots.canonical_root.join(BACKUP_ROOT_DIRECTORY);
        if path.exists() {
            crate::test_support::remove_dir_all(path);
        }
    }

    fn first_file_below(root: &Path) -> PathBuf {
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            let mut entries = fs::read_dir(directory)
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                } else {
                    return entry.path();
                }
            }
        }
        panic!("partial artifact did not leave a file")
    }

    fn partial_artifact_path(paths: &PublicationPaths, cut: MigrationBackupCrashCut) -> PathBuf {
        match cut {
            MigrationBackupCrashCut::PartialPayloadWrite => {
                first_file_below(&paths.stage.join(PAYLOAD_DIRECTORY))
            }
            MigrationBackupCrashCut::PartialManifestHeaderWrite
            | MigrationBackupCrashCut::PartialManifestEntryWrite => paths.stage.join(MANIFEST_FILE),
            MigrationBackupCrashCut::PartialRestoreProofStageWrite => {
                paths.final_directory.join(RESTORE_PROOF_STAGE_FILE)
            }
            MigrationBackupCrashCut::PartialCommitMarkerStageWrite => {
                paths.final_directory.join(COMMIT_MARKER_STAGE_FILE)
            }
            _ => panic!("phase-boundary cut has no partial artifact"),
        }
    }

    fn expected_partial_artifact_bytes(
        fixture: &Fixture,
        roots: &MigrationBackupRoot,
        paths: &PublicationPaths,
        cut: MigrationBackupCrashCut,
    ) -> Vec<u8> {
        let summary = summarize_source(fixture.prepared.source_capture()).unwrap();
        match cut {
            MigrationBackupCrashCut::PartialPayloadWrite => {
                let path = partial_artifact_path(paths, cut);
                let relative = path
                    .strip_prefix(paths.stage.join(PAYLOAD_DIRECTORY))
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                fixture.original_files[&relative].clone()
            }
            MigrationBackupCrashCut::PartialManifestHeaderWrite
            | MigrationBackupCrashCut::PartialManifestEntryWrite => {
                let mut expected = Vec::new();
                emit_manifest(
                    &mut expected,
                    &fixture.prepared,
                    &fixture.verified,
                    summary,
                    roots.backup_root_identity,
                )
                .unwrap();
                expected
            }
            MigrationBackupCrashCut::PartialRestoreProofStageWrite => {
                let manifest = compare_expected_manifest(
                    &paths.final_directory.join(MANIFEST_FILE),
                    &fixture.prepared,
                    &fixture.verified,
                    summary,
                    roots.backup_root_identity,
                )
                .unwrap();
                let mut instrumentation = MigrationBackupInstrumentation::default();
                let restored = verify_tree_from_manifest(
                    &paths.final_directory.join(MANIFEST_FILE),
                    &paths.final_directory.join(PAYLOAD_DIRECTORY),
                    &fixture.prepared,
                    &fixture.verified,
                    summary,
                    roots.backup_root_identity,
                    &mut instrumentation,
                    TreeReadKind::Backup,
                )
                .unwrap();
                restore_proof_bytes(
                    &fixture.prepared,
                    &fixture.verified,
                    summary,
                    roots.backup_root_identity,
                    manifest,
                    restored,
                )
            }
            MigrationBackupCrashCut::PartialCommitMarkerStageWrite => {
                let manifest = compare_expected_manifest(
                    &paths.final_directory.join(MANIFEST_FILE),
                    &fixture.prepared,
                    &fixture.verified,
                    summary,
                    roots.backup_root_identity,
                )
                .unwrap();
                let proof = BlobDescription::of(
                    &fs::read(paths.final_directory.join(RESTORE_PROOF_FILE)).unwrap(),
                );
                commit_marker_bytes(
                    &fixture.prepared,
                    &fixture.verified,
                    summary,
                    roots.backup_root_identity,
                    manifest,
                    proof,
                )
                .0
            }
            _ => panic!("phase-boundary cut has no partial artifact"),
        }
    }

    #[test]
    fn healthy_backup_verification_avoids_restore_rehearsal_and_is_bounded() {
        let fixture = Fixture::rich("roundtrip", 0x7100);
        let before = snapshot_tree(&fixture.graph_root);
        let roots = fixture.roots("backups");
        let backup =
            verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified).unwrap();
        assert_eq!(backup.file_count(), fixture.original_files.len() as u64);
        assert!(backup.total_bytes() > BOOTSTRAP_SOURCE_CHUNK_BYTES as u64);
        assert_eq!(backup.workspace_id(), fixture.verified.workspace_id());
        assert_eq!(backup.graph_resource(), fixture.verified.graph_resource());
        assert_eq!(
            backup.publication_id(),
            fixture.verified.publication_id().as_bytes()
        );
        assert_eq!(
            backup.aggregate_digest(),
            fixture.verified.aggregate_digest().as_bytes()
        );
        assert_eq!(
            backup.source_inventory(),
            fixture.prepared.source_capture().inventory_description()
        );
        assert_ne!(backup.manifest().byte_length(), 0);
        assert_ne!(backup.restore_proof().byte_length(), 0);
        assert_ne!(
            backup.evidence_digest(),
            ContentDigest::of(b"caller-supplied-proof")
        );
        assert!(backup.instrumentation().peak_owned_payload_buffer_bytes <= IO_BUFFER_BYTES as u64);
        assert_eq!(backup.instrumentation().restored_bytes_written, 0);
        assert_eq!(backup.instrumentation().restored_bytes_read, 0);
        assert_roundtrip_files(
            &backup.directory().join(PAYLOAD_DIRECTORY),
            &fixture.original_files,
        );
        let summary = summarize_source(fixture.prepared.source_capture()).unwrap();
        let mut proof_instrumentation = MigrationBackupInstrumentation::default();
        let verified_inventory = verify_tree_from_manifest(
            &backup.directory().join(MANIFEST_FILE),
            &backup.directory().join(PAYLOAD_DIRECTORY),
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            &mut proof_instrumentation,
            TreeReadKind::Backup,
        )
        .unwrap();
        let expected_proof = restore_proof_bytes(
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            backup.manifest(),
            verified_inventory,
        );
        assert_eq!(
            fs::read(backup.directory().join(RESTORE_PROOF_FILE)).unwrap(),
            expected_proof
        );
        assert!(!roots
            .canonical_root
            .join("migration-source-restore-scratch-v1")
            .exists());
        assert_eq!(snapshot_tree(&fixture.graph_root), before);
    }

    #[test]
    fn retained_recovery_resumes_and_restores_valid_backup_exactly() {
        let fixture = Fixture::rich("requested-recovery", 0x7150);
        let before = snapshot_tree(&fixture.graph_root);
        let roots = fixture.roots("backups");
        let backup =
            verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified).unwrap();
        let summary = summarize_source(fixture.prepared.source_capture()).unwrap();
        let restore = fixture.root.path().join("requested-recovery");
        fs::create_dir(&restore).unwrap();

        let mut partial_instrumentation = MigrationBackupInstrumentation::default();
        MIGRATION_BACKUP_CRASH_CUT
            .with(|pending| pending.set(Some(MigrationBackupCrashCut::PartialRestoreWrite)));
        assert!(matches!(
            restore_from_manifest(
                backup.directory(),
                &restore,
                &fixture.prepared,
                &fixture.verified,
                summary,
                roots.backup_root_identity,
                &mut partial_instrumentation,
            ),
            Err(MigrationBackupError::InjectedCrashCut(
                "partial_restore_write"
            ))
        ));
        let partial = first_file_below(&restore);
        let relative = partial
            .strip_prefix(&restore)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let prefix = fs::read(&partial).unwrap();
        assert!(!prefix.is_empty());
        assert!(prefix.len() < fixture.original_files[&relative].len());
        assert_eq!(prefix, fixture.original_files[&relative][..prefix.len()]);

        let mut recovery_instrumentation = MigrationBackupInstrumentation::default();
        restore_from_manifest(
            backup.directory(),
            &restore,
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            &mut recovery_instrumentation,
        )
        .unwrap();
        sync_tree(&restore, summary).unwrap();
        let mut backup_instrumentation = MigrationBackupInstrumentation::default();
        let expected_inventory = verify_tree_from_manifest(
            &backup.directory().join(MANIFEST_FILE),
            &backup.directory().join(PAYLOAD_DIRECTORY),
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            &mut backup_instrumentation,
            TreeReadKind::Backup,
        )
        .unwrap();
        let recovered_inventory = verify_tree_from_manifest(
            &backup.directory().join(MANIFEST_FILE),
            &restore,
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            &mut recovery_instrumentation,
            TreeReadKind::Restore,
        )
        .unwrap();
        assert_eq!(recovered_inventory, expected_inventory);
        assert_roundtrip_files(&restore, &fixture.original_files);

        let payload = backup.directory().join(PAYLOAD_DIRECTORY);
        let source = payload.join("Root.md");
        let source_bytes = fs::read(&source).unwrap();
        let mut corrupt_bytes = source_bytes.clone();
        corrupt_bytes[0] ^= 0x01;
        fs::write(&source, corrupt_bytes).unwrap();
        let corrupt_restore = fixture.root.path().join("corrupt-recovery");
        fs::create_dir(&corrupt_restore).unwrap();
        assert!(restore_from_manifest(
            backup.directory(),
            &corrupt_restore,
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            &mut MigrationBackupInstrumentation::default(),
        )
        .is_err());
        fs::write(&source, &source_bytes).unwrap();

        let missing = payload.join("Root.missing");
        fs::rename(&source, &missing).unwrap();
        let incomplete_restore = fixture.root.path().join("incomplete-recovery");
        fs::create_dir(&incomplete_restore).unwrap();
        assert!(restore_from_manifest(
            backup.directory(),
            &incomplete_restore,
            &fixture.prepared,
            &fixture.verified,
            summary,
            roots.backup_root_identity,
            &mut MigrationBackupInstrumentation::default(),
        )
        .is_err());
        fs::rename(&missing, &source).unwrap();
        assert_eq!(snapshot_tree(&fixture.graph_root), before);
    }

    #[test]
    fn deterministic_evidence_and_idempotent_fresh_reopen() {
        let fixture = Fixture::rich("idempotent", 0x7200);
        let roots = fixture.roots("backups");
        let first =
            verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified).unwrap();
        let second =
            verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.directory(), second.directory());
        assert_eq!(first.manifest(), second.manifest());
        assert_eq!(first.restore_proof(), second.restore_proof());
        assert_eq!(first.evidence_digest(), second.evidence_digest());
    }

    #[test]
    fn physical_backup_root_identity_changes_all_transitive_evidence() {
        let fixture = Fixture::rich("root-identity", 0x7250);
        let first_roots = fixture.roots("backups-a");
        let second_roots = fixture.roots("backups-b");
        let first =
            verify_migration_source_backup(&first_roots, &fixture.prepared, &fixture.verified)
                .unwrap();
        let second =
            verify_migration_source_backup(&second_roots, &fixture.prepared, &fixture.verified)
                .unwrap();
        assert_ne!(first.backup_root_identity(), second.backup_root_identity());
        assert_ne!(first.manifest(), second.manifest());
        assert_ne!(first.restore_proof(), second.restore_proof());
        assert_ne!(first.evidence_digest(), second.evidence_digest());
        assert_ne!(first, second);
    }

    #[test]
    fn mismatched_prepared_and_verified_publications_are_rejected() {
        let first = Fixture::rich("wrong-binding-a", 0x7300);
        let second = Fixture::rich("wrong-binding-b", 0x7400);
        let roots = second.roots("backups");
        assert!(matches!(
            verify_migration_source_backup(&roots, &second.prepared, &first.verified),
            Err(MigrationBackupError::BindingMismatch(_))
        ));
        assert!(!roots.canonical_root.join(BACKUP_ROOT_DIRECTORY).exists());
        assert!(matches!(
            MigrationBackupRoot::open_for_harness(&second.graph_root, &second.graph_root),
            Err(MigrationBackupError::InvalidRoot(_))
        ));
        assert!(matches!(
            MigrationBackupRoot::open_for_harness(second.root.path(), &second.graph_root),
            Err(MigrationBackupError::InvalidRoot(_))
        ));
    }

    #[test]
    fn corrupt_missing_extra_reordered_and_duplicate_evidence_fails_closed() {
        let fixture = Fixture::rich("corruption", 0x7500);
        let roots = fixture.roots("backups");
        let backup =
            verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified).unwrap();
        let directory = backup.directory().to_path_buf();
        let manifest_path = directory.join(MANIFEST_FILE);
        let manifest = fs::read(&manifest_path).unwrap();
        let header_len = manifest_header(
            &fixture.prepared,
            &fixture.verified,
            summarize_source(fixture.prepared.source_capture()).unwrap(),
            roots.backup_root_identity,
        )
        .unwrap()
        .len();
        let ranges = manifest_record_ranges(
            &manifest,
            header_len,
            fixture.prepared.source_capture().source_file_count(),
        );
        assert!(ranges.len() >= 2);

        let rejection_index = std::cell::Cell::new(0_usize);
        let assert_rejected = || {
            let index = rejection_index.get();
            rejection_index.set(index + 1);
            let result =
                verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified);
            assert!(
                matches!(
                    result,
                    Err(MigrationBackupError::CorruptOrConflicting(_)
                        | MigrationBackupError::ResourceLimit { .. })
                ),
                "corruption case {index} returned {result:?}"
            );
        };

        let missing_manifest = directory.join("manifest.missing");
        fs::rename(&manifest_path, &missing_manifest).unwrap();
        assert_rejected();
        fs::rename(&missing_manifest, &manifest_path).unwrap();

        fs::write(&manifest_path, &manifest[..manifest.len() - 1]).unwrap();
        assert_rejected();
        fs::write(&manifest_path, &manifest).unwrap();

        let mut modified_manifest = manifest.clone();
        modified_manifest[header_len + 5] ^= 0x01;
        fs::write(&manifest_path, &modified_manifest).unwrap();
        assert_rejected();
        fs::write(&manifest_path, &manifest).unwrap();

        let mut extra_manifest = manifest.clone();
        extra_manifest.push(0xff);
        fs::write(&manifest_path, &extra_manifest).unwrap();
        assert_rejected();
        fs::write(&manifest_path, &manifest).unwrap();

        let mut reordered = manifest[..header_len].to_vec();
        reordered.extend_from_slice(&manifest[ranges[1].clone()]);
        reordered.extend_from_slice(&manifest[ranges[0].clone()]);
        reordered.extend_from_slice(&manifest[ranges[1].end..]);
        fs::write(&manifest_path, &reordered).unwrap();
        assert_rejected();
        fs::write(&manifest_path, &manifest).unwrap();

        let mut duplicate = manifest[..header_len].to_vec();
        duplicate.extend_from_slice(&manifest[ranges[0].clone()]);
        duplicate.extend_from_slice(&manifest[ranges[0].clone()]);
        duplicate.extend_from_slice(&manifest[ranges[1].end..]);
        fs::write(&manifest_path, &duplicate).unwrap();
        assert_rejected();
        fs::write(&manifest_path, &manifest).unwrap();

        let payload = directory.join(PAYLOAD_DIRECTORY);
        let target = payload.join("Root.md");
        let target_bytes = fs::read(&target).unwrap();
        let missing_payload = payload.join("Root.missing");
        fs::rename(&target, &missing_payload).unwrap();
        assert_rejected();
        fs::rename(&missing_payload, &target).unwrap();

        OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap()
            .set_len(target_bytes.len() as u64 - 1)
            .unwrap();
        assert_rejected();
        fs::write(&target, &target_bytes).unwrap();

        let mut modified_payload = target_bytes.clone();
        modified_payload[0] ^= 1;
        fs::write(&target, &modified_payload).unwrap();
        assert_rejected();
        fs::write(&target, &target_bytes).unwrap();

        fs::write(payload.join("extra.md"), b"extra").unwrap();
        assert_rejected();
    }

    #[test]
    fn every_crash_cut_retries_to_identical_evidence_without_graph_writes() {
        let fixture = Fixture::rich("crash-cuts", 0x7600);
        let before = snapshot_tree(&fixture.graph_root);
        let cuts = [
            MigrationBackupCrashCut::AfterStagingCreation,
            MigrationBackupCrashCut::AfterPayloadWrite,
            MigrationBackupCrashCut::AfterPayloadSync,
            MigrationBackupCrashCut::AfterManifestPublication,
            MigrationBackupCrashCut::AfterStagingSync,
            MigrationBackupCrashCut::AfterStagingRename,
            MigrationBackupCrashCut::AfterCommittedBackupReopen,
            MigrationBackupCrashCut::AfterProofPublication,
            MigrationBackupCrashCut::AfterCommitMarkerPublication,
        ];
        for (index, cut) in cuts.into_iter().enumerate() {
            let roots = fixture.roots(&format!("backup-cut-{index}"));
            let uninterrupted =
                verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified)
                    .unwrap();
            reset_packet(&roots);
            MIGRATION_BACKUP_CRASH_CUT.with(|pending| pending.set(Some(cut)));
            assert!(matches!(
                verify_migration_source_backup(
                    &roots,
                    &fixture.prepared,
                    &fixture.verified
                ),
                Err(MigrationBackupError::InjectedCrashCut(label)) if label == cut.label()
            ));
            let retried =
                verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified)
                    .unwrap();
            assert!(
                retried.instrumentation().peak_owned_payload_buffer_bytes <= IO_BUFFER_BYTES as u64
            );
            assert_eq!(uninterrupted, retried);
            assert_eq!(snapshot_tree(&fixture.graph_root), before);
        }
    }

    #[test]
    fn every_partial_write_cut_resumes_exact_prefix_to_uninterrupted_evidence() {
        let fixture = Fixture::rich("partial-cuts", 0x7650);
        let before = snapshot_tree(&fixture.graph_root);
        let cuts = [
            MigrationBackupCrashCut::PartialPayloadWrite,
            MigrationBackupCrashCut::PartialManifestHeaderWrite,
            MigrationBackupCrashCut::PartialManifestEntryWrite,
            MigrationBackupCrashCut::PartialRestoreProofStageWrite,
            MigrationBackupCrashCut::PartialCommitMarkerStageWrite,
        ];
        for (index, cut) in cuts.into_iter().enumerate() {
            let roots = fixture.roots(&format!("partial-cut-{index}"));
            let uninterrupted =
                verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified)
                    .unwrap();
            reset_packet(&roots);
            let paths = publication_paths(&roots, &fixture.verified).unwrap();
            MIGRATION_BACKUP_CRASH_CUT.with(|pending| pending.set(Some(cut)));
            assert!(matches!(
                verify_migration_source_backup(
                    &roots,
                    &fixture.prepared,
                    &fixture.verified
                ),
                Err(MigrationBackupError::InjectedCrashCut(label)) if label == cut.label()
            ));
            let artifact = partial_artifact_path(&paths, cut);
            let prefix = fs::read(&artifact).unwrap();
            let expected = expected_partial_artifact_bytes(&fixture, &roots, &paths, cut);
            assert!(!prefix.is_empty(), "{cut:?}");
            assert!(prefix.len() < expected.len(), "{cut:?}");
            assert_eq!(prefix, expected[..prefix.len()], "{cut:?}");
            let retried =
                verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified)
                    .unwrap();
            assert_eq!(uninterrupted, retried, "{cut:?}");
            assert_eq!(snapshot_tree(&fixture.graph_root), before);
        }
    }

    #[test]
    fn every_partial_write_artifact_rejects_a_conflicting_prefix() {
        let fixture = Fixture::rich("partial-conflicts", 0x7660);
        let before = snapshot_tree(&fixture.graph_root);
        let cuts = [
            MigrationBackupCrashCut::PartialPayloadWrite,
            MigrationBackupCrashCut::PartialManifestHeaderWrite,
            MigrationBackupCrashCut::PartialManifestEntryWrite,
            MigrationBackupCrashCut::PartialRestoreProofStageWrite,
            MigrationBackupCrashCut::PartialCommitMarkerStageWrite,
        ];
        for (index, cut) in cuts.into_iter().enumerate() {
            let roots = fixture.roots(&format!("partial-conflict-{index}"));
            let paths = publication_paths(&roots, &fixture.verified).unwrap();
            MIGRATION_BACKUP_CRASH_CUT.with(|pending| pending.set(Some(cut)));
            assert!(matches!(
                verify_migration_source_backup(
                    &roots,
                    &fixture.prepared,
                    &fixture.verified
                ),
                Err(MigrationBackupError::InjectedCrashCut(label)) if label == cut.label()
            ));
            let artifact = partial_artifact_path(&paths, cut);
            let mut conflicting = fs::read(&artifact).unwrap();
            let last = conflicting
                .last_mut()
                .expect("partial write leaves a nonzero prefix");
            *last ^= 0x01;
            fs::write(&artifact, conflicting).unwrap();
            assert!(matches!(
                verify_migration_source_backup(&roots, &fixture.prepared, &fixture.verified),
                Err(MigrationBackupError::CorruptOrConflicting(_))
            ));
            assert_eq!(snapshot_tree(&fixture.graph_root), before);
        }
    }

    #[test]
    fn traversal_and_durability_fail_at_each_source_derived_ceiling() {
        fn assert_bounded(label: &str, summary: SourceSummary, build: impl FnOnce(&Path)) {
            let root = TestRoot::new(label);
            build(root.path());
            assert!(matches!(
                count_tree(root.path(), summary),
                Err(MigrationBackupError::ResourceLimit { .. })
            ));
            assert!(matches!(
                sync_tree(root.path(), summary),
                Err(MigrationBackupError::ResourceLimit { .. })
            ));
        }

        let base = SourceSummary {
            file_count: 1,
            chunk_count: 0,
            directory_count: 1,
            total_bytes: 4,
            max_path_bytes: 16,
            max_depth: 2,
        };
        assert_bounded("tree-files", base, |root| {
            fs::write(root.join("a"), b"a").unwrap();
            fs::write(root.join("b"), b"b").unwrap();
        });
        assert_bounded("tree-directories", base, |root| {
            fs::create_dir(root.join("a")).unwrap();
            fs::create_dir(root.join("b")).unwrap();
        });
        assert_bounded("tree-bytes", base, |root| {
            fs::write(root.join("a"), b"12345").unwrap();
        });
        assert_bounded(
            "tree-path-bytes",
            SourceSummary {
                max_path_bytes: 3,
                ..base
            },
            |root| fs::write(root.join("long"), b"a").unwrap(),
        );
        assert_bounded(
            "tree-depth",
            SourceSummary {
                directory_count: 2,
                max_depth: 2,
                ..base
            },
            |root| {
                fs::create_dir_all(root.join("a/b")).unwrap();
                fs::write(root.join("a/b/c"), b"a").unwrap();
            },
        );
    }

    fn manifest_record_ranges(
        bytes: &[u8],
        header_len: usize,
        count: u64,
    ) -> Vec<std::ops::Range<usize>> {
        fn take_u32(bytes: &[u8], cursor: &mut usize) -> u32 {
            let value = u32::from_be_bytes(bytes[*cursor..*cursor + 4].try_into().unwrap());
            *cursor += 4;
            value
        }
        let mut cursor = header_len;
        let mut ranges = Vec::new();
        for _ in 0..count {
            let start = cursor;
            let path = take_u32(bytes, &mut cursor) as usize;
            cursor += path;
            cursor += 1;
            let logical = take_u32(bytes, &mut cursor) as usize;
            cursor += logical;
            cursor += 40 + 32 + 8;
            let chunks = take_u32(bytes, &mut cursor) as usize;
            cursor += chunks * (4 + 40);
            ranges.push(start..cursor);
        }
        assert_eq!(cursor, bytes.len());
        ranges
    }
}
