//! Persistent-format identity for `tine-storage`.
//!
//! This module is the single citable place for every constant that describes
//! something already written to disk. The storage release receipt and Tine's
//! storage pin receipt both quote [`FORMAT_MANIFEST`], so a format change is a
//! visible, reviewable event rather than an incidental diff.
//!
//! # The rule this module exists to enforce
//!
//! **On-disk format versions are never inferred from the crate's semver.** The
//! crate version tracks the Rust API; these constants track the bytes. They
//! move independently and for different reasons: an API-breaking refactor that
//! reads and writes identical bytes does not touch anything here, and a
//! one-field change to a stored envelope does — even in a patch release.
//!
//! # What belongs here
//!
//! A constant belongs in the manifest when a reader must agree with a writer
//! about it: envelope versions, magic values, on-disk file and directory names,
//! layout geometry, and the bounds a writer may legally have produced (a reader
//! that lowers such a bound stops being able to read older data).
//!
//! Deliberately **excluded**: in-memory budgets and read-path limits, which a
//! future version may change freely without stranding stored bytes. That is why
//! `MAX_MATERIALIZATION_QUERY_ROWS`, `MAX_MATERIALIZATION_QUERY_BYTES`,
//! `MAX_MATERIALIZATION_READ_BYTES` and
//! `MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES` are not listed: they bound one
//! process's work, not the bytes it left behind.
//!
//! # Adding a constant
//!
//! Re-export it below, add one [`FormatConstant`] row, and update the value in
//! `format_identity_is_pinned`. That test asserts exact values, so changing an
//! on-disk format cannot pass CI without an explicit edit that a reviewer sees.
//!
//! The definitions themselves stay in their owning modules; this module only
//! re-exports them, so listing a constant here can never change its value.

// --- format identity: envelope/schema versions and magic ---------------------
pub use crate::durable_batch::{
    MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};
pub use crate::local_journal::LOCAL_JOURNAL_FRAME_SCHEMA_VERSION;
pub use crate::local_journal_v2::{
    LOCAL_JOURNAL_FRONTIER_BYTES, LOCAL_JOURNAL_FRONTIER_SUFFIX, LOCAL_JOURNAL_FRONTIER_V2_MAGIC,
    LOCAL_JOURNAL_SEGMENT_HEADER_BYTES, LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION,
    LOCAL_JOURNAL_SEGMENT_V2_MAGIC,
};
pub use crate::scratch::{SCRATCH_PAGE_SCHEMA_VERSION, SCRATCH_SCHEMA_VERSION};
pub use crate::sqlite_frontier::{SQLITE_APPLICATION_ID, SQLITE_SCHEMA_VERSION};

