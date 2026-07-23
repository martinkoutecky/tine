//! Candidate primitives and fenced storage substrate for Tine's sparse-first
//! operation log.
//!
//! The batch and object bytes enforce a deterministic candidate encoding, but
//! that encoding is not frozen until the later receipt and engine work lands.
//! Nothing here is wired to graph startup, enrollment, or mutation paths; the
//! store persists only when explicitly opened on a caller-supplied root.

pub mod batch;
pub(crate) mod causal_index;
pub(crate) mod dependency_queue;
pub(crate) mod document_state;
pub(crate) mod evidence_index;
pub mod hot_engine;
pub mod identity;
pub(crate) mod loro_store;
pub mod object_store;
pub mod receipt;
pub(crate) mod scratch_store;
pub mod semantic;
pub mod simulator;
pub mod sqlite;

pub use batch::{
    BatchCausalDot, BatchError, CausalPeerId, ContentDigest, LineageDigest, ObjectDescriptor,
    ObjectKind, OperationBatch, OperationObject, PreparedBatch, SemanticEffectDigest,
    ValidatedBatch, MANIFEST_ENCODING_VERSION, MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES,
    OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};
pub use hot_engine::{
    AcceptedBatch, AcceptedBatchEvidence, AcceptedFrontierRoot, AuthorBatch, BatchDisposition,
    BlockLocation, EngineError, EngineInstrumentation, EngineStatus, FatalEvidenceHandle,
    ImmutableHomeClaim, ImmutableHomeConflict, ImmutableHomeEvidence, MaterializationStats,
    MaterializedBlock, MaterializedPage, OperationTransaction, SemanticOperation, ShardedHotEngine,
    StageOutcome, WorkspaceStatus,
};
pub use identity::{
    BatchId, BlockId, CrdtPeerId, DeviceId, DocumentId, ImportId, LogseqUuid, PageId, SessionId,
    WorkspaceId,
};
pub use object_store::{BatchInspection, ObjectStore, ObjectStoreStats, StoreError};
pub use receipt::{
    AnnotatedIdentity, BaseBlob, BlobDescription, CompletionId, CrdtPeerCounter,
    DocumentCausalDigest, DocumentDependencies, FrontierV2, ImportInventoryEntry,
    ImportInventoryState, ImportLocator, ManagedPath, ProjectionCompletion, ProjectionIntent,
    ProjectionPolicy, ProjectionPrecondition, ReceiptError, StructuralLocator, StructuralSpan,
    DIFF_SCHEMA_VERSION, MANAGED_ENTITY_SET_VERSION, PROJECTION_POLICY_VERSION,
    PROJECTION_SCHEMA_VERSION, RECEIPT_SCHEMA_VERSION,
};
pub use semantic::{
    BlockDelta, BlockOwner, BlockState, CanonicalSnapshot, MembershipClaim, MembershipDelta,
    PageDelta, PageState, SemanticEffect, SemanticError, VisibleMembership,
    SEMANTIC_EFFECT_SCHEMA_VERSION,
};
pub use simulator::{
    DeterministicSimulator, FailureCapsule, FailureIdentity, MinimizedScenario, Scenario,
    ScenarioAction, ScenarioDevice, ScenarioError, SimulatorDeviceState,
    FAILURE_CAPSULE_SCHEMA_VERSION, SCENARIO_SCHEMA_VERSION,
};
pub use sqlite::{
    AcceptedBatchEvent, ApplicationRuntimeRoot, ApplyDisposition, ForensicEvidence, OpenProjection,
    ProjectionClaim, ProjectionError, ProjectionRecovery, RebuildInstrumentation, RebuildSource,
    SqliteFrontier, TailOverlay, TailOverlayError, TailOverlayStatus, TailReservation,
    SQLITE_APPLICATION_ID, SQLITE_SCHEMA_VERSION, TAIL_MAX_BATCHES, TAIL_MAX_BYTES,
};
