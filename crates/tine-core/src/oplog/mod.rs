//! Candidate primitives and fenced storage substrate for Tine's sparse-first
//! operation log.
//!
//! The batch and object bytes enforce a deterministic candidate encoding, but
//! that encoding is not frozen until the later receipt and engine work lands.
//! Nothing here is wired to graph startup, enrollment, or mutation paths; the
//! store persists only when explicitly opened on a caller-supplied root.

pub(crate) mod authenticated_patricia;
pub mod batch;
pub(crate) mod causal_index;
pub(crate) mod dependency_queue;
pub(crate) mod document_state;
pub(crate) mod evidence_index;
pub mod hot_engine;
pub mod identity;
pub mod import;
pub(crate) mod loro_store;
pub mod object_store;
pub(crate) mod portable_path_index;
pub mod projection;
pub mod projection_manifest;
pub mod projection_store;
pub mod projection_work_index;
pub mod receipt;
pub(crate) mod scratch_store;
pub mod semantic;
pub mod simulator;
pub mod sqlite;
pub mod sqlite_materialization;
pub(crate) mod uuid_claim_index;

pub use batch::{
    BatchCausalDot, BatchError, BatchOrigin, CausalPeerId, ContentDigest, LineageDigest,
    ObjectDescriptor, ObjectKind, OperationBatch, OperationObject, PreparedBatch,
    SemanticEffectDigest, ValidatedBatch, MANIFEST_ENCODING_VERSION, MAX_MANIFEST_BYTES,
    MAX_OBJECT_BYTES, OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION,
    OPLOG_PROTOCOL_VERSION,
};
pub use hot_engine::{
    AcceptedBatch, AcceptedBatchEvidence, AuthorBatch, AuthorTransactionDraft, BatchDisposition,
    BlockLocation, CapabilityCapturedProjectionInput, CapabilityCapturedProjectionState,
    CurrentPageAtPath, EngineError, EngineInstrumentation, EngineStatus, FatalEvidenceHandle,
    ImmutableHomeClaim, ImmutableHomeConflict, ImmutableHomeEvidence, LogseqIdentityMutation,
    LogseqIdentityTrigger, LogseqUuidClaim, LogseqUuidResolution, MaterializationStats,
    MaterializedBlock, MaterializedPage, OperationTransaction, PortablePathConflict,
    PortablePathConflictParticipant, ProjectionEndpointBinding, ProjectionPageState,
    ProjectionRequirement, ProjectionRequirementState, ProjectionWriteAuthorization,
    SemanticOperation, ShardedHotEngine, StageOutcome, WorkspaceStatus,
};
pub use identity::{
    BatchId, BlockId, CanonicalGraphResourceId, CrdtPeerId, DeviceId, DocumentId, ImportId,
    LogseqUuid, PageId, ProjectionEndpointId, ProjectionReceiptStoreId, SessionId, WorkspaceId,
};
pub use import::{
    classify_conflict_copy, inventory_affected, inventory_initial_shadow, plan_affected_import,
    BlockImportMatch, BlockMatchBasis, ConflictClassificationError, ConflictCopyClass, ExactBytes,
    ImportBlock, ImportBlockReason, ImportInstrumentation, ImportMatches, ImportPlan,
    ImportPlanStatus, InventoryError, PageImportMatch, PageMatchBasis, RawInventory,
    RawObservation, RejectedRawId, RejectedRawIdReason, MAX_IMPORT_CATALOG_ENTRIES,
    MAX_IMPORT_DEPTH, MAX_IMPORT_FILES, MAX_IMPORT_LOCATOR_COMPONENTS, MAX_IMPORT_PARSED_NODES,
    MAX_IMPORT_RAW_BYTES,
};
pub use object_store::{BatchInspection, ObjectStore, ObjectStoreStats, StoreError};
pub use portable_path_index::{
    PortablePathIndexRoot, PortablePathOccupied, PortablePathRecord, PortablePathReleased,
};
pub use projection::{
    derive_receiver_local_projection, execute_manifested_projection_work, plan_projection,
    recover_incomplete_projections, write_projection_exact, PolicyGeneratedAnchor, ProjectionError,
    ProjectionPlan, ProjectionWrite,
};
pub use projection_manifest::{
    annotated_base_document_id, projection_intent_document_id, AnnotatedProjectionBase,
    ManifestObjectRef, ManifestProjectionPrecondition, ManifestProjectionTarget,
    ManifestedProjectionIntent, ProjectionManifestError, ValidatedProjectionObjects,
    ANNOTATED_BASE_SCHEMA_VERSION, MANIFESTED_PROJECTION_SCHEMA_VERSION, MAX_ANNOTATED_BASE_BYTES,
    MAX_MANIFESTED_PROJECTION_BYTES,
};
pub use projection_store::{
    LocalProjectionEvidenceRecord, ProjectionAttemptReservation, ProjectionReceiptStore,
    ProjectionStoreError,
};
pub use projection_work_index::{
    ProjectionPendingActivation, ProjectionPendingCursor, ProjectionPendingPage, ProjectionWork,
    ProjectionWorkCursor, ProjectionWorkError, ProjectionWorkId, ProjectionWorkIndex,
    ProjectionWorkIndexStats, ProjectionWorkPage, ProjectionWorkStatus, ProjectionWorkTarget,
};
pub(crate) use projection_work_index::{
    ProjectionWorkBlockAuthority, ProjectionWorkCompletionAuthority,
};
pub use receipt::{
    AnnotatedIdentity, BaseBlob, BlobDescription, CrdtPeerCounter, DocumentCausalDigest,
    DocumentDependencies, FrontierV2, ImportInventoryEntry, ImportInventoryState, ImportLocator,
    LogicalCompletionId, ManagedPath, ManagedTextKind, PortablePathKey, PortablePathKeyDigest,
    ProjectionClaimEvidence, ProjectionClaimParticipant, ProjectionCompletion, ProjectionIntent,
    ProjectionIntentId, ProjectionPrecondition, ReceiptError, StructuralLocator, StructuralSpan,
    DIFF_SCHEMA_VERSION, MANAGED_ENTITY_SET_VERSION, PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION,
    PORTABLE_PATH_KEY_VERSION, PORTABLE_PATH_NORMALIZATION_UNICODE_VERSION,
    PROJECTION_POLICY_VERSION, PROJECTION_SCHEMA_VERSION, RECEIPT_SCHEMA_VERSION,
};
pub(crate) use receipt::managed_component_is_portable;
pub use semantic::{
    BlockDelta, BlockOwner, BlockState, CanonicalSnapshot, LogseqIdentityOrigin, MembershipClaim,
    MembershipDelta, PageDelta, PagePreambleDelta, PagePreambleState, PageState,
    PolicyGeneratedAnchorReason, SemanticEffect, SemanticError, VisibleMembership,
    SEMANTIC_EFFECT_SCHEMA_VERSION,
};
pub use simulator::{
    DeterministicSimulator, FailureCapsule, FailureIdentity, MinimizedScenario, Scenario,
    ScenarioAction, ScenarioDevice, ScenarioError, SimulatorDeviceState,
    FAILURE_CAPSULE_SCHEMA_VERSION, SCENARIO_SCHEMA_VERSION,
};
pub use sqlite::{
    AcceptedBatchEvent, ApplicationRuntimeRoot, ApplyDisposition, ForensicEvidence, OpenProjection,
    ProjectionClaim, ProjectionError as SqliteProjectionError, ProjectionRecovery,
    RebuildInstrumentation, RebuildSource, SqliteFrontier, TailOverlay, TailOverlayError,
    TailOverlayStatus, TailReservation, SQLITE_APPLICATION_ID, SQLITE_SCHEMA_VERSION,
    TAIL_MAX_BATCHES, TAIL_MAX_BYTES,
};
pub use sqlite_materialization::{
    MaterializationChange, MaterializationError, MaterializedBlockInput, MaterializedBlockRow,
    MaterializedEntityId, MaterializedPageInput, MaterializedPageRow, MaterializedProperty,
    MaterializedPropertyRow, MaterializedReference, MaterializedReferenceKind,
    MaterializedReferrerRow, MaterializedSearchHit, MaterializedTagRow, MaterializedTask,
    MaterializedTaskRow, SqliteMaterializedRead, MAX_MATERIALIZATION_CHANGE_BLOCKS,
    MAX_MATERIALIZATION_CHANGE_BYTES, MAX_MATERIALIZATION_CHANGE_FACET_VALUES,
    MAX_MATERIALIZATION_CHANGE_PAGES, MAX_MATERIALIZATION_FACET_BYTES,
    MAX_MATERIALIZATION_FACET_VALUES, MAX_MATERIALIZATION_FIELD_BYTES,
    MAX_MATERIALIZATION_PREAMBLE_BYTES, MAX_MATERIALIZATION_QUERY_BYTES,
    MAX_MATERIALIZATION_QUERY_ROWS, MAX_MATERIALIZATION_READ_BYTES,
};
