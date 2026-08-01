//! Generic physical storage mechanisms shared by Tine persistence domains.
//!
//! The dependency direction is `src-tauri -> tine-core -> tine-storage`.
//! This crate owns physical storage mechanisms; `tine-core` owns policy,
//! authority, validation, and domain interpretation. SQLite is a disposable
//! projection: the oplog/archive remains authoritative and can rebuild it.
//! Consequently, this crate never depends on `tine-core`, `lsdoc`, Tauri, or
//! UI crates.
//!
//! SQLite implementation modules remain private. Consumers use [`sqlite`],
//! the deliberately curated physical-storage boundary that does not expose a
//! raw SQLite connection or schema-construction details.

mod authenticated_patricia;
mod content_digest;
mod digest_sealed;
mod durable_batch;
mod filesystem;
mod packed_patricia;
mod scratch;
mod sqlite_database;
mod sqlite_fileset;
mod sqlite_frontier;
mod sqlite_materialization;

/// Curated physical SQLite API for the disposable projection.
///
/// This facade exposes typed DTOs, errors, bounded reads, instrumentation,
/// stable physical constants, physical file-set/candidate publication, and the
/// connection-owning database wrapper. It intentionally excludes raw DDL,
/// direct connection access, and lower-level production implementation helpers.
pub mod sqlite {
    pub use crate::sqlite_database::{PhysicalReferenceCatalogStamp, PhysicalSqliteDatabase};
    pub use crate::sqlite_fileset::{SqliteFileSet, SqliteFileSetError};
    pub use crate::sqlite_frontier::{
        ApplyDisposition, ApplyFault, ApplyResult, FrontierError, PhysicalAcceptedBatch,
        PhysicalApplyRequest, PhysicalClaim, PhysicalFrontierDocument, PhysicalFrontierRoot,
        PreflightDisposition, StoredBatch, StoredFrontier, SQLITE_APPLICATION_ID,
        SQLITE_SCHEMA_VERSION,
    };
    pub use crate::sqlite_materialization::{
        ApplyChangeInstrumentation, MaterializationError, PhysicalAliasDeclaration,
        PhysicalAuthenticatedReference, PhysicalBlock, PhysicalBlockRow, PhysicalEntityId,
        PhysicalMaterializationChange, PhysicalPage, PhysicalPageRow, PhysicalProperty,
        PhysicalPropertyRow, PhysicalReference, PhysicalReferenceCatalogChange,
        PhysicalReferencePosting, PhysicalReferenceTarget, PhysicalReferrerRow, PhysicalSearchHit,
        PhysicalSourceCoverage, PhysicalTagRow, PhysicalTask, PhysicalTaskRow,
        SqliteMaterializedRead, MAX_MATERIALIZATION_QUERY_BYTES, MAX_MATERIALIZATION_QUERY_ROWS,
        MAX_MATERIALIZATION_READ_BYTES,
    };

    #[cfg(feature = "test-support")]
    pub use crate::sqlite_materialization::{
        apply_change as apply_materialization_change_for_test,
        initialize_schema as initialize_materialization_schema_for_test,
    };
}

pub use authenticated_patricia::{
    PatriciaError, PatriciaIndexConstruction, PatriciaIndexConstructionStats, PatriciaIndexRoot,
    PatriciaIndexStats, PatriciaIndexStore, PatriciaNodePublisher, PatriciaPublicationError,
    MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
};
pub use content_digest::ContentDigest;
pub use digest_sealed::{DigestSealedError, DigestSealedPayload};
pub use durable_batch::{
    BatchCausalDot, BatchError, CausalPeerId, DurableBatchContract, LineageDigest,
    ObjectDescriptor, ObjectKind, OperationBatch, OperationObject, SemanticEffectDigest,
    MANIFEST_ENCODING_VERSION, MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES,
    OBJECT_ENVELOPE_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};
pub use filesystem::{
    ensure_directory_nofollow, open_dir_nofollow, open_existing_dir_nofollow, open_file_nofollow,
    publish_immutable_exact, read_optional_regular, read_required_regular, require_regular_entry,
    sync_dir_required, CompletedExactImmutablePublicationBatch, ExactImmutablePublicationBatch,
    FilesystemError, ValidatedDirectorySync,
};
pub use scratch::{
    census_retained_runs, reclaim_unreachable_retained_runs, RetainedRunCensus,
    RetainedRunReclamation, ScratchBlobRef, ScratchConstructionBoundary, ScratchLookupSession,
    ScratchLookupSessionStats, ScratchLsmRoot, ScratchOperationStats, ScratchPageRef,
    ScratchPageTag, ScratchRetention, ScratchRun, ScratchRunError, ScratchRunLifecycleStats,
    ScratchSegmentRef, MAX_SCRATCH_BLOB_BYTES, MAX_SCRATCH_PAGE_BYTES, SCRATCH_BLOBS_FILE,
    SCRATCH_DIR, SCRATCH_LEASE_FILE, SCRATCH_LSM_LEVELS, SCRATCH_MARKER_FILE, SCRATCH_PAGES_FILE,
    SCRATCH_PAGE_SCHEMA_VERSION, SCRATCH_SCHEMA_VERSION,
};