// --- on-disk layout: names and shape -----------------------------------------
pub use crate::managed_layout::{
    ARCHIVE_BATCHES_DIR, ARCHIVE_BOOTSTRAP_DIR, ARCHIVE_INSTANCE_CLAIM_FILE,
    ARCHIVE_LAZY_GENESIS_DIR, ARCHIVE_OBJECTS_DIR, BLOCK_CLAIM_INDEX_DIR, BLOCK_CLAIM_INDEX_FILE,
    BOOTSTRAP_AGGREGATES_DIR, BOOTSTRAP_COMMITS_DIR, BOOTSTRAP_EVIDENCE_DIR, BOOTSTRAP_OBJECTS_DIR,
    BOOTSTRAP_PARTS_DIR, BOOTSTRAP_PART_PACKS_DIR, BOOTSTRAP_PART_SPANS_DIR,
    BOOTSTRAP_SOURCE_BLOB_DIR, BOOTSTRAP_SOURCE_CAPTURE_CHUNKS_DIR, BOOTSTRAP_SOURCE_CAPTURE_DIR,
    BOOTSTRAP_SOURCE_CAPTURE_MANIFEST_FILE, BOOTSTRAP_SOURCE_CHUNKS_DIR,
    BOOTSTRAP_SOURCE_CHUNKS_FILE, BOOTSTRAP_SOURCE_ENTRIES_FILE, BOOTSTRAP_SOURCE_INVENTORY_DIR,
    BOOTSTRAP_SOURCE_INVENTORY_FILE, BOOTSTRAP_STREAM_AGGREGATE_FILE,
    BOOTSTRAP_STREAM_BLOB_PAGES_DIR, BOOTSTRAP_STREAM_BOUNDARY_SPOOL_FILE,
    BOOTSTRAP_STREAM_COMMIT_FILE, BOOTSTRAP_STREAM_DIR, BOOTSTRAP_STREAM_INVENTORY_PAGES_DIR,
    BOOTSTRAP_STREAM_OPERATION_SPOOL_FILE, BOOTSTRAP_STREAM_PARTS_DIR,
    BOOTSTRAP_STREAM_PART_EVIDENCE_FILE, BOOTSTRAP_STREAM_PART_MANIFEST_FILE,
    BOOTSTRAP_STREAM_PART_OBJECTS_FILE, BOOTSTRAP_STREAM_PART_SPANS_FILE,
    BOOTSTRAP_STREAM_SEAL_FILE, COMMIT_MARKER_FILE, COMMIT_MARKER_STAGE_FILE,
    ENGINE_HISTORY_CLAIM_FILE, ENGINE_HISTORY_DIR, ENGINE_HISTORY_HEAD_FILE,
    ENGINE_HISTORY_NODES_DIR, ENGINE_HISTORY_ROOTS_DIR, ENGINE_HISTORY_ROOT_SUFFIX,
    ENGINE_HISTORY_TRANSITION_LOCK_FILE, ENROLLMENT_AUTHORITY_FILE,
    ENROLLMENT_AUTHORITY_TEMP_PREFIX, ENROLLMENT_DIR, ENROLLMENT_HEAD_FILE,
    ENROLLMENT_HEAD_TEMP_PREFIX, ENROLLMENT_LEASE_FILE, ENROLLMENT_LOCAL_DIR,
    ENROLLMENT_RECORDS_DIR, ENROLLMENT_RECORD_SUFFIX, ENROLLMENT_RECORD_TEMP_PREFIX,
    ENROLLMENT_STORAGE_DIR, ENROLLMENT_VERSION_DIR, INTENT_NAMESPACE_AUTHORITY_SUFFIX,
    INTENT_NAMESPACE_RESERVATION_SUFFIX, LAZY_GENESIS_CATALOG_FILE, LAZY_GENESIS_COMMIT_FILE,
    LAZY_GENESIS_MANIFEST_FILE, LAZY_GENESIS_SEGMENT_PREFIX, LAZY_GENESIS_SEGMENT_SUFFIX,
    LINEAGE_CLAIM_FILE, LOCAL_ACTIVATION_RESERVATION_FILE, LOCAL_AUTHORSHIP_RECEIPT_DIR,
    LOGSEQ_CLAIM_INDEX_DIR, MANAGED_LOCAL_JOURNAL_DIR, MANIFEST_FILE, MIGRATION_BACKUP_ROOT_DIR,
    MIGRATION_PAYLOAD_DIR, MUTATION_AUTHORITY_LEASE_SUFFIX, MUTATION_AUTHORITY_SUFFIX,
    PAGE_NAME_EXACT_BLOB_SUFFIX, PAGE_NAME_EXACT_NAMES_DIR, PAGE_NAME_NODES_DIR,
    PAGE_NAME_OWNERSHIP_INDEX_DIR, PAGE_NAME_STORE_CLAIM_FILE, PORTABLE_PATH_INDEX_DIR,
    PRIVATE_BINDING_DIR, PRIVATE_BINDING_FILE, PRIVATE_RECOVERY_DIR, PROJECTION_ATTEMPTS_DIR,
    PROJECTION_BASES_DIR, PROJECTION_CLEANUP_ROUND_0_DIR, PROJECTION_CLEANUP_ROUND_1_DIR,
    PROJECTION_CLEANUP_ROUND_STATE_FILE, PROJECTION_COMPLETIONS_DIR, PROJECTION_FORENSICS_DIR,
    PROJECTION_INTENTS_DIR, PROJECTION_PENDING_CLEANUP_AUTHORITY_FILE,
    PROJECTION_PENDING_CLEANUP_DIR, PROJECTION_PENDING_CLEANUP_SUFFIX, PROJECTION_STORE_CLAIM_FILE,
    PROJECTION_STORE_INIT_FILE, PROJECTION_WORK_CLAIM_FILE, PROJECTION_WORK_DIR,
    PROJECTION_WORK_HEAD_FILE, PROJECTION_WORK_NODE_SUFFIX, PROJECTION_WORK_PREPARED_SUFFIX,
    PROJECTION_WORK_ROOT_SUFFIX, PROMOTED_RUNTIME_STATE_FILE, PROOF_FILE, PROOF_STAGE_FILE,
    PROVIDER_DEVICE_AUTHORITY_FILE, PROVIDER_INBOX_DIR, PROVIDER_OUTBOX_DIR,
    PROVIDER_PENDING_PUBLICATION_DIR, RECONCILIATION_DATABASE_FILE,
    RECONCILIATION_DATABASE_JOURNAL_FILE, RECONCILIATION_DATABASE_SHM_FILE,
    RECONCILIATION_DATABASE_WAL_FILE, RECONCILIATION_DIR, RECONCILIATION_FORENSIC_DATABASE_FILE,
    RECONCILIATION_FORENSIC_EVIDENCE_COMPLETE, RECONCILIATION_FORENSIC_JOURNAL_FILE,
    RECONCILIATION_FORENSIC_PREFIX, RECONCILIATION_FORENSIC_REBUILD_COMPLETE,
    RECONCILIATION_FORENSIC_SHM_FILE, RECONCILIATION_FORENSIC_WAL_FILE, REFERENCE_CATALOG_DIR,
    RESTORE_PROOF_FILE, RESTORE_PROOF_STAGE_FILE, RESUME_POINT_DIR, RESUME_POINT_SUFFIX,
    RESUME_POINT_TEMP_PREFIX, SHADOW_ROOT_DIR, SHARED_ENROLLMENT_DESCRIPTOR_PATH,
    SHARED_ENROLLMENT_DIR, SHARED_FRONTIER_HEADS_DIR, SHARED_MANIFESTS_DIR,
    SHARED_MANIFEST_RECOVERY_BLOBS_DIR, SHARED_MANIFEST_RECOVERY_LINKS_DIR, SHARED_OBJECTS_DIR,
    SHARED_PUBLICATION_INTENTS_DIR, SHARED_REMOVED_DIR, SHARED_RENAME_EVIDENCE_DIR,
    SHARED_TEMP_DIR, SQLITE_APPLIER_LOCK_FILE, SQLITE_RUNTIME_DIR, SQLITE_WORKSPACES_DIR,
};
pub use crate::scratch::{
    SCRATCH_BLOBS_FILE, SCRATCH_DIR, SCRATCH_LEASE_FILE, SCRATCH_LSM_LEVELS, SCRATCH_MARKER_FILE,
    SCRATCH_PAGES_FILE,
};

// --- bounds a writer may legally have produced -------------------------------
pub use crate::durable_batch::{MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES};
pub use crate::local_journal::{
    MAX_LOCAL_JOURNAL_FRAME_BYTES, MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES,
    MAX_LOCAL_JOURNAL_SEGMENT_BYTES,
};
pub use crate::scratch::{MAX_SCRATCH_BLOB_BYTES, MAX_SCRATCH_PAGE_BYTES};

// --- checkpoint fingerprint geometry -----------------------------------------
// A stored checkpoint is only comparable to a fresh one computed with the same
// geometry, so these values are part of the stored artifact's meaning.
pub use crate::sqlite_fileset::{
    MAX_SQLITE_CHECKPOINT_BYTES, SQLITE_CHECKPOINT_EDGE_BYTES,
    SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES, SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES,
};

