//! Persistent managed-storage path vocabulary.
//!
//! These are names only. Tine owns authority, lifecycle, and interpretation;
//! this crate owns the exact path grammar so its release receipt certifies the
//! layout consumed by every writer and reader.

// Shared graph-local provider: `<graph>/.tine-sync/v2/shared/`.
pub const SHARED_ENROLLMENT_DESCRIPTOR_PATH: &str = "enrollment/shared-enrollment-v1.json";
pub const SHARED_FRONTIER_HEADS_DIR: &str = "frontier-heads-v1";
pub const SHARED_PUBLICATION_INTENTS_DIR: &str = "publication-intents-v1";
pub const SHARED_MANIFEST_RECOVERY_LINKS_DIR: &str = "manifest-recovery-links-v1";
pub const SHARED_MANIFEST_RECOVERY_BLOBS_DIR: &str = "manifest-recovery-blobs-v1";
pub const SHARED_OBJECTS_DIR: &str = "objects";
pub const SHARED_MANIFESTS_DIR: &str = "manifests";
pub const SHARED_ENROLLMENT_DIR: &str = "enrollment";
pub const SHARED_TEMP_DIR: &str = ".part";
pub const SHARED_REMOVED_DIR: &str = "removed";
pub const SHARED_RENAME_EVIDENCE_DIR: &str = "rename-evidence";
pub const PROVIDER_PENDING_PUBLICATION_DIR: &str = "pending-publication-v1";
pub const PROVIDER_INBOX_DIR: &str = "inbox";
pub const PROVIDER_OUTBOX_DIR: &str = "outbox";

// App-private binding and recovery roots.
pub const PRIVATE_BINDING_DIR: &str = "sparse-v2";
pub const PRIVATE_BINDING_FILE: &str = "binding.json";
pub const PRIVATE_RECOVERY_DIR: &str = "sparse-v2-recovery";

// Archive object store.
pub const ARCHIVE_OBJECTS_DIR: &str = "objects";
pub const ARCHIVE_BATCHES_DIR: &str = "batches";
pub const ARCHIVE_BOOTSTRAP_DIR: &str = "bootstrap-v1";
pub const ARCHIVE_LAZY_GENESIS_DIR: &str = "lazy-genesis-v1";
pub const LAZY_GENESIS_MANIFEST_FILE: &str = "manifest.postcard";
pub const LAZY_GENESIS_COMMIT_FILE: &str = "commit.postcard";
pub const LAZY_GENESIS_CATALOG_FILE: &str = "catalog.updates";
pub const LAZY_GENESIS_SEGMENT_PREFIX: &str = "segment-";
pub const LAZY_GENESIS_SEGMENT_SUFFIX: &str = ".pack";
pub const BOOTSTRAP_SOURCE_INVENTORY_DIR: &str = "source-inventory-indexes";
pub const BOOTSTRAP_SOURCE_BLOB_DIR: &str = "source-blob-indexes";
pub const BOOTSTRAP_SOURCE_CHUNKS_DIR: &str = "source-chunks";
pub const BOOTSTRAP_PARTS_DIR: &str = "parts";
pub const BOOTSTRAP_PART_SPANS_DIR: &str = "part-spans";
pub const BOOTSTRAP_PART_PACKS_DIR: &str = "part-object-packs";
pub const BOOTSTRAP_OBJECTS_DIR: &str = "objects";
pub const BOOTSTRAP_EVIDENCE_DIR: &str = "evidence";
pub const BOOTSTRAP_AGGREGATES_DIR: &str = "aggregates";
pub const BOOTSTRAP_COMMITS_DIR: &str = "commits";
pub const LINEAGE_CLAIM_FILE: &str = "lineage.claim";
pub const ENGINE_HISTORY_DIR: &str = "engine-history";
pub const ENGINE_HISTORY_NODES_DIR: &str = "nodes";
pub const ENGINE_HISTORY_ROOTS_DIR: &str = "roots";
pub const ENGINE_HISTORY_CLAIM_FILE: &str = "engine-history.claim";
pub const ENGINE_HISTORY_HEAD_FILE: &str = "engine-history.head";
pub const ENGINE_HISTORY_TRANSITION_LOCK_FILE: &str = "engine-history.transition.lock";
pub const ENGINE_HISTORY_ROOT_SUFFIX: &str = ".history-root";
pub const PROMOTED_RUNTIME_STATE_FILE: &str = "promoted-runtime.state";
pub const BLOCK_CLAIM_INDEX_DIR: &str = "block-claim-index";
pub const BLOCK_CLAIM_INDEX_FILE: &str = "pages.index";
pub const LOGSEQ_CLAIM_INDEX_DIR: &str = "logseq-uuid-claim-index-v1";
pub const PORTABLE_PATH_INDEX_DIR: &str = "portable-path-index-v1";
pub const PAGE_NAME_OWNERSHIP_INDEX_DIR: &str = "page-name-ownership-index-v1";
pub const REFERENCE_CATALOG_DIR: &str = "reference-catalog-v2";
pub const PROJECTION_WORK_DIR: &str = "projection-work-index-v1";
pub const ARCHIVE_INSTANCE_CLAIM_FILE: &str = "archive-instance-v1.claim";

