//! Production storage substrate for Tine's opt-in managed-storage runtime.
//!
//! Enrollment, activation, mutation, projection, reconciliation, SQLite
//! materialization, and shared-provider synchronization are composed by
//! `crate::sync_runtime`. Direct Files does not enter this module tree: the
//! application selects that mutually exclusive runtime before opening a graph.
//! Immutable operation/object bytes are authoritative; SQLite and
//! projection-work state are disposable derived data.

pub(crate) mod absence_decision;
pub(crate) mod absence_sweep;
pub(crate) mod batch;
pub(crate) mod checkpoint_floor_policy;
#[cfg(test)]
mod checkpoint_floor_policy_tests;
pub(crate) mod checkpoint_generation;
pub(crate) mod cold_object_store;
pub(crate) mod conflict_history;
pub(crate) mod current_action_roots;
pub(crate) mod discovery;
pub(crate) mod enrollment;
pub(crate) mod external_import;
pub(crate) mod hot_engine;
#[cfg(test)]
mod hot_engine_integration_tests;
pub(crate) mod identity;
pub(crate) mod import;
#[cfg(test)]
mod import_integration_tests;
pub(crate) mod lazy_genesis;
pub(crate) mod local_active;
pub(crate) mod local_completion_index;
pub(crate) mod local_journal_drain;
pub(crate) mod local_journal_v2_anchor;
pub mod object_store;
pub(crate) mod operational_coordinator;
pub(crate) mod page_name_index;
pub(crate) mod portable_path_index;
pub(crate) mod projection;
#[cfg(test)]
mod projection_integration_tests;
pub(crate) mod projection_manifest;
pub(crate) mod projection_store;
pub(crate) mod projection_turn_journal;
pub(crate) mod projection_work;
pub(crate) mod query_cursor;
#[cfg(test)]
pub(crate) mod query_lowering;
pub(crate) mod receipt;
pub(crate) mod receiver_absence_map;
pub(crate) mod receiver_absence_summary;
pub(crate) mod recovery_input_journal;
pub(crate) mod reference_catalog;
pub(crate) mod refusal;
/// Retained retirable-document codec: dormant qualification machinery only.
/// The live write path uses the catalog and per-page shard documents, so the
/// codec's former live-path readers have no caller here.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod retirable_document;
pub(crate) mod semantic;
pub(crate) mod sqlite;
mod sqlite_identity;
pub(crate) mod sqlite_materialization;
pub mod sync_layout;
/// The character-level three-way machinery now lives at the crate root
/// (`crate::text_merge`) because Direct Files' conflict resolver shares it;
/// this re-export keeps `oplog::text_merge::…` naming its managed-storage
/// classifier.
pub use crate::text_merge;
pub(crate) mod trusted_local_commit;
pub(crate) mod uuid_claim_index;
pub(crate) mod wire;

/// Diagnostic trace gates, resolved once per process.
///
/// `std::env::var_os` walks the whole environment on every call, and these gates
/// sit on per-document and per-batch paths. Resolving them once keeps the
/// instrumentation free when it is off, which is the only way it is acceptable
/// on a hot path at all. The trade is that toggling mid-process does nothing --
/// correct for a diagnostic, wrong for a feature flag, so do not reuse these for
/// behaviour.
pub(crate) fn phase_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TINE_PHASE_TRACE").is_some())
}

/// Per-call-site `inspect_batch` attribution. Off, it costs one atomic load;
/// on, it formats a `file:line` key and takes a global lock per call.
pub(crate) fn inspect_site_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TINE_INSPECT_SITES").is_some())
}