/// What kind of compatibility obligation a constant carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    /// An envelope/schema version or magic value. A change means stored bytes
    /// of the old value must still be readable or explicitly migrated.
    Identity,
    /// A file name, directory name, or structural shape on disk.
    Layout,
    /// A limit a writer may have produced up to. Lowering one can strand data.
    WriterBound,
    /// Geometry that determines how a stored fingerprint was computed.
    CheckpointGeometry,
}

/// A constant's value, in the shape it takes on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatValue {
    Number(u64),
    Name(&'static str),
}

/// One row of the persistent-format manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatConstant {
    /// The constant's Rust name, as re-exported from this module.
    pub name: &'static str,
    /// Which on-disk artifact it governs.
    pub artifact: &'static str,
    pub kind: FormatKind,
    pub value: FormatValue,
}

const fn num(
    name: &'static str,
    artifact: &'static str,
    kind: FormatKind,
    v: u64,
) -> FormatConstant {
    FormatConstant {
        name,
        artifact,
        kind,
        value: FormatValue::Number(v),
    }
}

const fn name_of(
    name: &'static str,
    artifact: &'static str,
    kind: FormatKind,
    v: &'static str,
) -> FormatConstant {
    FormatConstant {
        name,
        artifact,
        kind,
        value: FormatValue::Name(v),
    }
}

