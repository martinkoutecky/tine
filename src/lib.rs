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
//!
//! Every constant that describes bytes already on disk is exported from
//! [`formats`] and **only** from there, so a release or pin receipt has exactly
//! one thing to quote and a reader cannot reach a format constant by a path the
//! receipt does not cover. On-disk format versions are deliberately independent
//! of this crate's semver; see that module for the rule and the manifest.
//! `formats::tests::no_format_constant_has_a_second_export_path` enforces the
//! single-path rule against this file.

pub mod api_surface;
mod authenticated_patricia;
mod content_digest;
mod digest_sealed;
mod durable_batch;
mod filesystem;
pub mod formats;
mod local_journal;
mod local_journal_v2;
mod packed_patricia;
mod scratch;
mod sqlite_database;
mod sqlite_fileset;
mod sqlite_frontier;
mod sqlite_graph_projection;
mod sqlite_materialization;

/// Curated physical SQLite API for the disposable projection.
///
/// This facade exposes typed DTOs, errors, bounded reads, instrumentation,
/// physical file-set/candidate publication, and the connection-owning database
/// wrapper. It intentionally excludes raw DDL, direct connection access, and
/// lower-level production implementation helpers. Persistent-format constants
/// are not here either: they live in [`formats`], which owns every value a
/// reader must agree with a writer about.
pub mod sqlite {
    pub use crate::sqlite_database::{
        PhysicalReferenceCatalogStamp, PhysicalSqliteDatabase, PhysicalWriteInstrumentation,
    };
    pub use crate::sqlite_fileset::{
        PhysicalFileCheckpoint, PhysicalSqliteCheckpoint, SqliteFileSet, SqliteFileSetError,
        SqliteForensicPathMapping,
    };
    pub use crate::sqlite_frontier::{
        ApplyDisposition, ApplyFault, ApplyResult, FrontierError, PhysicalAcceptedBatch,
        PhysicalApplyRequest, PhysicalClaim, PhysicalFrontierDocument, PhysicalFrontierRoot,
        PreflightDisposition, StoredBatch, StoredFrontier,
    };
    pub use crate::sqlite_graph_projection::PhysicalGraphProjectionDatabase;
    pub use crate::sqlite_materialization::{
        ApplyChangeInstrumentation, MaterializationError, PhysicalAliasDeclaration,
        PhysicalAuthenticatedReference, PhysicalBlock, PhysicalBlockPropertyCandidateRow,
        PhysicalBlockReferenceCountRow, PhysicalBlockReferrerCandidateRow, PhysicalBlockRow,
        PhysicalBlockStructureRow, PhysicalEntityId, PhysicalGraphProjectionChange,
        PhysicalMaterializationChange, PhysicalNavigationAliasRow, PhysicalNavigationPageRow,
        PhysicalNavigationReferenceNameRow, PhysicalPage, PhysicalPageInventoryRow,
        PhysicalPageReferrerCandidateRow, PhysicalPageRow, PhysicalPlainTextCandidatePageRow,
        PhysicalProperty, PhysicalPropertyFacetRow, PhysicalPropertyRow, PhysicalReference,
        PhysicalReferenceCatalogChange, PhysicalReferencePosting, PhysicalReferenceTarget,
        PhysicalReferrerRow, PhysicalSearchHit, PhysicalSourceCoverage, PhysicalTagRow,
        PhysicalTask, PhysicalTaskCandidateBlockRow, PhysicalTaskCandidatePageRow, PhysicalTaskRow,
        PhysicalTerminalCatalogStamp, PhysicalTerminalConstructionBatch,
        PhysicalTerminalMaterializationChunk, SqliteGraphProjectionRead, SqliteMaterializedRead,
        MAX_MATERIALIZATION_QUERY_BYTES, MAX_MATERIALIZATION_QUERY_ROWS,
        MAX_MATERIALIZATION_READ_BYTES,
    };

    #[cfg(feature = "test-support")]
    pub use crate::sqlite_fileset::physical_checkpoint_interior_ranges_for_test;

    #[cfg(feature = "test-support")]
    pub use crate::sqlite_materialization::{
        apply_change as apply_materialization_change_for_test,
        initialize_schema as initialize_materialization_schema_for_test,
    };
}

pub use authenticated_patricia::{
    CompletedPatriciaIndexConstruction, PatriciaError, PatriciaIndexConstruction,
    PatriciaIndexConstructionStats, PatriciaIndexReclamationError, PatriciaIndexReclamationReport,
    PatriciaIndexRoot, PatriciaIndexStats, PatriciaIndexStore, PatriciaNodePublisher,
    PatriciaPublicationError, DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
    MAX_PATRICIA_CONSTRUCTION_BULK_RECORDS, MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
};
pub use content_digest::ContentDigest;
pub use digest_sealed::{DigestSealedError, DigestSealedPayload};
pub use durable_batch::{
    BatchCausalDot, BatchError, CausalPeerId, DurableBatchContract, LineageDigest,
    ObjectDescriptor, ObjectKind, OperationBatch, OperationObject, SemanticEffectDigest,
};
pub use filesystem::{
    ensure_directory_nofollow, nonblocking_lock_is_contended, open_dir_nofollow,
    open_existing_dir_nofollow, open_file_nofollow, publish_immutable_exact, read_optional_regular,
    read_required_regular, require_regular_entry, sync_dir_required,
    CompletedExactImmutablePublicationBatch, DurableDirectoryPublication,
    ExactImmutablePublicationBatch, FilesystemError, StagedExactImmutablePublication,
};
pub use local_journal::{
    LocalJournalAppend, LocalJournalError, LocalJournalFrame, LocalJournalPayloadKind,
    LocalJournalRecovery, LocalJournalSegment, LocalJournalStats,
};
pub use local_journal_v2::{
    LocalJournalAppendError, LocalJournalSegmentV2, LocalJournalSegmentV2Selection,
    LockedLocalJournalV1Segment,
};
#[cfg(feature = "test-support")]
pub use packed_patricia::{
    fail_head_transition_after_for_test, fail_next_head_transition_for_test,
    HeadTransitionFailureForTest,
};
pub use scratch::{
    census_retained_runs, reclaim_unreachable_retained_runs, RetainedRunCensus,
    RetainedRunReclamation, ScratchBlobRef, ScratchConstructionBoundary, ScratchLookupSession,
    ScratchLookupSessionStats, ScratchLsmRoot, ScratchOperationStats, ScratchPageRef,
    ScratchPageTag, ScratchRetention, ScratchRun, ScratchRunError, ScratchRunLifecycleStats,
    ScratchSegmentRef,
};
