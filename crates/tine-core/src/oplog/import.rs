//! Exact, read-only external inventory and conservative identity matching.
//!
//! This module plans reconciliation only. It does not publish semantic
//! operations, write a graph, consult SQLite, or activate managed sync.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::fd::{AsFd as _, AsRawFd as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::bootstrap_import::{
    ArchiveLocalFrontierBindingV1, BootstrapAggregateCommitV1, BootstrapAggregateManifestV1,
    BootstrapImportError, BootstrapImportPartEvidenceV1, BootstrapManifestFingerprintV1,
    BootstrapPartDescriptorV1, BootstrapPartSpanIndexV1, BootstrapPartitionProfileV1,
    FullObjectDescriptorV1, OperationDigestV1, OperationLeafV1, OperationRootV1,
    PayloadObjectDescriptorV1, PayloadObjectRootV1, SourceBlobChunkDescriptorV1,
    SourceBlobChunkDigestV1, SourceBlobChunkRootBuilderV1, SourceBlobChunkRootV1,
    SourceBlobIndexBuilderV1, SourceContentDigestV1, SourceInventoryIndexBuilderV1,
    SourceInventoryRootBuilderV1, SourceInventoryRootV1, SourceLeafDigestV1, SourceLeafV1,
    SourceSpanRootV1, SourceSpanV1, MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART, MAX_BOOTSTRAP_PARTS,
    MAX_OPERATIONS_PER_BOOTSTRAP_PART, MAX_PARSED_NODES_PER_SOURCE_FILE,
    MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART, MAX_SOURCE_FILE_BYTES, MAX_SOURCE_INDEX_PAGES,
    MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART, MAX_TOTAL_SOURCE_BYTES,
};
use super::external_import::{
    ExternalImportObservationEntry, ExternalImportObservationMaterial,
    ExternalImportObservationMaterialError, ExternalImportObservationState,
};
use super::hot_engine::{
    AcceptedFrontierRoot, AuthorBatch, DetachedBootstrapAcceptedEngineMaterial,
    DetachedBootstrapAuthoringSession, DetachedBootstrapCandidate, DetachedBootstrapReplayIdentity,
    ProjectionStorageBinding, MAX_TRANSACTION_OPERATIONS,
};
use super::identity::BootstrapPartId;
use super::object_store::{
    BootstrapAggregateHistoryBindingV1, BootstrapAuthoringCapability,
    BootstrapPublicationInspectionV1, ControlDirectoryIdentity, DurablyStagedBootstrapPrefix,
    EngineHistoryBinding, ObjectStore, PreparedBootstrapHistoryRecordV1, StoreError,
    ValidatedBootstrapPublicationV1,
};
use super::receipt::ImportIdDerivation;
use super::shadow_projection::BootstrapProjectionAuthority;
use super::{
    plan_projection, AcceptedBatchEvent, AnnotatedIdentity, BatchId, BatchOrigin, BlobDescription,
    BlockId, BlockLocation, ContentDigest, CrdtPeerId, CurrentPageAtPath, DeviceId, DocumentId,
    ImportId, ImportInventoryEntry, ImportInventoryState, ImportLocator, LineageDigest,
    LogicalCompletionId, LogicalPageName, LogseqIdentityMutation, LogseqUuid, ManagedPath,
    ManagedTextKind, ObjectKind, OperationBatch, OperationObject, OperationTransaction, PageId,
    ProjectionCompletedReceipt, ProjectionCompletion, ProjectionIntent, ProjectionReceiptStore,
    ProjectionStoreError, ReferenceCatalogPolicyV1, SemanticOperation, SessionId, ShardedHotEngine,
    StructuralLocator, StructuralSpan, WorkspaceId, DIFF_SCHEMA_VERSION,
};
use crate::model::{
    path_is_sync_conflict, resolve_external_document_identity, AcceptedExternalDocumentIdentity,
    BootstrapSourceCapture, BootstrapSourceCaptureInstrumentation, BootstrapSourceChunk,
    BootstrapSourceEntry, Graph, PageEntry, PageKind,
};

#[cfg(test)]
thread_local! {
    static SNAPSHOT_REVALIDATION_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static POST_FRONTIER_OVERRIDE:
        std::cell::RefCell<Option<AcceptedFrontierRoot>> = const { std::cell::RefCell::new(None) };
    static INACTIVE_BOOTSTRAP_ORCHESTRATION_CUT:
        std::cell::Cell<Option<InactiveBootstrapOrchestrationCut>> =
            const { std::cell::Cell::new(None) };
    static NEXT_BOOTSTRAP_PART_OPERATION_LIMIT:
        std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    static INACTIVE_BOOTSTRAP_PREPARATION_BEFORE_SEAL:
        std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> =
            const { std::cell::RefCell::new(None) };
}

/// Force the operations-per-part limit of exactly the next bootstrap
/// preparation, so a small deterministic fixture can be genuinely multipart.
///
/// It changes only how the operation spool is partitioned; every part is
/// authored, published, installed, and replayed through the ordinary path.
#[cfg(test)]
pub(crate) fn force_next_bootstrap_part_operation_limit(operations: u32) {
    NEXT_BOOTSTRAP_PART_OPERATION_LIMIT.with(|limit| limit.set(Some(operations)));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum InactiveBootstrapOrchestrationCut {
    AfterSourcePublication,
    AfterOnePart,
    AfterAggregatePrefix,
    AfterAggregateCommit,
    BeforeHistoryHead,
    AfterHistoryHead,
}

fn inactive_bootstrap_orchestration_cut(
    cut: InactiveBootstrapOrchestrationCut,
) -> Result<(), BootstrapStreamingImportError> {
    #[cfg(test)]
    {
        let inject = INACTIVE_BOOTSTRAP_ORCHESTRATION_CUT
            .with(|pending| (pending.get() == Some(cut)).then(|| pending.set(None)))
            .is_some();
        if inject {
            return Err(io::Error::other(format!(
                "injected inactive bootstrap orchestration cut: {cut:?}"
            ))
            .into());
        }
    }
    #[cfg(not(test))]
    let _ = cut;
    Ok(())
}

/// The 1M-block program target is expected to fit below these aggregate
/// ceilings for ordinary shallow documents. Inputs beyond them remain exact
/// raw evidence but are not parsed into an authoritative import plan.
pub const MAX_IMPORT_FILES: usize = 1_000_000;
pub const MAX_IMPORT_RAW_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_PARSED_NODES: usize = 2_000_000;
pub const MAX_IMPORT_DEPTH: usize = 256;
pub const MAX_IMPORT_LOCATOR_COMPONENTS: usize = 16_000_000;
pub const MAX_IMPORT_CATALOG_ENTRIES: usize = 2_000_000;
pub const MAX_IMPORT_PATH_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_REPLAY_ENTRIES: usize = 1_000_000;
pub const MAX_IMPORT_REPLAY_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_RENDERED_TARGET_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_STRUCTURAL_KEY_WORK: usize = 64_000_000;

const BOOTSTRAP_STREAM_SORT_BUFFER_BYTES: usize = 1024 * 1024;
const BOOTSTRAP_STREAM_SORT_FAN_IN: usize = 4;
const BOOTSTRAP_STREAM_MAX_SORT_RUNS: usize = 4096;
const BOOTSTRAP_STREAM_FRAME_BYTES: usize = 64 * 1024 * 1024 + 1024 * 1024;
const BOOTSTRAP_STREAM_DIRECTORY: &str = "inactive-bootstrap-publication-v1";
const BOOTSTRAP_STREAM_SEAL: &str = "sealed.commit";
const BOOTSTRAP_STREAM_AGGREGATE: &str = "aggregate.bin";
const BOOTSTRAP_STREAM_COMMIT: &str = "commit.bin";
const BOOTSTRAP_STREAM_INVENTORY_PAGES: &str = "source-inventory-pages";
const BOOTSTRAP_STREAM_BLOB_PAGES: &str = "source-blob-pages";
const BOOTSTRAP_STREAM_PARTS: &str = "parts";
const BOOTSTRAP_STREAM_PART_MANIFEST: &str = "manifest.bin";
const BOOTSTRAP_STREAM_PART_EVIDENCE: &str = "evidence.bin";
const BOOTSTRAP_STREAM_PART_SPANS: &str = "spans.bin";
const BOOTSTRAP_STREAM_PART_OBJECTS: &str = "objects.frames";
const BOOTSTRAP_STREAM_OPERATION_SPOOL: &str = "operations.sorted";
const BOOTSTRAP_STREAM_BOUNDARY_SPOOL: &str = "part-boundaries.frames";
const BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES: usize = 768 * 1024;
/// A conservative page/document cardinality cap keeps the ordinary v4 JSON
/// manifest, payload descriptor list, accepted-evidence document frontier, and
/// aggregate part descriptor below their existing byte limits. Page content is
/// packed up to this bound; a single page exceeds it only by being one document.
const BOOTSTRAP_STREAM_MAX_PAGE_CAPSULES_PER_PART: u32 = 512;
/// Page declarations are small, but authoring adds payload metadata beyond the
/// declaration operation itself. This cap leaves ample room under the separate
/// 4,096-payload-object bound while still fitting one million empty pages into
/// the existing 1,024-part aggregate limit.
const BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART: u32 = 2048;
/// Fixed per-operation allowance for the before/after and membership fields
/// added by the existing semantic-effect encoder. The operation's own
/// canonical bytes are charged separately. Exact prepared bytes are still
/// checked before the authoritative detached pass.
const BOOTSTRAP_STREAM_SEMANTIC_EFFECT_OVERHEAD: u64 = 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct BootstrapStreamingImportInstrumentation {
    pub(crate) source_files: u64,
    pub(crate) source_chunks: u64,
    pub(crate) source_bytes: u64,
    pub(crate) source_bytes_read: u64,
    pub(crate) parser_nodes: u64,
    pub(crate) operations: u64,
    pub(crate) source_spans: u64,
    pub(crate) parts: u32,
    pub(crate) page_declarations: u64,
    pub(crate) page_capsules: u64,
    pub(crate) huge_page_splits: u64,
    pub(crate) max_part_documents: u64,
    pub(crate) max_part_manifest_bytes: u64,
    pub(crate) max_part_payload_descriptors: u64,
    pub(crate) operation_spool_bytes: u64,
    pub(crate) prepared_bytes: u64,
    pub(crate) external_sort_runs: u64,
    pub(crate) capture_passes: u64,
    pub(crate) peak_owned_source_bytes: u64,
    pub(crate) peak_owned_parser_nodes: u64,
    pub(crate) peak_owned_part_operations: u64,
    pub(crate) peak_owned_part_bytes: u64,
    pub(crate) peak_owned_sort_buffer_bytes: u64,
    pub(crate) source_protocol_micros: u64,
    pub(crate) operation_spool_micros: u64,
    pub(crate) partition_micros: u64,
    pub(crate) detached_authoring_micros: u64,
    pub(crate) preparation_sealing_micros: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapPreparationSubphase {
    SourceProtocol,
    OperationSpool,
    Partition,
    DetachedAuthoring,
    Sealing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapPreparationSummary {
    pub(crate) source_files: u64,
    pub(crate) source_bytes: u64,
    pub(crate) parser_nodes: u64,
    pub(crate) operations: u64,
    pub(crate) parts: u32,
    pub(crate) prepared_bytes: u64,
    pub(crate) source_protocol_micros: u64,
    pub(crate) operation_spool_micros: u64,
    pub(crate) partition_micros: u64,
    pub(crate) detached_authoring_micros: u64,
    pub(crate) sealing_micros: u64,
}

impl From<&BootstrapStreamingImportInstrumentation> for BootstrapPreparationSummary {
    fn from(instrumentation: &BootstrapStreamingImportInstrumentation) -> Self {
        Self {
            source_files: instrumentation.source_files,
            source_bytes: instrumentation.source_bytes,
            parser_nodes: instrumentation.parser_nodes,
            operations: instrumentation.operations,
            parts: instrumentation.parts,
            prepared_bytes: instrumentation.prepared_bytes,
            source_protocol_micros: instrumentation.source_protocol_micros,
            operation_spool_micros: instrumentation.operation_spool_micros,
            partition_micros: instrumentation.partition_micros,
            detached_authoring_micros: instrumentation.detached_authoring_micros,
            sealing_micros: instrumentation.preparation_sealing_micros,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapPreparationProgress {
    Subphase(BootstrapPreparationSubphase),
    DetachedAuthoring { completed: u32, total: u32 },
    Summary(BootstrapPreparationSummary),
}

#[derive(Debug)]
pub(crate) enum BootstrapStreamingImportError {
    Io(io::Error),
    Protocol(BootstrapImportError),
    Store(StoreError),
    Engine(super::hot_engine::EngineError),
    InvalidSource(String),
    InvalidOperation(String),
    ResourceLimit {
        resource: &'static str,
        observed: u64,
        limit: u64,
    },
    SingletonOverLimit(&'static str),
    ConflictingSeal,
}

impl fmt::Display for BootstrapStreamingImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Protocol(error) => error.fmt(formatter),
            Self::Store(error) => error.fmt(formatter),
            Self::Engine(error) => error.fmt(formatter),
            Self::InvalidSource(detail) | Self::InvalidOperation(detail) => {
                formatter.write_str(detail)
            }
            Self::ResourceLimit {
                resource,
                observed,
                limit,
            } => write!(
                formatter,
                "{resource} limit exceeded: observed {observed}, limit {limit}"
            ),
            Self::SingletonOverLimit(resource) => {
                write!(
                    formatter,
                    "one bootstrap operation cannot fit the {resource} limit"
                )
            }
            Self::ConflictingSeal => {
                formatter.write_str("conflicting sealed bootstrap preparation")
            }
        }
    }
}

impl std::error::Error for BootstrapStreamingImportError {}

impl From<io::Error> for BootstrapStreamingImportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BootstrapImportError> for BootstrapStreamingImportError {
    fn from(error: BootstrapImportError) -> Self {
        Self::Protocol(error)
    }
}

impl From<StoreError> for BootstrapStreamingImportError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<super::hot_engine::EngineError> for BootstrapStreamingImportError {
    fn from(error: super::hot_engine::EngineError) -> Self {
        Self::Engine(error)
    }
}

/// Inactive ownership of a complete, sealed bootstrap preparation. It carries
/// no object-store, history, graph-writer, projection, SQLite, enrollment, or
/// runtime capability.
#[allow(dead_code)]
pub(crate) struct InactiveBootstrapPreparedPublication {
    source_capture: BootstrapSourceCapture,
    sealed_directory: PathBuf,
    aggregate: BootstrapAggregateManifestV1,
    commit: BootstrapAggregateCommitV1,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    /// The archive whose durable authenticated reference catalog this
    /// preparation's accepted roots were built in. Installation must target
    /// exactly that archive.
    reference_catalog_archive_identity: ControlDirectoryIdentity,
    candidate: Rc<DetachedBootstrapCandidate>,
    engine_materials: Vec<DetachedBootstrapAcceptedEngineMaterial>,
    terminal_construction: Option<TerminalBootstrapConstructionMaterial>,
    instrumentation: BootstrapStreamingImportInstrumentation,
}

#[allow(dead_code)]
impl InactiveBootstrapPreparedPublication {
    pub(crate) const fn aggregate(&self) -> &BootstrapAggregateManifestV1 {
        &self.aggregate
    }

    pub(crate) const fn commit(&self) -> BootstrapAggregateCommitV1 {
        self.commit
    }

    pub(crate) fn candidate(&self) -> &DetachedBootstrapCandidate {
        &self.candidate
    }

    pub(crate) fn engine_materials(&self) -> &[DetachedBootstrapAcceptedEngineMaterial] {
        &self.engine_materials
    }

    /// Move the one-shot terminal construction capability out of this
    /// preparation. A second call yields `None`, so no caller can build two
    /// candidates from the same retained material.
    pub(crate) fn take_terminal_construction_material(
        &mut self,
    ) -> Option<TerminalBootstrapConstructionMaterial> {
        self.terminal_construction.take()
    }

    pub(crate) const fn instrumentation(&self) -> &BootstrapStreamingImportInstrumentation {
        &self.instrumentation
    }

    pub(crate) const fn source_capture(&self) -> &BootstrapSourceCapture {
        &self.source_capture
    }

    pub(crate) fn aggregate_bytes(&self) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        Ok(read_bounded_file(
            &self.sealed_directory.join(BOOTSTRAP_STREAM_AGGREGATE),
            super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES,
        )?)
    }

    pub(crate) fn commit_bytes(&self) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        Ok(read_bounded_file(
            &self.sealed_directory.join(BOOTSTRAP_STREAM_COMMIT),
            super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES,
        )?)
    }

    pub(crate) fn source_inventory_page(
        &self,
        ordinal: u32,
    ) -> Result<super::bootstrap_import::SourceInventoryIndexPageV1, BootstrapStreamingImportError>
    {
        let bytes = read_bounded_file(
            &numbered_path(
                &self.sealed_directory.join(BOOTSTRAP_STREAM_INVENTORY_PAGES),
                ordinal,
            ),
            super::bootstrap_import::MAX_SOURCE_INDEX_PAGE_BYTES,
        )?;
        super::bootstrap_import::SourceInventoryIndexPageV1::decode(&bytes).map_err(Into::into)
    }

    pub(crate) fn source_blob_page(
        &self,
        ordinal: u32,
    ) -> Result<super::bootstrap_import::SourceBlobIndexPageV1, BootstrapStreamingImportError> {
        let bytes = read_bounded_file(
            &numbered_path(
                &self.sealed_directory.join(BOOTSTRAP_STREAM_BLOB_PAGES),
                ordinal,
            ),
            super::bootstrap_import::MAX_SOURCE_INDEX_PAGE_BYTES,
        )?;
        super::bootstrap_import::SourceBlobIndexPageV1::decode(&bytes).map_err(Into::into)
    }

    pub(crate) fn open_part(
        &self,
        ordinal: u32,
    ) -> Result<InactiveBootstrapPreparedPartCursor, BootstrapStreamingImportError> {
        if ordinal >= self.aggregate.parts().len() as u32 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "bootstrap part ordinal is outside the sealed aggregate".into(),
            ));
        }
        let directory = self
            .sealed_directory
            .join(BOOTSTRAP_STREAM_PARTS)
            .join(format!("{ordinal:08}"));
        let manifest = read_bounded_file(
            &directory.join(BOOTSTRAP_STREAM_PART_MANIFEST),
            BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES,
        )?;
        let evidence = read_bounded_file(
            &directory.join(BOOTSTRAP_STREAM_PART_EVIDENCE),
            super::bootstrap_import::MAX_BOOTSTRAP_PART_EVIDENCE_BYTES,
        )?;
        let spans = read_bounded_file(
            &directory.join(BOOTSTRAP_STREAM_PART_SPANS),
            super::bootstrap_import::MAX_PART_SPAN_INDEX_BYTES,
        )?;
        Ok(InactiveBootstrapPreparedPartCursor {
            ordinal,
            manifest,
            evidence,
            spans,
            objects: FrameReader::open(
                &directory.join(BOOTSTRAP_STREAM_PART_OBJECTS),
                super::batch::MAX_OBJECT_BYTES,
            )?,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct InactiveBootstrapOrchestrationInstrumentation {
    pub(crate) source_inventory_pages: u32,
    pub(crate) source_blob_pages: u32,
    pub(crate) source_chunks: u64,
    pub(crate) parts: u32,
    pub(crate) objects: u64,
    pub(crate) cold_records: u64,
    pub(crate) peak_owned_source_chunks: u32,
    pub(crate) peak_owned_parts: u32,
    pub(crate) peak_owned_cold_records: u32,
    pub(crate) durability_syncs: u64,
    pub(crate) preparation_validation_micros: u64,
    pub(crate) object_part_publication_micros: u64,
    pub(crate) aggregate_history_commit_micros: u64,
    pub(crate) fresh_validation_replay_micros: u64,
}

/// Fully reopened proof of one inactive bootstrap installation. It contains
/// immutable identities and read-only frontier values only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InactiveBootstrapVerifiedPublication {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    graph_resource: super::CanonicalGraphResourceId,
    publication_id: super::bootstrap_import::BootstrapPublicationIdV1,
    aggregate_digest: super::bootstrap_import::BootstrapAggregateDigestV1,
    import_id: ImportId,
    part_count: u32,
    predecessor_terminal: Option<BootstrapPartId>,
    accepted_frontier: AcceptedFrontierRoot,
    engine_binding: EngineHistoryBinding,
    storage_binding: ProjectionStorageBinding,
    bootstrap_binding: BootstrapAggregateHistoryBindingV1,
    archive_identity: ControlDirectoryIdentity,
    history_generation: u64,
    history_root: ContentDigest,
    cold_record_count: u64,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    instrumentation: InactiveBootstrapOrchestrationInstrumentation,
}

impl InactiveBootstrapVerifiedPublication {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn graph_resource(&self) -> super::CanonicalGraphResourceId {
        self.graph_resource
    }

    pub(crate) const fn publication_id(&self) -> super::bootstrap_import::BootstrapPublicationIdV1 {
        self.publication_id
    }

    pub(crate) const fn aggregate_digest(
        &self,
    ) -> super::bootstrap_import::BootstrapAggregateDigestV1 {
        self.aggregate_digest
    }

    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) const fn part_count(&self) -> u32 {
        self.part_count
    }

    pub(crate) const fn predecessor_terminal(&self) -> Option<BootstrapPartId> {
        self.predecessor_terminal
    }

    pub(crate) const fn accepted_frontier(&self) -> &AcceptedFrontierRoot {
        &self.accepted_frontier
    }

    pub(crate) const fn engine_binding(&self) -> &EngineHistoryBinding {
        &self.engine_binding
    }

    pub(crate) const fn storage_binding(&self) -> ProjectionStorageBinding {
        self.storage_binding
    }

    pub(crate) const fn bootstrap_binding(&self) -> BootstrapAggregateHistoryBindingV1 {
        self.bootstrap_binding
    }

    pub(crate) const fn archive_identity(&self) -> ControlDirectoryIdentity {
        self.archive_identity
    }

    pub(crate) const fn history_generation(&self) -> u64 {
        self.history_generation
    }

    pub(crate) const fn history_root(&self) -> ContentDigest {
        self.history_root
    }

    pub(crate) const fn cold_record_count(&self) -> u64 {
        self.cold_record_count
    }

    pub(crate) const fn instrumentation(&self) -> &InactiveBootstrapOrchestrationInstrumentation {
        &self.instrumentation
    }

    pub(crate) const fn catalog_document_id(&self) -> DocumentId {
        self.catalog_document_id
    }

    pub(crate) const fn reference_catalog_policy(&self) -> &ReferenceCatalogPolicyV1 {
        &self.reference_catalog_policy
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InactiveBootstrapAcceptedAuthorityBinding {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    graph_resource: super::CanonicalGraphResourceId,
    publication_id: super::bootstrap_import::BootstrapPublicationIdV1,
    aggregate_digest: super::bootstrap_import::BootstrapAggregateDigestV1,
    import_id: ImportId,
    part_count: u32,
    predecessor_terminal: Option<BootstrapPartId>,
    accepted_frontier: AcceptedFrontierRoot,
    engine_binding: EngineHistoryBinding,
    storage_binding: ProjectionStorageBinding,
    bootstrap_binding: BootstrapAggregateHistoryBindingV1,
    archive_identity: ControlDirectoryIdentity,
    history_generation: u64,
    history_root: ContentDigest,
    cold_record_count: u64,
}

impl InactiveBootstrapAcceptedAuthorityBinding {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn accepted_frontier(&self) -> &AcceptedFrontierRoot {
        &self.accepted_frontier
    }

    pub(crate) const fn part_count(&self) -> u32 {
        self.part_count
    }

    pub(crate) const fn graph_resource(&self) -> super::CanonicalGraphResourceId {
        self.graph_resource
    }

    pub(crate) const fn publication_id(&self) -> super::bootstrap_import::BootstrapPublicationIdV1 {
        self.publication_id
    }

    pub(crate) const fn aggregate_digest(
        &self,
    ) -> super::bootstrap_import::BootstrapAggregateDigestV1 {
        self.aggregate_digest
    }

    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) const fn predecessor_terminal(&self) -> Option<BootstrapPartId> {
        self.predecessor_terminal
    }

    pub(crate) const fn engine_binding(&self) -> &EngineHistoryBinding {
        &self.engine_binding
    }

    pub(crate) const fn storage_binding(&self) -> ProjectionStorageBinding {
        self.storage_binding
    }

    pub(crate) const fn bootstrap_binding(&self) -> BootstrapAggregateHistoryBindingV1 {
        self.bootstrap_binding
    }

    pub(crate) const fn archive_identity(&self) -> ControlDirectoryIdentity {
        self.archive_identity
    }

    pub(crate) const fn history_generation(&self) -> u64 {
        self.history_generation
    }

    pub(crate) const fn history_root(&self) -> ContentDigest {
        self.history_root
    }

    pub(crate) const fn cold_record_count(&self) -> u64 {
        self.cold_record_count
    }
}

/// Retained, read-only accepted-history authority for one inactive bootstrap.
///
/// All fields are private and the only constructor freshly reopens the exact
/// archive, publication, durable history, and detached replay. Keeping the
/// candidate alive also keeps its scratch-backed accepted indexes alive.
pub(crate) struct InactiveBootstrapAcceptedAuthority {
    store: ObjectStore,
    publication: ValidatedBootstrapPublicationV1,
    candidate: Rc<DetachedBootstrapCandidate>,
    binding: InactiveBootstrapAcceptedAuthorityBinding,
}

/// One-shot process-local access to the exact detached candidate retained by
/// an inactive bootstrap authority.
///
/// This handle is intentionally neither `Clone` nor serializable. It carries
/// no writable authority and exposes no engine directly; promotion can only
/// consume it through the durable-binding migration path.
pub(crate) struct RetainedBootstrapPromotionCandidate {
    candidate: Rc<DetachedBootstrapCandidate>,
    binding: InactiveBootstrapAcceptedAuthorityBinding,
}

impl RetainedBootstrapPromotionCandidate {
    pub(crate) fn candidate(&self) -> &DetachedBootstrapCandidate {
        &self.candidate
    }

    pub(crate) const fn binding(&self) -> &InactiveBootstrapAcceptedAuthorityBinding {
        &self.binding
    }
}

impl InactiveBootstrapAcceptedAuthority {
    pub(crate) const fn store(&self) -> &ObjectStore {
        &self.store
    }

    pub(crate) const fn publication(&self) -> &ValidatedBootstrapPublicationV1 {
        &self.publication
    }

    pub(crate) fn accepted_engine(&self) -> &ShardedHotEngine {
        self.candidate.accepted_engine()
    }

    pub(crate) const fn binding(&self) -> &InactiveBootstrapAcceptedAuthorityBinding {
        &self.binding
    }

    pub(crate) fn retain_promotion_candidate(&self) -> RetainedBootstrapPromotionCandidate {
        RetainedBootstrapPromotionCandidate {
            candidate: Rc::clone(&self.candidate),
            binding: self.binding.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn corrupt_retained_candidate_scratch_for_test(&self) {
        self.candidate.corrupt_scratch_for_promotion_test();
    }
}

/// One-shot, process-local capability that lets an uninterrupted fresh
/// activation build its SQLite projection from the terminal accepted state it
/// just authored, instead of reloading and replaying every physical bootstrap
/// part.
///
/// It retains only values this preparation already produced: the existing
/// operation spool file and the already-typed accepted event of each authored
/// part. It is deliberately neither `Clone` nor serializable, is never sealed,
/// fsynced, or named by any durable artifact, and removes its relocated spool
/// on drop. Crash residue is ordinary incomplete-preparation garbage that a new
/// process ignores.
///
/// It is an optimization capability, never an authority: every consumer must
/// independently bind it to the retained candidate, aggregate, durable history
/// root, and accepted frontier. Absence or refusal simply selects the existing
/// archive replay path.
pub(crate) struct TerminalBootstrapConstructionMaterial {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    import_id: ImportId,
    operations: PathBuf,
    operation_count: u64,
    declaration_count: u64,
    accepted_events: Vec<AcceptedBatchEvent>,
}

#[allow(dead_code)]
impl TerminalBootstrapConstructionMaterial {
    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) fn accepted_events(&self) -> &[AcceptedBatchEvent] {
        &self.accepted_events
    }

    pub(crate) const fn operation_count(&self) -> u64 {
        self.operation_count
    }

    pub(crate) const fn declaration_count(&self) -> u64 {
        self.declaration_count
    }

    /// Reopen the retained operation spool. Chunk 2's manifest-intent sink is
    /// this handle's other consumer; chunk 1 only proves the spool survived the
    /// working-directory removal so the capability stays one artifact.
    pub(crate) fn open_operations(
        &self,
    ) -> Result<BootstrapOperationSpoolReader, BootstrapStreamingImportError> {
        Ok(BootstrapOperationSpoolReader::open(&self.operations)?)
    }

    pub(crate) fn operations_path(&self) -> &Path {
        &self.operations
    }
}

impl Drop for TerminalBootstrapConstructionMaterial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.operations);
    }
}

#[allow(dead_code)]
pub(crate) struct InactiveBootstrapPreparedPartCursor {
    ordinal: u32,
    manifest: Vec<u8>,
    evidence: Vec<u8>,
    spans: Vec<u8>,
    objects: FrameReader,
}

#[allow(dead_code)]
impl InactiveBootstrapPreparedPartCursor {
    pub(crate) const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub(crate) fn manifest_bytes(&self) -> &[u8] {
        &self.manifest
    }

    pub(crate) fn evidence(
        &self,
    ) -> Result<BootstrapImportPartEvidenceV1, BootstrapStreamingImportError> {
        BootstrapImportPartEvidenceV1::decode(&self.evidence).map_err(Into::into)
    }

    pub(crate) fn span_index(
        &self,
    ) -> Result<BootstrapPartSpanIndexV1, BootstrapStreamingImportError> {
        BootstrapPartSpanIndexV1::decode(&self.spans).map_err(Into::into)
    }

    pub(crate) fn next_object_bytes(
        &mut self,
    ) -> Result<Option<Vec<u8>>, BootstrapStreamingImportError> {
        self.objects.next().map_err(Into::into)
    }
}

fn invalid_bootstrap_orchestration(detail: impl Into<String>) -> BootstrapStreamingImportError {
    BootstrapStreamingImportError::InvalidOperation(detail.into())
}

fn elapsed_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn validate_inactive_bootstrap_preparation(
    prepared: &InactiveBootstrapPreparedPublication,
    store: &ObjectStore,
    storage_binding: ProjectionStorageBinding,
) -> Result<
    (
        ControlDirectoryIdentity,
        BootstrapAggregateHistoryBindingV1,
        AcceptedFrontierRoot,
        EngineHistoryBinding,
        u64,
    ),
    BootstrapStreamingImportError,
> {
    let aggregate_bytes = prepared.aggregate_bytes()?;
    let sealed_aggregate = BootstrapAggregateManifestV1::decode(&aggregate_bytes)?;
    if sealed_aggregate != prepared.aggregate {
        return Err(invalid_bootstrap_orchestration(
            "sealed aggregate differs from the prepared aggregate",
        ));
    }
    let commit_bytes = prepared.commit_bytes()?;
    let sealed_commit = BootstrapAggregateCommitV1::decode(&commit_bytes)?;
    if sealed_commit != prepared.commit {
        return Err(invalid_bootstrap_orchestration(
            "sealed commit differs from the prepared commit",
        ));
    }
    sealed_commit.validate_aggregate(&sealed_aggregate)?;

    let aggregate = &prepared.aggregate;
    if store.workspace_id() != aggregate.workspace_id()
        || storage_binding.endpoint.graph_resource_id != aggregate.graph_resource()
        || prepared.source_capture.source_file_count()
            != u64::from(aggregate.source_inventory_root().source_count())
        || prepared.source_capture.source_chunk_count()
            != u64::from(aggregate.source_blob_root().chunk_count())
        || prepared.candidate.part_count() != aggregate.parts().len() as u32
        || prepared.candidate.last_part() != aggregate.final_frontier().last_part()
        || prepared.engine_materials.len() != aggregate.parts().len()
        || prepared.candidate.index_archive_identity()
            != prepared.reference_catalog_archive_identity
    {
        return Err(invalid_bootstrap_orchestration(
            "workspace, graph, capture, candidate, or aggregate identity mismatch",
        ));
    }
    // Every accepted cold record about to be installed binds a reference
    // catalog root that was built in one exact archive's durable authenticated
    // store. Installing that history into any other archive would bind a root
    // that archive cannot open.
    if store.canonical_archive_identity()? != prepared.reference_catalog_archive_identity {
        return Err(invalid_bootstrap_orchestration(
            "bootstrap preparation was authored against a different archive's reference catalog",
        ));
    }

    let bootstrap_binding = BootstrapAggregateHistoryBindingV1::for_aggregate(aggregate)?;
    let accepted_frontier = prepared.candidate.accepted_frontier_root()?;
    let engine_binding = prepared.candidate.durable_history_binding();
    let mut object_count = 0_u64;
    if aggregate.parts().is_empty() {
        if engine_binding != EngineHistoryBinding::empty() {
            return Err(invalid_bootstrap_orchestration(
                "zero-part detached candidate is not canonical empty state",
            ));
        }
    }
    let reference_catalog = store.open_reference_catalog()?;

    for (ordinal, (descriptor, material)) in aggregate
        .parts()
        .iter()
        .copied()
        .zip(prepared.engine_materials.iter())
        .enumerate()
    {
        let mut part = prepared.open_part(ordinal as u32)?;
        let evidence = part.evidence()?;
        let spans = part.span_index()?;
        let manifest = OperationBatch::decode(part.manifest_bytes())
            .map_err(|error| invalid_bootstrap_orchestration(error.to_string()))?;
        let manifest_digest = ContentDigest::of(part.manifest_bytes());
        if part.ordinal() != ordinal as u32
            || evidence != descriptor.evidence()
            || manifest.origin() != BatchOrigin::BootstrapImport
            || manifest.workspace_id() != aggregate.workspace_id()
            || manifest.lineage_digest() != aggregate.lineage_digest()
            || manifest.batch_id() != descriptor.batch_id()
            || material.accepted_evidence().batch_id() != descriptor.batch_id()
            || material.accepted_evidence().acceptance_sequence()
                != u64::from(descriptor.acceptance_sequence())
            || material.accepted_evidence().manifest_fingerprint() != manifest_digest
            || material.reference_catalog_policy() != &prepared.reference_catalog_policy
        {
            return Err(invalid_bootstrap_orchestration(
                "prepared part, manifest, descriptor, or engine material mismatch",
            ));
        }
        // This exact record is about to be installed as durable history that
        // names `reference_catalog_root`. Authoring built that root in this
        // archive's durable catalog; prove the archive holds it before any
        // history record can name it.
        reference_catalog
            .require_catalog_root_nodes(material.reference_catalog_root())
            .map_err(|error| {
                invalid_bootstrap_orchestration(format!(
                    "archive is missing the durable reference catalog this bootstrap part binds: \
                     {error}"
                ))
            })?;
        spans.validate_part(descriptor.evidence())?;
        let span_bytes = spans.encode()?;
        let payload_descriptors = manifest
            .required_objects()
            .iter()
            .map(|object| {
                PayloadObjectDescriptorV1::new(
                    object.content_digest(),
                    object.encoded_byte_length(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        object_count = object_count
            .checked_add(manifest.required_objects().len() as u64)
            .ok_or_else(|| invalid_bootstrap_orchestration("prepared object count overflow"))?;
        descriptor.validate_loaded_artifacts(
            BootstrapManifestFingerprintV1::from_bytes(*manifest_digest.as_bytes()),
            &payload_descriptors,
            &[FullObjectDescriptorV1::manifest_defined(
                *ContentDigest::of(&span_bytes).as_bytes(),
                span_bytes.len() as u64,
            )?],
        )?;
        for expected in manifest.required_objects() {
            let bytes = part.next_object_bytes()?.ok_or_else(|| {
                invalid_bootstrap_orchestration("prepared part object stream ended early")
            })?;
            let object = OperationObject::decode(&bytes)
                .map_err(|error| invalid_bootstrap_orchestration(error.to_string()))?;
            if object
                .descriptor()
                .map_err(|error| invalid_bootstrap_orchestration(error.to_string()))?
                != *expected
            {
                return Err(invalid_bootstrap_orchestration(
                    "prepared part object differs from its manifest descriptor",
                ));
            }
        }
        if part.next_object_bytes()?.is_some() {
            return Err(invalid_bootstrap_orchestration(
                "prepared part object stream has trailing objects",
            ));
        }

        let record_bytes = material.encode_history_record(descriptor, bootstrap_binding)?;
        PreparedBootstrapHistoryRecordV1::new(descriptor, &record_bytes, bootstrap_binding)?;
    }
    if let Some(material) = prepared.engine_materials.last() {
        if material.accepted_evidence().post_frontier_root() != &accepted_frontier
            || material.history_binding() != &engine_binding
        {
            return Err(invalid_bootstrap_orchestration(
                "terminal engine material differs from the prepared detached candidate",
            ));
        }
    }

    Ok((
        store.canonical_archive_identity()?,
        bootstrap_binding,
        accepted_frontier,
        engine_binding,
        object_count,
    ))
}

fn publish_inactive_bootstrap_prefix(
    prepared: &InactiveBootstrapPreparedPublication,
    store: &ObjectStore,
    instrumentation: &mut InactiveBootstrapOrchestrationInstrumentation,
) -> Result<DurablyStagedBootstrapPrefix, BootstrapStreamingImportError> {
    let aggregate = prepared.aggregate();
    let mut publication = store.begin_bootstrap_publication_batch()?;
    for ordinal in 0..aggregate.source_inventory_page_count() {
        let page = prepared.source_inventory_page(ordinal)?;
        publication.publish_source_inventory_page(aggregate.source_inventory_root(), &page)?;
        instrumentation.source_inventory_pages += 1;
    }
    for ordinal in 0..aggregate.source_blob_page_count() {
        let page = prepared.source_blob_page(ordinal)?;
        publication.publish_source_blob_page(aggregate.source_blob_root(), &page)?;
        instrumentation.source_blob_pages += 1;
    }
    let mut chunks = prepared.source_capture.chunks_cursor()?;
    while let Some(chunk) = chunks.next()? {
        let mut reader = prepared.source_capture.open_chunk(&chunk)?;
        let capacity = usize::try_from(chunk.description().byte_length()).map_err(|_| {
            invalid_bootstrap_orchestration("source chunk byte length cannot be represented")
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        reader.read_to_end(&mut bytes)?;
        reader.finish()?;
        publication.publish_source_chunk(
            SourceBlobChunkDigestV1::from_bytes(*chunk.description().sha256()),
            &bytes,
        )?;
        instrumentation.source_chunks += 1;
        instrumentation.peak_owned_source_chunks = 1;
    }
    inactive_bootstrap_orchestration_cut(
        InactiveBootstrapOrchestrationCut::AfterSourcePublication,
    )?;

    for (ordinal, descriptor) in aggregate.parts().iter().copied().enumerate() {
        let mut part = prepared.open_part(ordinal as u32)?;
        while let Some(bytes) = part.next_object_bytes()? {
            publication.publish_object_bytes(&bytes)?;
            instrumentation.objects += 1;
        }
        let spans = part.span_index()?;
        publication.publish_part_artifacts(descriptor, part.manifest_bytes(), &spans)?;
        instrumentation.parts += 1;
        instrumentation.peak_owned_parts = 1;
        if ordinal == 0 {
            inactive_bootstrap_orchestration_cut(InactiveBootstrapOrchestrationCut::AfterOnePart)?;
        }
    }
    let durable = publication.finish(aggregate)?;
    instrumentation.durability_syncs = instrumentation.durability_syncs.saturating_add(1);
    inactive_bootstrap_orchestration_cut(InactiveBootstrapOrchestrationCut::AfterAggregatePrefix)?;
    Ok(durable)
}

/// Publish, install, freshly reopen, and verify one complete bootstrap while
/// leaving graph, projection, enrollment, SQLite, and runtime authority
/// untouched.
pub(crate) fn publish_install_verify_inactive_bootstrap(
    prepared: &InactiveBootstrapPreparedPublication,
    store: ObjectStore,
    storage_binding: ProjectionStorageBinding,
) -> Result<InactiveBootstrapVerifiedPublication, BootstrapStreamingImportError> {
    let phase_started = Instant::now();
    let (
        archive_identity,
        bootstrap_binding,
        expected_frontier,
        expected_engine_binding,
        expected_object_count,
    ) = validate_inactive_bootstrap_preparation(prepared, &store, storage_binding)?;
    let aggregate = prepared.aggregate();
    let archive_path = store.root_path().to_path_buf();
    let workspace_id = aggregate.workspace_id();
    let mut instrumentation = InactiveBootstrapOrchestrationInstrumentation::default();
    instrumentation.preparation_validation_micros = elapsed_micros(phase_started);

    let phase_started = Instant::now();
    let mut durable_prefix = None;
    let was_committed = match store.inspect_bootstrap_aggregate(aggregate) {
        BootstrapPublicationInspectionV1::Committed(_) => true,
        BootstrapPublicationInspectionV1::Absent | BootstrapPublicationInspectionV1::Pending => {
            durable_prefix = Some(publish_inactive_bootstrap_prefix(
                prepared,
                &store,
                &mut instrumentation,
            )?);
            false
        }
        BootstrapPublicationInspectionV1::CorruptOrConflicting(error) => {
            return Err(error.into());
        }
    };
    instrumentation.object_part_publication_micros = elapsed_micros(phase_started);

    let phase_started = Instant::now();
    let open = store
        .seal_history_only(storage_binding)
        .map_err(|(_store, error)| error)?;
    if open.binding() != storage_binding {
        return Err(invalid_bootstrap_orchestration(
            "sealed history storage binding changed",
        ));
    }
    let (store, history) = open.into_history().map_err(|(_store, error)| error)?;
    let (existing_generation, existing_root, existing_latest, existing_engine_binding) =
        history.current_with_binding()?;
    let existing_record_count = history.current_record_count()?;
    let (history_already_installed, effective_engine_binding) =
        match history.current_bootstrap_binding()? {
            None => {
                if existing_generation != 0
                    || existing_root != super::object_store::EngineHistoryStore::empty_root()
                    || existing_latest.is_some()
                    || existing_engine_binding != EngineHistoryBinding::empty()
                    || existing_record_count != 0
                {
                    return Err(invalid_bootstrap_orchestration(
                        "bootstrap installation requires empty ordinary history",
                    ));
                }
                (false, expected_engine_binding.clone())
            }
            Some(existing) => {
                if existing != bootstrap_binding
                    || !was_committed
                    || existing_generation != u64::from(bootstrap_binding.part_count())
                    || existing_record_count != u64::from(bootstrap_binding.part_count())
                {
                    return Err(invalid_bootstrap_orchestration(
                        "durable history belongs to a different or incomplete bootstrap",
                    ));
                }
                (true, existing_engine_binding)
            }
        };

    if !was_committed {
        let publication_id = store.commit_durably_staged_bootstrap_aggregate(
            aggregate,
            durable_prefix.expect("new bootstrap publication has a durable prefix"),
        )?;
        instrumentation.durability_syncs = instrumentation.durability_syncs.saturating_add(1);
        if publication_id != aggregate.publication_id() {
            return Err(invalid_bootstrap_orchestration(
                "aggregate commit returned a different publication identity",
            ));
        }
        inactive_bootstrap_orchestration_cut(
            InactiveBootstrapOrchestrationCut::AfterAggregateCommit,
        )?;
    }
    let publication = store.load_bootstrap_publication(aggregate.publication_id())?;
    if publication.aggregate() != aggregate {
        return Err(invalid_bootstrap_orchestration(
            "direct-loaded committed aggregate differs from preparation",
        ));
    }

    if !history_already_installed {
        let mut builder =
            history.begin_publish_many_exact(&publication, effective_engine_binding.clone())?;
        for (descriptor, material) in aggregate
            .parts()
            .iter()
            .copied()
            .zip(prepared.engine_materials.iter())
        {
            let bytes = material.encode_history_record(descriptor, bootstrap_binding)?;
            let record =
                PreparedBootstrapHistoryRecordV1::new(descriptor, &bytes, bootstrap_binding)?;
            builder.push(&record)?;
            instrumentation.cold_records += 1;
            instrumentation.peak_owned_cold_records = 1;
        }
        inactive_bootstrap_orchestration_cut(InactiveBootstrapOrchestrationCut::BeforeHistoryHead)?;
        builder.finish()?;
        inactive_bootstrap_orchestration_cut(InactiveBootstrapOrchestrationCut::AfterHistoryHead)?;
    }
    instrumentation.aggregate_history_commit_micros = elapsed_micros(phase_started);

    drop(history);
    drop(store);

    let phase_started = Instant::now();
    let reopened_store = ObjectStore::open(&archive_path, workspace_id)?;
    if reopened_store.canonical_archive_identity()? != archive_identity {
        return Err(invalid_bootstrap_orchestration(
            "fresh archive open changed retained store identity",
        ));
    }
    let reopened_publication =
        reopened_store.load_bootstrap_publication(aggregate.publication_id())?;
    if reopened_publication.aggregate() != aggregate {
        return Err(invalid_bootstrap_orchestration(
            "fresh direct-loaded aggregate differs from preparation",
        ));
    }
    let open = reopened_store
        .seal_history_only(storage_binding)
        .map_err(|(_store, error)| error)?;
    let (reopened_store, reopened_history) =
        open.into_history().map_err(|(_store, error)| error)?;
    let (history_generation, history_root, latest_batch_id, reopened_engine_binding) =
        reopened_history.current_with_binding()?;
    let cold_record_count = reopened_history.current_record_count()?;
    if reopened_history.current_bootstrap_binding()? != Some(bootstrap_binding)
        || history_generation != u64::from(bootstrap_binding.part_count())
        || cold_record_count != u64::from(bootstrap_binding.part_count())
        || reopened_engine_binding != effective_engine_binding
        || latest_batch_id
            != aggregate
                .parts()
                .last()
                .map(|descriptor| descriptor.batch_id())
    {
        return Err(invalid_bootstrap_orchestration(
            "fresh durable history differs from prepared bootstrap authority",
        ));
    }
    for (descriptor, material) in aggregate
        .parts()
        .iter()
        .copied()
        .zip(prepared.engine_materials.iter())
    {
        let loaded = reopened_history
            .lookup(history_root, descriptor.batch_id())?
            .ok_or_else(|| {
                invalid_bootstrap_orchestration("fresh history is missing an exact cold record")
            })?;
        if !history_already_installed
            && loaded != material.encode_history_record(descriptor, bootstrap_binding)?
        {
            return Err(invalid_bootstrap_orchestration(
                "fresh history cold record bytes differ from preparation",
            ));
        }
        PreparedBootstrapHistoryRecordV1::new(descriptor, &loaded, bootstrap_binding)?;
    }
    drop(reopened_history);
    drop(reopened_store);
    instrumentation.fresh_validation_replay_micros = elapsed_micros(phase_started);

    instrumentation.source_inventory_pages = aggregate.source_inventory_page_count();
    instrumentation.source_blob_pages = aggregate.source_blob_page_count();
    instrumentation.source_chunks = prepared.source_capture.source_chunk_count();
    instrumentation.parts = aggregate.parts().len() as u32;
    instrumentation.objects = expected_object_count;
    instrumentation.cold_records = u64::from(bootstrap_binding.part_count());
    instrumentation.peak_owned_source_chunks = u32::from(instrumentation.source_chunks != 0);
    instrumentation.peak_owned_parts = u32::from(instrumentation.parts != 0);
    instrumentation.peak_owned_cold_records = u32::from(instrumentation.cold_records != 0);

    Ok(InactiveBootstrapVerifiedPublication {
        workspace_id,
        lineage_digest: aggregate.lineage_digest(),
        graph_resource: aggregate.graph_resource(),
        publication_id: aggregate.publication_id(),
        aggregate_digest: aggregate.aggregate_digest(),
        import_id: aggregate.import_id(),
        part_count: aggregate.parts().len() as u32,
        predecessor_terminal: aggregate.final_frontier().last_part(),
        accepted_frontier: expected_frontier,
        engine_binding: reopened_engine_binding,
        storage_binding,
        bootstrap_binding,
        archive_identity,
        history_generation,
        history_root,
        cold_record_count,
        catalog_document_id: prepared.catalog_document_id,
        reference_catalog_policy: prepared.reference_catalog_policy.clone(),
        instrumentation,
    })
}

/// Carry the one detached candidate validated during uninterrupted authoring
/// across publication and SQLite construction. Durable aggregate/history roots
/// are freshly reopened here, but semantic payloads are not replayed into a
/// second engine. A new process still uses the full replaying reopen below.
pub(crate) fn retain_inactive_bootstrap_accepted_authority(
    prepared: &InactiveBootstrapPreparedPublication,
    verified: &InactiveBootstrapVerifiedPublication,
    store: ObjectStore,
) -> Result<InactiveBootstrapAcceptedAuthority, BootstrapStreamingImportError> {
    let archive_identity = store.canonical_archive_identity()?;
    if store.workspace_id() != verified.workspace_id
        || archive_identity != verified.archive_identity
        || prepared.reference_catalog_archive_identity != archive_identity
        || prepared.candidate.index_archive_identity() != archive_identity
    {
        return Err(invalid_bootstrap_orchestration(
            "retained bootstrap store or preparation identity changed",
        ));
    }
    let publication = store.load_bootstrap_publication(verified.publication_id)?;
    let aggregate = publication.aggregate();
    let bootstrap_binding = BootstrapAggregateHistoryBindingV1::for_aggregate(aggregate)?;
    if aggregate != prepared.aggregate()
        || bootstrap_binding != verified.bootstrap_binding
        || prepared.candidate.part_count() != verified.part_count
        || prepared.candidate.last_part() != verified.predecessor_terminal
        || prepared.candidate.accepted_frontier_root()? != verified.accepted_frontier
        || !prepared
            .candidate
            .durable_history_binding()
            .same_replay_authority(&verified.engine_binding)
    {
        return Err(invalid_bootstrap_orchestration(
            "retained typed candidate differs from committed bootstrap roots",
        ));
    }
    let open = store
        .seal_history_only(verified.storage_binding)
        .map_err(|(_store, error)| error)?;
    let (store, history) = open.into_history().map_err(|(_store, error)| error)?;
    let (history_generation, history_root, latest_batch_id, engine_binding) =
        history.current_with_binding()?;
    let cold_record_count = history.current_record_count()?;
    if history.current_bootstrap_binding()? != Some(bootstrap_binding)
        || history_generation != verified.history_generation
        || history_root != verified.history_root
        || cold_record_count != verified.cold_record_count
        || engine_binding != verified.engine_binding
        || latest_batch_id
            != aggregate
                .parts()
                .last()
                .map(|descriptor| descriptor.batch_id())
    {
        return Err(invalid_bootstrap_orchestration(
            "retained durable history differs from verified publication",
        ));
    }
    drop(history);
    Ok(InactiveBootstrapAcceptedAuthority {
        store,
        publication,
        candidate: Rc::clone(&prepared.candidate),
        binding: InactiveBootstrapAcceptedAuthorityBinding {
            workspace_id: verified.workspace_id,
            lineage_digest: verified.lineage_digest,
            graph_resource: verified.graph_resource,
            publication_id: verified.publication_id,
            aggregate_digest: verified.aggregate_digest,
            import_id: verified.import_id,
            part_count: verified.part_count,
            predecessor_terminal: verified.predecessor_terminal,
            accepted_frontier: verified.accepted_frontier.clone(),
            engine_binding,
            storage_binding: verified.storage_binding,
            bootstrap_binding,
            archive_identity,
            history_generation,
            history_root,
            cold_record_count,
        },
    })
}

/// Freshly reopen and retain the exact accepted authority described by a
/// previously minted inactive-bootstrap publication proof.
pub(crate) fn reopen_inactive_bootstrap_accepted_authority(
    verified: &InactiveBootstrapVerifiedPublication,
    store: ObjectStore,
) -> Result<InactiveBootstrapAcceptedAuthority, BootstrapStreamingImportError> {
    let archive_identity = store.canonical_archive_identity()?;
    if store.workspace_id() != verified.workspace_id
        || archive_identity != verified.archive_identity
    {
        return Err(invalid_bootstrap_orchestration(
            "reopened bootstrap store or archive identity differs from verified publication",
        ));
    }

    let publication = store.load_bootstrap_publication(verified.publication_id)?;
    let aggregate = publication.aggregate();
    let bootstrap_binding = BootstrapAggregateHistoryBindingV1::for_aggregate(aggregate)?;
    if aggregate.workspace_id() != verified.workspace_id
        || aggregate.lineage_digest() != verified.lineage_digest
        || aggregate.graph_resource() != verified.graph_resource
        || aggregate.publication_id() != verified.publication_id
        || aggregate.aggregate_digest() != verified.aggregate_digest
        || aggregate.import_id() != verified.import_id
        || aggregate.parts().len() as u32 != verified.part_count
        || aggregate.final_frontier().last_part() != verified.predecessor_terminal
        || aggregate.final_frontier().accepted_count() != verified.part_count
        || bootstrap_binding != verified.bootstrap_binding
        || verified.storage_binding.endpoint.graph_resource_id != verified.graph_resource
    {
        return Err(invalid_bootstrap_orchestration(
            "direct-loaded aggregate differs from verified inactive publication",
        ));
    }

    let open = store
        .seal_history_only(verified.storage_binding)
        .map_err(|(_store, error)| error)?;
    if open.binding() != verified.storage_binding {
        return Err(invalid_bootstrap_orchestration(
            "reopened bootstrap history storage binding changed",
        ));
    }
    let (store, history) = open.into_history().map_err(|(_store, error)| error)?;
    let (history_generation, history_root, latest_batch_id, engine_binding) =
        history.current_with_binding()?;
    let cold_record_count = history.current_record_count()?;
    if history.current_bootstrap_binding()? != Some(bootstrap_binding)
        || history_generation != verified.history_generation
        || history_generation != u64::from(verified.part_count)
        || history_root != verified.history_root
        || cold_record_count != verified.cold_record_count
        || cold_record_count != u64::from(verified.part_count)
        || engine_binding != verified.engine_binding
        || latest_batch_id
            != aggregate
                .parts()
                .last()
                .map(|descriptor| descriptor.batch_id())
    {
        return Err(invalid_bootstrap_orchestration(
            "fresh durable history differs from verified inactive publication",
        ));
    }

    let replay_identity = DetachedBootstrapReplayIdentity::new(
        verified.workspace_id,
        verified.lineage_digest,
        verified.catalog_document_id,
        verified.reference_catalog_policy.clone(),
        verified.storage_binding,
        archive_identity,
    );
    let (candidate, terminal_history_binding) =
        super::hot_engine::replay_direct_loaded_bootstrap_validating_history(
            &store,
            &publication,
            &replay_identity,
            &history,
            history_root,
            bootstrap_binding,
        )?;
    let accepted_frontier = candidate.accepted_frontier_root()?;
    let candidate_binding = candidate.durable_history_binding();
    if candidate.part_count() != verified.part_count
        || candidate.last_part() != verified.predecessor_terminal
        || accepted_frontier != verified.accepted_frontier
        || !candidate_binding.same_replay_authority(&verified.engine_binding)
        || terminal_history_binding != verified.engine_binding
    {
        return Err(invalid_bootstrap_orchestration(
            "fresh detached replay differs from verified inactive publication",
        ));
    }

    let mut object_count = 0_u64;
    for descriptor in aggregate.parts().iter().copied() {
        object_count = object_count
            .checked_add(u64::from(
                descriptor.evidence().payload_object_root().object_count(),
            ))
            .ok_or_else(|| invalid_bootstrap_orchestration("bootstrap object count overflow"))?;
    }

    let instrumentation = InactiveBootstrapOrchestrationInstrumentation {
        source_inventory_pages: aggregate.source_inventory_page_count(),
        source_blob_pages: aggregate.source_blob_page_count(),
        source_chunks: u64::from(aggregate.source_blob_root().chunk_count()),
        parts: aggregate.parts().len() as u32,
        objects: object_count,
        cold_records: cold_record_count,
        peak_owned_source_chunks: u32::from(aggregate.source_blob_root().chunk_count() != 0),
        peak_owned_parts: u32::from(!aggregate.parts().is_empty()),
        peak_owned_cold_records: u32::from(cold_record_count != 0),
        durability_syncs: 0,
        preparation_validation_micros: 0,
        object_part_publication_micros: 0,
        aggregate_history_commit_micros: 0,
        fresh_validation_replay_micros: 0,
    };
    let mut verified_shape = verified.instrumentation.clone();
    verified_shape.durability_syncs = 0;
    verified_shape.preparation_validation_micros = 0;
    verified_shape.object_part_publication_micros = 0;
    verified_shape.aggregate_history_commit_micros = 0;
    verified_shape.fresh_validation_replay_micros = 0;
    if instrumentation != verified_shape {
        return Err(invalid_bootstrap_orchestration(
            "reopened bootstrap instrumentation differs from verified publication",
        ));
    }

    let binding = InactiveBootstrapAcceptedAuthorityBinding {
        workspace_id: verified.workspace_id,
        lineage_digest: verified.lineage_digest,
        graph_resource: verified.graph_resource,
        publication_id: verified.publication_id,
        aggregate_digest: verified.aggregate_digest,
        import_id: verified.import_id,
        part_count: verified.part_count,
        predecessor_terminal: verified.predecessor_terminal,
        accepted_frontier,
        engine_binding,
        storage_binding: verified.storage_binding,
        bootstrap_binding,
        archive_identity,
        history_generation,
        history_root,
        cold_record_count,
    };
    drop(history);
    Ok(InactiveBootstrapAcceptedAuthority {
        store,
        publication,
        candidate: Rc::new(candidate),
        binding,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SortRecord {
    key: Vec<u8>,
    value: Vec<u8>,
}

struct ExternalSort {
    directory: PathBuf,
    label: String,
    buffer: Vec<SortRecord>,
    buffer_bytes: usize,
    runs: Vec<PathBuf>,
    next_run: usize,
    total_bytes: u64,
    total_runs: u64,
    peak_buffer_bytes: u64,
}

impl ExternalSort {
    fn new(directory: &Path, label: &str) -> Result<Self, BootstrapStreamingImportError> {
        let directory = directory.join(format!("{label}-sort"));
        create_private_directory(&directory)?;
        Ok(Self {
            directory,
            label: label.to_owned(),
            buffer: Vec::new(),
            buffer_bytes: 0,
            runs: Vec::new(),
            next_run: 0,
            total_bytes: 0,
            total_runs: 0,
            peak_buffer_bytes: 0,
        })
    }

    fn push(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), BootstrapStreamingImportError> {
        let bytes = key
            .len()
            .checked_add(value.len())
            .and_then(|bytes| bytes.checked_add(8))
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "external-sort record length overflow".into(),
                )
            })?;
        if bytes > BOOTSTRAP_STREAM_FRAME_BYTES {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "external-sort record bytes",
                observed: bytes as u64,
                limit: BOOTSTRAP_STREAM_FRAME_BYTES as u64,
            });
        }
        if !self.buffer.is_empty()
            && self.buffer_bytes.saturating_add(bytes) > BOOTSTRAP_STREAM_SORT_BUFFER_BYTES
        {
            self.flush()?;
        }
        self.buffer.push(SortRecord { key, value });
        self.buffer_bytes = self.buffer_bytes.saturating_add(bytes);
        self.peak_buffer_bytes = self.peak_buffer_bytes.max(self.buffer_bytes as u64);
        if self.buffer_bytes >= BOOTSTRAP_STREAM_SORT_BUFFER_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    fn finish(
        mut self,
        destination: &Path,
    ) -> Result<ExternalSortReceipt, BootstrapStreamingImportError> {
        self.flush()?;
        if self.runs.is_empty() {
            write_exact_new(destination, &[])?;
        } else {
            while self.runs.len() > 1 {
                let current = std::mem::take(&mut self.runs);
                for group in current.chunks(BOOTSTRAP_STREAM_SORT_FAN_IN) {
                    let output = self.next_run_path("merge");
                    merge_sort_runs(group, &output)?;
                    self.total_runs = self.total_runs.saturating_add(1);
                    if self.total_runs as usize > BOOTSTRAP_STREAM_MAX_SORT_RUNS {
                        return Err(BootstrapStreamingImportError::ResourceLimit {
                            resource: "external-sort runs",
                            observed: self.total_runs,
                            limit: BOOTSTRAP_STREAM_MAX_SORT_RUNS as u64,
                        });
                    }
                    self.runs.push(output);
                }
                for path in current {
                    fs::remove_file(path)?;
                }
            }
            fs::rename(&self.runs[0], destination)?;
        }
        Ok(ExternalSortReceipt {
            bytes: self.total_bytes,
            runs: self.total_runs,
            peak_buffer_bytes: self.peak_buffer_bytes,
        })
    }

    fn flush(&mut self) -> Result<(), BootstrapStreamingImportError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.buffer.sort_unstable_by(|left, right| {
            (&left.key, &left.value).cmp(&(&right.key, &right.value))
        });
        let path = self.next_run_path("run");
        let mut writer = BufWriter::new(create_new_file(&path)?);
        for record in self.buffer.drain(..) {
            self.total_bytes = self
                .total_bytes
                .checked_add(write_sort_record(&mut writer, &record)?)
                .ok_or_else(|| {
                    BootstrapStreamingImportError::InvalidOperation(
                        "external-sort byte count overflow".into(),
                    )
                })?;
        }
        writer.flush()?;
        self.runs.push(path);
        self.total_runs = self.total_runs.saturating_add(1);
        if self.total_runs as usize > BOOTSTRAP_STREAM_MAX_SORT_RUNS {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "external-sort runs",
                observed: self.total_runs,
                limit: BOOTSTRAP_STREAM_MAX_SORT_RUNS as u64,
            });
        }
        self.buffer_bytes = 0;
        Ok(())
    }

    fn next_run_path(&mut self, kind: &str) -> PathBuf {
        let path = self
            .directory
            .join(format!("{}-{kind}-{:08}", self.label, self.next_run));
        self.next_run += 1;
        path
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ExternalSortReceipt {
    bytes: u64,
    runs: u64,
    peak_buffer_bytes: u64,
}

struct SortRecordReader {
    reader: BufReader<File>,
}

impl SortRecordReader {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, File::open(path)?),
        })
    }

    fn next(&mut self) -> io::Result<Option<SortRecord>> {
        let Some(key_length) = read_optional_u32(&mut self.reader)? else {
            return Ok(None);
        };
        let value_length = read_u32_io(&mut self.reader)?;
        let total = usize::try_from(key_length)
            .ok()
            .and_then(|key| {
                usize::try_from(value_length)
                    .ok()
                    .and_then(|value| key.checked_add(value))
            })
            .ok_or_else(|| invalid_bootstrap_data("external-sort record length overflow"))?;
        if total > BOOTSTRAP_STREAM_FRAME_BYTES {
            return Err(invalid_bootstrap_data(
                "external-sort record exceeds its bounded frame",
            ));
        }
        let mut key = vec![0; key_length as usize];
        let mut value = vec![0; value_length as usize];
        self.reader.read_exact(&mut key)?;
        self.reader.read_exact(&mut value)?;
        Ok(Some(SortRecord { key, value }))
    }
}

fn merge_sort_runs(inputs: &[PathBuf], output: &Path) -> Result<(), BootstrapStreamingImportError> {
    let mut readers = inputs
        .iter()
        .map(|path| SortRecordReader::open(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut heads = readers
        .iter_mut()
        .map(SortRecordReader::next)
        .collect::<Result<Vec<_>, _>>()?;
    let mut writer = BufWriter::new(create_new_file(output)?);
    loop {
        let next = heads
            .iter()
            .enumerate()
            .filter_map(|(index, record)| record.as_ref().map(|record| (index, record)))
            .min_by(|(_, left), (_, right)| {
                (&left.key, &left.value).cmp(&(&right.key, &right.value))
            })
            .map(|(index, _)| index);
        let Some(index) = next else {
            break;
        };
        let record = heads[index]
            .take()
            .expect("selected external-sort input has a record");
        write_sort_record(&mut writer, &record)?;
        heads[index] = readers[index].next()?;
    }
    writer.flush()?;
    Ok(())
}

fn write_sort_record(
    writer: &mut impl Write,
    record: &SortRecord,
) -> Result<u64, BootstrapStreamingImportError> {
    let key_length = u32::try_from(record.key.len()).map_err(|_| {
        BootstrapStreamingImportError::InvalidOperation(
            "external-sort key length cannot be represented".into(),
        )
    })?;
    let value_length = u32::try_from(record.value.len()).map_err(|_| {
        BootstrapStreamingImportError::InvalidOperation(
            "external-sort value length cannot be represented".into(),
        )
    })?;
    writer.write_all(&key_length.to_be_bytes())?;
    writer.write_all(&value_length.to_be_bytes())?;
    writer.write_all(&record.key)?;
    writer.write_all(&record.value)?;
    Ok(8 + u64::from(key_length) + u64::from(value_length))
}

pub(crate) struct FrameReader {
    reader: BufReader<File>,
    max_frame: usize,
}

impl FrameReader {
    fn open(path: &Path, max_frame: usize) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, File::open(path)?),
            max_frame,
        })
    }

    fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
        let Some(length) = read_optional_u32(&mut self.reader)? else {
            return Ok(None);
        };
        let length = length as usize;
        if length > self.max_frame {
            return Err(invalid_bootstrap_data(
                "sealed bootstrap frame exceeds its declared bound",
            ));
        }
        let mut bytes = vec![0; length];
        self.reader.read_exact(&mut bytes)?;
        Ok(Some(bytes))
    }
}

fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> io::Result<u64> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| invalid_bootstrap_data("bootstrap frame length cannot be represented"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(bytes)?;
    Ok(4 + u64::from(length))
}

fn read_optional_u32(reader: &mut impl Read) -> io::Result<Option<u32>> {
    let mut bytes = [0; 4];
    let mut read = 0;
    while read != bytes.len() {
        let current = reader.read(&mut bytes[read..])?;
        if current == 0 {
            return if read == 0 {
                Ok(None)
            } else {
                Err(invalid_bootstrap_data("truncated bootstrap frame length"))
            };
        }
        read += current;
    }
    Ok(Some(u32::from_be_bytes(bytes)))
}

fn read_u32_io(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn invalid_bootstrap_data(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.into())
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(invalid_bootstrap_data(
            "bootstrap scratch path is not a real directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path),
        Err(error) => Err(error),
    }
}

fn create_new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn write_exact_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new_file(path)?;
    file.write_all(bytes)
}

fn publish_exact_file(path: &Path, bytes: &[u8]) -> Result<(), BootstrapStreamingImportError> {
    match create_new_file(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if read_bounded_file(path, bytes.len())? == bytes {
                Ok(())
            } else {
                Err(BootstrapStreamingImportError::ConflictingSeal)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn read_bounded_file(path: &Path, max_bytes: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_bootstrap_data(
            "bootstrap artifact is not a regular no-follow file",
        ));
    }
    let length = usize::try_from(metadata.len())
        .map_err(|_| invalid_bootstrap_data("bootstrap artifact length cannot be represented"))?;
    if length > max_bytes {
        return Err(invalid_bootstrap_data(
            "bootstrap artifact exceeds its bounded length",
        ));
    }
    let mut bytes = Vec::with_capacity(length);
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() != length {
        return Err(invalid_bootstrap_data(
            "bootstrap artifact changed while it was read",
        ));
    }
    Ok(bytes)
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_bootstrap_data("bootstrap path has no parent"))?;
    File::open(parent)?.sync_all()
}

fn numbered_path(directory: &Path, ordinal: u32) -> PathBuf {
    directory.join(format!("{ordinal:08}.bin"))
}

struct BootstrapSourceReader<'a> {
    capture: &'a BootstrapSourceCapture,
    chunks: crate::model::BootstrapSourceChunkCursor,
    next_chunk: Option<BootstrapSourceChunk>,
}

impl<'a> BootstrapSourceReader<'a> {
    fn new(capture: &'a BootstrapSourceCapture) -> io::Result<Self> {
        let mut chunks = capture.chunks_cursor()?;
        let next_chunk = chunks.next()?;
        Ok(Self {
            capture,
            chunks,
            next_chunk,
        })
    }

    fn read_entry(
        &mut self,
        entry: &BootstrapSourceEntry,
        instrumentation: &mut BootstrapStreamingImportInstrumentation,
    ) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        let declared = entry.description().byte_length();
        if declared > MAX_SOURCE_FILE_BYTES {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "source file bytes",
                observed: declared,
                limit: MAX_SOURCE_FILE_BYTES,
            });
        }
        let capacity = usize::try_from(declared).map_err(|_| {
            BootstrapStreamingImportError::InvalidSource(
                "source file length cannot be represented".into(),
            )
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        for ordinal in 0..entry.chunk_count() {
            let chunk = self.next_chunk.take().ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed source {} is missing chunk {ordinal}",
                    entry.path()
                ))
            })?;
            if chunk.path() != entry.path() || chunk.ordinal() != ordinal {
                return Err(BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed source chunk order differs for {}",
                    entry.path()
                )));
            }
            let mut reader = self.capture.open_chunk(&chunk)?;
            reader.read_to_end(&mut bytes)?;
            reader.finish()?;
            self.next_chunk = self.chunks.next()?;
        }
        if bytes.len() as u64 != declared || BlobDescription::of(&bytes) != entry.description() {
            return Err(BootstrapStreamingImportError::InvalidSource(format!(
                "sealed source bytes differ for {}",
                entry.path()
            )));
        }
        instrumentation.source_bytes_read = instrumentation
            .source_bytes_read
            .checked_add(declared)
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(
                    "source byte instrumentation overflow".into(),
                )
            })?;
        instrumentation.peak_owned_source_bytes =
            instrumentation.peak_owned_source_bytes.max(declared);
        Ok(bytes)
    }

    fn finish(self) -> Result<(), BootstrapStreamingImportError> {
        if self.next_chunk.is_some() {
            return Err(BootstrapStreamingImportError::InvalidSource(
                "sealed source capture has trailing chunks".into(),
            ));
        }
        Ok(())
    }
}

struct BootstrapSourceProtocolPreparation {
    import_id: ImportId,
    inventory_root: SourceInventoryRootV1,
    inventory_page_count: u32,
    blob_root: SourceBlobChunkRootV1,
    blob_page_count: u32,
    source_count: u32,
}

fn prepare_bootstrap_source_protocol(
    workspace_id: WorkspaceId,
    capture: &BootstrapSourceCapture,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<BootstrapSourceProtocolPreparation, BootstrapStreamingImportError> {
    let source_count = u32::try_from(capture.source_file_count()).map_err(|_| {
        BootstrapStreamingImportError::ResourceLimit {
            resource: "source files",
            observed: capture.source_file_count(),
            limit: super::bootstrap_import::MAX_SOURCE_INVENTORY_LEAVES as u64,
        }
    })?;
    if source_count > super::bootstrap_import::MAX_SOURCE_INVENTORY_LEAVES {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source files",
            observed: u64::from(source_count),
            limit: super::bootstrap_import::MAX_SOURCE_INVENTORY_LEAVES as u64,
        });
    }
    if capture.source_chunk_count() > u64::from(super::bootstrap_import::MAX_SOURCE_BLOB_CHUNKS) {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source chunks",
            observed: capture.source_chunk_count(),
            limit: super::bootstrap_import::MAX_SOURCE_BLOB_CHUNKS as u64,
        });
    }

    let blob_sorted = working.join("source-blob-records.sorted");
    let names_sorted = working.join("logical-page-names.sorted");
    let mut blob_sort = ExternalSort::new(working, "source-blobs")?;
    let mut names_sort = ExternalSort::new(working, "logical-names")?;
    let mut inventory_root = SourceInventoryRootBuilderV1::new();
    let mut derivation =
        ImportIdDerivation::new(workspace_id, 0, source_count as usize, DIFF_SCHEMA_VERSION)
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
    derivation
        .begin_inventory()
        .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;

    let mut entries = capture.entries_cursor()?;
    let mut chunks = capture.chunks_cursor()?;
    let mut next_chunk = chunks.next()?;
    let mut observed_sources = 0_u32;
    let mut total_source_bytes = 0_u64;
    while let Some(entry) = entries.next()? {
        observed_sources = observed_sources.checked_add(1).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidSource("source count overflow".into())
        })?;
        let description = entry.description();
        total_source_bytes = total_source_bytes
            .checked_add(description.byte_length())
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource("total source bytes overflow".into())
            })?;
        if total_source_bytes > MAX_TOTAL_SOURCE_BYTES {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "total source bytes",
                observed: total_source_bytes,
                limit: MAX_TOTAL_SOURCE_BYTES,
            });
        }
        let leaf = SourceLeafV1::new(
            entry.kind(),
            entry.path().clone(),
            SourceContentDigestV1::from_bytes(*description.sha256()),
            description.byte_length(),
        )?;
        inventory_root.push(&leaf)?;
        derivation
            .push_inventory(&ImportInventoryEntry::with_kind(
                entry.kind(),
                entry.path().clone(),
                ImportInventoryState::Present(description),
            ))
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;

        let logical_name = LogicalPageName::parse(entry.logical_name().to_owned())
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
        names_sort.push(
            logical_name.key_digest().as_bytes().to_vec(),
            entry.path().as_str().as_bytes().to_vec(),
        )?;

        let leaf_digest = leaf.digest();
        let mut marker_key = leaf_digest.as_bytes().to_vec();
        marker_key.push(0);
        blob_sort.push(marker_key, leaf.encode())?;
        let mut offset = 0_u64;
        for ordinal in 0..entry.chunk_count() {
            let chunk = next_chunk.take().ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed source {} is missing chunk {ordinal}",
                    entry.path()
                ))
            })?;
            if chunk.path() != entry.path() || chunk.ordinal() != ordinal {
                return Err(BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed chunk order differs for {}",
                    entry.path()
                )));
            }
            let byte_length = u32::try_from(chunk.description().byte_length()).map_err(|_| {
                BootstrapStreamingImportError::InvalidSource(
                    "source chunk length cannot be represented".into(),
                )
            })?;
            let descriptor = SourceBlobChunkDescriptorV1::new(
                leaf_digest,
                ordinal,
                entry.chunk_count(),
                offset,
                byte_length,
                SourceBlobChunkDigestV1::from_bytes(*chunk.description().sha256()),
            )?;
            let mut descriptor_key = leaf_digest.as_bytes().to_vec();
            descriptor_key.push(1);
            descriptor_key.extend_from_slice(&ordinal.to_be_bytes());
            blob_sort.push(descriptor_key, descriptor.encode())?;
            offset = offset.checked_add(u64::from(byte_length)).ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource("source chunk offset overflow".into())
            })?;
            next_chunk = chunks.next()?;
        }
        if offset != description.byte_length() {
            return Err(BootstrapStreamingImportError::InvalidSource(format!(
                "sealed chunk lengths do not cover {}",
                entry.path()
            )));
        }
    }
    if observed_sources != source_count || next_chunk.is_some() {
        return Err(BootstrapStreamingImportError::InvalidSource(
            "sealed source entry/chunk counts differ from the capture".into(),
        ));
    }
    let inventory_root = inventory_root.finish();
    let import_id = derivation
        .finish()
        .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
    let blob_receipt = blob_sort.finish(&blob_sorted)?;
    let name_receipt = names_sort.finish(&names_sorted)?;
    record_sort_receipt(instrumentation, blob_receipt);
    record_sort_receipt(instrumentation, name_receipt);
    let blob_root = build_source_blob_root(&blob_sorted)?;
    let inventory_pages = working.join(BOOTSTRAP_STREAM_INVENTORY_PAGES);
    let blob_pages = working.join(BOOTSTRAP_STREAM_BLOB_PAGES);
    create_private_directory(&inventory_pages)?;
    create_private_directory(&blob_pages)?;
    let inventory_page_count =
        write_source_inventory_pages(capture, inventory_root, &inventory_pages)?;
    let blob_page_count = write_source_blob_pages(&blob_sorted, blob_root, &blob_pages)?;
    instrumentation.source_files = u64::from(source_count);
    instrumentation.source_chunks = capture.source_chunk_count();
    instrumentation.source_bytes = total_source_bytes;
    Ok(BootstrapSourceProtocolPreparation {
        import_id,
        inventory_root,
        inventory_page_count,
        blob_root,
        blob_page_count,
        source_count,
    })
}

fn record_sort_receipt(
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
    receipt: ExternalSortReceipt,
) {
    instrumentation.operation_spool_bytes = instrumentation
        .operation_spool_bytes
        .saturating_add(receipt.bytes);
    instrumentation.external_sort_runs = instrumentation
        .external_sort_runs
        .saturating_add(receipt.runs);
    instrumentation.peak_owned_sort_buffer_bytes = instrumentation
        .peak_owned_sort_buffer_bytes
        .max(receipt.peak_buffer_bytes);
}

/// Select the deterministic bootstrap page set while retaining the capture's
/// complete exact source inventory and blob evidence.
///
/// Entries are sealed in exact-path order. The first member that does not
/// collide with an already selected effective name or portable path wins,
/// matching graph discovery's deterministic path ordering. Later members stay
/// in the authenticated source protocol but acquire no semantic operations.
pub(crate) fn bootstrap_authoritative_source_paths(
    capture: &BootstrapSourceCapture,
) -> Result<HashSet<ManagedPath>, BootstrapStreamingImportError> {
    let mut logical_names = HashSet::new();
    let mut portable_paths = HashSet::new();
    let mut authoritative = HashSet::new();
    let mut entries = capture.entries_cursor()?;
    while let Some(entry) = entries.next()? {
        let logical_name = LogicalPageName::parse(entry.logical_name().to_owned())
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
        let logical_key = logical_name.key_digest();
        let portable_key = entry.path().portable_key();
        if logical_names.contains(&logical_key) || portable_paths.contains(&portable_key) {
            continue;
        }
        logical_names.insert(logical_key);
        portable_paths.insert(portable_key);
        authoritative.insert(entry.path().clone());
    }
    Ok(authoritative)
}

fn build_source_blob_root(
    sorted: &Path,
) -> Result<SourceBlobChunkRootV1, BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(sorted)?;
    let mut builder = SourceBlobChunkRootBuilderV1::new();
    while let Some(record) = reader.next()? {
        match record.key.get(32).copied() {
            Some(0) => builder.begin_source(&SourceLeafV1::decode(&record.value)?)?,
            Some(1) => builder.push(SourceBlobChunkDescriptorV1::decode(&record.value)?)?,
            _ => {
                return Err(BootstrapStreamingImportError::InvalidSource(
                    "invalid source-blob sort record".into(),
                ))
            }
        }
    }
    builder.finish().map_err(Into::into)
}

fn write_source_inventory_pages(
    capture: &BootstrapSourceCapture,
    root: SourceInventoryRootV1,
    directory: &Path,
) -> Result<u32, BootstrapStreamingImportError> {
    let mut builder = SourceInventoryIndexBuilderV1::new(root);
    let mut entries = capture.entries_cursor()?;
    let mut pages = 0_u32;
    while let Some(entry) = entries.next()? {
        let leaf = SourceLeafV1::new(
            entry.kind(),
            entry.path().clone(),
            SourceContentDigestV1::from_bytes(*entry.description().sha256()),
            entry.description().byte_length(),
        )?;
        if let Some(page) = builder.push(leaf)? {
            write_exact_new(
                &numbered_path(directory, page.page_ordinal()),
                &page.encode()?,
            )?;
            pages += 1;
        }
    }
    if let Some(page) = builder.finish()? {
        write_exact_new(
            &numbered_path(directory, page.page_ordinal()),
            &page.encode()?,
        )?;
        pages += 1;
    }
    if pages > MAX_SOURCE_INDEX_PAGES {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source inventory index pages",
            observed: u64::from(pages),
            limit: u64::from(MAX_SOURCE_INDEX_PAGES),
        });
    }
    Ok(pages)
}

fn write_source_blob_pages(
    sorted: &Path,
    root: SourceBlobChunkRootV1,
    directory: &Path,
) -> Result<u32, BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(sorted)?;
    let mut builder = SourceBlobIndexBuilderV1::new(root);
    let mut pages = 0_u32;
    while let Some(record) = reader.next()? {
        if record.key.get(32) != Some(&1) {
            continue;
        }
        if let Some(page) = builder.push(SourceBlobChunkDescriptorV1::decode(&record.value)?)? {
            write_exact_new(
                &numbered_path(directory, page.page_ordinal()),
                &page.encode()?,
            )?;
            pages += 1;
        }
    }
    if let Some(page) = builder.finish()? {
        write_exact_new(
            &numbered_path(directory, page.page_ordinal()),
            &page.encode()?,
        )?;
        pages += 1;
    }
    if pages > MAX_SOURCE_INDEX_PAGES {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source blob index pages",
            observed: u64::from(pages),
            limit: u64::from(MAX_SOURCE_INDEX_PAGES),
        });
    }
    Ok(pages)
}

#[derive(Clone)]
struct BootstrapOperationRecord {
    operation: SemanticOperation,
    source_leaf: SourceLeafDigestV1,
    source_offset: u64,
    source_length: u64,
    canonical_bytes: Vec<u8>,
}

impl BootstrapOperationRecord {
    fn new(
        operation: SemanticOperation,
        source_leaf: SourceLeafDigestV1,
        source_span: Option<StructuralSpan>,
    ) -> Result<Self, BootstrapStreamingImportError> {
        let canonical_bytes = postcard::to_allocvec(&operation).map_err(|error| {
            BootstrapStreamingImportError::InvalidOperation(format!(
                "cannot encode bootstrap semantic operation: {error}"
            ))
        })?;
        if canonical_bytes.len() > BOOTSTRAP_STREAM_FRAME_BYTES {
            return Err(BootstrapStreamingImportError::SingletonOverLimit(
                "operation record bytes",
            ));
        }
        let (source_offset, source_length) = source_span
            .map(|span| (span.start(), span.end().saturating_sub(span.start())))
            .unwrap_or((0, 0));
        Ok(Self {
            operation,
            source_leaf,
            source_offset,
            source_length,
            canonical_bytes,
        })
    }

    fn encode(&self) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        let operation_length = u32::try_from(self.canonical_bytes.len()).map_err(|_| {
            BootstrapStreamingImportError::InvalidOperation(
                "operation record length cannot be represented".into(),
            )
        })?;
        let mut bytes = Vec::with_capacity(52 + self.canonical_bytes.len());
        bytes.extend_from_slice(self.source_leaf.as_bytes());
        bytes.extend_from_slice(&self.source_offset.to_be_bytes());
        bytes.extend_from_slice(&self.source_length.to_be_bytes());
        bytes.extend_from_slice(&operation_length.to_be_bytes());
        bytes.extend_from_slice(&self.canonical_bytes);
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapStreamingImportError> {
        if bytes.len() < 52 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "truncated operation spool record".into(),
            ));
        }
        let source_leaf = SourceLeafDigestV1::from_bytes(
            bytes[..32]
                .try_into()
                .expect("checked operation record digest length"),
        );
        let source_offset = u64::from_be_bytes(
            bytes[32..40]
                .try_into()
                .expect("checked operation record offset length"),
        );
        let source_length = u64::from_be_bytes(
            bytes[40..48]
                .try_into()
                .expect("checked operation record span length"),
        );
        let operation_length = u32::from_be_bytes(
            bytes[48..52]
                .try_into()
                .expect("checked operation record payload length"),
        ) as usize;
        if operation_length != bytes.len() - 52 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "operation spool record length differs".into(),
            ));
        }
        source_offset.checked_add(source_length).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "operation source span overflows".into(),
            )
        })?;
        let canonical_bytes = bytes[52..].to_vec();
        let operation = postcard::from_bytes(&canonical_bytes).map_err(|error| {
            BootstrapStreamingImportError::InvalidOperation(format!(
                "cannot decode bootstrap semantic operation: {error}"
            ))
        })?;
        if postcard::to_allocvec(&operation)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?
            != canonical_bytes
        {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "operation spool record is not canonical".into(),
            ));
        }
        Ok(Self {
            operation,
            source_leaf,
            source_offset,
            source_length,
            canonical_bytes,
        })
    }

    fn operation_leaf(&self) -> Result<OperationLeafV1, BootstrapStreamingImportError> {
        OperationLeafV1::new(
            OperationDigestV1::from_bytes(*ContentDigest::of(&self.canonical_bytes).as_bytes()),
            self.canonical_bytes.len() as u64,
        )
        .map_err(Into::into)
    }

    fn source_span(&self) -> Result<Option<SourceSpanV1>, BootstrapStreamingImportError> {
        if self.source_length == 0 {
            Ok(None)
        } else {
            SourceSpanV1::new(self.source_leaf, self.source_offset, self.source_length)
                .map(Some)
                .map_err(Into::into)
        }
    }
}

struct BootstrapOperationSpool {
    path: PathBuf,
    operation_count: u64,
    declaration_count: u64,
}

fn page_capsule_sort_key(path: &ManagedPath, sequence: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(path.as_str().len() + 9);
    key.extend_from_slice(path.as_str().as_bytes());
    key.push(0);
    key.extend_from_slice(&sequence.to_be_bytes());
    key
}

fn spool_bootstrap_operations(
    capture: &BootstrapSourceCapture,
    import_id: ImportId,
    workspace_id: WorkspaceId,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<BootstrapOperationSpool, BootstrapStreamingImportError> {
    let authoritative_paths = bootstrap_authoritative_source_paths(capture)?;
    let page_path = working.join("phase-page.sorted");
    let content_path = working.join("phase-content.sorted");
    let capsule_path = working.join("phase-capsule.sorted");
    let identity_candidates_path = working.join("identity-candidates.sorted");
    let identity_path = working.join("phase-identity.sorted");
    let mut page_sort = ExternalSort::new(working, "phase-page")?;
    let mut content_sort = ExternalSort::new(working, "phase-content")?;
    let mut identity_candidates = ExternalSort::new(working, "identity-candidates")?;
    let mut source_reader = BootstrapSourceReader::new(capture)?;
    let mut entries = capture.entries_cursor()?;
    let mut operation_count = 0_u64;
    let mut declaration_count = 0_u64;

    while let Some(entry) = entries.next()? {
        let bytes = source_reader.read_entry(&entry, instrumentation)?;
        if !authoritative_paths.contains(entry.path()) {
            continue;
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            BootstrapStreamingImportError::InvalidSource(format!(
                "captured source {} is not UTF-8",
                entry.path()
            ))
        })?;
        let logical_name = LogicalPageName::parse(entry.logical_name().to_owned())
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
        let leaf = SourceLeafV1::new(
            entry.kind(),
            entry.path().clone(),
            SourceContentDigestV1::from_bytes(*entry.description().sha256()),
            entry.description().byte_length(),
        )?;
        let source_leaf = leaf.digest();
        let page_id = import_id.unmatched_page_id(&ImportLocator::page(entry.path().clone()));
        let home_document_id =
            DocumentId::for_unmatched_import_page(workspace_id, entry.path().as_str().as_bytes());
        let full_span = (!bytes.is_empty())
            .then(|| StructuralSpan::new(0, bytes.len() as u64))
            .transpose()
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;

        let mut parser_instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(entry.path(), bytes.as_slice(), &mut parser_instrumentation)
            .map_err(|block| {
                BootstrapStreamingImportError::InvalidSource(format!(
                    "{}: {}",
                    entry.path(),
                    block.detail
                ))
            })?;
        if tree.nodes.len() as u32 > MAX_PARSED_NODES_PER_SOURCE_FILE {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "parser nodes per source file",
                observed: tree.nodes.len() as u64,
                limit: u64::from(MAX_PARSED_NODES_PER_SOURCE_FILE),
            });
        }
        // Source admission and parsing complete before the first semantic
        // operation for this external document is constructible.
        let page_operation = BootstrapOperationRecord::new(
            SemanticOperation::CreatePage {
                page_id,
                home_document_id,
                name: logical_name,
                path: entry.path().clone(),
                kind: entry.kind(),
            },
            source_leaf,
            full_span,
        )?;
        page_sort.push(
            entry.path().as_str().as_bytes().to_vec(),
            page_operation.encode()?,
        )?;
        operation_count = checked_bootstrap_operation_count(operation_count)?;
        declaration_count += 1;
        instrumentation.parser_nodes = instrumentation
            .parser_nodes
            .checked_add(tree.nodes.len() as u64)
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(
                    "parser-node instrumentation overflow".into(),
                )
            })?;
        instrumentation.peak_owned_parser_nodes = instrumentation
            .peak_owned_parser_nodes
            .max(tree.nodes.len() as u64);
        if tree.preamble.is_some() {
            let preamble = BootstrapOperationRecord::new(
                SemanticOperation::SetPagePreamble {
                    page_id,
                    preamble: tree.preamble.clone(),
                },
                source_leaf,
                full_span,
            )?;
            content_sort.push(page_capsule_sort_key(entry.path(), 0), preamble.encode()?)?;
            operation_count = checked_bootstrap_operation_count(operation_count)?;
        }
        let mut node_ids = Vec::with_capacity(tree.nodes.len());
        for index in 0..tree.nodes.len() {
            let locator = materialize_locator(&tree, index, &mut parser_instrumentation).map_err(
                |block| {
                    BootstrapStreamingImportError::InvalidSource(format!(
                        "{}: {}",
                        entry.path(),
                        block.detail
                    ))
                },
            )?;
            let block_id =
                import_id.unmatched_block_id(&ImportLocator::block(entry.path().clone(), locator));
            let parent = tree.nodes[index].parent.map(|parent| node_ids[parent]);
            node_ids.push(block_id);
            let span = Some(tree.nodes[index].span);
            let operation = BootstrapOperationRecord::new(
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id,
                    },
                    page_id,
                    parent,
                    order: imported_order(tree.nodes[index].sibling_position),
                    content: tree.nodes[index].raw.clone(),
                },
                source_leaf,
                span,
            )?;
            let block_sequence = 1_u64.saturating_add((index as u64).saturating_mul(2));
            content_sort.push(
                page_capsule_sort_key(entry.path(), block_sequence),
                operation.encode()?,
            )?;
            operation_count = checked_bootstrap_operation_count(operation_count)?;

            if tree.nodes[index].raw_ids.len() == 1 {
                if let Ok(logseq_uuid) = LogseqUuid::parse(tree.nodes[index].raw_ids[0].trim()) {
                    let identity = BootstrapOperationRecord::new(
                        SemanticOperation::MutateBlockLogseqIdentity {
                            block: BlockLocation {
                                block_id,
                                home_document_id,
                            },
                            mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid },
                        },
                        source_leaf,
                        span,
                    )?;
                    let content_key = page_capsule_sort_key(entry.path(), block_sequence + 1);
                    let key_length = u32::try_from(content_key.len()).map_err(|_| {
                        BootstrapStreamingImportError::InvalidOperation(
                            "page capsule key length cannot be represented".into(),
                        )
                    })?;
                    let mut value = Vec::with_capacity(4 + content_key.len() + 128);
                    value.extend_from_slice(&key_length.to_be_bytes());
                    value.extend_from_slice(&content_key);
                    value.extend_from_slice(&identity.encode()?);
                    identity_candidates.push(logseq_uuid.as_uuid().as_bytes().to_vec(), value)?;
                }
            }
        }
        let _ = text;
    }
    source_reader.finish()?;

    for (sort, destination) in [
        (page_sort, &page_path),
        (content_sort, &content_path),
        (identity_candidates, &identity_candidates_path),
    ] {
        let receipt = sort.finish(destination)?;
        record_sort_receipt(instrumentation, receipt);
    }
    let identity_count = collapse_unique_identity_candidates(
        &identity_candidates_path,
        &identity_path,
        working,
        instrumentation,
    )?;
    operation_count = operation_count.checked_add(identity_count).ok_or_else(|| {
        BootstrapStreamingImportError::InvalidOperation("bootstrap operation count overflow".into())
    })?;
    require_bootstrap_operation_limit(operation_count)?;
    merge_sort_runs(
        &[content_path.clone(), identity_path.clone()],
        &capsule_path,
    )?;

    let operation_path = working.join(BOOTSTRAP_STREAM_OPERATION_SPOOL);
    let mut output = BufWriter::new(create_new_file(&operation_path)?);
    for phase in [&page_path, &capsule_path] {
        let mut input = File::open(phase)?;
        io::copy(&mut input, &mut output)?;
    }
    output.flush()?;
    instrumentation.operations = operation_count;
    instrumentation.operation_spool_bytes = instrumentation
        .operation_spool_bytes
        .saturating_add(operation_path.metadata()?.len());
    Ok(BootstrapOperationSpool {
        path: operation_path,
        operation_count,
        declaration_count,
    })
}

fn collapse_unique_identity_candidates(
    candidates: &Path,
    destination: &Path,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<u64, BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(candidates)?;
    let mut output_sort = ExternalSort::new(working, "phase-identity")?;
    let mut pending: Option<SortRecord> = None;
    let mut duplicate = false;
    let mut count = 0_u64;
    while let Some(record) = reader.next()? {
        match &pending {
            Some(previous) if previous.key == record.key => {
                duplicate = true;
            }
            Some(_) => {
                if !duplicate {
                    let unique = pending.take().expect("pending identity exists");
                    if unique.value.len() < 4 {
                        return Err(BootstrapStreamingImportError::InvalidOperation(
                            "truncated identity candidate".into(),
                        ));
                    }
                    let key_length =
                        u32::from_be_bytes(unique.value[..4].try_into().unwrap()) as usize;
                    let key_end = 4_usize.checked_add(key_length).ok_or_else(|| {
                        BootstrapStreamingImportError::InvalidOperation(
                            "identity capsule key length overflow".into(),
                        )
                    })?;
                    if key_end > unique.value.len() {
                        return Err(BootstrapStreamingImportError::InvalidOperation(
                            "truncated identity capsule key".into(),
                        ));
                    }
                    output_sort.push(
                        unique.value[4..key_end].to_vec(),
                        unique.value[key_end..].to_vec(),
                    )?;
                    count = count.saturating_add(1);
                }
                pending = Some(record);
                duplicate = false;
            }
            None => pending = Some(record),
        }
    }
    if let Some(unique) = pending {
        if !duplicate {
            if unique.value.len() < 4 {
                return Err(BootstrapStreamingImportError::InvalidOperation(
                    "truncated identity candidate".into(),
                ));
            }
            let key_length = u32::from_be_bytes(unique.value[..4].try_into().unwrap()) as usize;
            let key_end = 4_usize.checked_add(key_length).ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "identity capsule key length overflow".into(),
                )
            })?;
            if key_end > unique.value.len() {
                return Err(BootstrapStreamingImportError::InvalidOperation(
                    "truncated identity capsule key".into(),
                ));
            }
            output_sort.push(
                unique.value[4..key_end].to_vec(),
                unique.value[key_end..].to_vec(),
            )?;
            count = count.saturating_add(1);
        }
    }
    let receipt = output_sort.finish(destination)?;
    record_sort_receipt(instrumentation, receipt);
    Ok(count)
}

fn checked_bootstrap_operation_count(current: u64) -> Result<u64, BootstrapStreamingImportError> {
    let observed = current.checked_add(1).ok_or_else(|| {
        BootstrapStreamingImportError::InvalidOperation("bootstrap operation count overflow".into())
    })?;
    require_bootstrap_operation_limit(observed)?;
    Ok(observed)
}

fn require_bootstrap_operation_limit(count: u64) -> Result<(), BootstrapStreamingImportError> {
    let limit = u64::from(MAX_BOOTSTRAP_PARTS) * u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART);
    if count > limit {
        Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "bootstrap operations",
            observed: count,
            limit,
        })
    } else {
        Ok(())
    }
}

pub(crate) struct BootstrapOperationSpoolReader {
    records: SortRecordReader,
}

impl BootstrapOperationSpoolReader {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            records: SortRecordReader::open(path)?,
        })
    }

    fn next(&mut self) -> Result<Option<BootstrapOperationRecord>, BootstrapStreamingImportError> {
        self.records
            .next()?
            .map(|record| BootstrapOperationRecord::decode(&record.value))
            .transpose()
    }
}

fn partition_bootstrap_operation_spool(
    operations: &BootstrapOperationSpool,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<u32, BootstrapStreamingImportError> {
    #[cfg(test)]
    let max_part_operations = NEXT_BOOTSTRAP_PART_OPERATION_LIMIT
        .with(|limit| limit.replace(None))
        .unwrap_or(MAX_OPERATIONS_PER_BOOTSTRAP_PART);
    #[cfg(not(test))]
    let max_part_operations = MAX_OPERATIONS_PER_BOOTSTRAP_PART;
    if max_part_operations == 0 || max_part_operations > MAX_OPERATIONS_PER_BOOTSTRAP_PART {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "bootstrap part operation limit is invalid".into(),
        ));
    }
    #[derive(Clone, Copy)]
    struct Unit {
        source_leaf: SourceLeafDigestV1,
        operations: u32,
        semantic_bytes: u64,
        spans: u32,
        declarations: bool,
        split_continuation: bool,
    }

    let mut reader = BootstrapOperationSpoolReader::open(&operations.path)?;
    let mut units = Vec::new();
    let mut observed_operations = 0_u64;
    let mut current: Option<Unit> = None;
    let mut current_spans = BTreeSet::new();
    while let Some(operation) = reader.next()? {
        let semantic_bytes = operation.canonical_bytes.len() as u64;
        let partition_bytes = semantic_bytes
            .checked_add(BOOTSTRAP_STREAM_SEMANTIC_EFFECT_OVERHEAD)
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "semantic-effect partition charge overflow".into(),
                )
            })?;
        if partition_bytes > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapStreamingImportError::SingletonOverLimit(
                "semantic-effect bytes",
            ));
        }
        let declarations = observed_operations < operations.declaration_count;
        let source_span = operation.source_span()?;
        let same_capsule = current.is_some_and(|unit| {
            unit.declarations == declarations
                && !declarations
                && unit.source_leaf == operation.source_leaf
        });
        if !same_capsule {
            if let Some(mut unit) = current.take() {
                unit.spans = current_spans.len() as u32;
                units.push(unit);
                current_spans.clear();
            }
            current = Some(Unit {
                source_leaf: operation.source_leaf,
                operations: 0,
                semantic_bytes: 0,
                spans: 0,
                declarations,
                split_continuation: false,
            });
        }
        let adds_span = source_span.is_some_and(|span| !current_spans.contains(&span));
        let unit = current.as_ref().expect("partition unit exists");
        let exceeds = unit.operations == max_part_operations
            || unit.semantic_bytes.saturating_add(partition_bytes)
                > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART
            || (adds_span && current_spans.len() as u32 == MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART);
        if exceeds {
            let mut full = current.take().expect("full partition unit exists");
            if full.operations == 0 {
                return Err(BootstrapStreamingImportError::SingletonOverLimit(
                    "bootstrap part",
                ));
            }
            full.spans = current_spans.len() as u32;
            units.push(full);
            current_spans.clear();
            instrumentation.huge_page_splits = instrumentation.huge_page_splits.saturating_add(1);
            current = Some(Unit {
                source_leaf: operation.source_leaf,
                operations: 0,
                semantic_bytes: 0,
                spans: 0,
                declarations,
                split_continuation: true,
            });
        }
        let unit = current.as_mut().expect("partition unit exists");
        unit.operations += 1;
        unit.semantic_bytes += partition_bytes;
        if let Some(span) = source_span {
            current_spans.insert(span);
        }
        observed_operations += 1;
    }
    if let Some(mut unit) = current {
        unit.spans = current_spans.len() as u32;
        units.push(unit);
    }

    let path = working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL);
    let mut writer = BufWriter::new(create_new_file(&path)?);
    let mut part_count = 0_u32;
    let mut part_operations = 0_u32;
    let mut part_semantic_bytes = 0_u64;
    let mut part_spans = 0_u32;
    let mut part_documents = BTreeSet::new();
    let mut part_declarations = None;
    let flush = |writer: &mut BufWriter<File>,
                 part_operations: &mut u32,
                 part_semantic_bytes: &mut u64,
                 part_spans: &mut u32,
                 part_documents: &mut BTreeSet<SourceLeafDigestV1>,
                 part_declarations: &mut Option<bool>,
                 part_count: &mut u32,
                 instrumentation: &mut BootstrapStreamingImportInstrumentation|
     -> Result<(), BootstrapStreamingImportError> {
        if *part_operations == 0 {
            return Ok(());
        }
        write_frame(writer, &part_operations.to_be_bytes())?;
        *part_count = part_count.checked_add(1).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation("bootstrap part count overflow".into())
        })?;
        if *part_count > MAX_BOOTSTRAP_PARTS {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "bootstrap parts",
                observed: u64::from(*part_count),
                limit: u64::from(MAX_BOOTSTRAP_PARTS),
            });
        }
        instrumentation.max_part_documents = instrumentation
            .max_part_documents
            .max(part_documents.len() as u64);
        *part_operations = 0;
        *part_semantic_bytes = 0;
        *part_spans = 0;
        part_documents.clear();
        *part_declarations = None;
        Ok(())
    };
    for unit in units {
        let changes_phase = part_declarations.is_some_and(|phase| phase != unit.declarations);
        let adds_document = !part_documents.contains(&unit.source_leaf);
        let exceeds = changes_phase
            || part_operations.saturating_add(unit.operations) > max_part_operations
            || part_semantic_bytes.saturating_add(unit.semantic_bytes)
                > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART
            || part_spans.saturating_add(unit.spans) > MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART
            || (unit.declarations
                && adds_document
                && part_documents.len() as u32 == BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART)
            || (!unit.declarations
                && adds_document
                && part_documents.len() as u32 == BOOTSTRAP_STREAM_MAX_PAGE_CAPSULES_PER_PART);
        if exceeds {
            flush(
                &mut writer,
                &mut part_operations,
                &mut part_semantic_bytes,
                &mut part_spans,
                &mut part_documents,
                &mut part_declarations,
                &mut part_count,
                instrumentation,
            )?;
        }
        part_declarations = Some(unit.declarations);
        part_operations += unit.operations;
        part_semantic_bytes += unit.semantic_bytes;
        part_spans += unit.spans;
        part_documents.insert(unit.source_leaf);
        if unit.declarations {
            instrumentation.page_declarations = instrumentation.page_declarations.saturating_add(1);
        } else if !unit.split_continuation {
            instrumentation.page_capsules = instrumentation.page_capsules.saturating_add(1);
        }
    }
    flush(
        &mut writer,
        &mut part_operations,
        &mut part_semantic_bytes,
        &mut part_spans,
        &mut part_documents,
        &mut part_declarations,
        &mut part_count,
        instrumentation,
    )?;
    writer.flush()?;
    if observed_operations != operations.operation_count {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "operation spool count differs during partitioning".into(),
        ));
    }
    if part_count > MAX_BOOTSTRAP_PARTS {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "bootstrap parts",
            observed: u64::from(part_count),
            limit: u64::from(MAX_BOOTSTRAP_PARTS),
        });
    }
    instrumentation.parts = part_count;
    Ok(part_count)
}

struct AuthoredBootstrapParts {
    descriptors: Vec<BootstrapPartDescriptorV1>,
    candidate: Box<DetachedBootstrapCandidate>,
    engine_materials: Vec<DetachedBootstrapAcceptedEngineMaterial>,
    accepted_events: Vec<AcceptedBatchEvent>,
    final_frontier: ArchiveLocalFrontierBindingV1,
}

#[allow(clippy::too_many_arguments)]
fn author_bootstrap_parts(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    reference_catalog: &BootstrapAuthoringCapability,
    import_id: ImportId,
    operation_spool: &BootstrapOperationSpool,
    part_count: u32,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
    progress: &mut dyn FnMut(BootstrapPreparationProgress),
) -> Result<AuthoredBootstrapParts, BootstrapStreamingImportError> {
    let profile_digest = BootstrapPartitionProfileV1::v1().digest();
    // The provisional evidence and the exact descriptor have the same part
    // identity; payload commitment is filled from the prepared bytes below.
    // Keep the typed engine material returned by this one authoring pass rather
    // than replaying the same transaction into a second detached engine.
    let mut authoring = boxed_detached_bootstrap_session(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
        reference_catalog,
    )?;
    let author_device_id = DeviceId::for_external_import_author(workspace_id);
    let author_session_id = SessionId::for_external_import_author(workspace_id, import_id);
    let crdt_peer_id = (0..=1024)
        .map(|attempt| CrdtPeerId::external_import_candidate(workspace_id, import_id, attempt))
        .find(|peer| peer.as_u64() != 0)
        .ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "no nonzero deterministic bootstrap CRDT peer".into(),
            )
        })?;

    let parts_directory = working.join(BOOTSTRAP_STREAM_PARTS);
    create_private_directory(&parts_directory)?;
    let mut boundaries = FrameReader::open(
        &working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL),
        std::mem::size_of::<u32>(),
    )?;
    let mut operations = BootstrapOperationSpoolReader::open(&operation_spool.path)?;
    let mut predecessor = None;
    let mut archive_frontier = ArchiveLocalFrontierBindingV1::initial(import_id, profile_digest);
    let mut descriptors = Vec::with_capacity(part_count as usize);
    let mut engine_materials = Vec::with_capacity(part_count as usize);
    let mut accepted_events = Vec::with_capacity(part_count as usize);
    let mut authored_operations = 0_u64;

    for ordinal in 0..part_count {
        #[cfg(test)]
        let part_started = Instant::now();
        let boundary = boundaries.next()?.ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "part-boundary spool ended early".into(),
            )
        })?;
        if boundary.len() != 4 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "invalid part-boundary frame".into(),
            ));
        }
        let operation_count = u32::from_be_bytes(
            boundary
                .try_into()
                .expect("checked part-boundary frame length"),
        );
        let mut records = Vec::with_capacity(operation_count as usize);
        let mut transaction_operations = Vec::with_capacity(operation_count as usize);
        let mut operation_leaves = Vec::with_capacity(operation_count as usize);
        let mut source_spans = BTreeSet::new();
        for _ in 0..operation_count {
            let record = operations.next()?.ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "operation spool ended before its part boundary".into(),
                )
            })?;
            transaction_operations.push(record.operation.clone());
            operation_leaves.push(record.operation_leaf()?);
            if let Some(span) = record.source_span()? {
                source_spans.insert(span);
            }
            records.push(record);
        }
        authored_operations += u64::from(operation_count);
        let transaction = OperationTransaction::new(transaction_operations)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let source_spans = source_spans.into_iter().collect::<Vec<_>>();
        let source_span_root = SourceSpanRootV1::from_spans(&source_spans)?;
        let operation_root = OperationRootV1::from_operations(&operation_leaves)?;
        let provisional_evidence = BootstrapImportPartEvidenceV1::new(
            import_id,
            profile_digest,
            ordinal,
            part_count,
            source_span_root,
            operation_root,
            PayloadObjectRootV1::empty(),
            predecessor,
        )?;
        let author = AuthorBatch {
            batch_id: provisional_evidence.batch_id(),
            author_device_id,
            author_session_id,
            crdt_peer_id,
        };
        let authored_part = authoring
            .author_part(author, &transaction, provisional_evidence)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let (prepared, engine_material) = authored_part.into_parts();
        let manifest_bytes = prepared
            .manifest()
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let payload_descriptors = prepared_payload_descriptors(&prepared)?;
        let payload_root = PayloadObjectRootV1::from_objects(&payload_descriptors)?;
        validate_prepared_part_limits(&prepared, operation_count)?;
        let evidence = BootstrapImportPartEvidenceV1::new(
            import_id,
            profile_digest,
            ordinal,
            part_count,
            source_span_root,
            operation_root,
            payload_root,
            predecessor,
        )?;
        if evidence.part_id() != provisional_evidence.part_id()
            || evidence.batch_id() != provisional_evidence.batch_id()
        {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "payload commitment changed bootstrap part identity".into(),
            ));
        }
        let manifest_digest = ContentDigest::of(&manifest_bytes);
        let manifest_fingerprint =
            BootstrapManifestFingerprintV1::from_bytes(*manifest_digest.as_bytes());
        let span_index = BootstrapPartSpanIndexV1::new(evidence.part_id(), source_spans.clone())?;
        let span_bytes = span_index.encode()?;
        let part_artifacts = [FullObjectDescriptorV1::manifest_defined(
            *ContentDigest::of(&span_bytes).as_bytes(),
            span_bytes.len() as u64,
        )?];
        let descriptor = BootstrapPartDescriptorV1::accepted(
            evidence,
            manifest_fingerprint,
            &payload_descriptors,
            &part_artifacts,
            archive_frontier,
        )?;
        archive_frontier = descriptor.post_frontier();
        write_prepared_bootstrap_part(
            &parts_directory,
            ordinal,
            evidence,
            &source_spans,
            &manifest_bytes,
            prepared.objects(),
            instrumentation,
        )?;
        instrumentation.peak_owned_part_operations = instrumentation
            .peak_owned_part_operations
            .max(u64::from(operation_count));
        instrumentation.source_spans = instrumentation
            .source_spans
            .saturating_add(source_spans.len() as u64);
        instrumentation.max_part_manifest_bytes = instrumentation
            .max_part_manifest_bytes
            .max(manifest_bytes.len() as u64);
        instrumentation.max_part_payload_descriptors = instrumentation
            .max_part_payload_descriptors
            .max(payload_descriptors.len() as u64);
        instrumentation.max_part_documents = instrumentation.max_part_documents.max(
            engine_material
                .accepted_evidence()
                .affected_documents()
                .len() as u64,
        );
        // Retain the exact accepted event this authoring pass produced, from
        // the same prepared bytes and the same accepted evidence the archive
        // replay path reconstructs. Nothing here asserts acceptance: the
        // evidence is the detached engine's own, and every later consumer
        // re-binds this value to the engine's authenticated history.
        accepted_events.push(
            AcceptedBatchEvent::from_authored_bootstrap_part(
                &super::ValidatedBatch::new(prepared),
                engine_material.accepted_evidence(),
            )
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?,
        );
        descriptors.push(descriptor);
        engine_materials.push(engine_material);
        predecessor = Some(evidence.part_id());
        drop(records);
        progress(BootstrapPreparationProgress::DetachedAuthoring {
            completed: ordinal + 1,
            total: part_count,
        });
        #[cfg(test)]
        if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
            eprintln!(
                "bootstrap authored part {}/{}: {} operations in {} ms",
                ordinal + 1,
                part_count,
                operation_count,
                part_started.elapsed().as_millis()
            );
        }
    }
    if boundaries.next()?.is_some() || operations.next()?.is_some() {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "sealed part boundaries do not consume the exact operation spool".into(),
        ));
    }
    if authored_operations != operation_spool.operation_count {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "authored operation count differs from the sealed spool".into(),
        ));
    }
    let candidate = finish_boxed_detached_bootstrap_session(authoring)?;
    Ok(AuthoredBootstrapParts {
        descriptors,
        candidate,
        engine_materials,
        accepted_events,
        final_frontier: archive_frontier,
    })
}

#[inline(never)]
fn boxed_detached_bootstrap_session(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    reference_catalog: &BootstrapAuthoringCapability,
) -> Result<Box<DetachedBootstrapAuthoringSession>, BootstrapStreamingImportError> {
    DetachedBootstrapAuthoringSession::new(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
        reference_catalog,
    )
    .map(Box::new)
    .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))
}

#[inline(never)]
fn finish_boxed_detached_bootstrap_session(
    session: Box<DetachedBootstrapAuthoringSession>,
) -> Result<Box<DetachedBootstrapCandidate>, BootstrapStreamingImportError> {
    (*session)
        .finish()
        .map(Box::new)
        .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))
}

fn prepared_payload_descriptors(
    prepared: &super::PreparedBatch,
) -> Result<Vec<PayloadObjectDescriptorV1>, BootstrapStreamingImportError> {
    prepared
        .objects()
        .iter()
        .map(|object| {
            let bytes = object.encode().map_err(|error| {
                BootstrapStreamingImportError::InvalidOperation(error.to_string())
            })?;
            PayloadObjectDescriptorV1::new(ContentDigest::of(&bytes), bytes.len() as u64)
                .map_err(Into::into)
        })
        .collect()
}

fn validate_prepared_part_limits(
    prepared: &super::PreparedBatch,
    operation_count: u32,
) -> Result<(), BootstrapStreamingImportError> {
    let manifest = prepared
        .manifest()
        .encode()
        .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
    if manifest.len() > BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES {
        return if operation_count == 1 {
            Err(BootstrapStreamingImportError::SingletonOverLimit(
                "batch manifest bytes",
            ))
        } else {
            Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "batch manifest bytes",
                observed: manifest.len() as u64,
                limit: BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES as u64,
            })
        };
    }
    let mut total = 0_u64;
    let mut semantic = None;
    for object in prepared.objects() {
        let bytes = object
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        total = total.checked_add(bytes.len() as u64).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "prepared object byte count overflow".into(),
            )
        })?;
        if object.kind() == ObjectKind::SemanticEffect {
            semantic = Some(object.payload().len() as u64);
        }
    }
    let semantic = semantic.ok_or_else(|| {
        BootstrapStreamingImportError::InvalidOperation(
            "prepared bootstrap part has no semantic effect".into(),
        )
    })?;
    for (resource, observed, limit) in [
        (
            "semantic-effect bytes",
            semantic,
            MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART,
        ),
        (
            "payload/object bytes",
            total,
            MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART,
        ),
    ] {
        if observed > limit {
            return if operation_count == 1 {
                Err(BootstrapStreamingImportError::SingletonOverLimit(resource))
            } else {
                Err(BootstrapStreamingImportError::ResourceLimit {
                    resource,
                    observed,
                    limit,
                })
            };
        }
    }
    Ok(())
}

fn write_prepared_bootstrap_part(
    parts: &Path,
    ordinal: u32,
    evidence: BootstrapImportPartEvidenceV1,
    source_spans: &[SourceSpanV1],
    manifest_bytes: &[u8],
    objects: &[super::OperationObject],
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<(), BootstrapStreamingImportError> {
    let directory = parts.join(format!("{ordinal:08}"));
    create_private_directory(&directory)?;
    let evidence_bytes = evidence.encode()?;
    let spans = BootstrapPartSpanIndexV1::new(evidence.part_id(), source_spans.to_vec())?;
    let span_bytes = spans.encode()?;
    write_exact_new(
        &directory.join(BOOTSTRAP_STREAM_PART_MANIFEST),
        manifest_bytes,
    )?;
    write_exact_new(
        &directory.join(BOOTSTRAP_STREAM_PART_EVIDENCE),
        &evidence_bytes,
    )?;
    write_exact_new(&directory.join(BOOTSTRAP_STREAM_PART_SPANS), &span_bytes)?;
    let object_path = directory.join(BOOTSTRAP_STREAM_PART_OBJECTS);
    let mut writer = BufWriter::new(create_new_file(&object_path)?);
    let mut object_bytes = 0_u64;
    for object in objects {
        let bytes = object
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        object_bytes = object_bytes.saturating_add(write_frame(&mut writer, &bytes)?);
    }
    writer.flush()?;
    let prepared_bytes = manifest_bytes.len() as u64
        + evidence_bytes.len() as u64
        + span_bytes.len() as u64
        + object_bytes;
    instrumentation.prepared_bytes = instrumentation
        .prepared_bytes
        .saturating_add(prepared_bytes);
    instrumentation.peak_owned_part_bytes =
        instrumentation.peak_owned_part_bytes.max(prepared_bytes);
    Ok(())
}

/// Prepare, but do not publish or admit, one complete multipart bootstrap.
///
/// `capture` must be the sealed capture minted by `Graph`; this function
/// consumes it so the final capture-C proof cannot be separated from the
/// preparation it authorizes. `scratch` is caller-owned disposable storage
/// outside the graph. Only the final `sealed.commit` file marks a reusable
/// preparation; all UUID-named work directories are non-authoritative residue.
///
/// `reference_catalog` must be the durable capability of the exact archive this
/// preparation will be installed into. Authoring binds a `reference_catalog_root`
/// into every accepted cold record; building it anywhere else would produce a
/// root the installed archive could never open. The preparation retains that
/// archive's control-directory identity so installation cannot later be pointed
/// at a different archive.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_inactive_bootstrap_import(
    graph: &Graph,
    capture: BootstrapSourceCapture,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    reference_catalog: &BootstrapAuthoringCapability,
    scratch: &Path,
) -> Result<InactiveBootstrapPreparedPublication, BootstrapStreamingImportError> {
    prepare_inactive_bootstrap_import_with_progress(
        graph,
        capture,
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
        reference_catalog,
        scratch,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_inactive_bootstrap_import_with_progress(
    graph: &Graph,
    capture: BootstrapSourceCapture,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    reference_catalog: &BootstrapAuthoringCapability,
    scratch: &Path,
    mut progress: impl FnMut(BootstrapPreparationProgress),
) -> Result<InactiveBootstrapPreparedPublication, BootstrapStreamingImportError> {
    if reference_catalog.workspace_id() != workspace_id {
        return Err(invalid_bootstrap_orchestration(
            "bootstrap reference-catalog capability belongs to another workspace",
        ));
    }
    prepare_bootstrap_scratch(graph, scratch)?;
    let root = scratch.join(BOOTSTRAP_STREAM_DIRECTORY);
    create_private_directory(&root)?;
    let working = root.join(format!(".building-{}", Uuid::new_v4().simple()));
    create_private_directory(&working)?;
    let artifacts = working.join("artifacts");
    create_private_directory(&artifacts)?;
    let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
    record_capture_instrumentation(&mut instrumentation, capture.instrumentation());

    progress(BootstrapPreparationProgress::Subphase(
        BootstrapPreparationSubphase::SourceProtocol,
    ));
    let phase_started = Instant::now();
    let source =
        prepare_bootstrap_source_protocol(workspace_id, &capture, &working, &mut instrumentation)?;
    instrumentation.source_protocol_micros = elapsed_micros(phase_started);
    #[cfg(test)]
    if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
        eprintln!(
            "bootstrap source protocol: {} ms",
            instrumentation.source_protocol_micros / 1_000
        );
    }
    progress(BootstrapPreparationProgress::Subphase(
        BootstrapPreparationSubphase::OperationSpool,
    ));
    let phase_started = Instant::now();
    let operations = spool_bootstrap_operations(
        &capture,
        source.import_id,
        workspace_id,
        &working,
        &mut instrumentation,
    )?;
    instrumentation.operation_spool_micros = elapsed_micros(phase_started);
    #[cfg(test)]
    if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
        eprintln!(
            "bootstrap operation spool: {} ms",
            instrumentation.operation_spool_micros / 1_000
        );
    }
    progress(BootstrapPreparationProgress::Subphase(
        BootstrapPreparationSubphase::Partition,
    ));
    let phase_started = Instant::now();
    let part_count =
        partition_bootstrap_operation_spool(&operations, &working, &mut instrumentation)?;
    instrumentation.partition_micros = elapsed_micros(phase_started);
    #[cfg(test)]
    if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
        eprintln!(
            "bootstrap partition: {} ms",
            instrumentation.partition_micros / 1_000
        );
    }
    let graph_resource = graph.canonical_resource_id()?;

    // This is deliberately the final source action. Everything below owns only
    // sealed spools and detached engine/scratch capabilities.
    let final_capture = capture.verify_before_inactive_bootstrap_authoring(graph)?;
    record_capture_instrumentation(&mut instrumentation, &final_capture);
    let retained_reference_catalog_policy = reference_catalog_policy.clone();
    progress(BootstrapPreparationProgress::Subphase(
        BootstrapPreparationSubphase::DetachedAuthoring,
    ));
    progress(BootstrapPreparationProgress::DetachedAuthoring {
        completed: 0,
        total: part_count,
    });
    let phase_started = Instant::now();
    let authored = author_bootstrap_parts(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
        reference_catalog,
        source.import_id,
        &operations,
        part_count,
        &working,
        &mut instrumentation,
        &mut progress,
    )?;
    instrumentation.detached_authoring_micros = elapsed_micros(phase_started);
    #[cfg(test)]
    if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
        eprintln!(
            "bootstrap detached authoring: {} ms",
            instrumentation.detached_authoring_micros / 1_000
        );
    }

    let profile_digest = BootstrapPartitionProfileV1::v1().digest();
    let initial_frontier = ArchiveLocalFrontierBindingV1::initial(source.import_id, profile_digest);
    let aggregate = BootstrapAggregateManifestV1::new_for_import(
        workspace_id,
        lineage_digest,
        graph_resource,
        source.import_id,
        source.source_count,
        source.inventory_root,
        source.inventory_page_count,
        source.blob_root,
        source.blob_page_count,
        profile_digest,
        authored.descriptors,
        initial_frontier,
        authored.final_frontier,
    )?;
    let aggregate_bytes = aggregate.encode()?;
    let commit = BootstrapAggregateCommitV1::for_aggregate(&aggregate)?;
    let commit_bytes = commit.encode()?;

    for name in [
        BOOTSTRAP_STREAM_INVENTORY_PAGES,
        BOOTSTRAP_STREAM_BLOB_PAGES,
        BOOTSTRAP_STREAM_PARTS,
    ] {
        fs::rename(working.join(name), artifacts.join(name))?;
    }
    write_exact_new(
        &artifacts.join(BOOTSTRAP_STREAM_AGGREGATE),
        &aggregate_bytes,
    )?;
    write_exact_new(&artifacts.join(BOOTSTRAP_STREAM_COMMIT), &commit_bytes)?;

    progress(BootstrapPreparationProgress::Subphase(
        BootstrapPreparationSubphase::Sealing,
    ));
    let phase_started = Instant::now();
    let sealed_directory = root.join(hex_bootstrap_digest(commit.publication_id().as_bytes()));
    seal_bootstrap_preparation(&artifacts, &sealed_directory, &commit_bytes)?;
    let sealed_aggregate = read_bounded_file(
        &sealed_directory.join(BOOTSTRAP_STREAM_AGGREGATE),
        super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES,
    )?;
    if BootstrapAggregateManifestV1::decode(&sealed_aggregate)? != aggregate {
        return Err(BootstrapStreamingImportError::ConflictingSeal);
    }
    let sealed_commit = read_bounded_file(
        &sealed_directory.join(BOOTSTRAP_STREAM_SEAL),
        super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES,
    )?;
    let decoded_commit = BootstrapAggregateCommitV1::decode(&sealed_commit)?;
    decoded_commit.validate_aggregate(&aggregate)?;
    instrumentation.preparation_sealing_micros = elapsed_micros(phase_started);
    #[cfg(test)]
    if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
        eprintln!(
            "bootstrap preparation sealing: {} ms",
            instrumentation.preparation_sealing_micros / 1_000
        );
    }
    // Move the existing operation spool out of the working directory before it
    // is removed, under a fresh random name in the same preparation prefix. The
    // relocation is a rename of a file this preparation already wrote; it is
    // never fsynced or sealed, and the handle removes it on drop.
    let terminal_construction = retain_terminal_construction_material(
        workspace_id,
        lineage_digest,
        source.import_id,
        &root,
        &operations,
        authored.accepted_events,
    );
    let _ = fs::remove_dir_all(&working);
    progress(BootstrapPreparationProgress::Summary(
        BootstrapPreparationSummary::from(&instrumentation),
    ));

    if authored.candidate.index_archive_identity() != reference_catalog.archive_identity() {
        return Err(invalid_bootstrap_orchestration(
            "detached candidate durability proof belongs to another archive",
        ));
    }

    Ok(InactiveBootstrapPreparedPublication {
        source_capture: capture,
        sealed_directory,
        aggregate,
        commit,
        catalog_document_id,
        reference_catalog_policy: retained_reference_catalog_policy,
        reference_catalog_archive_identity: reference_catalog.archive_identity(),
        candidate: Rc::from(authored.candidate),
        engine_materials: authored.engine_materials,
        terminal_construction,
        instrumentation,
    })
}

/// Retain the process-only terminal construction capability, or `None` if the
/// spool could not be relocated. `None` is not a failure: it only means this
/// activation takes the existing archive replay path.
fn retain_terminal_construction_material(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    import_id: ImportId,
    root: &Path,
    operations: &BootstrapOperationSpool,
    accepted_events: Vec<AcceptedBatchEvent>,
) -> Option<TerminalBootstrapConstructionMaterial> {
    let retained = root.join(format!(".terminal-{}", Uuid::new_v4().simple()));
    if fs::rename(&operations.path, &retained).is_err() {
        return None;
    }
    Some(TerminalBootstrapConstructionMaterial {
        workspace_id,
        lineage_digest,
        import_id,
        operations: retained,
        operation_count: operations.operation_count,
        declaration_count: operations.declaration_count,
        accepted_events,
    })
}

fn record_capture_instrumentation(
    target: &mut BootstrapStreamingImportInstrumentation,
    source: &BootstrapSourceCaptureInstrumentation,
) {
    target.capture_passes = target.capture_passes.saturating_add(source.passes);
    target.external_sort_runs = target.external_sort_runs.saturating_add(source.sort_runs);
    target.source_bytes_read = target
        .source_bytes_read
        .saturating_add(source.physical_bytes);
    target.operation_spool_bytes = target
        .operation_spool_bytes
        .saturating_add(source.spool_bytes);
    target.peak_owned_sort_buffer_bytes = target
        .peak_owned_sort_buffer_bytes
        .max(source.peak_owned_buffer_bytes);
}

fn prepare_bootstrap_scratch(graph: &Graph, scratch: &Path) -> io::Result<()> {
    create_private_directory(scratch)?;
    let scratch = fs::canonicalize(scratch)?;
    let graph_root = fs::canonicalize(&graph.root)?;
    if scratch == graph_root || scratch.starts_with(&graph_root) {
        return Err(invalid_bootstrap_data(
            "inactive bootstrap scratch must be outside the graph",
        ));
    }
    Ok(())
}

fn seal_bootstrap_preparation(
    source: &Path,
    destination: &Path,
    commit_bytes: &[u8],
) -> Result<(), BootstrapStreamingImportError> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            flush_bootstrap_preparation_tree(source)?;
            fs::rename(source, destination)?;
            sync_parent(destination)?;
        }
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            // A same-digest retry may reuse an already sealed preparation. The
            // normal construction path is the rename above and never recopies
            // graph-sized artifacts.
            copy_bootstrap_tree_exact(source, destination)?;
            flush_bootstrap_preparation_tree(destination)?;
        }
        Ok(_) => return Err(BootstrapStreamingImportError::ConflictingSeal),
        Err(error) => return Err(error.into()),
    }
    inactive_bootstrap_preparation_before_seal_hook()?;
    publish_exact_file(&destination.join(BOOTSTRAP_STREAM_SEAL), commit_bytes)?;
    Ok(())
}

#[cfg(test)]
fn inactive_bootstrap_preparation_before_seal_hook() -> io::Result<()> {
    INACTIVE_BOOTSTRAP_PREPARATION_BEFORE_SEAL.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn inactive_bootstrap_preparation_before_seal_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn flush_bootstrap_preparation_tree(path: &Path) -> io::Result<()> {
    let directory = File::open(path)?;
    // SAFETY: the opened directory descriptor names the filesystem containing
    // the complete authenticated preparation prefix.
    let result = unsafe { libc::syncfs(directory.as_fd().as_raw_fd()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn flush_bootstrap_preparation_tree(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_bootstrap_data(
                "bootstrap preparation tree contains a symlink",
            ));
        }
        if metadata.is_dir() {
            flush_bootstrap_preparation_tree(&child)?;
        } else if metadata.is_file() {
            File::open(&child)?.sync_all()?;
        } else {
            return Err(invalid_bootstrap_data(
                "bootstrap preparation tree contains a non-file entry",
            ));
        }
    }
    File::open(path)?.sync_all()
}

fn copy_bootstrap_tree_exact(
    source: &Path,
    destination: &Path,
) -> Result<(), BootstrapStreamingImportError> {
    // Each exact child has an independent destination name, so filesystem
    // enumeration order cannot affect the sealed tree. Keep only one entry
    // live while copying graph-sized artifact directories.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(BootstrapStreamingImportError::InvalidSource(
                "bootstrap artifact tree contains a symlink".into(),
            ));
        }
        if metadata.is_dir() {
            create_private_directory(&destination_path)?;
            copy_bootstrap_tree_exact(&source_path, &destination_path)?;
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            File::open(&destination_path)?.sync_all()?;
        } else if metadata.is_file() {
            copy_bootstrap_file_exact(&source_path, &destination_path)?;
        } else {
            return Err(BootstrapStreamingImportError::InvalidSource(
                "bootstrap artifact tree contains a non-file entry".into(),
            ));
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    File::open(destination)?.sync_all()?;
    Ok(())
}

fn copy_bootstrap_file_exact(
    source: &Path,
    destination: &Path,
) -> Result<(), BootstrapStreamingImportError> {
    match create_new_file(destination) {
        Ok(mut output) => {
            let mut input = File::open(source)?;
            io::copy(&mut input, &mut output)?;
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            {
                output.sync_all()?;
                sync_parent(destination)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if bootstrap_files_equal(source, destination)? {
                Ok(())
            } else {
                Err(BootstrapStreamingImportError::ConflictingSeal)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn bootstrap_files_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let left_metadata = fs::symlink_metadata(left)?;
    let right_metadata = fs::symlink_metadata(right)?;
    if left_metadata.file_type().is_symlink()
        || right_metadata.file_type().is_symlink()
        || !left_metadata.is_file()
        || !right_metadata.is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return Ok(false);
    }
    let mut left = BufReader::with_capacity(64 * 1024, File::open(left)?);
    let mut right = BufReader::with_capacity(64 * 1024, File::open(right)?);
    let mut left_buffer = [0; 64 * 1024];
    let mut right_buffer = [0; 64 * 1024];
    loop {
        let left_read = left.read(&mut left_buffer)?;
        let right_read = right.read(&mut right_buffer)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

fn hex_bootstrap_digest(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[derive(Clone, Copy)]
struct ImportReplayLimits {
    entries: usize,
    base_bytes: u64,
    rendered_bytes: u64,
}

const IMPORT_REPLAY_LIMITS: ImportReplayLimits = ImportReplayLimits {
    entries: MAX_IMPORT_REPLAY_ENTRIES,
    base_bytes: MAX_IMPORT_REPLAY_BYTES,
    rendered_bytes: MAX_IMPORT_RENDERED_TARGET_BYTES,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactBytes {
    bytes: Vec<u8>,
    description: BlobDescription,
}

impl ExactBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        let description = BlobDescription::of(&bytes);
        Self { bytes, description }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn description(&self) -> BlobDescription {
        self.description
    }

    fn from_description(bytes: Vec<u8>, description: BlobDescription) -> Self {
        Self { bytes, description }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawObservation {
    Present(ExactBytes),
    Absent,
}

impl RawObservation {
    pub fn present(bytes: Vec<u8>) -> Self {
        Self::Present(ExactBytes::new(bytes))
    }

    pub const fn description(&self) -> Option<BlobDescription> {
        match self {
            Self::Present(bytes) => Some(bytes.description()),
            Self::Absent => None,
        }
    }
}

/// Exact graph observations keyed by exact, case-preserved managed paths.
///
/// Construction rejects duplicate requested paths instead of silently
/// overwriting one BTreeMap value with another.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RawInventory {
    entries: BTreeMap<ManagedPath, RawObservation>,
}

impl RawInventory {
    pub fn from_entries(
        entries: impl IntoIterator<Item = (ManagedPath, RawObservation)>,
    ) -> Result<Self, InventoryError> {
        let mut inventory = BTreeMap::new();
        let mut path_bytes = 0_u64;
        for (path, observation) in entries {
            if inventory.len() == MAX_IMPORT_FILES {
                return Err(InventoryError::ResourceBudgetExceeded {
                    resource: "managed file count",
                    observed: inventory.len().saturating_add(1) as u64,
                    limit: MAX_IMPORT_FILES as u64,
                });
            }
            path_bytes = charge_budget(
                "aggregate managed path bytes",
                path_bytes,
                path.as_str().len() as u64,
                MAX_IMPORT_PATH_BYTES,
            )?;
            if inventory.insert(path.clone(), observation).is_some() {
                return Err(InventoryError::DuplicateRequestedPath(
                    path.as_str().to_owned(),
                ));
            }
        }
        require_portable_unique(inventory.keys())?;
        Ok(Self { entries: inventory })
    }

    pub fn entries(&self) -> &BTreeMap<ManagedPath, RawObservation> {
        &self.entries
    }

    pub fn present(&self, path: &str) -> Option<&ExactBytes> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate.as_str() == path)
            .and_then(|(_, observation)| match observation {
                RawObservation::Present(bytes) => Some(bytes),
                RawObservation::Absent => None,
            })
    }

    fn derivation_entries(
        &self,
        path_identities: &BTreeMap<ManagedPath, ImportedPathIdentity>,
    ) -> Result<Vec<ImportInventoryEntry>, ImportExecutionError> {
        self.entries
            .iter()
            .map(|(path, observation)| {
                let identity =
                    path_identities
                        .get(path)
                        .ok_or(ImportExecutionError::IncompletePlan(
                            "sealed inventory path has no Graph-decoded managed kind",
                        ))?;
                Ok(ImportInventoryEntry::with_kind(
                    identity.kind,
                    path.clone(),
                    match observation {
                        RawObservation::Present(bytes) => {
                            ImportInventoryState::Present(bytes.description())
                        }
                        RawObservation::Absent => ImportInventoryState::Absent,
                    },
                ))
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum InventoryError {
    UnsafePath(String),
    DuplicateRequestedPath(String),
    PortablePathCollision {
        first: String,
        second: String,
    },
    ResourceBudgetExceeded {
        resource: &'static str,
        observed: u64,
        limit: u64,
    },
    UnsupportedManagedLayout {
        pages_directory: String,
        journals_directory: String,
    },
    UnsafeEntry {
        path: Option<String>,
        message: String,
    },
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePath(path) => write!(f, "unsafe managed path: {path:?}"),
            Self::DuplicateRequestedPath(path) => {
                write!(f, "managed path was requested more than once: {path}")
            }
            Self::PortablePathCollision { first, second } => write!(
                f,
                "managed paths share one portable key: {first} and {second}"
            ),
            Self::ResourceBudgetExceeded {
                resource,
                observed,
                limit,
            } => write!(
                f,
                "{resource} budget exceeded: observed {observed}, limit {limit}"
            ),
            Self::UnsupportedManagedLayout {
                pages_directory,
                journals_directory,
            } => write!(
                f,
                "unsupported managed layout: pages={pages_directory:?}, journals={journals_directory:?}"
            ),
            Self::UnsafeEntry { path, message } => match path {
                Some(path) => write!(f, "unsafe managed input {path}: {message}"),
                None => write!(f, "unsafe managed input: {message}"),
            },
        }
    }
}

impl std::error::Error for InventoryError {}

fn require_portable_unique<'a>(
    paths: impl IntoIterator<Item = &'a ManagedPath>,
) -> Result<(), InventoryError> {
    let mut portable = BTreeMap::new();
    for path in paths {
        if let Some(first) = portable.insert(path.portable_key(), path.as_str().to_owned()) {
            return Err(InventoryError::PortablePathCollision {
                first,
                second: path.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

fn charge_budget(
    resource: &'static str,
    current: u64,
    amount: u64,
    limit: u64,
) -> Result<u64, InventoryError> {
    let observed = current.checked_add(amount).unwrap_or(u64::MAX);
    if observed > limit {
        Err(InventoryError::ResourceBudgetExceeded {
            resource,
            observed,
            limit,
        })
    } else {
        Ok(observed)
    }
}

fn reserve_base_replay(
    instrumentation: &mut ImportInstrumentation,
    declared_base_bytes: u64,
    limits: ImportReplayLimits,
    path: &ManagedPath,
) -> Result<(), ImportBlock> {
    if instrumentation.base_replay_entries == limits.entries {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "base replay entry budget exceeded: limit {}",
                limits.entries
            ),
        ));
    }
    let replay_bytes = instrumentation
        .base_replay_bytes
        .checked_add(declared_base_bytes)
        .unwrap_or(u64::MAX);
    if replay_bytes > limits.base_bytes {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "base replay byte budget exceeded: observed {replay_bytes}, limit {}",
                limits.base_bytes
            ),
        ));
    }
    instrumentation.base_replay_entries = instrumentation.base_replay_entries.saturating_add(1);
    instrumentation.base_replay_bytes = replay_bytes;
    Ok(())
}

fn retain_rendered_target(
    instrumentation: &mut ImportInstrumentation,
    bytes: u64,
    limits: ImportReplayLimits,
    path: &ManagedPath,
) -> Result<(), ImportBlock> {
    let rendered_bytes = instrumentation
        .rendered_target_bytes
        .checked_add(bytes)
        .unwrap_or(u64::MAX);
    if rendered_bytes > limits.rendered_bytes {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "rendered target byte budget exceeded: observed {rendered_bytes}, limit {}",
                limits.rendered_bytes
            ),
        ));
    }
    instrumentation.rendered_target_bytes = rendered_bytes;
    Ok(())
}

/// Read only the explicitly named affected paths. No directory enumeration is
/// performed, including when a requested path is absent.
pub fn inventory_affected(
    graph: &Graph,
    requested_paths: &[&str],
) -> Result<RawInventory, InventoryError> {
    if requested_paths.len() > MAX_IMPORT_FILES {
        return Err(InventoryError::ResourceBudgetExceeded {
            resource: "requested managed path count",
            observed: requested_paths.len() as u64,
            limit: MAX_IMPORT_FILES as u64,
        });
    }
    let mut entries = Vec::with_capacity(requested_paths.len());
    let mut seen = BTreeSet::new();
    let mut portable = BTreeMap::new();
    let mut raw_bytes = 0_u64;
    for requested in requested_paths {
        let path = ManagedPath::parse((*requested).to_owned())
            .map_err(|_| InventoryError::UnsafePath((*requested).to_owned()))?;
        if !seen.insert(path.clone()) {
            return Err(InventoryError::DuplicateRequestedPath(
                path.as_str().to_owned(),
            ));
        }
        if let Some(first) = portable.insert(path.portable_key(), path.as_str().to_owned()) {
            return Err(InventoryError::PortablePathCollision {
                first,
                second: path.as_str().to_owned(),
            });
        }
        let observation = match graph.read_raw_managed_text(&path).map_err(|error| {
            InventoryError::UnsafeEntry {
                path: Some(path.as_str().to_owned()),
                message: error.to_string(),
            }
        })? {
            Some(observation) => {
                raw_bytes = charge_budget(
                    "aggregate raw bytes",
                    raw_bytes,
                    observation.bytes().len() as u64,
                    MAX_IMPORT_RAW_BYTES,
                )?;
                RawObservation::present(observation.into_bytes())
            }
            None => RawObservation::Absent,
        };
        entries.push((path, observation));
    }
    RawInventory::from_entries(entries)
}

/// The only whole-graph raw inventory entry point. It is intentionally named
/// for initial shadow import so ordinary reconciliation cannot obtain a global
/// scan accidentally.
///
/// This is capture evidence, not semantic publication authority. Shadow import
/// must repeat the same capability-bound inventory/semantic comparison at its
/// later import boundary; a caller may not retain this snapshot and assume the
/// live graph remained unchanged.
pub fn inventory_initial_shadow(graph: &Graph) -> Result<RawInventory, InventoryError> {
    let captured = graph
        .fresh_initial_shadow_raw_managed_text_inventory()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData
                && error.to_string().contains("bound exceeded")
            {
                InventoryError::ResourceBudgetExceeded {
                    resource: "initial shadow resources",
                    observed: 1,
                    limit: 0,
                }
            } else {
                InventoryError::UnsafeEntry {
                    path: None,
                    message: error.to_string(),
                }
            }
        })?;
    RawInventory::from_entries(
        captured
            .into_iter()
            .map(|(path, bytes)| (path, RawObservation::present(bytes))),
    )
}

/// Sealed import base. Only `capture_import_scope` can mint one after the
/// enrolled receipt store and accepted engine jointly authenticate it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ReceiptBackedPage {
    intent: ProjectionIntent,
    completion: ProjectionCompletion,
    replayed_target: ExactBytes,
    page: super::MaterializedPage,
}

impl ReceiptBackedPage {
    const fn page_id(&self) -> PageId {
        self.intent.page_id()
    }

    fn path(&self) -> &ManagedPath {
        self.intent.path()
    }

    const fn logical_completion_id(&self) -> LogicalCompletionId {
        self.completion.logical_completion_id()
    }

    fn bytes(&self) -> &[u8] {
        self.replayed_target.bytes()
    }

    const fn description(&self) -> BlobDescription {
        self.replayed_target.description()
    }

    fn annotations(&self) -> &[AnnotatedIdentity] {
        self.intent.annotations()
    }

    fn materialized_page(&self) -> &super::MaterializedPage {
        &self.page
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ScopedPathEvidence {
    Existing(ReceiptBackedPage),
    Released(LogicalCompletionId),
    New,
}

/// One complete affected-scope authority snapshot. Its fields and constructor
/// are private, so downstream code cannot omit an existing receipt, mix
/// frontiers, or relabel an engine-owned path as new.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportScopeSnapshot {
    workspace_id: WorkspaceId,
    paths: BTreeMap<ManagedPath, ScopedPathEvidence>,
    path_identities: BTreeMap<ManagedPath, ImportedPathIdentity>,
}

/// Exact logical identity of one requested external path, captured through the
/// Graph's normal managed-entry decoder before the sealed plan is built.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportedPathIdentity {
    name: LogicalPageName,
    kind: ManagedTextKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InventoryPathFingerprint {
    state: ImportInventoryState,
    file_resource_id: Option<ContentDigest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AffectedReceiptEntry {
    completed: Option<ProjectionCompletedReceipt>,
    bootstrap_base: Option<ExactBytes>,
    intent: ProjectionIntent,
    completion: ProjectionCompletion,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct AffectedReceiptCatalog {
    by_path: BTreeMap<ManagedPath, Vec<AffectedReceiptEntry>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CatalogAuthority {
    digest: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportBlockReason {
    MissingBase,
    CorruptBase,
    AuthorityUnavailable,
    ConflictingLocalTail,
    StaleScope,
    DuplicateAnchorDependent,
    AmbiguousStructuralMatch,
    AmbiguousDestructiveMatch,
    TwoSidedDivergence,
    UnsafeInput,
    UnsupportedManagedLayout,
    ResourceLimit,
    PortablePathCollision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportBlock {
    pub reason: ImportBlockReason,
    pub paths: Vec<String>,
    pub logical_completion_ids: Vec<LogicalCompletionId>,
    pub observation: Option<(ManagedPath, ImportInventoryState)>,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImportInstrumentation {
    pub requested_paths: usize,
    pub inventory_passes: usize,
    pub bytes_read: u64,
    pub bytes_hashed: u64,
    pub peak_owned_raw_bytes: u64,
    pub path_bytes: u64,
    pub catalog_entries: usize,
    pub catalog_bytes_hashed: u64,
    pub base_replay_entries: usize,
    pub base_replay_bytes: u64,
    pub rendered_target_bytes: u64,
    pub catalog_path_inserts: usize,
    pub catalog_path_lookups: usize,
    pub inventory_path_lookups: usize,
    pub present_document_parses: usize,
    pub authenticated_base_document_parses: usize,
    pub parsed_nodes: usize,
    pub max_depth: usize,
    pub locator_components_materialized: usize,
    pub structural_class_nodes: usize,
    pub structural_class_allocations: usize,
    pub structural_key_components: usize,
    pub structural_key_comparisons: usize,
    pub exact_bucket_inserts: usize,
    pub exact_bucket_lookups: usize,
    pub ordered_alignment_visits: usize,
    pub retained_block_matches: usize,
    pub anchored_page_match_set_inserts: usize,
    pub anchored_page_match_set_lookups: usize,
    pub anchored_page_owner_inserts: usize,
    pub anchored_page_owner_lookups: usize,
    pub anchored_page_uuid_owner_inserts: usize,
    pub anchored_page_uuid_owner_lookups: usize,
    pub anchored_page_edge_inserts: usize,
    pub rejected_raw_id_occurrences: usize,
}

impl ImportInstrumentation {
    /// Sum of explicitly recorded byte/component/event counters. This is a
    /// regression signal, not a claim that every platform/library comparison
    /// has one portable unit cost; independent hard ceilings remain authoritative.
    pub fn recorded_work_units(self) -> usize {
        let byte_work = self
            .bytes_read
            .saturating_add(self.bytes_hashed)
            .saturating_add(self.path_bytes)
            .saturating_add(self.catalog_bytes_hashed)
            .saturating_add(self.base_replay_bytes)
            .saturating_add(self.rendered_target_bytes);
        let byte_work = usize::try_from(byte_work).unwrap_or(usize::MAX);
        self.requested_paths
            .saturating_add(byte_work)
            .saturating_add(self.catalog_entries)
            .saturating_add(self.base_replay_entries)
            .saturating_add(self.catalog_path_inserts)
            .saturating_add(self.catalog_path_lookups)
            .saturating_add(self.inventory_path_lookups)
            .saturating_add(self.present_document_parses)
            .saturating_add(self.authenticated_base_document_parses)
            .saturating_add(self.parsed_nodes)
            .saturating_add(self.locator_components_materialized)
            .saturating_add(self.structural_class_nodes)
            .saturating_add(self.structural_class_allocations)
            .saturating_add(self.structural_key_components)
            .saturating_add(self.structural_key_comparisons)
            .saturating_add(self.exact_bucket_inserts)
            .saturating_add(self.exact_bucket_lookups)
            .saturating_add(self.ordered_alignment_visits)
            .saturating_add(self.retained_block_matches)
            .saturating_add(self.anchored_page_match_set_inserts)
            .saturating_add(self.anchored_page_match_set_lookups)
            .saturating_add(self.anchored_page_owner_inserts)
            .saturating_add(self.anchored_page_owner_lookups)
            .saturating_add(self.anchored_page_uuid_owner_inserts)
            .saturating_add(self.anchored_page_uuid_owner_lookups)
            .saturating_add(self.anchored_page_edge_inserts)
            .saturating_add(self.rejected_raw_id_occurrences)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageMatchBasis {
    SamePathCompletion,
    ReceiptBackedExactRename,
    ReceiptBackedAnchoredRename,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageImportMatch {
    path: ManagedPath,
    previous_path: ManagedPath,
    page_id: PageId,
    basis: PageMatchBasis,
}

impl PageImportMatch {
    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub fn previous_path(&self) -> &ManagedPath {
        &self.previous_path
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub const fn basis(&self) -> PageMatchBasis {
        self.basis
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockMatchBasis {
    UniqueLogseqUuid,
    ReceiptStructuralExact,
    ReceiptOrderedTreeAlignment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockImportMatch {
    path: ManagedPath,
    locator: StructuralLocator,
    block_id: BlockId,
    basis: BlockMatchBasis,
}

impl BlockImportMatch {
    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub fn locator(&self) -> &StructuralLocator {
        &self.locator
    }

    pub const fn block_id(&self) -> BlockId {
        self.block_id
    }

    pub const fn basis(&self) -> BlockMatchBasis {
        self.basis
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectedRawIdReason {
    InvalidSyntax,
    Duplicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedRawId {
    path: ManagedPath,
    locator: StructuralLocator,
    raw_value: String,
    reason: RejectedRawIdReason,
}

impl RejectedRawId {
    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub fn locator(&self) -> &StructuralLocator {
        &self.locator
    }

    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    pub const fn reason(&self) -> RejectedRawIdReason {
        self.reason
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportMatches {
    pages: Vec<PageImportMatch>,
    blocks: Vec<BlockImportMatch>,
    rejected_raw_ids: Vec<RejectedRawId>,
}

impl ImportMatches {
    pub fn pages(&self) -> &[PageImportMatch] {
        &self.pages
    }

    pub fn blocks(&self) -> &[BlockImportMatch] {
        &self.blocks
    }

    pub fn rejected_raw_ids(&self) -> &[RejectedRawId] {
        &self.rejected_raw_ids
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportPlanStatus {
    Noop,
    Reconcile,
    Blocked,
}

/// Opaque diagnostic import result.
///
/// This read-only checkpoint deliberately carries no publication witness,
/// mutation capability, or reusable preflight authority. A later checkpoint
/// must recapture its predicates inside a one-shot semantic publisher.
///
/// ```compile_fail
/// use tine_core::oplog::{ImportPlan, ImportPlanStatus};
///
/// fn forge() -> ImportPlan {
///     ImportPlan {
///         status: ImportPlanStatus::Reconcile,
///         import_id: None,
///         inventory: None,
///         matches: None,
///         blocks: Vec::new(),
///         instrumentation: Default::default(),
///     }
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportPlan {
    status: ImportPlanStatus,
    import_id: Option<ImportId>,
    inventory: Option<RawInventory>,
    matches: Option<ImportMatches>,
    scope: Option<ImportScopeSnapshot>,
    execution: Option<ImportExecutionMaterial>,
    formatting: Option<ImportFormattingMaterial>,
    blocks: Vec<ImportBlock>,
    instrumentation: ImportInstrumentation,
}

impl ImportPlan {
    pub const fn status(&self) -> ImportPlanStatus {
        self.status
    }

    pub const fn import_id(&self) -> Option<ImportId> {
        self.import_id
    }

    pub fn inventory(&self) -> Option<&RawInventory> {
        self.inventory.as_ref()
    }

    pub fn matches(&self) -> Option<&ImportMatches> {
        self.matches.as_ref()
    }

    pub fn blocks(&self) -> &[ImportBlock] {
        &self.blocks
    }

    pub const fn instrumentation(&self) -> ImportInstrumentation {
        self.instrumentation
    }

    /// Return the sealed, non-authorizing execution material for one accepted
    /// reconciliation. The hot-engine adapter must still recapture all live
    /// predicates before it drafts or publishes a batch.
    #[allow(dead_code)]
    pub(crate) fn execution_material(
        &self,
    ) -> Result<&ImportExecutionMaterial, ImportExecutionError> {
        match self.status {
            ImportPlanStatus::Noop | ImportPlanStatus::Blocked => {
                Err(ImportExecutionError::RefusedStatus(self.status))
            }
            ImportPlanStatus::Reconcile => {
                self.scope
                    .as_ref()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed import scope",
                    ))?;
                self.execution
                    .as_ref()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed execution material",
                    ))
            }
        }
    }

    /// Consume a reconciliable plan at the engine handoff boundary.  This
    /// deliberately moves the observation bytes: the execution adapter must
    /// not clone an already-bounded external observation just to author it.
    #[allow(dead_code)]
    pub(crate) fn into_execution_material(
        mut self,
    ) -> Result<ImportExecutionMaterial, ImportExecutionError> {
        match self.status {
            ImportPlanStatus::Noop | ImportPlanStatus::Blocked => {
                Err(ImportExecutionError::RefusedStatus(self.status))
            }
            ImportPlanStatus::Reconcile => {
                self.scope
                    .take()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed import scope",
                    ))?;
                self.execution
                    .take()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed execution material",
                    ))
            }
        }
    }

    pub(crate) fn into_formatting_material(mut self) -> Option<ImportFormattingMaterial> {
        (self.status == ImportPlanStatus::Noop)
            .then(|| self.formatting.take())
            .flatten()
    }
}

/// Minimal crate-internal handoff from receipt-backed import planning to the
/// hot-engine draft adapter. It carries no write capability, engine reference,
/// captured projection input, or publish authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportExecutionMaterial {
    import_id: ImportId,
    transaction: OperationTransaction,
    observation: ExternalImportObservationMaterial,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportFormattingPage {
    page_id: PageId,
    path: ManagedPath,
    bytes: Vec<u8>,
    annotations: Vec<AnnotatedIdentity>,
}

impl ImportFormattingPage {
    pub(crate) const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub(crate) fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn annotations(&self) -> &[AnnotatedIdentity] {
        &self.annotations
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportFormattingMaterial {
    pages: Vec<ImportFormattingPage>,
}

impl ImportFormattingMaterial {
    pub(crate) fn pages(&self) -> &[ImportFormattingPage] {
        &self.pages
    }
}

// The hot-engine adapter consumes this sealed material for drafting only;
// capability recapture and publication remain separate authority boundaries.
#[allow(dead_code)]
impl ImportExecutionMaterial {
    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) fn batch_id(&self) -> BatchId {
        self.import_id.batch_id()
    }

    pub(crate) const fn origin(&self) -> BatchOrigin {
        BatchOrigin::ExternalReconciliation {
            import_id: self.import_id,
        }
    }

    pub(crate) fn transaction(&self) -> &OperationTransaction {
        &self.transaction
    }

    pub(crate) fn observation(&self) -> &ExternalImportObservationMaterial {
        &self.observation
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ImportId,
        OperationTransaction,
        ExternalImportObservationMaterial,
    ) {
        (self.import_id, self.transaction, self.observation)
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ImportExecutionError {
    RefusedStatus(ImportPlanStatus),
    IncompletePlan(&'static str),
    InvalidMaterial(String),
    OperationLimit,
    Observation(ExternalImportObservationMaterialError),
}

impl fmt::Display for ImportExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RefusedStatus(status) => {
                write!(
                    formatter,
                    "import plan status {status:?} cannot produce execution material"
                )
            }
            Self::IncompletePlan(detail) => formatter.write_str(detail),
            Self::InvalidMaterial(detail) => formatter.write_str(detail),
            Self::OperationLimit => write!(
                formatter,
                "external reconciliation operation count exceeds {MAX_TRANSACTION_OPERATIONS}"
            ),
            Self::Observation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportExecutionError {}

impl From<ExternalImportObservationMaterialError> for ImportExecutionError {
    fn from(error: ExternalImportObservationMaterialError) -> Self {
        Self::Observation(error)
    }
}

pub fn plan_affected_import(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    requested_paths: &[&str],
) -> ImportPlan {
    plan_affected_import_with_bootstrap(graph, receipts, engine, None, requested_paths)
}

pub(crate) fn plan_affected_import_with_bootstrap(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    bootstrap: Option<&BootstrapProjectionAuthority>,
    requested_paths: &[&str],
) -> ImportPlan {
    let mut instrumentation = ImportInstrumentation {
        requested_paths: requested_paths.len(),
        ..ImportInstrumentation::default()
    };
    let paths = match parse_requested_paths(requested_paths) {
        Ok(paths) => paths,
        Err(error) => return blocked_inventory_error(error, instrumentation),
    };
    instrumentation.path_bytes = paths.iter().map(|path| path.as_str().len() as u64).sum();
    let accepted_frontier = match engine.accepted_frontier_root() {
        Ok(root) => root,
        Err(error) => {
            return blocked_authority_error(
                None,
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    None,
                    error.to_string(),
                ),
                instrumentation,
            );
        }
    };
    let (catalog, catalog_authority) =
        match capture_affected_catalog(receipts, engine, bootstrap, &paths, &mut instrumentation) {
            Ok(snapshot) => snapshot,
            Err(block) => return blocked_authority_error(None, block, instrumentation),
        };
    let (inventory, inventory_fingerprints, first_raw_bytes) =
        match capture_inventory(graph, &paths, true, 0, &mut instrumentation) {
            Ok((Some(inventory), fingerprints, raw_bytes)) => (inventory, fingerprints, raw_bytes),
            Ok((None, _, _)) => unreachable!("retaining capture returns inventory"),
            Err(error) => return blocked_inventory_error(error, instrumentation),
        };
    let scope = match capture_import_scope(
        graph,
        receipts,
        engine,
        &paths,
        catalog,
        &mut instrumentation,
    ) {
        Ok(scope) => scope,
        Err(mut block) => {
            if block.observation.is_none() {
                block.observation = block
                    .paths
                    .first()
                    .and_then(|path| inventory_observation(&inventory, path));
            }
            return blocked_authority_error(Some(inventory), block, instrumentation);
        }
    };
    snapshot_revalidation_hook();
    let (_, second_fingerprints, _) =
        match capture_inventory(graph, &paths, false, first_raw_bytes, &mut instrumentation) {
            Ok(capture) => capture,
            Err(error) => {
                return blocked_authority_error(
                    Some(inventory),
                    authority_block(ImportBlockReason::StaleScope, None, error.to_string()),
                    instrumentation,
                );
            }
        };
    let (_, post_catalog_authority) =
        match capture_affected_catalog(receipts, engine, bootstrap, &paths, &mut instrumentation) {
            Ok(snapshot) => snapshot,
            Err(mut block) => {
                block.reason = ImportBlockReason::StaleScope;
                return blocked_authority_error(Some(inventory), block, instrumentation);
            }
        };
    let post_frontier = match post_snapshot_frontier(engine) {
        Ok(root) => root,
        Err(error) => {
            return blocked_authority_error(
                Some(inventory),
                authority_block(ImportBlockReason::StaleScope, None, error.to_string()),
                instrumentation,
            );
        }
    };
    if inventory_fingerprints != second_fingerprints
        || catalog_authority != post_catalog_authority
        || accepted_frontier != post_frontier
    {
        return blocked_authority_error(
            Some(inventory),
            authority_block(
                ImportBlockReason::StaleScope,
                None,
                "inventory, exact affected receipt authority, or accepted frontier changed between snapshot passes",
            ),
            instrumentation,
        );
    };
    // Equal bounded collections detect stale diagnostic input only under the
    // explicit quiescent-writer boundary. They are neither a portable atomic
    // filesystem snapshot nor authority for later publication.
    plan_import(graph, inventory, scope, engine, instrumentation)
}

#[cfg(test)]
fn snapshot_revalidation_hook() {
    SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn snapshot_revalidation_hook() {}

#[cfg(test)]
fn post_snapshot_frontier(
    engine: &ShardedHotEngine,
) -> Result<AcceptedFrontierRoot, super::EngineError> {
    POST_FRONTIER_OVERRIDE
        .with(|root| root.borrow_mut().take())
        .map_or_else(|| engine.accepted_frontier_root(), Ok)
}

#[cfg(not(test))]
fn post_snapshot_frontier(
    engine: &ShardedHotEngine,
) -> Result<AcceptedFrontierRoot, super::EngineError> {
    engine.accepted_frontier_root()
}

fn parse_requested_paths(requested_paths: &[&str]) -> Result<Vec<ManagedPath>, InventoryError> {
    if requested_paths.len() > MAX_IMPORT_FILES {
        return Err(InventoryError::ResourceBudgetExceeded {
            resource: "requested managed path count",
            observed: requested_paths.len() as u64,
            limit: MAX_IMPORT_FILES as u64,
        });
    }
    let mut paths = Vec::with_capacity(requested_paths.len());
    let mut exact = BTreeSet::new();
    let mut path_bytes = 0_u64;
    for requested in requested_paths {
        path_bytes = charge_budget(
            "aggregate requested path bytes",
            path_bytes,
            requested.len() as u64,
            MAX_IMPORT_PATH_BYTES,
        )?;
        let path = ManagedPath::parse((*requested).to_owned())
            .map_err(|_| InventoryError::UnsafePath((*requested).to_owned()))?;
        if !exact.insert(path.clone()) {
            return Err(InventoryError::DuplicateRequestedPath(
                path.as_str().to_owned(),
            ));
        }
        paths.push(path);
    }
    require_portable_unique(&paths)?;
    paths.sort_unstable();
    Ok(paths)
}

fn capture_inventory(
    graph: &Graph,
    paths: &[ManagedPath],
    retain: bool,
    retained_raw_bytes: u64,
    instrumentation: &mut ImportInstrumentation,
) -> Result<
    (
        Option<RawInventory>,
        BTreeMap<ManagedPath, InventoryPathFingerprint>,
        u64,
    ),
    InventoryError,
> {
    instrumentation.inventory_passes = instrumentation.inventory_passes.saturating_add(1);
    let mut entries = retain.then(|| Vec::with_capacity(paths.len()));
    let mut fingerprints = BTreeMap::new();
    let mut raw_bytes = 0_u64;
    for path in paths {
        let observation =
            graph
                .read_raw_managed_text(path)
                .map_err(|error| InventoryError::UnsafeEntry {
                    path: Some(path.as_str().to_owned()),
                    message: error.to_string(),
                })?;
        let (raw, fingerprint) = match observation {
            Some(observation) => {
                let description = observation.description();
                raw_bytes = charge_budget(
                    "aggregate raw bytes",
                    raw_bytes,
                    observation.bytes().len() as u64,
                    MAX_IMPORT_RAW_BYTES,
                )?;
                instrumentation.bytes_read = instrumentation
                    .bytes_read
                    .saturating_add(observation.physical_bytes_read());
                instrumentation.bytes_hashed = instrumentation
                    .bytes_hashed
                    .saturating_add(observation.physical_bytes_read());
                instrumentation.peak_owned_raw_bytes = instrumentation.peak_owned_raw_bytes.max(
                    retained_raw_bytes.saturating_add(observation.peak_capture_buffer_bytes()),
                );
                let fingerprint = InventoryPathFingerprint {
                    state: ImportInventoryState::Present(description),
                    file_resource_id: Some(observation.file_resource_id()),
                };
                let (bytes, description) = observation.into_parts();
                let raw = RawObservation::Present(ExactBytes::from_description(bytes, description));
                (raw, fingerprint)
            }
            None => (
                RawObservation::Absent,
                InventoryPathFingerprint {
                    state: ImportInventoryState::Absent,
                    file_resource_id: None,
                },
            ),
        };
        if let Some(entries) = &mut entries {
            entries.push((path.clone(), raw));
            instrumentation.peak_owned_raw_bytes =
                instrumentation.peak_owned_raw_bytes.max(raw_bytes);
        }
        fingerprints.insert(path.clone(), fingerprint);
    }
    let inventory = entries.map(RawInventory::from_entries).transpose()?;
    Ok((inventory, fingerprints, raw_bytes))
}

/// Capture only the durable receipts reachable from the enrolled completed-work
/// mapping for the requested exact paths.  The work index's authenticated
/// Patricia root supplies the path-to-intent ids; the receipt store then makes
/// the matching immutable point reads.  No receipt directory is enumerated.
fn capture_affected_catalog(
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    bootstrap: Option<&BootstrapProjectionAuthority>,
    requested_paths: &[ManagedPath],
    instrumentation: &mut ImportInstrumentation,
) -> Result<(AffectedReceiptCatalog, CatalogAuthority), ImportBlock> {
    let (_, work_index) = engine.enrolled_projection_runtime().map_err(|error| {
        authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            error.to_string(),
        )
    })?;
    if work_index.receipt_store_id() != receipts.store_id() {
        return Err(authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            "enrolled work index is not bound to the receipt store",
        ));
    }

    let mut catalog = AffectedReceiptCatalog::default();
    let mut captured_entries = 0usize;
    let mut hasher = Sha256::new();
    hasher.update(b"tine/import-affected-receipt-snapshot/v2\0");
    hasher.update(receipts.store_id().as_bytes());
    for path in requested_paths {
        hasher.update((path.as_str().len() as u64).to_be_bytes());
        hasher.update(path.as_str().as_bytes());
        let completed = work_index
            .completed_receipts_for_path(path)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    format!("authenticated completed-work path lookup failed: {error}"),
                )
            })?;
        if completed.len() > MAX_IMPORT_CATALOG_ENTRIES.saturating_sub(captured_entries) {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!(
                    "affected completed-work entry budget exceeded: observed {} after {}, limit {}",
                    completed.len(),
                    captured_entries,
                    MAX_IMPORT_CATALOG_ENTRIES
                ),
            ));
        }
        let mut entries = Vec::with_capacity(completed.len());
        for completed in completed {
            let (intent, completion) =
                receipts
                    .load_completed_receipt(&completed)
                    .map_err(|error| {
                        let reason =
                            if matches!(error, ProjectionStoreError::MissingPriorCompletion) {
                                ImportBlockReason::MissingBase
                            } else {
                                ImportBlockReason::CorruptBase
                            };
                        authority_block(
                            reason,
                            Some(path),
                            format!("durable exact receipt lookup is invalid: {error}"),
                        )
                    })?;
            let intent_bytes = intent.encode().map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    error.to_string(),
                )
            })?;
            let completion_bytes = completion.encode().map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    error.to_string(),
                )
            })?;
            instrumentation.catalog_entries = instrumentation.catalog_entries.saturating_add(1);
            instrumentation.catalog_bytes_hashed = instrumentation
                .catalog_bytes_hashed
                .saturating_add(intent_bytes.len() as u64)
                .saturating_add(completion_bytes.len() as u64);
            hasher.update(completed.intent_id().as_bytes());
            hasher.update((intent_bytes.len() as u64).to_be_bytes());
            hasher.update(intent_bytes);
            hasher.update((completion_bytes.len() as u64).to_be_bytes());
            hasher.update(completion_bytes);
            entries.push(AffectedReceiptEntry {
                completed: Some(completed),
                bootstrap_base: None,
                intent,
                completion,
            });
            captured_entries = captured_entries.saturating_add(1);
        }
        if entries.is_empty() {
            if let Some(bootstrap) = bootstrap {
                let baseline = bootstrap.baseline_at(path).map_err(|error| {
                    authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        format!("aggregate bootstrap baseline lookup failed: {error}"),
                    )
                })?;
                if let Some(baseline) = baseline {
                    if captured_entries == MAX_IMPORT_CATALOG_ENTRIES {
                        return Err(authority_block(
                            ImportBlockReason::ResourceLimit,
                            Some(path),
                            "affected aggregate baseline entry budget exceeded",
                        ));
                    }
                    let owner = engine
                        .current_path_catalog_row_at_path(path)
                        .map_err(|error| {
                            authority_block(
                                ImportBlockReason::AuthorityUnavailable,
                                Some(path),
                                format!("aggregate bootstrap current-path lookup failed: {error}"),
                            )
                        })?
                        .ok_or_else(|| {
                            authority_block(
                                ImportBlockReason::ConflictingLocalTail,
                                Some(path),
                                "aggregate bootstrap path has no current accepted owner",
                            )
                        })?;
                    let state = engine
                        .materialize_page_for_projection(owner.page_id())
                        .map_err(|error| {
                            authority_block(
                                ImportBlockReason::AuthorityUnavailable,
                                Some(path),
                                format!("aggregate bootstrap page materialization failed: {error}"),
                            )
                        })?;
                    if owner.path() != path
                        || owner.kind() != baseline.kind()
                        || state.page.page_id != owner.page_id()
                        || state.page.path != *path
                        || state.page.kind != baseline.kind()
                    {
                        return Err(authority_block(
                            ImportBlockReason::ConflictingLocalTail,
                            Some(path),
                            "aggregate bootstrap owner identity changed",
                        ));
                    }
                    let rebound = baseline
                        .rebind_semantic_successor(engine.workspace_id(), &state)
                        .map_err(|error| {
                            authority_block(
                                ImportBlockReason::ConflictingLocalTail,
                                Some(path),
                                format!(
                                    "aggregate bootstrap source does not match current accepted semantics: {error:?}"
                                ),
                            )
                        })?;
                    let intent = rebound.intent().clone();
                    let base = ExactBytes::from_description(
                        baseline.source_bytes().to_vec(),
                        BlobDescription::of(baseline.source_bytes()),
                    );
                    let completion =
                        ProjectionCompletion::for_intent(&intent, baseline.source_bytes())
                            .map_err(|error| {
                                authority_block(
                                    ImportBlockReason::CorruptBase,
                                    Some(path),
                                    format!("aggregate bootstrap completion is invalid: {error}"),
                                )
                            })?;
                    let intent_bytes = intent.encode().map_err(|error| {
                        authority_block(
                            ImportBlockReason::CorruptBase,
                            Some(path),
                            error.to_string(),
                        )
                    })?;
                    let completion_bytes = completion.encode().map_err(|error| {
                        authority_block(
                            ImportBlockReason::CorruptBase,
                            Some(path),
                            error.to_string(),
                        )
                    })?;
                    instrumentation.catalog_entries =
                        instrumentation.catalog_entries.saturating_add(1);
                    instrumentation.catalog_bytes_hashed = instrumentation
                        .catalog_bytes_hashed
                        .saturating_add(intent_bytes.len() as u64)
                        .saturating_add(completion_bytes.len() as u64)
                        .saturating_add(32);
                    hasher.update(b"aggregate-bootstrap-baseline\0");
                    hasher.update(baseline.owner_binding().as_bytes());
                    hasher.update((intent_bytes.len() as u64).to_be_bytes());
                    hasher.update(intent_bytes);
                    hasher.update((completion_bytes.len() as u64).to_be_bytes());
                    hasher.update(completion_bytes);
                    entries.push(AffectedReceiptEntry {
                        completed: None,
                        bootstrap_base: Some(base),
                        intent,
                        completion,
                    });
                    captured_entries = captured_entries.saturating_add(1);
                }
            }
        }
        catalog.by_path.insert(path.clone(), entries);
    }
    Ok((
        catalog,
        CatalogAuthority {
            digest: ContentDigest::from_bytes(hasher.finalize().into()),
        },
    ))
}

fn capture_import_scope(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    requested_paths: &[ManagedPath],
    catalog: AffectedReceiptCatalog,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ImportScopeSnapshot, ImportBlock> {
    let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
        authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            "import authority requires an enrolled projection endpoint",
        )
    })?;
    if engine.workspace_id() != receipts.workspace_id()
        || receipts.endpoint_binding() != Some(endpoint)
        || engine.projection_receipt_store_id() != Some(receipts.store_id())
        || graph.canonical_resource_id().map_err(|error| {
            authority_block(
                ImportBlockReason::AuthorityUnavailable,
                None,
                error.to_string(),
            )
        })? != endpoint.graph_resource_id
    {
        return Err(authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            "graph, engine, receipt workspace, or endpoint binding differs",
        ));
    }

    let mut paths = BTreeMap::new();
    let mut path_identities = BTreeMap::new();
    for path in requested_paths {
        let entry = graph
            .managed_entry_for_managed_path(path)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::UnsafeInput,
                    Some(path),
                    format!("managed path cannot be decoded with Graph loading semantics: {error}"),
                )
            })?;
        let name = LogicalPageName::parse(entry.name).map_err(|error| {
            authority_block(
                ImportBlockReason::UnsafeInput,
                Some(path),
                format!("managed path has an invalid logical page name: {error}"),
            )
        })?;
        let kind = match entry.kind {
            PageKind::Page => ManagedTextKind::Page,
            PageKind::Journal => ManagedTextKind::Journal,
        };
        path_identities.insert(path.clone(), ImportedPathIdentity { name, kind });
        instrumentation.catalog_path_lookups =
            instrumentation.catalog_path_lookups.saturating_add(1);
        let catalog_entries = catalog.by_path.get(path).map(Vec::as_slice).unwrap_or(&[]);
        let current_owner = engine.current_page_at_path(path).map_err(|error| {
            authority_block(
                ImportBlockReason::AuthorityUnavailable,
                Some(path),
                error.to_string(),
            )
        })?;
        let page_id = match current_owner {
            CurrentPageAtPath::ExactOwner(occupied) => occupied.page_id(),
            CurrentPageAtPath::Released(release) => {
                let (_, work_index) = engine.enrolled_projection_runtime().map_err(|error| {
                    authority_block(
                        ImportBlockReason::AuthorityUnavailable,
                        Some(path),
                        error.to_string(),
                    )
                })?;
                let completed = engine
                    .authorize_projected_release(&work_index, &release)
                    .map_err(|error| {
                        authority_block(
                            ImportBlockReason::ConflictingLocalTail,
                            Some(path),
                            format!("released path lacks completed durable work: {error}"),
                        )
                    })?;
                let mut completion_id = None;
                for entry in catalog_entries {
                    if entry.intent.workspace_id() != engine.workspace_id()
                        || entry.intent.page_id() != release.prior_page_id()
                        || entry.intent.path() != path
                        || entry.intent.frontier() != completed.frontier()
                        || entry.intent.target() != BlobDescription::of(&[])
                        || entry.completed.as_ref().is_none_or(|entry| {
                            entry.page_id() != completed.page_id()
                                || entry.frontier() != completed.frontier()
                                || entry.target() != super::ProjectionWorkTarget::Absent
                        })
                    {
                        continue;
                    }
                    let logical = entry.completion.logical_completion_id();
                    if completion_id.replace(logical).is_some() {
                        return Err(authority_block(
                            ImportBlockReason::CorruptBase,
                            Some(path),
                            "multiple completed receipts claim one authenticated path release",
                        ));
                    }
                }
                let completion_id = completion_id.ok_or_else(|| {
                    authority_block(
                        ImportBlockReason::ConflictingLocalTail,
                        Some(path),
                        "authenticated path release has no exact completed receipt",
                    )
                })?;
                paths.insert(path.clone(), ScopedPathEvidence::Released(completion_id));
                continue;
            }
            CurrentPageAtPath::Unowned => {
                if !catalog_entries.is_empty() {
                    return Err(authority_block(
                        ImportBlockReason::ConflictingLocalTail,
                        Some(path),
                        "receipt-backed path is no longer owned at the accepted engine frontier",
                    ));
                }
                paths.insert(path.clone(), ScopedPathEvidence::New);
                continue;
            }
            CurrentPageAtPath::PortableCollision(occupied) => {
                return Err(authority_block(
                    ImportBlockReason::PortablePathCollision,
                    Some(path),
                    format!(
                        "requested path collides with engine-owned {} for page {}",
                        occupied.exact_path(),
                        occupied.page_id()
                    ),
                ));
            }
            CurrentPageAtPath::ReleasedPortableCollision(release) => {
                return Err(authority_block(
                    ImportBlockReason::PortablePathCollision,
                    Some(path),
                    format!(
                        "requested path collides with authenticated released spelling {} for page {}",
                        release.prior_exact_path(),
                        release.prior_page_id()
                    ),
                ));
            }
        };

        let accepted_identity = engine
            .current_path_catalog_row_at_path(path)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    Some(path),
                    format!("accepted current-path identity is invalid: {error}"),
                )
            })?
            .ok_or_else(|| {
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    Some(path),
                    "exact path owner has no accepted current-path identity",
                )
            })?;
        if accepted_identity.page_id() != page_id {
            return Err(authority_block(
                ImportBlockReason::AuthorityUnavailable,
                Some(path),
                "portable-path owner and accepted current-path PageId disagree",
            ));
        }
        let current = engine
            .authorize_projection_write(page_id)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    Some(path),
                    format!("accepted Ready projection authority is unavailable: {error}"),
                )
            })?;
        if current.state().page.path != *path {
            return Err(authority_block(
                ImportBlockReason::ConflictingLocalTail,
                Some(path),
                "portable-path ownership and materialized page path disagree",
            ));
        }
        if accepted_identity.kind() != current.state().page.kind
            || accepted_identity.accepted_name_digest()
                != ContentDigest::of(current.state().page.name.as_str().as_bytes())
        {
            return Err(authority_block(
                ImportBlockReason::AuthorityUnavailable,
                Some(path),
                "accepted current-path name or kind disagrees with materialized PageId state",
            ));
        }
        // Exact accepted path ownership is the rename boundary. A reopened
        // Graph may decode the same filename differently after config.edn
        // changes, but configuration is not authority to rename an oplog page
        // or change Page versus Journal. Preserve the accepted catalog identity
        // here; an unowned destination in an actual external path move still
        // uses the current Graph decoder above.
        path_identities.insert(
            path.clone(),
            ImportedPathIdentity {
                name: current.state().page.name.clone(),
                kind: current.state().page.kind,
            },
        );

        let mut exact = None;
        let mut replay_cache =
            BTreeMap::<Option<BlobDescription>, (ProjectionIntent, Vec<u8>)>::new();
        for entry in catalog_entries {
            if entry.intent.workspace_id() != engine.workspace_id()
                || entry.intent.page_id() != page_id
                || entry.intent.path() != path
            {
                continue;
            }
            let base_key = match entry.intent.precondition() {
                super::ProjectionPrecondition::Absent => None,
                super::ProjectionPrecondition::Base(description) => Some(*description),
            };
            if let std::collections::btree_map::Entry::Vacant(slot) = replay_cache.entry(base_key) {
                let declared_base_bytes = base_key.map_or(0, BlobDescription::byte_length);
                reserve_base_replay(
                    instrumentation,
                    declared_base_bytes,
                    IMPORT_REPLAY_LIMITS,
                    path,
                )?;
                let base = match &entry.bootstrap_base {
                    Some(base) => Some(super::BaseBlob::new(base.bytes().to_vec())),
                    None => receipts.load_base(&entry.intent).map_err(|error| {
                        authority_block(
                            ImportBlockReason::CorruptBase,
                            Some(path),
                            format!("canonical base evidence is unavailable: {error}"),
                        )
                    })?,
                };
                let replay = plan_projection(
                    engine.workspace_id(),
                    current.state(),
                    base.as_ref().map(super::BaseBlob::bytes),
                )
                .map_err(|error| {
                    authority_block(
                        ImportBlockReason::AuthorityUnavailable,
                        Some(path),
                        format!("accepted state cannot be replayed canonically: {error}"),
                    )
                })?;
                retain_rendered_target(
                    instrumentation,
                    replay.target().len() as u64,
                    IMPORT_REPLAY_LIMITS,
                    path,
                )?;
                slot.insert(replay.into_intent_and_target());
            }
            let (replayed_intent, _) = &replay_cache[&base_key];
            if entry.intent.matches_replay_except_frontier(replayed_intent)
                && (entry.bootstrap_base.is_none()
                    || entry.intent.frontier() == &current.state().frontier)
            {
                if exact.is_some() {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "multiple durable receipt rows claim one current accepted path/frontier",
                    ));
                }
                exact = Some((entry, base_key));
            }
        }
        let Some((entry, base_key)) = exact else {
            return Err(authority_block(
                ImportBlockReason::ConflictingLocalTail,
                Some(path),
                "no durable completion/base exactly matches the current accepted affected frontier",
            ));
        };
        let replayed_target = replay_cache
            .remove(&base_key)
            .expect("exact replay key remains cached")
            .1;
        let completion = entry.completion.clone();
        completion
            .validate_against(&entry.intent)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    error.to_string(),
                )
            })?;
        paths.insert(
            path.clone(),
            ScopedPathEvidence::Existing(ReceiptBackedPage {
                intent: entry.intent.clone(),
                completion,
                replayed_target: ExactBytes::from_description(
                    replayed_target,
                    entry.intent.target(),
                ),
                page: current.state().page.clone(),
            }),
        );
    }

    Ok(ImportScopeSnapshot {
        workspace_id: engine.workspace_id(),
        paths,
        path_identities,
    })
}

fn authority_block(
    reason: ImportBlockReason,
    path: Option<&ManagedPath>,
    detail: impl Into<String>,
) -> ImportBlock {
    ImportBlock {
        reason,
        paths: path
            .into_iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        logical_completion_ids: Vec::new(),
        observation: None,
        detail: detail.into(),
    }
}

fn plan_import(
    graph: &Graph,
    inventory: RawInventory,
    mut scope: ImportScopeSnapshot,
    engine: &ShardedHotEngine,
    mut instrumentation: ImportInstrumentation,
) -> ImportPlan {
    if scope.paths.len() != inventory.entries().len()
        || scope.path_identities.len() != inventory.entries().len()
        || scope
            .paths
            .keys()
            .zip(inventory.entries().keys())
            .any(|(left, right)| left != right)
        || scope
            .path_identities
            .keys()
            .zip(inventory.entries().keys())
            .any(|(left, right)| left != right)
    {
        return blocked_authority_error(
            Some(inventory),
            authority_block(
                ImportBlockReason::StaleScope,
                None,
                "sealed scope and exact inventory path sets differ",
            ),
            instrumentation,
        );
    }
    let completed = scope
        .paths
        .values()
        .filter_map(|evidence| match evidence {
            ScopedPathEvidence::Existing(page) => Some(page),
            ScopedPathEvidence::Released(_) | ScopedPathEvidence::New => None,
        })
        .collect::<Vec<_>>();

    let conflict_copy = inventory
        .entries()
        .keys()
        .find(|path| path_is_sync_conflict(Path::new(path.as_str())))
        .cloned();
    if let Some(path) = conflict_copy {
        return blocked_authority_error(
            Some(inventory),
            ImportBlock {
                reason: ImportBlockReason::UnsafeInput,
                paths: vec![path.as_str().to_owned()],
                logical_completion_ids: Vec::new(),
                observation: None,
                detail: "provider conflict copies are diagnostic inputs and cannot authorize import identity or deletion".into(),
            },
            instrumentation,
        );
    }

    let invalid_inventory = inventory.entries().iter().find_map(|(path, observation)| {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        matches!(observation, RawObservation::Present(bytes) if std::str::from_utf8(bytes.bytes()).is_err())
            .then(|| path.clone())
    });
    if let Some(path) = invalid_inventory {
        let block = ImportBlock {
            reason: ImportBlockReason::UnsafeInput,
            paths: vec![path.as_str().to_owned()],
            logical_completion_ids: Vec::new(),
            observation: inventory_observation(&inventory, path.as_str()),
            detail: "raw bytes were retained, but semantic import requires valid UTF-8".into(),
        };
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }
    if let Some(page) = completed
        .iter()
        .find(|page| std::str::from_utf8(page.bytes()).is_err())
    {
        let block = receipt_block(
            ImportBlockReason::CorruptBase,
            page.path(),
            Some(page.logical_completion_id()),
            &inventory,
            "receipt-backed replay target is not UTF-8",
        );
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }

    let page_matches = match match_pages(&inventory, &completed, &mut instrumentation) {
        Ok(matches) => matches,
        Err(block) => return blocked_authority_error(Some(inventory), block, instrumentation),
    };
    let mut matches = ImportMatches {
        pages: page_matches,
        ..ImportMatches::default()
    };
    let parsed_documents = match match_blocks(
        graph,
        &inventory,
        &completed,
        &mut matches,
        &mut instrumentation,
    ) {
        Ok(parsed) => parsed,
        Err(block) => {
            return blocked_authority_error(Some(inventory), block, instrumentation);
        }
    };
    if let Err(block) =
        match_anchored_page_moves(&inventory, &completed, &mut matches, &mut instrumentation)
    {
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }
    let resolved_path_identities =
        match resolve_import_path_identities(&inventory, &matches, &scope, &parsed_documents) {
            Ok(identities) => identities,
            Err(block) => {
                return blocked_authority_error(Some(inventory), block, instrumentation);
            }
        };

    let mut completion_ids = completed
        .iter()
        .map(|page| page.logical_completion_id())
        .collect::<Vec<_>>();
    completion_ids.extend(scope.paths.values().filter_map(|evidence| match evidence {
        ScopedPathEvidence::Released(completion_id) => Some(*completion_id),
        ScopedPathEvidence::Existing(_) | ScopedPathEvidence::New => None,
    }));
    completion_ids.sort_unstable();
    completion_ids.dedup();
    let derivation_entries = match inventory.derivation_entries(&resolved_path_identities) {
        Ok(entries) => entries,
        Err(error) => {
            return blocked_authority_error(
                Some(inventory),
                ImportBlock {
                    reason: ImportBlockReason::StaleScope,
                    paths: Vec::new(),
                    logical_completion_ids: completion_ids,
                    observation: None,
                    detail: error.to_string(),
                },
                instrumentation,
            );
        }
    };
    let import_id = match ImportId::derive(
        scope.workspace_id,
        &completion_ids,
        &derivation_entries,
        DIFF_SCHEMA_VERSION,
    ) {
        Ok(import_id) => import_id,
        Err(error) => {
            return blocked_authority_error(
                Some(inventory),
                ImportBlock {
                    reason: ImportBlockReason::CorruptBase,
                    paths: Vec::new(),
                    logical_completion_ids: completion_ids,
                    observation: None,
                    detail: error.to_string(),
                },
                instrumentation,
            );
        }
    };

    let page_transition = match build_desired_page_transition(
        &inventory,
        &matches,
        &scope,
        &resolved_path_identities,
        import_id,
    ) {
        Ok(transition) => transition,
        Err(block) => {
            return blocked_authority_error(Some(inventory), block, instrumentation);
        }
    };
    if let Err(block) = preflight_desired_page_names(&inventory, &page_transition, engine) {
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }

    let completed_paths = completed
        .iter()
        .map(|page| page.path().clone())
        .collect::<BTreeSet<_>>();
    let changed = completed.iter().any(|page| {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        !matches!(
            inventory.entries().get(page.path()),
            Some(RawObservation::Present(bytes)) if bytes.description() == page.description()
        )
    }) || inventory.entries().iter().any(|(path, observation)| {
        matches!(observation, RawObservation::Present(_)) && !completed_paths.contains(path)
    });
    drop(completed);
    scope.path_identities = resolved_path_identities;
    let (status, scope, execution, formatting) = if changed {
        match build_execution_material(
            import_id,
            &inventory,
            &matches,
            &scope,
            &page_transition,
            &parsed_documents,
            &mut instrumentation,
        ) {
            Ok(BuiltImportMaterial::Semantic(execution)) => (
                ImportPlanStatus::Reconcile,
                Some(scope),
                Some(execution),
                None,
            ),
            Ok(BuiltImportMaterial::Formatting(formatting)) => {
                (ImportPlanStatus::Noop, None, None, Some(formatting))
            }
            Err(error) => {
                return blocked_authority_error(
                    Some(inventory),
                    ImportBlock {
                        reason: if matches!(&error, ImportExecutionError::OperationLimit) {
                            ImportBlockReason::ResourceLimit
                        } else {
                            ImportBlockReason::UnsafeInput
                        },
                        paths: Vec::new(),
                        logical_completion_ids: completion_ids,
                        observation: None,
                        detail: format!(
                            "sealed external reconciliation cannot produce canonical execution material: {error}"
                        ),
                    },
                    instrumentation,
                );
            }
        }
    } else {
        (ImportPlanStatus::Noop, None, None, None)
    };
    ImportPlan {
        status,
        import_id: Some(import_id),
        inventory: Some(inventory),
        matches: Some(matches),
        scope,
        execution,
        formatting,
        blocks: Vec::new(),
        instrumentation,
    }
}

/// Refuse a transaction before authoring when two affected files would acquire
/// one logical page name, or when an affected destination name is already
/// owned by another authenticated page.  Paths are deliberately not used as a
/// name namespace: duplicate basenames at different paths remain visible
/// ambiguity instead of a silently successful reconciliation.
fn preflight_desired_page_names(
    inventory: &RawInventory,
    transition: &DesiredPageTransition,
    engine: &ShardedHotEngine,
) -> Result<(), ImportBlock> {
    let mut desired = BTreeMap::new();
    for (path, page) in &transition.pages {
        if let Some((prior_path, prior_page_id, prior_name)) = desired.insert(
            page.name.key_digest(),
            (path.clone(), page.page_id, page.name.clone()),
        ) {
            if prior_page_id != page.page_id {
                return Err(ImportBlock {
                    reason: ImportBlockReason::ConflictingLocalTail,
                    paths: vec![prior_path.as_str().to_owned(), path.as_str().to_owned()],
                    logical_completion_ids: Vec::new(),
                    observation: inventory_observation(inventory, path.as_str()),
                    detail: format!(
                        "affected paths decode to the same logical page name: {} and {}",
                        prior_name.as_str(),
                        page.name.as_str()
                    ),
                });
            }
        }
    }
    for (_, (path, page_id, name)) in desired {
        let owner = engine
            .current_page_for_logical_name(&name)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    Some(&path),
                    format!("authenticated logical page-name lookup failed: {error}"),
                )
            })?;
        if owner.is_some_and(|owner| {
            owner != page_id && !transition.released_name_owners.contains(&owner)
        }) {
            return Err(authority_block(
                ImportBlockReason::ConflictingLocalTail,
                Some(&path),
                format!(
                    "decoded destination logical page name {} is already owned by page {}",
                    name.as_str(),
                    owner.expect("checked above")
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct CurrentImportBlock {
    page_id: PageId,
    block: super::MaterializedBlock,
}

#[derive(Clone, Debug)]
struct DesiredImportPage {
    page_id: PageId,
    home_document_id: DocumentId,
    name: LogicalPageName,
    path: ManagedPath,
    kind: ManagedTextKind,
    existing: bool,
}

#[derive(Clone, Debug)]
struct DesiredPageTransition {
    pages: BTreeMap<ManagedPath, DesiredImportPage>,
    /// Current affected owners whose present logical name is absent from the
    /// final ownership set for that same page identity. The page-name index
    /// validates a transaction's final catalog atomically, so chains and cycles
    /// may consume these released names without exposing an intermediate state.
    released_name_owners: BTreeSet<PageId>,
}

fn imported_identity(
    name: &str,
    kind: PageKind,
    path: &ManagedPath,
) -> Result<ImportedPathIdentity, ImportBlock> {
    let name = LogicalPageName::parse(name.to_owned()).map_err(|error| {
        authority_block(
            ImportBlockReason::UnsafeInput,
            Some(path),
            format!("parsed external document has an invalid logical page name: {error}"),
        )
    })?;
    let kind = match kind {
        PageKind::Page => ManagedTextKind::Page,
        PageKind::Journal => ManagedTextKind::Journal,
    };
    Ok(ImportedPathIdentity { name, kind })
}

/// Resolve current document semantics without collapsing filename fallback,
/// accepted identity, and parser-declared title authority.
fn resolve_import_path_identities(
    inventory: &RawInventory,
    matches: &ImportMatches,
    scope: &ImportScopeSnapshot,
    parsed: &ParsedImportDocuments,
) -> Result<BTreeMap<ManagedPath, ImportedPathIdentity>, ImportBlock> {
    let mut identities = scope.path_identities.clone();
    let page_matches = matches
        .pages()
        .iter()
        .map(|matched| (matched.path(), matched))
        .collect::<BTreeMap<_, _>>();
    for (path, observation) in inventory.entries() {
        if !matches!(observation, RawObservation::Present(_)) {
            continue;
        }
        let current = parsed.current.get(path).ok_or_else(|| {
            authority_block(
                ImportBlockReason::CorruptBase,
                Some(path),
                "present external document has no parser-owned semantic result",
            )
        })?;
        let accepted = match page_matches.get(path) {
            None => None,
            Some(matched) if matched.basis() != PageMatchBasis::SamePathCompletion => None,
            Some(matched) => {
                let Some(ScopedPathEvidence::Existing(existing)) =
                    scope.paths.get(matched.previous_path())
                else {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "same-path match has no authenticated accepted predecessor",
                    ));
                };
                let base = parsed.base.get(matched.previous_path()).ok_or_else(|| {
                    receipt_block(
                        ImportBlockReason::CorruptBase,
                        matched.previous_path(),
                        Some(existing.logical_completion_id()),
                        inventory,
                        "authenticated completed-base document has no parser-owned semantic result",
                    )
                })?;
                Some(AcceptedExternalDocumentIdentity {
                    name: existing.materialized_page().name.as_str(),
                    kind: match existing.materialized_page().kind {
                        ManagedTextKind::Page => PageKind::Page,
                        ManagedTextKind::Journal => PageKind::Journal,
                    },
                    explicit_title: base.explicit_title.as_deref(),
                })
            }
        };
        let identity = resolve_external_document_identity(
            current.explicit_title.as_deref(),
            &current.filename_fallback,
            &current.effective,
            accepted,
        );
        let identity = imported_identity(&identity.name, identity.kind, path)?;
        identities.insert(path.clone(), identity);
    }
    Ok(identities)
}

fn build_desired_page_transition(
    inventory: &RawInventory,
    matches: &ImportMatches,
    scope: &ImportScopeSnapshot,
    path_identities: &BTreeMap<ManagedPath, ImportedPathIdentity>,
    import_id: ImportId,
) -> Result<DesiredPageTransition, ImportBlock> {
    let mut page_matches = BTreeMap::<ManagedPath, &PageImportMatch>::new();
    for page_match in matches.pages() {
        if page_matches
            .insert(page_match.path().clone(), page_match)
            .is_some()
        {
            return Err(authority_block(
                ImportBlockReason::CorruptBase,
                Some(page_match.path()),
                "sealed import matches contain duplicate external page paths",
            ));
        }
    }

    let mut pages = BTreeMap::new();
    let mut desired_paths_by_page = BTreeMap::new();
    for (path, observation) in inventory.entries() {
        if !matches!(observation, RawObservation::Present(_)) {
            continue;
        }
        let path_identity = path_identities.get(path).ok_or_else(|| {
            authority_block(
                ImportBlockReason::StaleScope,
                Some(path),
                "present inventory path has no Graph-decoded logical identity",
            )
        })?;
        let desired = match page_matches.get(path) {
            Some(page_match) => {
                let Some(ScopedPathEvidence::Existing(existing)) =
                    scope.paths.get(page_match.previous_path())
                else {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "matched external page has no sealed receipt-backed predecessor",
                    ));
                };
                let current = existing.materialized_page();
                if current.page_id != page_match.page_id() {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "matched external page identity differs from its sealed predecessor",
                    ));
                }
                DesiredImportPage {
                    page_id: current.page_id,
                    home_document_id: current.home_document_id,
                    name: path_identity.name.clone(),
                    path: path.clone(),
                    kind: path_identity.kind,
                    existing: true,
                }
            }
            None => DesiredImportPage {
                page_id: import_id.unmatched_page_id(&ImportLocator::page(path.clone())),
                home_document_id: DocumentId::for_unmatched_import_page(
                    scope.workspace_id,
                    path.as_str().as_bytes(),
                ),
                name: path_identity.name.clone(),
                path: path.clone(),
                kind: path_identity.kind,
                existing: false,
            },
        };
        if let Some(prior_path) = desired_paths_by_page.insert(desired.page_id, path.clone()) {
            return Err(ImportBlock {
                reason: ImportBlockReason::ConflictingLocalTail,
                paths: vec![prior_path.as_str().to_owned(), path.as_str().to_owned()],
                logical_completion_ids: Vec::new(),
                observation: inventory_observation(inventory, path.as_str()),
                detail: "one affected page identity would survive at more than one path".into(),
            });
        }
        pages.insert(path.clone(), desired);
    }

    let final_names_by_page = pages
        .values()
        .map(|page| (page.page_id, page.name.key_digest()))
        .collect::<BTreeMap<_, _>>();
    let released_name_owners = scope
        .paths
        .values()
        .filter_map(|evidence| {
            let ScopedPathEvidence::Existing(existing) = evidence else {
                return None;
            };
            let current = existing.materialized_page();
            (final_names_by_page.get(&current.page_id).copied() != Some(current.name.key_digest()))
                .then_some(current.page_id)
        })
        .collect();
    Ok(DesiredPageTransition {
        pages,
        released_name_owners,
    })
}

#[derive(Clone, Debug)]
struct DesiredImportBlock {
    block_id: BlockId,
    page_id: PageId,
    home_document_id: DocumentId,
    parent: Option<BlockId>,
    order: String,
    content: String,
    logseq_uuid: Option<LogseqUuid>,
    existing: bool,
}

enum BuiltImportMaterial {
    Semantic(ImportExecutionMaterial),
    Formatting(ImportFormattingMaterial),
}

fn push_operation(
    operations: &mut Vec<SemanticOperation>,
    operation: SemanticOperation,
) -> Result<(), ImportExecutionError> {
    if operations.len() == MAX_TRANSACTION_OPERATIONS {
        return Err(ImportExecutionError::OperationLimit);
    }
    operations.push(operation);
    Ok(())
}

fn build_execution_material(
    import_id: ImportId,
    inventory: &RawInventory,
    matches: &ImportMatches,
    scope: &ImportScopeSnapshot,
    page_transition: &DesiredPageTransition,
    parsed_documents: &ParsedImportDocuments,
    instrumentation: &mut ImportInstrumentation,
) -> Result<BuiltImportMaterial, ImportExecutionError> {
    let mut current_pages = BTreeMap::<PageId, &ReceiptBackedPage>::new();
    let mut current_blocks = BTreeMap::<BlockId, CurrentImportBlock>::new();
    for evidence in scope.paths.values() {
        let ScopedPathEvidence::Existing(page) = evidence else {
            continue;
        };
        let materialized = page.materialized_page();
        if materialized.page_id != page.page_id() || materialized.path != *page.path() {
            return Err(ImportExecutionError::InvalidMaterial(
                "receipt-backed page identity does not match its materialized accepted state"
                    .into(),
            ));
        }
        if current_pages.insert(materialized.page_id, page).is_some() {
            return Err(ImportExecutionError::InvalidMaterial(
                "sealed import scope contains one page more than once".into(),
            ));
        }
        for block in &materialized.blocks {
            if current_blocks
                .insert(
                    block.block_id,
                    CurrentImportBlock {
                        page_id: materialized.page_id,
                        block: block.clone(),
                    },
                )
                .is_some()
            {
                return Err(ImportExecutionError::InvalidMaterial(
                    "sealed import scope contains one visible block more than once".into(),
                ));
            }
        }
    }

    let desired_pages = &page_transition.pages;

    let trees = &parsed_documents.current;

    let mut block_matches = BTreeMap::<(ManagedPath, StructuralLocator), BlockId>::new();
    for block_match in matches.blocks() {
        if !trees.contains_key(block_match.path()) {
            return Err(ImportExecutionError::InvalidMaterial(
                "sealed block match refers to an absent external path".into(),
            ));
        }
        if block_matches
            .insert(
                (block_match.path().clone(), block_match.locator().clone()),
                block_match.block_id(),
            )
            .is_some()
        {
            return Err(ImportExecutionError::InvalidMaterial(
                "sealed import matches contain duplicate external block locators".into(),
            ));
        }
    }
    let rejected_raw_ids = matches
        .rejected_raw_ids()
        .iter()
        .map(|rejected| (rejected.path().clone(), rejected.locator().clone()))
        .collect::<BTreeSet<_>>();

    let mut desired_blocks = BTreeMap::<BlockId, DesiredImportBlock>::new();
    let mut desired_node_ids = BTreeMap::<(ManagedPath, usize), BlockId>::new();
    let mut observation_entries = Vec::with_capacity(inventory.entries().len());
    for (path, observation) in inventory.entries() {
        let kind = scope
            .path_identities
            .get(path)
            .ok_or(ImportExecutionError::IncompletePlan(
                "sealed inventory path has no Graph-decoded managed kind",
            ))?
            .kind;
        let state = match observation {
            RawObservation::Absent => ExternalImportObservationState::Absent,
            RawObservation::Present(bytes) => {
                let tree = trees.get(path).ok_or_else(|| {
                    ImportExecutionError::InvalidMaterial(
                        "sealed present inventory path has no parsed tree".into(),
                    )
                })?;
                let page = desired_pages.get(path).ok_or_else(|| {
                    ImportExecutionError::InvalidMaterial(
                        "sealed present inventory path has no desired page".into(),
                    )
                })?;
                let mut annotations = Vec::with_capacity(tree.nodes.len());
                for index in 0..tree.nodes.len() {
                    let locator = materialize_locator(tree, index, instrumentation)
                        .map_err(|block| ImportExecutionError::InvalidMaterial(block.detail))?;
                    let matched = block_matches.get(&(path.clone(), locator.clone())).copied();
                    let (block_id, existing, home_document_id) = match matched {
                        Some(block_id) => {
                            let current = current_blocks.get(&block_id).ok_or_else(|| {
                                ImportExecutionError::InvalidMaterial(
                                    "sealed block match has no accepted current block".into(),
                                )
                            })?;
                            (block_id, true, current.block.home_document_id)
                        }
                        None => (
                            import_id.unmatched_block_id(&ImportLocator::block(
                                path.clone(),
                                locator.clone(),
                            )),
                            false,
                            page.home_document_id,
                        ),
                    };
                    if desired_node_ids
                        .insert((path.clone(), index), block_id)
                        .is_some()
                    {
                        return Err(ImportExecutionError::InvalidMaterial(
                            "sealed parsed tree contains a duplicate block node".into(),
                        ));
                    }
                    let logseq_uuid =
                        external_logseq_uuid(path, &locator, &tree.nodes[index], &rejected_raw_ids);
                    annotations.push(AnnotatedIdentity::new(
                        locator.clone(),
                        tree.nodes[index].span,
                        block_id,
                        logseq_uuid,
                    ));
                    let parent = tree.nodes[index].parent.map(|parent| {
                        desired_node_ids
                            .get(&(path.clone(), parent))
                            .expect("parsed tree parents precede their children")
                            .to_owned()
                    });
                    let desired = DesiredImportBlock {
                        block_id,
                        page_id: page.page_id,
                        home_document_id,
                        parent,
                        order: imported_order(tree.nodes[index].sibling_position),
                        content: tree.nodes[index].raw.clone(),
                        logseq_uuid,
                        existing,
                    };
                    if desired_blocks.insert(block_id, desired).is_some() {
                        return Err(ImportExecutionError::InvalidMaterial(
                            "sealed matches assign one block identity more than once".into(),
                        ));
                    }
                }
                ExternalImportObservationState::present(bytes.bytes().to_vec(), annotations)
                    .map_err(|error| {
                        ImportExecutionError::Observation(
                            ExternalImportObservationMaterialError::Observation(error),
                        )
                    })?
            }
        };
        observation_entries.push(
            ExternalImportObservationEntry::new(path.clone(), kind, state).map_err(|error| {
                ImportExecutionError::Observation(
                    ExternalImportObservationMaterialError::Observation(error),
                )
            })?,
        );
    }
    let observation =
        ExternalImportObservationMaterial::new(scope.workspace_id, import_id, observation_entries)
            .map_err(|error| {
                ImportExecutionError::Observation(
                    ExternalImportObservationMaterialError::Observation(error),
                )
            })?;

    let mut operations = Vec::new();
    for page in desired_pages.values().filter(|page| !page.existing) {
        push_operation(
            &mut operations,
            SemanticOperation::CreatePage {
                page_id: page.page_id,
                home_document_id: page.home_document_id,
                name: page.name.clone(),
                path: page.path.clone(),
                kind: page.kind,
            },
        )?;
    }
    for page in desired_pages.values().filter(|page| page.existing) {
        let current = current_pages.get(&page.page_id).ok_or_else(|| {
            ImportExecutionError::InvalidMaterial(
                "desired existing page is absent from sealed accepted state".into(),
            )
        })?;
        let current = current.materialized_page();
        if current.name != page.name || current.path != page.path || current.kind != page.kind {
            push_operation(
                &mut operations,
                SemanticOperation::ReconcileExternalPageState {
                    page_id: page.page_id,
                    name: page.name.clone(),
                    path: page.path.clone(),
                    kind: page.kind,
                },
            )?;
        }
    }
    let mut new_blocks = desired_blocks
        .values()
        .filter(|block| !block.existing)
        .collect::<Vec<_>>();
    new_blocks.sort_unstable_by(|left, right| {
        desired_block_depth(left.block_id, &desired_blocks)
            .cmp(&desired_block_depth(right.block_id, &desired_blocks))
            .then_with(|| left.block_id.cmp(&right.block_id))
    });
    for desired in new_blocks {
        push_operation(
            &mut operations,
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: desired.block_id,
                    home_document_id: desired.home_document_id,
                },
                page_id: desired.page_id,
                parent: desired.parent,
                order: desired.order.clone(),
                content: desired.content.clone(),
            },
        )?;
    }

    let mut moves = desired_blocks
        .iter()
        .filter_map(|(block_id, desired)| {
            let current = current_blocks.get(block_id)?;
            (current.page_id != desired.page_id).then_some((*block_id, desired, current))
        })
        .collect::<Vec<_>>();
    moves.sort_unstable_by(|(left_id, _, left), (right_id, _, right)| {
        current_block_depth(*left_id, &current_blocks)
            .cmp(&current_block_depth(*right_id, &current_blocks))
            .reverse()
            .then_with(|| left_id.cmp(right_id))
            .then_with(|| left.page_id.cmp(&right.page_id))
    });
    for (block_id, desired, current) in &moves {
        push_operation(
            &mut operations,
            SemanticOperation::MoveSubtree {
                root: BlockLocation {
                    block_id: *block_id,
                    home_document_id: current.block.home_document_id,
                },
                from_page_id: current.page_id,
                to_page_id: desired.page_id,
                parent: desired.parent,
                order: desired.order.clone(),
            },
        )?;
    }

    let moved_blocks = moves
        .iter()
        .map(|(block_id, _, _)| *block_id)
        .collect::<BTreeSet<_>>();
    for (block_id, desired) in &desired_blocks {
        let Some(current) = current_blocks.get(block_id) else {
            continue;
        };
        if !moved_blocks.contains(block_id)
            && (current.block.parent != desired.parent || current.block.order != desired.order)
        {
            push_operation(
                &mut operations,
                SemanticOperation::ReorderBlock {
                    block_id: *block_id,
                    page_id: desired.page_id,
                    parent: desired.parent,
                    order: desired.order.clone(),
                },
            )?;
        }
    }

    let mut deletions = current_blocks
        .iter()
        .filter_map(|(block_id, current)| {
            (!desired_blocks.contains_key(block_id)
                && current
                    .block
                    .parent
                    .is_none_or(|parent| desired_blocks.contains_key(&parent)))
            .then_some((*block_id, current.page_id))
        })
        .collect::<Vec<_>>();
    deletions.sort_unstable();
    for (block_id, page_id) in deletions {
        push_operation(
            &mut operations,
            SemanticOperation::DeleteSubtree {
                root_block_id: block_id,
                page_id,
            },
        )?;
    }

    for (block_id, desired) in &desired_blocks {
        let Some(current) = current_blocks.get(block_id) else {
            continue;
        };
        if current.block.content != desired.content {
            push_operation(
                &mut operations,
                SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: *block_id,
                        home_document_id: current.block.home_document_id,
                    },
                    content: desired.content.clone(),
                },
            )?;
        }
    }
    for (block_id, desired) in &desired_blocks {
        let current_uuid = current_blocks
            .get(block_id)
            .map(|current| current.block.logseq_uuid);
        let mutation = match (current_uuid.flatten(), desired.logseq_uuid) {
            (None, Some(logseq_uuid)) => {
                Some(LogseqIdentityMutation::AssignExternal { logseq_uuid })
            }
            (Some(current), Some(logseq_uuid)) if current != logseq_uuid => {
                Some(LogseqIdentityMutation::ReplaceExternal { logseq_uuid })
            }
            (Some(_), None) => Some(LogseqIdentityMutation::RemoveExternal),
            (None, None) | (Some(_), Some(_)) => None,
        };
        if let Some(mutation) = mutation {
            push_operation(
                &mut operations,
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: BlockLocation {
                        block_id: *block_id,
                        home_document_id: desired.home_document_id,
                    },
                    mutation,
                },
            )?;
        }
    }

    for (path, page) in desired_pages {
        let Some(tree) = trees.get(path) else {
            continue;
        };
        let current_preamble = current_pages
            .get(&page.page_id)
            .map(|current| current.materialized_page().preamble.clone());
        if current_preamble != Some(tree.preamble.clone())
            && (page.existing || tree.preamble.is_some())
        {
            push_operation(
                &mut operations,
                SemanticOperation::SetPagePreamble {
                    page_id: page.page_id,
                    preamble: tree.preamble.clone(),
                },
            )?;
        }
    }

    let desired_page_ids = desired_pages
        .values()
        .map(|page| page.page_id)
        .collect::<BTreeSet<_>>();
    for page_id in current_pages.keys() {
        if !desired_page_ids.contains(page_id) {
            push_operation(
                &mut operations,
                SemanticOperation::DeletePage { page_id: *page_id },
            )?;
        }
    }

    if operations.is_empty() {
        let mut pages = Vec::new();
        for entry in observation.entries() {
            let Some(bytes) = entry.state().bytes() else {
                return Err(ImportExecutionError::InvalidMaterial(
                    "operation-free reconciliation contains an absent source".into(),
                ));
            };
            let desired = desired_pages.get(entry.path()).ok_or_else(|| {
                ImportExecutionError::InvalidMaterial(
                    "operation-free reconciliation has no desired page".into(),
                )
            })?;
            let current = current_pages.get(&desired.page_id).ok_or_else(|| {
                ImportExecutionError::InvalidMaterial(
                    "operation-free reconciliation has no accepted page".into(),
                )
            })?;
            if current.description() == super::BlobDescription::of(bytes) {
                continue;
            }
            pages.push(ImportFormattingPage {
                page_id: desired.page_id,
                path: entry.path().clone(),
                bytes: bytes.to_vec(),
                annotations: entry.state().annotations().to_vec(),
            });
        }
        if pages.is_empty() {
            return Err(ImportExecutionError::InvalidMaterial(
                "changed operation-free reconciliation has no formatting baseline to adopt".into(),
            ));
        }
        return Ok(BuiltImportMaterial::Formatting(ImportFormattingMaterial {
            pages,
        }));
    }
    let transaction = OperationTransaction::new(operations)
        .map_err(|error| ImportExecutionError::InvalidMaterial(error.to_string()))?;
    Ok(BuiltImportMaterial::Semantic(ImportExecutionMaterial {
        import_id,
        transaction,
        observation,
    }))
}

pub(crate) fn imported_order(sibling_position: u32) -> String {
    format!("{sibling_position:010}")
}

fn external_logseq_uuid(
    path: &ManagedPath,
    locator: &StructuralLocator,
    node: &ParsedNode,
    rejected_raw_ids: &BTreeSet<(ManagedPath, StructuralLocator)>,
) -> Option<LogseqUuid> {
    if rejected_raw_ids.contains(&(path.clone(), locator.clone())) || node.raw_ids.len() != 1 {
        return None;
    }
    LogseqUuid::parse(node.raw_ids[0].trim()).ok()
}

fn current_block_depth(
    block_id: BlockId,
    current_blocks: &BTreeMap<BlockId, CurrentImportBlock>,
) -> usize {
    let mut depth = 0_usize;
    let mut cursor = Some(block_id);
    let mut visited = BTreeSet::new();
    while let Some(block_id) = cursor {
        if !visited.insert(block_id) {
            return usize::MAX;
        }
        let Some(block) = current_blocks.get(&block_id) else {
            return usize::MAX;
        };
        depth = depth.saturating_add(1);
        cursor = block.block.parent;
    }
    depth
}

fn desired_block_depth(
    block_id: BlockId,
    desired_blocks: &BTreeMap<BlockId, DesiredImportBlock>,
) -> usize {
    let mut depth = 0_usize;
    let mut cursor = Some(block_id);
    let mut visited = BTreeSet::new();
    while let Some(block_id) = cursor {
        if !visited.insert(block_id) {
            return usize::MAX;
        }
        let Some(block) = desired_blocks.get(&block_id) else {
            return usize::MAX;
        };
        depth = depth.saturating_add(1);
        cursor = block.parent;
    }
    depth
}

fn blocked_inventory_error(
    error: InventoryError,
    instrumentation: ImportInstrumentation,
) -> ImportPlan {
    let (reason, paths) = match &error {
        InventoryError::UnsupportedManagedLayout { .. } => {
            (ImportBlockReason::UnsupportedManagedLayout, Vec::new())
        }
        InventoryError::UnsafePath(path) | InventoryError::DuplicateRequestedPath(path) => {
            (ImportBlockReason::UnsafeInput, vec![path.clone()])
        }
        InventoryError::PortablePathCollision { first, second } => (
            ImportBlockReason::PortablePathCollision,
            vec![first.clone(), second.clone()],
        ),
        InventoryError::ResourceBudgetExceeded { .. } => {
            (ImportBlockReason::ResourceLimit, Vec::new())
        }
        InventoryError::UnsafeEntry { path, .. } => (
            ImportBlockReason::UnsafeInput,
            path.iter().cloned().collect(),
        ),
    };
    ImportPlan {
        status: ImportPlanStatus::Blocked,
        import_id: None,
        inventory: None,
        matches: None,
        scope: None,
        execution: None,
        formatting: None,
        blocks: vec![ImportBlock {
            reason,
            paths,
            logical_completion_ids: Vec::new(),
            observation: None,
            detail: error.to_string(),
        }],
        instrumentation,
    }
}

fn blocked_authority_error(
    inventory: Option<RawInventory>,
    block: ImportBlock,
    instrumentation: ImportInstrumentation,
) -> ImportPlan {
    ImportPlan {
        status: ImportPlanStatus::Blocked,
        import_id: None,
        inventory,
        matches: None,
        scope: None,
        execution: None,
        formatting: None,
        blocks: vec![block],
        instrumentation,
    }
}

fn receipt_block(
    reason: ImportBlockReason,
    path: &ManagedPath,
    completion_id: Option<LogicalCompletionId>,
    inventory: &RawInventory,
    detail: impl Into<String>,
) -> ImportBlock {
    ImportBlock {
        reason,
        paths: vec![path.as_str().to_owned()],
        logical_completion_ids: completion_id.into_iter().collect(),
        observation: inventory_observation(inventory, path.as_str()),
        detail: detail.into(),
    }
}

fn inventory_observation(
    inventory: &RawInventory,
    path: &str,
) -> Option<(ManagedPath, ImportInventoryState)> {
    inventory
        .entries()
        .iter()
        .find(|(candidate, _)| candidate.as_str() == path)
        .map(|(path, observation)| {
            let state = match observation {
                RawObservation::Present(bytes) => {
                    ImportInventoryState::Present(bytes.description())
                }
                RawObservation::Absent => ImportInventoryState::Absent,
            };
            (path.clone(), state)
        })
}

fn match_pages(
    inventory: &RawInventory,
    completed: &[&ReceiptBackedPage],
    instrumentation: &mut ImportInstrumentation,
) -> Result<Vec<PageImportMatch>, ImportBlock> {
    let completed_paths = completed
        .iter()
        .map(|page| page.path().clone())
        .collect::<BTreeSet<_>>();
    let mut new_by_description = BTreeMap::<BlobDescription, Vec<&ManagedPath>>::new();
    for (path, observation) in inventory.entries() {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        if completed_paths.contains(path) {
            continue;
        }
        if let RawObservation::Present(bytes) = observation {
            new_by_description
                .entry(bytes.description())
                .or_default()
                .push(path);
        }
    }

    let mut source_to_candidate = BTreeMap::<ManagedPath, ManagedPath>::new();
    let mut candidate_to_sources = BTreeMap::<ManagedPath, Vec<&ReceiptBackedPage>>::new();
    for page in completed {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        if !matches!(
            inventory.entries().get(page.path()),
            Some(RawObservation::Absent)
        ) {
            continue;
        }
        let candidates = new_by_description
            .get(&page.description())
            .into_iter()
            .flatten()
            .filter(|path| {
                instrumentation.inventory_path_lookups =
                    instrumentation.inventory_path_lookups.saturating_add(1);
                inventory.entries().get(*path).is_some_and(|observation| {
                    matches!(observation, RawObservation::Present(bytes) if bytes.bytes() == page.bytes())
                })
            })
            .copied()
            .collect::<Vec<_>>();
        if candidates.len() > 1 {
            return Err(ImportBlock {
                reason: ImportBlockReason::AmbiguousDestructiveMatch,
                paths: std::iter::once(page.path().as_str().to_owned())
                    .chain(
                        candidates
                            .iter()
                            .map(|candidate| candidate.as_str().to_owned()),
                    )
                    .collect(),
                logical_completion_ids: vec![page.logical_completion_id()],
                observation: inventory_observation(inventory, page.path().as_str()),
                detail: "one absent receipt path has multiple exact new-path candidates".into(),
            });
        }
        if let Some(candidate) = candidates.first() {
            source_to_candidate.insert(page.path().clone(), (*candidate).clone());
            candidate_to_sources
                .entry((*candidate).clone())
                .or_default()
                .push(page);
        }
    }
    if let Some((candidate, sources)) = candidate_to_sources
        .iter()
        .find(|(_, sources)| sources.len() > 1)
    {
        return Err(ImportBlock {
            reason: ImportBlockReason::AmbiguousDestructiveMatch,
            paths: sources
                .iter()
                .map(|page| page.path().as_str().to_owned())
                .chain(std::iter::once(candidate.as_str().to_owned()))
                .collect(),
            logical_completion_ids: sources
                .iter()
                .map(|page| page.logical_completion_id())
                .collect(),
            observation: inventory_observation(inventory, candidate.as_str()),
            detail: "multiple absent receipt paths claim one exact new path".into(),
        });
    }

    let mut matches = Vec::new();
    for page in completed {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        match inventory.entries().get(page.path()) {
            Some(RawObservation::Present(_)) => matches.push(PageImportMatch {
                path: page.path().clone(),
                previous_path: page.path().clone(),
                page_id: page.page_id(),
                basis: PageMatchBasis::SamePathCompletion,
            }),
            Some(RawObservation::Absent) => {
                if let Some(path) = source_to_candidate.get(page.path()) {
                    matches.push(PageImportMatch {
                        path: path.clone(),
                        previous_path: page.path().clone(),
                        page_id: page.page_id(),
                        basis: PageMatchBasis::ReceiptBackedExactRename,
                    });
                }
            }
            None => unreachable!("receipt paths are required in the affected inventory"),
        }
    }
    matches.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(matches)
}

struct ParsedNode {
    parent: Option<usize>,
    sibling_position: u32,
    depth: usize,
    children: Vec<usize>,
    span: StructuralSpan,
    raw: String,
    raw_ids: Vec<String>,
}

struct ParsedTree {
    path: ManagedPath,
    preamble: Option<String>,
    roots: Vec<usize>,
    nodes: Vec<ParsedNode>,
}

struct ParsedExternalTree {
    tree: ParsedTree,
    explicit_title: Option<String>,
    filename_fallback: PageEntry,
    effective: PageEntry,
}

impl std::ops::Deref for ParsedExternalTree {
    type Target = ParsedTree;

    fn deref(&self) -> &Self::Target {
        &self.tree
    }
}

struct ParsedImportDocuments {
    current: BTreeMap<ManagedPath, ParsedExternalTree>,
    base: BTreeMap<ManagedPath, ParsedExternalTree>,
}

/// Preserve a moved page's receipt-backed identity after block matching only
/// when a unique Logseq UUID joins an absent source to an unmatched present
/// destination. Content similarity is deliberately not page-move evidence:
/// it cannot distinguish a move from an unrelated delete/create.
fn match_anchored_page_moves(
    inventory: &RawInventory,
    completed: &[&ReceiptBackedPage],
    matches: &mut ImportMatches,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    instrumentation.anchored_page_match_set_inserts = instrumentation
        .anchored_page_match_set_inserts
        .saturating_add(matches.pages.len().saturating_mul(2));
    let matched_sources = matches
        .pages
        .iter()
        .map(|matched| matched.previous_path.clone())
        .collect::<BTreeSet<_>>();
    let matched_destinations = matches
        .pages
        .iter()
        .map(|matched| matched.path.clone())
        .collect::<BTreeSet<_>>();

    let mut block_owners = BTreeMap::<BlockId, Vec<&ReceiptBackedPage>>::new();
    let mut uuid_owners = BTreeMap::<LogseqUuid, Vec<&ReceiptBackedPage>>::new();
    for page in completed {
        for annotation in page.annotations() {
            instrumentation.anchored_page_owner_inserts = instrumentation
                .anchored_page_owner_inserts
                .saturating_add(1);
            block_owners
                .entry(annotation.block_id())
                .or_default()
                .push(page);
            if let Some(uuid) = annotation.logseq_uuid() {
                instrumentation.anchored_page_uuid_owner_inserts = instrumentation
                    .anchored_page_uuid_owner_inserts
                    .saturating_add(1);
                uuid_owners.entry(uuid).or_default().push(page);
            }
        }
    }

    for rejected in matches
        .rejected_raw_ids
        .iter()
        .filter(|rejected| rejected.reason == RejectedRawIdReason::Duplicate)
    {
        let Ok(uuid) = LogseqUuid::parse(rejected.raw_value.trim()) else {
            continue;
        };
        instrumentation.anchored_page_uuid_owner_lookups = instrumentation
            .anchored_page_uuid_owner_lookups
            .saturating_add(1);
        let Some(owners) = uuid_owners.get(&uuid) else {
            continue;
        };
        let destructive_owners = owners
            .iter()
            .copied()
            .filter(|page| {
                instrumentation.anchored_page_match_set_lookups = instrumentation
                    .anchored_page_match_set_lookups
                    .saturating_add(2);
                instrumentation.inventory_path_lookups =
                    instrumentation.inventory_path_lookups.saturating_add(2);
                page.path() != &rejected.path
                    && !matched_sources.contains(page.path())
                    && !matched_destinations.contains(&rejected.path)
                    && matches!(
                        inventory.entries().get(page.path()),
                        Some(RawObservation::Absent)
                    )
                    && matches!(
                        inventory.entries().get(&rejected.path),
                        Some(RawObservation::Present(_))
                    )
            })
            .collect::<Vec<_>>();
        if !destructive_owners.is_empty() {
            return Err(ImportBlock {
                reason: ImportBlockReason::AmbiguousDestructiveMatch,
                paths: destructive_owners
                    .iter()
                    .map(|page| page.path().as_str().to_owned())
                    .chain(std::iter::once(rejected.path.as_str().to_owned()))
                    .collect(),
                logical_completion_ids: destructive_owners
                    .iter()
                    .map(|page| page.logical_completion_id())
                    .collect(),
                observation: inventory_observation(inventory, rejected.path.as_str()),
                detail: format!(
                    "duplicate UUID {uuid} is ambiguous destructive page-move evidence"
                ),
            });
        }
    }

    let mut source_destinations =
        BTreeMap::<ManagedPath, (PageId, LogicalCompletionId, BTreeSet<ManagedPath>)>::new();
    let mut destination_sources =
        BTreeMap::<ManagedPath, BTreeSet<(ManagedPath, PageId, LogicalCompletionId)>>::new();
    for block_match in matches
        .blocks
        .iter()
        .filter(|matched| matched.basis == BlockMatchBasis::UniqueLogseqUuid)
    {
        instrumentation.anchored_page_owner_lookups = instrumentation
            .anchored_page_owner_lookups
            .saturating_add(1);
        let Some(owners) = block_owners.get(&block_match.block_id) else {
            continue;
        };
        if owners.len() != 1 {
            return Err(ImportBlock {
                reason: ImportBlockReason::DuplicateAnchorDependent,
                paths: owners
                    .iter()
                    .map(|page| page.path().as_str().to_owned())
                    .collect(),
                logical_completion_ids: owners
                    .iter()
                    .map(|page| page.logical_completion_id())
                    .collect(),
                observation: inventory_observation(inventory, block_match.path.as_str()),
                detail: format!(
                    "block {} has multiple receipt-backed page owners",
                    block_match.block_id
                ),
            });
        }
        let page = owners[0];
        let source = page.path();
        let destination = &block_match.path;
        instrumentation.anchored_page_match_set_lookups = instrumentation
            .anchored_page_match_set_lookups
            .saturating_add(2);
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(2);
        if source == destination
            || matched_sources.contains(source)
            || matched_destinations.contains(destination)
            || !matches!(
                inventory.entries().get(source),
                Some(RawObservation::Absent)
            )
            || !matches!(
                inventory.entries().get(destination),
                Some(RawObservation::Present(_))
            )
        {
            continue;
        }
        instrumentation.anchored_page_edge_inserts =
            instrumentation.anchored_page_edge_inserts.saturating_add(2);
        source_destinations
            .entry(source.clone())
            .or_insert_with(|| {
                (
                    page.page_id(),
                    page.logical_completion_id(),
                    BTreeSet::new(),
                )
            })
            .2
            .insert(destination.clone());
        destination_sources
            .entry(destination.clone())
            .or_default()
            .insert((source.clone(), page.page_id(), page.logical_completion_id()));
    }

    if let Some((source, (_, completion_id, destinations))) = source_destinations
        .iter()
        .find(|(_, (_, _, destinations))| destinations.len() > 1)
    {
        return Err(ImportBlock {
            reason: ImportBlockReason::AmbiguousDestructiveMatch,
            paths: std::iter::once(source.as_str().to_owned())
                .chain(
                    destinations
                        .iter()
                        .map(|destination| destination.as_str().to_owned()),
                )
                .collect(),
            logical_completion_ids: vec![*completion_id],
            observation: inventory_observation(inventory, source.as_str()),
            detail: "one absent receipt page anchors to multiple present destinations".into(),
        });
    }
    if let Some((destination, sources)) = destination_sources
        .iter()
        .find(|(_, sources)| sources.len() > 1)
    {
        return Err(ImportBlock {
            reason: ImportBlockReason::AmbiguousDestructiveMatch,
            paths: sources
                .iter()
                .map(|(source, _, _)| source.as_str().to_owned())
                .chain(std::iter::once(destination.as_str().to_owned()))
                .collect(),
            logical_completion_ids: sources
                .iter()
                .map(|(_, _, completion_id)| *completion_id)
                .collect(),
            observation: inventory_observation(inventory, destination.as_str()),
            detail: "multiple absent receipt pages anchor to one present destination".into(),
        });
    }

    for (source, (page_id, _, destinations)) in source_destinations {
        let destination = destinations
            .into_iter()
            .next()
            .expect("empty anchor sets are never inserted");
        matches.pages.push(PageImportMatch {
            path: destination,
            previous_path: source,
            page_id,
            basis: PageMatchBasis::ReceiptBackedAnchoredRename,
        });
    }
    matches
        .pages
        .sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(())
}

fn parse_external_nodes(
    graph: &Graph,
    path: &ManagedPath,
    bytes: &[u8],
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedExternalTree, ImportBlock> {
    let is_org = path.is_org();
    let parsed = graph
        .parse_external_document(path, bytes, true)
        .map_err(|error| {
            authority_block(
                ImportBlockReason::UnsafeInput,
                Some(path),
                format!("external document parser rejected source: {error}"),
            )
        })?;
    enforce_outline_limits(path, &parsed.parsed, instrumentation.parsed_nodes)?;
    if parsed.source_round_trips != Some(true) {
        return Err(authority_block(
            ImportBlockReason::UnsafeInput,
            Some(path),
            if is_org {
                "external Org source is byte-preserved and read-only because its heading structure is not editable and does not round-trip exactly"
            } else {
                "external Markdown source is byte-preserved and read-only because parsing and reserialization change its block structure"
            },
        ));
    }
    let tree = flatten_document(path, parsed.parsed, instrumentation)?;
    Ok(ParsedExternalTree {
        tree,
        explicit_title: parsed.explicit_title,
        filename_fallback: parsed.filename_fallback,
        effective: parsed.effective,
    })
}

fn parse_nodes(
    path: &ManagedPath,
    bytes: &[u8],
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedTree, ImportBlock> {
    let text = std::str::from_utf8(bytes).expect("UTF-8 checked before semantic parsing");
    let is_org = path.is_org();
    let parsed = if is_org {
        crate::org::try_parse_org_with_source_spans(text)
    } else {
        crate::doc::try_parse_with_source_spans(text)
    }
    .map_err(|error| {
        authority_block(
            ImportBlockReason::UnsafeInput,
            Some(path),
            format!("lsdoc outline cannot be represented safely: {error}"),
        )
    })?;
    enforce_outline_limits(path, &parsed, instrumentation.parsed_nodes)?;
    let source_admitted = if is_org {
        crate::org::org_editable_parsed(text, &parsed)
    } else {
        crate::doc::markdown_structurally_round_trips_parsed(text, &parsed)
    };
    if !source_admitted {
        return Err(authority_block(
            ImportBlockReason::UnsafeInput,
            Some(path),
            if is_org {
                "external Org source is byte-preserved and read-only because its heading structure is not editable and does not round-trip exactly"
            } else {
                "external Markdown source is byte-preserved and read-only because parsing and reserialization change its block structure"
            },
        ));
    }
    flatten_document(path, parsed, instrumentation)
}

fn enforce_outline_limits(
    path: &ManagedPath,
    parsed: &crate::doc::ParsedDocument,
    parsed_nodes: usize,
) -> Result<(), ImportBlock> {
    if parsed.outline_depth > MAX_IMPORT_DEPTH {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "parser-owned document nesting depth {} exceeds import limit {MAX_IMPORT_DEPTH}",
                parsed.outline_depth
            ),
        ));
    }
    let observed = parsed_nodes.saturating_add(parsed.outline_nodes);
    if observed > MAX_IMPORT_PARSED_NODES {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "parser-owned outline exceeds parsed-node budget: observed {observed}, limit {MAX_IMPORT_PARSED_NODES}"
            ),
        ));
    }
    Ok(())
}

fn flatten_document(
    path: &ManagedPath,
    parsed: crate::doc::ParsedDocument,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedTree, ImportBlock> {
    let spans = parsed
        .block_spans
        .into_iter()
        .map(|span| {
            StructuralSpan::new(span.start as u64, span.end as u64).map_err(|error| {
                authority_block(
                    ImportBlockReason::UnsafeInput,
                    Some(path),
                    error.to_string(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let document = parsed.document;
    let mut nodes = Vec::<ParsedNode>::new();
    let mut roots = Vec::new();
    let mut pending = document
        .roots
        .iter()
        .enumerate()
        .rev()
        .map(|(position, block)| (block, None, position as u32, 1_usize))
        .collect::<Vec<_>>();
    while let Some((block, parent, sibling_position, depth)) = pending.pop() {
        if depth > MAX_IMPORT_DEPTH {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!("parsed document depth exceeds import limit {MAX_IMPORT_DEPTH}"),
            ));
        }
        if instrumentation.parsed_nodes == MAX_IMPORT_PARSED_NODES {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!("parsed-node budget exceeded: limit {MAX_IMPORT_PARSED_NODES}"),
            ));
        }
        let raw_ids = block
            .properties()
            .into_iter()
            .filter_map(|(key, value)| {
                (crate::doc::property_key_norm(&key) == "id").then_some(value)
            })
            .collect();
        let index = nodes.len();
        let span = spans.get(index).copied().ok_or_else(|| {
            authority_block(
                ImportBlockReason::UnsafeInput,
                Some(path),
                "parser tree has more blocks than exact source-span capture",
            )
        })?;
        nodes.push(ParsedNode {
            parent,
            sibling_position,
            depth,
            children: Vec::with_capacity(block.children.len()),
            span,
            raw: block.raw.clone(),
            raw_ids,
        });
        instrumentation.parsed_nodes = instrumentation.parsed_nodes.saturating_add(1);
        instrumentation.max_depth = instrumentation.max_depth.max(depth);
        if let Some(parent) = parent {
            nodes[parent].children.push(index);
        } else {
            roots.push(index);
        }
        for (position, child) in block.children.iter().enumerate().rev() {
            pending.push((child, Some(index), position as u32, depth.saturating_add(1)));
        }
    }
    if spans.len() != nodes.len() {
        return Err(authority_block(
            ImportBlockReason::UnsafeInput,
            Some(path),
            "exact source-span capture disagrees with the parsed block tree",
        ));
    }
    Ok(ParsedTree {
        path: path.clone(),
        preamble: document.pre_block.clone(),
        roots,
        nodes,
    })
}

fn materialize_locator(
    tree: &ParsedTree,
    index: usize,
    instrumentation: &mut ImportInstrumentation,
) -> Result<StructuralLocator, ImportBlock> {
    let depth = tree.nodes[index].depth;
    let next = instrumentation
        .locator_components_materialized
        .saturating_add(depth);
    if next > MAX_IMPORT_LOCATOR_COMPONENTS {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(&tree.path),
            format!(
                "structural-locator component budget exceeded: observed {next}, limit {MAX_IMPORT_LOCATOR_COMPONENTS}"
            ),
        ));
    }
    instrumentation.locator_components_materialized = next;
    let mut components = Vec::with_capacity(depth);
    let mut cursor = Some(index);
    while let Some(node) = cursor {
        components.push(tree.nodes[node].sibling_position);
        cursor = tree.nodes[node].parent;
    }
    components.reverse();
    StructuralLocator::new(components).map_err(|error| {
        authority_block(
            ImportBlockReason::CorruptBase,
            Some(&tree.path),
            error.to_string(),
        )
    })
}

fn resolve_locator(
    tree: &ParsedTree,
    locator: &StructuralLocator,
    instrumentation: &mut ImportInstrumentation,
) -> Result<Option<usize>, ImportBlock> {
    let next = instrumentation
        .locator_components_materialized
        .saturating_add(locator.components().len());
    if next > MAX_IMPORT_LOCATOR_COMPONENTS {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(&tree.path),
            format!(
                "structural-locator component budget exceeded: observed {next}, limit {MAX_IMPORT_LOCATOR_COMPONENTS}"
            ),
        ));
    }
    instrumentation.locator_components_materialized = next;
    let mut components = locator.components().iter().copied();
    let Some(root) = components.next() else {
        return Ok(None);
    };
    let Some(mut current) = tree.roots.get(root as usize).copied() else {
        return Ok(None);
    };
    for component in components {
        let Some(child) = tree.nodes[current]
            .children
            .get(component as usize)
            .copied()
        else {
            return Ok(None);
        };
        current = child;
    }
    Ok(Some(current))
}

struct StructuralClassEntry {
    raw: String,
    child_classes: Vec<usize>,
    class: usize,
}

#[derive(Default)]
struct StructuralInterner {
    buckets: HashMap<ContentDigest, Vec<StructuralClassEntry>>,
    next_class: usize,
}

impl StructuralInterner {
    fn new() -> Self {
        Self::default()
    }
}

/// Assign exact structural classes through a digest index whose candidates are
/// always collision-checked against raw bytes and child classes. Hash-table
/// lookup avoids ordered vector-key comparisons with adversarial common
/// prefixes, and every candidate comparison is charged.
fn structural_classes(
    tree: &ParsedTree,
    interner: &mut StructuralInterner,
    instrumentation: &mut ImportInstrumentation,
) -> Result<Vec<usize>, ImportBlock> {
    let mut classes = vec![0; tree.nodes.len()];
    for index in (0..tree.nodes.len()).rev() {
        let child_classes = tree.nodes[index]
            .children
            .iter()
            .map(|child| classes[*child])
            .collect::<Vec<_>>();
        instrumentation.structural_key_components = instrumentation
            .structural_key_components
            .saturating_add(1)
            .saturating_add(child_classes.len());
        if instrumentation.structural_key_components > MAX_IMPORT_STRUCTURAL_KEY_WORK {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(&tree.path),
                format!(
                    "structural key component budget exceeded: limit {MAX_IMPORT_STRUCTURAL_KEY_WORK}"
                ),
            ));
        }
        let node = &tree.nodes[index];
        let mut hasher = Sha256::new();
        hasher.update(b"tine/import-structural-class/v1\0");
        hasher.update((node.raw.len() as u64).to_be_bytes());
        hasher.update(node.raw.as_bytes());
        hasher.update((child_classes.len() as u64).to_be_bytes());
        for class in &child_classes {
            hasher.update((*class as u64).to_be_bytes());
        }
        instrumentation.bytes_hashed = instrumentation
            .bytes_hashed
            .saturating_add(node.raw.len() as u64)
            .saturating_add((child_classes.len() as u64).saturating_mul(8));
        let digest = ContentDigest::from_bytes(hasher.finalize().into());
        let bucket = interner.buckets.entry(digest).or_default();
        let mut class = None;
        for candidate in bucket.iter() {
            instrumentation.structural_key_comparisons = instrumentation
                .structural_key_comparisons
                .saturating_add(node.raw.len())
                .saturating_add(child_classes.len());
            if instrumentation.structural_key_comparisons > MAX_IMPORT_STRUCTURAL_KEY_WORK {
                return Err(authority_block(
                    ImportBlockReason::ResourceLimit,
                    Some(&tree.path),
                    format!(
                        "structural key comparison budget exceeded: limit {MAX_IMPORT_STRUCTURAL_KEY_WORK}"
                    ),
                ));
            }
            if candidate.raw == node.raw && candidate.child_classes == child_classes {
                class = Some(candidate.class);
                break;
            }
        }
        let class = match class {
            Some(class) => class,
            None => {
                let class = interner.next_class;
                interner.next_class = interner.next_class.saturating_add(1);
                instrumentation.structural_class_allocations = instrumentation
                    .structural_class_allocations
                    .saturating_add(1);
                bucket.push(StructuralClassEntry {
                    raw: node.raw.clone(),
                    child_classes,
                    class,
                });
                class
            }
        };
        classes[index] = class;
    }
    Ok(classes)
}

fn match_blocks(
    graph: &Graph,
    inventory: &RawInventory,
    completed: &[&ReceiptBackedPage],
    matches: &mut ImportMatches,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedImportDocuments, ImportBlock> {
    let mut external_by_path = BTreeMap::<ManagedPath, ParsedExternalTree>::new();
    for (path, observation) in inventory.entries() {
        if let RawObservation::Present(bytes) = observation {
            instrumentation.present_document_parses =
                instrumentation.present_document_parses.saturating_add(1);
            external_by_path.insert(
                path.clone(),
                parse_external_nodes(graph, path, bytes.bytes(), instrumentation)?,
            );
        }
    }
    let mut base_by_path = BTreeMap::<ManagedPath, ParsedExternalTree>::new();
    for page in completed {
        instrumentation.authenticated_base_document_parses = instrumentation
            .authenticated_base_document_parses
            .saturating_add(1);
        base_by_path.insert(
            page.path().clone(),
            parse_external_nodes(graph, page.path(), page.bytes(), instrumentation).map_err(
                |mut block| {
                    block.reason = ImportBlockReason::CorruptBase;
                    block.logical_completion_ids = vec![page.logical_completion_id()];
                    block.detail = format!(
                        "authenticated completed-base document is not parseable within import limits: {}",
                        block.detail
                    );
                    block
                },
            )?,
        );
    }

    let mut external_anchors = BTreeMap::<LogseqUuid, Vec<(ManagedPath, usize, String)>>::new();
    let mut rejected = BTreeSet::<(ManagedPath, usize)>::new();
    for tree in external_by_path.values() {
        for (index, node) in tree.nodes.iter().enumerate() {
            if node.raw_ids.is_empty() {
                continue;
            }
            if node.raw_ids.len() != 1 {
                rejected.insert((tree.path.clone(), index));
                for raw_id in &node.raw_ids {
                    let reason = if LogseqUuid::parse(raw_id.trim()).is_ok() {
                        RejectedRawIdReason::Duplicate
                    } else {
                        RejectedRawIdReason::InvalidSyntax
                    };
                    matches.rejected_raw_ids.push(RejectedRawId {
                        path: tree.path.clone(),
                        locator: materialize_locator(tree, index, instrumentation)?,
                        raw_value: raw_id.clone(),
                        reason,
                    });
                }
                continue;
            }
            let raw_id = &node.raw_ids[0];
            match LogseqUuid::parse(raw_id.trim()) {
                Ok(uuid) => external_anchors.entry(uuid).or_default().push((
                    tree.path.clone(),
                    index,
                    raw_id.clone(),
                )),
                Err(_) => {
                    rejected.insert((tree.path.clone(), index));
                    matches.rejected_raw_ids.push(RejectedRawId {
                        path: tree.path.clone(),
                        locator: materialize_locator(tree, index, instrumentation)?,
                        raw_value: raw_id.clone(),
                        reason: RejectedRawIdReason::InvalidSyntax,
                    });
                }
            }
        }
    }
    for owners in external_anchors.values().filter(|owners| owners.len() > 1) {
        for (path, index, raw_value) in owners {
            rejected.insert((path.clone(), *index));
            let tree = &external_by_path[path];
            matches.rejected_raw_ids.push(RejectedRawId {
                path: path.clone(),
                locator: materialize_locator(tree, *index, instrumentation)?,
                raw_value: raw_value.clone(),
                reason: RejectedRawIdReason::Duplicate,
            });
        }
    }
    instrumentation.rejected_raw_id_occurrences = matches.rejected_raw_ids.len();
    matches.rejected_raw_ids.sort_unstable_by(|left, right| {
        (&left.path, &left.locator, &left.raw_value).cmp(&(
            &right.path,
            &right.locator,
            &right.raw_value,
        ))
    });

    let mut receipt_anchors =
        BTreeMap::<LogseqUuid, Vec<(BlockId, LogicalCompletionId, ManagedPath, usize)>>::new();
    let mut annotations_by_path = BTreeMap::<ManagedPath, BTreeMap<usize, BlockId>>::new();
    for page in completed {
        let tree = &base_by_path[page.path()];
        let mut annotations = BTreeMap::new();
        for annotation in page.annotations() {
            let Some(index) = resolve_locator(tree, annotation.locator(), instrumentation)? else {
                continue;
            };
            annotations.insert(index, annotation.block_id());
            if let Some(uuid) = annotation.logseq_uuid() {
                receipt_anchors.entry(uuid).or_default().push((
                    annotation.block_id(),
                    page.logical_completion_id(),
                    page.path().clone(),
                    index,
                ));
            }
        }
        annotations_by_path.insert(page.path().clone(), annotations);
    }
    let mut matched_external = BTreeSet::<(ManagedPath, usize)>::new();
    let mut matched_base = BTreeMap::<(ManagedPath, usize), (ManagedPath, usize)>::new();
    let mut used_blocks = BTreeSet::<BlockId>::new();
    for (uuid, owners) in external_anchors
        .iter()
        .filter(|(_, owners)| owners.len() == 1)
    {
        let Some(receipt_owners) = receipt_anchors.get(uuid) else {
            continue;
        };
        if receipt_owners.len() != 1 {
            let (path, _, _) = &owners[0];
            return Err(ImportBlock {
                reason: ImportBlockReason::DuplicateAnchorDependent,
                paths: vec![path.as_str().to_owned()],
                logical_completion_ids: receipt_owners
                    .iter()
                    .map(|(_, completion, _, _)| *completion)
                    .collect(),
                observation: inventory_observation(inventory, path.as_str()),
                detail: format!("UUID {uuid} has multiple receipt-backed owners"),
            });
        }
        let (path, external_index, _) = &owners[0];
        let (block_id, _, base_path, base_index) = &receipt_owners[0];
        let external_tree = &external_by_path[path];
        matches.blocks.push(BlockImportMatch {
            path: path.clone(),
            locator: materialize_locator(external_tree, *external_index, instrumentation)?,
            block_id: *block_id,
            basis: BlockMatchBasis::UniqueLogseqUuid,
        });
        used_blocks.insert(*block_id);
        matched_external.insert((path.clone(), *external_index));
        matched_base.insert(
            (base_path.clone(), *base_index),
            (path.clone(), *external_index),
        );
    }

    let mut structural_interner = StructuralInterner::new();
    let mut base_classes_by_path = BTreeMap::new();
    for (path, tree) in &base_by_path {
        let classes = structural_classes(tree, &mut structural_interner, instrumentation)?;
        instrumentation.structural_class_nodes = instrumentation
            .structural_class_nodes
            .saturating_add(tree.nodes.len());
        base_classes_by_path.insert(path.clone(), classes);
    }
    let mut external_classes_by_path = BTreeMap::new();
    for (path, tree) in &external_by_path {
        let classes = structural_classes(tree, &mut structural_interner, instrumentation)?;
        instrumentation.structural_class_nodes = instrumentation
            .structural_class_nodes
            .saturating_add(tree.nodes.len());
        external_classes_by_path.insert(path.clone(), classes);
    }

    let mut base_exact = BTreeMap::<usize, Vec<(ManagedPath, usize, BlockId)>>::new();
    for (path, tree) in &base_by_path {
        let annotations = &annotations_by_path[path];
        let classes = &base_classes_by_path[path];
        for index in 0..tree.nodes.len() {
            if annotations.contains_key(&index)
                && !matched_base.contains_key(&(path.clone(), index))
            {
                instrumentation.exact_bucket_inserts =
                    instrumentation.exact_bucket_inserts.saturating_add(1);
                base_exact.entry(classes[index]).or_default().push((
                    path.clone(),
                    index,
                    annotations[&index],
                ));
            }
        }
    }
    let mut external_exact = BTreeMap::<usize, Vec<(ManagedPath, usize)>>::new();
    for (path, tree) in &external_by_path {
        let classes = &external_classes_by_path[path];
        for index in 0..tree.nodes.len() {
            let key = (path.clone(), index);
            if !rejected.contains(&key) && !matched_external.contains(&key) {
                instrumentation.exact_bucket_inserts =
                    instrumentation.exact_bucket_inserts.saturating_add(1);
                external_exact
                    .entry(classes[index])
                    .or_default()
                    .push((path.clone(), index));
            }
        }
    }
    let base_class_counts = base_exact
        .iter()
        .map(|(class, candidates)| (*class, candidates.len()))
        .collect::<BTreeMap<_, _>>();
    let external_class_counts = external_exact
        .iter()
        .map(|(class, candidates)| (*class, candidates.len()))
        .collect::<BTreeMap<_, _>>();
    for (class, base_candidates) in &base_exact {
        instrumentation.exact_bucket_lookups =
            instrumentation.exact_bucket_lookups.saturating_add(1);
        let Some(external_candidates) = external_exact.get(class) else {
            continue;
        };
        if base_candidates.len() != 1 || external_candidates.len() != 1 {
            continue;
        }
        let (base_path, base_index, block_id) = &base_candidates[0];
        let (external_path, external_index) = &external_candidates[0];
        if used_blocks.insert(*block_id) {
            record_block_match(
                matches,
                &mut matched_external,
                &mut matched_base,
                base_path,
                *base_index,
                &external_by_path[external_path],
                *external_index,
                *block_id,
                BlockMatchBasis::ReceiptStructuralExact,
                instrumentation,
            )?;
        }
    }

    let page_matches = matches.pages.clone();
    for page_match in &page_matches {
        let base_tree = &base_by_path[&page_match.previous_path];
        let external_tree = &external_by_path[&page_match.path];
        let annotations = &annotations_by_path[&page_match.previous_path];
        align_ordered_tree(
            &page_match.previous_path,
            base_tree,
            external_tree,
            &base_classes_by_path[&page_match.previous_path],
            &external_classes_by_path[&page_match.path],
            &base_class_counts,
            &external_class_counts,
            annotations,
            &rejected,
            &mut used_blocks,
            &mut matched_external,
            &mut matched_base,
            matches,
            instrumentation,
        )?;
    }
    matches.blocks.sort_unstable_by(|left, right| {
        (&left.path, &left.locator).cmp(&(&right.path, &right.locator))
    });
    Ok(ParsedImportDocuments {
        current: external_by_path,
        base: base_by_path,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_block_match(
    matches: &mut ImportMatches,
    matched_external: &mut BTreeSet<(ManagedPath, usize)>,
    matched_base: &mut BTreeMap<(ManagedPath, usize), (ManagedPath, usize)>,
    base_path: &ManagedPath,
    base_index: usize,
    external_tree: &ParsedTree,
    external_index: usize,
    block_id: BlockId,
    basis: BlockMatchBasis,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    matches.blocks.push(BlockImportMatch {
        path: external_tree.path.clone(),
        locator: materialize_locator(external_tree, external_index, instrumentation)?,
        block_id,
        basis,
    });
    matched_external.insert((external_tree.path.clone(), external_index));
    matched_base.insert(
        (base_path.clone(), base_index),
        (external_tree.path.clone(), external_index),
    );
    instrumentation.retained_block_matches =
        instrumentation.retained_block_matches.saturating_add(1);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn align_ordered_tree(
    base_path: &ManagedPath,
    base_tree: &ParsedTree,
    external_tree: &ParsedTree,
    base_classes: &[usize],
    external_classes: &[usize],
    base_class_counts: &BTreeMap<usize, usize>,
    external_class_counts: &BTreeMap<usize, usize>,
    annotations: &BTreeMap<usize, BlockId>,
    rejected: &BTreeSet<(ManagedPath, usize)>,
    used_blocks: &mut BTreeSet<BlockId>,
    matched_external: &mut BTreeSet<(ManagedPath, usize)>,
    matched_base: &mut BTreeMap<(ManagedPath, usize), (ManagedPath, usize)>,
    matches: &mut ImportMatches,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    let mut pending = vec![(None, None)];
    pending.extend(
        matched_base
            .iter()
            .filter(|((path, _), (external_path, _))| {
                path == base_path && external_path == &external_tree.path
            })
            .map(|((_, base), (_, external))| (Some(*base), Some(*external))),
    );
    let mut visited = BTreeSet::new();
    while let Some((base_parent, external_parent)) = pending.pop() {
        if !visited.insert((base_parent, external_parent)) {
            continue;
        }
        let base_sequence = base_parent
            .map(|parent| base_tree.nodes[parent].children.as_slice())
            .unwrap_or(&base_tree.roots);
        let external_sequence = external_parent
            .map(|parent| external_tree.nodes[parent].children.as_slice())
            .unwrap_or(&external_tree.roots);
        instrumentation.ordered_alignment_visits = instrumentation
            .ordered_alignment_visits
            .saturating_add(base_sequence.len())
            .saturating_add(external_sequence.len());

        let external_positions = external_sequence
            .iter()
            .enumerate()
            .map(|(position, index)| (*index, position))
            .collect::<BTreeMap<_, _>>();
        let mut boundaries = Vec::new();
        let mut last_external = None;
        for (base_position, base_index) in base_sequence.iter().enumerate() {
            let Some((external_path, external_index)) =
                matched_base.get(&(base_path.clone(), *base_index))
            else {
                continue;
            };
            if external_path != &external_tree.path {
                continue;
            }
            let Some(external_position) = external_positions.get(external_index).copied() else {
                continue;
            };
            if last_external.is_some_and(|last| external_position <= last) {
                boundaries.clear();
                break;
            }
            boundaries.push((base_position, external_position));
            last_external = Some(external_position);
        }
        let trusted_anchor_count = base_sequence
            .iter()
            .filter(|base_index| matched_base.contains_key(&(base_path.clone(), **base_index)))
            .count();
        if trusted_anchor_count > 0 && boundaries.len() != trusted_anchor_count {
            continue;
        }

        let mut previous_base = 0;
        let mut previous_external = 0;
        for (next_base, next_external) in boundaries.into_iter().chain(std::iter::once((
            base_sequence.len(),
            external_sequence.len(),
        ))) {
            let base_gap = base_sequence[previous_base..next_base]
                .iter()
                .copied()
                .filter(|index| {
                    annotations.get(index).is_some_and(|block_id| {
                        !used_blocks.contains(block_id)
                            && !matched_base.contains_key(&(base_path.clone(), *index))
                    })
                })
                .collect::<Vec<_>>();
            let external_gap = external_sequence[previous_external..next_external]
                .iter()
                .copied()
                .filter(|index| {
                    let key = (external_tree.path.clone(), *index);
                    !rejected.contains(&key) && !matched_external.contains(&key)
                })
                .collect::<Vec<_>>();
            if let ([base_index], [external_index]) = (base_gap.as_slice(), external_gap.as_slice())
            {
                if base_class_counts.get(&base_classes[*base_index]) != Some(&1)
                    || external_class_counts.get(&external_classes[*external_index]) != Some(&1)
                {
                    if next_base < base_sequence.len() && next_external < external_sequence.len() {
                        previous_base = next_base.saturating_add(1);
                        previous_external = next_external.saturating_add(1);
                    }
                    continue;
                }
                let block_id = annotations[base_index];
                if used_blocks.insert(block_id) {
                    record_block_match(
                        matches,
                        matched_external,
                        matched_base,
                        base_path,
                        *base_index,
                        external_tree,
                        *external_index,
                        block_id,
                        BlockMatchBasis::ReceiptOrderedTreeAlignment,
                        instrumentation,
                    )?;
                    pending.push((Some(*base_index), Some(*external_index)));
                }
            }
            if next_base < base_sequence.len() && next_external < external_sequence.len() {
                previous_base = next_base.saturating_add(1);
                previous_external = next_external.saturating_add(1);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictCopyClass {
    GeneratedExact,
    External,
    MixedUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictClassificationError {
    path: ManagedPath,
}

impl fmt::Display for ConflictClassificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not a recognized sync conflict copy", self.path)
    }
}

impl std::error::Error for ConflictClassificationError {}

/// Diagnostic classification from caller-supplied exact hashes.
///
/// The function is read-only and never removes the inventory entry or file.
/// Its result is not sealed generated-output evidence and must never authorize
/// deletion; a later deletion path must obtain its own authoritative proof.
pub fn classify_conflict_copy(
    path: ManagedPath,
    observed: &ExactBytes,
    generated_target: BlobDescription,
    exact_external: Option<BlobDescription>,
) -> Result<ConflictCopyClass, ConflictClassificationError> {
    if !path_is_sync_conflict(Path::new(path.as_str())) {
        return Err(ConflictClassificationError { path });
    }
    Ok(if observed.description() == generated_target {
        ConflictCopyClass::GeneratedExact
    } else if exact_external.is_some_and(|external| external == observed.description()) {
        ConflictCopyClass::External
    } else {
        ConflictCopyClass::MixedUnknown
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use rusqlite::Connection;
    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        execute_manifested_projection_work, write_projection_exact, ApplicationRuntimeRoot,
        AuthorBatch, BatchDisposition, BatchId, BlockLocation, CrdtPeerId, DeviceId, DocumentId,
        LineageDigest, ManagedTextKind, ObjectStore, OperationTransaction, PortablePathIndexRoot,
        ProjectionClaim, ProjectionEndpointBinding, ProjectionEndpointId, ProjectionRecovery,
        RebuildSource, SemanticEffect, SemanticOperation, SessionId, SqliteFrontier,
        MAX_MATERIALIZATION_QUERY_ROWS,
    };

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-import-snapshot-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(path.join("graph/pages")).unwrap();
            fs::create_dir_all(path.join("graph/journals")).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct SnapshotFixture {
        _root: TestRoot,
        graph_root: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        engine: ShardedHotEngine,
        intents: Vec<ProjectionIntent>,
        empty_history_head: Vec<u8>,
    }

    impl SnapshotFixture {
        fn new(label: &str, paths: &[&str]) -> Self {
            Self::new_with_initial_uuid_and_config(label, paths, None, None, None, None, None)
        }

        fn new_with_initial_uuid(
            label: &str,
            paths: &[&str],
            initial_uuid: Option<LogseqUuid>,
        ) -> Self {
            Self::new_with_initial_uuid_and_config(
                label,
                paths,
                initial_uuid,
                None,
                None,
                None,
                None,
            )
        }

        fn new_with_graph_config(label: &str, paths: &[&str], config: &str) -> Self {
            Self::new_with_initial_uuid_and_config(
                label,
                paths,
                None,
                Some(config),
                None,
                None,
                None,
            )
        }

        fn new_with_graph_config_names_and_contents(
            label: &str,
            paths: &[&str],
            config: &str,
            names: &[&str],
            contents: &[&str],
        ) -> Self {
            Self::new_with_initial_uuid_and_config(
                label,
                paths,
                None,
                Some(config),
                Some(names),
                Some(contents),
                None,
            )
        }

        fn new_with_graph_config_names_contents_and_preambles(
            label: &str,
            paths: &[&str],
            config: &str,
            names: &[&str],
            contents: &[&str],
            preambles: &[&str],
        ) -> Self {
            Self::new_with_initial_uuid_and_config(
                label,
                paths,
                None,
                Some(config),
                Some(names),
                Some(contents),
                Some(preambles),
            )
        }

        fn new_with_initial_uuid_and_config(
            label: &str,
            paths: &[&str],
            initial_uuid: Option<LogseqUuid>,
            config: Option<&str>,
            names: Option<&[&str]>,
            contents: Option<&[&str]>,
            preambles: Option<&[&str]>,
        ) -> Self {
            assert!(names.is_none_or(|names| names.len() == paths.len()));
            assert!(contents.is_none_or(|contents| contents.len() == paths.len()));
            assert!(preambles.is_none_or(|preambles| preambles.len() == paths.len()));
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            if let Some(config) = config {
                fs::create_dir_all(graph_root.join("logseq")).unwrap();
                fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
            }
            let graph = Graph::open(&graph_root);
            for path in paths {
                let parent = graph_root.join(path).parent().unwrap().to_path_buf();
                fs::create_dir_all(parent).unwrap();
            }
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(Uuid::from_u128(2)),
                DeviceId::from_uuid(Uuid::from_u128(3)),
            )
            .unwrap();
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &root.path().join("receipts"),
                workspace,
                endpoint,
            )
            .unwrap();
            let lineage = LineageDigest::of(b"snapshot-test");
            let catalog = DocumentId::from_uuid(Uuid::from_u128(4));
            let archive = root.path().join("archive");
            let author = ShardedHotEngine::with_enrolled_projection(
                ObjectStore::open(&archive, workspace).unwrap(),
                lineage,
                catalog,
                &graph,
                &receipts,
            );
            let mut operations = Vec::new();
            let mut page_ids = Vec::new();
            for (index, path) in paths.iter().enumerate() {
                let seed = 100 + index as u128 * 10;
                let page_id = PageId::from_uuid(Uuid::from_u128(seed));
                let home = DocumentId::from_uuid(Uuid::from_u128(seed + 1));
                let managed_path = ManagedPath::parse((*path).to_owned()).unwrap();
                let kind = graph.classify_managed_text_path(&managed_path).unwrap();
                page_ids.push(page_id);
                operations.push(SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    name: crate::oplog::LogicalPageName::parse(
                        names
                            .map(|names| names[index].to_owned())
                            .unwrap_or_else(|| format!("Snapshot Page {index}")),
                    )
                    .unwrap(),
                    path: managed_path,
                    kind,
                });
                operations.push(SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                        home_document_id: home,
                    },
                    page_id,
                    parent: None,
                    order: "a".into(),
                    content: contents
                        .map(|contents| contents[index].to_owned())
                        .unwrap_or_else(|| match (index, initial_uuid) {
                            (0, Some(logseq_uuid)) => {
                                format!("page {index}\nid:: {logseq_uuid}")
                            }
                            _ => format!("page {index}"),
                        }),
                });
                if let Some(preambles) = preambles {
                    operations.push(SemanticOperation::SetPagePreamble {
                        page_id,
                        preamble: Some(preambles[index].to_owned()),
                    });
                }
                if index == 0 {
                    if let Some(logseq_uuid) = initial_uuid {
                        operations.push(SemanticOperation::MutateBlockLogseqIdentity {
                            block: BlockLocation {
                                block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                                home_document_id: home,
                            },
                            mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid },
                        });
                    }
                }
            }
            let transaction = OperationTransaction::new(operations).unwrap();
            let batch_id = BatchId::from_uuid(Uuid::from_u128(5));
            let draft = author
                .draft_author_transaction(
                    AuthorBatch {
                        batch_id,
                        author_device_id: endpoint.device_id,
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(7)),
                        crdt_peer_id: CrdtPeerId::from_u64(8),
                    },
                    BatchOrigin::LocalMutation,
                    &transaction,
                )
                .unwrap();
            let prepared = author
                .finalize_author_transaction(draft, &graph, &receipts, endpoint)
                .unwrap();
            drop(author);
            ObjectStore::open(&archive, workspace)
                .unwrap()
                .publish_prepared(&prepared)
                .unwrap();
            let mut engine = ShardedHotEngine::with_enrolled_projection(
                ObjectStore::open(&archive, workspace).unwrap(),
                lineage,
                catalog,
                &graph,
                &receipts,
            );
            let empty_history_head = fs::read(
                archive
                    .join("engine-history")
                    .join(endpoint.endpoint_id.to_string())
                    .join("engine-history.head"),
            )
            .unwrap();
            engine.stage_archive_batch(batch_id).unwrap();
            let intents = page_ids
                .into_iter()
                .map(|page_id| {
                    write_projection_exact(&graph, &receipts, &engine, page_id, None)
                        .unwrap()
                        .plan
                        .intent()
                        .clone()
                })
                .collect();
            Self {
                _root: root,
                graph_root,
                graph,
                receipts,
                engine,
                intents,
                empty_history_head,
            }
        }

        fn plan(&self, paths: &[&str]) -> ImportPlan {
            plan_affected_import(&self.graph, &self.receipts, &self.engine, paths)
        }

        fn reopen_after_config_change(self) -> Self {
            let Self {
                _root,
                graph_root,
                graph,
                receipts,
                engine,
                intents,
                empty_history_head,
            } = self;
            drop(graph);
            drop(engine);
            let graph = Graph::open(&graph_root);
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
            let mut engine = ShardedHotEngine::with_enrolled_projection(
                ObjectStore::open(&_root.path().join("archive"), workspace).unwrap(),
                LineageDigest::of(b"snapshot-test"),
                DocumentId::from_uuid(Uuid::from_u128(4)),
                &graph,
                &receipts,
            );
            engine.prepare_operational_recovery_replay().unwrap();
            let manifests = ObjectStore::open(&_root.path().join("archive"), workspace)
                .unwrap()
                .committed_manifests()
                .unwrap();
            for manifest in manifests {
                engine
                    .stage_archive_batch_for_recovery(manifest.batch_id())
                    .unwrap();
            }
            engine.finish_operational_recovery_replay().unwrap();
            Self {
                _root,
                graph_root,
                graph,
                receipts,
                engine,
                intents,
                empty_history_head,
            }
        }

        fn apply_external_plan(&mut self, plan: ImportPlan, seed: u128) -> BatchId {
            let endpoint = self.engine.projection_endpoint_binding().unwrap();
            let material = plan.into_execution_material().unwrap();
            let batch_id = material.batch_id();
            let draft = self
                .engine
                .draft_external_import_transaction(
                    AuthorBatch {
                        batch_id,
                        author_device_id: endpoint.device_id,
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(seed)),
                        crdt_peer_id: CrdtPeerId::from_u64(seed as u64 + 1),
                    },
                    material,
                )
                .unwrap();
            let captured = self
                .engine
                .capture_external_author_transaction(
                    draft,
                    &self.graph,
                    &self.receipts,
                    endpoint,
                    None,
                )
                .unwrap();
            let prepared = self
                .engine
                .finalize_captured_author_transaction(captured, &self.receipts)
                .unwrap();
            ObjectStore::open(
                &self._root.path().join("archive"),
                self.engine.workspace_id(),
            )
            .unwrap()
            .publish_prepared(&prepared)
            .unwrap();
            let disposition = self
                .engine
                .stage_archive_batch(batch_id)
                .unwrap()
                .disposition()
                .clone();
            assert!(
                matches!(disposition, BatchDisposition::Accepted { .. }),
                "{disposition:?}"
            );
            batch_id
        }
    }

    fn completion_name(intent: &ProjectionIntent) -> String {
        let mut value = String::new();
        for byte in intent.id().unwrap().as_bytes() {
            use std::fmt::Write as _;
            write!(&mut value, "{byte:02x}").unwrap();
        }
        format!("{value}.completion")
    }

    #[test]
    fn snapshot_revalidation_rejects_content_replacement_between_passes() {
        let fixture = SnapshotFixture::new("content", &["pages/a.md"]);
        let target = fixture.graph_root.join("pages/a.md");
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(target, b"- replaced\n").unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn source_admission_blocks_non_round_tripping_org_before_material() {
        for (label, path, source) in [(
            "skipped-org-admission",
            "pages/a.org",
            "* changed\n*** skipped\n",
        )] {
            let fixture = SnapshotFixture::new(label, &[path]);
            let target = fixture.graph_root.join(path);
            fs::write(&target, source).unwrap();
            let plan = fixture.plan(&[path]);
            assert_eq!(
                plan.status(),
                ImportPlanStatus::Blocked,
                "{label}: {plan:?}"
            );
            assert_eq!(plan.blocks()[0].reason, ImportBlockReason::UnsafeInput);
            assert!(
                plan.execution_material().is_err(),
                "{label} exposed semantic execution material"
            );
            assert_eq!(fs::read(target).unwrap(), source.as_bytes());
        }
    }

    #[test]
    fn source_admission_accepts_structurally_round_tripping_markdown_before_material() {
        let source = "- changed\n\t- child\n  - grandchild\n";
        let fixture = SnapshotFixture::new("mixed-markdown-admission", &["pages/a.md"]);
        let target = fixture.graph_root.join("pages/a.md");
        fs::write(&target, source).unwrap();

        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile, "{plan:?}");
        assert!(
            plan.execution_material().is_ok(),
            "structurally stable Markdown must expose execution material"
        );
        assert_eq!(fs::read(target).unwrap(), source.as_bytes());
    }

    #[test]
    fn source_admission_refuses_overlapping_lsdoc_events_without_touching_bytes() {
        let source = "- $$x$$ # #+BEGIN_NOTE\r\nx\r\n#+END_NOTE";
        let fixture = SnapshotFixture::new("overlapping-outline-admission", &["pages/overlap.md"]);
        let target = fixture.graph_root.join("pages/overlap.md");
        fs::write(&target, source).unwrap();

        let plan = fixture.plan(&["pages/overlap.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            plan.blocks()
                .iter()
                .map(|block| block.reason)
                .collect::<Vec<_>>(),
            vec![ImportBlockReason::UnsafeInput]
        );
        assert!(
            plan.blocks()[0]
                .detail
                .contains("external document parser rejected source"),
            "{:?}",
            plan.blocks()[0]
        );
        assert_eq!(fs::read(target).unwrap(), source.as_bytes());
    }

    #[test]
    fn parser_owned_markdown_and_org_admission_preserves_exact_source_bytes() {
        for (label, path, source) in [
            (
                "parser-owned-markdown-source",
                "pages/parser-owned.md",
                "title:: café\r\n\r\n# Project Ω\r\n\t- child\r\n- sibling\r\n",
            ),
            (
                "parser-owned-org-source",
                "pages/parser-owned.org",
                "#+TITLE: café\r\n\r\n* Project Ω\r\n** child\r\n* sibling\r\n",
            ),
        ] {
            let fixture = SnapshotFixture::new(label, &[path]);
            let target = fixture.graph_root.join(path);
            fs::write(&target, source).unwrap();

            let plan = fixture.plan(&[path]);
            assert_eq!(plan.status(), ImportPlanStatus::Reconcile, "{plan:?}");
            assert!(plan.execution_material().is_ok(), "{plan:?}");
            assert_eq!(fs::read(target).unwrap(), source.as_bytes());
        }
    }

    #[test]
    fn snapshot_revalidation_rejects_two_path_rename_between_passes() {
        let fixture = SnapshotFixture::new("rename", &["pages/a.md", "pages/b.md"]);
        let a = fixture.graph_root.join("pages/a.md");
        let b = fixture.graph_root.join("pages/b.md");
        let temporary = fixture.graph_root.join("pages/swap.tmp");
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&a, &temporary).unwrap();
                fs::rename(&b, &a).unwrap();
                fs::rename(&temporary, &b).unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/a.md", "pages/b.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn snapshot_revalidation_rejects_catalog_change_between_passes() {
        let fixture = SnapshotFixture::new("catalog", &["pages/a.md"]);
        let completion = fixture
            .receipts
            .root_path()
            .join("completions")
            .join(completion_name(&fixture.intents[0]));
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(completion).unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn snapshot_revalidation_rejects_accepted_frontier_change_between_passes() {
        let fixture = SnapshotFixture::new("frontier", &["pages/a.md"]);
        let other = SnapshotFixture::new("frontier-other", &["pages/a.md", "pages/b.md"]);
        POST_FRONTIER_OVERRIDE.with(|root| {
            *root.borrow_mut() = Some(other.engine.accepted_frontier_root().unwrap());
        });
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn execution_material_refuses_noop_and_blocked_plans() {
        let fixture = SnapshotFixture::new("execution-refusal", &["pages/a.md"]);
        let noop = fixture.plan(&["pages/a.md"]);
        assert_eq!(noop.status(), ImportPlanStatus::Noop);
        assert_eq!(
            noop.execution_material().unwrap_err(),
            ImportExecutionError::RefusedStatus(ImportPlanStatus::Noop)
        );

        fs::write(fixture.graph_root.join("pages/a.md"), [0xff]).unwrap();
        let blocked = fixture.plan(&["pages/a.md"]);
        assert_eq!(blocked.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            blocked.execution_material().unwrap_err(),
            ImportExecutionError::RefusedStatus(ImportPlanStatus::Blocked)
        );

        fs::write(
            fixture
                .graph_root
                .join("pages/a.sync-conflict-20260725-120000-AAAAAAA.md"),
            b"- diagnostic only\n",
        )
        .unwrap();
        let conflict = fixture.plan(&["pages/a.sync-conflict-20260725-120000-AAAAAAA.md"]);
        assert_eq!(conflict.status(), ImportPlanStatus::Blocked);
        assert!(conflict
            .blocks()
            .iter()
            .any(|block| block.detail.contains("diagnostic inputs")));
    }

    #[test]
    fn identical_sealed_reconciliations_produce_identical_execution_and_observation_bytes() {
        let left = SnapshotFixture::new("execution-identical-left", &["pages/a.md"]);
        let right = SnapshotFixture::new("execution-identical-right", &["pages/a.md"]);
        fs::write(left.graph_root.join("pages/a.md"), b"- changed\n").unwrap();
        fs::write(right.graph_root.join("pages/a.md"), b"- changed\n").unwrap();

        let left_plan = left.plan(&["pages/a.md"]);
        let right_plan = right.plan(&["pages/a.md"]);
        assert_eq!(left_plan.status(), ImportPlanStatus::Reconcile);
        assert_eq!(right_plan.status(), ImportPlanStatus::Reconcile);
        assert_eq!(left_plan.import_id(), right_plan.import_id());
        let left_material = left_plan.execution_material().unwrap();
        let right_material = right_plan.execution_material().unwrap();
        assert_eq!(left_material, right_material);
        assert_eq!(
            left_material.batch_id(),
            left_material.import_id().batch_id()
        );
        assert_eq!(
            left_material.origin(),
            BatchOrigin::ExternalReconciliation {
                import_id: left_material.import_id()
            }
        );

        let left_object = left_material
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        let right_object = right_material
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        assert_eq!(left_object, right_object);
        assert_eq!(
            left_object.descriptor().unwrap(),
            right_object.descriptor().unwrap()
        );
        let mut malformed = left_object.payload().to_vec();
        malformed[0] ^= 0xff;
        assert!(
            super::super::external_import::ExternalImportObservation::decode(&malformed).is_err()
        );
    }

    #[test]
    fn execution_material_preserves_explicit_external_id_change_and_removal() {
        let old = LogseqUuid::from_uuid(Uuid::from_u128(910));
        let changed = LogseqUuid::from_uuid(Uuid::from_u128(911));
        let replacement = SnapshotFixture::new_with_initial_uuid(
            "execution-id-replacement",
            &["pages/a.md"],
            Some(old),
        );
        fs::write(
            replacement.graph_root.join("pages/a.md"),
            format!("- page 0\n  id:: {changed}\n"),
        )
        .unwrap();
        let replacement_plan = replacement.plan(&["pages/a.md"]);
        let replacement_transaction = &replacement_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(replacement_transaction.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::ReplaceExternal { logseq_uuid },
                    ..
                } if *logseq_uuid == changed
            )
        }));

        let removal = SnapshotFixture::new_with_initial_uuid(
            "execution-id-removal",
            &["pages/a.md"],
            Some(old),
        );
        fs::write(removal.graph_root.join("pages/a.md"), b"- page 0\n").unwrap();
        let removal_plan = removal.plan(&["pages/a.md"]);
        let removal_transaction = &removal_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(removal_transaction.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::RemoveExternal,
                    ..
                }
            )
        }));
    }

    #[test]
    fn execution_material_retains_invalid_and_duplicate_raw_ids_without_identity_authority() {
        let duplicate = SnapshotFixture::new("execution-duplicate-id", &["pages/a.md"]);
        let duplicate_bytes = format!(
            "- page 0\n  id:: {}\n  id:: {}\n",
            LogseqUuid::from_uuid(Uuid::from_u128(920)),
            LogseqUuid::from_uuid(Uuid::from_u128(920)),
        )
        .into_bytes();
        fs::write(duplicate.graph_root.join("pages/a.md"), &duplicate_bytes).unwrap();
        let duplicate_plan = duplicate.plan(&["pages/a.md"]);
        let duplicate_object = duplicate_plan
            .execution_material()
            .unwrap()
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        let duplicate_observation =
            super::super::external_import::ExternalImportObservation::decode(
                duplicate_object.payload(),
            )
            .unwrap();
        let duplicate_entry = &duplicate_observation.entries()[0];
        assert_eq!(
            duplicate_entry.state().bytes(),
            Some(duplicate_bytes.as_slice())
        );
        assert!(duplicate_entry
            .state()
            .annotations()
            .iter()
            .all(|annotation| annotation.logseq_uuid().is_none()));

        let invalid = SnapshotFixture::new("execution-invalid-id", &["pages/a.md"]);
        let invalid_bytes = b"- page 0\n  id:: definitely-not-a-uuid\n";
        fs::write(invalid.graph_root.join("pages/a.md"), invalid_bytes).unwrap();
        let invalid_plan = invalid.plan(&["pages/a.md"]);
        let invalid_object = invalid_plan
            .execution_material()
            .unwrap()
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        let invalid_observation = super::super::external_import::ExternalImportObservation::decode(
            invalid_object.payload(),
        )
        .unwrap();
        let invalid_entry = &invalid_observation.entries()[0];
        assert_eq!(
            invalid_entry.state().bytes(),
            Some(invalid_bytes.as_slice())
        );
        assert!(invalid_entry
            .state()
            .annotations()
            .iter()
            .all(|annotation| annotation.logseq_uuid().is_none()));
    }

    #[test]
    fn execution_material_retains_nested_rename_and_delete_semantics() {
        let renamed = SnapshotFixture::new("execution-nested-rename", &["pages/topic/old-name.md"]);
        fs::create_dir_all(renamed.graph_root.join("pages/topic/next")).unwrap();
        fs::rename(
            renamed.graph_root.join("pages/topic/old-name.md"),
            renamed.graph_root.join("pages/topic/next/new-name.md"),
        )
        .unwrap();
        let rename_plan =
            renamed.plan(&["pages/topic/old-name.md", "pages/topic/next/new-name.md"]);
        let rename_transaction = &rename_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(rename_transaction.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState { path, .. }
                    if path.as_str() == "pages/topic/next/new-name.md"
            )
        }));

        let deleted =
            SnapshotFixture::new("execution-nested-delete", &["pages/topic/delete-me.md"]);
        fs::remove_file(deleted.graph_root.join("pages/topic/delete-me.md")).unwrap();
        let delete_plan = deleted.plan(&["pages/topic/delete-me.md"]);
        let delete_transaction = &delete_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(delete_transaction
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::DeleteSubtree { .. })));
        assert!(delete_transaction
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::DeletePage { .. })));
    }

    #[test]
    fn execution_material_uses_graph_filename_decoding_for_affected_new_paths() {
        let legacy = SnapshotFixture::new_with_graph_config(
            "execution-path-names-legacy",
            &["pages/seed.md"],
            "{:journal/file-name-format \"dd-MM-yyyy\" :journal/page-title-format \"yyyy-MM-dd\"}\n",
        );
        for path in [
            "pages/first/second/Project%2FPlan.md",
            "pages/left/shared.md",
            "pages/right/shared.md",
            "journals/archive/deep/25-07-2026.md",
        ] {
            let target = legacy.graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"- external\n").unwrap();
        }
        let legacy_plan = legacy.plan(&[
            "pages/first/second/Project%2FPlan.md",
            "journals/archive/deep/25-07-2026.md",
        ]);
        assert_eq!(legacy_plan.status(), ImportPlanStatus::Reconcile);
        let legacy_creates = legacy_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::CreatePage {
                    name, path, kind, ..
                } => Some((path.as_str(), (name.as_str(), *kind))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            legacy_creates["pages/first/second/Project%2FPlan.md"],
            ("Project/Plan", ManagedTextKind::Page),
            "nested directories select the managed root but never become a page namespace"
        );
        assert_eq!(
            legacy_creates["journals/archive/deep/25-07-2026.md"],
            ("2026-07-25", ManagedTextKind::Journal),
            "nested journals use the configured JournalFormat title"
        );

        let duplicate_names = legacy.plan(&["pages/left/shared.md", "pages/right/shared.md"]);
        assert_eq!(duplicate_names.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            duplicate_names.blocks()[0].reason,
            ImportBlockReason::ConflictingLocalTail,
            "same basenames in distinct paths are a visible ambiguity, never a successful import"
        );

        let triple_lowbar = SnapshotFixture::new_with_graph_config(
            "execution-path-names-triple-lowbar",
            &["pages/seed.md"],
            "{:file/name-format :triple-lowbar}\n",
        );
        for path in [
            "pages/deep/Team___Planning.md",
            "pages/deep/literal%5F%5F%5Fmarker.md",
        ] {
            let target = triple_lowbar.graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"- external\n").unwrap();
        }
        let triple_plan = triple_lowbar.plan(&[
            "pages/deep/Team___Planning.md",
            "pages/deep/literal%5F%5F%5Fmarker.md",
        ]);
        assert_eq!(triple_plan.status(), ImportPlanStatus::Reconcile);
        let triple_creates = triple_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::CreatePage {
                    name, path, kind, ..
                } => Some((path.as_str(), (name.as_str(), *kind))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            triple_creates["pages/deep/Team___Planning.md"],
            ("Team/Planning", ManagedTextKind::Page)
        );
        assert_eq!(
            triple_creates["pages/deep/literal%5F%5F%5Fmarker.md"],
            ("literal___marker", ManagedTextKind::Page),
            "TripleLowbar decodes separators before percent escapes, preserving encoded literals"
        );
    }

    #[test]
    fn accepted_page_name_survives_filename_policy_reopen_while_new_pages_use_new_policy() {
        let fixture = SnapshotFixture::new_with_graph_config_names_and_contents(
            "accepted-page-name-policy-reopen",
            &["pages/A.B.md", "pages/referrer.md"],
            "{:file/name-format :legacy}\n",
            &["A/B", "Referrer"],
            &["old", "see [[A/B]]"],
        );
        let accepted_page_id = fixture.intents[0].page_id();
        let referrer_page_id = fixture.intents[1].page_id();
        let accepted_referrer = fixture.engine.materialize_page(referrer_page_id).unwrap();
        let referrer_block_id = accepted_referrer.blocks[0].block_id;

        fs::write(
            fixture.graph_root.join("logseq/config.edn"),
            "{:file/name-format :triple-lowbar}\n",
        )
        .unwrap();
        let mut fixture = fixture.reopen_after_config_change();
        fs::write(fixture.graph_root.join("pages/A.B.md"), b"- changed\n").unwrap();
        fs::write(fixture.graph_root.join("pages/New___Page.md"), b"- new\n").unwrap();

        let plan = plan_affected_import(
            &fixture.graph,
            &fixture.receipts,
            &fixture.engine,
            &["pages/A.B.md", "pages/New___Page.md"],
        );
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let operations = &plan.execution_material().unwrap().transaction().operations;
        assert!(
            !operations.iter().any(|operation| matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState { page_id, .. }
                    if *page_id == accepted_page_id
            )),
            "an edit at an exactly owned path must not reinterpret its accepted logical name"
        );
        assert!(operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::CreatePage { name, path, kind, .. }
                if name.as_str() == "New/Page"
                    && path.as_str() == "pages/New___Page.md"
                    && *kind == ManagedTextKind::Page
        )));
        assert!(
            !operations.iter().any(|operation| matches!(
                operation,
                SemanticOperation::EditBlockContent { block, .. }
                    if block.block_id == referrer_block_id
            )),
            "preserving the accepted page name must leave existing textual referrers untouched"
        );
        assert_eq!(
            fixture
                .engine
                .materialize_page(accepted_page_id)
                .unwrap()
                .name
                .as_str(),
            "A/B"
        );
        assert_eq!(accepted_referrer.blocks[0].content, "see [[A/B]]");

        let new_page_id = operations
            .iter()
            .find_map(|operation| match operation {
                SemanticOperation::CreatePage { page_id, path, .. }
                    if path.as_str() == "pages/New___Page.md" =>
                {
                    Some(*page_id)
                }
                _ => None,
            })
            .unwrap();
        fixture.apply_external_plan(plan, 7_500);
        let fixture = fixture.reopen_after_config_change();
        let accepted = fixture.engine.materialize_page(accepted_page_id).unwrap();
        assert_eq!(accepted.page_id, accepted_page_id);
        assert_eq!(accepted.name.as_str(), "A/B");
        assert_eq!(accepted.path.as_str(), "pages/A.B.md");
        assert_eq!(accepted.kind, ManagedTextKind::Page);
        let referrer = fixture.engine.materialize_page(referrer_page_id).unwrap();
        assert_eq!(referrer.page_id, referrer_page_id);
        assert_eq!(referrer.name.as_str(), "Referrer");
        assert_eq!(referrer.kind, ManagedTextKind::Page);
        assert_eq!(referrer.blocks[0].block_id, referrer_block_id);
        assert_eq!(referrer.blocks[0].content, "see [[A/B]]");
        let created = fixture.engine.materialize_page(new_page_id).unwrap();
        assert_eq!(created.page_id, new_page_id);
        assert_eq!(created.name.as_str(), "New/Page");
        assert_eq!(created.path.as_str(), "pages/New___Page.md");
        assert_eq!(created.kind, ManagedTextKind::Page);
    }

    #[test]
    fn accepted_journal_name_survives_journal_policy_reopen_while_new_journals_use_new_policy() {
        let fixture = SnapshotFixture::new_with_graph_config_names_and_contents(
            "accepted-journal-name-policy-reopen",
            &["journals/25.07.2026.md", "pages/referrer.md"],
            "{:journal/file-name-format \"dd.MM.yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
            &["2026-07-25", "Referrer"],
            &["old journal", "see [[2026-07-25]]"],
        );
        let accepted_page_id = fixture.intents[0].page_id();
        let referrer_page_id = fixture.intents[1].page_id();
        let accepted_referrer = fixture.engine.materialize_page(referrer_page_id).unwrap();
        let referrer_block_id = accepted_referrer.blocks[0].block_id;

        fs::write(
            fixture.graph_root.join("logseq/config.edn"),
            "{:journal/file-name-format \"MM~dd~yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
        )
        .unwrap();
        let mut fixture = fixture.reopen_after_config_change();
        fs::write(
            fixture.graph_root.join("journals/25.07.2026.md"),
            b"- changed journal\n",
        )
        .unwrap();
        fs::write(
            fixture.graph_root.join("journals/07~26~2026.md"),
            b"- new journal\n",
        )
        .unwrap();

        let plan = plan_affected_import(
            &fixture.graph,
            &fixture.receipts,
            &fixture.engine,
            &["journals/25.07.2026.md", "journals/07~26~2026.md"],
        );
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let operations = &plan.execution_material().unwrap().transaction().operations;
        assert!(
            !operations.iter().any(|operation| matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState { page_id, .. }
                    if *page_id == accepted_page_id
            )),
            "an edit at an exactly owned journal path must not reinterpret its accepted name"
        );
        assert!(operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::CreatePage { name, path, kind, .. }
                if name.as_str() == "2026-07-26"
                    && path.as_str() == "journals/07~26~2026.md"
                    && *kind == ManagedTextKind::Journal
        )));
        assert!(
            !operations.iter().any(|operation| matches!(
                operation,
                SemanticOperation::EditBlockContent { block, .. }
                    if block.block_id == referrer_block_id
            )),
            "preserving the accepted journal name must leave existing textual referrers untouched"
        );
        assert_eq!(
            fixture
                .engine
                .materialize_page(accepted_page_id)
                .unwrap()
                .name
                .as_str(),
            "2026-07-25"
        );
        assert_eq!(accepted_referrer.blocks[0].content, "see [[2026-07-25]]");

        let new_journal_id = operations
            .iter()
            .find_map(|operation| match operation {
                SemanticOperation::CreatePage { page_id, path, .. }
                    if path.as_str() == "journals/07~26~2026.md" =>
                {
                    Some(*page_id)
                }
                _ => None,
            })
            .unwrap();
        fixture.apply_external_plan(plan, 7_600);
        let fixture = fixture.reopen_after_config_change();
        let accepted = fixture.engine.materialize_page(accepted_page_id).unwrap();
        assert_eq!(accepted.page_id, accepted_page_id);
        assert_eq!(accepted.name.as_str(), "2026-07-25");
        assert_eq!(accepted.path.as_str(), "journals/25.07.2026.md");
        assert_eq!(accepted.kind, ManagedTextKind::Journal);
        let referrer = fixture.engine.materialize_page(referrer_page_id).unwrap();
        assert_eq!(referrer.page_id, referrer_page_id);
        assert_eq!(referrer.name.as_str(), "Referrer");
        assert_eq!(referrer.kind, ManagedTextKind::Page);
        assert_eq!(referrer.blocks[0].block_id, referrer_block_id);
        assert_eq!(referrer.blocks[0].content, "see [[2026-07-25]]");
        let created = fixture.engine.materialize_page(new_journal_id).unwrap();
        assert_eq!(created.page_id, new_journal_id);
        assert_eq!(created.name.as_str(), "2026-07-26");
        assert_eq!(created.path.as_str(), "journals/07~26~2026.md");
        assert_eq!(created.kind, ManagedTextKind::Journal);
    }

    #[test]
    fn unchanged_explicit_date_title_preserves_accepted_identity_across_journal_format_change() {
        let fixture = SnapshotFixture::new_with_graph_config_names_contents_and_preambles(
            "accepted-explicit-journal-title-policy-reopen",
            &["journals/physical.md"],
            "{:journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
            &["2026-07-25"],
            &["old journal"],
            &["title:: 25-07-2026"],
        );
        let page_id = fixture.intents[0].page_id();
        fs::write(
            fixture.graph_root.join("logseq/config.edn"),
            "{:journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        let fixture = fixture.reopen_after_config_change();
        fs::write(
            fixture.graph_root.join("journals/physical.md"),
            b"title:: 25-07-2026\n\n- changed journal\n",
        )
        .unwrap();

        let plan = fixture.plan(&["journals/physical.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile, "{plan:?}");
        let operations = &plan.execution_material().unwrap().transaction().operations;
        assert!(
            !operations.iter().any(|operation| matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState {
                    page_id: candidate,
                    ..
                } if *candidate == page_id
            )),
            "unchanged explicit title evidence must preserve the accepted name/kind despite a new journal renderer: {operations:#?}"
        );
        assert!(operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::EditBlockContent { content, .. }
                if content == "changed journal"
        )));
    }

    #[test]
    fn semantically_wrong_authenticated_current_path_identity_blocks_before_external_draft() {
        let mut fixture = SnapshotFixture::new_with_graph_config_names_and_contents(
            "semantically-wrong-current-path-identity",
            &["pages/accepted.md"],
            "{:file/name-format :legacy}\n",
            &["Accepted Name"],
            &["old"],
        );
        let page_id = fixture.intents[0].page_id();
        let path = ManagedPath::parse("pages/accepted.md").unwrap();
        fixture
            .engine
            .replace_current_path_catalog_row_with_name_for_test(
                page_id,
                path,
                ManagedTextKind::Journal,
                LogicalPageName::parse("authenticated but semantically wrong name").unwrap(),
            );
        fixture.engine.reconstruct_run_local_state().unwrap();
        fs::write(
            fixture.graph_root.join("pages/accepted.md"),
            b"- external edit\n",
        )
        .unwrap();

        let plan = fixture.plan(&["pages/accepted.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            plan.blocks()[0].reason,
            ImportBlockReason::AuthorityUnavailable
        );
        assert!(
            plan.blocks()[0]
                .detail
                .contains("accepted catalog page state"),
            "{}",
            plan.blocks()[0].detail
        );
        assert!(matches!(
            plan.into_execution_material(),
            Err(ImportExecutionError::RefusedStatus(
                ImportPlanStatus::Blocked
            ))
        ));
    }

    #[test]
    fn external_title_rename_updates_accepted_owner_after_restart_without_rewriting_referrers() {
        let mut fixture = SnapshotFixture::new_with_graph_config_names_and_contents(
            "external-title-rename-referrers",
            &["pages/physical.md", "pages/referrer.md"],
            "{:file/name-format :legacy}\n",
            &["Old Logical", "Referrer"],
            &["target body", "see [[Old Logical]] and [[New Logical]]"],
        );
        let target_page_id = fixture.intents[0].page_id();
        let referrer_page_id = fixture.intents[1].page_id();
        let referrer_path = fixture.graph_root.join("pages/referrer.md");
        let referrer_bytes = fs::read(&referrer_path).unwrap();
        fs::write(
            fixture.graph_root.join("pages/physical.md"),
            b"title:: New Logical\n\n- target body\n",
        )
        .unwrap();

        let plan = fixture.plan(&["pages/physical.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile, "{plan:?}");
        let operations = &plan.execution_material().unwrap().transaction().operations;
        assert!(operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::ReconcileExternalPageState {
                page_id,
                name,
                ..
            } if *page_id == target_page_id && name.as_str() == "New Logical"
        )));
        assert!(!operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::EditBlockContent { block, .. }
                if fixture
                    .engine
                    .materialize_page(referrer_page_id)
                    .unwrap()
                    .blocks
                    .iter()
                    .any(|candidate| candidate.block_id == block.block_id)
        )));

        fixture.apply_external_plan(plan, 7_700);
        assert_eq!(fs::read(&referrer_path).unwrap(), referrer_bytes);
        let work = fixture
            .engine
            .projection_work_index()
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        execute_manifested_projection_work(
            &fixture.graph,
            &fixture.receipts,
            &mut fixture.engine,
            &work,
        )
        .unwrap();
        let fixture = fixture.reopen_after_config_change();
        assert_eq!(
            fixture
                .engine
                .current_page_for_logical_name(&LogicalPageName::parse("New Logical").unwrap())
                .unwrap(),
            Some(target_page_id)
        );
        assert_eq!(
            fixture
                .engine
                .current_page_for_logical_name(&LogicalPageName::parse("Old Logical").unwrap())
                .unwrap(),
            None
        );
        assert_eq!(
            fixture
                .engine
                .materialize_page(referrer_page_id)
                .unwrap()
                .blocks[0]
                .content,
            "see [[Old Logical]] and [[New Logical]]"
        );
        assert_eq!(
            Graph::open(&fixture.graph_root)
                .list_pages()
                .into_iter()
                .find(|entry| entry.rel_path == "pages/physical.md")
                .unwrap()
                .name,
            "New Logical"
        );
        assert_eq!(
            fixture.plan(&["pages/physical.md"]).status(),
            ImportPlanStatus::Noop,
            "projection receipt and restart must not oscillate the accepted title"
        );
    }

    #[test]
    fn configured_nested_managed_roots_use_graph_kind_and_filename_decoding() {
        let fixture = SnapshotFixture::new_with_graph_config(
            "configured-nested-roots",
            &["content/pages/seed.md"],
            "{:pages-directory \"content/pages\"\n\
              :journals-directory \"content/journals\"\n\
              :journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
        );
        for path in [
            "content/pages/deep/Project%2FPlan.md",
            "content/journals/archive/deep/25-07-2026.md",
        ] {
            let target = fixture.graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"- external\n").unwrap();
        }
        let plan = fixture.plan(&[
            "content/pages/deep/Project%2FPlan.md",
            "content/journals/archive/deep/25-07-2026.md",
        ]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let created = plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::CreatePage {
                    path, name, kind, ..
                } => Some((path.as_str(), (name.as_str(), *kind))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            created["content/pages/deep/Project%2FPlan.md"],
            ("Project/Plan", ManagedTextKind::Page)
        );
        assert_eq!(
            created["content/journals/archive/deep/25-07-2026.md"],
            ("2026-07-25", ManagedTextKind::Journal)
        );
    }

    #[test]
    fn initial_shadow_inventory_enrolls_configured_nested_roots() {
        let root = TestRoot::new("initial-configured-roots");
        let graph_root = root.path().join("graph");
        fs::create_dir_all(graph_root.join("logseq")).unwrap();
        fs::write(
            graph_root.join("logseq/config.edn"),
            "{:pages-directory \"content/pages\"\n\
              :journals-directory \"content/journals\"}\n",
        )
        .unwrap();
        for (path, bytes) in [
            ("content/pages/deep/page.md", b"- page\n".as_slice()),
            (
                "content/journals/archive/journal.org",
                b"* journal\n".as_slice(),
            ),
        ] {
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, bytes).unwrap();
        }
        let inventory = inventory_initial_shadow(&Graph::open(&graph_root)).unwrap();
        assert_eq!(
            inventory
                .entries()
                .keys()
                .map(ManagedPath::as_str)
                .collect::<Vec<_>>(),
            vec![
                "content/journals/archive/journal.org",
                "content/pages/deep/page.md",
            ]
        );
        assert!(inventory.entries().values().all(
            |observation| matches!(observation, RawObservation::Present(bytes) if !bytes.bytes().is_empty())
        ));
    }

    #[test]
    fn exact_rename_adopts_graph_decoded_destination_name_before_authoring() {
        let fixture = SnapshotFixture::new("rename-destination-name", &["pages/old.md"]);
        let destination = fixture.graph_root.join("pages/Project%2FPlan.md");
        fs::rename(fixture.graph_root.join("pages/old.md"), &destination).unwrap();
        let plan = fixture.plan(&["pages/old.md", "pages/Project%2FPlan.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        assert!(plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState { name, path, .. }
                    if name.as_str() == "Project/Plan" && path.as_str() == "pages/Project%2FPlan.md"
            )));
    }

    #[test]
    fn sealed_external_execution_drafts_through_the_engine_in_parent_before_child_order() {
        let fixture = SnapshotFixture::new("external-engine-draft", &["pages/old.md"]);
        let destination = fixture.graph_root.join("pages/Project%2FPlan.md");
        fs::rename(fixture.graph_root.join("pages/old.md"), &destination).unwrap();
        fs::write(
            fixture.graph_root.join("pages/new.md"),
            b"- parent\n\t- child\n",
        )
        .unwrap();

        let plan = fixture.plan(&["pages/old.md", "pages/Project%2FPlan.md", "pages/new.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let material = plan.into_execution_material().unwrap();
        let operations = &material.transaction().operations;
        assert!(operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::ReconcileExternalPageState { name, path, .. }
                if name.as_str() == "Project/Plan" && path.as_str() == "pages/Project%2FPlan.md"
        )));

        let created = operations
            .iter()
            .enumerate()
            .filter_map(|(index, operation)| match operation {
                SemanticOperation::CreateBlock { block, parent, .. } => {
                    Some((index, block.block_id, *parent))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let (child_index, _, parent) = created
            .iter()
            .copied()
            .find(|(_, _, parent)| parent.is_some())
            .expect("new nested child must be created");
        let parent = parent.expect("selected child has a parent");
        let parent_index = created
            .iter()
            .find_map(|(index, block_id, _)| (*block_id == parent).then_some(*index))
            .expect("new child parent must be created in this transaction");
        assert!(parent_index < child_index);

        // The external-import draft adapter applies the sealed operations to the
        // engine's prospective documents, so this catches ordering and
        // semantic preflight regressions that an operation-list inspection
        // would miss. Final external publication deliberately remains behind
        // the capability recapture boundary.
        let draft = fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_412)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_413),
                },
                material,
            )
            .unwrap();
        assert!(!draft.requirements().is_empty());

        // Identity replacement and removal travel through the same engine
        // draft path; these are semantic mutations, not observation-only
        // annotations.
        let prior = LogseqUuid::from_uuid(Uuid::from_u128(9_410));
        let replacement = LogseqUuid::from_uuid(Uuid::from_u128(9_411));
        let replacement_fixture = SnapshotFixture::new_with_initial_uuid(
            "external-engine-id-replacement",
            &["pages/a.md"],
            Some(prior),
        );
        fs::write(
            replacement_fixture.graph_root.join("pages/a.md"),
            format!("- page 0\n  id:: {replacement}\n"),
        )
        .unwrap();
        let replacement_plan = replacement_fixture.plan(&["pages/a.md"]);
        let replacement_material = replacement_plan.into_execution_material().unwrap();
        assert!(replacement_material
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::ReplaceExternal { logseq_uuid },
                    ..
                } if *logseq_uuid == replacement
            )));
        replacement_fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: replacement_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_414)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_415),
                },
                replacement_material,
            )
            .unwrap();

        let removal_fixture = SnapshotFixture::new_with_initial_uuid(
            "external-engine-id-removal",
            &["pages/a.md"],
            Some(prior),
        );
        fs::write(removal_fixture.graph_root.join("pages/a.md"), b"- page 0\n").unwrap();
        let removal_plan = removal_fixture.plan(&["pages/a.md"]);
        let removal_material = removal_plan.into_execution_material().unwrap();
        assert!(removal_material
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::RemoveExternal,
                    ..
                }
            )));
        removal_fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: removal_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_416)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_417),
                },
                removal_material,
            )
            .unwrap();
    }

    #[test]
    fn existing_authenticated_logical_name_collision_blocks_before_execution_material() {
        let fixture = SnapshotFixture::new("existing-name-collision", &["pages/seed.md"]);
        let target = fixture.graph_root.join("pages/Snapshot%20Page%200.md");
        fs::write(&target, b"- external\n").unwrap();
        let plan = fixture.plan(&["pages/Snapshot%20Page%200.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            plan.blocks()[0].reason,
            ImportBlockReason::ConflictingLocalTail
        );
        assert!(plan.blocks()[0].detail.contains("already owned"));
    }

    #[test]
    fn atomic_page_name_transition_allows_chains_deletion_reuse_and_cycles() {
        let chain = SnapshotFixture::new("name-chain", &["pages/a.md", "pages/b.md"]);
        fs::rename(
            chain.graph_root.join("pages/a.md"),
            chain.graph_root.join("pages/Snapshot%20Page%201.md"),
        )
        .unwrap();
        fs::rename(
            chain.graph_root.join("pages/b.md"),
            chain.graph_root.join("pages/final.md"),
        )
        .unwrap();
        let chain_plan = chain.plan(&[
            "pages/a.md",
            "pages/b.md",
            "pages/Snapshot%20Page%201.md",
            "pages/final.md",
        ]);
        assert_eq!(chain_plan.status(), ImportPlanStatus::Reconcile);
        let chain_material = chain_plan.into_execution_material().unwrap();
        let chain_renames = chain_material
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::ReconcileExternalPageState {
                    page_id,
                    name,
                    path,
                    ..
                } => Some((*page_id, name.as_str(), path.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(chain_renames.len(), 2);
        assert!(chain_renames
            .iter()
            .any(|(_, name, _)| *name == "Snapshot Page 1"));
        chain
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: chain_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_420)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_421),
                },
                chain_material,
            )
            .unwrap();

        let reuse = SnapshotFixture::new("delete-name-reuse", &["pages/a.md", "pages/b.md"]);
        fs::remove_file(reuse.graph_root.join("pages/b.md")).unwrap();
        fs::write(
            reuse.graph_root.join("pages/Snapshot%20Page%201.md"),
            b"- replacement identity\n",
        )
        .unwrap();
        let reuse_plan = reuse.plan(&["pages/b.md", "pages/Snapshot%20Page%201.md"]);
        assert_eq!(reuse_plan.status(), ImportPlanStatus::Reconcile);
        let reuse_material = reuse_plan.into_execution_material().unwrap();
        assert!(reuse_material.transaction().operations.iter().any(
            |operation| matches!(operation, SemanticOperation::CreatePage { name, .. }
                if name.as_str() == "Snapshot Page 1")
        ));
        assert!(reuse_material
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::DeletePage { .. })));
        reuse
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: reuse_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_422)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_423),
                },
                reuse_material,
            )
            .unwrap();

        let cycle = SnapshotFixture::new("name-cycle", &["pages/a.md", "pages/b.md"]);
        let temporary = cycle.graph_root.join("pages/cycle.tmp");
        fs::rename(cycle.graph_root.join("pages/a.md"), &temporary).unwrap();
        fs::rename(
            cycle.graph_root.join("pages/b.md"),
            cycle.graph_root.join("pages/Snapshot%20Page%200.md"),
        )
        .unwrap();
        fs::rename(
            temporary,
            cycle.graph_root.join("pages/Snapshot%20Page%201.md"),
        )
        .unwrap();
        let cycle_plan = cycle.plan(&[
            "pages/a.md",
            "pages/b.md",
            "pages/Snapshot%20Page%200.md",
            "pages/Snapshot%20Page%201.md",
        ]);
        assert_eq!(cycle_plan.status(), ImportPlanStatus::Reconcile);
        let cycle_material = cycle_plan.into_execution_material().unwrap();
        cycle
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: cycle_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_424)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_425),
                },
                cycle_material,
            )
            .unwrap();
    }

    #[test]
    fn external_observation_annotations_use_each_nested_block_exact_byte_span() {
        let fixture = SnapshotFixture::new("nested-exact-spans", &["pages/a.md"]);
        let bytes = b"- parent\n\t- child\n";
        fs::write(fixture.graph_root.join("pages/a.md"), bytes).unwrap();
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let entry = &plan.execution_material().unwrap().observation().entries()[0];
        let spans = entry
            .state()
            .annotations()
            .iter()
            .map(|annotation| (annotation.span().start(), annotation.span().end()))
            .collect::<Vec<_>>();
        assert_eq!(spans, vec![(0, 9), (9, bytes.len() as u64)]);
    }

    #[test]
    fn sparse_observation_accepts_promoted_heading_spans_and_locators_in_source_order() {
        let fixture = SnapshotFixture::new("promoted-heading-sparse-spans", &["pages/a.md"]);
        let bytes = b"# Project\n\t- child one\n\t- child two\n- sibling\n\t- nested sibling child";
        fs::write(fixture.graph_root.join("pages/a.md"), bytes).unwrap();
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let annotations = plan.execution_material().unwrap().observation().entries()[0]
            .state()
            .annotations();
        let starts = [
            b"# Project".as_slice(),
            b"\t- child one".as_slice(),
            b"\t- child two".as_slice(),
            b"- sibling".as_slice(),
            b"\t- nested sibling child".as_slice(),
        ]
        .map(|needle| {
            bytes
                .windows(needle.len())
                .position(|window| window == needle)
                .unwrap() as u64
        });
        assert_eq!(
            annotations
                .iter()
                .map(|annotation| (annotation.span().start(), annotation.span().end()))
                .collect::<Vec<_>>(),
            vec![
                (starts[0], starts[1]),
                (starts[1], starts[2]),
                (starts[2], starts[3]),
                (starts[3], starts[4]),
                (starts[4], bytes.len() as u64),
            ]
        );
        assert_eq!(
            annotations
                .iter()
                .map(|annotation| annotation.locator().components().to_vec())
                .collect::<Vec<_>>(),
            vec![vec![0], vec![0, 0], vec![0, 1], vec![1], vec![1, 0]]
        );
    }

    #[test]
    fn parser_owned_spans_cover_literal_regions_crlf_preambles_and_multiple_roots() {
        fn offsets(text: &[u8], needles: &[&[u8]]) -> Vec<u64> {
            needles
                .iter()
                .map(|needle| {
                    text.windows(needle.len())
                        .position(|window| window == *needle)
                        .unwrap() as u64
                })
                .collect()
        }

        let markdown = b"title:: Page\r\n\r\n- parent\r\n  continuation\r\n  ```\r\n  - literal\r\n  ```\r\n  - child\r\n- root two\r\n";
        let markdown_path = ManagedPath::parse("pages/literal.md").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&markdown_path, markdown, &mut instrumentation).unwrap();
        let starts = offsets(markdown, &[b"- parent", b"  - child", b"- root two"]);
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (starts[0], starts[1]),
                (starts[1], starts[2]),
                (starts[2], markdown.len() as u64),
            ]
        );
        assert!(tree.nodes[0].raw.contains("- literal"));
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.roots, vec![0, 2]);

        let org = b"#+TITLE: Page\r\n* parent\r\n#+BEGIN_SRC\r\n* literal\r\n#+END_SRC\r\n** child\r\n* root two\r\n";
        let org_path = ManagedPath::parse("pages/literal.org").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&org_path, org, &mut instrumentation).unwrap();
        let starts = offsets(org, &[b"* parent", b"** child", b"* root two"]);
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (starts[0], starts[1]),
                (starts[1], starts[2]),
                (starts[2], org.len() as u64),
            ]
        );
        assert!(tree.nodes[0].raw.contains("* literal"));
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.roots, vec![0, 2]);
    }

    #[test]
    fn import_admission_reuses_the_parser_owned_document() {
        let markdown_path = ManagedPath::parse("pages/reused.md").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        crate::outline::reset_parse_attempts();
        parse_nodes(
            &markdown_path,
            b"- parent\n  - child\n",
            &mut instrumentation,
        )
        .unwrap();
        assert_eq!(
            crate::outline::parse_attempts(),
            2,
            "Markdown admission needs the original parse and canonical reparse"
        );

        let org_path = ManagedPath::parse("pages/reused.org").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        crate::outline::reset_parse_attempts();
        parse_nodes(&org_path, b"* parent\n** child\n", &mut instrumentation).unwrap();
        assert_eq!(
            crate::outline::parse_attempts(),
            1,
            "Org admission reproduces bytes from the original parse"
        );
    }

    #[test]
    fn collapsed_heading_and_flat_bullets_have_exact_parser_owned_sibling_spans() {
        let bytes =
            b"page:: property\r\n\r\n# Collapsed\r\ncollapsed:: true\r\n- child\r\n- sibling\r\n";
        let path = ManagedPath::parse("pages/promoted.md").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&path, bytes, &mut instrumentation).unwrap();
        let heading = bytes
            .windows(b"# Collapsed".len())
            .position(|window| window == b"# Collapsed")
            .unwrap() as u64;
        let child = bytes
            .windows(b"- child".len())
            .position(|window| window == b"- child")
            .unwrap() as u64;
        let sibling = bytes
            .windows(b"- sibling".len())
            .position(|window| window == b"- sibling")
            .unwrap() as u64;
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (heading, child),
                (child, sibling),
                (sibling, bytes.len() as u64),
            ]
        );
        assert_eq!(tree.roots, vec![0, 1, 2]);
        assert!(tree.nodes.iter().all(|node| node.children.is_empty()));
    }

    #[test]
    fn lsdoc_promoted_heading_nested_run_has_exact_crlf_parser_owned_spans() {
        let bytes = b"title:: Synthetic\r\n\r\n# Project \xce\xa9\r\n\t- child one\r\n\t- child two\r\n- sibling\r\n\t- nested sibling child\r\n";
        let path = ManagedPath::parse("pages/promoted-nested.md").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&path, bytes, &mut instrumentation).unwrap();
        let starts = [
            b"# Project \xce\xa9".as_slice(),
            b"\t- child one".as_slice(),
            b"\t- child two".as_slice(),
            b"- sibling".as_slice(),
            b"\t- nested sibling child".as_slice(),
        ]
        .map(|needle| {
            bytes
                .windows(needle.len())
                .position(|window| window == needle)
                .unwrap() as u64
        });
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (starts[0], starts[1]),
                (starts[1], starts[2]),
                (starts[2], starts[3]),
                (starts[3], starts[4]),
                (starts[4], bytes.len() as u64),
            ]
        );
        assert_eq!(tree.roots, vec![0, 3]);
        assert_eq!(tree.nodes[0].children, vec![1, 2]);
        assert_eq!(tree.nodes[3].children, vec![4]);
    }

    #[test]
    fn inactive_bootstrap_accepts_lsdoc_promoted_heading_source() {
        let source =
            "# Project\n\t- child one\n\t- child two\n- sibling\n\t- nested sibling child\n";
        let (_root, prepared, _) = prepare_streaming_bootstrap(
            "bootstrap-promoted-heading",
            &[("pages/promoted.md", source)],
        );
        let page_id = prepared
            .aggregate()
            .import_id()
            .unmatched_page_id(&ImportLocator::page(
                ManagedPath::parse("pages/promoted.md").unwrap(),
            ));
        let page = prepared
            .candidate()
            .accepted_engine()
            .materialize_page(page_id)
            .unwrap();
        let block_indexes = page
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.block_id, index))
            .collect::<BTreeMap<_, _>>();
        let block_parents = page
            .blocks
            .iter()
            .map(|block| {
                (
                    block.content.clone(),
                    block
                        .parent
                        .map(|parent| page.blocks[block_indexes[&parent]].content.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            block_parents,
            BTreeMap::from([
                ("# Project".into(), None),
                ("child one".into(), Some("# Project".into())),
                ("child two".into(), Some("# Project".into())),
                ("sibling".into(), None),
                ("nested sibling child".into(), Some("sibling".into())),
            ])
        );
        assert_eq!(prepared.instrumentation().parser_nodes, 5);
        assert!(prepared.instrumentation().source_spans >= page.blocks.len() as u64);
    }

    #[test]
    fn completed_direct_receipt_cannot_reach_catalog_capture_after_empty_rollback() {
        let fixture = SnapshotFixture::new("direct-receipt-empty-rollback", &["pages/a.md"]);
        let SnapshotFixture {
            _root,
            graph,
            receipts,
            engine,
            empty_history_head,
            ..
        } = fixture;
        let workspace = engine.workspace_id();
        let endpoint = engine.projection_endpoint_binding().unwrap();
        let archive = _root.path().join("archive");
        let history_head = archive
            .join("engine-history")
            .join(endpoint.endpoint_id.to_string())
            .join("engine-history.head");
        drop(engine);
        fs::write(history_head, empty_history_head).unwrap();

        let reopened = ShardedHotEngine::with_enrolled_projection(
            ObjectStore::open(&archive, workspace).unwrap(),
            LineageDigest::of(b"snapshot-test"),
            DocumentId::from_uuid(Uuid::from_u128(4)),
            &graph,
            &receipts,
        );
        let mut instrumentation = ImportInstrumentation::default();
        let requested = vec![ManagedPath::parse("pages/a.md").unwrap()];
        let block =
            capture_affected_catalog(&receipts, &reopened, None, &requested, &mut instrumentation)
                .unwrap_err();
        assert_eq!(block.reason, ImportBlockReason::AuthorityUnavailable);
        assert_eq!(instrumentation.catalog_entries, 0);
        assert!(block.detail.contains("history") || block.detail.contains("projection"));
    }

    #[test]
    fn affected_receipt_capture_ignores_unrelated_completed_receipts() {
        use crate::oplog::projection_store::{
            projection_store_test_counters, reset_projection_store_test_counters,
        };

        let paths = (0..33)
            .map(|index| format!("pages/unrelated/{index:02}.md"))
            .collect::<Vec<_>>();
        let path_refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let fixture = SnapshotFixture::new("bounded-receipt-capture", &path_refs);
        fs::write(
            fixture.graph_root.join("pages/unrelated/00.md"),
            b"- changed\n",
        )
        .unwrap();
        let unrelated_completion = fixture
            .receipts
            .root_path()
            .join("completions")
            .join(completion_name(&fixture.intents[32]));
        reset_projection_store_test_counters();
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(unrelated_completion).unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/unrelated/00.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let counters = projection_store_test_counters();
        assert_eq!(counters.catalog_directory_entries, 0);
        assert_eq!(
            counters.completion_lookups, 2,
            "only the requested receipt is point-loaded in each snapshot pass"
        );
        assert_eq!(plan.instrumentation().catalog_entries, 2);
    }

    #[test]
    fn affected_import_never_scans_unrelated_pages_for_home_documents() {
        let paths = (0..32)
            .map(|index| format!("pages/unrelated/{index:02}.md"))
            .collect::<Vec<_>>();
        let path_refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let fixture = SnapshotFixture::new("affected-home-scope", &path_refs);
        fs::write(
            fixture.graph_root.join("pages/unrelated/00.md"),
            b"- externally changed\n",
        )
        .unwrap();

        let plan = fixture.plan(&["pages/unrelated/00.md"]);

        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        assert_eq!(plan.instrumentation().catalog_path_lookups, 1);
        assert_eq!(
            fixture.engine.canonical_snapshot_calls_for_test(),
            0,
            "page-home capture must stay at the point-scoped accepted materialization"
        );
    }

    #[test]
    fn aggregate_budget_refuses_before_overflow_or_allocation() {
        assert_eq!(
            charge_budget(
                "aggregate raw bytes",
                MAX_IMPORT_RAW_BYTES - 1,
                1,
                MAX_IMPORT_RAW_BYTES
            )
            .unwrap(),
            MAX_IMPORT_RAW_BYTES
        );
        assert!(matches!(
            charge_budget(
                "aggregate raw bytes",
                MAX_IMPORT_RAW_BYTES,
                1,
                MAX_IMPORT_RAW_BYTES
            ),
            Err(InventoryError::ResourceBudgetExceeded {
                resource: "aggregate raw bytes",
                ..
            })
        ));
        assert!(charge_budget("aggregate raw bytes", u64::MAX, 1, MAX_IMPORT_RAW_BYTES).is_err());

        let path = ManagedPath::parse("pages/a.md").unwrap();
        let parsed = crate::doc::try_parse_with_source_spans("- one more\n").unwrap();
        assert_eq!(
            enforce_outline_limits(&path, &parsed, MAX_IMPORT_PARSED_NODES)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );
        let tree = ParsedTree {
            path,
            preamble: None,
            roots: vec![0],
            nodes: vec![ParsedNode {
                parent: None,
                sibling_position: 0,
                depth: 1,
                children: Vec::new(),
                span: StructuralSpan::new(0, 0).unwrap(),
                raw: "node".into(),
                raw_ids: Vec::new(),
            }],
        };
        let mut instrumentation = ImportInstrumentation {
            locator_components_materialized: MAX_IMPORT_LOCATOR_COMPONENTS,
            ..ImportInstrumentation::default()
        };
        assert_eq!(
            materialize_locator(&tree, 0, &mut instrumentation)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );

        let mut replay = ImportInstrumentation::default();
        let replay_limits = ImportReplayLimits {
            entries: 2,
            base_bytes: 8,
            rendered_bytes: 8,
        };
        let path = ManagedPath::parse("pages/replay.md").unwrap();
        reserve_base_replay(&mut replay, 4, replay_limits, &path).unwrap();
        reserve_base_replay(&mut replay, 4, replay_limits, &path).unwrap();
        assert_eq!(
            reserve_base_replay(&mut replay, 0, replay_limits, &path)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );
        retain_rendered_target(&mut replay, 8, replay_limits, &path).unwrap();
        assert_eq!(
            retain_rendered_target(&mut replay, 1, replay_limits, &path)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );
    }

    #[test]
    fn operation_count_bound_refuses_the_100001st_operation_before_publication() {
        let mut operations = Vec::with_capacity(MAX_TRANSACTION_OPERATIONS);
        for index in 0..MAX_TRANSACTION_OPERATIONS {
            push_operation(
                &mut operations,
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                },
            )
            .unwrap();
        }
        assert_eq!(operations.len(), MAX_TRANSACTION_OPERATIONS);
        assert!(matches!(
            push_operation(
                &mut operations,
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(100_001)),
                },
            ),
            Err(ImportExecutionError::OperationLimit)
        ));
    }

    #[derive(Debug, Eq, PartialEq)]
    struct BootstrapConstructionShape {
        pages: usize,
        parts: u32,
        page_part_touches: usize,
        publication_durability_syncs: usize,
    }

    fn page_coherent_bootstrap_shape(pages: usize) -> BootstrapConstructionShape {
        const BLOCK_DEPTHS: usize = 11;
        let root = TestRoot::new(&format!("bootstrap-shape-{pages}"));
        let operation_path = root.path().join(BOOTSTRAP_STREAM_OPERATION_SPOOL);
        let mut output = BufWriter::new(create_new_file(&operation_path).unwrap());
        let mut operation_count = 0_u64;
        let mut emit = |page: usize, ordinal: usize| {
            let source_leaf = SourceLeafDigestV1::from_bytes(
                *ContentDigest::of(format!("page-{page}").as_bytes()).as_bytes(),
            );
            let record = BootstrapOperationRecord::new(
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(
                        1 + page as u128 * 32 + ordinal as u128,
                    )),
                },
                source_leaf,
                Some(StructuralSpan::new(ordinal as u64, ordinal as u64 + 1).unwrap()),
            )
            .unwrap();
            write_sort_record(
                &mut output,
                &SortRecord {
                    key: Vec::new(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
            operation_count += 1;
        };
        for page in 0..pages {
            emit(page, 0);
        }
        for page in 0..pages {
            for depth in 0..BLOCK_DEPTHS {
                emit(page, depth + 1);
            }
        }
        output.flush().unwrap();
        drop(output);

        let spool = BootstrapOperationSpool {
            path: operation_path,
            operation_count,
            declaration_count: pages as u64,
        };
        force_next_bootstrap_part_operation_limit(128);
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        let parts =
            partition_bootstrap_operation_spool(&spool, root.path(), &mut instrumentation).unwrap();
        let mut boundaries = FrameReader::open(
            &root.path().join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL),
            std::mem::size_of::<u32>(),
        )
        .unwrap();
        let mut operations = BootstrapOperationSpoolReader::open(&spool.path).unwrap();
        let mut page_part_touches = 0_usize;
        while let Some(boundary) = boundaries.next().unwrap() {
            let count = u32::from_be_bytes(boundary.try_into().unwrap());
            let mut touched = BTreeSet::new();
            for _ in 0..count {
                touched.insert(operations.next().unwrap().unwrap().source_leaf);
            }
            page_part_touches += touched.len();
        }
        assert!(operations.next().unwrap().is_none());
        BootstrapConstructionShape {
            pages,
            parts,
            page_part_touches,
            // One prefix sync plus the commit-last publication barrier.
            publication_durability_syncs: 2,
        }
    }

    #[test]
    fn bootstrap_construction_page_coherence_and_syncs_are_bounded_at_128_512_pages() {
        let small = page_coherent_bootstrap_shape(128);
        let large = page_coherent_bootstrap_shape(512);
        eprintln!("bootstrap pass-after small={small:?} large={large:?}");
        assert!(small.parts <= 14);
        assert!(large.parts <= 55);
        assert_eq!(small.page_part_touches, 128 * 2);
        assert_eq!(large.page_part_touches, 512 * 2);
        assert_eq!(small.publication_durability_syncs, 2);
        assert_eq!(large.publication_durability_syncs, 2);
    }

    #[test]
    fn structural_common_prefix_work_and_repeated_deep_locators_are_charged() {
        let path = ManagedPath::parse("pages/structural.md").unwrap();
        let mut text = String::new();
        for _ in 0..64 {
            text.push_str("- parent\n");
            for _ in 0..16 {
                text.push_str("\t- same child\n");
            }
        }
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&path, text.as_bytes(), &mut instrumentation).unwrap();
        let mut interner = StructuralInterner::new();
        structural_classes(&tree, &mut interner, &mut instrumentation).unwrap();
        structural_classes(&tree, &mut interner, &mut instrumentation).unwrap();
        assert!(instrumentation.structural_key_components >= tree.nodes.len() * 2);
        assert!(instrumentation.structural_key_comparisons > tree.nodes.len());

        let mut nodes = Vec::new();
        for depth in 1..=MAX_IMPORT_DEPTH {
            nodes.push(ParsedNode {
                parent: if depth > 1 { Some(depth - 2) } else { None },
                sibling_position: 0,
                depth,
                children: (depth < MAX_IMPORT_DEPTH)
                    .then_some(depth)
                    .into_iter()
                    .collect(),
                span: StructuralSpan::new(0, 0).unwrap(),
                raw: "node".into(),
                raw_ids: vec!["duplicate".into(), "duplicate".into()],
            });
        }
        let deep = ParsedTree {
            path,
            preamble: None,
            roots: vec![0],
            nodes,
        };
        let before = instrumentation.locator_components_materialized;
        materialize_locator(&deep, MAX_IMPORT_DEPTH - 1, &mut instrumentation).unwrap();
        materialize_locator(&deep, MAX_IMPORT_DEPTH - 1, &mut instrumentation).unwrap();
        assert_eq!(
            instrumentation.locator_components_materialized - before,
            MAX_IMPORT_DEPTH * 2
        );
    }

    #[test]
    fn structural_class_allocation_work_is_linear_across_many_pages() {
        fn measured(page_count: usize) -> ImportInstrumentation {
            let mut interner = StructuralInterner::new();
            let mut instrumentation = ImportInstrumentation::default();
            for index in 0..page_count {
                let tree = ParsedTree {
                    path: ManagedPath::parse(&format!("pages/p{index:08}.md")).unwrap(),
                    preamble: None,
                    roots: vec![0],
                    nodes: vec![ParsedNode {
                        parent: None,
                        sibling_position: 0,
                        depth: 1,
                        children: Vec::new(),
                        span: StructuralSpan::new(0, 0).unwrap(),
                        raw: format!("unique-{index:08}"),
                        raw_ids: Vec::new(),
                    }],
                };
                structural_classes(&tree, &mut interner, &mut instrumentation).unwrap();
                instrumentation.structural_class_nodes = instrumentation
                    .structural_class_nodes
                    .saturating_add(tree.nodes.len());
            }
            instrumentation
        }

        let small = measured(1_024);
        let large = measured(8_192);
        assert_eq!(small.structural_class_allocations, 1_024);
        assert_eq!(large.structural_class_allocations, 8_192);
        assert_eq!(small.structural_key_comparisons, 0);
        assert_eq!(large.structural_key_comparisons, 0);
        assert!(
            large.recorded_work_units() <= small.recorded_work_units().saturating_mul(8),
            "structural work did not scale linearly: small={small:?}, large={large:?}"
        );
    }

    /// The durable reference-catalog capability of one target archive.
    ///
    /// A bootstrap preparation is authored for exactly one archive: its
    /// accepted cold records bind catalog roots that live in that archive's
    /// authenticated Patricia store, so installing it elsewhere fails closed.
    fn target_catalog(archive: &Path, workspace: WorkspaceId) -> BootstrapAuthoringCapability {
        ObjectStore::open(archive, workspace)
            .unwrap()
            .bootstrap_authoring_capability()
            .unwrap()
    }

    fn prepare_streaming_bootstrap(
        label: &str,
        files: &[(&str, &str)],
    ) -> (TestRoot, InactiveBootstrapPreparedPublication, WorkspaceId) {
        prepare_streaming_bootstrap_with_config(label, None, files)
    }

    fn bootstrap_preparation_scratch(root: &TestRoot, label: &str) -> (PathBuf, PathBuf) {
        let nonce = Uuid::new_v4();
        let capture_scratch = root.path().join(format!("capture-{label}-{nonce}"));
        let preparation_scratch = root.path().join(format!("preparation-{label}-{nonce}"));
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        (capture_scratch, preparation_scratch)
    }

    fn prepare_streaming_bootstrap_with_config(
        label: &str,
        config: Option<&str>,
        files: &[(&str, &str)],
    ) -> (TestRoot, InactiveBootstrapPreparedPublication, WorkspaceId) {
        let root = TestRoot::new(label);
        let graph_root = root.path().join("graph");
        if let Some(config) = config {
            fs::create_dir_all(graph_root.join("logseq")).unwrap();
            fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
        }
        for (path, contents) in files {
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, contents).unwrap();
        }
        let graph = Graph::open(&graph_root);
        let (capture_scratch, preparation_scratch) =
            bootstrap_preparation_scratch(&root, "streaming-bootstrap");
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a01));
        let prepared = prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            LineageDigest::of(b"inactive-streaming-bootstrap-test"),
            DocumentId::from_uuid(Uuid::from_u128(0x5a02)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(&root.path().join("archive"), workspace),
            &preparation_scratch,
        )
        .unwrap();
        (root, prepared, workspace)
    }

    fn prepare_streaming_bootstrap_attempt(
        root: &TestRoot,
        suffix: &str,
        workspace: WorkspaceId,
    ) -> Result<InactiveBootstrapPreparedPublication, BootstrapStreamingImportError> {
        let graph = Graph::open(&root.path().join("graph"));
        let (capture_scratch, preparation_scratch) = bootstrap_preparation_scratch(root, suffix);
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            LineageDigest::of(b"inactive-streaming-bootstrap-retry-test"),
            DocumentId::from_uuid(Uuid::from_u128(0x5a22)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(&root.path().join("archive"), workspace),
            &preparation_scratch,
        )
    }

    fn count_packed_patricia_heads(path: &Path) -> usize {
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| {
                        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                            count_packed_patricia_heads(&entry.path())
                        } else {
                            usize::from(entry.file_name() == "patricia-pack-head-v1")
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }

    fn abandoned_packed_patricia_object(path: &Path) -> Option<PathBuf> {
        std::fs::read_dir(path)
            .ok()?
            .filter_map(Result::ok)
            .find_map(|entry| {
                let entry_type = entry.file_type().ok()?;
                let path = entry.path();
                if entry_type.is_dir() {
                    abandoned_packed_patricia_object(&path)
                } else if entry_type.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "patricia-pack-v1")
                {
                    Some(path)
                } else {
                    None
                }
            })
    }

    #[test]
    #[ignore = "calibrated bootstrap-authoring trace"]
    fn inactive_streaming_bootstrap_authoring_trace_calibrated() {
        let file_count = std::env::var("TINE_BOOTSTRAP_TRACE_FILES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100);
        let dense = std::env::var_os("TINE_BOOTSTRAP_TRACE_DENSE").is_some();
        let owned = (0..file_count)
            .map(|file_index| {
                let path = format!("pages/trace-{file_index:04}.md");
                let contents = (0..10)
                    .map(|block_index| {
                        if dense {
                            format!(
                                "- trace {file_index:04}/{block_index:02} [[trace-{:04}]]\n  id:: {}\n",
                                (file_index + 1) % file_count,
                                Uuid::from_u128(
                                    0x5a40_0000
                                        + (file_index * 10 + block_index) as u128
                                )
                            )
                        } else {
                            format!("- trace {file_index:04}/{block_index:02}\n")
                        }
                    })
                    .collect::<String>();
                (path, contents)
            })
            .collect::<Vec<_>>();
        let files = owned
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
            .collect::<Vec<_>>();
        let (_root, prepared, _) = prepare_streaming_bootstrap("authoring-trace", &files);
        let publication = prepared.candidate().detached_publication_stats().unwrap();
        assert_eq!(publication.successful_batch_completions, 1);
        eprintln!(
            "bootstrap authoring trace files={} dense={} operations={} parts={} immutable_publications={} batch_completions={} source_protocol_ms={:.3} spool_ms={:.3} partition_ms={:.3} detached_authoring_ms={:.3} sealing_ms={:.3} max_part_documents={} max_part_operations={}",
            file_count,
            dense,
            prepared.instrumentation().operations,
            prepared.instrumentation().parts,
            publication.immutable_publications,
            publication.successful_batch_completions,
            prepared.instrumentation().source_protocol_micros as f64 / 1_000.0,
            prepared.instrumentation().operation_spool_micros as f64 / 1_000.0,
            prepared.instrumentation().partition_micros as f64 / 1_000.0,
            prepared.instrumentation().detached_authoring_micros as f64 / 1_000.0,
            prepared.instrumentation().preparation_sealing_micros as f64 / 1_000.0,
            prepared.instrumentation().max_part_documents,
            prepared.instrumentation().peak_owned_part_operations,
        );
    }

    #[test]
    #[ignore = "1000-page structural bootstrap-authoring proof"]
    fn inactive_streaming_bootstrap_1000_page_catalog_and_document_io_are_linear() {
        const PAGE_COUNT: usize = 1_000;
        const MAX_SCRATCH_PAGE_READS_PER_CAPSULE: usize = 256;
        const MAX_SCRATCH_BYTES_PER_CAPSULE: usize = 384 * 1024;
        let owned = (0..PAGE_COUNT)
            .map(|index| {
                (
                    format!("pages/linear-{index:04}.md"),
                    format!("- linear block {index:04}\n"),
                )
            })
            .collect::<Vec<_>>();
        let files = owned
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
            .collect::<Vec<_>>();
        let (_root, prepared, _) = prepare_streaming_bootstrap("authoring-linear-1000", &files);

        assert_eq!(prepared.instrumentation().page_capsules, PAGE_COUNT as u64);
        assert_eq!(prepared.instrumentation().parts, 17);
        assert_eq!(prepared.aggregate().parts().len(), 17);
        assert!(
            prepared.instrumentation().peak_owned_part_operations
                <= u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART)
        );
        assert!(
            prepared.instrumentation().max_part_manifest_bytes
                <= BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES as u64
        );
        assert!(
            prepared.instrumentation().max_part_payload_descriptors
                <= u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART)
        );

        let work = prepared.candidate().bootstrap_catalog_work_stats();
        assert_eq!(work.full_catalog_author_clones, 0);
        assert_eq!(work.reference_fallback_document_reconstructions, 0);
        assert!(
            work.reference_catalog_peak_resident_bytes
                <= super::super::authenticated_patricia::MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
            "private Patricia construction exceeded its fixed resident budget: {work:?}"
        );
        assert_eq!(
            work.reference_catalog_prepared_validations,
            prepared.aggregate().parts().len(),
            "each accepted part must consume one exact prepared-candidate proof"
        );
        assert_eq!(
            work.reference_catalog_full_delta_validations, 0,
            "private same-call construction must not replay prepared catalog deltas"
        );
        assert_eq!(
            work.reference_catalog_final_validations, 1,
            "the complete reachable catalog must be validated exactly once before the candidate leaves construction"
        );
        assert_eq!(
            work.authenticated_page_identity_lookups,
            PAGE_COUNT * 3,
            "author page-home resolution, prospective-reference validation, and reference-source preparation must each use one bounded authenticated point per page"
        );
        let io = prepared.candidate().accepted_engine().instrumentation();
        eprintln!(
            "bootstrap document I/O pages={PAGE_COUNT} scratch_reads={} scratch_bytes={} logical_document_reads={} external_point_reads={}",
            io.scratch_page_reads,
            io.scratch_page_bytes_read,
            io.document_point_reads,
            io.external_point_reads,
        );
        assert!(
            io.scratch_page_reads <= PAGE_COUNT * MAX_SCRATCH_PAGE_READS_PER_CAPSULE,
            "authenticated scratch page reads exceeded the linear capsule ceiling: {io:?}"
        );
        assert!(
            io.scratch_page_bytes_read <= PAGE_COUNT * MAX_SCRATCH_BYTES_PER_CAPSULE,
            "authenticated scratch bytes exceeded the linear capsule ceiling: {io:?}"
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_zero_source_is_canonical_and_sealed() {
        let (_root, prepared, _) = prepare_streaming_bootstrap("streaming-zero", &[]);
        assert!(prepared.aggregate().parts().is_empty());
        assert_eq!(prepared.commit().part_count(), 0);
        assert_eq!(prepared.candidate().part_count(), 0);
        assert_eq!(prepared.instrumentation().operations, 0);
        assert_eq!(prepared.instrumentation().parts, 0);
        prepared
            .commit()
            .validate_aggregate(prepared.aggregate())
            .unwrap();
        assert_eq!(
            BootstrapAggregateCommitV1::decode(&prepared.commit_bytes().unwrap()).unwrap(),
            prepared.commit()
        );
        let publication = prepared.candidate().detached_publication_stats().unwrap();
        assert_eq!(publication.immutable_publications, 0);
        assert_eq!(publication.successful_batch_completions, 1);
    }

    #[test]
    fn detached_bootstrap_density_changes_object_work_not_completion_or_semantics() {
        let sparse_files = [
            ("pages/Sparse 0.md", "title:: Sparse 0\n\n- plain zero\n"),
            ("pages/Sparse 1.md", "title:: Sparse 1\n\n- plain one\n"),
        ];
        let dense_uuid_0 = LogseqUuid::from_uuid(Uuid::from_u128(0x5a10));
        let dense_uuid_1 = LogseqUuid::from_uuid(Uuid::from_u128(0x5a11));
        let dense_owned = [
            (
                "pages/Dense 0.md",
                format!(
                    "title:: Dense 0\n\n- dense zero [[Dense 1]]\n  id:: {}\n",
                    dense_uuid_0.as_uuid()
                ),
            ),
            (
                "pages/Dense 1.md",
                format!(
                    "title:: Dense 1\n\n- dense one [[Dense 0]]\n  id:: {}\n",
                    dense_uuid_1.as_uuid()
                ),
            ),
        ];
        let dense_files = dense_owned
            .iter()
            .map(|(path, contents)| (*path, contents.as_str()))
            .collect::<Vec<_>>();
        let (_sparse_root, sparse, _) =
            prepare_streaming_bootstrap("detached-batch-sparse", &sparse_files);
        let (_dense_root, dense, _) =
            prepare_streaming_bootstrap("detached-batch-dense", &dense_files);

        let sparse_stats = sparse.candidate().detached_publication_stats().unwrap();
        let dense_stats = dense.candidate().detached_publication_stats().unwrap();
        assert!(sparse_stats.immutable_publications > 0);
        assert!(
            dense_stats.immutable_publications > sparse_stats.immutable_publications,
            "reference/UUID density should increase immutable object work: sparse={sparse_stats:?} dense={dense_stats:?}"
        );
        assert_eq!(sparse_stats.successful_batch_completions, 1);
        assert_eq!(dense_stats.successful_batch_completions, 1);

        for (prepared, expected) in [
            (
                &sparse,
                [
                    ("pages/Sparse 0.md", "Sparse 0", "plain zero"),
                    ("pages/Sparse 1.md", "Sparse 1", "plain one"),
                ],
            ),
            (
                &dense,
                [
                    ("pages/Dense 0.md", "Dense 0", "dense zero [[Dense 1]]"),
                    ("pages/Dense 1.md", "Dense 1", "dense one [[Dense 0]]"),
                ],
            ),
        ] {
            for (path, name, content) in expected {
                let path = ManagedPath::parse(path).unwrap();
                let page_id = prepared
                    .aggregate()
                    .import_id()
                    .unmatched_page_id(&ImportLocator::page(path));
                let page = prepared
                    .candidate()
                    .accepted_engine()
                    .materialize_page(page_id)
                    .unwrap();
                assert_eq!(page.name.as_str(), name);
                assert_eq!(page.blocks.len(), 1);
                assert!(page.blocks[0].content.starts_with(content));
            }
        }

        for (path, uuid) in [
            ("pages/Dense 0.md", dense_uuid_0),
            ("pages/Dense 1.md", dense_uuid_1),
        ] {
            let page_id = dense
                .aggregate()
                .import_id()
                .unmatched_page_id(&ImportLocator::page(ManagedPath::parse(path).unwrap()));
            let page = dense
                .candidate()
                .accepted_engine()
                .materialize_page(page_id)
                .unwrap();
            assert_eq!(page.blocks[0].logseq_uuid, Some(uuid));
            assert!(matches!(
                dense.candidate().accepted_engine().resolve_logseq_uuid(uuid).unwrap(),
                super::super::hot_engine::LogseqUuidResolution::Unique(claim)
                    if claim.page_id == page_id && claim.block_id == page.blocks[0].block_id
            ));
            let posting = dense
                .candidate()
                .accepted_engine()
                .reference_posting_for_test(page_id)
                .unwrap()
                .unwrap();
            assert_eq!(posting.facts().len(), 1);
            assert!(matches!(
                &posting.facts()[0],
                super::super::reference_catalog::ReferenceFactV1::PageName(_)
            ));
        }
    }

    #[test]
    fn detached_bootstrap_retry_verifies_abandoned_objects_before_yielding_candidate() {
        let root = TestRoot::new("detached-batch-retry");
        let graph_root = root.path().join("graph/pages");
        fs::create_dir_all(&graph_root).unwrap();
        fs::write(
            graph_root.join("retry.md"),
            "title:: Retry\n\n- retry [[Retry]]\n  id:: 00000000-0000-0000-0000-000000005a23\n",
        )
        .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a21));

        let clean_root = TestRoot::new("detached-batch-clean");
        let clean_graph_root = clean_root.path().join("graph/pages");
        fs::create_dir_all(&clean_graph_root).unwrap();
        fs::copy(
            graph_root.join("retry.md"),
            clean_graph_root.join("retry.md"),
        )
        .unwrap();
        let clean = prepare_streaming_bootstrap_attempt(&clean_root, "clean", workspace).unwrap();
        let clean_stats = clean.candidate().detached_publication_stats().unwrap();

        super::super::object_store::fail_next_detached_bootstrap_batch_finish();
        let interrupted = prepare_streaming_bootstrap_attempt(&root, "interrupted", workspace);
        assert!(interrupted.is_err());

        let retried = prepare_streaming_bootstrap_attempt(&root, "retry", workspace).unwrap();
        let stats = retried.candidate().detached_publication_stats().unwrap();
        assert!(stats.immutable_publications > 0);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(
            stats.verified_existing_publications > clean_stats.verified_existing_publications,
            "retry did not add byte-verification work for abandoned exact objects: clean={clean_stats:?} retry={stats:?}"
        );
        assert_eq!(stats.successful_batch_completions, 1);
        assert_eq!(
            retried.candidate().index_archive_identity(),
            target_catalog(&root.path().join("archive"), workspace).archive_identity()
        );
    }

    #[test]
    fn detached_bootstrap_partial_packed_heads_cannot_yield_a_candidate_and_retry() {
        let root = TestRoot::new("detached-packed-partial-heads");
        let graph_root = root.path().join("graph/pages");
        fs::create_dir_all(&graph_root).unwrap();
        fs::write(
            graph_root.join("partial.md"),
            "title:: Partial\n\n- partial [[Partial]]\n  id:: 00000000-0000-0000-0000-000000005a25\n",
        )
        .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a24));

        tine_storage::fail_head_transition_after_for_test(
            1,
            tine_storage::HeadTransitionFailureForTest::Before,
        );
        let interrupted = prepare_streaming_bootstrap_attempt(&root, "interrupted", workspace);
        assert!(
            interrupted.is_err(),
            "a partial physical head set yielded a candidate"
        );
        let partial_heads = count_packed_patricia_heads(&root.path().join("archive"));
        assert_eq!(
            partial_heads, 1,
            "fixture did not stop after one physical head"
        );

        let retried = prepare_streaming_bootstrap_attempt(&root, "retry", workspace).unwrap();
        let publication = retried.candidate().detached_publication_stats().unwrap();
        assert_eq!(publication.successful_batch_completions, 1);
        assert_eq!(publication.packed_capacity_fallbacks, 0);
        assert!(publication.packed_immutable_publications > 0);
        assert_eq!(count_packed_patricia_heads(&root.path().join("archive")), 4);
    }

    #[test]
    fn detached_bootstrap_conflicting_abandoned_content_address_fails_closed() {
        let root = TestRoot::new("detached-batch-conflict");
        let graph_root = root.path().join("graph/pages");
        fs::create_dir_all(&graph_root).unwrap();
        fs::write(graph_root.join("conflict.md"), "- conflict\n").unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a31));

        super::super::object_store::fail_next_detached_bootstrap_batch_finish();
        assert!(prepare_streaming_bootstrap_attempt(&root, "interrupted", workspace).is_err());
        let abandoned = abandoned_packed_patricia_object(&root.path().join("archive"))
            .expect("abandoned detached authoring left a packed Patricia object");
        let mut conflicting_bytes = fs::read(&abandoned).unwrap();
        assert!(!conflicting_bytes.is_empty());
        conflicting_bytes[0] ^= 0x01;
        fs::write(&abandoned, &conflicting_bytes).unwrap();

        let retry = prepare_streaming_bootstrap_attempt(&root, "retry", workspace);
        assert!(
            retry.is_err(),
            "a conflicting content-addressed immutable name must fail closed"
        );
        assert_eq!(fs::read(abandoned).unwrap(), conflicting_bytes);
    }

    #[test]
    fn detached_candidate_durability_proof_refuses_a_different_archive() {
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            "detached-candidate-archive-mismatch",
            &[("pages/bound.md", "- archive bound\n")],
        );
        let wrong_archive = root.path().join("wrong-archive");
        let result = publish_install_verify_inactive_bootstrap(
            &prepared,
            ObjectStore::open(&wrong_archive, workspace).unwrap(),
            orchestration_binding(&prepared, 0x5a41),
        );
        assert!(result.is_err());
        assert!(!wrong_archive.join("bootstrap-v1").exists());
    }

    #[test]
    fn inactive_streaming_bootstrap_import_id_and_prepared_bytes_are_canonical() {
        let (_root, prepared, workspace) = prepare_streaming_bootstrap(
            "streaming-one",
            &[(
                "pages/Unicode α.md",
                "title:: Streaming title\n- parent\n  - child",
            )],
        );
        let mut inventory = Vec::new();
        let mut cursor = prepared.source_capture().entries_cursor().unwrap();
        while let Some(entry) = cursor.next().unwrap() {
            inventory.push(ImportInventoryEntry::with_kind(
                entry.kind(),
                entry.path().clone(),
                ImportInventoryState::Present(entry.description()),
            ));
        }
        let materialized =
            ImportId::derive(workspace, &[], &inventory, DIFF_SCHEMA_VERSION).unwrap();
        assert_eq!(prepared.aggregate().import_id(), materialized);
        assert_eq!(prepared.aggregate().parts().len(), 2);
        assert_eq!(prepared.instrumentation().operations, 4);

        let mut operation_count = 0_u32;
        for ordinal in 0..2 {
            let mut part = prepared.open_part(ordinal).unwrap();
            let evidence = part.evidence().unwrap();
            operation_count += evidence.operation_root().operation_count();
            let span_index = part.span_index().unwrap();
            span_index.validate_part(evidence).unwrap();
            let manifest = super::super::OperationBatch::decode(part.manifest_bytes()).unwrap();
            let mut objects = Vec::new();
            while let Some(bytes) = part.next_object_bytes().unwrap() {
                objects.push(super::super::OperationObject::decode(&bytes).unwrap());
            }
            super::super::PreparedBatch::new(manifest, objects).unwrap();
        }
        assert_eq!(operation_count, 4);
    }

    #[test]
    fn inactive_bootstrap_refuses_non_round_tripping_org_before_operations() {
        for (label, path, source) in [(
            "bootstrap-skipped-org",
            "pages/skipped.org",
            "* parent\n*** child\n",
        )] {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, source).unwrap();
            let graph = Graph::open(&graph_root);
            let capture_scratch = root.path().join("capture-scratch");
            let preparation_scratch = root.path().join("preparation-scratch");
            fs::create_dir(&capture_scratch).unwrap();
            fs::create_dir(&preparation_scratch).unwrap();
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a11));
            let result = prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"inactive-bootstrap-source-admission"),
                DocumentId::from_uuid(Uuid::from_u128(0x5a12)),
                ReferenceCatalogPolicyV1::default(),
                &target_catalog(&root.path().join("archive"), workspace),
                &preparation_scratch,
            );
            let Err(BootstrapStreamingImportError::InvalidSource(detail)) = result else {
                panic!("{label} constructed bootstrap publication");
            };
            assert!(
                detail.starts_with(&format!("{path}: ")),
                "{label} omitted the graph-relative source path: {detail}"
            );
            assert_eq!(fs::read(target).unwrap(), source.as_bytes());
        }
    }

    #[test]
    fn inactive_bootstrap_accepts_structurally_round_tripping_markdown() {
        let source = "- parent\n\t- a\n  - b\n";
        let root = TestRoot::new("bootstrap-mixed-markdown");
        let graph_root = root.path().join("graph");
        let target = graph_root.join("pages/mixed.md");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, source).unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture-scratch");
        let preparation_scratch = root.path().join("preparation-scratch");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a21));

        let result = prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            LineageDigest::of(b"inactive-bootstrap-structural-admission"),
            DocumentId::from_uuid(Uuid::from_u128(0x5a22)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(&root.path().join("archive"), workspace),
            &preparation_scratch,
        );

        if let Err(error) = result {
            panic!("structurally stable Markdown must construct bootstrap publication: {error:?}");
        }
        assert_eq!(fs::read(target).unwrap(), source.as_bytes());
    }

    fn synthetic_operation_spool(
        directory: &Path,
        operation_count: u64,
    ) -> BootstrapOperationSpool {
        let path = directory.join("synthetic-operations.sorted");
        let mut writer = BufWriter::new(create_new_file(&path).unwrap());
        for index in 0..operation_count {
            let record = BootstrapOperationRecord::new(
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                },
                SourceLeafDigestV1::from_bytes([0x61; 32]),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: index.to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        BootstrapOperationSpool {
            path,
            operation_count,
            declaration_count: 0,
        }
    }

    fn synthetic_page_capsule_spool(directory: &Path, page_count: u64) -> BootstrapOperationSpool {
        let path = directory.join("synthetic-page-capsules.sorted");
        let mut writer = BufWriter::new(create_new_file(&path).unwrap());
        for index in 0..page_count {
            let mut source_leaf = [0_u8; 32];
            source_leaf[..8].copy_from_slice(&index.to_be_bytes());
            let record = BootstrapOperationRecord::new(
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                },
                SourceLeafDigestV1::from_bytes(source_leaf),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: index.to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        BootstrapOperationSpool {
            path,
            operation_count: page_count,
            declaration_count: 0,
        }
    }

    fn synthetic_declaration_spool(
        directory: &Path,
        declaration_count: u64,
    ) -> BootstrapOperationSpool {
        let path = directory.join("synthetic-declarations.sorted");
        let mut writer = BufWriter::new(create_new_file(&path).unwrap());
        for index in 0..declaration_count {
            let mut source_leaf = [0_u8; 32];
            source_leaf[..8].copy_from_slice(&index.to_be_bytes());
            let record = BootstrapOperationRecord::new(
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                },
                SourceLeafDigestV1::from_bytes(source_leaf),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: index.to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        BootstrapOperationSpool {
            path,
            operation_count: declaration_count,
            declaration_count,
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_partitions_zero_one_4096_and_4097_without_retention() {
        for (count, expected_parts) in [(0, 0), (1, 1), (4096, 1), (4097, 2)] {
            let root = TestRoot::new(&format!("streaming-partition-{count}"));
            let working = root.path().join("partition");
            fs::create_dir(&working).unwrap();
            let spool = synthetic_operation_spool(&working, count);
            let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
            assert_eq!(
                partition_bootstrap_operation_spool(&spool, &working, &mut instrumentation)
                    .unwrap(),
                expected_parts
            );
            assert_eq!(instrumentation.parts, expected_parts);
            assert!(instrumentation.peak_owned_part_operations == 0);
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_partitions_512_and_513_page_capsules_losslessly() {
        for (page_count, expected_boundaries) in [(512, vec![512]), (513, vec![512, 1])] {
            let root = TestRoot::new(&format!("streaming-page-capsules-{page_count}"));
            let working = root.path().join("partition");
            fs::create_dir(&working).unwrap();
            let spool = synthetic_page_capsule_spool(&working, page_count);
            let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
            assert_eq!(
                partition_bootstrap_operation_spool(&spool, &working, &mut instrumentation)
                    .unwrap(),
                expected_boundaries.len() as u32
            );
            assert_eq!(instrumentation.page_capsules, page_count);
            assert_eq!(instrumentation.max_part_documents, page_count.min(512));

            let mut boundaries = FrameReader::open(
                &working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL),
                std::mem::size_of::<u32>(),
            )
            .unwrap();
            let mut operations = BootstrapOperationSpoolReader::open(&spool.path).unwrap();
            let mut observed_boundaries = Vec::new();
            let mut observed_pages = Vec::new();
            while let Some(boundary) = boundaries.next().unwrap() {
                let operation_count = u32::from_be_bytes(boundary.try_into().unwrap());
                observed_boundaries.push(operation_count);
                for _ in 0..operation_count {
                    let record = operations.next().unwrap().unwrap();
                    let SemanticOperation::DeletePage { page_id } = record.operation else {
                        panic!("synthetic page capsule changed operation kind");
                    };
                    observed_pages.push(page_id);
                }
            }
            assert_eq!(observed_boundaries, expected_boundaries);
            assert_eq!(
                observed_pages,
                (1..=page_count)
                    .map(|index| PageId::from_uuid(Uuid::from_u128(index as u128)))
                    .collect::<Vec<_>>()
            );
            assert!(operations.next().unwrap().is_none());
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_partitions_100001_synthetic_operations_boundedly() {
        let root = TestRoot::new("streaming-partition-100001");
        let working = root.path().join("partition");
        fs::create_dir(&working).unwrap();
        let spool = synthetic_operation_spool(&working, 100_001);
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        assert_eq!(
            partition_bootstrap_operation_spool(&spool, &working, &mut instrumentation).unwrap(),
            25
        );
        assert_eq!(instrumentation.parts, 25);
        assert_eq!(instrumentation.peak_owned_part_operations, 0);
        assert!(instrumentation.source_spans <= 100_001);
    }

    #[test]
    fn inactive_streaming_bootstrap_packs_more_than_65536_empty_declarations() {
        let root = TestRoot::new("streaming-partition-65537-empty-declarations");
        let working = root.path().join("partition");
        fs::create_dir(&working).unwrap();
        let spool = synthetic_declaration_spool(&working, 65_537);
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        assert_eq!(
            partition_bootstrap_operation_spool(&spool, &working, &mut instrumentation).unwrap(),
            33
        );
        assert_eq!(instrumentation.page_declarations, 65_537);
        assert_eq!(instrumentation.page_capsules, 0);
        assert_eq!(instrumentation.max_part_documents, 2_048);
    }

    #[test]
    fn inactive_streaming_bootstrap_capture_c_mutation_leaves_no_seal() {
        let root = TestRoot::new("streaming-capture-c-mutation");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/mutable.md"), "- before").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        fs::write(graph_root.join("pages/mutable.md"), "- after").unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5b01));
        assert!(prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            LineageDigest::of(b"capture-c-mutation"),
            DocumentId::from_uuid(Uuid::from_u128(0x5b02)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(&root.path().join("archive"), workspace),
            &preparation_scratch,
        )
        .is_err());
        let publication_root = preparation_scratch.join(BOOTSTRAP_STREAM_DIRECTORY);
        assert!(
            !publication_root.exists()
                || fs::read_dir(publication_root).unwrap().all(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".building-"))
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_repeated_run_reuses_exact_seal() {
        let root = TestRoot::new("streaming-repeat");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/repeat.md"), "- repeat").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5c01));
        let catalog = target_catalog(&root.path().join("archive"), workspace);
        let prepare_once = || {
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"streaming-repeat"),
                DocumentId::from_uuid(Uuid::from_u128(0x5c02)),
                ReferenceCatalogPolicyV1::default(),
                &catalog,
                &preparation_scratch,
            )
            .unwrap()
        };
        let first = prepare_once();
        let first_aggregate = first.aggregate_bytes().unwrap();
        let first_commit = first.commit_bytes().unwrap();
        drop(first);
        let second = prepare_once();
        assert_eq!(second.aggregate_bytes().unwrap(), first_aggregate);
        assert_eq!(second.commit_bytes().unwrap(), first_commit);
    }

    #[test]
    fn inactive_streaming_bootstrap_preseal_crash_retries_exactly() {
        let root = TestRoot::new("streaming-preseal-crash-retry");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/retry.md"), "- exact retry\n").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5c06));
        let catalog = target_catalog(&root.path().join("archive"), workspace);
        let prepare_once = || {
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"streaming-preseal-crash-retry"),
                DocumentId::from_uuid(Uuid::from_u128(0x5c07)),
                ReferenceCatalogPolicyV1::default(),
                &catalog,
                &preparation_scratch,
            )
        };
        INACTIVE_BOOTSTRAP_PREPARATION_BEFORE_SEAL.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|| {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "injected preparation crash before seal",
                ))
            }));
        });
        assert!(prepare_once().is_err());
        let publication_root = preparation_scratch.join(BOOTSTRAP_STREAM_DIRECTORY);
        let unsealed = fs::read_dir(&publication_root)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .expect("durably staged deterministic preparation residue")
            .path();
        assert!(!unsealed.join(BOOTSTRAP_STREAM_SEAL).exists());

        let prepared = prepare_once().unwrap();
        assert_eq!(prepared.sealed_directory, unsealed);
        assert_eq!(
            fs::read(prepared.sealed_directory.join(BOOTSTRAP_STREAM_SEAL)).unwrap(),
            prepared.commit_bytes().unwrap()
        );
        prepared
            .commit()
            .validate_aggregate(prepared.aggregate())
            .unwrap();
    }

    #[test]
    fn inactive_streaming_bootstrap_seals_many_artifact_entries_streamingly() {
        let root = TestRoot::new("streaming-seal-many-entries");
        let artifacts = root.path().join("artifacts");
        let sealed = root.path().join("sealed");
        let nested = artifacts.join("nested");
        let write_artifacts = || {
            fs::create_dir(&artifacts).unwrap();
            fs::create_dir(&nested).unwrap();
            for index in 0_u32..128 {
                let directory = if index % 2 == 0 { &artifacts } else { &nested };
                fs::write(
                    directory.join(format!("artifact-{index:03}.bin")),
                    index.to_be_bytes(),
                )
                .unwrap();
            }
        };

        write_artifacts();
        seal_bootstrap_preparation(&artifacts, &sealed, b"commit").unwrap();
        // A real same-digest retry reconstructs an identical preparation after
        // the first one was atomically renamed into place.
        write_artifacts();
        seal_bootstrap_preparation(&artifacts, &sealed, b"commit").unwrap();

        assert_eq!(
            fs::read(sealed.join(BOOTSTRAP_STREAM_SEAL)).unwrap(),
            b"commit"
        );
        let sealed_nested = sealed.join("nested");
        for index in 0_u32..128 {
            let directory = if index % 2 == 0 {
                &sealed
            } else {
                &sealed_nested
            };
            assert_eq!(
                fs::read(directory.join(format!("artifact-{index:03}.bin"))).unwrap(),
                index.to_be_bytes()
            );
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_conflicting_sealed_artifact_fails_closed() {
        let root = TestRoot::new("streaming-conflicting-seal");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/conflict.md"), "- conflict").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5c11));
        let catalog = target_catalog(&root.path().join("archive"), workspace);
        let prepare_once = || {
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"streaming-conflicting-seal"),
                DocumentId::from_uuid(Uuid::from_u128(0x5c12)),
                ReferenceCatalogPolicyV1::default(),
                &catalog,
                &preparation_scratch,
            )
        };
        let first = prepare_once().unwrap();
        fs::write(
            first.sealed_directory.join(BOOTSTRAP_STREAM_AGGREGATE),
            b"accidental corruption",
        )
        .unwrap();
        drop(first);
        assert!(matches!(
            prepare_once(),
            Err(BootstrapStreamingImportError::ConflictingSeal)
        ));
    }

    #[test]
    fn inactive_streaming_bootstrap_authors_4096_and_4097_operation_boundaries() {
        for (block_count, expected_operations) in [
            (4_095, vec![1, 4_095]),
            (4_096, vec![1, 4_096]),
            (4_097, vec![1, 4_096, 1]),
        ] {
            let mut source = String::new();
            for index in 0..block_count {
                source.push_str(&format!("- block {index:04}\n"));
            }
            let label = format!("streaming-author-boundary-{block_count}");
            let (_root, prepared, _) =
                prepare_streaming_bootstrap(&label, &[("pages/boundary.md", &source)]);
            assert_eq!(
                prepared
                    .aggregate()
                    .parts()
                    .iter()
                    .map(|part| part
                    .evidence()
                    .operation_root()
                    .operation_count())
                    .collect::<Vec<_>>(),
                expected_operations,
                "the declaration phase is canonical and a huge page splits only at the hard content-operation bound"
            );
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_cross_part_child_and_later_content_depend_on_candidate() {
        let root = TestRoot::new("streaming-cross-part");
        let working = root.path().join("author");
        fs::create_dir(&working).unwrap();
        let page = PageId::from_uuid(Uuid::from_u128(0x5d01));
        let home = DocumentId::from_uuid(Uuid::from_u128(0x5d02));
        let parent = BlockId::from_uuid(Uuid::from_u128(0x5d03));
        let child = BlockId::from_uuid(Uuid::from_u128(0x5d04));
        let operations = [
            SemanticOperation::CreatePage {
                page_id: page,
                home_document_id: home,
                name: LogicalPageName::parse("Cross part").unwrap(),
                path: ManagedPath::parse("pages/cross-part.md").unwrap(),
                kind: ManagedTextKind::Page,
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: parent,
                    home_document_id: home,
                },
                page_id: page,
                parent: None,
                order: "0000000000".into(),
                content: "parent".into(),
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: child,
                    home_document_id: home,
                },
                page_id: page,
                parent: Some(parent),
                order: "0000000000".into(),
                content: "child".into(),
            },
            SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: parent,
                    home_document_id: home,
                },
                content: "parent updated later".into(),
            },
        ];
        let operation_path = working.join(BOOTSTRAP_STREAM_OPERATION_SPOOL);
        let mut writer = BufWriter::new(create_new_file(&operation_path).unwrap());
        for (index, operation) in operations.into_iter().enumerate() {
            let record = BootstrapOperationRecord::new(
                operation,
                SourceLeafDigestV1::from_bytes([0x62; 32]),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: (index as u64).to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        let boundary_path = working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL);
        let mut boundaries = BufWriter::new(create_new_file(&boundary_path).unwrap());
        write_frame(&mut boundaries, &2_u32.to_be_bytes()).unwrap();
        write_frame(&mut boundaries, &2_u32.to_be_bytes()).unwrap();
        boundaries.flush().unwrap();
        boundaries.get_ref().sync_all().unwrap();
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        let mut progress = |_| {};
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5d05));
        let authored = author_bootstrap_parts(
            workspace,
            LineageDigest::of(b"cross-part-author"),
            DocumentId::from_uuid(Uuid::from_u128(0x5d06)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(&root.path().join("archive"), workspace),
            ImportId::from_digest([0x63; 32]),
            &BootstrapOperationSpool {
                path: operation_path,
                operation_count: 4,
                declaration_count: 0,
            },
            2,
            &working,
            &mut instrumentation,
            &mut progress,
        )
        .unwrap();
        assert_eq!(authored.descriptors.len(), 2);
        assert_eq!(authored.candidate.part_count(), 2);
        assert_eq!(
            authored.descriptors[1].evidence().predecessor(),
            Some(authored.descriptors[0].part_id())
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_declaration_part_stays_below_payload_object_bound() {
        let root = TestRoot::new("streaming-declaration-payload-bound");
        let working = root.path().join("author");
        fs::create_dir(&working).unwrap();
        let operation_path = working.join(BOOTSTRAP_STREAM_OPERATION_SPOOL);
        let mut writer = BufWriter::new(create_new_file(&operation_path).unwrap());
        for index in 0..BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART {
            let path = format!("pages/declaration-{index}.md");
            let record = BootstrapOperationRecord::new(
                SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(0x6d00_0000 + index as u128)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(
                        0x6e00_0000 + index as u128,
                    )),
                    name: LogicalPageName::parse(&format!("Declaration {index}")).unwrap(),
                    path: ManagedPath::parse(&path).unwrap(),
                    kind: ManagedTextKind::Page,
                },
                SourceLeafDigestV1::from_bytes(*ContentDigest::of(path.as_bytes()).as_bytes()),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: index.to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        let boundary_path = working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL);
        let mut boundaries = BufWriter::new(create_new_file(&boundary_path).unwrap());
        write_frame(
            &mut boundaries,
            &BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART.to_be_bytes(),
        )
        .unwrap();
        boundaries.flush().unwrap();

        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x6f00_0001));
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        let mut progress = |_| {};
        let authored = author_bootstrap_parts(
            workspace,
            LineageDigest::of(b"declaration-payload-bound"),
            DocumentId::from_uuid(Uuid::from_u128(0x6f00_0002)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(&root.path().join("archive"), workspace),
            ImportId::from_digest([0x6f; 32]),
            &BootstrapOperationSpool {
                path: operation_path,
                operation_count: u64::from(BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART),
                declaration_count: u64::from(BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART),
            },
            1,
            &working,
            &mut instrumentation,
            &mut progress,
        )
        .unwrap();
        assert_eq!(authored.descriptors.len(), 1);
        assert_eq!(
            instrumentation.max_part_documents,
            u64::from(BOOTSTRAP_STREAM_MAX_PAGE_DECLARATIONS_PER_PART) + 1
        );
        assert!(
            instrumentation.max_part_payload_descriptors
                <= u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART)
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_matches_materialized_initial_semantic_order_and_layout() {
        let root = TestRoot::new("streaming-differential");
        let graph_root = root.path().join("graph");
        fs::create_dir_all(graph_root.join("logseq")).unwrap();
        fs::write(
            graph_root.join("logseq/config.edn"),
            r#"{:pages-directory "notes"
                :journals-directory "diary"
                :file/name-format :triple-lowbar
                :journal/file-name-format "dd-MM-yyyy"
                :journal/page-title-format "yyyy-MM-dd"}"#,
        )
        .unwrap();
        for (path, contents) in [
            ("Root.md", "title:: Root logical name\n\n- root\n"),
            (
                "notes/arbitrary/nested/Markdown.md",
                "title:: Markdown title ☕\n\n- parent\n  - child\n",
            ),
            ("notes/深い/Déjà___計画.markdown", "- filename page\n"),
            (
                "diary/nested/25-07-2026.org",
                "#+TITLE: Org title ��\n\n* org page\n",
            ),
            ("diary/nested/26-07-2026.md", "- journal\n"),
        ] {
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, contents).unwrap();
        }
        for page in 0..257 {
            let mut contents = String::new();
            for block in 0..8 {
                let identity = Uuid::from_u128(0x5e10_0000 + page * 8 + block);
                contents.push_str(&format!("- dense {page:03}/{block}\n  id:: {identity}\n"));
            }
            fs::write(
                graph_root.join(format!("notes/dense-{page:03}.md")),
                contents,
            )
            .unwrap();
        }
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let working = root.path().join("working");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&working).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5e01));

        let mut source_instrumentation = BootstrapStreamingImportInstrumentation::default();
        let source = prepare_bootstrap_source_protocol(
            workspace,
            &capture,
            &working,
            &mut source_instrumentation,
        )
        .unwrap();
        let streaming = spool_bootstrap_operations(
            &capture,
            source.import_id,
            workspace,
            &working,
            &mut source_instrumentation,
        )
        .unwrap();
        let mut partition_instrumentation = BootstrapStreamingImportInstrumentation::default();
        assert_eq!(
            partition_bootstrap_operation_spool(
                &streaming,
                &working,
                &mut partition_instrumentation,
            )
            .unwrap(),
            3
        );
        let mut boundaries = FrameReader::open(
            &working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL),
            std::mem::size_of::<u32>(),
        )
        .unwrap();
        let mut operation_boundaries = Vec::new();
        while let Some(boundary) = boundaries.next().unwrap() {
            operation_boundaries.push(u32::from_be_bytes(boundary.try_into().unwrap()));
        }
        assert_eq!(operation_boundaries.len(), 3);
        assert!(
            MAX_OPERATIONS_PER_BOOTSTRAP_PART - operation_boundaries[1] < 16,
            "the next 16-operation dense page capsule must be stopped by the operation limit"
        );
        assert!(
            partition_instrumentation.max_part_documents
                < u64::from(BOOTSTRAP_STREAM_MAX_PAGE_CAPSULES_PER_PART)
        );
        let mut streaming_reader = BootstrapOperationSpoolReader::open(&streaming.path).unwrap();
        let mut streaming_operations = Vec::new();
        while let Some(operation) = streaming_reader.next().unwrap() {
            streaming_operations.push(operation.operation);
        }

        let mut source_reader = BootstrapSourceReader::new(&capture).unwrap();
        let mut entries = capture.entries_cursor().unwrap();
        let mut raw_entries = Vec::new();
        let mut scope_paths = BTreeMap::new();
        let mut path_identities = BTreeMap::new();
        let mut read_instrumentation = BootstrapStreamingImportInstrumentation::default();
        while let Some(entry) = entries.next().unwrap() {
            let bytes = source_reader
                .read_entry(&entry, &mut read_instrumentation)
                .unwrap();
            raw_entries.push((entry.path().clone(), RawObservation::present(bytes)));
            scope_paths.insert(entry.path().clone(), ScopedPathEvidence::New);
            path_identities.insert(
                entry.path().clone(),
                ImportedPathIdentity {
                    name: LogicalPageName::parse(entry.logical_name()).unwrap(),
                    kind: entry.kind(),
                },
            );
        }
        source_reader.finish().unwrap();
        let inventory = RawInventory::from_entries(raw_entries).unwrap();
        let materialized_id = ImportId::derive(
            workspace,
            &[],
            &inventory.derivation_entries(&path_identities).unwrap(),
            DIFF_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(source.import_id, materialized_id);
        let scope = ImportScopeSnapshot {
            workspace_id: workspace,
            paths: scope_paths,
            path_identities,
        };
        let mut matches = ImportMatches::default();
        let parsed_documents = match_blocks(
            &graph,
            &inventory,
            &[],
            &mut matches,
            &mut ImportInstrumentation::default(),
        )
        .unwrap();
        let transition = build_desired_page_transition(
            &inventory,
            &matches,
            &scope,
            &scope.path_identities,
            materialized_id,
        )
        .unwrap();
        let old = build_execution_material(
            materialized_id,
            &inventory,
            &matches,
            &scope,
            &transition,
            &parsed_documents,
            &mut ImportInstrumentation::default(),
        )
        .unwrap();
        let BuiltImportMaterial::Semantic(old) = old else {
            panic!("bootstrap differential unexpectedly produced formatting-only material");
        };
        // The streaming profile intentionally authors all page declarations
        // before content capsules. The materialized differential groups by
        // operation kind, so equality is the semantic state produced below,
        // not incidental transaction ordering across those two builders.
        let streaming_transaction =
            OperationTransaction::new(streaming_operations.clone()).unwrap();
        let canonical_snapshot = |transaction: &OperationTransaction| {
            let mut engine = ShardedHotEngine::new(
                workspace,
                LineageDigest::of(b"streaming-differential-snapshot"),
                DocumentId::from_uuid(Uuid::from_u128(0x5e02)),
            );
            engine
                .configure_reference_catalog_policy(ReferenceCatalogPolicyV1::default())
                .unwrap();
            let prepared = engine
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(0x5e03)),
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(0x5e04)),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(0x5e05)),
                        crdt_peer_id: CrdtPeerId::from_u64(0x5e06),
                    },
                    transaction,
                )
                .unwrap();
            assert!(matches!(
                engine
                    .stage_ready(super::super::ValidatedBatch::new(prepared))
                    .disposition,
                super::super::BatchDisposition::Accepted { .. }
            ));
            engine.canonical_snapshot().unwrap()
        };
        assert_eq!(
            canonical_snapshot(&streaming_transaction),
            canonical_snapshot(&old.transaction)
        );
        assert_eq!(
            capture
                .entries_cursor()
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
                .as_str(),
            "Root.md"
        );
    }

    fn orchestration_binding(
        prepared: &InactiveBootstrapPreparedPublication,
        endpoint: u128,
    ) -> ProjectionStorageBinding {
        ProjectionStorageBinding {
            endpoint: ProjectionEndpointBinding {
                endpoint_id: ProjectionEndpointId::from_uuid(Uuid::from_u128(endpoint)),
                device_id: DeviceId::from_uuid(Uuid::from_u128(endpoint + 1)),
                graph_resource_id: prepared.aggregate().graph_resource(),
            },
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"inactive-bootstrap-orchestration-test",
                &(endpoint + 2).to_be_bytes(),
            ),
        }
    }

    fn prepare_existing_bootstrap(
        root: &TestRoot,
        label: &str,
        workspace: WorkspaceId,
        archive: &Path,
    ) -> InactiveBootstrapPreparedPublication {
        let graph = Graph::open(&root.path().join("graph"));
        let (capture_scratch, preparation_scratch) = bootstrap_preparation_scratch(root, label);
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            LineageDigest::of(b"inactive-bootstrap-orchestration-lineage"),
            DocumentId::from_uuid(Uuid::from_u128(0x6a02)),
            ReferenceCatalogPolicyV1::default(),
            &target_catalog(archive, workspace),
            &preparation_scratch,
        )
        .unwrap()
    }

    fn rich_orchestration_fixture(
        label: &str,
    ) -> (TestRoot, InactiveBootstrapPreparedPublication, WorkspaceId) {
        let root = TestRoot::new(label);
        let graph = root.path().join("graph");
        fs::create_dir_all(graph.join("logseq")).unwrap();
        fs::write(
            graph.join("logseq/config.edn"),
            r#"{:pages-directory "notes"
                :journals-directory "diary"
                :file/name-format :triple-lowbar
                :journal/file-name-format "dd-MM-yyyy"
                :journal/page-title-format "yyyy-MM-dd"}"#,
        )
        .unwrap();
        fs::create_dir_all(graph.join("notes/arbitrary/nested")).unwrap();
        fs::create_dir_all(graph.join("notes/深い")).unwrap();
        fs::create_dir_all(graph.join("diary/nested")).unwrap();
        fs::write(
            graph.join("Root.md"),
            "title:: Root logical name\n\n- root\n",
        )
        .unwrap();
        fs::write(
            graph.join("notes/arbitrary/nested/Markdown.md"),
            "title:: Markdown title ☕\n\n- semantic page\n",
        )
        .unwrap();
        fs::write(
            graph.join("notes/深い/Déjà___計画.markdown"),
            "- filename-derived page\n",
        )
        .unwrap();
        fs::write(
            graph.join("diary/nested/25-07-2026.org"),
            "#+TITLE: Org title ��\n\n* semantic journal\n",
        )
        .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x6a01));
        let archive = root.path().join("archive");
        let prepared = prepare_existing_bootstrap(&root, "rich", workspace, &archive);
        (root, prepared, workspace)
    }

    fn assert_verified_orchestration(
        prepared: &InactiveBootstrapPreparedPublication,
        verified: &InactiveBootstrapVerifiedPublication,
        binding: ProjectionStorageBinding,
    ) {
        assert_eq!(verified.workspace_id(), prepared.aggregate().workspace_id());
        assert_eq!(
            verified.lineage_digest(),
            prepared.aggregate().lineage_digest()
        );
        assert_eq!(
            verified.graph_resource(),
            prepared.aggregate().graph_resource()
        );
        assert_eq!(
            verified.publication_id(),
            prepared.aggregate().publication_id()
        );
        assert_eq!(
            verified.aggregate_digest(),
            prepared.aggregate().aggregate_digest()
        );
        assert_eq!(verified.import_id(), prepared.aggregate().import_id());
        assert_eq!(
            verified.part_count(),
            prepared.aggregate().parts().len() as u32
        );
        assert_eq!(
            verified.predecessor_terminal(),
            prepared.aggregate().final_frontier().last_part()
        );
        assert_eq!(
            verified.accepted_frontier(),
            &prepared.candidate().accepted_frontier_root().unwrap()
        );
        assert_eq!(
            verified.engine_binding(),
            &prepared.candidate().durable_history_binding()
        );
        assert_eq!(verified.storage_binding(), binding);
        assert_eq!(
            verified.bootstrap_binding(),
            BootstrapAggregateHistoryBindingV1::for_aggregate(prepared.aggregate()).unwrap()
        );
        assert_eq!(
            verified.history_generation(),
            u64::from(verified.part_count())
        );
        assert_eq!(
            verified.cold_record_count(),
            u64::from(verified.part_count())
        );
        assert_ne!(
            verified.history_root(),
            ContentDigest::of(b"not-the-history-root")
        );
        assert_eq!(
            verified.archive_identity(),
            verified.archive_identity(),
            "archive identity accessor is stable"
        );
        assert!(verified.instrumentation().peak_owned_source_chunks <= 1);
        assert!(verified.instrumentation().peak_owned_parts <= 1);
        assert!(verified.instrumentation().peak_owned_cold_records <= 1);
    }

    fn assert_prepared_bootstrap_limits(prepared: &InactiveBootstrapPreparedPublication) {
        assert!(prepared.aggregate().parts().len() <= MAX_BOOTSTRAP_PARTS as usize);
        assert!(
            prepared.aggregate_bytes().unwrap().len()
                <= super::super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES
        );
        assert!(
            prepared.commit_bytes().unwrap().len()
                <= super::super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES
        );
        assert!(
            prepared.instrumentation().max_part_manifest_bytes
                <= BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES as u64
        );
        assert!(
            prepared.instrumentation().max_part_payload_descriptors
                <= u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART)
        );
        assert!(
            prepared.instrumentation().peak_owned_part_operations
                <= u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART)
        );

        for (ordinal, descriptor) in prepared.aggregate().parts().iter().enumerate() {
            let evidence = descriptor.evidence();
            assert!(
                evidence.operation_root().operation_count() <= MAX_OPERATIONS_PER_BOOTSTRAP_PART
            );
            assert!(
                evidence.operation_root().semantic_effect_bytes()
                    <= MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART
            );
            assert!(
                evidence.source_span_root().span_count() <= MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART
            );
            assert!(
                evidence.payload_object_root().object_count() <= MAX_OPERATIONS_PER_BOOTSTRAP_PART
            );
            assert!(
                evidence.payload_object_root().total_bytes()
                    <= MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART
            );
            assert!(
                evidence.encode().unwrap().len()
                    <= super::super::bootstrap_import::MAX_BOOTSTRAP_PART_EVIDENCE_BYTES
            );

            let mut part = prepared.open_part(ordinal as u32).unwrap();
            assert!(part.manifest_bytes().len() <= BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES);
            assert_eq!(part.evidence().unwrap(), evidence);
            let spans = part.span_index().unwrap();
            spans.validate_part(evidence).unwrap();
            assert!(spans.spans().len() <= MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART as usize);
            assert!(
                spans.encode().unwrap().len()
                    <= super::super::bootstrap_import::MAX_PART_SPAN_INDEX_BYTES
            );
            let mut payload_objects = 0_u32;
            let mut payload_bytes = 0_u64;
            while let Some(bytes) = part.next_object_bytes().unwrap() {
                payload_objects += 1;
                payload_bytes += bytes.len() as u64;
            }
            assert_eq!(
                payload_objects,
                evidence.payload_object_root().object_count()
            );
            assert_eq!(payload_bytes, evidence.payload_object_root().total_bytes());
        }
    }

    fn install_accepted_authority(
        root: &TestRoot,
        prepared: &InactiveBootstrapPreparedPublication,
        workspace: WorkspaceId,
        endpoint: u128,
        archive_label: &str,
    ) -> (
        PathBuf,
        InactiveBootstrapVerifiedPublication,
        InactiveBootstrapAcceptedAuthority,
    ) {
        let archive = root.path().join(archive_label);
        let binding = orchestration_binding(prepared, endpoint);
        let verified = publish_install_verify_inactive_bootstrap(
            prepared,
            ObjectStore::open(&archive, workspace).unwrap(),
            binding,
        )
        .unwrap();
        let authority = reopen_inactive_bootstrap_accepted_authority(
            &verified,
            ObjectStore::open(&archive, workspace).unwrap(),
        )
        .unwrap();
        (archive, verified, authority)
    }

    fn forced_authority_changing_multipart(
        label: &str,
    ) -> (TestRoot, InactiveBootstrapPreparedPublication, WorkspaceId) {
        NEXT_BOOTSTRAP_PART_OPERATION_LIMIT.with(|limit| limit.set(Some(1)));
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            label,
            &[
                (
                    "pages/a-first.md",
                    "title:: First Authority\n\n- [[Second Authority]]\n",
                ),
                (
                    "pages/z-second.md",
                    "title:: Second Authority\n\n- [[First Authority]] distinct reference\n",
                ),
            ],
        );
        assert!(prepared.aggregate().parts().len() > 2);
        let mut second_part = prepared.open_part(1).unwrap();
        let mut second_effect = None;
        while let Some(bytes) = second_part.next_object_bytes().unwrap() {
            let object = OperationObject::decode(&bytes).unwrap();
            if object.kind() == ObjectKind::SemanticEffect {
                second_effect = Some(SemanticEffect::decode(object.payload()).unwrap());
            }
        }
        let second_effect = second_effect.unwrap();
        assert_eq!(second_effect.pages().len(), 1);
        let second_page = second_effect.pages()[0].after.as_ref().unwrap();
        assert_eq!(second_page.path().unwrap().as_str(), "pages/z-second.md");
        assert_eq!(second_page.name().as_str(), "Second Authority");
        let first_material = &prepared.engine_materials[0];
        let second_material = &prepared.engine_materials[1];
        assert_ne!(
            first_material.reference_catalog_root(),
            second_material.reference_catalog_root()
        );
        (root, prepared, workspace)
    }

    fn cold_history_leaf_path(nodes: &Path, batch_id: BatchId) -> PathBuf {
        let key = batch_id.as_uuid();
        let mut matches = fs::read_dir(nodes)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "index")
            })
            .filter(|path| {
                fs::read(path)
                    .unwrap()
                    .windows(key.as_bytes().len())
                    .filter(|window| *window == key.as_bytes())
                    .count()
                    > 1
            })
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1);
        matches.pop().unwrap()
    }

    fn assert_materialized_snapshot_matches(
        authority: &InactiveBootstrapAcceptedAuthority,
        database: &SqliteFrontier,
    ) {
        let snapshot = authority.accepted_engine().canonical_snapshot().unwrap();
        let read = database.materialized_read().unwrap();
        let live_pages = snapshot
            .pages
            .iter()
            .filter(|(_, state)| state.path().is_some())
            .count();
        assert_eq!(live_pages, snapshot.pages.len());
        for (page_id, state) in &snapshot.pages {
            let row = read.page(*page_id).unwrap().unwrap();
            assert_eq!(&row.path, state.path().unwrap());
            let expected_blocks = snapshot
                .memberships
                .iter()
                .filter(|membership| membership.page_id == *page_id)
                .count();
            assert_eq!(
                read.blocks_on_page(*page_id, MAX_MATERIALIZATION_QUERY_ROWS)
                    .unwrap()
                    .len(),
                expected_blocks
            );
        }
    }

    #[derive(Clone, Copy)]
    struct DifferentialInitialDocument {
        path: &'static str,
        name: &'static str,
        content: &'static str,
        preamble: Option<&'static str>,
    }

    #[derive(Clone, Copy)]
    struct DifferentialCurrentDocument {
        path: &'static str,
        bytes: &'static str,
        graph_name: &'static str,
        accepted_name: &'static str,
        kind: ManagedTextKind,
    }

    struct ExternalSemanticDifferentialCase {
        label: &'static str,
        initial_config: Option<&'static str>,
        current_config: Option<&'static str>,
        initial: Vec<DifferentialInitialDocument>,
        current: Vec<DifferentialCurrentDocument>,
        affected: Vec<&'static str>,
        initial_uuid: Option<LogseqUuid>,
        retained_initial: Vec<(&'static str, usize)>,
        unchanged_bytes: Vec<&'static str>,
        collision: bool,
    }

    fn graph_kind(kind: crate::PageKind) -> ManagedTextKind {
        match kind {
            crate::PageKind::Page => ManagedTextKind::Page,
            crate::PageKind::Journal => ManagedTextKind::Journal,
        }
    }

    fn flatten_graph_blocks(
        blocks: &[crate::BlockDto],
        parent: Option<usize>,
        flattened: &mut Vec<(Option<usize>, String)>,
    ) {
        for block in blocks {
            let index = flattened.len();
            flattened.push((parent, block.raw.clone()));
            flatten_graph_blocks(&block.children, Some(index), flattened);
        }
    }

    fn assert_external_semantic_observation(
        label: &str,
        graph: &Graph,
        engine: &ShardedHotEngine,
        database: &SqliteFrontier,
        expected: DifferentialCurrentDocument,
        bootstrap: bool,
    ) -> PageId {
        let graph_page = graph.load_by_path(expected.path).unwrap().unwrap();
        assert_eq!(graph_page.name, expected.graph_name, "{label}: Graph name");
        assert_eq!(
            graph_kind(graph_page.kind),
            expected.kind,
            "{label}: Graph kind"
        );
        let accepted_name = if bootstrap {
            expected.graph_name
        } else {
            expected.accepted_name
        };
        let snapshot = engine.canonical_snapshot().unwrap();
        let page_id = snapshot
            .pages
            .iter()
            .find_map(|(page_id, state)| {
                state
                    .path()
                    .is_some_and(|path| path.as_str() == expected.path)
                    .then_some(*page_id)
            })
            .unwrap_or_else(|| panic!("{label}: accepted engine has no {}", expected.path));
        let materialized = engine.materialize_page(page_id).unwrap();
        assert_eq!(
            materialized.name.as_str(),
            accepted_name,
            "{label}: engine name"
        );
        assert_eq!(materialized.kind, expected.kind, "{label}: engine kind");
        assert_eq!(
            materialized.preamble, graph_page.pre_block,
            "{label}: engine/Graph preamble"
        );

        let mut graph_blocks = Vec::new();
        flatten_graph_blocks(&graph_page.blocks, None, &mut graph_blocks);
        let engine_indexes = materialized
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.block_id, index))
            .collect::<BTreeMap<_, _>>();
        let engine_blocks = materialized
            .blocks
            .iter()
            .map(|block| {
                (
                    block.parent.map(|parent| engine_indexes[&parent]),
                    block.content.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(engine_blocks, graph_blocks, "{label}: engine/Graph tree");

        let read = database.materialized_read().unwrap();
        let sqlite_page = read.page(page_id).unwrap().unwrap();
        assert_eq!(sqlite_page.name, accepted_name, "{label}: SQLite name");
        assert_eq!(sqlite_page.kind, expected.kind, "{label}: SQLite kind");
        assert_eq!(
            sqlite_page.preamble, graph_page.pre_block,
            "{label}: SQLite/Graph preamble"
        );
        let by_name = read
            .pages_by_name_key_and_kind(
                &LogicalPageName::parse(accepted_name)
                    .unwrap()
                    .canonical_key(),
                expected.kind,
                2,
            )
            .unwrap();
        assert_eq!(by_name.len(), 1, "{label}: SQLite logical-name owner");
        assert_eq!(by_name[0].page_id, page_id);
        let sqlite_blocks = read
            .blocks_on_page(page_id, MAX_MATERIALIZATION_QUERY_ROWS)
            .unwrap();
        let sqlite_indexes = sqlite_blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.block_id, index))
            .collect::<BTreeMap<_, _>>();
        let sqlite_tree = sqlite_blocks
            .iter()
            .map(|block| {
                (
                    block.parent.map(|parent| sqlite_indexes[&parent]),
                    block.content.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(sqlite_tree, graph_blocks, "{label}: SQLite/Graph tree");
        page_id
    }

    fn write_differential_current(
        graph_root: &Path,
        config: Option<&str>,
        current: &[DifferentialCurrentDocument],
        affected: &[&str],
    ) {
        let config_path = graph_root.join("logseq/config.edn");
        match config {
            Some(config) => {
                fs::create_dir_all(config_path.parent().unwrap()).unwrap();
                fs::write(&config_path, config).unwrap();
            }
            None if config_path.exists() => fs::remove_file(&config_path).unwrap(),
            None => {}
        }
        for path in affected {
            if !current.iter().any(|document| document.path == *path) {
                let target = graph_root.join(path);
                if target.exists() {
                    fs::remove_file(target).unwrap();
                }
            }
        }
        for document in current {
            let target = graph_root.join(document.path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, document.bytes).unwrap();
        }
    }

    fn run_external_semantic_differential(case: ExternalSemanticDifferentialCase, seed: u128) {
        let initial_paths = case
            .initial
            .iter()
            .map(|document| document.path)
            .collect::<Vec<_>>();
        let initial_names = case
            .initial
            .iter()
            .map(|document| document.name)
            .collect::<Vec<_>>();
        let initial_contents = case
            .initial
            .iter()
            .map(|document| document.content)
            .collect::<Vec<_>>();
        let initial_preambles = case
            .initial
            .iter()
            .map(|document| document.preamble)
            .collect::<Vec<_>>();
        assert!(
            initial_preambles.iter().all(Option::is_some)
                || initial_preambles.iter().all(Option::is_none),
            "{}: SnapshotFixture requires uniform initial preamble presence",
            case.label
        );
        let preambles = initial_preambles.iter().all(Option::is_some).then(|| {
            initial_preambles
                .iter()
                .map(|preamble| preamble.unwrap())
                .collect::<Vec<_>>()
        });
        let fixture = SnapshotFixture::new_with_initial_uuid_and_config(
            case.label,
            &initial_paths,
            case.initial_uuid,
            case.initial_config,
            Some(&initial_names),
            Some(&initial_contents),
            preambles.as_deref(),
        );
        let retained = case
            .retained_initial
            .iter()
            .map(|(path, index)| (*path, fixture.intents[*index].page_id()))
            .collect::<Vec<_>>();
        let unchanged = case
            .unchanged_bytes
            .iter()
            .map(|path| (*path, fs::read(fixture.graph_root.join(path)).unwrap()))
            .collect::<Vec<_>>();
        write_differential_current(
            &fixture.graph_root,
            case.current_config,
            &case.current,
            &case.affected,
        );
        let mut fixture = fixture.reopen_after_config_change();

        let bootstrap_files = case
            .current
            .iter()
            .map(|document| (document.path, document.bytes))
            .collect::<Vec<_>>();
        if case.collision {
            let collision_root = TestRoot::new(&format!("{}-bootstrap-collision", case.label));
            write_differential_current(
                &collision_root.path().join("graph"),
                case.current_config,
                &case.current,
                &case.affected,
            );
            let graph = Graph::open(&collision_root.path().join("graph"));
            for document in &case.current {
                let page = graph.load_by_path(document.path).unwrap().unwrap();
                assert_eq!(page.name, document.graph_name);
            }
            let capture_scratch = collision_root.path().join("capture");
            let preparation_scratch = collision_root.path().join("preparation");
            fs::create_dir(&capture_scratch).unwrap();
            fs::create_dir(&preparation_scratch).unwrap();
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a01));
            let prepared = prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"inactive-streaming-bootstrap-test"),
                DocumentId::from_uuid(Uuid::from_u128(0x5a02)),
                ReferenceCatalogPolicyV1::default(),
                &target_catalog(&collision_root.path().join("archive"), workspace),
                &preparation_scratch,
            )
            .unwrap();
            let mut captured = prepared.source_capture().entries_cursor().unwrap();
            let mut captured_names = Vec::new();
            while let Some(entry) = captured.next().unwrap() {
                captured_names.push((
                    entry.path().as_str().to_owned(),
                    entry.logical_name().to_owned(),
                ));
            }
            assert_eq!(
                captured_names,
                vec![
                    ("pages/first.md".into(), "Shared Explicit".into()),
                    ("pages/second.md".into(), "Shared Explicit".into()),
                ]
            );
            let plan = fixture.plan(&case.affected);
            assert_eq!(plan.status(), ImportPlanStatus::Blocked, "{plan:?}");
            assert!(plan
                .blocks()
                .iter()
                .any(|block| block.reason == ImportBlockReason::ConflictingLocalTail));
            return;
        }

        let (bootstrap_root, prepared, workspace) = prepare_streaming_bootstrap_with_config(
            &format!("{}-bootstrap", case.label),
            case.current_config,
            &bootstrap_files,
        );
        let (_, _, bootstrap_authority) = install_accepted_authority(
            &bootstrap_root,
            &prepared,
            workspace,
            0x7900 + seed,
            "archive",
        );
        let bootstrap_runtime =
            ApplicationRuntimeRoot::open_for_test(&bootstrap_root.path().join("bootstrap-runtime"))
                .unwrap();
        let (bootstrap_sqlite, _) = SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &bootstrap_root.path().join("bootstrap.sqlite"),
            &bootstrap_runtime,
            &bootstrap_authority,
        )
        .unwrap();
        let bootstrap_graph = Graph::open(&bootstrap_root.path().join("graph"));
        for document in case.current.iter().copied() {
            assert_external_semantic_observation(
                case.label,
                &bootstrap_graph,
                bootstrap_authority.accepted_engine(),
                &bootstrap_sqlite.database,
                document,
                true,
            );
        }

        let plan = fixture.plan(&case.affected);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile, "{plan:?}");
        fixture.apply_external_plan(plan, 0x7a00 + seed);
        loop {
            let Some(work) = fixture
                .engine
                .projection_work_index()
                .unwrap()
                .next()
                .unwrap()
            else {
                break;
            };
            execute_manifested_projection_work(
                &fixture.graph,
                &fixture.receipts,
                &mut fixture.engine,
                &work,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{}: projection failed: {error:?}; work={work:#?}",
                    case.label
                )
            });
        }
        for document in &case.current {
            if case.affected.contains(&document.path) {
                assert_eq!(
                    fs::read(fixture.graph_root.join(document.path)).unwrap(),
                    document.bytes.as_bytes(),
                    "{}: projection changed accepted source bytes",
                    case.label
                );
            }
        }
        for (path, bytes) in unchanged {
            assert_eq!(
                fs::read(fixture.graph_root.join(path)).unwrap(),
                bytes,
                "{}: unrelated referrer bytes changed",
                case.label
            );
        }

        let fixture = fixture.reopen_after_config_change();
        let runtime =
            ApplicationRuntimeRoot::open_for_test(&fixture._root.path().join("steady-runtime"))
                .unwrap();
        let claim = ProjectionClaim::current(
            fixture.engine.workspace_id(),
            fixture.engine.lineage_digest(),
        );
        let source =
            RebuildSource::new(&fixture.engine, fixture.engine.archive_store().unwrap()).unwrap();
        let sqlite_path = fixture._root.path().join("steady.sqlite");
        let opened =
            SqliteFrontier::open_or_rebuild(&sqlite_path, &runtime, claim, source).unwrap();
        for document in case.current.iter().copied() {
            let page_id = assert_external_semantic_observation(
                case.label,
                &Graph::open(&fixture.graph_root),
                &fixture.engine,
                &opened.database,
                document,
                false,
            );
            if let Some((_, retained_page_id)) =
                retained.iter().find(|(path, _)| *path == document.path)
            {
                assert_eq!(page_id, *retained_page_id, "{}: PageId changed", case.label);
            }
        }
        drop(opened);
        let reopened = SqliteFrontier::open_or_rebuild(
            &sqlite_path,
            &runtime,
            claim,
            RebuildSource::new(&fixture.engine, fixture.engine.archive_store().unwrap()).unwrap(),
        )
        .unwrap();
        for document in case.current.iter().copied() {
            assert_external_semantic_observation(
                case.label,
                &Graph::open(&fixture.graph_root),
                &fixture.engine,
                &reopened.database,
                document,
                false,
            );
        }
        drop(reopened);
        let reimport = fixture.plan(&case.affected);
        assert_eq!(
            reimport.status(),
            ImportPlanStatus::Noop,
            "{}: projection/reimport oscillated: {reimport:#?}",
            case.label,
        );
    }

    #[test]
    fn external_document_semantic_cases_share_bootstrap_steady_sqlite_and_restart_oracles() {
        const LEGACY: &str = "{:file/name-format :legacy}\n";
        const TRIPLE: &str = "{:file/name-format :triple-lowbar}\n";
        const JOURNAL_OLD: &str = "{:journal/file-name-format \"dd-MM-yyyy\"\n\
             :journal/page-title-format \"yyyy-MM-dd\"}\n";
        const JOURNAL_NEW: &str = "{:journal/file-name-format \"dd-MM-yyyy\"\n\
             :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n";
        const NESTED: &str = r#"{:pages-directory "notes"
            :journals-directory "diary"
            :file/name-format :triple-lowbar
            :journal/file-name-format "dd-MM-yyyy"
            :journal/page-title-format "yyyy-MM-dd"}"#;
        let seed = DifferentialInitialDocument {
            path: "pages/seed.md",
            name: "Seed",
            content: "seed",
            preamble: None,
        };
        let cases = vec![
            ExternalSemanticDifferentialCase {
                label: "matrix-new-markdown-title",
                initial_config: None,
                current_config: None,
                initial: vec![seed],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/physical.md",
                    bytes: "title:: Logical Title\n\n- external\n",
                    graph_name: "Logical Title",
                    accepted_name: "Logical Title",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/physical.md"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-org-lower",
                initial_config: None,
                current_config: None,
                initial: vec![seed],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/lower.org",
                    bytes: "#+title: Lower Title\n\n* external\n",
                    graph_name: "Lower Title",
                    accepted_name: "Lower Title",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/lower.org"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-org-upper",
                initial_config: None,
                current_config: None,
                initial: vec![seed],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/upper.org",
                    bytes: "#+TITLE: Upper Title\n\n* external\n",
                    graph_name: "Upper Title",
                    accepted_name: "Upper Title",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/upper.org"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-org-mixed",
                initial_config: None,
                current_config: None,
                initial: vec![seed],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/mixed.org",
                    bytes: "#+TiTlE: Mixed Title\n\n* external\n",
                    graph_name: "Mixed Title",
                    accepted_name: "Mixed Title",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/mixed.org"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-org-drawer",
                initial_config: None,
                current_config: None,
                initial: vec![seed],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/drawer.org",
                    bytes: ":PROPERTIES:\n:TiTlE: Drawer Title\n:END:\n\n* external\n",
                    graph_name: "Drawer Title",
                    accepted_name: "Drawer Title",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/drawer.org"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-exact-title-add",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/physical.md",
                    name: "physical",
                    content: "body",
                    preamble: None,
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/physical.md",
                    bytes: "title:: Added Logical\n\n- body\n",
                    graph_name: "Added Logical",
                    accepted_name: "Added Logical",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/physical.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/physical.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-exact-title-change",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/physical.md",
                    name: "Old Logical",
                    content: "body",
                    preamble: Some("title:: Old Logical"),
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/physical.md",
                    bytes: "title:: New Logical\n\n- body\n",
                    graph_name: "New Logical",
                    accepted_name: "New Logical",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/physical.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/physical.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-exact-title-removal",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/physical.md",
                    name: "Old Logical",
                    content: "body",
                    preamble: Some("title:: Old Logical"),
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/physical.md",
                    bytes: "- body\n",
                    graph_name: "physical",
                    accepted_name: "physical",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/physical.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/physical.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-formatting-only-title",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/physical.md",
                    name: "Logical",
                    content: "body",
                    preamble: Some("title:: Logical"),
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/physical.md",
                    bytes: "title::   Logical  \nsubtitle:: retained\n\n- body\n",
                    graph_name: "Logical",
                    accepted_name: "Logical",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/physical.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/physical.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-policy-only-body-edit",
                initial_config: Some(LEGACY),
                current_config: Some(TRIPLE),
                initial: vec![DifferentialInitialDocument {
                    path: "pages/A.B.md",
                    name: "A/B",
                    content: "old",
                    preamble: None,
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/A.B.md",
                    bytes: "- changed\n",
                    graph_name: "A.B",
                    accepted_name: "A/B",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/A.B.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/A.B.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-new-after-policy-change",
                initial_config: Some(LEGACY),
                current_config: Some(TRIPLE),
                initial: vec![seed],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/Team___Planning.md",
                    bytes: "- new\n",
                    graph_name: "Team/Planning",
                    accepted_name: "Team/Planning",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/Team___Planning.md"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-explicit-date-policy-change",
                initial_config: Some(JOURNAL_OLD),
                current_config: Some(JOURNAL_NEW),
                initial: vec![DifferentialInitialDocument {
                    path: "journals/physical.md",
                    name: "2026-07-25",
                    content: "old journal",
                    preamble: Some("title:: 25-07-2026"),
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "journals/physical.md",
                    bytes: "title:: 25-07-2026\n\n- changed journal\n",
                    graph_name: "Saturday, 25-07-2026",
                    accepted_name: "2026-07-25",
                    kind: ManagedTextKind::Journal,
                }],
                affected: vec!["journals/physical.md"],
                initial_uuid: None,
                retained_initial: vec![("journals/physical.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-title-edited-anchored-move",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/old.md",
                    name: "Old",
                    content: "old body\nid:: 00000000-0000-0000-0000-000000053009",
                    preamble: Some("title:: Old"),
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/new.md",
                    bytes:
                        "title:: New\n\n- moved body\n  id:: 00000000-0000-0000-0000-000000053009\n",
                    graph_name: "New",
                    accepted_name: "New",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/old.md", "pages/new.md"],
                initial_uuid: Some(LogseqUuid::from_uuid(Uuid::from_u128(0x53009))),
                retained_initial: vec![("pages/new.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-exact-byte-unanchored-move",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/old.md",
                    name: "Old",
                    content: "exact",
                    preamble: None,
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/new.md",
                    bytes: "- exact\n",
                    graph_name: "new",
                    accepted_name: "new",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/old.md", "pages/new.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/new.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-date-nondate-nested-layouts",
                initial_config: Some(NESTED),
                current_config: Some(NESTED),
                initial: vec![DifferentialInitialDocument {
                    path: "notes/seed.md",
                    name: "Seed",
                    content: "seed",
                    preamble: None,
                }],
                current: vec![
                    DifferentialCurrentDocument {
                        path: "notes/nested/not-a-date.md",
                        bytes: "title:: 25-07-2026\n\n- date title\n",
                        graph_name: "2026-07-25",
                        accepted_name: "2026-07-25",
                        kind: ManagedTextKind::Journal,
                    },
                    DifferentialCurrentDocument {
                        path: "diary/nested/25-07-2026.md",
                        bytes: "title:: Ordinary Page\n\n- non-date title\n",
                        graph_name: "Ordinary Page",
                        accepted_name: "Ordinary Page",
                        kind: ManagedTextKind::Page,
                    },
                    DifferentialCurrentDocument {
                        path: "archive/deep/physical.org",
                        bytes: "#+TiTlE: 26-07-2026\n\n* date title\n",
                        graph_name: "2026-07-26",
                        accepted_name: "2026-07-26",
                        kind: ManagedTextKind::Journal,
                    },
                ],
                affected: vec![
                    "notes/nested/not-a-date.md",
                    "diary/nested/25-07-2026.md",
                    "archive/deep/physical.org",
                ],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-referrer-unchanged-restart",
                initial_config: None,
                current_config: None,
                initial: vec![
                    DifferentialInitialDocument {
                        path: "pages/physical.md",
                        name: "Old Logical",
                        content: "target body",
                        preamble: None,
                    },
                    DifferentialInitialDocument {
                        path: "pages/referrer.md",
                        name: "Referrer",
                        content: "see [[Old Logical]] and [[New Logical]]",
                        preamble: None,
                    },
                ],
                current: vec![
                    DifferentialCurrentDocument {
                        path: "pages/physical.md",
                        bytes: "title:: New Logical\n\n- target body\n",
                        graph_name: "New Logical",
                        accepted_name: "New Logical",
                        kind: ManagedTextKind::Page,
                    },
                    DifferentialCurrentDocument {
                        path: "pages/referrer.md",
                        bytes: "- see [[Old Logical]] and [[New Logical]]\n",
                        graph_name: "referrer",
                        accepted_name: "Referrer",
                        kind: ManagedTextKind::Page,
                    },
                ],
                affected: vec!["pages/physical.md"],
                initial_uuid: None,
                retained_initial: vec![("pages/physical.md", 0)],
                unchanged_bytes: vec!["pages/referrer.md"],
                collision: false,
            },
            ExternalSemanticDifferentialCase {
                label: "matrix-explicit-title-collision",
                initial_config: None,
                current_config: None,
                initial: vec![seed],
                current: vec![
                    DifferentialCurrentDocument {
                        path: "pages/first.md",
                        bytes: "title:: Shared Explicit\n\n- first\n",
                        graph_name: "Shared Explicit",
                        accepted_name: "Shared Explicit",
                        kind: ManagedTextKind::Page,
                    },
                    DifferentialCurrentDocument {
                        path: "pages/second.md",
                        bytes: "title:: Shared Explicit\n\n- second\n",
                        graph_name: "Shared Explicit",
                        accepted_name: "Shared Explicit",
                        kind: ManagedTextKind::Page,
                    },
                ],
                affected: vec!["pages/first.md", "pages/second.md"],
                initial_uuid: None,
                retained_initial: vec![],
                unchanged_bytes: vec![],
                collision: true,
            },
        ];
        for (index, case) in cases.into_iter().enumerate() {
            run_external_semantic_differential(case, index as u128);
        }
    }

    #[test]
    fn fresh_attack_anchored_crlf_move_projects_and_stabilizes() {
        run_external_semantic_differential(
            ExternalSemanticDifferentialCase {
                label: "fresh-attack-anchored-crlf-move",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/old.md",
                    name: "old",
                    content: "body\nid:: 00000000-0000-0000-0000-00000005300a",
                    preamble: None,
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/new.md",
                    bytes: "- body\r\n  id:: 00000000-0000-0000-0000-00000005300a\r\n",
                    graph_name: "new",
                    accepted_name: "new",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/old.md", "pages/new.md"],
                initial_uuid: Some(LogseqUuid::from_uuid(Uuid::from_u128(0x5300a))),
                retained_initial: vec![("pages/new.md", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            0x5300a,
        );
    }

    #[test]
    fn fresh_attack_anchored_crlf_markdown_to_org_move_projects_and_stabilizes() {
        run_external_semantic_differential(
            ExternalSemanticDifferentialCase {
                label: "fresh-attack-anchored-crlf-markdown-to-org-move",
                initial_config: None,
                current_config: None,
                initial: vec![DifferentialInitialDocument {
                    path: "pages/old.md",
                    name: "old",
                    content: "body\nid:: 00000000-0000-0000-0000-00000005300b",
                    preamble: None,
                }],
                current: vec![DifferentialCurrentDocument {
                    path: "pages/new.org",
                    bytes: "* body\r\n:PROPERTIES:\r\n:id: 00000000-0000-0000-0000-00000005300b\r\n:END:\r\n",
                    graph_name: "new",
                    accepted_name: "new",
                    kind: ManagedTextKind::Page,
                }],
                affected: vec!["pages/old.md", "pages/new.org"],
                initial_uuid: Some(LogseqUuid::from_uuid(Uuid::from_u128(0x5300b))),
                retained_initial: vec![("pages/new.org", 0)],
                unchanged_bytes: vec![],
                collision: false,
            },
            0x5300b,
        );
    }

    #[test]
    fn fresh_attack_move_destination_changed_after_observation_refuses_without_overwrite() {
        let fixture = SnapshotFixture::new_with_initial_uuid(
            "fresh-attack-move-destination-stale",
            &["pages/old.md"],
            Some(LogseqUuid::from_uuid(Uuid::from_u128(0x5300c))),
        );
        let destination = fixture.graph_root.join("pages/new.md");
        fs::rename(fixture.graph_root.join("pages/old.md"), &destination).unwrap();
        let plan = fixture.plan(&["pages/old.md", "pages/new.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let material = plan.into_execution_material().unwrap();
        let endpoint = fixture.engine.projection_endpoint_binding().unwrap();
        let draft = fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: material.batch_id(),
                    author_device_id: endpoint.device_id,
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(0x5300d)),
                    crdt_peer_id: CrdtPeerId::from_u64(0x5300e),
                },
                material,
            )
            .unwrap();
        let changed = b"- changed after observation\r\n".to_vec();
        fs::write(&destination, &changed).unwrap();

        assert!(matches!(
            fixture.engine.capture_external_author_transaction(
                draft,
                &fixture.graph,
                &fixture.receipts,
                endpoint,
                None,
            ),
            Err(crate::oplog::EngineError::ProjectionManifest(message))
                if message.contains("observation") && message.contains("stale")
        ));
        assert_eq!(fs::read(destination).unwrap(), changed);
    }

    #[test]
    fn inactive_bootstrap_orchestration_all_cuts_retry_fresh_and_is_idempotent() {
        let (root, prepared, workspace) = rich_orchestration_fixture("orchestration-cuts");
        let binding = orchestration_binding(&prepared, 0x6a10);
        let graph_root = root.path().join("graph");
        let graph_before = [
            "Root.md",
            "notes/arbitrary/nested/Markdown.md",
            "notes/深い/Déjà___計画.markdown",
            "diary/nested/25-07-2026.org",
            "logseq/config.edn",
        ]
        .map(|path| (path, fs::read(graph_root.join(path)).unwrap()));
        let mut captured_paths = Vec::new();
        let mut entries = prepared.source_capture().entries_cursor().unwrap();
        while let Some(entry) = entries.next().unwrap() {
            captured_paths.push((entry.path().as_str().to_owned(), entry.kind()));
        }
        assert!(captured_paths.contains(&("Root.md".into(), ManagedTextKind::Page)));
        assert!(captured_paths.contains(&(
            "notes/arbitrary/nested/Markdown.md".into(),
            ManagedTextKind::Page
        )));
        assert!(captured_paths.contains(&(
            "notes/深い/Déjà___計画.markdown".into(),
            ManagedTextKind::Page
        )));
        assert!(
            captured_paths.contains(&("diary/nested/25-07-2026.org".into(), ManagedTextKind::Page))
        );

        let cuts = [
            InactiveBootstrapOrchestrationCut::AfterSourcePublication,
            InactiveBootstrapOrchestrationCut::AfterOnePart,
            InactiveBootstrapOrchestrationCut::AfterAggregatePrefix,
            InactiveBootstrapOrchestrationCut::AfterAggregateCommit,
            InactiveBootstrapOrchestrationCut::BeforeHistoryHead,
            InactiveBootstrapOrchestrationCut::AfterHistoryHead,
        ];
        // Each cut needs its own target archive, and a bootstrap preparation
        // belongs to exactly one archive: authoring persists the catalog roots
        // its cold records bind into that archive's durable store.
        for (index, cut) in cuts.into_iter().enumerate() {
            let archive = root.path().join(format!("archive-cut-{index}"));
            let cut_prepared =
                prepare_existing_bootstrap(&root, &format!("cut-{index}"), workspace, &archive);
            let cut_binding = orchestration_binding(&cut_prepared, 0x6a10);
            INACTIVE_BOOTSTRAP_ORCHESTRATION_CUT.with(|pending| pending.set(Some(cut)));
            assert!(publish_install_verify_inactive_bootstrap(
                &cut_prepared,
                ObjectStore::open(&archive, workspace).unwrap(),
                cut_binding,
            )
            .is_err());
            if cut == InactiveBootstrapOrchestrationCut::AfterHistoryHead {
                let store = ObjectStore::open(&archive, workspace).unwrap();
                let (store, history) = store
                    .seal_history_only(cut_binding)
                    .unwrap()
                    .into_history()
                    .unwrap();
                assert_eq!(
                    history.current_bootstrap_binding().unwrap(),
                    Some(
                        BootstrapAggregateHistoryBindingV1::for_aggregate(cut_prepared.aggregate())
                            .unwrap()
                    )
                );
                drop(history);
                drop(store);
            }
            let verified = publish_install_verify_inactive_bootstrap(
                &cut_prepared,
                ObjectStore::open(&archive, workspace).unwrap(),
                cut_binding,
            )
            .unwrap();
            assert_verified_orchestration(&cut_prepared, &verified, cut_binding);
            assert!(!archive.join("projection-work").exists());
        }

        // The fixture's own preparation installs idempotently into the exact
        // archive it was authored against.
        let archive = root.path().join("archive");
        let first = publish_install_verify_inactive_bootstrap(
            &prepared,
            ObjectStore::open(&archive, workspace).unwrap(),
            binding,
        )
        .unwrap();
        let second = publish_install_verify_inactive_bootstrap(
            &prepared,
            ObjectStore::open(&archive, workspace).unwrap(),
            binding,
        )
        .unwrap();
        assert_eq!(first.instrumentation.durability_syncs, 2);
        assert_eq!(second.instrumentation.durability_syncs, 0);
        let mut first_identity = first.clone();
        first_identity.instrumentation = InactiveBootstrapOrchestrationInstrumentation::default();
        let mut second_identity = second.clone();
        second_identity.instrumentation = InactiveBootstrapOrchestrationInstrumentation::default();
        assert_eq!(first_identity, second_identity);
        for (path, bytes) in graph_before {
            assert_eq!(fs::read(graph_root.join(path)).unwrap(), bytes);
        }
        assert!(!archive.join("projection-work").exists());
    }

    #[test]
    fn inactive_bootstrap_orchestration_zero_and_multipart_are_canonical_and_bounded() {
        let (zero_root, zero, workspace) = prepare_streaming_bootstrap("orchestration-zero", &[]);
        let zero_binding = orchestration_binding(&zero, 0x6b10);
        let zero_verified = publish_install_verify_inactive_bootstrap(
            &zero,
            ObjectStore::open(&zero_root.path().join("archive"), workspace).unwrap(),
            zero_binding,
        )
        .unwrap();
        assert_verified_orchestration(&zero, &zero_verified, zero_binding);
        assert_eq!(zero_verified.part_count(), 0);
        assert_eq!(
            zero_verified.engine_binding(),
            &EngineHistoryBinding::empty()
        );

        let owned = (0..65)
            .map(|ordinal| {
                (
                    format!("pages/multipart-{ordinal:03}.md"),
                    format!("- multipart page {ordinal}\n"),
                )
            })
            .collect::<Vec<_>>();
        let files = owned
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
            .collect::<Vec<_>>();
        let (multi_root, multi, workspace) =
            prepare_streaming_bootstrap("orchestration-multipart", &files);
        assert_eq!(multi.instrumentation().page_capsules, 65);
        assert_eq!(multi.aggregate().parts().len(), 2);
        assert_prepared_bootstrap_limits(&multi);
        let expected_snapshot = multi
            .candidate()
            .accepted_engine()
            .canonical_snapshot()
            .unwrap();
        let multi_binding = orchestration_binding(&multi, 0x6b20);
        let archive = multi_root.path().join("archive");
        let verified = publish_install_verify_inactive_bootstrap(
            &multi,
            ObjectStore::open(&archive, workspace).unwrap(),
            multi_binding,
        )
        .unwrap();
        assert_verified_orchestration(&multi, &verified, multi_binding);
        assert_eq!(
            verified.instrumentation().cold_records,
            u64::from(verified.part_count())
        );
        assert_eq!(verified.instrumentation().parts, verified.part_count());
        let reopened = reopen_inactive_bootstrap_accepted_authority(
            &verified,
            ObjectStore::open(&archive, workspace).unwrap(),
        )
        .unwrap();
        assert_eq!(
            reopened.accepted_engine().canonical_snapshot().unwrap(),
            expected_snapshot
        );
    }

    #[test]
    fn inactive_bootstrap_orchestration_refuses_different_publication() {
        let (root, first, workspace) =
            rich_orchestration_fixture("orchestration-different-publication");
        let binding = orchestration_binding(&first, 0x6c10);
        let archive = root.path().join("archive");
        let installed = publish_install_verify_inactive_bootstrap(
            &first,
            ObjectStore::open(&archive, workspace).unwrap(),
            binding,
        )
        .unwrap();
        let stale_binding = ProjectionStorageBinding {
            endpoint: binding.endpoint,
            receipt_store_id: crate::oplog::ProjectionReceiptStoreId::from_capability_identity(
                b"inactive-bootstrap-stale-binding",
                b"different-receipt-store",
            ),
        };
        assert!(publish_install_verify_inactive_bootstrap(
            &first,
            ObjectStore::open(&archive, workspace).unwrap(),
            stale_binding,
        )
        .is_err());

        fs::write(
            root.path().join("graph/Root.md"),
            "title:: Root logical name\n\n- changed publication\n",
        )
        .unwrap();
        let second = prepare_existing_bootstrap(&root, "different", workspace, &archive);
        assert_ne!(
            first.aggregate().publication_id(),
            second.aggregate().publication_id()
        );
        assert!(publish_install_verify_inactive_bootstrap(
            &second,
            ObjectStore::open(&archive, workspace).unwrap(),
            binding,
        )
        .is_err());
        let store = ObjectStore::open(&archive, workspace).unwrap();
        let (store, history) = store
            .seal_history_only(binding)
            .unwrap()
            .into_history()
            .unwrap();
        assert_eq!(
            history.current_bootstrap_binding().unwrap(),
            Some(installed.bootstrap_binding())
        );
        assert_eq!(
            history.current_with_binding().unwrap().1,
            installed.history_root()
        );
        drop(history);
        drop(store);
        assert!(!archive.join("projection-work").exists());
    }

    #[test]
    fn inactive_bootstrap_orchestration_refuses_missing_and_truncated_committed_artifacts() {
        let (root, prepared, workspace) =
            rich_orchestration_fixture("orchestration-artifact-damage");
        drop(prepared);

        let missing_archive = root.path().join("archive-missing");
        let missing_prepared =
            prepare_existing_bootstrap(&root, "missing", workspace, &missing_archive);
        let missing_binding = orchestration_binding(&missing_prepared, 0x6d10);
        let part_name =
            hex_bootstrap_digest(missing_prepared.aggregate().parts()[0].part_id().as_bytes());
        publish_install_verify_inactive_bootstrap(
            &missing_prepared,
            ObjectStore::open(&missing_archive, workspace).unwrap(),
            missing_binding,
        )
        .unwrap();
        fs::remove_file(
            missing_archive
                .join("bootstrap-v1/part-spans")
                .join(&part_name),
        )
        .unwrap();
        assert!(publish_install_verify_inactive_bootstrap(
            &missing_prepared,
            ObjectStore::open(&missing_archive, workspace).unwrap(),
            missing_binding,
        )
        .is_err());

        let truncated_archive = root.path().join("archive-truncated");
        let truncated_prepared =
            prepare_existing_bootstrap(&root, "truncated", workspace, &truncated_archive);
        let truncated_binding = orchestration_binding(&truncated_prepared, 0x6d10);
        let truncated_part_name = hex_bootstrap_digest(
            truncated_prepared.aggregate().parts()[0]
                .part_id()
                .as_bytes(),
        );
        publish_install_verify_inactive_bootstrap(
            &truncated_prepared,
            ObjectStore::open(&truncated_archive, workspace).unwrap(),
            truncated_binding,
        )
        .unwrap();
        fs::write(
            truncated_archive
                .join("bootstrap-v1/parts")
                .join(&truncated_part_name),
            b"truncated",
        )
        .unwrap();
        assert!(publish_install_verify_inactive_bootstrap(
            &truncated_prepared,
            ObjectStore::open(&truncated_archive, workspace).unwrap(),
            truncated_binding,
        )
        .is_err());
        assert!(!missing_archive.join("projection-work").exists());
        assert!(!truncated_archive.join("projection-work").exists());
    }

    #[test]
    fn inactive_bootstrap_sqlite_rebuilds_zero_one_and_forced_multipart_exactly() {
        let (zero_root, zero, workspace) =
            prepare_streaming_bootstrap("bootstrap-sqlite-zero", &[]);
        let (zero_archive, _, zero_authority) =
            install_accepted_authority(&zero_root, &zero, workspace, 0x6e10, "archive");
        let zero_runtime =
            ApplicationRuntimeRoot::open_for_test(&zero_root.path().join("runtime")).unwrap();
        let (zero_opened, zero_proof) = SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &zero_root.path().join("zero.sqlite"),
            &zero_runtime,
            &zero_authority,
        )
        .unwrap();
        assert_eq!(zero_proof.accepted_batch_count(), 0);
        assert_eq!(
            zero_proof.frontier_root(),
            zero_authority.binding().accepted_frontier()
        );
        assert_eq!(zero_proof.bootstrap_rebuild().bootstrap_part_reads, 0);
        assert_eq!(zero_opened.rebuild.physical_candidate_transactions, 1);
        assert_eq!(
            zero_opened.rebuild.physical_candidate_durability_barriers,
            1
        );
        assert_eq!(zero_opened.rebuild.physical_ordinary_transactions, 0);
        assert_materialized_snapshot_matches(&zero_authority, &zero_opened.database);
        assert!(!zero_archive.join("projection-work").exists());

        let (one_root, one, workspace) =
            prepare_streaming_bootstrap("bootstrap-sqlite-one", &[("pages/one.md", "- one\n")]);
        let (one_archive, _, one_authority) =
            install_accepted_authority(&one_root, &one, workspace, 0x6e20, "archive");
        let one_runtime =
            ApplicationRuntimeRoot::open_for_test(&one_root.path().join("runtime")).unwrap();
        let (one_opened, one_proof) = SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &one_root.path().join("one.sqlite"),
            &one_runtime,
            &one_authority,
        )
        .unwrap();
        assert_eq!(
            one_proof.accepted_batch_count(),
            one.aggregate().parts().len() as u64
        );
        assert_eq!(
            one_proof.claim(),
            crate::oplog::ProjectionClaim::current(
                one_authority.binding().workspace_id(),
                one_authority.binding().lineage_digest(),
            )
        );
        assert_eq!(
            one_proof.semantic_projection_digest(),
            one_opened.database.semantic_projection_digest().unwrap()
        );
        assert_eq!(
            one_proof.materialized_row_digest(),
            one_opened
                .database
                .materialized_row_digest_for_harness()
                .unwrap()
        );
        assert_eq!(
            one_proof.bootstrap_rebuild().bootstrap_part_reads,
            one.aggregate().parts().len()
        );
        assert_eq!(one_proof.bootstrap_rebuild().max_live_bootstrap_parts, 1);
        assert_materialized_snapshot_matches(&one_authority, &one_opened.database);
        assert!(!one_archive.join("projection-work").exists());

        let mut multipart_source = String::new();
        for ordinal in 0..MAX_OPERATIONS_PER_BOOTSTRAP_PART {
            multipart_source.push_str(&format!("- multipart {ordinal}\n"));
        }
        let (multi_root, multi, workspace) = prepare_streaming_bootstrap(
            "bootstrap-sqlite-multipart",
            &[("pages/multipart.md", &multipart_source)],
        );
        assert!(multi.aggregate().parts().len() > 1);
        let graph_bytes = fs::read(multi_root.path().join("graph/pages/multipart.md")).unwrap();
        let (multi_archive, _, multi_authority) =
            install_accepted_authority(&multi_root, &multi, workspace, 0x6e30, "archive");
        let multi_runtime =
            ApplicationRuntimeRoot::open_for_test(&multi_root.path().join("runtime")).unwrap();
        let (multi_opened, multi_proof) = SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &multi_root.path().join("multi.sqlite"),
            &multi_runtime,
            &multi_authority,
        )
        .unwrap();
        assert_eq!(
            multi_proof.accepted_batch_count(),
            multi.aggregate().parts().len() as u64
        );
        assert_eq!(
            multi_proof.frontier_root(),
            multi_authority.binding().accepted_frontier()
        );
        assert_eq!(
            multi_proof.semantic_projection_digest(),
            multi_opened.database.semantic_projection_digest().unwrap()
        );
        assert_eq!(
            multi_proof.materialized_row_digest(),
            multi_opened
                .database
                .materialized_row_digest_for_harness()
                .unwrap()
        );
        assert_eq!(
            multi_proof.bootstrap_rebuild().bootstrap_part_reads,
            multi.aggregate().parts().len()
        );
        assert_eq!(
            multi_proof.bootstrap_rebuild().bootstrap_object_reads,
            multi
                .aggregate()
                .parts()
                .iter()
                .map(|part| { part.evidence().payload_object_root().object_count() as usize })
                .sum::<usize>()
        );
        assert_eq!(multi_opened.rebuild.accepted_events_validated, 2);
        assert_eq!(multi_opened.rebuild.accepted_events_applied, 2);
        assert_eq!(multi_opened.rebuild.max_live_events, 1);
        assert_eq!(multi_opened.rebuild.max_live_evidence_records, 1);
        assert_eq!(multi_opened.rebuild.accepted_root_authentications, 2);
        assert_eq!(multi_opened.rebuild.exact_catalog_loads, 2);
        assert_eq!(
            multi_opened.rebuild.reference_coverage_inductive_checks,
            multi.aggregate().parts().len()
        );
        assert_eq!(multi_opened.rebuild.reference_coverage_full_scans, 1);
        assert_eq!(multi_opened.rebuild.final_semantic_equivalence_proofs, 1);
        assert_eq!(multi_opened.rebuild.final_row_digest_equivalence_proofs, 1);
        assert_eq!(multi_opened.rebuild.physical_candidate_transactions, 1);
        assert_eq!(
            multi_opened.rebuild.physical_candidate_durability_barriers,
            1
        );
        assert_eq!(multi_opened.rebuild.physical_ordinary_transactions, 0);
        assert_eq!(
            multi_opened.rebuild.physical_ordinary_durability_barriers,
            0
        );
        assert!(multi_opened.rebuild.cleanup_fts_rowids <= multi_opened.rebuild.cleanup_owned_rows);
        assert_eq!(multi_proof.bootstrap_rebuild().max_live_bootstrap_parts, 1);
        assert_materialized_snapshot_matches(&multi_authority, &multi_opened.database);
        assert_eq!(
            fs::read(multi_root.path().join("graph/pages/multipart.md")).unwrap(),
            graph_bytes
        );
        // Destruction is part of the normal-stack regression: both retained
        // detached candidates and the large materialized projection die here.
        drop(multi_opened);
        drop(multi_authority);
        drop(multi);
        assert!(!multi_archive.join("projection-work").exists());
    }

    #[test]
    #[ignore = "release-only 1k/2k inactive-bootstrap SQLite phase receipt"]
    fn inactive_bootstrap_sqlite_candidate_1k_2k_release_phase_receipt() {
        assert!(
            !cfg!(debug_assertions),
            "run this receipt with cargo test -p tine-core --release inactive_bootstrap_sqlite_candidate_1k_2k_release_phase_receipt -- --ignored --nocapture"
        );
        for page_count in [1_000_usize, 2_000] {
            let fixture_started = Instant::now();
            let owned = (0..page_count)
                .map(|index| {
                    (
                        format!("pages/sqlite-candidate-{index:04}.md"),
                        format!("- candidate receipt {index:04}\n"),
                    )
                })
                .collect::<Vec<_>>();
            let files = owned
                .iter()
                .map(|(path, contents)| (path.as_str(), contents.as_str()))
                .collect::<Vec<_>>();
            let label = format!("bootstrap-sqlite-candidate-{page_count}");
            let (root, prepared, workspace) = prepare_streaming_bootstrap(&label, &files);
            let fixture_elapsed = fixture_started.elapsed();

            let authority_started = Instant::now();
            let (_, _, authority) = install_accepted_authority(
                &root,
                &prepared,
                workspace,
                0x6e40 + page_count as u128,
                "archive",
            );
            let authority_elapsed = authority_started.elapsed();
            let runtime =
                ApplicationRuntimeRoot::open_for_test(&root.path().join("runtime")).unwrap();

            let sqlite_started = Instant::now();
            let (opened, proof) = SqliteFrontier::open_or_rebuild_inactive_bootstrap(
                &root.path().join("candidate.sqlite"),
                &runtime,
                &authority,
            )
            .unwrap();
            let sqlite_elapsed = sqlite_started.elapsed();
            let parts = prepared.aggregate().parts().len();
            assert_eq!(opened.rebuild.accepted_events_applied, parts);
            assert_eq!(opened.rebuild.physical_candidate_transactions, 1);
            assert_eq!(opened.rebuild.physical_candidate_durability_barriers, 1);
            assert_eq!(opened.rebuild.physical_ordinary_transactions, 0);
            assert_eq!(proof.bootstrap_rebuild().bootstrap_part_reads, parts);
            assert_materialized_snapshot_matches(&authority, &opened.database);
            eprintln!(
                "bootstrap_sqlite_candidate_release pages={page_count} parts={parts} fixture_ms={} authority_ms={} sqlite_ms={} terminal_row_proof_us={} candidate_transactions={} candidate_durability_barriers={} semantic_proofs={} row_proofs={}",
                fixture_elapsed.as_millis(),
                authority_elapsed.as_millis(),
                sqlite_elapsed.as_millis(),
                opened.rebuild.final_row_digest_proof_micros,
                opened.rebuild.physical_candidate_transactions,
                opened.rebuild.physical_candidate_durability_barriers,
                opened.rebuild.final_semantic_equivalence_proofs,
                opened.rebuild.final_row_digest_equivalence_proofs,
            );
        }
    }

    #[test]
    fn ordinary_rebuild_source_does_not_fall_back_to_bootstrap_parts() {
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            "ordinary-source-bootstrap-isolation",
            &[("pages/only-bootstrap.md", "- isolated\n")],
        );
        let (_, _, authority) =
            install_accepted_authority(&root, &prepared, workspace, 0x6f10, "archive");
        let runtime = ApplicationRuntimeRoot::open_for_test(&root.path().join("runtime")).unwrap();
        let ordinary = RebuildSource::new(authority.accepted_engine(), authority.store()).unwrap();
        assert!(SqliteFrontier::open_or_rebuild(
            &root.path().join("ordinary.sqlite"),
            &runtime,
            crate::oplog::ProjectionClaim::current(
                authority.binding().workspace_id(),
                authority.binding().lineage_digest(),
            ),
            ordinary,
        )
        .is_err());
        SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &root.path().join("bootstrap.sqlite"),
            &runtime,
            &authority,
        )
        .unwrap();
    }

    #[test]
    fn inactive_bootstrap_authority_validates_each_changing_multipart_cold_record() {
        let (root, prepared, workspace) =
            forced_authority_changing_multipart("bootstrap-authority-changing-multipart");
        let (archive, verified, authority) =
            install_accepted_authority(&root, &prepared, workspace, 0x6f20, "archive");
        assert_eq!(
            authority.binding().accepted_frontier(),
            &prepared.candidate().accepted_frontier_root().unwrap()
        );
        drop(authority);

        let nodes = archive
            .join("engine-history")
            .join(verified.storage_binding.endpoint.endpoint_id.to_string())
            .join("nodes");
        let intermediate =
            cold_history_leaf_path(&nodes, prepared.aggregate().parts()[0].batch_id());
        let terminal = cold_history_leaf_path(
            &nodes,
            prepared.aggregate().parts().last().unwrap().batch_id(),
        );
        fs::write(&intermediate, fs::read(&terminal).unwrap()).unwrap();
        assert_authority_reopen_rejected(&verified, &archive, workspace);
    }

    fn assert_authority_reopen_rejected(
        verified: &InactiveBootstrapVerifiedPublication,
        archive: &Path,
        workspace: WorkspaceId,
    ) {
        assert!(reopen_inactive_bootstrap_accepted_authority(
            verified,
            ObjectStore::open(archive, workspace).unwrap(),
        )
        .is_err());
    }

    #[test]
    fn inactive_bootstrap_authority_rejects_substituted_bindings() {
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            "bootstrap-authority-bindings",
            &[("pages/bindings.md", "- bindings\n")],
        );
        let (archive, verified, authority) =
            install_accepted_authority(&root, &prepared, workspace, 0x7010, "archive");
        drop(authority);

        let mut wrong = verified.clone();
        wrong.workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(0x7011));
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        let mut wrong = verified.clone();
        wrong.lineage_digest = LineageDigest::of(b"substituted-lineage");
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        let mut wrong = verified.clone();
        wrong.publication_id =
            super::super::bootstrap_import::BootstrapPublicationIdV1::from_bytes([0x70; 32]);
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        let mut wrong = verified.clone();
        wrong.aggregate_digest =
            super::super::bootstrap_import::BootstrapAggregateDigestV1::from_bytes([0x71; 32]);
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        let mut wrong = verified.clone();
        wrong.history_root = ContentDigest::of(b"substituted-history-root");
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        let mut wrong = verified.clone();
        wrong.accepted_frontier = AcceptedFrontierRoot::empty();
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        let mut wrong = verified.clone();
        wrong.engine_binding = EngineHistoryBinding::empty();
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        // A second archive requires its own preparation: a bootstrap belongs to
        // exactly one archive's durable reference catalog.
        let other_archive = root.path().join("other-archive");
        let first_other_prepared =
            prepare_existing_bootstrap(&root, "other-archive", workspace, &other_archive);
        let other_aggregate = first_other_prepared.aggregate_bytes().unwrap();
        let other_commit = first_other_prepared.commit_bytes().unwrap();
        drop(first_other_prepared);
        let other_prepared =
            prepare_existing_bootstrap(&root, "other-archive", workspace, &other_archive);
        assert_eq!(other_prepared.aggregate_bytes().unwrap(), other_aggregate);
        assert_eq!(other_prepared.commit_bytes().unwrap(), other_commit);
        let (_, other_verified, other_authority) =
            install_accepted_authority(&root, &other_prepared, workspace, 0x7020, "other-archive");
        drop(other_authority);
        let mut wrong = verified.clone();
        wrong.archive_identity = other_verified.archive_identity;
        assert_authority_reopen_rejected(&wrong, &archive, workspace);

        assert!(reopen_inactive_bootstrap_accepted_authority(
            &verified,
            ObjectStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(0x7021))).unwrap(),
        )
        .is_err());
    }

    #[test]
    fn inactive_bootstrap_authority_rejects_missing_objects_and_substituted_cold_history() {
        let (object_root, object_prepared, workspace) = prepare_streaming_bootstrap(
            "bootstrap-authority-missing-object",
            &[("pages/object.md", "- object\n")],
        );
        let (object_archive, object_verified, object_authority) = install_accepted_authority(
            &object_root,
            &object_prepared,
            workspace,
            0x7110,
            "archive",
        );
        drop(object_authority);
        let mut part = object_prepared.open_part(0).unwrap();
        let manifest = OperationBatch::decode(part.manifest_bytes()).unwrap();
        assert!(part.next_object_bytes().unwrap().is_some());
        let object_name = format!("{}.object", manifest.required_objects()[0].content_digest());
        fs::remove_file(
            object_archive
                .join("bootstrap-v1/objects")
                .join(object_name),
        )
        .unwrap();
        assert_authority_reopen_rejected(&object_verified, &object_archive, workspace);

        let (history_root, history_prepared, workspace) = prepare_streaming_bootstrap(
            "bootstrap-authority-cold-history",
            &[("pages/history.md", "- history\n")],
        );
        let (history_archive, history_verified, history_authority) = install_accepted_authority(
            &history_root,
            &history_prepared,
            workspace,
            0x7120,
            "archive",
        );
        drop(history_authority);
        let nodes = history_archive
            .join("engine-history")
            .join(
                history_verified
                    .storage_binding
                    .endpoint
                    .endpoint_id
                    .to_string(),
            )
            .join("nodes");
        let mut indexes = fs::read_dir(&nodes)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "index")
            })
            .collect::<Vec<_>>();
        indexes.sort();
        assert!(indexes.len() > 1);
        let substituted = fs::read(&indexes[1]).unwrap();
        fs::write(&indexes[0], substituted).unwrap();
        assert_authority_reopen_rejected(&history_verified, &history_archive, workspace);
    }

    #[test]
    fn inactive_bootstrap_authority_rejects_corrupt_part() {
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            "bootstrap-authority-corrupt-part",
            &[("pages/part.md", "- part\n")],
        );
        let (archive, verified, authority) =
            install_accepted_authority(&root, &prepared, workspace, 0x7118, "archive");
        drop(authority);
        let part_name = hex_bootstrap_digest(prepared.aggregate().parts()[0].part_id().as_bytes());
        fs::write(
            archive.join("bootstrap-v1/parts").join(part_name),
            b"corrupt bootstrap part",
        )
        .unwrap();
        assert_authority_reopen_rejected(&verified, &archive, workspace);
    }

    #[test]
    fn inactive_bootstrap_sqlite_rebuild_is_deterministic_and_retryable() {
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            "bootstrap-sqlite-recovery",
            &[("pages/recovery.md", "- recovery\n")],
        );
        let (_, _, authority) =
            install_accepted_authority(&root, &prepared, workspace, 0x7210, "archive");
        let runtime = ApplicationRuntimeRoot::open_for_test(&root.path().join("runtime")).unwrap();
        let path = root.path().join("frontier.sqlite");
        let (opened, expected) =
            SqliteFrontier::open_or_rebuild_inactive_bootstrap(&path, &runtime, &authority)
                .unwrap();
        drop(opened);

        let (existing, existing_proof) =
            SqliteFrontier::open_or_rebuild_inactive_bootstrap(&path, &runtime, &authority)
                .unwrap();
        assert_eq!(existing_proof, expected);
        assert!(matches!(
            existing.recovery,
            ProjectionRecovery::RebuiltPreservingEvidence { .. }
        ));
        assert_eq!(
            existing_proof.bootstrap_rebuild().bootstrap_part_reads,
            prepared.aggregate().parts().len()
        );
        drop(existing);

        fs::remove_file(&path).unwrap();
        let auth_path = PathBuf::from(format!("{}-auth", path.display()));
        fs::remove_file(&auth_path).unwrap();
        let (deleted, deleted_proof) =
            SqliteFrontier::open_or_rebuild_inactive_bootstrap(&path, &runtime, &authority)
                .unwrap();
        assert_eq!(deleted_proof, expected);
        drop(deleted);

        fs::write(&path, b"corrupt sqlite projection").unwrap();
        let (corrupt, corrupt_proof) =
            SqliteFrontier::open_or_rebuild_inactive_bootstrap(&path, &runtime, &authority)
                .unwrap();
        assert_eq!(corrupt_proof, expected);
        drop(corrupt);

        let interrupted_path = root.path().join("interrupted.sqlite");
        crate::oplog::sqlite::fail_next_apply_during_materialization_for_harness();
        assert!(SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &interrupted_path,
            &runtime,
            &authority,
        )
        .is_err());
        assert!(!interrupted_path.exists());
        assert!(!PathBuf::from(format!("{}-auth", interrupted_path.display())).exists());
        let (_retried, retried_proof) = SqliteFrontier::open_or_rebuild_inactive_bootstrap(
            &interrupted_path,
            &runtime,
            &authority,
        )
        .unwrap();
        assert_eq!(retried_proof, expected);
        assert_eq!(
            retried_proof.bootstrap_rebuild().bootstrap_part_reads,
            prepared.aggregate().parts().len()
        );
    }

    #[test]
    fn inactive_bootstrap_sqlite_never_proves_retained_internal_rows() {
        let mut source = "- [[multipart]] retained-reference\n".to_string();
        for ordinal in 1..MAX_OPERATIONS_PER_BOOTSTRAP_PART {
            source.push_str(&format!("- retained-row {ordinal}\n"));
        }
        let (root, prepared, workspace) = prepare_streaming_bootstrap(
            "bootstrap-sqlite-internal-corruption",
            &[("pages/multipart.md", &source)],
        );
        assert!(prepared.aggregate().parts().len() > 1);
        let (_, _, authority) =
            install_accepted_authority(&root, &prepared, workspace, 0x7220, "archive");
        let runtime = ApplicationRuntimeRoot::open_for_test(&root.path().join("runtime")).unwrap();
        let path = root.path().join("frontier.sqlite");
        let graph_before = [(
            "pages/multipart.md",
            fs::read(root.path().join("graph/pages/multipart.md")).unwrap(),
        )];
        let (clean, expected) =
            SqliteFrontier::open_or_rebuild_inactive_bootstrap(&path, &runtime, &authority)
                .unwrap();
        let part_count = prepared.aggregate().parts().len();
        assert!(matches!(
            clean.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches }
                if applied_batches == part_count
        ));
        let claim = expected.claim();
        drop(clean);

        let corruptions = [
            (
                "delete nonterminal accepted row",
                "DELETE FROM applied_batches WHERE sequence = 1",
            ),
            (
                "change nonterminal accepted row",
                "UPDATE applied_batches
                 SET semantic_effect = CAST(semantic_effect || x'00' AS BLOB)
                 WHERE sequence = 1",
            ),
            (
                "insert extra accepted row",
                "INSERT INTO applied_batches
                 SELECT 1000, randomblob(16), manifest_digest, semantic_effect,
                        semantic_effect_digest, dependency_frontier,
                        dependency_frontier_digest, prior_frontier_root,
                        prior_frontier_root_digest, post_frontier_root,
                        post_frontier_root_digest, affected_documents,
                        affected_documents_digest, causal_dependency_heads,
                        causal_peer_id, causal_counter, causal_clock_root_key,
                        causal_clock_root_digest, 1000, retained_bytes
                 FROM applied_batches WHERE sequence = 1",
            ),
            (
                "delete authoritative page row",
                "DELETE FROM pages
                 WHERE page_id = (SELECT MIN(page_id) FROM pages)",
            ),
            (
                "insert extra page row",
                "INSERT INTO pages
                 SELECT randomblob(16), home_document_id, name || ' stale',
                        name_key || ' stale', path || '.stale', text_kind,
                        preamble, searchable_text
                 FROM pages ORDER BY page_id LIMIT 1",
            ),
            (
                "change authoritative block row",
                "UPDATE blocks SET content = content || ' stale'
                 WHERE block_id = (SELECT MIN(block_id) FROM blocks)",
            ),
            (
                "change authoritative page-name row",
                "UPDATE pages
                 SET name = name || ' stale', name_key = name_key || ' stale'
                 WHERE page_id = (SELECT MIN(page_id) FROM pages)",
            ),
            (
                "delete authoritative reference posting",
                "DELETE FROM reference_postings
                 WHERE (
                     source_page_id, source_entity_type, source_entity_id,
                     source_locator, ordinal
                 ) = (
                     SELECT source_page_id, source_entity_type, source_entity_id,
                            source_locator, ordinal
                     FROM reference_postings LIMIT 1
                 )",
            ),
            (
                "insert extra reference posting",
                "INSERT INTO reference_postings
                 SELECT source_page_id, source_entity_type, source_entity_id,
                        source_locator, ordinal + 100000, reference_kind,
                        target_type, raw_name, normalized_name, raw_uuid_claim,
                        resolved_page_id, resolved_block_id
                 FROM reference_postings LIMIT 1",
            ),
            (
                "change materialization batch row",
                "UPDATE materialization_batches SET input_digest = randomblob(32)
                 WHERE acceptance_sequence = 1",
            ),
            (
                "insert extra materialization batch row",
                "INSERT INTO materialization_batches
                 SELECT 1000, randomblob(16), input_digest, event_binding_digest,
                        prior_frontier_root_digest, post_frontier_root_digest,
                        prior_catalog_root, prior_catalog_root_digest,
                        post_catalog_root, post_catalog_root_digest,
                        catalog_change, catalog_change_digest,
                        canonical_input_digest
                 FROM materialization_batches WHERE acceptance_sequence = 1",
            ),
        ];

        for (label, sql) in corruptions {
            let connection = Connection::open(&path).unwrap();
            assert_ne!(connection.execute(sql, []).unwrap(), 0, "{label}");
            drop(connection);
            crate::oplog::sqlite::refresh_projection_checkpoint_for_harness(&path, claim).unwrap();

            let (rebuilt, proof) =
                SqliteFrontier::open_or_rebuild_inactive_bootstrap(&path, &runtime, &authority)
                    .unwrap();
            assert_eq!(proof, expected, "{label}");
            assert!(
                matches!(
                    rebuilt.recovery,
                    ProjectionRecovery::RebuiltPreservingEvidence { .. }
                ),
                "{label}"
            );
            assert_eq!(
                rebuilt.rebuild.accepted_events_validated, part_count,
                "{label}"
            );
            assert_eq!(
                rebuilt.rebuild.accepted_events_applied, part_count,
                "{label}"
            );
            assert_eq!(
                proof.bootstrap_rebuild().bootstrap_part_reads,
                part_count,
                "{label}"
            );
            drop(rebuilt);
        }

        for (relative, bytes) in graph_before {
            assert_eq!(
                fs::read(root.path().join("graph").join(relative)).unwrap(),
                bytes
            );
        }
        assert!(!root.path().join("archive/projection-work").exists());
    }
}
