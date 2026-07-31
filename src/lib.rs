//! Generic physical storage mechanisms shared by Tine persistence domains.

mod authenticated_patricia;
mod content_digest;
mod filesystem;
mod scratch;

pub use authenticated_patricia::{
    PatriciaError, PatriciaIndexConstruction, PatriciaIndexConstructionStats, PatriciaIndexRoot,
    PatriciaIndexStats, PatriciaIndexStore, PatriciaNodePublisher, PatriciaPublicationError,
    MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
};
pub use content_digest::ContentDigest;
pub use filesystem::{
    ensure_directory_nofollow, open_dir_nofollow, open_existing_dir_nofollow, open_file_nofollow,
    publish_immutable_exact, read_optional_regular, read_required_regular, require_regular_entry,
    sync_dir_required, CompletedExactImmutablePublicationBatch, ExactImmutablePublicationBatch,
    FilesystemError, ValidatedDirectorySync,
};
pub use scratch::{
    census_retained_runs, reclaim_unreachable_retained_runs, RetainedRunCensus,
    RetainedRunReclamation, ScratchConstructionBoundary, ScratchRetention, ScratchRun,
    ScratchRunError, ScratchRunLifecycleStats, SCRATCH_BLOBS_FILE, SCRATCH_DIR, SCRATCH_LEASE_FILE,
    SCRATCH_MARKER_FILE, SCRATCH_PAGES_FILE, SCRATCH_SCHEMA_VERSION,
};