pub use crate::graph_text_scope::{
    GraphTextScopeBinding, GraphTextScopeBindingError, GRAPH_TEXT_SCOPE_BINDING_SCHEMA_VERSION,
    GRAPH_TEXT_SCOPE_VERSION,
};
pub use batch::{
    BatchCausalDot, BatchError, BatchOrigin, CausalPeerId, ContentDigest, LineageDigest,
    ObjectDescriptor, ObjectKind, OperationBatch, OperationObject, PreparedBatch,
    SemanticEffectDigest, ValidatedBatch, MANIFEST_ENCODING_VERSION, MAX_MANIFEST_BYTES,
    MAX_OBJECT_BYTES, OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION,
    OPLOG_PROTOCOL_VERSION,
};
pub(crate) use hot_engine::{
    append_managed_local_record, projection_turn_attempt_id, projection_turn_recovery_filename,
    projection_turn_staged_filename, projection_turn_withdrawn_filename,
    CleanTombstoneAuthorization, CleanTombstoneDeferral, CleanTombstoneSupersession,
    ManagedLocalAppendError, ManagedLocalAppendProof, ManagedLocalJournalAppend, NeedsFullHistory,
    ProjectionTombstoneAuthorization, ProjectionTurn, ProjectionTurnError,
    ProjectionTurnPayloadKind, SequenceDomain, TurnOrigin, TurnPage, TurnPrecondition, TurnTarget,
    LIVE_PROJECTION_TURN_DERIVATION_SCHEMES, PROJECTION_TURN_DERIVATION_SCHEME_V1,
    PROJECTION_TURN_SCHEMA_VERSION,
};
pub use hot_engine::{
    decode_managed_local_record, AcceptedBatch, AcceptedBatchEvidence, AuthorBatch,
    AuthorTransactionDraft, BatchDisposition, BlockContentRewrite, BlockLocation, BlockRestore,
    CapabilityCapturedProjectionInput, CapabilityCapturedProjectionState, ConflictPair,
    ConflictResolutionIntent, CurrentPageAtPath, EngineError, EngineInstrumentation, EngineStatus,
    FatalEvidenceHandle, ImmutableHomeClaim, ImmutableHomeConflict, ImmutableHomeEvidence,
    LogseqIdentityMutation, LogseqIdentityTrigger, LogseqUuidClaim, LogseqUuidResolution,
    ManagedLocalApplyOutcome, ManagedLocalJournalPayloadKind, ManagedLocalPrefixState,
    ManagedLocalProjection, ManagedLocalRecord, ManagedLocalRecordError, ManagedLocalWork,
    MaterializationStats, MaterializedBlock, MaterializedPage, OperationTransaction,
    PagePreambleRewrite, PageRename, PortablePathConflict, PortablePathConflictParticipant,
    PreparedManagedLocalRecord, ProjectionEndpointBinding, ProjectionPageState,
    ProjectionRequirement, ProjectionRequirementState, ProjectionWriteAuthorization,
    SemanticOperation, ShardedHotEngine, StageOutcome, WorkspaceStatus,
};
#[cfg(test)]
pub(crate) use hot_engine::{inject_managed_local_append_fault_for_test, ManagedLocalAppendFault};
pub use identity::{
    BatchId, BlockId, CanonicalArchiveResourceId, CanonicalGraphResourceId, CrdtPeerId, DeviceId,
    DocumentId, DocumentKey, ImportId, LogseqUuid, PageId, ProjectionEndpointId,
    ProjectionReceiptStoreId, SessionId, WorkspaceId, WriterIncarnationId,
};
pub use import::{
    classify_conflict_copy, inventory_affected, inventory_initial_shadow, BlockImportMatch,
    BlockMatchBasis, ConflictClassificationError, ConflictCopyClass, ExactBytes, ImportBlock,
    ImportBlockReason, ImportInstrumentation, ImportMatches, ImportPlan, ImportPlanStatus,
    InventoryError, PageImportMatch, PageMatchBasis, RawInventory, RawObservation, RejectedRawId,
    RejectedRawIdReason, MAX_IMPORT_CATALOG_ENTRIES, MAX_IMPORT_DEPTH, MAX_IMPORT_FILES,
    MAX_IMPORT_LOCATOR_COMPONENTS, MAX_IMPORT_PARSED_NODES, MAX_IMPORT_RAW_BYTES,
};
pub(crate) use local_journal_v2_anchor::{
    classify_managed_local_anchor, managed_local_v2_anchor_name,
    parse_managed_local_v2_anchor_name, ManagedLocalAnchorEncoding, ManagedLocalGenerationAnchorV2,
    ManagedLocalJournal, MANAGED_LOCAL_ANCHOR_V2_BYTES,
};
pub use object_store::{BatchInspection, ObjectStore, ObjectStoreStats, StoreError};
pub use page_name_index::{
    ExactLogicalPageNameBlobV1, ExactLogicalPageNameDigest, ExactLogicalPageNameRefV1,
    PageNameOwnershipOccupiedV1, PageNameOwnershipRecordV1, PageNameOwnershipReleasedV1,
    PageNameOwnershipRootV1, EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION,
    EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION, MAX_PAGE_NAME_POINT_BATCH,
    PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION, PAGE_NAME_OWNERSHIP_RECORD_SCHEMA_VERSION,
    PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION, PAGE_NAME_OWNERSHIP_STORE_SCHEMA_VERSION,
};
pub use portable_path_index::{
    PortablePathIndexRoot, PortablePathOccupied, PortablePathRecord, PortablePathReleased,
};
pub use projection::{
    derive_receiver_local_projection, plan_projection, recover_incomplete_projections,
    PolicyGeneratedAnchor, ProjectionError, ProjectionPlan, ProjectionWrite,
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
pub(crate) use projection_work::ProjectionCompletedReceipt;
pub use projection_work::{ProjectionWork, ProjectionWorkId, ProjectionWorkTarget};
pub(crate) use receipt::managed_component_is_portable;
pub use receipt::{
    AnnotatedIdentity, BaseBlob, BlobDescription, CrdtPeerCounter, DocumentCausalDigest,
    DocumentDependencies, FrontierV2, ImportInventoryEntry, ImportInventoryState, ImportLocator,
    LogicalCompletionId, ManagedPath, ManagedTextKind, PortablePathKey, PortablePathKeyDigest,
    ProjectionClaimEvidence, ProjectionClaimParticipant, ProjectionCompletion, ProjectionIntent,
    ProjectionIntentId, ProjectionPrecondition, ProjectionTargetKind, ReceiptError,
    StructuralLocator, StructuralSpan, DIFF_SCHEMA_VERSION, MANAGED_ENTITY_SET_VERSION,
    PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION, PORTABLE_PATH_KEY_VERSION,
    PORTABLE_PATH_NORMALIZATION_UNICODE_VERSION, PROJECTION_POLICY_VERSION,
    PROJECTION_SCHEMA_VERSION, RECEIPT_SCHEMA_VERSION,
};
pub use reference_catalog::{
    BlockReferenceFactV1, BlockReferenceKindV1, PageNameReferenceFactV1, PageReferenceKindV1,
    ReferenceCatalogError, ReferenceCatalogPolicyV1, ReferenceFactV1, ReferenceSourceLocatorV1,
    ReferenceSourcePostingV2, REFERENCE_CATALOG_EXTRACTOR_VERSION,
    REFERENCE_CATALOG_POLICY_VERSION, REFERENCE_CATALOG_SCHEMA_VERSION,
};
pub use refusal::ManagedStorageRefusalScenario;
pub(crate) use refusal::BLOCKED_REASON_SCENARIOS;
pub use semantic::{
    BlockBirth, BlockDelta, BlockOwner, BlockReconstructionSource, BlockState, CanonicalSnapshot,
    LogicalPageName, LogicalPageNameError, LogseqIdentityOrigin, MembershipClaim, MembershipDelta,
    PageDelta, PageDeltaLifecycle, PageNameKeyDigest, PagePreambleDelta, PagePreambleState,
    PageState, PolicyGeneratedAnchorReason, SemanticEffect, SemanticError, VisibleMembership,
    CATALOG_PAGE_STATE_SCHEMA_VERSION, MAX_LOGICAL_PAGE_NAME_BYTES, PAGE_NAME_KEY_VERSION,
    SEMANTIC_EFFECT_SCHEMA_VERSION,
};
pub use sqlite::{
    AcceptedBatchEvent, ApplicationRuntimeRoot, ApplyDisposition, ForensicEvidence,
    FrontierReferenceHit, FrontierReferenceQuery, FrontierReferenceResults, FrontierRenamePlan,
    OpenProjection, ProjectionClaim, ProjectionError as SqliteProjectionError, ProjectionRecovery,
    RebuildInstrumentation, RebuildSource, ReferenceQueryInstrumentation, SqliteFrontier,
    TailOverlay, TailOverlayError, TailOverlayStatus, TailReservation, SQLITE_APPLICATION_ID,
    SQLITE_SCHEMA_VERSION, TAIL_MAX_BATCHES, TAIL_MAX_BYTES,
};
pub use sqlite_materialization::{
    MaterializationChange, MaterializationError, MaterializedBlockInput, MaterializedBlockRow,
    MaterializedEntityId, MaterializedPageInput, MaterializedPageRow, MaterializedPlanning,
    MaterializedProperty, MaterializedPropertyRow, MaterializedReference,
    MaterializedReferenceKind, MaterializedReferrerRow, MaterializedSearchHit, MaterializedTagRow,
    MaterializedTask, MaterializedTaskRow, SqliteMaterializedRead,
    MAX_MATERIALIZATION_CHANGE_BLOCKS, MAX_MATERIALIZATION_CHANGE_BYTES,
    MAX_MATERIALIZATION_CHANGE_FACET_VALUES, MAX_MATERIALIZATION_CHANGE_PAGES,
    MAX_MATERIALIZATION_FACET_BYTES, MAX_MATERIALIZATION_FACET_VALUES,
    MAX_MATERIALIZATION_FIELD_BYTES, MAX_MATERIALIZATION_PREAMBLE_BYTES,
    MAX_MATERIALIZATION_QUERY_BYTES, MAX_MATERIALIZATION_QUERY_ROWS,
    MAX_MATERIALIZATION_READ_BYTES,
};
pub use wire::SHARED_PROVIDER_TREE_NAMESPACES;

#[cfg(test)]
mod external_surface_tests {
    use sha2::{Digest, Sha256};

    #[test]
    fn oplog_external_module_surface_is_exactly_the_named_consumers() {
        let production = include_str!("mod.rs")
            .split("#[cfg(test)]\nmod external_surface_tests")
            .next()
            .unwrap();
        let public_modules = production
            .lines()
            .filter_map(|line| line.strip_prefix("pub mod "))
            .map(|line| line.trim_end_matches(';'))
            .collect::<Vec<_>>();
        assert_eq!(public_modules, ["object_store", "sync_layout"]);

        let mut public_uses = Vec::new();
        let mut declaration = None::<String>;
        for line in production.lines() {
            let trimmed = line.trim();
            if declaration.is_none() && trimmed.starts_with("pub use ") {
                declaration = Some(trimmed.to_owned());
            } else if let Some(current) = declaration.as_mut() {
                current.push_str(trimmed);
            }
            if trimmed.ends_with(';') {
                if let Some(current) = declaration.take() {
                    public_uses.push(
                        current
                            .chars()
                            .filter(|character| !character.is_whitespace())
                            .collect::<String>(),
                    );
                }
            }
        }
        assert_eq!(public_uses.len(), 20);
        let digest = Sha256::digest(public_uses.join("\n").as_bytes());
        assert_eq!(
            format!("{digest:x}"),
            // Re-pinned for P2's one current reconstruction-authority schema:
            // `BlockReconstructionSource` joins the semantic re-exports
            // because it is the public field type of
            // `BlockDelta::reconstruction`. No declaration was added or
            // removed; there are still exactly 20.
            "3ddb5191baad1a5c56d8998696a1fc9f4e2d0f24b6854cc58276b2da54dc1e51",
            "the exact public oplog re-export surface changed"
        );

        let unexpected_direct_public_items = production
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("pub "))
            .filter(|line| !line.starts_with("pub mod ") && !line.starts_with("pub use "))
            .collect::<Vec<_>>();
        assert!(
            unexpected_direct_public_items.is_empty(),
            "unexpected direct public oplog items: {unexpected_direct_public_items:?}"
        );
    }
}

pub(crate) mod writer_lane;
