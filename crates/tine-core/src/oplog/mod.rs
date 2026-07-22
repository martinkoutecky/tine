//! Candidate primitives and fenced storage substrate for Tine's sparse-first
//! operation log.
//!
//! The batch and object bytes enforce a deterministic candidate encoding, but
//! that encoding is not frozen until the later receipt and engine work lands.
//! Nothing here is wired to graph startup, enrollment, or mutation paths; the
//! store persists only when explicitly opened on a caller-supplied root.

pub mod batch;
pub mod identity;
pub mod object_store;
pub mod receipt;

pub use batch::{
    BatchError, ContentDigest, LineageDigest, ObjectDescriptor, ObjectKind, OperationBatch,
    OperationObject, PreparedBatch, SemanticEffectDigest, ValidatedBatch,
    MANIFEST_ENCODING_VERSION, MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES,
    OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};
pub use identity::{
    BatchId, BlockId, CrdtPeerId, DeviceId, DocumentId, ImportId, LogseqUuid, PageId, SessionId,
    WorkspaceId,
};
pub use object_store::{BatchInspection, ObjectStore, StoreError};
pub use receipt::{
    AnnotatedIdentity, BaseBlob, BatchClosureDigest, BlobDescription, CompletionId,
    CrdtPeerCounter, DocumentDependencies, FrontierV2, ImportInventoryEntry, ImportInventoryState,
    ImportLocator, ManagedPath, ProjectionCompletion, ProjectionIntent, ProjectionPolicy,
    ProjectionPrecondition, ReceiptError, StructuralLocator, StructuralSpan, DIFF_SCHEMA_VERSION,
    MANAGED_ENTITY_SET_VERSION, PROJECTION_POLICY_VERSION, PROJECTION_SCHEMA_VERSION,
    RECEIPT_SCHEMA_VERSION,
};