// Derived receipts/work indexes.
pub const PROJECTION_STORE_CLAIM_FILE: &str = "projection-receipts.claim";
pub const PROJECTION_STORE_INIT_FILE: &str = "projection-receipts.init";
pub const PROJECTION_BASES_DIR: &str = "bases";
pub const PROJECTION_INTENTS_DIR: &str = "intents";
pub const PROJECTION_COMPLETIONS_DIR: &str = "completions";
pub const PROJECTION_ATTEMPTS_DIR: &str = "attempts";
pub const PROJECTION_FORENSICS_DIR: &str = "forensics";
pub const PROJECTION_PENDING_CLEANUP_SUFFIX: &str = ".projection-cleanup";
pub const PROJECTION_PENDING_CLEANUP_DIR: &str = ".pending-cleanup";
pub const PROJECTION_PENDING_CLEANUP_AUTHORITY_FILE: &str = ".pending-cleanup.authority";
pub const PROJECTION_CLEANUP_ROUND_STATE_FILE: &str = "round-robin.state";
pub const PROJECTION_CLEANUP_ROUND_0_DIR: &str = "round-0";
pub const PROJECTION_CLEANUP_ROUND_1_DIR: &str = "round-1";
pub const INTENT_NAMESPACE_RESERVATION_SUFFIX: &str = ".namespace-reservation";
pub const INTENT_NAMESPACE_AUTHORITY_SUFFIX: &str = ".namespace-authority";
pub const MUTATION_AUTHORITY_SUFFIX: &str = ".mutation-authority";
pub const MUTATION_AUTHORITY_LEASE_SUFFIX: &str = ".mutation-authority.lock";
pub const PROJECTION_WORK_CLAIM_FILE: &str = "projection-work.claim";
pub const PROJECTION_WORK_HEAD_FILE: &str = "projection-work.head";
pub const PROJECTION_WORK_PREPARED_SUFFIX: &str = ".prepared";
pub const PROJECTION_WORK_NODE_SUFFIX: &str = ".work-node";
pub const PROJECTION_WORK_ROOT_SUFFIX: &str = ".work-root";
pub const RESUME_POINT_DIR: &str = "resume-points";
pub const RESUME_POINT_SUFFIX: &str = ".resume-point";

// Device-private reconciliation, scratch, and journal state.
pub const RECONCILIATION_DIR: &str = "reconciliation";
pub const RECONCILIATION_DATABASE_FILE: &str = "scan.sqlite";
pub const RECONCILIATION_DATABASE_WAL_FILE: &str = "scan.sqlite-wal";
pub const RECONCILIATION_DATABASE_SHM_FILE: &str = "scan.sqlite-shm";
pub const RECONCILIATION_DATABASE_JOURNAL_FILE: &str = "scan.sqlite-journal";
pub const RECONCILIATION_FORENSIC_PREFIX: &str = "scan.sqlite.forensic-";
pub const RECONCILIATION_FORENSIC_DATABASE_FILE: &str = "database";
pub const RECONCILIATION_FORENSIC_WAL_FILE: &str = "wal";
pub const RECONCILIATION_FORENSIC_SHM_FILE: &str = "shm";
pub const RECONCILIATION_FORENSIC_JOURNAL_FILE: &str = "journal";
pub const RECONCILIATION_FORENSIC_EVIDENCE_COMPLETE: &str = "EVIDENCE_COMPLETE";
pub const RECONCILIATION_FORENSIC_REBUILD_COMPLETE: &str = "REBUILD_COMPLETE";
pub const SQLITE_RUNTIME_DIR: &str = ".tine-runtime";
pub const SQLITE_WORKSPACES_DIR: &str = "sqlite-workspaces";
pub const SQLITE_APPLIER_LOCK_FILE: &str = "sqlite-applier.lock";
pub const MANAGED_LOCAL_JOURNAL_DIR: &str = "managed-local-journal-v1";
pub const LOCAL_AUTHORSHIP_RECEIPT_DIR: &str = "local-authorship-v1";

