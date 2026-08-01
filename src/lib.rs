//! Generic physical storage mechanisms shared by Tine persistence domains.

mod authenticated_patricia;
mod content_digest;
mod digest_sealed;
mod durable_batch;
mod filesystem;
mod scratch;

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
