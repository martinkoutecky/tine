//! Candidate, in-memory primitives for Tine's sparse-first operation log.
//!
//! These types deliberately do not define a persistent layout or stable wire
//! encoding. They use ADR 0049's sharding-neutral `FrontierV2` semantics;
//! persistence and engine wiring remain deferred.

pub mod identity;
pub mod receipt;

pub use identity::{
    BatchId, BlockId, CrdtPeerId, DeviceId, DocumentId, ImportId, LogseqUuid, PageId, SessionId,
    WorkspaceId,
};
pub use receipt::{
    AnnotatedIdentity, BaseBlob, BatchClosureDigest, BlobDescription, CompletionId,
    CrdtPeerCounter, DocumentDependencies, FrontierV2, ImportInventoryEntry, ImportInventoryState,
    ImportLocator, ManagedPath, ProjectionCompletion, ProjectionIntent, ProjectionPolicy,
    ProjectionPrecondition, ReceiptError, StructuralLocator, StructuralSpan, DIFF_SCHEMA_VERSION,
    MANAGED_ENTITY_SET_VERSION, PROJECTION_POLICY_VERSION, PROJECTION_SCHEMA_VERSION,
    RECEIPT_SCHEMA_VERSION,
};