// Enrollment.
pub const ENROLLMENT_STORAGE_DIR: &str = "sparse-storage";
pub const ENROLLMENT_VERSION_DIR: &str = "v2";
pub const ENROLLMENT_LOCAL_DIR: &str = "local";
pub const ENROLLMENT_DIR: &str = "enrollment";
pub const ENROLLMENT_RECORDS_DIR: &str = "records";
pub const ENROLLMENT_LEASE_FILE: &str = "lease";
pub const ENROLLMENT_AUTHORITY_FILE: &str = "authority-v1.claim";
pub const ENROLLMENT_HEAD_FILE: &str = "head";
pub const ENROLLMENT_RECORD_SUFFIX: &str = ".enrollment";
pub const ENROLLMENT_HEAD_TEMP_PREFIX: &str = ".head-tmp-";
pub const ENROLLMENT_RECORD_TEMP_PREFIX: &str = ".record-tmp-";
pub const ENROLLMENT_AUTHORITY_TEMP_PREFIX: &str = ".authority-tmp-";
pub const LOCAL_ACTIVATION_RESERVATION_FILE: &str = "local-activation-v1.reservation";

// Bootstrap/shadow/migration staging.
pub const BOOTSTRAP_STREAM_DIR: &str = "inactive-bootstrap-publication-v1";
pub const BOOTSTRAP_STREAM_SEAL_FILE: &str = "sealed.commit";
pub const BOOTSTRAP_STREAM_AGGREGATE_FILE: &str = "aggregate.bin";
pub const BOOTSTRAP_STREAM_COMMIT_FILE: &str = "commit.bin";
pub const BOOTSTRAP_STREAM_INVENTORY_PAGES_DIR: &str = "source-inventory-pages";
pub const BOOTSTRAP_STREAM_BLOB_PAGES_DIR: &str = "source-blob-pages";
pub const BOOTSTRAP_STREAM_PARTS_DIR: &str = "parts";
pub const BOOTSTRAP_STREAM_PART_MANIFEST_FILE: &str = "manifest.bin";
pub const BOOTSTRAP_STREAM_PART_EVIDENCE_FILE: &str = "evidence.bin";
pub const BOOTSTRAP_STREAM_PART_SPANS_FILE: &str = "spans.bin";
pub const BOOTSTRAP_STREAM_PART_OBJECTS_FILE: &str = "objects.frames";
pub const BOOTSTRAP_STREAM_OPERATION_SPOOL_FILE: &str = "operations.sorted";
pub const BOOTSTRAP_STREAM_BOUNDARY_SPOOL_FILE: &str = "part-boundaries.frames";
pub const SHADOW_ROOT_DIR: &str = "inactive-shadow-projections-v1";
pub const MIGRATION_BACKUP_ROOT_DIR: &str = "migration-source-backups-v1";
pub const MIGRATION_PAYLOAD_DIR: &str = "payload";
pub const MANIFEST_FILE: &str = "manifest.bin";
pub const PROOF_FILE: &str = "proof.bin";
pub const PROOF_STAGE_FILE: &str = ".proof.bin.staging";
pub const RESTORE_PROOF_FILE: &str = "restore-proof.bin";
pub const RESTORE_PROOF_STAGE_FILE: &str = ".restore-proof.bin.staging";
pub const COMMIT_MARKER_FILE: &str = "committed.bin";
pub const COMMIT_MARKER_STAGE_FILE: &str = ".committed.bin.staging";

// Page-name index and Direct-to-managed source capture.
pub const PAGE_NAME_EXACT_BLOB_SUFFIX: &str = ".exact-page-name";
pub const PAGE_NAME_STORE_CLAIM_FILE: &str = "page-name-index.claim";
pub const PAGE_NAME_NODES_DIR: &str = "nodes";
pub const PAGE_NAME_EXACT_NAMES_DIR: &str = "exact-names";
pub const BOOTSTRAP_SOURCE_CAPTURE_DIR: &str = "bootstrap-source-capture-v1";
pub const BOOTSTRAP_SOURCE_CAPTURE_CHUNKS_DIR: &str = "source-chunks";
pub const BOOTSTRAP_SOURCE_CAPTURE_MANIFEST_FILE: &str = "capture-manifest.bin";
pub const BOOTSTRAP_SOURCE_INVENTORY_FILE: &str = "inventory.sorted";
pub const BOOTSTRAP_SOURCE_ENTRIES_FILE: &str = "entries.sorted";
pub const BOOTSTRAP_SOURCE_CHUNKS_FILE: &str = "chunks.sorted";
pub const RESUME_POINT_TEMP_PREFIX: &str = ".tmp-";
pub const PROVIDER_DEVICE_AUTHORITY_FILE: &str = "provider-transaction.authority";