/// Every persistent-format constant this crate commits to, for mechanical
/// inclusion in a storage release receipt or a Tine storage pin receipt.
///
/// Generate a receipt section from this rather than transcribing values by
/// hand: a hand-copied receipt drifts silently, and the drift is invisible
/// exactly when it matters.
pub const FORMAT_MANIFEST: &[FormatConstant] = &[
    // identity
    num(
        "OPLOG_PROTOCOL_VERSION",
        "oplog manifest/object protocol",
        FormatKind::Identity,
        OPLOG_PROTOCOL_VERSION as u64,
    ),
    num(
        "OBJECT_ENVELOPE_SCHEMA_VERSION",
        "durable object envelope",
        FormatKind::Identity,
        OBJECT_ENVELOPE_SCHEMA_VERSION as u64,
    ),
    num(
        "MANIFEST_ENCODING_VERSION",
        "durable batch manifest",
        FormatKind::Identity,
        MANIFEST_ENCODING_VERSION as u64,
    ),
    num(
        "LOCAL_JOURNAL_FRAME_SCHEMA_VERSION",
        "local journal frame",
        FormatKind::Identity,
        LOCAL_JOURNAL_FRAME_SCHEMA_VERSION as u64,
    ),
    num(
        "LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION",
        "local journal v2 segment and frontier",
        FormatKind::Identity,
        LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION as u64,
    ),
    name_of(
        "LOCAL_JOURNAL_SEGMENT_V2_MAGIC",
        "local journal v2 segment header",
        FormatKind::Identity,
        LOCAL_JOURNAL_SEGMENT_V2_MAGIC,
    ),
    name_of(
        "LOCAL_JOURNAL_FRONTIER_V2_MAGIC",
        "local journal v2 frontier",
        FormatKind::Identity,
        LOCAL_JOURNAL_FRONTIER_V2_MAGIC,
    ),
    num(
        "SCRATCH_SCHEMA_VERSION",
        "engine scratch run",
        FormatKind::Identity,
        SCRATCH_SCHEMA_VERSION as u64,
    ),
    num(
        "SCRATCH_PAGE_SCHEMA_VERSION",
        "engine scratch page",
        FormatKind::Identity,
        SCRATCH_PAGE_SCHEMA_VERSION as u64,
    ),
    num(
        "SQLITE_APPLICATION_ID",
        "SQLite projection header",
        FormatKind::Identity,
        SQLITE_APPLICATION_ID as u64,
    ),
    num(
        "SQLITE_SCHEMA_VERSION",
        "SQLite projection schema",
        FormatKind::Identity,
        SQLITE_SCHEMA_VERSION as u64,
    ),
    // layout
    num(
        "LOCAL_JOURNAL_SEGMENT_HEADER_BYTES",
        "local journal v2 segment header",
        FormatKind::Layout,
        LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64,
    ),
    num(
        "LOCAL_JOURNAL_FRONTIER_BYTES",
        "local journal v2 frontier",
        FormatKind::Layout,
        LOCAL_JOURNAL_FRONTIER_BYTES as u64,
    ),
    name_of(
        "LOCAL_JOURNAL_FRONTIER_SUFFIX",
        "local journal v2 frontier",
        FormatKind::Layout,
        LOCAL_JOURNAL_FRONTIER_SUFFIX,
    ),
    name_of(
        "SCRATCH_DIR",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_DIR,
    ),
    name_of(
        "SCRATCH_MARKER_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_MARKER_FILE,
    ),
    name_of(
        "SCRATCH_LEASE_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_LEASE_FILE,
    ),
    name_of(
        "SCRATCH_PAGES_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_PAGES_FILE,
    ),
    name_of(
        "SCRATCH_BLOBS_FILE",
        "engine scratch directory",
        FormatKind::Layout,
        SCRATCH_BLOBS_FILE,
    ),
    num(
        "SCRATCH_LSM_LEVELS",
        "engine scratch LSM",
        FormatKind::Layout,
        SCRATCH_LSM_LEVELS as u64,
    ),
    // Managed-storage path grammar. These rows are an ownership/certification
    // migration only: every value is frozen to the preceding Tine-owned
    // sync_layout vocabulary.
    name_of(
        "SHARED_ENROLLMENT_DESCRIPTOR_PATH",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_ENROLLMENT_DESCRIPTOR_PATH,
    ),
    name_of(
        "SHARED_FRONTIER_HEADS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_FRONTIER_HEADS_DIR,
    ),
    name_of(
        "SHARED_PUBLICATION_INTENTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_PUBLICATION_INTENTS_DIR,
    ),
    name_of(
        "SHARED_MANIFEST_RECOVERY_LINKS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_MANIFEST_RECOVERY_LINKS_DIR,
    ),
    name_of(
        "SHARED_MANIFEST_RECOVERY_BLOBS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_MANIFEST_RECOVERY_BLOBS_DIR,
    ),
    name_of(
        "SHARED_OBJECTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_OBJECTS_DIR,
    ),
    name_of(
        "SHARED_MANIFESTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_MANIFESTS_DIR,
    ),
    name_of(
        "SHARED_ENROLLMENT_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_ENROLLMENT_DIR,
    ),
    name_of(
        "SHARED_TEMP_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_TEMP_DIR,
    ),
    name_of(
        "SHARED_REMOVED_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_REMOVED_DIR,
    ),
    name_of(
        "SHARED_RENAME_EVIDENCE_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHARED_RENAME_EVIDENCE_DIR,
    ),
    name_of(
        "PROVIDER_PENDING_PUBLICATION_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROVIDER_PENDING_PUBLICATION_DIR,
    ),
    name_of(
        "PROVIDER_INBOX_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROVIDER_INBOX_DIR,
    ),
    name_of(
        "PROVIDER_OUTBOX_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROVIDER_OUTBOX_DIR,
    ),
    name_of(
        "PRIVATE_BINDING_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PRIVATE_BINDING_DIR,
    ),
    name_of(
        "PRIVATE_BINDING_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PRIVATE_BINDING_FILE,
    ),
    name_of(
        "PRIVATE_RECOVERY_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PRIVATE_RECOVERY_DIR,
    ),
    name_of(
        "ARCHIVE_OBJECTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ARCHIVE_OBJECTS_DIR,
    ),
    name_of(
        "ARCHIVE_BATCHES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ARCHIVE_BATCHES_DIR,
    ),
    name_of(
        "ARCHIVE_BOOTSTRAP_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ARCHIVE_BOOTSTRAP_DIR,
    ),
    name_of(
        "ARCHIVE_LAZY_GENESIS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ARCHIVE_LAZY_GENESIS_DIR,
    ),
    name_of(
        "LAZY_GENESIS_MANIFEST_FILE",
        "managed storage layout",
        FormatKind::Layout,
        LAZY_GENESIS_MANIFEST_FILE,
    ),
    name_of(
        "LAZY_GENESIS_COMMIT_FILE",
        "managed storage layout",
        FormatKind::Layout,
        LAZY_GENESIS_COMMIT_FILE,
    ),
    name_of(
        "LAZY_GENESIS_CATALOG_FILE",
        "managed storage layout",
        FormatKind::Layout,
        LAZY_GENESIS_CATALOG_FILE,
    ),
    name_of(
        "LAZY_GENESIS_SEGMENT_PREFIX",
        "managed storage layout",
        FormatKind::Layout,
        LAZY_GENESIS_SEGMENT_PREFIX,
    ),
    name_of(
        "LAZY_GENESIS_SEGMENT_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        LAZY_GENESIS_SEGMENT_SUFFIX,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_INVENTORY_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_INVENTORY_DIR,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_BLOB_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_BLOB_DIR,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_CHUNKS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_CHUNKS_DIR,
    ),
    name_of(
        "BOOTSTRAP_PARTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_PARTS_DIR,
    ),
    name_of(
        "BOOTSTRAP_PART_SPANS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_PART_SPANS_DIR,
    ),
    name_of(
        "BOOTSTRAP_PART_PACKS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_PART_PACKS_DIR,
    ),
    name_of(
        "BOOTSTRAP_OBJECTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_OBJECTS_DIR,
    ),
    name_of(
        "BOOTSTRAP_EVIDENCE_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_EVIDENCE_DIR,
    ),
    name_of(
        "BOOTSTRAP_AGGREGATES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_AGGREGATES_DIR,
    ),
    name_of(
        "BOOTSTRAP_COMMITS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_COMMITS_DIR,
    ),
    name_of(
        "LINEAGE_CLAIM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        LINEAGE_CLAIM_FILE,
    ),
    name_of(
        "ENGINE_HISTORY_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_DIR,
    ),
    name_of(
        "ENGINE_HISTORY_NODES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_NODES_DIR,
    ),
    name_of(
        "ENGINE_HISTORY_ROOTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_ROOTS_DIR,
    ),
    name_of(
        "ENGINE_HISTORY_CLAIM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_CLAIM_FILE,
    ),
    name_of(
        "ENGINE_HISTORY_HEAD_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_HEAD_FILE,
    ),
    name_of(
        "ENGINE_HISTORY_TRANSITION_LOCK_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_TRANSITION_LOCK_FILE,
    ),
    name_of(
        "ENGINE_HISTORY_ROOT_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        ENGINE_HISTORY_ROOT_SUFFIX,
    ),
    name_of(
        "PROMOTED_RUNTIME_STATE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROMOTED_RUNTIME_STATE_FILE,
    ),
    name_of(
        "BLOCK_CLAIM_INDEX_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BLOCK_CLAIM_INDEX_DIR,
    ),
    name_of(
        "BLOCK_CLAIM_INDEX_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BLOCK_CLAIM_INDEX_FILE,
    ),
    name_of(
        "LOGSEQ_CLAIM_INDEX_DIR",
        "managed storage layout",
        FormatKind::Layout,
        LOGSEQ_CLAIM_INDEX_DIR,
    ),
    name_of(
        "PORTABLE_PATH_INDEX_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PORTABLE_PATH_INDEX_DIR,
    ),
    name_of(
        "PAGE_NAME_OWNERSHIP_INDEX_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PAGE_NAME_OWNERSHIP_INDEX_DIR,
    ),
    name_of(
        "REFERENCE_CATALOG_DIR",
        "managed storage layout",
        FormatKind::Layout,
        REFERENCE_CATALOG_DIR,
    ),
    name_of(
        "PROJECTION_WORK_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_WORK_DIR,
    ),
    name_of(
        "ARCHIVE_INSTANCE_CLAIM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ARCHIVE_INSTANCE_CLAIM_FILE,
    ),
    name_of(
        "PROJECTION_STORE_CLAIM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_STORE_CLAIM_FILE,
    ),
    name_of(
        "PROJECTION_STORE_INIT_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_STORE_INIT_FILE,
    ),
    name_of(
        "PROJECTION_BASES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_BASES_DIR,
    ),
    name_of(
        "PROJECTION_INTENTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_INTENTS_DIR,
    ),
    name_of(
        "PROJECTION_COMPLETIONS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_COMPLETIONS_DIR,
    ),
    name_of(
        "PROJECTION_ATTEMPTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_ATTEMPTS_DIR,
    ),
    name_of(
        "PROJECTION_FORENSICS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_FORENSICS_DIR,
    ),
    name_of(
        "PROJECTION_PENDING_CLEANUP_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_PENDING_CLEANUP_SUFFIX,
    ),
    name_of(
        "PROJECTION_PENDING_CLEANUP_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_PENDING_CLEANUP_DIR,
    ),
    name_of(
        "PROJECTION_PENDING_CLEANUP_AUTHORITY_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_PENDING_CLEANUP_AUTHORITY_FILE,
    ),
    name_of(
        "PROJECTION_CLEANUP_ROUND_STATE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_CLEANUP_ROUND_STATE_FILE,
    ),
    name_of(
        "PROJECTION_CLEANUP_ROUND_0_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_CLEANUP_ROUND_0_DIR,
    ),
    name_of(
        "PROJECTION_CLEANUP_ROUND_1_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_CLEANUP_ROUND_1_DIR,
    ),
    name_of(
        "INTENT_NAMESPACE_RESERVATION_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        INTENT_NAMESPACE_RESERVATION_SUFFIX,
    ),
    name_of(
        "INTENT_NAMESPACE_AUTHORITY_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        INTENT_NAMESPACE_AUTHORITY_SUFFIX,
    ),
    name_of(
        "MUTATION_AUTHORITY_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        MUTATION_AUTHORITY_SUFFIX,
    ),
    name_of(
        "MUTATION_AUTHORITY_LEASE_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        MUTATION_AUTHORITY_LEASE_SUFFIX,
    ),
    name_of(
        "PROJECTION_WORK_CLAIM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_WORK_CLAIM_FILE,
    ),
    name_of(
        "PROJECTION_WORK_HEAD_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_WORK_HEAD_FILE,
    ),
    name_of(
        "PROJECTION_WORK_PREPARED_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_WORK_PREPARED_SUFFIX,
    ),
    name_of(
        "PROJECTION_WORK_NODE_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_WORK_NODE_SUFFIX,
    ),
    name_of(
        "PROJECTION_WORK_ROOT_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        PROJECTION_WORK_ROOT_SUFFIX,
    ),
    name_of(
        "RESUME_POINT_DIR",
        "managed storage layout",
        FormatKind::Layout,
        RESUME_POINT_DIR,
    ),
    name_of(
        "RESUME_POINT_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        RESUME_POINT_SUFFIX,
    ),
    name_of(
        "RECONCILIATION_DIR",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_DIR,
    ),
    name_of(
        "RECONCILIATION_DATABASE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_DATABASE_FILE,
    ),
    name_of(
        "RECONCILIATION_DATABASE_WAL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_DATABASE_WAL_FILE,
    ),
    name_of(
        "RECONCILIATION_DATABASE_SHM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_DATABASE_SHM_FILE,
    ),
    name_of(
        "RECONCILIATION_DATABASE_JOURNAL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_DATABASE_JOURNAL_FILE,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_PREFIX",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_PREFIX,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_DATABASE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_DATABASE_FILE,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_WAL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_WAL_FILE,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_SHM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_SHM_FILE,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_JOURNAL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_JOURNAL_FILE,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_EVIDENCE_COMPLETE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_EVIDENCE_COMPLETE,
    ),
    name_of(
        "RECONCILIATION_FORENSIC_REBUILD_COMPLETE",
        "managed storage layout",
        FormatKind::Layout,
        RECONCILIATION_FORENSIC_REBUILD_COMPLETE,
    ),
    name_of(
        "SQLITE_RUNTIME_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SQLITE_RUNTIME_DIR,
    ),
    name_of(
        "SQLITE_WORKSPACES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SQLITE_WORKSPACES_DIR,
    ),
    name_of(
        "SQLITE_APPLIER_LOCK_FILE",
        "managed storage layout",
        FormatKind::Layout,
        SQLITE_APPLIER_LOCK_FILE,
    ),
    name_of(
        "MANAGED_LOCAL_JOURNAL_DIR",
        "managed storage layout",
        FormatKind::Layout,
        MANAGED_LOCAL_JOURNAL_DIR,
    ),
    name_of(
        "LOCAL_AUTHORSHIP_RECEIPT_DIR",
        "managed storage layout",
        FormatKind::Layout,
        LOCAL_AUTHORSHIP_RECEIPT_DIR,
    ),
    name_of(
        "ENROLLMENT_STORAGE_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_STORAGE_DIR,
    ),
    name_of(
        "ENROLLMENT_VERSION_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_VERSION_DIR,
    ),
    name_of(
        "ENROLLMENT_LOCAL_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_LOCAL_DIR,
    ),
    name_of(
        "ENROLLMENT_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_DIR,
    ),
    name_of(
        "ENROLLMENT_RECORDS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_RECORDS_DIR,
    ),
    name_of(
        "ENROLLMENT_LEASE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_LEASE_FILE,
    ),
    name_of(
        "ENROLLMENT_AUTHORITY_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_AUTHORITY_FILE,
    ),
    name_of(
        "ENROLLMENT_HEAD_FILE",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_HEAD_FILE,
    ),
    name_of(
        "ENROLLMENT_RECORD_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_RECORD_SUFFIX,
    ),
    name_of(
        "ENROLLMENT_HEAD_TEMP_PREFIX",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_HEAD_TEMP_PREFIX,
    ),
    name_of(
        "ENROLLMENT_RECORD_TEMP_PREFIX",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_RECORD_TEMP_PREFIX,
    ),
    name_of(
        "ENROLLMENT_AUTHORITY_TEMP_PREFIX",
        "managed storage layout",
        FormatKind::Layout,
        ENROLLMENT_AUTHORITY_TEMP_PREFIX,
    ),
    name_of(
        "LOCAL_ACTIVATION_RESERVATION_FILE",
        "managed storage layout",
        FormatKind::Layout,
        LOCAL_ACTIVATION_RESERVATION_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_DIR,
    ),
    name_of(
        "BOOTSTRAP_STREAM_SEAL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_SEAL_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_AGGREGATE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_AGGREGATE_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_COMMIT_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_COMMIT_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_INVENTORY_PAGES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_INVENTORY_PAGES_DIR,
    ),
    name_of(
        "BOOTSTRAP_STREAM_BLOB_PAGES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_BLOB_PAGES_DIR,
    ),
    name_of(
        "BOOTSTRAP_STREAM_PARTS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_PARTS_DIR,
    ),
    name_of(
        "BOOTSTRAP_STREAM_PART_MANIFEST_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_PART_MANIFEST_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_PART_EVIDENCE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_PART_EVIDENCE_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_PART_SPANS_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_PART_SPANS_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_PART_OBJECTS_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_PART_OBJECTS_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_OPERATION_SPOOL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_OPERATION_SPOOL_FILE,
    ),
    name_of(
        "BOOTSTRAP_STREAM_BOUNDARY_SPOOL_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_STREAM_BOUNDARY_SPOOL_FILE,
    ),
    name_of(
        "SHADOW_ROOT_DIR",
        "managed storage layout",
        FormatKind::Layout,
        SHADOW_ROOT_DIR,
    ),
    name_of(
        "MIGRATION_BACKUP_ROOT_DIR",
        "managed storage layout",
        FormatKind::Layout,
        MIGRATION_BACKUP_ROOT_DIR,
    ),
    name_of(
        "MIGRATION_PAYLOAD_DIR",
        "managed storage layout",
        FormatKind::Layout,
        MIGRATION_PAYLOAD_DIR,
    ),
    name_of(
        "MANIFEST_FILE",
        "managed storage layout",
        FormatKind::Layout,
        MANIFEST_FILE,
    ),
    name_of(
        "PROOF_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROOF_FILE,
    ),
    name_of(
        "PROOF_STAGE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROOF_STAGE_FILE,
    ),
    name_of(
        "RESTORE_PROOF_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RESTORE_PROOF_FILE,
    ),
    name_of(
        "RESTORE_PROOF_STAGE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        RESTORE_PROOF_STAGE_FILE,
    ),
    name_of(
        "COMMIT_MARKER_FILE",
        "managed storage layout",
        FormatKind::Layout,
        COMMIT_MARKER_FILE,
    ),
    name_of(
        "COMMIT_MARKER_STAGE_FILE",
        "managed storage layout",
        FormatKind::Layout,
        COMMIT_MARKER_STAGE_FILE,
    ),
    name_of(
        "PAGE_NAME_EXACT_BLOB_SUFFIX",
        "managed storage layout",
        FormatKind::Layout,
        PAGE_NAME_EXACT_BLOB_SUFFIX,
    ),
    name_of(
        "PAGE_NAME_STORE_CLAIM_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PAGE_NAME_STORE_CLAIM_FILE,
    ),
    name_of(
        "PAGE_NAME_NODES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PAGE_NAME_NODES_DIR,
    ),
    name_of(
        "PAGE_NAME_EXACT_NAMES_DIR",
        "managed storage layout",
        FormatKind::Layout,
        PAGE_NAME_EXACT_NAMES_DIR,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_CAPTURE_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_CAPTURE_DIR,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_CAPTURE_CHUNKS_DIR",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_CAPTURE_CHUNKS_DIR,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_CAPTURE_MANIFEST_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_CAPTURE_MANIFEST_FILE,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_INVENTORY_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_INVENTORY_FILE,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_ENTRIES_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_ENTRIES_FILE,
    ),
    name_of(
        "BOOTSTRAP_SOURCE_CHUNKS_FILE",
        "managed storage layout",
        FormatKind::Layout,
        BOOTSTRAP_SOURCE_CHUNKS_FILE,
    ),
    name_of(
        "RESUME_POINT_TEMP_PREFIX",
        "managed storage layout",
        FormatKind::Layout,
        RESUME_POINT_TEMP_PREFIX,
    ),
    name_of(
        "PROVIDER_DEVICE_AUTHORITY_FILE",
        "managed storage layout",
        FormatKind::Layout,
        PROVIDER_DEVICE_AUTHORITY_FILE,
    ),
    // writer bounds
    num(
        "MAX_MANIFEST_BYTES",
        "durable batch manifest",
        FormatKind::WriterBound,
        MAX_MANIFEST_BYTES as u64,
    ),
    num(
        "MAX_OBJECT_BYTES",
        "durable object envelope",
        FormatKind::WriterBound,
        MAX_OBJECT_BYTES as u64,
    ),
    num(
        "MAX_LOCAL_JOURNAL_FRAME_BYTES",
        "local journal frame",
        FormatKind::WriterBound,
        MAX_LOCAL_JOURNAL_FRAME_BYTES as u64,
    ),
    num(
        "MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES",
        "local journal frame header",
        FormatKind::WriterBound,
        MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES as u64,
    ),
    num(
        "MAX_LOCAL_JOURNAL_SEGMENT_BYTES",
        "local journal segment",
        FormatKind::WriterBound,
        MAX_LOCAL_JOURNAL_SEGMENT_BYTES,
    ),
    num(
        "MAX_SCRATCH_PAGE_BYTES",
        "engine scratch page",
        FormatKind::WriterBound,
        MAX_SCRATCH_PAGE_BYTES as u64,
    ),
    num(
        "MAX_SCRATCH_BLOB_BYTES",
        "engine scratch blob",
        FormatKind::WriterBound,
        MAX_SCRATCH_BLOB_BYTES as u64,
    ),
    // checkpoint geometry
    num(
        "MAX_SQLITE_CHECKPOINT_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        MAX_SQLITE_CHECKPOINT_BYTES as u64,
    ),
    num(
        "SQLITE_CHECKPOINT_EDGE_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        SQLITE_CHECKPOINT_EDGE_BYTES as u64,
    ),
    num(
        "SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES as u64,
    ),
    num(
        "SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES",
        "SQLite checkpoint fingerprint",
        FormatKind::CheckpointGeometry,
        SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES as u64,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins every persistent-format value. A failure here is not a broken
    /// test — it means an on-disk format changed. Update the expectation only
    /// together with the migration/compatibility story for existing graphs,
    /// and record it in the storage release receipt.
    #[test]
    fn format_identity_is_pinned() {
        assert_eq!(OPLOG_PROTOCOL_VERSION, 2);
        assert_eq!(OBJECT_ENVELOPE_SCHEMA_VERSION, 2);
        assert_eq!(MANIFEST_ENCODING_VERSION, 4);
        assert_eq!(LOCAL_JOURNAL_FRAME_SCHEMA_VERSION, 1);
        assert_eq!(LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION, 2);
        assert_eq!(LOCAL_JOURNAL_SEGMENT_V2_MAGIC, "TINEJNL2");
        assert_eq!(LOCAL_JOURNAL_FRONTIER_V2_MAGIC, "TINEFRT2");
        assert_eq!(SCRATCH_SCHEMA_VERSION, 13);
        assert_eq!(SCRATCH_PAGE_SCHEMA_VERSION, 1);
        assert_eq!(SQLITE_APPLICATION_ID, 0x5449_4e45);
        assert_eq!(SQLITE_SCHEMA_VERSION, 17);

        assert_eq!(LOCAL_JOURNAL_SEGMENT_HEADER_BYTES, 136);
        assert_eq!(LOCAL_JOURNAL_FRONTIER_BYTES, 240);
        assert_eq!(LOCAL_JOURNAL_FRONTIER_SUFFIX, ".frontier-v2");
        assert_eq!(SCRATCH_DIR, "engine-scratch-v2");
        assert_eq!(SCRATCH_MARKER_FILE, "marker");
        assert_eq!(SCRATCH_LEASE_FILE, "lease");
        assert_eq!(SCRATCH_PAGES_FILE, "pages.index");
        assert_eq!(SCRATCH_BLOBS_FILE, "blobs.data");
        assert_eq!(SCRATCH_LSM_LEVELS, 32);

        assert_eq!(MAX_MANIFEST_BYTES, 1024 * 1024);
        assert_eq!(MAX_OBJECT_BYTES, 256 * 1024 * 1024);
        assert_eq!(MAX_LOCAL_JOURNAL_FRAME_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES, 4 * 1024);
        assert_eq!(MAX_LOCAL_JOURNAL_SEGMENT_BYTES, 4 * 1024 * 1024 * 1024);
        assert_eq!(MAX_SCRATCH_PAGE_BYTES, 256 * 1024 * 1024);
        assert_eq!(MAX_SCRATCH_BLOB_BYTES, 256 * 1024 * 1024);

        assert_eq!(MAX_SQLITE_CHECKPOINT_BYTES, 64 * 1024);
        assert_eq!(SQLITE_CHECKPOINT_EDGE_BYTES, 64 * 1024);
        assert_eq!(SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES, 16 * 1024);
        assert_eq!(SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES, 1024 * 1024);
    }

    /// The manifest must quote the live constants, not a stale copy. Every row
    /// is checked against the constant it names, so a value edited in its
    /// owning module cannot leave a divergent value in a generated receipt.
    #[test]
    fn manifest_rows_match_the_live_constants() {
        let expected: &[(&str, FormatValue)] = &[
            (
                "OPLOG_PROTOCOL_VERSION",
                FormatValue::Number(OPLOG_PROTOCOL_VERSION as u64),
            ),
            (
                "OBJECT_ENVELOPE_SCHEMA_VERSION",
                FormatValue::Number(OBJECT_ENVELOPE_SCHEMA_VERSION as u64),
            ),
            (
                "MANIFEST_ENCODING_VERSION",
                FormatValue::Number(MANIFEST_ENCODING_VERSION as u64),
            ),
            (
                "LOCAL_JOURNAL_FRAME_SCHEMA_VERSION",
                FormatValue::Number(LOCAL_JOURNAL_FRAME_SCHEMA_VERSION as u64),
            ),
            (
                "LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION",
                FormatValue::Number(LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION as u64),
            ),
            (
                "LOCAL_JOURNAL_SEGMENT_V2_MAGIC",
                FormatValue::Name(LOCAL_JOURNAL_SEGMENT_V2_MAGIC),
            ),
            (
                "LOCAL_JOURNAL_FRONTIER_V2_MAGIC",
                FormatValue::Name(LOCAL_JOURNAL_FRONTIER_V2_MAGIC),
            ),
            (
                "SCRATCH_SCHEMA_VERSION",
                FormatValue::Number(SCRATCH_SCHEMA_VERSION as u64),
            ),
            (
                "SCRATCH_PAGE_SCHEMA_VERSION",
                FormatValue::Number(SCRATCH_PAGE_SCHEMA_VERSION as u64),
            ),
            (
                "SQLITE_APPLICATION_ID",
                FormatValue::Number(SQLITE_APPLICATION_ID as u64),
            ),
            (
                "SQLITE_SCHEMA_VERSION",
                FormatValue::Number(SQLITE_SCHEMA_VERSION as u64),
            ),
            (
                "LOCAL_JOURNAL_SEGMENT_HEADER_BYTES",
                FormatValue::Number(LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64),
            ),
            (
                "LOCAL_JOURNAL_FRONTIER_BYTES",
                FormatValue::Number(LOCAL_JOURNAL_FRONTIER_BYTES as u64),
            ),
            (
                "LOCAL_JOURNAL_FRONTIER_SUFFIX",
                FormatValue::Name(LOCAL_JOURNAL_FRONTIER_SUFFIX),
            ),
            ("SCRATCH_DIR", FormatValue::Name(SCRATCH_DIR)),
            (
                "SCRATCH_MARKER_FILE",
                FormatValue::Name(SCRATCH_MARKER_FILE),
            ),
            ("SCRATCH_LEASE_FILE", FormatValue::Name(SCRATCH_LEASE_FILE)),
            ("SCRATCH_PAGES_FILE", FormatValue::Name(SCRATCH_PAGES_FILE)),
            ("SCRATCH_BLOBS_FILE", FormatValue::Name(SCRATCH_BLOBS_FILE)),
            (
                "SCRATCH_LSM_LEVELS",
                FormatValue::Number(SCRATCH_LSM_LEVELS as u64),
            ),
            (
                "MAX_MANIFEST_BYTES",
                FormatValue::Number(MAX_MANIFEST_BYTES as u64),
            ),
            (
                "MAX_OBJECT_BYTES",
                FormatValue::Number(MAX_OBJECT_BYTES as u64),
            ),
            (
                "MAX_LOCAL_JOURNAL_FRAME_BYTES",
                FormatValue::Number(MAX_LOCAL_JOURNAL_FRAME_BYTES as u64),
            ),
            (
                "MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES",
                FormatValue::Number(MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES as u64),
            ),
            (
                "MAX_LOCAL_JOURNAL_SEGMENT_BYTES",
                FormatValue::Number(MAX_LOCAL_JOURNAL_SEGMENT_BYTES),
            ),
            (
                "MAX_SCRATCH_PAGE_BYTES",
                FormatValue::Number(MAX_SCRATCH_PAGE_BYTES as u64),
            ),
            (
                "MAX_SCRATCH_BLOB_BYTES",
                FormatValue::Number(MAX_SCRATCH_BLOB_BYTES as u64),
            ),
            (
                "MAX_SQLITE_CHECKPOINT_BYTES",
                FormatValue::Number(MAX_SQLITE_CHECKPOINT_BYTES as u64),
            ),
            (
                "SQLITE_CHECKPOINT_EDGE_BYTES",
                FormatValue::Number(SQLITE_CHECKPOINT_EDGE_BYTES as u64),
            ),
            (
                "SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES",
                FormatValue::Number(SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES as u64),
            ),
            (
                "SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES",
                FormatValue::Number(SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES as u64),
            ),
        ];
        let base = FORMAT_MANIFEST
            .iter()
            .filter(|row| row.artifact != "managed storage layout")
            .collect::<Vec<_>>();
        assert_eq!(
            base.len(),
            expected.len(),
            "the base persistent-format manifest changed without updating this test"
        );
        for (row, (name, value)) in base.into_iter().zip(expected) {
            assert_eq!(&row.name, name, "manifest order changed");
            assert_eq!(
                &row.value, value,
                "manifest row for {name} does not quote the live constant"
            );
        }
    }

    #[test]
    fn managed_layout_matches_the_frozen_pre_migration_vocabulary() {
        const FROZEN: &str = include_str!("../tests/fixtures/managed-layout-v1.txt");
        let expected = FROZEN
            .lines()
            .map(|line| {
                line.split_once('=')
                    .expect("every frozen managed-layout row has name=value")
            })
            .collect::<Vec<_>>();
        let actual = FORMAT_MANIFEST
            .iter()
            .filter(|row| row.artifact == "managed storage layout")
            .map(|row| {
                let FormatValue::Name(value) = row.value else {
                    panic!("managed-layout value {} is not a name", row.name);
                };
                (row.name, value)
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected, "managed-storage path vocabulary drifted");
        assert_eq!(
            FORMAT_MANIFEST.len(),
            31 + expected.len(),
            "a format row was added outside the pinned base or managed-layout inventories",
        );
    }

    #[test]
    fn manifest_names_are_unique() {
        let mut names: Vec<&str> = FORMAT_MANIFEST.iter().map(|c| c.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate name in FORMAT_MANIFEST");
    }

    /// A persistent-format constant is reachable by exactly one path:
    /// `tine_storage::formats::NAME`.
    ///
    /// This is not tidiness. A release receipt and a Tine pin receipt are
    /// generated from [`FORMAT_MANIFEST`], and their claim is "these are the
    /// format values this build commits to". A second export path lets a
    /// consumer bind to a format constant the receipt never mentions, so the
    /// receipt stops being a complete statement about the crate's format
    /// surface. Re-exporting a manifest name from `lib.rs` therefore fails
    /// here, and the fix is to import it from `formats` at the call site.
    ///
    /// Source-level rather than type-level because Rust has no way to ask
    /// "how many public paths reach this item?" — but the check is exact
    /// about what it inspects: the `pub use` items of `lib.rs`.
    #[test]
    fn no_format_constant_has_a_second_export_path() {
        const LIB_RS: &str = include_str!("lib.rs");

        let manifest_names: Vec<&str> = FORMAT_MANIFEST.iter().map(|c| c.name).collect();

        // Collect the identifiers `lib.rs` re-exports, ignoring its own
        // `pub mod formats;` declaration and any `formats::` re-export.
        let mut exported: Vec<&str> = Vec::new();
        let mut rest = LIB_RS;
        while let Some(start) = rest.find("pub use ") {
            rest = &rest[start + "pub use ".len()..];
            let end = rest.find(';').expect("a `pub use` item must be terminated");
            let item = &rest[..end];
            rest = &rest[end..];
            if item.starts_with("crate::formats") || item.starts_with("formats") {
                continue;
            }
            for token in item.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if !token.is_empty() {
                    exported.push(token);
                }
            }
        }

        let leaked: Vec<&str> = manifest_names
            .iter()
            .copied()
            .filter(|name| exported.contains(name))
            .collect();

        assert!(
            leaked.is_empty(),
            "persistent-format constants re-exported from lib.rs as well as `formats`: {leaked:?}\n\
             Remove them from the `lib.rs` re-exports; consumers import \
             `tine_storage::formats::NAME`."
        );

        // Guard the guard: if the parse ever stops seeing `lib.rs`'s exports,
        // the emptiness above would be vacuous rather than meaningful.
        assert!(
            exported.contains(&"ContentDigest"),
            "the lib.rs re-export parse found nothing recognizable; this test would pass vacuously"
        );
    }
}
