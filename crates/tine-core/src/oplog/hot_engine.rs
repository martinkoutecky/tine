use std::cell::{Cell, RefCell};
use std::collections::{btree_map::Entry, BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use loro::{
    Container, ContainerType, EncodedBlobMode, ExportMode, LoroDoc, LoroMap, LoroValue,
    UpdateOptions, ValueOrContainer, VersionVector,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::object_store::{BlockClaimIndexRoot, BlockClaimIndexStore, BlockClaimIndexValue};
use super::scratch_store::{ScratchRoots, ScratchStore};
use super::{
    BatchCausalDot, BatchId, BatchInspection, BlockDelta, BlockId, BlockOwner, BlockState,
    CausalPeerId, ContentDigest, CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentCausalDigest,
    DocumentDependencies, DocumentId, FrontierV2, LineageDigest, ManagedPath, MembershipClaim,
    MembershipDelta, ObjectKind, ObjectStore, OperationBatch, OperationObject, PageDelta, PageId,
    PageState, PreparedBatch, SemanticEffect, SemanticEffectDigest, SemanticError, SessionId,
    ValidatedBatch, WorkspaceId,
};

const CATALOG_PAGES: &str = "pages";
const SHARD_META: &str = "shard_meta";
const SHARD_PAGE_ID: &str = "page_id";
const SHARD_OWNERS: &str = "owners";
const SHARD_MEMBERS: &str = "members";
const SHARD_CONTENT: &str = "content";
const TOMBSTONE: &str = "tombstone";
const MAX_TRANSACTION_OPERATIONS: usize = 100_000;
const MAX_DOCUMENT_ENTRIES: usize = 1_000_000;
const MAX_HOT_NON_CATALOG_DOCUMENTS: usize = 64;
const CRDT_UPDATE_PAYLOAD_SCHEMA_VERSION: u32 = 5;
const ENGINE_HISTORY_SCHEMA_VERSION: u32 = 4;
const BLOCK_CLAIM_RECORD_SCHEMA_VERSION: u32 = 2;
const ACCEPTED_EVIDENCE_SCHEMA_VERSION: u32 = 2;
const ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION: u32 = 1;
const MAX_EPHEMERAL_BLOCK_CLAIMS: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CrdtUpdatePayload {
    schema_version: u32,
    batch_id: BatchId,
    document_id: DocumentId,
    dependency_heads: Vec<BatchId>,
    batch_dependency_heads: Vec<BatchId>,
    causal_state_digest: Option<DocumentCausalDigest>,
    raw_update: Vec<u8>,
}

#[derive(Debug)]
struct PendingAuthorDocuments {
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    documents: BTreeMap<DocumentId, LoroDoc>,
}

#[derive(Debug)]
enum EngineDocument {
    InMemory(LoroDoc),
    External(super::document_state::ExternalDocument),
}

impl EngineDocument {
    fn document(&self) -> &LoroDoc {
        match self {
            Self::InMemory(document) => document,
            Self::External(document) => document.document(),
        }
    }

    fn into_document(self) -> LoroDoc {
        match self {
            Self::InMemory(document) => document,
            Self::External(document) => document.into_document(),
        }
    }

    fn external(&self) -> Option<&super::document_state::ExternalDocument> {
        match self {
            Self::External(document) => Some(document),
            Self::InMemory(_) => None,
        }
    }
}

struct IdentityPublicationCandidate {
    blocked: bool,
    scratch_roots: ScratchRoots,
    block_claim_root: BlockClaimIndexRoot,
    fatal_handle: Option<FatalEvidenceHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BlockClaimRecord {
    schema_version: u32,
    block_id: BlockId,
    claims: Vec<ImmutableHomeClaim>,
}

#[derive(Serialize)]
struct BlockClaimRecordRef<'a> {
    schema_version: u32,
    block_id: BlockId,
    claims: &'a [ImmutableHomeClaim],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockLocation {
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableHomeClaim {
    batch_id: BatchId,
    home_document_id: DocumentId,
    causal_dot: Option<BatchCausalDot>,
}

impl PartialEq for ImmutableHomeClaim {
    fn eq(&self, other: &Self) -> bool {
        (self.batch_id, self.home_document_id) == (other.batch_id, other.home_document_id)
    }
}

impl Eq for ImmutableHomeClaim {}

impl PartialOrd for ImmutableHomeClaim {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ImmutableHomeClaim {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.batch_id, self.home_document_id).cmp(&(other.batch_id, other.home_document_id))
    }
}

impl ImmutableHomeClaim {
    pub const fn new(batch_id: BatchId, home_document_id: DocumentId) -> Self {
        Self {
            batch_id,
            home_document_id,
            causal_dot: None,
        }
    }

    pub const fn with_causal_dot(
        batch_id: BatchId,
        home_document_id: DocumentId,
        causal_dot: BatchCausalDot,
    ) -> Self {
        Self {
            batch_id,
            home_document_id,
            causal_dot: Some(causal_dot),
        }
    }

    pub const fn batch_id(self) -> BatchId {
        self.batch_id
    }

    pub const fn home_document_id(self) -> DocumentId {
        self.home_document_id
    }

    pub const fn causal_dot(self) -> Option<BatchCausalDot> {
        self.causal_dot
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableHomeConflict {
    block_id: BlockId,
    claims: Vec<ImmutableHomeClaim>,
}

impl ImmutableHomeConflict {
    pub fn new(block_id: BlockId, first: ImmutableHomeClaim, second: ImmutableHomeClaim) -> Self {
        Self::from_claims(block_id, [first, second])
    }

    pub fn from_claims(
        block_id: BlockId,
        claims: impl IntoIterator<Item = ImmutableHomeClaim>,
    ) -> Self {
        let mut claims: Vec<_> = claims.into_iter().collect();
        claims.sort_unstable();
        claims.dedup();
        Self { block_id, claims }
    }

    pub const fn block_id(&self) -> BlockId {
        self.block_id
    }

    pub fn claims(&self) -> &[ImmutableHomeClaim] {
        &self.claims
    }
}

impl fmt::Display for ImmutableHomeConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "block {} has conflicting immutable-home claims",
            self.block_id
        )?;
        for claim in &self.claims {
            write!(
                f,
                ": batch {} home {}",
                claim.batch_id, claim.home_document_id
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableHomeEvidence {
    conflicts: Vec<ImmutableHomeConflict>,
}

impl ImmutableHomeEvidence {
    pub fn new(mut conflicts: Vec<ImmutableHomeConflict>) -> Self {
        conflicts.sort_unstable_by_key(ImmutableHomeConflict::block_id);
        Self { conflicts }
    }

    pub fn conflicts(&self) -> &[ImmutableHomeConflict] {
        &self.conflicts
    }
}

impl fmt::Display for ImmutableHomeEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, conflict) in self.conflicts.iter().enumerate() {
            if index != 0 {
                f.write_str("; ")?;
            }
            write!(f, "{conflict}")?;
        }
        Ok(())
    }
}

fn in_memory_evidence_handle(evidence: &ImmutableHomeEvidence) -> FatalEvidenceHandle {
    let bytes = postcard::to_allocvec(evidence)
        .expect("immutable-home evidence has an infallible canonical encoding");
    let conflict_root = ContentDigest::of(&bytes);
    let conflicting_block_count = evidence.conflicts().len() as u64;
    let claim_count = evidence
        .conflicts()
        .iter()
        .map(|conflict| conflict.claims().len() as u64)
        .sum();
    let summary = postcard::to_allocvec(&(conflict_root, conflicting_block_count, claim_count))
        .expect("fatal-evidence summary has an infallible canonical encoding");
    FatalEvidenceHandle {
        conflict_root,
        conflicting_block_count,
        claim_count,
        canonical_digest: ContentDigest::of(&summary),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FatalEvidenceHandle {
    pub(crate) conflict_root: ContentDigest,
    pub(crate) conflicting_block_count: u64,
    pub(crate) claim_count: u64,
    pub(crate) canonical_digest: ContentDigest,
}

const MAX_FATAL_EVIDENCE_PAGE_CONFLICTS: usize = 32;

/// Opaque continuation for one authenticated fatal-evidence root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FatalEvidenceCursor {
    conflict_root: ContentDigest,
    after: BlockId,
}

/// A fixed-size inspection result for terminal conflict evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FatalEvidencePage {
    conflicts: Vec<ImmutableHomeConflict>,
    next: Option<FatalEvidenceCursor>,
}

impl FatalEvidencePage {
    pub fn conflicts(&self) -> &[ImmutableHomeConflict] {
        &self.conflicts
    }

    pub const fn next(&self) -> Option<FatalEvidenceCursor> {
        self.next
    }
}

impl FatalEvidenceHandle {
    pub const fn conflict_root(self) -> ContentDigest {
        self.conflict_root
    }

    pub const fn conflicting_block_count(self) -> u64 {
        self.conflicting_block_count
    }

    pub const fn claim_count(self) -> u64 {
        self.claim_count
    }

    pub const fn canonical_digest(self) -> ContentDigest {
        self.canonical_digest
    }
}

impl Default for FatalEvidenceHandle {
    fn default() -> Self {
        let empty = ContentDigest::of(b"tine/oplog-conflict-evidence/v2/empty");
        Self {
            conflict_root: empty,
            conflicting_block_count: 0,
            claim_count: 0,
            canonical_digest: empty,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticOperation {
    CreatePage {
        page_id: PageId,
        home_document_id: DocumentId,
        path: ManagedPath,
    },
    EditPagePath {
        page_id: PageId,
        path: ManagedPath,
    },
    CreateBlock {
        block: BlockLocation,
        page_id: PageId,
        parent: Option<BlockId>,
        order: String,
        content: String,
    },
    EditBlockContent {
        block: BlockLocation,
        content: String,
    },
    MoveSubtree {
        root: BlockLocation,
        from_page_id: PageId,
        to_page_id: PageId,
        parent: Option<BlockId>,
        order: String,
    },
    ReorderBlock {
        block_id: BlockId,
        page_id: PageId,
        parent: Option<BlockId>,
        order: String,
    },
    DeleteSubtree {
        root_block_id: BlockId,
        page_id: PageId,
    },
    DeletePage {
        page_id: PageId,
    },
    RenamePageAndRewriteReferrers {
        page_id: PageId,
        path: ManagedPath,
        referrers: Vec<(BlockLocation, String)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationTransaction {
    pub operations: Vec<SemanticOperation>,
}

impl OperationTransaction {
    pub fn new(operations: Vec<SemanticOperation>) -> Result<Self, EngineError> {
        if operations.is_empty() || operations.len() > MAX_TRANSACTION_OPERATIONS {
            return Err(EngineError::InvalidTransaction(format!(
                "transaction operation count {} is outside 1..={MAX_TRANSACTION_OPERATIONS}",
                operations.len()
            )));
        }
        Ok(Self { operations })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorBatch {
    pub batch_id: BatchId,
    pub author_device_id: DeviceId,
    pub author_session_id: SessionId,
    pub crdt_peer_id: CrdtPeerId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedBatch {
    pub batch_id: BatchId,
    pub no_op: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedBatchEvidence {
    schema_version: u32,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    event_binding_digest: ContentDigest,
    acceptance_sequence: u64,
    prior_frontier_root: AcceptedFrontierRoot,
    post_frontier_root: AcceptedFrontierRoot,
    affected_documents: Vec<DocumentDependencies>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedFrontierRoot {
    schema_version: u32,
    acceptance_sequence: u64,
    document_count: u64,
    state_digest: ContentDigest,
    scratch_root: Option<super::scratch_store::ScratchLsmRoot>,
}

impl AcceptedBatchEvidence {
    #[cfg(test)]
    pub(crate) fn for_test(
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        event_binding_digest: ContentDigest,
        prior_frontier_root: AcceptedFrontierRoot,
        affected_documents: Vec<DocumentDependencies>,
        document_count: u64,
    ) -> Self {
        let acceptance_sequence = prior_frontier_root.acceptance_sequence.saturating_add(1);
        let post_frontier_root = next_accepted_frontier_root(
            &prior_frontier_root,
            event_binding_digest,
            acceptance_sequence,
            document_count,
            &affected_documents,
            None,
        )
        .expect("canonical test accepted-frontier transition");
        Self {
            schema_version: ACCEPTED_EVIDENCE_SCHEMA_VERSION,
            batch_id,
            manifest_fingerprint,
            event_binding_digest,
            acceptance_sequence,
            prior_frontier_root,
            post_frontier_root,
            affected_documents,
        }
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub const fn manifest_fingerprint(&self) -> ContentDigest {
        self.manifest_fingerprint
    }

    pub const fn event_binding_digest(&self) -> ContentDigest {
        self.event_binding_digest
    }

    pub(crate) fn binding_digest_for(
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        semantic_effect_digest: SemanticEffectDigest,
        dependency_frontier: &FrontierV2,
        causal_dependency_heads: &[BatchId],
    ) -> Result<ContentDigest, EngineError> {
        let bytes = postcard::to_allocvec(&(
            b"tine/oplog/accepted-event-binding/v1".as_slice(),
            batch_id,
            manifest_fingerprint,
            semantic_effect_digest,
            dependency_frontier,
            causal_dependency_heads,
        ))
        .map_err(|error| EngineError::Archive(error.to_string()))?;
        Ok(ContentDigest::of(&bytes))
    }

    pub const fn acceptance_sequence(&self) -> u64 {
        self.acceptance_sequence
    }

    pub const fn prior_frontier_root(&self) -> &AcceptedFrontierRoot {
        &self.prior_frontier_root
    }

    pub const fn post_frontier_root(&self) -> &AcceptedFrontierRoot {
        &self.post_frontier_root
    }

    pub fn affected_documents(&self) -> &[DocumentDependencies] {
        &self.affected_documents
    }
}

impl AcceptedFrontierRoot {
    pub fn empty() -> Self {
        empty_accepted_frontier_root()
    }

    pub const fn acceptance_sequence(&self) -> u64 {
        self.acceptance_sequence
    }

    pub const fn document_count(&self) -> u64 {
        self.document_count
    }

    pub const fn state_digest(&self) -> ContentDigest {
        self.state_digest
    }

    pub(crate) const fn has_persistent_point_index(&self) -> bool {
        self.scratch_root.is_some()
    }

    pub(crate) fn validates_transition(
        &self,
        event_binding_digest: ContentDigest,
        acceptance_sequence: u64,
        document_count: u64,
        affected_documents: &[DocumentDependencies],
        post: &Self,
    ) -> Result<bool, EngineError> {
        Ok(next_accepted_frontier_root(
            self,
            event_binding_digest,
            acceptance_sequence,
            document_count,
            affected_documents,
            post.scratch_root.clone(),
        )? == *post)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchDisposition {
    IncompleteStaged {
        missing_objects: usize,
        missing_dependencies: Vec<BatchId>,
    },
    Accepted {
        no_op: bool,
    },
    DuplicateAccepted {
        no_op: bool,
    },
    Quarantined,
    Rejected {
        error: EngineError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceStatus {
    Operational,
    Blocked(FatalEvidenceHandle),
}

#[derive(Clone, Debug)]
pub struct EngineStatus {
    history_source: StatusHistorySource,
    history: OnceLock<Result<StatusHistory, EngineError>>,
    workspace: WorkspaceStatus,
}

impl EngineStatus {
    pub fn try_eq(&self, other: &Self) -> Result<bool, EngineError> {
        Ok(self.workspace == other.workspace && self.history()? == other.history()?)
    }

    pub fn accepted_batches(&self) -> Result<&[AcceptedBatch], EngineError> {
        Ok(&self.history()?.accepted_batches)
    }

    pub fn accepted_batch_ids(&self) -> Result<Vec<BatchId>, EngineError> {
        Ok(self
            .history()?
            .accepted_batches
            .iter()
            .map(|accepted| accepted.batch_id)
            .collect())
    }

    /// Fully validated batches retained only on the terminal forensic
    /// frontier. These batches never authorize user-visible state.
    pub fn validated_unpublished_batch_ids(&self) -> Result<&[BatchId], EngineError> {
        Ok(&self.history()?.validated_unpublished_batches)
    }

    /// Canonical set of namespace-valid, collision-checked Ready batches that
    /// this engine has observed, including staged and rejected ingress.
    pub fn offered_batch_ids(&self) -> Result<&[BatchId], EngineError> {
        Ok(&self.history()?.offered_batches)
    }

    pub const fn workspace(&self) -> &WorkspaceStatus {
        &self.workspace
    }

    fn history(&self) -> Result<&StatusHistory, EngineError> {
        self.history
            .get_or_init(|| self.history_source.materialize())
            .as_ref()
            .map_err(Clone::clone)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StatusHistory {
    accepted_batches: Vec<AcceptedBatch>,
    validated_unpublished_batches: Vec<BatchId>,
    offered_batches: Vec<BatchId>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum StatusHistorySource {
    Inline(StatusHistory),
    Failed(EngineError),
    Cold {
        store: Arc<super::object_store::EngineHistoryStore>,
        through_generation: u64,
        history_root: ContentDigest,
        active: Vec<ColdHistoryRecord>,
    },
    Scratch {
        store: Arc<ScratchStore>,
        roots: ScratchRoots,
    },
}

impl StatusHistorySource {
    fn materialize(&self) -> Result<StatusHistory, EngineError> {
        let records = match self {
            Self::Inline(history) => return Ok(history.clone()),
            Self::Failed(error) => return Err(error.clone()),
            Self::Cold {
                store,
                through_generation,
                history_root,
                active,
            } => {
                let mut records =
                    validated_history_records(store, *through_generation, *history_root)?;
                records.extend(active.iter().cloned());
                records
            }
            Self::Scratch { store, roots } => super::dependency_queue::all_records(store, roots)
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .into_iter()
                .map(|record| {
                    let status = match record.status() {
                        super::dependency_queue::CompactBatchStatus::Final => {
                            decode_archive_status(record.final_status().ok_or_else(|| {
                                EngineError::Archive("final scratch status has no result".into())
                            })?)?
                        }
                        super::dependency_queue::CompactBatchStatus::Waiting
                        | super::dependency_queue::CompactBatchStatus::Ready
                        | super::dependency_queue::CompactBatchStatus::Processing => {
                            ArchiveStatus::Staged
                        }
                    };
                    Ok(ColdHistoryRecord {
                        schema_version: ENGINE_HISTORY_SCHEMA_VERSION,
                        generation: 0,
                        batch_id: record.batch_id(),
                        manifest_fingerprint: record.manifest_fingerprint(),
                        status,
                    })
                })
                .collect::<Result<Vec<_>, EngineError>>()?,
        };
        Ok(status_history_from_records(records))
    }
}

#[derive(Clone, Debug)]
pub struct StageOutcome {
    batch_id: BatchId,
    pub disposition: BatchDisposition,
    newly_accepted: Vec<AcceptedBatch>,
    status: EngineStatus,
}

impl StageOutcome {
    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub fn disposition(&self) -> BatchDisposition {
        self.disposition.clone()
    }

    pub fn newly_accepted(&self) -> &[AcceptedBatch] {
        &self.newly_accepted
    }

    pub const fn status(&self) -> &EngineStatus {
        &self.status
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ArchiveStatus {
    Staged,
    Accepted {
        no_op: bool,
        evidence: AcceptedBatchEvidence,
    },
    Quarantined,
    Rejected(EngineError),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColdHistoryRecord {
    schema_version: u32,
    generation: u64,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    status: ArchiveStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BatchApplication {
    Accepted {
        no_op: bool,
        evidence: AcceptedBatchEvidence,
    },
    Quarantined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedBlock {
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MaterializationStats {
    pub catalog_documents_loaded: usize,
    pub membership_documents_loaded: usize,
    pub home_documents_loaded: usize,
    pub distinct_home_documents: Vec<DocumentId>,
    pub physical_manifest_reads: usize,
    pub physical_object_reads: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedPage {
    pub page_id: PageId,
    pub path: ManagedPath,
    pub blocks: Vec<MaterializedBlock>,
    pub stats: MaterializationStats,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct HistoryWorkStats {
    prepare_transactions: usize,
    prepare_document_head_visits: usize,
    author_snapshot_clones: usize,
    author_snapshot_clone_ops: usize,
    stage_snapshot_clones: usize,
    stage_snapshot_clone_ops: usize,
    stage_structural_buffer_reuses: usize,
    drain_candidate_visits: usize,
    dependency_status_lookups: usize,
    document_point_reads: usize,
    state_page_bytes_read: usize,
    state_page_bytes_written: usize,
    wait_edge_visits: usize,
    ready_queue_residency: usize,
    external_flushes: usize,
    external_point_reads: usize,
    external_range_scans: usize,
    external_history_page_reads: usize,
    external_history_blob_reads: usize,
    ancestry_traversals: usize,
    block_claim_validation_nanos: usize,
    block_claim_lookup_nanos: usize,
    block_claim_encode_nanos: usize,
    block_claim_insert_nanos: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineInstrumentation {
    pub prepare_transactions: usize,
    pub prepare_document_head_visits: usize,
    pub author_snapshot_clones: usize,
    pub author_snapshot_clone_ops: usize,
    pub stage_snapshot_clones: usize,
    pub stage_snapshot_clone_ops: usize,
    pub stage_structural_buffer_reuses: usize,
    pub drain_candidate_visits: usize,
    pub dependency_status_lookups: usize,
    pub document_point_reads: usize,
    pub state_page_bytes_read: usize,
    pub state_page_bytes_written: usize,
    pub wait_edge_visits: usize,
    pub ready_queue_residency: usize,
    pub external_flushes: usize,
    pub external_point_reads: usize,
    pub external_range_scans: usize,
    pub external_history_page_reads: usize,
    pub external_history_blob_reads: usize,
    pub ancestry_traversals: usize,
    pub scratch_syncs: usize,
    pub stale_scratch_runs_reclaimed: usize,
    pub live_scratch_runs_skipped: usize,
    pub batch_status_hot_entries: usize,
    pub ready_payload_hot_entries: usize,
    pub document_hot_entries: usize,
    pub conflict_hot_entries: usize,
    pub block_claim_hot_entries: usize,
    pub block_claim_validation_nanos: usize,
    pub block_claim_lookup_nanos: usize,
    pub block_claim_encode_nanos: usize,
    pub block_claim_insert_nanos: usize,
    pub store: super::ObjectStoreStats,
}

/// Experimental, disconnected v2 hot state. The immutable archive is retained
/// separately from visible Loro documents. Only `ValidatedBatch` values from
/// the object-store Ready boundary enter semantic validation.
pub struct ShardedHotEngine {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    archive: BTreeMap<BatchId, ValidatedBatch>,
    archive_store: Option<Arc<ObjectStore>>,
    scratch: Option<Arc<ScratchStore>>,
    scratch_roots: ScratchRoots,
    ephemeral_causal_chain: RefCell<BTreeMap<CausalPeerId, (u64, BatchId)>>,
    history_store: Option<Arc<super::object_store::EngineHistoryStore>>,
    history_generation: u64,
    history_root: ContentDigest,
    history_failure: Option<EngineError>,
    archive_fingerprints: BTreeMap<BatchId, ContentDigest>,
    persisted_staged: BTreeSet<BatchId>,
    statuses: BTreeMap<BatchId, ArchiveStatus>,
    // Authenticated point-validation evidence, never a live owner authority.
    // Store-backed engines retain only this root; the sole live owner remains
    // in the immutable home shard. The bounded map is a no-store test harness.
    block_claim_index: Option<Arc<BlockClaimIndexStore>>,
    block_claim_root: BlockClaimIndexRoot,
    ephemeral_block_claims: AHashMap<u128, BTreeSet<ImmutableHomeClaim>>,
    fatal_evidence: Option<ImmutableHomeEvidence>,
    fatal_handle: Option<FatalEvidenceHandle>,
    visible_documents: BTreeMap<DocumentId, LoroDoc>,
    // A second current-state buffer is reused across ordinary authorship.
    // It accumulates the same incremental updates as the visible buffer, so
    // preparing the next bounded edit never snapshots accumulated CRDT history.
    spare_documents: RefCell<BTreeMap<DocumentId, LoroDoc>>,
    pending_author_documents: RefCell<Option<PendingAuthorDocuments>>,
    visible_document_lru: VecDeque<DocumentId>,
    visible_document_heads: BTreeMap<DocumentId, BTreeSet<BatchId>>,
    // Lazily created only after the terminal latch. This CRDT frontier
    // validates offered descendants without ever becoming visible authority.
    terminal_documents: BTreeMap<DocumentId, LoroDoc>,
    terminal_document_heads: BTreeMap<DocumentId, BTreeSet<BatchId>>,
    // Point lookups are memoized only within one public operation. The cache
    // is cleared between operations and whenever the authenticated root
    // advances, so a later cold read still revalidates immutable bytes.
    status_point_cache: RefCell<BTreeMap<BatchId, Option<ColdHistoryRecord>>>,
    external_anchor_point_cache:
        RefCell<BTreeSet<(DocumentId, BatchId, ContentDigest, ContentDigest)>>,
    history_work: Cell<HistoryWorkStats>,
    accepted_frontier: BTreeMap<DocumentId, DocumentDependencies>,
    accepted_frontier_root: AcceptedFrontierRoot,
    accepted_sequence: BTreeMap<u64, BatchId>,
    next_acceptance_sequence: u64,
    #[cfg(test)]
    validation_phase_nanos: [u128; 10],
    #[cfg(test)]
    external_publication_failure_index: Option<usize>,
}

impl fmt::Debug for ShardedHotEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardedHotEngine")
            .field("workspace_id", &self.workspace_id)
            .field("lineage_digest", &self.lineage_digest)
            .field("catalog_document_id", &self.catalog_document_id)
            .field("archive_batches", &self.archive.len())
            .field("store_backed", &self.archive_store.is_some())
            .field("visible_documents", &self.visible_documents.len())
            .field("fatal_evidence", &self.fatal_evidence)
            .finish()
    }
}

impl ShardedHotEngine {
    pub fn new(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
    ) -> Self {
        Self {
            workspace_id,
            lineage_digest,
            catalog_document_id,
            archive: BTreeMap::new(),
            archive_store: None,
            scratch: None,
            scratch_roots: ScratchRoots::default(),
            ephemeral_causal_chain: RefCell::new(BTreeMap::new()),
            history_store: None,
            history_generation: 0,
            history_root: super::object_store::EngineHistoryStore::empty_root(),
            history_failure: None,
            archive_fingerprints: BTreeMap::new(),
            persisted_staged: BTreeSet::new(),
            statuses: BTreeMap::new(),
            block_claim_index: None,
            block_claim_root: BlockClaimIndexRoot::default(),
            ephemeral_block_claims: AHashMap::new(),
            fatal_evidence: None,
            fatal_handle: None,
            visible_documents: BTreeMap::new(),
            spare_documents: RefCell::new(BTreeMap::new()),
            pending_author_documents: RefCell::new(None),
            visible_document_lru: VecDeque::new(),
            visible_document_heads: BTreeMap::new(),
            terminal_documents: BTreeMap::new(),
            terminal_document_heads: BTreeMap::new(),
            status_point_cache: RefCell::new(BTreeMap::new()),
            external_anchor_point_cache: RefCell::new(BTreeSet::new()),
            history_work: Cell::new(HistoryWorkStats::default()),
            accepted_frontier: BTreeMap::new(),
            accepted_frontier_root: empty_accepted_frontier_root(),
            accepted_sequence: BTreeMap::new(),
            next_acceptance_sequence: 0,
            #[cfg(test)]
            validation_phase_nanos: [0; 10],
            #[cfg(test)]
            external_publication_failure_index: None,
        }
    }

    /// Construct a sparse engine that follows compact direct heads through
    /// immutable manifests on cold fallback. Accepted non-catalog shards are
    /// evicted from hot memory and reconstructed from authenticated DAG
    /// ancestry on demand.
    pub fn with_archive_store(
        store: ObjectStore,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
    ) -> Self {
        let workspace_id = store.workspace_id();
        let mut engine = Self::new(workspace_id, lineage_digest, catalog_document_id);
        match store.start_engine_scratch() {
            Ok((scratch, index)) => {
                engine.scratch = Some(scratch);
                engine.block_claim_index = Some(Arc::new(index));
            }
            Err(error) => engine.history_failure = Some(EngineError::Archive(error.to_string())),
        }
        engine.archive_store = Some(Arc::new(store));
        engine
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub const fn catalog_document_id(&self) -> DocumentId {
        self.catalog_document_id
    }

    pub fn instrumentation(&self) -> EngineInstrumentation {
        let work = self.history_work.get();
        let scratch = self
            .scratch
            .as_ref()
            .map(|store| store.stats())
            .unwrap_or_default();
        EngineInstrumentation {
            prepare_transactions: work.prepare_transactions,
            prepare_document_head_visits: work.prepare_document_head_visits,
            author_snapshot_clones: work.author_snapshot_clones,
            author_snapshot_clone_ops: work.author_snapshot_clone_ops,
            stage_snapshot_clones: work.stage_snapshot_clones,
            stage_snapshot_clone_ops: work.stage_snapshot_clone_ops,
            stage_structural_buffer_reuses: work.stage_structural_buffer_reuses,
            drain_candidate_visits: work.drain_candidate_visits,
            dependency_status_lookups: work.dependency_status_lookups,
            document_point_reads: work.document_point_reads,
            state_page_bytes_read: work.state_page_bytes_read,
            state_page_bytes_written: work.state_page_bytes_written,
            wait_edge_visits: work.wait_edge_visits,
            ready_queue_residency: work.ready_queue_residency,
            external_flushes: work.external_flushes,
            external_point_reads: work.external_point_reads,
            external_range_scans: work.external_range_scans,
            external_history_page_reads: work.external_history_page_reads,
            external_history_blob_reads: work.external_history_blob_reads,
            ancestry_traversals: work.ancestry_traversals,
            scratch_syncs: scratch.scratch_syncs,
            stale_scratch_runs_reclaimed: scratch.stale_runs_reclaimed,
            live_scratch_runs_skipped: scratch.live_runs_skipped,
            batch_status_hot_entries: self.statuses.len(),
            ready_payload_hot_entries: self.archive.len(),
            document_hot_entries: self
                .visible_documents
                .keys()
                .chain(self.terminal_documents.keys())
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            conflict_hot_entries: self
                .fatal_evidence
                .as_ref()
                .map(|evidence| evidence.conflicts().len())
                .unwrap_or(0),
            block_claim_hot_entries: self.ephemeral_block_claims.len(),
            block_claim_validation_nanos: work.block_claim_validation_nanos,
            block_claim_lookup_nanos: work.block_claim_lookup_nanos,
            block_claim_encode_nanos: work.block_claim_encode_nanos,
            block_claim_insert_nanos: work.block_claim_insert_nanos,
            store: self
                .archive_store
                .as_ref()
                .map(|store| store.instrumentation())
                .unwrap_or_default(),
        }
    }

    /// One atomic diagnostic view. Accepted batches remain historical facts
    /// even when `workspace` is terminally blocked.
    pub fn status(&self) -> EngineStatus {
        let active = self
            .statuses
            .iter()
            .map(|(batch_id, status)| {
                new_history_record(
                    self.history_generation.saturating_add(1),
                    *batch_id,
                    self.archive_fingerprints[batch_id],
                    status.clone(),
                )
            })
            .collect();
        let history_source = match (&self.history_failure, &self.scratch, &self.history_store) {
            (Some(error), _, _) => StatusHistorySource::Failed(error.clone()),
            (None, Some(store), _) => StatusHistorySource::Scratch {
                store: Arc::clone(store),
                roots: self.scratch_roots.clone(),
            },
            (None, None, Some(store)) => StatusHistorySource::Cold {
                store: Arc::clone(store),
                through_generation: self.history_generation,
                history_root: self.history_root,
                active,
            },
            (None, None, None) => StatusHistorySource::Inline(status_history_from_records(active)),
        };
        EngineStatus {
            history_source,
            history: OnceLock::new(),
            workspace: self.workspace_status(),
        }
    }

    /// Return the incrementally maintained complete accepted frontier.
    pub fn exact_frontier(&self) -> Result<FrontierV2, EngineError> {
        self.ensure_not_blocked()?;
        if let Some(store) = &self.scratch {
            return materialize_accepted_frontier(
                store,
                &self.scratch_roots.accepted_frontier_root,
            );
        }
        FrontierV2::new(self.accepted_frontier.values().cloned().collect())
            .map_err(EngineError::from)
    }

    pub fn accepted_frontier_root(&self) -> Result<AcceptedFrontierRoot, EngineError> {
        self.ensure_not_blocked()?;
        validate_accepted_frontier_root(&self.accepted_frontier_root)?;
        Ok(self.accepted_frontier_root.clone())
    }

    pub fn accepted_batch_count(&self) -> Result<u64, EngineError> {
        self.ensure_not_blocked()?;
        Ok(self.next_acceptance_sequence)
    }

    pub fn accepted_batch_id_at(&self, sequence: u64) -> Result<Option<BatchId>, EngineError> {
        self.ensure_not_blocked()?;
        if sequence == 0 || sequence > self.next_acceptance_sequence {
            return Ok(None);
        }
        if let Some(store) = &self.scratch {
            let bytes = store
                .lookup(
                    &self.scratch_roots.accepted_sequence_root,
                    super::scratch_store::ScratchPageKind::AcceptedSequence,
                    &sequence.to_be_bytes(),
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .ok_or_else(|| {
                    EngineError::Archive(format!(
                        "accepted sequence {sequence} has no authenticated batch"
                    ))
                })?;
            let uuid = Uuid::from_slice(&bytes).map_err(|error| {
                EngineError::Archive(format!(
                    "accepted sequence {sequence} has invalid batch bytes: {error}"
                ))
            })?;
            return Ok(Some(BatchId::from_uuid(uuid)));
        }
        Ok(self.accepted_sequence.get(&sequence).copied())
    }

    pub fn accepted_frontier_document(
        &self,
        root: &AcceptedFrontierRoot,
        document_id: DocumentId,
    ) -> Result<Option<DocumentDependencies>, EngineError> {
        validate_accepted_frontier_root(root)?;
        let Some(scratch_root) = &root.scratch_root else {
            if root == &self.accepted_frontier_root {
                return Ok(self.accepted_frontier.get(&document_id).cloned());
            }
            return Err(EngineError::Archive(
                "historical frontier point queries require store-backed accepted history".into(),
            ));
        };
        let store = self.scratch.as_ref().ok_or_else(|| {
            EngineError::Archive(
                "store-backed accepted frontier root has no authenticated scratch store".into(),
            )
        })?;
        let bytes = store
            .lookup(
                scratch_root,
                super::scratch_store::ScratchPageKind::AcceptedFrontier,
                document_id.as_uuid().as_bytes(),
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        bytes
            .map(|bytes| decode_accepted_document(document_id, &bytes))
            .transpose()
    }

    /// Point lookup of immutable evidence bound when this batch became
    /// accepted. Store-backed engines authenticate the record from scratch
    /// state and cross-check its manifest fingerprint against batch history.
    pub fn accepted_batch_evidence(
        &self,
        batch_id: BatchId,
    ) -> Result<AcceptedBatchEvidence, EngineError> {
        self.begin_point_operation();
        let evidence = match self.archive_status(batch_id)? {
            Some(ArchiveStatus::Accepted { evidence, .. }) => evidence,
            _ => return Err(EngineError::MissingDependency(batch_id)),
        };
        let expected_fingerprint = if let Some(store) = &self.scratch {
            super::dependency_queue::lookup(store, &self.scratch_roots, batch_id)
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .ok_or_else(|| {
                    EngineError::Archive(format!("accepted batch {batch_id} has no status history"))
                })?
                .manifest_fingerprint()
        } else {
            self.archive_fingerprints
                .get(&batch_id)
                .copied()
                .ok_or(EngineError::MissingDependency(batch_id))?
        };
        if evidence.manifest_fingerprint != expected_fingerprint {
            return Err(EngineError::Archive(format!(
                "accepted batch {batch_id} frontier evidence fingerprint mismatch"
            )));
        }
        validate_accepted_evidence(&evidence)?;
        Ok(evidence)
    }

    fn prepare_acceptance_evidence(
        &self,
        batch_id: BatchId,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
        replacements: &BTreeMap<DocumentId, EngineDocument>,
        replacement_heads: &BTreeMap<DocumentId, BTreeSet<BatchId>>,
        candidate_roots: &ScratchRoots,
    ) -> Result<
        (
            Option<BTreeMap<DocumentId, DocumentDependencies>>,
            AcceptedBatchEvidence,
            ScratchRoots,
        ),
        EngineError,
    > {
        let mut changed_documents = BTreeMap::new();
        for (document_id, replacement) in replacements {
            let mut heads = if let Some(heads) = replacement_heads.get(document_id) {
                heads.clone()
            } else {
                self.accepted_document_dependencies(*document_id)?
                    .map(|document| document.direct_dependency_heads().iter().copied().collect())
                    .unwrap_or_default()
            };
            for dependency in &updates[document_id].dependency_heads {
                heads.remove(dependency);
            }
            heads.insert(batch_id);
            let dependencies = DocumentDependencies::new(
                *document_id,
                canonical_peer_counters(&replacement.document().oplog_vv())?,
                heads.into_iter().collect(),
            )?;
            changed_documents.insert(*document_id, dependencies);
        }
        let mut roots = candidate_roots.clone();
        let acceptance_sequence = self
            .next_acceptance_sequence
            .checked_add(1)
            .ok_or_else(|| EngineError::Archive("acceptance sequence overflowed".into()))?;
        let prior_frontier_root = self.accepted_frontier_root.clone();
        let new_document_count = changed_documents.keys().try_fold(
            prior_frontier_root.document_count,
            |count, document_id| -> Result<u64, EngineError> {
                Ok(
                    if self.accepted_document_dependencies(*document_id)?.is_some() {
                        count
                    } else {
                        count.checked_add(1).ok_or_else(|| {
                            EngineError::Archive("accepted document count overflowed".into())
                        })?
                    },
                )
            },
        )?;
        let (post_documents, scratch_root) = if let Some(store) = &self.scratch {
            let records = changed_documents
                .iter()
                .map(|(document_id, dependencies)| {
                    Ok((
                        document_id.as_uuid().as_bytes().to_vec(),
                        Some(encode_accepted_document(dependencies)?),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, EngineError>>()?;
            roots.accepted_frontier_root = store
                .insert_many(
                    &roots.accepted_frontier_root,
                    super::scratch_store::ScratchPageKind::AcceptedFrontier,
                    &records,
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            (None, Some(roots.accepted_frontier_root.clone()))
        } else {
            let mut post_documents = self.accepted_frontier.clone();
            post_documents.extend(changed_documents.clone());
            (Some(post_documents), None)
        };
        let manifest_fingerprint = self
            .archive_fingerprints
            .get(&batch_id)
            .copied()
            .ok_or(EngineError::MissingDependency(batch_id))?;
        let manifest = self.archive[&batch_id].manifest();
        let event_binding_digest = AcceptedBatchEvidence::binding_digest_for(
            batch_id,
            manifest_fingerprint,
            manifest.semantic_effect_digest(),
            manifest.dependency_frontier(),
            manifest.causal_dependency_heads(),
        )?;
        let affected_documents = changed_documents.into_values().collect::<Vec<_>>();
        let post_frontier_root = next_accepted_frontier_root(
            &prior_frontier_root,
            event_binding_digest,
            acceptance_sequence,
            new_document_count,
            &affected_documents,
            scratch_root,
        )?;
        let evidence = AcceptedBatchEvidence {
            schema_version: ACCEPTED_EVIDENCE_SCHEMA_VERSION,
            batch_id,
            manifest_fingerprint,
            event_binding_digest,
            acceptance_sequence,
            prior_frontier_root,
            post_frontier_root,
            affected_documents,
        };
        if let Some(store) = &self.scratch {
            let records = BTreeMap::from([(
                acceptance_sequence.to_be_bytes().to_vec(),
                Some(batch_id.as_uuid().as_bytes().to_vec()),
            )]);
            roots.accepted_sequence_root = store
                .insert_many(
                    &roots.accepted_sequence_root,
                    super::scratch_store::ScratchPageKind::AcceptedSequence,
                    &records,
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
        }
        Ok((post_documents, evidence, roots))
    }

    fn commit_acceptance_evidence(
        &mut self,
        post_documents: Option<BTreeMap<DocumentId, DocumentDependencies>>,
        evidence: AcceptedBatchEvidence,
        roots: ScratchRoots,
    ) {
        self.next_acceptance_sequence = evidence.acceptance_sequence;
        self.accepted_frontier_root = evidence.post_frontier_root.clone();
        if self.scratch.is_some() {
            debug_assert!(post_documents.is_none());
            debug_assert!(self.accepted_frontier.is_empty());
            debug_assert!(self.accepted_sequence.is_empty());
            self.scratch_roots = roots;
        } else {
            self.accepted_frontier = post_documents.expect("inline accepted frontier");
            self.accepted_sequence
                .insert(evidence.acceptance_sequence, evidence.batch_id);
        }
    }

    fn accepted_document_dependencies(
        &self,
        document_id: DocumentId,
    ) -> Result<Option<DocumentDependencies>, EngineError> {
        if let Some(store) = &self.scratch {
            let bytes = store
                .lookup(
                    &self.scratch_roots.accepted_frontier_root,
                    super::scratch_store::ScratchPageKind::AcceptedFrontier,
                    document_id.as_uuid().as_bytes(),
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            return bytes
                .map(|bytes| decode_accepted_document(document_id, &bytes))
                .transpose();
        }
        Ok(self.accepted_frontier.get(&document_id).cloned())
    }

    /// Legacy no-store evidence snapshot. Store-backed engines retain only an
    /// authenticated handle and must be inspected through bounded pages.
    pub fn fatal_evidence(&self) -> Option<&ImmutableHomeEvidence> {
        self.fatal_evidence.as_ref()
    }

    pub fn fatal_evidence_handle(&self) -> Option<FatalEvidenceHandle> {
        self.fatal_handle
    }

    pub fn fatal_evidence_page(
        &self,
        cursor: Option<FatalEvidenceCursor>,
        limit: usize,
    ) -> Result<Option<FatalEvidencePage>, EngineError> {
        let Some(handle) = self.fatal_handle else {
            return Ok(None);
        };
        if let Some(cursor) = &cursor {
            if cursor.conflict_root != handle.conflict_root {
                return Err(EngineError::Archive(
                    "fatal-evidence cursor is bound to another root".into(),
                ));
            }
        }
        let after = cursor.map(|cursor| cursor.after);
        let (conflicts, next_after) = if let Some(evidence) = &self.fatal_evidence {
            if limit == 0 || limit > MAX_FATAL_EVIDENCE_PAGE_CONFLICTS {
                return Err(EngineError::Archive(format!(
                    "fatal-evidence page limit {limit} is outside 1..={MAX_FATAL_EVIDENCE_PAGE_CONFLICTS}"
                )));
            }
            let mut conflicts: Vec<_> = evidence
                .conflicts()
                .iter()
                .filter(|conflict| after.is_none_or(|after| conflict.block_id() > after))
                .take(limit.saturating_add(1))
                .cloned()
                .collect();
            let has_more = conflicts.len() > limit;
            if has_more {
                conflicts.pop();
            }
            let next_after = has_more.then(|| {
                conflicts
                    .last()
                    .expect("nonempty bounded legacy evidence page")
                    .block_id()
            });
            (conflicts, next_after)
        } else {
            let store = self.scratch.as_ref().ok_or_else(|| {
                EngineError::Archive("fatal evidence has no authenticated scratch store".into())
            })?;
            super::evidence_index::page_conflicts(store, &self.scratch_roots, handle, after, limit)
                .map_err(|error| EngineError::Archive(error.to_string()))?
        };
        Ok(Some(FatalEvidencePage {
            conflicts,
            next: next_after.map(|after| FatalEvidenceCursor {
                conflict_root: handle.conflict_root,
                after,
            }),
        }))
    }

    #[cfg(test)]
    pub(crate) fn batch_statuses(&self) -> Result<Vec<(BatchId, BatchDisposition)>, EngineError> {
        self.history_records()?
            .into_iter()
            .map(|(batch_id, status)| -> Result<_, EngineError> {
                let disposition = match status {
                    ArchiveStatus::Staged => BatchDisposition::IncompleteStaged {
                        missing_objects: 0,
                        missing_dependencies: self.missing_dependencies(batch_id)?,
                    },
                    ArchiveStatus::Accepted { no_op, .. } => BatchDisposition::Accepted { no_op },
                    ArchiveStatus::Quarantined => BatchDisposition::Quarantined,
                    ArchiveStatus::Rejected(error) => BatchDisposition::Rejected { error },
                };
                Ok((batch_id, disposition))
            })
            .collect()
    }

    fn workspace_status(&self) -> WorkspaceStatus {
        self.fatal_handle
            .map(WorkspaceStatus::Blocked)
            .unwrap_or(WorkspaceStatus::Operational)
    }

    fn is_blocked(&self) -> bool {
        self.fatal_handle.is_some() || self.fatal_evidence.is_some()
    }

    fn outcome(
        &self,
        batch_id: BatchId,
        disposition: BatchDisposition,
        mut newly_accepted: Vec<AcceptedBatch>,
    ) -> StageOutcome {
        newly_accepted.sort_unstable_by_key(|accepted| accepted.batch_id);
        StageOutcome {
            batch_id,
            disposition,
            newly_accepted,
            status: self.status(),
        }
    }

    pub fn stage_from_store(
        &mut self,
        store: &ObjectStore,
        batch_id: BatchId,
    ) -> Result<StageOutcome, EngineError> {
        self.begin_point_operation();
        if store.workspace_id() != self.workspace_id {
            return Err(EngineError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: store.workspace_id(),
            });
        }
        match store
            .inspect_batch(batch_id)
            .map_err(|error| EngineError::Archive(error.to_string()))?
        {
            BatchInspection::Absent => Ok(self.outcome(
                batch_id,
                BatchDisposition::IncompleteStaged {
                    missing_objects: 1,
                    missing_dependencies: Vec::new(),
                },
                Vec::new(),
            )),
            BatchInspection::Staged { missing, .. } => Ok(self.outcome(
                batch_id,
                BatchDisposition::IncompleteStaged {
                    missing_objects: missing.len(),
                    missing_dependencies: Vec::new(),
                },
                Vec::new(),
            )),
            BatchInspection::Ready(batch) => Ok(self.stage_ready(batch)),
        }
    }

    pub fn stage_archive_batch(&mut self, batch_id: BatchId) -> Result<StageOutcome, EngineError> {
        self.begin_point_operation();
        self.ensure_history_store()?;
        let inspection = self
            .archive_store
            .as_ref()
            .ok_or_else(|| EngineError::Archive("engine has no immutable archive store".into()))?
            .inspect_batch(batch_id)
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        match inspection {
            BatchInspection::Absent => Ok(self.outcome(
                batch_id,
                BatchDisposition::IncompleteStaged {
                    missing_objects: 1,
                    missing_dependencies: Vec::new(),
                },
                Vec::new(),
            )),
            BatchInspection::Staged { missing, .. } => Ok(self.outcome(
                batch_id,
                BatchDisposition::IncompleteStaged {
                    missing_objects: missing.len(),
                    missing_dependencies: Vec::new(),
                },
                Vec::new(),
            )),
            BatchInspection::Ready(batch) => {
                let batch_id = batch.manifest().batch_id();
                let outcome = self.stage_ready_internal(batch, true);
                self.resolve_pending_author(batch_id, &outcome.disposition);
                self.prune_persisted_archive_cache();
                Ok(outcome)
            }
        }
    }

    fn ensure_history_store(&mut self) -> Result<(), EngineError> {
        if self.scratch.is_some() {
            return Ok(());
        }
        Err(self.history_failure.clone().unwrap_or_else(|| {
            EngineError::Archive(
                "store-backed engine has no authenticated run-local scratch".into(),
            )
        }))
    }

    pub fn stage_ready(&mut self, batch: ValidatedBatch) -> StageOutcome {
        let batch_id = batch.manifest().batch_id();
        let outcome = self.stage_ready_internal(batch, false);
        self.resolve_pending_author(batch_id, &outcome.disposition);
        outcome
    }

    fn resolve_pending_author(&self, batch_id: BatchId, disposition: &BatchDisposition) {
        let terminal = !matches!(
            disposition,
            BatchDisposition::IncompleteStaged {
                missing_objects: _,
                missing_dependencies: _,
            }
        );
        if !terminal
            || self
                .pending_author_documents
                .borrow()
                .as_ref()
                .is_none_or(|pending| pending.batch_id != batch_id)
        {
            return;
        }
        let pending = self
            .pending_author_documents
            .borrow_mut()
            .take()
            .expect("matching pending author documents exist");
        drop(pending);
    }

    fn stage_ready_internal(&mut self, batch: ValidatedBatch, persisted: bool) -> StageOutcome {
        self.begin_point_operation();
        let batch_id = batch.manifest().batch_id();
        if persisted && self.scratch.is_some() {
            return self.stage_ready_scratch(batch);
        }
        if let Some(error) = &self.history_failure {
            return self.outcome(
                batch_id,
                BatchDisposition::Rejected {
                    error: error.clone(),
                },
                Vec::new(),
            );
        }
        if let Err(error) = self.check_batch_namespace(&batch) {
            return self.outcome(batch_id, BatchDisposition::Rejected { error }, Vec::new());
        }
        let fingerprint = batch_fingerprint(&batch);
        let existing = match self.cold_history_record(batch_id) {
            Ok(existing) => existing,
            Err(error) => {
                return self.outcome(batch_id, BatchDisposition::Rejected { error }, Vec::new());
            }
        };
        if let Some(existing) = existing {
            if existing.manifest_fingerprint != fingerprint {
                let error = EngineError::BatchCollision(batch_id);
                return self.outcome(batch_id, BatchDisposition::Rejected { error }, Vec::new());
            }
            let disposition = disposition_from_final_status(existing.status, true);
            return self.outcome(batch_id, disposition, Vec::new());
        }
        if let Some(existing_fingerprint) = self.archive_fingerprints.get(&batch_id) {
            if *existing_fingerprint != fingerprint {
                let error = EngineError::BatchCollision(batch_id);
                return self.outcome(batch_id, BatchDisposition::Rejected { error }, Vec::new());
            }
            if matches!(self.statuses.get(&batch_id), Some(ArchiveStatus::Staged))
                && self.is_blocked()
            {
                self.drain_blocked_evidence();
            }
            let disposition = match self.statuses.get(&batch_id).cloned() {
                Some(ArchiveStatus::Rejected(error)) => BatchDisposition::Rejected { error },
                Some(ArchiveStatus::Staged) => self.incomplete_staged_disposition(batch_id),
                Some(ArchiveStatus::Accepted { no_op, .. }) => {
                    BatchDisposition::DuplicateAccepted { no_op }
                }
                Some(ArchiveStatus::Quarantined) => BatchDisposition::Quarantined,
                None => unreachable!("fingerprinted batch has a status"),
            };
            return self.outcome(batch_id, disposition, Vec::new());
        }

        self.archive_fingerprints.insert(batch_id, fingerprint);
        self.archive.insert(batch_id, batch);
        self.statuses.insert(batch_id, ArchiveStatus::Staged);
        if persisted {
            self.persisted_staged.insert(batch_id);
        }
        let accepted = if self.is_blocked() {
            self.drain_blocked_evidence();
            Vec::new()
        } else {
            self.drain_staged()
        };
        if let Some(error) = &self.history_failure {
            return self.outcome(
                batch_id,
                BatchDisposition::Rejected {
                    error: error.clone(),
                },
                Vec::new(),
            );
        }
        let disposition = match self.archive_status(batch_id) {
            Err(error) => BatchDisposition::Rejected { error },
            Ok(Some(ArchiveStatus::Accepted { no_op, .. })) => BatchDisposition::Accepted { no_op },
            Ok(Some(ArchiveStatus::Quarantined)) => BatchDisposition::Quarantined,
            Ok(Some(ArchiveStatus::Rejected(error))) => BatchDisposition::Rejected { error },
            Ok(Some(ArchiveStatus::Staged)) => self.incomplete_staged_disposition(batch_id),
            Ok(None) => unreachable!("newly inserted batch has a status"),
        };
        self.outcome(batch_id, disposition, accepted)
    }

    fn stage_ready_scratch(&mut self, batch: ValidatedBatch) -> StageOutcome {
        let offered_batch_id = batch.manifest().batch_id();
        if let Some(error) = &self.history_failure {
            return self.outcome(
                offered_batch_id,
                BatchDisposition::Rejected {
                    error: error.clone(),
                },
                Vec::new(),
            );
        }
        if let Err(error) = self.check_batch_namespace(&batch) {
            return self.outcome(
                offered_batch_id,
                BatchDisposition::Rejected { error },
                Vec::new(),
            );
        }
        let fingerprint = batch_fingerprint(&batch);
        let store = Arc::clone(self.scratch.as_ref().expect("scratch branch"));
        let direct_dependencies = batch.manifest().causal_dependency_heads().to_vec();
        let staged = super::dependency_queue::stage(
            &store,
            &self.scratch_roots,
            offered_batch_id,
            fingerprint,
            direct_dependencies,
            |dependency| {
                Ok(matches!(
                    self.archive_status(dependency).map_err(|error| {
                        super::dependency_queue::DependencyQueueError::Scratch(error.to_string())
                    })?,
                    Some(ArchiveStatus::Accepted { .. } | ArchiveStatus::Quarantined)
                ))
            },
        );
        let (roots, record, queue_work) = match staged {
            Ok(result) => result,
            Err(error) => {
                let (error, latch_failure) = match error {
                    super::dependency_queue::DependencyQueueError::BatchCollision(batch) => {
                        (EngineError::BatchCollision(batch), false)
                    }
                    other => (EngineError::Archive(other.to_string()), true),
                };
                if latch_failure {
                    self.history_failure = Some(error.clone());
                }
                return self.outcome(
                    offered_batch_id,
                    BatchDisposition::Rejected { error },
                    Vec::new(),
                );
            }
        };
        self.scratch_roots = roots;
        self.record_queue_work(queue_work);

        if record.status() == super::dependency_queue::CompactBatchStatus::Final {
            let status = record
                .final_status()
                .and_then(|bytes| decode_archive_status(bytes).ok())
                .unwrap_or_else(|| {
                    ArchiveStatus::Rejected(EngineError::Archive(
                        "malformed final scratch status".into(),
                    ))
                });
            return self.outcome(
                offered_batch_id,
                disposition_from_final_status(status, true),
                Vec::new(),
            );
        }

        let mut supplied = Some(batch);
        let mut accepted = Vec::new();
        loop {
            let (roots, ready) =
                match super::dependency_queue::pop_ready(&store, &self.scratch_roots) {
                    Ok(result) => result,
                    Err(error) => {
                        self.history_failure = Some(EngineError::Archive(error.to_string()));
                        break;
                    }
                };
            self.scratch_roots = roots;
            let Some(batch_id) = ready else {
                break;
            };
            self.record_drain_candidate_visit();
            let ready_batch = if supplied
                .as_ref()
                .is_some_and(|candidate| candidate.manifest().batch_id() == batch_id)
            {
                supplied.take().expect("matching supplied batch")
            } else {
                let inspection = self
                    .archive_store
                    .as_ref()
                    .expect("scratch engine has archive")
                    .inspect_batch(batch_id)
                    .map_err(|error| EngineError::Archive(error.to_string()));
                match inspection {
                    Ok(BatchInspection::Ready(batch)) => batch,
                    Ok(BatchInspection::Absent | BatchInspection::Staged { .. }) => {
                        self.history_failure = Some(EngineError::Archive(format!(
                            "queued Ready batch {batch_id} is no longer complete"
                        )));
                        break;
                    }
                    Err(error) => {
                        self.history_failure = Some(error);
                        break;
                    }
                }
            };
            let ready_fingerprint = batch_fingerprint(&ready_batch);
            self.archive.insert(batch_id, ready_batch);
            self.archive_fingerprints
                .insert(batch_id, ready_fingerprint);
            self.statuses.insert(batch_id, ArchiveStatus::Staged);

            let dependencies: BTreeSet<_> = self.archive[&batch_id]
                .manifest()
                .causal_dependency_heads()
                .iter()
                .copied()
                .collect();
            let allow_publication = !self.is_blocked();
            let final_status = match self.dependency_status_gate(&dependencies, !allow_publication)
            {
                Err(error) => ArchiveStatus::Rejected(error),
                Ok(false) => {
                    self.history_failure = Some(EngineError::Archive(format!(
                        "ready queue released {batch_id} before its dependencies"
                    )));
                    break;
                }
                Ok(true) => {
                    match super::causal_index::insert_batch(
                        &store,
                        &self.scratch_roots,
                        self.archive[&batch_id].manifest(),
                    ) {
                        Err(error) => {
                            ArchiveStatus::Rejected(EngineError::InvalidCrdt(error.to_string()))
                        }
                        Ok(causal_roots) => {
                            match self.validate_and_apply(
                                batch_id,
                                allow_publication,
                                Some(causal_roots),
                            ) {
                                Ok(BatchApplication::Accepted { no_op, evidence }) => {
                                    accepted.push(AcceptedBatch { batch_id, no_op });
                                    ArchiveStatus::Accepted { no_op, evidence }
                                }
                                Ok(BatchApplication::Quarantined) => ArchiveStatus::Quarantined,
                                Err(error) => ArchiveStatus::Rejected(error),
                            }
                        }
                    }
                }
            };
            let encoded = match encode_archive_status(&final_status) {
                Ok(encoded) => encoded,
                Err(error) => {
                    self.history_failure = Some(error);
                    break;
                }
            };
            match super::dependency_queue::finish(&store, &self.scratch_roots, batch_id, encoded) {
                Ok((roots, _, queue_work)) => {
                    self.scratch_roots = roots;
                    self.record_queue_work(queue_work);
                }
                Err(error) => {
                    self.history_failure = Some(EngineError::Archive(error.to_string()));
                    break;
                }
            }
            self.statuses.remove(&batch_id);
            self.archive_fingerprints.remove(&batch_id);
            self.archive.remove(&batch_id);
        }

        if let Some(error) = &self.history_failure {
            return self.outcome(
                offered_batch_id,
                BatchDisposition::Rejected {
                    error: error.clone(),
                },
                Vec::new(),
            );
        }
        let disposition = match self.archive_status(offered_batch_id) {
            Ok(Some(ArchiveStatus::Accepted { no_op, .. })) => BatchDisposition::Accepted { no_op },
            Ok(Some(ArchiveStatus::Quarantined)) => BatchDisposition::Quarantined,
            Ok(Some(ArchiveStatus::Rejected(error))) => BatchDisposition::Rejected { error },
            Ok(Some(ArchiveStatus::Staged)) => self.incomplete_staged_disposition(offered_batch_id),
            Ok(None) => BatchDisposition::Rejected {
                error: EngineError::Archive("offered batch disappeared from scratch status".into()),
            },
            Err(error) => BatchDisposition::Rejected { error },
        };
        self.outcome(offered_batch_id, disposition, accepted)
    }

    fn prune_persisted_archive_cache(&mut self) {
        if self.archive_store.is_none() {
            return;
        }
        self.archive
            .retain(|batch_id, _| self.statuses.contains_key(batch_id));
    }

    pub fn prepare_transaction(
        &self,
        author: AuthorBatch,
        transaction: &OperationTransaction,
    ) -> Result<PreparedBatch, EngineError> {
        self.begin_point_operation();
        // A pending author buffer is only an optimization for the immediately
        // following stage of that exact prepared batch. Starting any later
        // prepare evicts it before validation, including when the new prepare
        // fails, so stale speculative state can never authorize publication.
        self.pending_author_documents.borrow_mut().take();
        self.ensure_not_blocked()?;
        let mut work_stats = self.history_work.get();
        work_stats.prepare_transactions = work_stats.prepare_transactions.saturating_add(1);
        self.history_work.set(work_stats);
        if transaction.operations.is_empty()
            || transaction.operations.len() > MAX_TRANSACTION_OPERATIONS
        {
            return Err(EngineError::InvalidTransaction(
                "transaction operation count is out of bounds".into(),
            ));
        }
        if author.crdt_peer_id.as_u64() == 0 {
            return Err(EngineError::InvalidTransaction(
                "CRDT peer identity zero is reserved".into(),
            ));
        }

        let mut created_block_ids = BTreeSet::new();
        let mut created_blocks = Vec::new();
        for operation in &transaction.operations {
            if let SemanticOperation::CreateBlock { block, .. } = operation {
                let block_key = block.block_id.as_uuid().as_u128();
                if !created_block_ids.insert(block_key) {
                    return Err(EngineError::BlockAlreadyExists(block.block_id));
                }
                created_blocks.push(block.block_id);
            }
        }
        for (block_key, claims) in self.block_home_claims_many(&created_blocks)? {
            if !claims.is_empty() {
                let block_id = BlockId::from_uuid(uuid::Uuid::from_u128(block_key));
                return Err(EngineError::BlockAlreadyExists(block_id));
            }
        }

        let mut working = BTreeMap::<DocumentId, EngineDocument>::new();
        let mut before_vectors = BTreeMap::<DocumentId, VersionVector>::new();
        let mut before_snapshots = BTreeMap::<DocumentId, SemanticDocumentSnapshot>::new();
        for operation in &transaction.operations {
            self.apply_author_operation(
                &mut working,
                &mut before_vectors,
                &mut before_snapshots,
                author.crdt_peer_id,
                operation,
            )?;
        }

        let affected: Vec<DocumentId> = working.keys().copied().collect();
        let after_snapshots = snapshot_engine_documents(self.catalog_document_id, &working, true)?;
        let effect = derive_effect_from_snapshots(&before_snapshots, &after_snapshots)?;
        let effect_bytes = effect.encode()?;

        let mut frontier_documents = Vec::with_capacity(affected.len());
        let mut affected_heads = BTreeMap::new();
        let mut batch_dependency_heads = BTreeSet::new();
        for document_id in &affected {
            let peer_counters = canonical_peer_counters(
                before_vectors
                    .get(document_id)
                    .expect("affected before vector exists"),
            )?;
            let direct_heads: Vec<_> = self
                .document_dependency_heads(*document_id, false)?
                .into_iter()
                .collect();
            let mut work_stats = self.history_work.get();
            work_stats.prepare_document_head_visits = work_stats
                .prepare_document_head_visits
                .saturating_add(direct_heads.len());
            self.history_work.set(work_stats);
            batch_dependency_heads.extend(direct_heads.iter().copied());
            affected_heads.insert(*document_id, direct_heads.clone());
            if !peer_counters.is_empty() || !direct_heads.is_empty() {
                frontier_documents.push(DocumentDependencies::new(
                    *document_id,
                    peer_counters,
                    direct_heads,
                )?);
            }
        }
        let frontier = FrontierV2::new(frontier_documents)?;
        let batch_dependency_heads: Vec<_> = batch_dependency_heads.into_iter().collect();

        let mut objects = Vec::with_capacity(working.len() + 1);
        objects.push(OperationObject::new(
            self.workspace_id,
            self.catalog_document_id,
            ObjectKind::SemanticEffect,
            effect_bytes.clone(),
        )?);
        for (document_id, document) in &working {
            let document = document.document();
            let before_vector = before_vectors
                .get(document_id)
                .expect("working document has an initial vector");
            let update = document
                .export(ExportMode::updates(before_vector))
                .map_err(|error| EngineError::InvalidCrdt(error.to_string()))?;
            if update.is_empty() {
                return Err(EngineError::InvalidTransaction(format!(
                    "document {document_id} produced an empty CRDT update"
                )));
            }
            objects.push(OperationObject::new(
                self.workspace_id,
                *document_id,
                ObjectKind::CrdtUpdate,
                encode_crdt_update_payload(
                    author.batch_id,
                    *document_id,
                    affected_heads[document_id].clone(),
                    batch_dependency_heads.clone(),
                    frontier
                        .documents()
                        .iter()
                        .find(|dependencies| dependencies.document_id() == *document_id)
                        .map(DocumentDependencies::causal_state_digest),
                    update,
                )?,
            )?);
        }
        let descriptors = objects
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()?;
        let manifest = if let Some(store) = &self.scratch {
            let peer = CausalPeerId::from_device_id(author.author_device_id);
            let (dot, prior_batch) =
                super::causal_index::next_dot(store, &self.scratch_roots, peer)
                    .map_err(|error| EngineError::InvalidTransaction(error.to_string()))?;
            let mut causal_dependency_heads = batch_dependency_heads;
            causal_dependency_heads.extend(prior_batch);
            OperationBatch::new_with_causality(
                self.workspace_id,
                self.lineage_digest,
                author.batch_id,
                author.author_device_id,
                author.author_session_id,
                dot,
                causal_dependency_heads,
                frontier,
                SemanticEffectDigest::of(&effect_bytes),
                descriptors,
            )?
        } else {
            let peer = CausalPeerId::from_device_id(author.author_device_id);
            let prior = self.ephemeral_causal_chain.borrow().get(&peer).copied();
            let counter = prior
                .map(|(counter, _)| counter)
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| EngineError::InvalidTransaction("causal counter overflow".into()))?;
            let mut causal_dependency_heads = batch_dependency_heads;
            causal_dependency_heads.extend(prior.map(|(_, batch_id)| batch_id));
            OperationBatch::new_with_causality(
                self.workspace_id,
                self.lineage_digest,
                author.batch_id,
                author.author_device_id,
                author.author_session_id,
                BatchCausalDot::new(peer, counter)?,
                causal_dependency_heads,
                frontier,
                SemanticEffectDigest::of(&effect_bytes),
                descriptors,
            )?
        };
        let prepared = PreparedBatch::new(manifest, objects).map_err(EngineError::from)?;
        if self.scratch.is_none() {
            *self.pending_author_documents.borrow_mut() = Some(PendingAuthorDocuments {
                batch_id: author.batch_id,
                manifest_fingerprint: prepared_manifest_fingerprint(&prepared),
                documents: working
                    .into_iter()
                    .map(|(document_id, document)| {
                        let EngineDocument::InMemory(document) = document else {
                            unreachable!("no-store authoring created an external document")
                        };
                        (document_id, document)
                    })
                    .collect(),
            });
        }
        Ok(prepared)
    }

    pub fn materialize_page(&self, page_id: PageId) -> Result<MaterializedPage, EngineError> {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let reads_before = self.archive_read_stats();
        let catalog = self
            .visible_documents
            .get(&self.catalog_document_id)
            .ok_or(EngineError::PageNotFound(page_id))?;
        validate_catalog(self.catalog_document_id, catalog)?;
        let page_state =
            read_page_state(catalog, page_id)?.ok_or(EngineError::PageNotFound(page_id))?;
        let PageState::Live {
            path,
            home_document_id: page_document_id,
        } = page_state
        else {
            return Err(EngineError::PageDeleted(page_id));
        };
        let page_document = self.clone_visible_document(page_document_id, 1)?;
        validate_shard(self.catalog_document_id, page_document_id, &page_document)?;
        if shard_page_id(&page_document)? != Some(page_id) {
            return Err(EngineError::MalformedDocument {
                document_id: page_document_id,
                reason: "membership shard page identity mismatch".into(),
            });
        }

        let members = read_memberships(page_document_id, &page_document)?;
        let mut by_home = BTreeMap::<DocumentId, Vec<(BlockId, MembershipClaim)>>::new();
        for (block_id, claim) in members {
            by_home
                .entry(claim.home_document_id)
                .or_default()
                .push((block_id, claim));
        }
        let mut blocks = Vec::new();
        for (home_document_id, claims) in &by_home {
            if *home_document_id == page_document_id {
                validate_shard(self.catalog_document_id, *home_document_id, &page_document)?;
                for (block_id, claim) in claims {
                    let Some(state) =
                        read_block_state(*home_document_id, &page_document, *block_id)?
                    else {
                        return Err(EngineError::MalformedDocument {
                            document_id: *home_document_id,
                            reason: format!("membership references missing block {block_id}"),
                        });
                    };
                    if state.owner == BlockOwner::Page(page_id) {
                        blocks.push(MaterializedBlock {
                            block_id: *block_id,
                            home_document_id: *home_document_id,
                            parent: claim.parent,
                            order: claim.order.clone(),
                            content: state.content,
                        });
                    }
                }
                continue;
            }
            let home = self.clone_visible_document(*home_document_id, 1)?;
            validate_shard(self.catalog_document_id, *home_document_id, &home)?;
            for (block_id, claim) in claims {
                let Some(state) = read_block_state(*home_document_id, &home, *block_id)? else {
                    return Err(EngineError::MalformedDocument {
                        document_id: *home_document_id,
                        reason: format!("membership references missing block {block_id}"),
                    });
                };
                if state.owner == BlockOwner::Page(page_id) {
                    blocks.push(MaterializedBlock {
                        block_id: *block_id,
                        home_document_id: *home_document_id,
                        parent: claim.parent,
                        order: claim.order.clone(),
                        content: state.content,
                    });
                }
            }
        }
        blocks.sort_unstable_by(|left, right| {
            (&left.order, left.block_id).cmp(&(&right.order, right.block_id))
        });
        let reads_after = self.archive_read_stats();
        Ok(MaterializedPage {
            page_id,
            path,
            blocks,
            stats: MaterializationStats {
                catalog_documents_loaded: 1,
                membership_documents_loaded: 1,
                home_documents_loaded: by_home.len(),
                distinct_home_documents: by_home.keys().copied().collect(),
                physical_manifest_reads: reads_after
                    .manifest_reads
                    .saturating_sub(reads_before.manifest_reads),
                physical_object_reads: reads_after
                    .object_reads
                    .saturating_sub(reads_before.object_reads),
            },
        })
    }

    pub fn canonical_snapshot(&self) -> Result<super::CanonicalSnapshot, EngineError> {
        self.ensure_not_blocked()?;
        let Some(catalog) = self.visible_documents.get(&self.catalog_document_id) else {
            return Ok(super::CanonicalSnapshot::default());
        };
        let all_pages = validate_catalog(self.catalog_document_id, catalog)?;
        let mut pages = Vec::new();
        let mut blocks = Vec::new();
        let mut memberships = Vec::new();
        let mut paths = BTreeMap::<ManagedPath, Vec<PageId>>::new();
        for (page_id, state) in all_pages {
            if let PageState::Live { path, .. } = &state {
                paths.entry(path.clone()).or_default().push(page_id);
                for block in self.materialize_page(page_id)?.blocks {
                    memberships.push(super::VisibleMembership {
                        page_id,
                        block_id: block.block_id,
                        home_document_id: block.home_document_id,
                        parent: block.parent,
                        order: block.order.clone(),
                    });
                    blocks.push(BlockState {
                        block_id: block.block_id,
                        home_document_id: block.home_document_id,
                        owner: BlockOwner::Page(page_id),
                        content: block.content,
                    });
                }
                pages.push((page_id, state));
            }
        }
        blocks.sort_unstable_by_key(|state| (state.home_document_id, state.block_id));
        memberships.sort_unstable_by_key(|membership| (membership.page_id, membership.block_id));
        let path_conflicts = paths
            .into_iter()
            .filter_map(|(path, mut page_ids)| {
                if page_ids.len() > 1 {
                    page_ids.sort_unstable();
                    Some((path, page_ids))
                } else {
                    None
                }
            })
            .collect();
        Ok(super::CanonicalSnapshot {
            pages,
            blocks,
            memberships,
            path_conflicts,
        })
    }

    /// Recovery/debug view of immutable home content, including content whose
    /// owner is tombstoned and therefore absent from normal materialization.
    pub fn recover_block_state(
        &self,
        home_document_id: DocumentId,
        block_id: BlockId,
    ) -> Result<Option<BlockState>, EngineError> {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let home = self.clone_visible_document(home_document_id, 1)?;
        validate_shard(self.catalog_document_id, home_document_id, &home)?;
        read_block_state(home_document_id, &home, block_id)
    }

    fn check_batch_namespace(&self, batch: &ValidatedBatch) -> Result<(), EngineError> {
        let manifest = batch.manifest();
        if manifest.workspace_id() != self.workspace_id {
            return Err(EngineError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        if manifest.lineage_digest() != self.lineage_digest {
            return Err(EngineError::LineageMismatch {
                expected: self.lineage_digest,
                found: manifest.lineage_digest(),
            });
        }
        Ok(())
    }

    fn ensure_not_blocked(&self) -> Result<(), EngineError> {
        if let Some(error) = &self.history_failure {
            return Err(error.clone());
        }
        match self.fatal_handle {
            Some(handle) => Err(EngineError::WorkspaceBlocked(handle)),
            None if self.fatal_evidence.is_some() => {
                Err(EngineError::WorkspaceBlocked(in_memory_evidence_handle(
                    self.fatal_evidence
                        .as_ref()
                        .expect("checked in-memory fatal evidence"),
                )))
            }
            None => Ok(()),
        }
    }

    fn drain_staged(&mut self) -> Vec<AcceptedBatch> {
        let mut accepted = Vec::new();
        'drain: loop {
            if self.is_blocked() || self.history_failure.is_some() {
                break;
            }
            let staged: Vec<BatchId> = self
                .statuses
                .iter()
                .filter_map(|(batch_id, status)| {
                    matches!(status, ArchiveStatus::Staged).then_some(*batch_id)
                })
                .collect();
            let mut progressed = false;
            for batch_id in staged {
                self.record_drain_candidate_visit();
                let frontier = self.archive[&batch_id].manifest().dependency_frontier();
                if frontier_contains_batch(frontier, batch_id) {
                    self.set_final_status(
                        batch_id,
                        ArchiveStatus::Rejected(EngineError::SelfDependency(batch_id)),
                    );
                    progressed = true;
                    continue;
                }
                let updates = match self.decoded_updates(batch_id) {
                    Ok(updates) => updates,
                    Err(error) => {
                        self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                        progressed = true;
                        continue;
                    }
                };
                if let Err(error) = self.validate_dependency_witnesses(frontier, &updates) {
                    self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                    progressed = true;
                    continue;
                }
                let fast_ready =
                    match self.dependency_witnesses_are_current(frontier, &updates, false) {
                        Ok(ready) => ready,
                        Err(error) => {
                            self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                            progressed = true;
                            continue;
                        }
                    };
                let dependencies = (!fast_ready).then(|| self.declared_dependencies(batch_id));
                if let Some(dependencies) = &dependencies {
                    match self.dependency_status_gate(dependencies, false) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(error) => {
                            self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                            progressed = true;
                            continue;
                        }
                    }
                }
                match self.validate_and_apply(batch_id, true, None) {
                    Ok(BatchApplication::Accepted { no_op, evidence }) => {
                        self.set_final_status(
                            batch_id,
                            ArchiveStatus::Accepted { no_op, evidence },
                        );
                        accepted.push(AcceptedBatch { batch_id, no_op });
                    }
                    Ok(BatchApplication::Quarantined) => {
                        self.set_final_status(batch_id, ArchiveStatus::Quarantined);
                        break 'drain;
                    }
                    Err(error) => {
                        self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                    }
                }
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        if self.is_blocked() {
            self.drain_blocked_evidence();
        }
        accepted
    }

    /// Validate already offered Ready batches after the terminal latch without
    /// publishing any replacement. Accepted and validated-unpublished parents
    /// are both eligible, and the loop reaches a deterministic fixed point.
    fn drain_blocked_evidence(&mut self) {
        loop {
            if self.history_failure.is_some() {
                break;
            }
            let staged: Vec<BatchId> = self
                .statuses
                .iter()
                .filter_map(|(batch_id, status)| {
                    matches!(status, ArchiveStatus::Staged).then_some(*batch_id)
                })
                .collect();
            let mut progressed = false;
            for batch_id in staged {
                self.record_drain_candidate_visit();
                let frontier = self.archive[&batch_id].manifest().dependency_frontier();
                if frontier_contains_batch(frontier, batch_id) {
                    self.set_final_status(
                        batch_id,
                        ArchiveStatus::Rejected(EngineError::SelfDependency(batch_id)),
                    );
                    progressed = true;
                    continue;
                }
                let updates = match self.decoded_updates(batch_id) {
                    Ok(updates) => updates,
                    Err(error) => {
                        self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                        progressed = true;
                        continue;
                    }
                };
                if let Err(error) = self.validate_dependency_witnesses(frontier, &updates) {
                    self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                    progressed = true;
                    continue;
                }
                let fast_ready =
                    match self.dependency_witnesses_are_current(frontier, &updates, true) {
                        Ok(ready) => ready,
                        Err(error) => {
                            self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                            progressed = true;
                            continue;
                        }
                    };
                let dependencies = (!fast_ready).then(|| self.declared_dependencies(batch_id));
                if let Some(dependencies) = &dependencies {
                    match self.dependency_status_gate(dependencies, true) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(error) => {
                            self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                            progressed = true;
                            continue;
                        }
                    }
                }
                match self.validate_and_apply(batch_id, false, None) {
                    Ok(_) => {
                        self.set_final_status(batch_id, ArchiveStatus::Quarantined);
                    }
                    Err(error) => {
                        self.set_final_status(batch_id, ArchiveStatus::Rejected(error));
                    }
                }
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
    }

    fn incomplete_staged_disposition(&mut self, batch_id: BatchId) -> BatchDisposition {
        match self.missing_dependencies(batch_id) {
            Ok(missing_dependencies) => BatchDisposition::IncompleteStaged {
                missing_objects: 0,
                missing_dependencies,
            },
            Err(error) => {
                self.history_failure = Some(error.clone());
                BatchDisposition::Rejected { error }
            }
        }
    }

    fn missing_dependencies(&self, batch_id: BatchId) -> Result<Vec<BatchId>, EngineError> {
        let dependencies = if let Some(store) = &self.scratch {
            super::dependency_queue::lookup(store, &self.scratch_roots, batch_id)
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .ok_or_else(|| {
                    EngineError::Archive(format!("missing staged dependency record for {batch_id}"))
                })?
                .direct_dependencies()
                .to_vec()
        } else {
            self.declared_dependencies(batch_id).into_iter().collect()
        };
        let mut missing = Vec::new();
        for dependency in dependencies {
            if !matches!(
                self.archive_status(dependency)?,
                Some(ArchiveStatus::Accepted { .. } | ArchiveStatus::Quarantined)
            ) {
                missing.push(dependency);
            }
        }
        Ok(missing)
    }

    fn dependency_status_gate(
        &self,
        dependencies: &BTreeSet<BatchId>,
        allow_quarantined: bool,
    ) -> Result<bool, EngineError> {
        for dependency in dependencies {
            match self.archive_status(*dependency)? {
                Some(ArchiveStatus::Accepted { .. }) => {}
                Some(ArchiveStatus::Quarantined) if allow_quarantined => {}
                Some(ArchiveStatus::Rejected(_)) => {
                    return Err(EngineError::RejectedDependency(*dependency));
                }
                Some(ArchiveStatus::Staged) | Some(ArchiveStatus::Quarantined) | None => {
                    return Ok(false)
                }
            }
        }
        Ok(true)
    }

    fn set_final_status(&mut self, batch_id: BatchId, status: ArchiveStatus) {
        if matches!(
            status,
            ArchiveStatus::Accepted { .. } | ArchiveStatus::Quarantined
        ) {
            if let Some(batch) = self.archive.get(&batch_id) {
                let dot = batch.manifest().causal_dot();
                let mut chain = self.ephemeral_causal_chain.borrow_mut();
                let entry = chain.entry(dot.peer_id()).or_insert((0, batch_id));
                if dot.counter() >= entry.0 {
                    *entry = (dot.counter(), batch_id);
                }
            }
        }
        self.statuses.insert(batch_id, status.clone());
        if !self.persisted_staged.contains(&batch_id) {
            return;
        }
        let Some(store) = &self.history_store else {
            return;
        };
        let Some(manifest_fingerprint) = self.archive_fingerprints.get(&batch_id).copied() else {
            return;
        };
        let generation = self.history_generation.saturating_add(1);
        let record = new_history_record(generation, batch_id, manifest_fingerprint, status);
        let Ok(bytes) = encode_history_record(&record) else {
            return;
        };
        let history_root = match store.insert(self.history_root, batch_id, &bytes) {
            Ok(root) => root,
            Err(error) => {
                self.history_failure = Some(EngineError::Archive(error.to_string()));
                return;
            }
        };
        self.history_generation = generation;
        self.history_root = history_root;
        let mut point_cache = self.status_point_cache.borrow_mut();
        point_cache.clear();
        point_cache.insert(batch_id, Some(record));
        drop(point_cache);
        self.persisted_staged.remove(&batch_id);
        self.statuses.remove(&batch_id);
        self.archive_fingerprints.remove(&batch_id);
        self.archive.remove(&batch_id);
    }

    fn archive_status(&self, batch_id: BatchId) -> Result<Option<ArchiveStatus>, EngineError> {
        let mut work = self.history_work.get();
        work.dependency_status_lookups = work.dependency_status_lookups.saturating_add(1);
        self.history_work.set(work);
        if let Some(status) = self.statuses.get(&batch_id) {
            return Ok(Some(status.clone()));
        }
        Ok(self
            .cold_history_record(batch_id)?
            .map(|record| record.status))
    }

    fn cold_history_record(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<ColdHistoryRecord>, EngineError> {
        if let Some(store) = &self.scratch {
            let Some(record) =
                super::dependency_queue::lookup(store, &self.scratch_roots, batch_id)
                    .map_err(|error| EngineError::Archive(error.to_string()))?
            else {
                return Ok(None);
            };
            let status = match record.status() {
                super::dependency_queue::CompactBatchStatus::Final => {
                    decode_archive_status(record.final_status().ok_or_else(|| {
                        EngineError::Archive("final scratch status has no result".into())
                    })?)?
                }
                super::dependency_queue::CompactBatchStatus::Waiting
                | super::dependency_queue::CompactBatchStatus::Ready
                | super::dependency_queue::CompactBatchStatus::Processing => ArchiveStatus::Staged,
            };
            return Ok(Some(ColdHistoryRecord {
                schema_version: ENGINE_HISTORY_SCHEMA_VERSION,
                generation: 0,
                batch_id,
                manifest_fingerprint: record.manifest_fingerprint(),
                status,
            }));
        }
        let Some(store) = &self.history_store else {
            return Ok(None);
        };
        if let Some(error) = &self.history_failure {
            return Err(error.clone());
        }
        if let Some(record) = self.status_point_cache.borrow().get(&batch_id) {
            return Ok(record.clone());
        }
        let bytes = store
            .lookup(self.history_root, batch_id)
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        if bytes.is_some() {
            store.note_history_decode();
        }
        let record = bytes
            .map(|bytes| decode_history_record(batch_id, &bytes))
            .transpose()?;
        self.status_point_cache
            .borrow_mut()
            .insert(batch_id, record.clone());
        Ok(record)
    }

    fn begin_point_operation(&self) {
        self.status_point_cache.borrow_mut().clear();
        self.external_anchor_point_cache.borrow_mut().clear();
    }

    #[cfg(test)]
    fn history_records(&self) -> Result<Vec<(BatchId, ArchiveStatus)>, EngineError> {
        let mut records = if let Some(store) = &self.scratch {
            super::dependency_queue::all_records(store, &self.scratch_roots)
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .into_iter()
                .map(|record| {
                    let status = match record.status() {
                        super::dependency_queue::CompactBatchStatus::Final => {
                            decode_archive_status(record.final_status().ok_or_else(|| {
                                EngineError::Archive("final scratch status has no result".into())
                            })?)?
                        }
                        super::dependency_queue::CompactBatchStatus::Waiting
                        | super::dependency_queue::CompactBatchStatus::Ready
                        | super::dependency_queue::CompactBatchStatus::Processing => {
                            ArchiveStatus::Staged
                        }
                    };
                    Ok((record.batch_id(), status))
                })
                .collect::<Result<Vec<_>, EngineError>>()?
        } else if let Some(store) = &self.history_store {
            validated_history_records(store, self.history_generation, self.history_root)?
                .into_iter()
                .map(|record| (record.batch_id, record.status))
                .collect()
        } else {
            Vec::new()
        };
        records.extend(
            self.statuses
                .iter()
                .map(|(batch_id, status)| (*batch_id, status.clone())),
        );
        records.sort_unstable_by_key(|(batch_id, _)| *batch_id);
        Ok(records)
    }

    fn record_drain_candidate_visit(&self) {
        let mut work = self.history_work.get();
        work.drain_candidate_visits = work.drain_candidate_visits.saturating_add(1);
        self.history_work.set(work);
    }

    fn record_queue_work(&self, queue: super::dependency_queue::QueueWork) {
        let mut work = self.history_work.get();
        work.wait_edge_visits = work.wait_edge_visits.saturating_add(queue.wait_edge_visits);
        work.ready_queue_residency = work.ready_queue_residency.max(queue.ready_queue_residency);
        self.history_work.set(work);
    }

    fn record_document_state_work(&self, document: super::document_state::DocumentStateWork) {
        let mut work = self.history_work.get();
        work.document_point_reads = work
            .document_point_reads
            .saturating_add(document.document_point_reads);
        work.state_page_bytes_read = work
            .state_page_bytes_read
            .saturating_add(document.state_page_bytes_read);
        work.state_page_bytes_written = work
            .state_page_bytes_written
            .saturating_add(document.state_page_bytes_written);
        work.external_flushes = work
            .external_flushes
            .saturating_add(document.external_flushes);
        work.external_point_reads = work
            .external_point_reads
            .saturating_add(document.external_point_reads);
        work.external_range_scans = work
            .external_range_scans
            .saturating_add(document.external_range_scans);
        work.external_history_page_reads = work
            .external_history_page_reads
            .saturating_add(document.external_history_page_reads);
        work.external_history_blob_reads = work
            .external_history_blob_reads
            .saturating_add(document.external_history_blob_reads);
        self.history_work.set(work);
    }

    fn record_author_snapshot_clone(&self, document: &LoroDoc) {
        let mut work = self.history_work.get();
        work.author_snapshot_clones = work.author_snapshot_clones.saturating_add(1);
        work.author_snapshot_clone_ops = work.author_snapshot_clone_ops.saturating_add(
            document
                .oplog_vv()
                .values()
                .filter_map(|end| usize::try_from((*end).max(0)).ok())
                .sum::<usize>(),
        );
        self.history_work.set(work);
    }

    fn record_stage_snapshot_clone(&self, document: &LoroDoc) {
        let mut work = self.history_work.get();
        work.stage_snapshot_clones = work.stage_snapshot_clones.saturating_add(1);
        work.stage_snapshot_clone_ops = work.stage_snapshot_clone_ops.saturating_add(
            document
                .oplog_vv()
                .values()
                .filter_map(|end| usize::try_from((*end).max(0)).ok())
                .sum::<usize>(),
        );
        self.history_work.set(work);
    }

    fn declared_dependencies(&self, batch_id: BatchId) -> BTreeSet<BatchId> {
        self.archive
            .get(&batch_id)
            .map(|batch| {
                batch
                    .manifest()
                    .dependency_frontier()
                    .documents()
                    .iter()
                    .flat_map(|document| document.direct_dependency_heads().iter().copied())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn decoded_updates(
        &self,
        batch_id: BatchId,
    ) -> Result<BTreeMap<DocumentId, CrdtUpdatePayload>, EngineError> {
        let batch = self
            .archive
            .get(&batch_id)
            .ok_or(EngineError::MissingDependency(batch_id))?;
        let mut updates = BTreeMap::new();
        for object in batch.objects() {
            if object.kind() != ObjectKind::CrdtUpdate {
                continue;
            }
            let payload =
                decode_crdt_update_payload(batch_id, object.document_id(), object.payload())?;
            if updates.insert(object.document_id(), payload).is_some() {
                return Err(EngineError::DuplicateDocumentUpdate(object.document_id()));
            }
        }
        Ok(updates)
    }

    fn validate_dependency_witnesses(
        &self,
        frontier: &FrontierV2,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
    ) -> Result<(), EngineError> {
        let declared_batch_heads: Vec<_> = frontier
            .documents()
            .iter()
            .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut batch_witness = None;
        for (document_id, update) in updates {
            if batch_witness.get_or_insert_with(|| update.batch_dependency_heads.clone())
                != &update.batch_dependency_heads
            {
                return Err(EngineError::InvalidCrdt(
                    "batch dependency witnesses disagree within one atomic batch".into(),
                ));
            }
            let dependencies = frontier
                .documents()
                .iter()
                .find(|dependencies| dependencies.document_id() == *document_id);
            let declared_document_heads = dependencies
                .map(DocumentDependencies::direct_dependency_heads)
                .unwrap_or_default();
            if dependencies.map(DocumentDependencies::causal_state_digest)
                != update.causal_state_digest
                || update.dependency_heads != declared_document_heads
                || update.batch_dependency_heads != declared_batch_heads
            {
                return Err(EngineError::CausalWitnessMismatch {
                    document_id: *document_id,
                });
            }
        }
        if updates.is_empty() && !declared_batch_heads.is_empty() {
            return Err(EngineError::InvalidCrdt(
                "dependency frontier exists without a CRDT update witness".into(),
            ));
        }
        Ok(())
    }

    fn dependency_witnesses_are_current(
        &self,
        frontier: &FrontierV2,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
        allow_quarantined: bool,
    ) -> Result<bool, EngineError> {
        for dependencies in frontier.documents() {
            let document_id = dependencies.document_id();
            let current_heads = if self.is_blocked() {
                self.terminal_document_heads
                    .get(&document_id)
                    .or_else(|| self.visible_document_heads.get(&document_id))
            } else {
                self.visible_document_heads.get(&document_id)
            };
            let heads_match = current_heads
                .into_iter()
                .flatten()
                .copied()
                .eq(dependencies.direct_dependency_heads().iter().copied());
            if !heads_match {
                return Ok(false);
            }
        }
        for (document_id, update) in updates {
            if frontier
                .documents()
                .iter()
                .any(|dependencies| dependencies.document_id() == *document_id)
            {
                continue;
            }
            if !update.dependency_heads.is_empty()
                || !self
                    .document_dependency_heads(*document_id, self.is_blocked())?
                    .is_empty()
            {
                return Ok(false);
            }
        }
        for head in updates
            .values()
            .next()
            .into_iter()
            .flat_map(|update| &update.batch_dependency_heads)
        {
            match self.archive_status(*head)? {
                Some(ArchiveStatus::Accepted { .. }) => {}
                Some(ArchiveStatus::Quarantined) if allow_quarantined => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn current_frontier_documents(
        &self,
        frontier: &FrontierV2,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
    ) -> Result<Option<BTreeMap<DocumentId, EngineDocument>>, EngineError> {
        self.validate_dependency_witnesses(frontier, updates)?;
        if let Some(store) = &self.scratch {
            let mut documents = BTreeMap::new();
            for dependencies in frontier.documents() {
                let lane = if self.is_blocked() {
                    super::document_state::DocumentLane::Terminal
                } else {
                    super::document_state::DocumentLane::Visible
                };
                let mut loaded = super::document_state::load_external_exact(
                    store,
                    &self.scratch_roots,
                    lane,
                    dependencies.document_id(),
                    dependencies.causal_state_digest(),
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
                if loaded.is_none() && lane == super::document_state::DocumentLane::Terminal {
                    loaded = super::document_state::load_external_exact(
                        store,
                        &self.scratch_roots,
                        super::document_state::DocumentLane::Visible,
                        dependencies.document_id(),
                        dependencies.causal_state_digest(),
                    )
                    .map_err(|error| EngineError::Archive(error.to_string()))?;
                }
                let Some((record, document, state_work)) = loaded else {
                    return Err(EngineError::FrontierVectorMismatch(
                        dependencies.document_id(),
                    ));
                };
                self.record_document_state_work(state_work);
                self.validate_external_record_anchor(dependencies.document_id(), &record)?;
                if record.peer_counters() != dependencies.peer_counters()
                    || record.exact_direct_heads() != dependencies.direct_dependency_heads()
                {
                    return Err(EngineError::FrontierVectorMismatch(
                        dependencies.document_id(),
                    ));
                }
                documents.insert(
                    dependencies.document_id(),
                    EngineDocument::External(document),
                );
            }
            for (document_id, update) in updates {
                if documents.contains_key(document_id) {
                    continue;
                }
                if !update.dependency_heads.is_empty() {
                    return Err(EngineError::CausalWitnessMismatch {
                        document_id: *document_id,
                    });
                }
                let current = super::document_state::load_external_current(
                    store,
                    &self.scratch_roots,
                    if self.is_blocked() {
                        super::document_state::DocumentLane::Terminal
                    } else {
                        super::document_state::DocumentLane::Visible
                    },
                    *document_id,
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
                if let Some((record, _, state_work)) = current {
                    self.record_document_state_work(state_work);
                    self.validate_external_record_anchor(*document_id, &record)?;
                    return Err(EngineError::FrontierVectorMismatch(*document_id));
                }
                documents.insert(
                    *document_id,
                    EngineDocument::External(
                        super::document_state::ExternalDocument::empty(Arc::clone(store))
                            .map_err(|error| EngineError::Archive(error.to_string()))?,
                    ),
                );
            }
            return Ok(Some(documents));
        }
        if !self.dependency_witnesses_are_current(frontier, updates, self.is_blocked())? {
            return Ok(None);
        }
        let mut documents = BTreeMap::new();
        for dependencies in frontier.documents() {
            let document = self.clone_validation_document(dependencies.document_id(), 1)?;
            self.record_stage_snapshot_clone(&document);
            if canonical_peer_counters(&document.oplog_vv())? != dependencies.peer_counters() {
                return Ok(None);
            }
            documents.insert(
                dependencies.document_id(),
                EngineDocument::InMemory(document),
            );
        }
        for document_id in updates.keys() {
            if documents.contains_key(document_id) {
                continue;
            }
            let document = self.clone_validation_document(*document_id, 1)?;
            self.record_stage_snapshot_clone(&document);
            if !document.oplog_vv().is_empty() {
                return Ok(None);
            }
            documents.insert(*document_id, EngineDocument::InMemory(document));
        }
        Ok(Some(documents))
    }

    fn validate_and_apply(
        &mut self,
        batch_id: BatchId,
        allow_publication: bool,
        candidate_roots: Option<ScratchRoots>,
    ) -> Result<BatchApplication, EngineError> {
        #[cfg(test)]
        let mut phase_started = Instant::now();
        let batch = self
            .archive
            .get(&batch_id)
            .expect("staged archive batch exists");
        self.check_batch_namespace(batch)?;
        let frontier = batch.manifest().dependency_frontier().clone();

        let mut updates = BTreeMap::<DocumentId, CrdtUpdatePayload>::new();
        let mut semantic_payload = None;
        for object in batch.objects() {
            match object.kind() {
                ObjectKind::SemanticEffect => semantic_payload = Some(object.payload().to_vec()),
                ObjectKind::CrdtUpdate => {
                    let update = decode_crdt_update_payload(
                        batch_id,
                        object.document_id(),
                        object.payload(),
                    )?;
                    if updates.insert(object.document_id(), update).is_some() {
                        return Err(EngineError::DuplicateDocumentUpdate(object.document_id()));
                    }
                }
                ObjectKind::ProjectionIntent | ObjectKind::AnnotatedBaseBlob => {}
            }
        }
        self.validate_dependency_witnesses(&frontier, &updates)?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[0] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let manifest_fingerprint = self.archive_fingerprints.get(&batch_id).copied();
        let pending_documents = self
            .pending_author_documents
            .borrow()
            .as_ref()
            .filter(|pending| {
                pending.batch_id == batch_id
                    && Some(pending.manifest_fingerprint) == manifest_fingerprint
            })
            .map(|pending| {
                pending
                    .documents
                    .iter()
                    .map(|(document_id, document)| (*document_id, document.clone()))
                    .collect::<BTreeMap<_, _>>()
            });
        if let Some(pending_documents) = pending_documents {
            if let Ok(Some(application)) = self.validate_and_apply_pending_author(
                batch_id,
                batch.manifest().causal_dot(),
                allow_publication,
                &frontier,
                &updates,
                semantic_payload
                    .as_deref()
                    .expect("Ready batch has one semantic effect"),
                pending_documents,
            ) {
                return Ok(application);
            }
            // Pending author state is an untrusted optimization. Any mismatch,
            // stale frontier, malformed buffer, or validation uncertainty
            // discards that route and continues through immutable update
            // reconstruction below.
        }
        let mut before = match self.current_frontier_documents(&frontier, &updates)? {
            Some(documents) => documents,
            None if self.scratch.is_none() => self
                .reconstruct_frontier(&frontier)?
                .into_iter()
                .map(|(document_id, document)| (document_id, EngineDocument::InMemory(document)))
                .collect(),
            None => {
                return Err(EngineError::Archive(
                    "exact document checkpoint unexpectedly unavailable".into(),
                ))
            }
        };
        #[cfg(test)]
        {
            self.validation_phase_nanos[1] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let semantic_payload = semantic_payload.expect("Ready batch has one semantic effect");
        let declared_effect = SemanticEffect::decode(&semantic_payload)?;
        for document_id in updates.keys() {
            if !before.contains_key(document_id) {
                let document = if let Some(store) = &self.scratch {
                    EngineDocument::External(
                        super::document_state::ExternalDocument::empty(Arc::clone(store))
                            .map_err(|error| EngineError::Archive(error.to_string()))?,
                    )
                } else {
                    EngineDocument::InMemory(LoroDoc::new())
                };
                before.insert(*document_id, document);
            }
        }
        let exact_before_vectors = before
            .iter()
            .map(|(document_id, document)| (*document_id, document.document().oplog_vv()))
            .collect::<BTreeMap<_, _>>();
        let new_exact_shard_candidates = updates
            .keys()
            .copied()
            .filter(|document_id| {
                *document_id != self.catalog_document_id
                    && exact_before_vectors[document_id].is_empty()
            })
            .collect::<BTreeSet<_>>();
        for document_id in &new_exact_shard_candidates {
            prime_empty_shard_roots(before[document_id].document());
        }
        let exact_before_page_ids = before
            .iter()
            .filter(|(document_id, _)| **document_id != self.catalog_document_id)
            .map(|(document_id, document)| Ok((*document_id, shard_page_id(document.document())?)))
            .collect::<Result<BTreeMap<_, _>, EngineError>>()?;
        let before_snapshots = snapshot_engine_documents_excluding(
            self.catalog_document_id,
            &before,
            false,
            &new_exact_shard_candidates,
        )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[2] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let mut after = BTreeMap::new();
        if self.scratch.is_some() {
            for (document_id, document) in std::mem::take(&mut before) {
                if let Some(update) = updates.get(&document_id) {
                    validate_update_base(document_id, document.document(), &update.raw_update)?;
                    import_complete(
                        document_id,
                        document.document(),
                        std::slice::from_ref(&update.raw_update),
                    )?;
                }
                after.insert(document_id, document);
            }
        } else {
            for (document_id, before_document) in &before {
                let document = clone_doc(before_document.document(), 1)?;
                self.record_stage_snapshot_clone(&document);
                if let Some(update) = updates.get(document_id) {
                    validate_update_base(
                        *document_id,
                        before_document.document(),
                        &update.raw_update,
                    )?;
                    import_complete(
                        *document_id,
                        &document,
                        std::slice::from_ref(&update.raw_update),
                    )?;
                }
                after.insert(*document_id, EngineDocument::InMemory(document));
            }
        }
        #[cfg(test)]
        {
            self.validation_phase_nanos[3] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let validated_new_shards = validate_new_exact_shards_against_declared(
            self.catalog_document_id,
            &after,
            &new_exact_shard_candidates,
            &declared_effect,
        )?;
        let after_snapshots = snapshot_engine_documents_excluding(
            self.catalog_document_id,
            &after,
            true,
            &validated_new_shards.documents,
        )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[4] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let mut derived_catalog_pages =
            compare_declared_effect_against_snapshots_with_catalog_skipping(
                &declared_effect,
                &before_snapshots,
                &after_snapshots,
                &validated_new_shards,
            )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[5] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        // Prepare every current-state replacement first. No visible document is
        // changed until all imports and structural checks have succeeded.
        let mut replacements = BTreeMap::new();
        let mut replacement_heads = BTreeMap::new();
        let mut new_exact_shards = BTreeSet::new();
        let mut validated_catalog_pages = None;
        for (document_id, update) in &updates {
            let exact_before_vector = &exact_before_vectors[document_id];
            let hot_current = if self.is_blocked() {
                self.terminal_documents
                    .get(document_id)
                    .or_else(|| self.visible_documents.get(document_id))
            } else {
                self.visible_documents.get(document_id)
            };
            let hot_heads = if self.is_blocked() {
                self.terminal_document_heads
                    .get(document_id)
                    .or_else(|| self.visible_document_heads.get(document_id))
            } else {
                self.visible_document_heads.get(document_id)
            };
            let fast_exact_current = self.scratch.is_none()
                && (hot_current.is_some_and(|document| {
                    document.oplog_vv() == *exact_before_vector
                        && hot_heads
                            .into_iter()
                            .flatten()
                            .copied()
                            .eq(update.dependency_heads.iter().copied())
                }) || (hot_current.is_none()
                    && !self.is_blocked()
                    && exact_before_vector.is_empty()
                    && update.dependency_heads.is_empty()
                    && update.causal_state_digest.is_none()));
            let (current, current_heads) = if fast_exact_current {
                (None, None)
            } else if self.scratch.is_some() {
                let (document, heads) = self.load_external_validation_document(*document_id)?;
                (Some(document), Some(heads))
            } else {
                let current = self.clone_validation_document(*document_id, 1)?;
                self.record_stage_snapshot_clone(&current);
                (Some(EngineDocument::InMemory(current)), None)
            };
            let exact_current = fast_exact_current
                || current
                    .as_ref()
                    .is_some_and(|current| current.document().oplog_vv() == *exact_before_vector);
            let current_page_id = if *document_id == self.catalog_document_id {
                None
            } else if exact_current {
                exact_before_page_ids[document_id]
            } else {
                shard_page_id(
                    current
                        .as_ref()
                        .expect("non-exact current document")
                        .document(),
                )?
            };
            let replacement = if exact_current {
                // The already validated exact-frontier transition is also the
                // current-state join. Reusing it avoids importing every sealed
                // update twice during causal replay while preserving the same
                // CRDT state and atomic publication boundary.
                after
                    .remove(document_id)
                    .expect("updated document has a validated after state")
            } else {
                let current = current.expect("divergent current document");
                import_complete(
                    *document_id,
                    current.document(),
                    std::slice::from_ref(&update.raw_update),
                )?;
                current
            };
            if *document_id == self.catalog_document_id {
                validated_catalog_pages = if exact_current {
                    Some(
                        derived_catalog_pages
                            .take()
                            .expect("derived catalog update has validated page state")
                            .clone(),
                    )
                } else {
                    Some(validate_catalog(
                        self.catalog_document_id,
                        replacement.document(),
                    )?)
                };
            } else if exact_current && exact_before_vector.is_empty() {
                // The snapshot comparator or the bounded direct validator
                // exhaustively checked this brand-new shard. Recheck metadata
                // here because page identity also feeds replacement handling.
                validate_shard_metadata_shape(*document_id, replacement.document())?;
                new_exact_shards.insert(*document_id);
            } else {
                validate_shard(
                    self.catalog_document_id,
                    *document_id,
                    replacement.document(),
                )?;
                validate_immutable_shard_identity(
                    *document_id,
                    current_page_id.or(exact_before_page_ids[document_id]),
                    replacement.document(),
                )?;
            }
            if let Some(mut heads) = current_heads {
                heads.retain(|head| update.dependency_heads.binary_search(head).is_err());
                heads.insert(batch_id);
                replacement_heads.insert(*document_id, heads);
            }
            replacements.insert(*document_id, replacement);
        }
        self.validate_prospective_references(
            &replacements,
            &declared_effect,
            &new_exact_shards,
            validated_catalog_pages.as_ref(),
        )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[6] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let dependencies = if self.scratch.is_none()
            && declared_effect
                .blocks()
                .iter()
                .any(|delta| delta.before.is_none() && delta.after.is_some())
        {
            self.collect_batch_ancestry(&declared_batch_heads(&frontier), self.is_blocked())?
                .into_keys()
                .collect()
        } else {
            BTreeSet::new()
        };
        let starting_roots = candidate_roots.unwrap_or_else(|| self.scratch_roots.clone());
        let identity = self.validate_and_record_semantic_roles_and_block_homes(
            &starting_roots,
            batch_id,
            self.archive[&batch_id].manifest().causal_dot(),
            &dependencies,
            &declared_effect,
        )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[7] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let quarantined = identity.blocked || !allow_publication || self.is_blocked();
        let lane = if quarantined {
            super::document_state::DocumentLane::Terminal
        } else {
            super::document_state::DocumentLane::Visible
        };
        // Divergent exact-frontier records and selected current records compose
        // on one local candidate. No engine-visible root advances until every
        // external flush, witness, and LSM publication has succeeded.
        let candidate_roots = self.prepare_exact_document_checkpoints(
            &identity.scratch_roots,
            batch_id,
            &updates,
            &after,
            lane,
        )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[8] += phase_started.elapsed().as_nanos();
            phase_started = Instant::now();
        }
        let candidate_roots = self.prepare_external_document_checkpoints(
            &candidate_roots,
            batch_id,
            &replacements,
            &replacement_heads,
            lane,
        )?;
        #[cfg(test)]
        {
            self.validation_phase_nanos[9] += phase_started.elapsed().as_nanos();
        }
        if quarantined {
            if self.scratch.is_some() {
                self.scratch_roots = candidate_roots;
                self.block_claim_root = identity.block_claim_root;
                self.fatal_handle = identity.fatal_handle;
            }
            self.commit_terminal_replacements(batch_id, &updates, replacements)?;
            return Ok(BatchApplication::Quarantined);
        }
        let (post_documents, accepted_evidence, candidate_roots) = self
            .prepare_acceptance_evidence(
                batch_id,
                &updates,
                &replacements,
                &replacement_heads,
                &candidate_roots,
            )?;
        if self.scratch.is_some() {
            self.block_claim_root = identity.block_claim_root;
            self.fatal_handle = identity.fatal_handle;
        }
        let status_evidence = accepted_evidence.clone();
        self.commit_acceptance_evidence(post_documents, accepted_evidence, candidate_roots);
        let bulk_hot_documents = self.scratch.as_ref().and_then(|_| {
            let non_catalog = replacements
                .keys()
                .copied()
                .filter(|document_id| *document_id != self.catalog_document_id)
                .collect::<Vec<_>>();
            (non_catalog.len() >= MAX_HOT_NON_CATALOG_DOCUMENTS).then(|| {
                non_catalog
                    .into_iter()
                    .rev()
                    .take(MAX_HOT_NON_CATALOG_DOCUMENTS)
                    .collect::<BTreeSet<_>>()
            })
        });
        if let Some(keep_hot) = bulk_hot_documents {
            self.visible_documents.retain(|document_id, _| {
                *document_id == self.catalog_document_id || keep_hot.contains(document_id)
            });
            self.visible_document_heads.retain(|document_id, _| {
                *document_id == self.catalog_document_id || keep_hot.contains(document_id)
            });
            self.terminal_documents
                .retain(|document_id, _| *document_id == self.catalog_document_id);
            self.terminal_document_heads
                .retain(|document_id, _| *document_id == self.catalog_document_id);
            self.spare_documents
                .borrow_mut()
                .retain(|document_id, _| keep_hot.contains(document_id));
            self.visible_document_lru.clear();
            for (document_id, document) in replacements {
                if document_id != self.catalog_document_id && !keep_hot.contains(&document_id) {
                    continue;
                }
                self.visible_documents
                    .insert(document_id, document.into_document());
                let heads = self.visible_document_heads.entry(document_id).or_default();
                let dependencies = &updates[&document_id].dependency_heads;
                heads.retain(|head| dependencies.binary_search(head).is_err());
                heads.insert(batch_id);
                if document_id != self.catalog_document_id {
                    self.visible_document_lru.push_back(document_id);
                }
            }
        } else {
            let mut touched_documents = Vec::with_capacity(replacements.len());
            for (document_id, document) in replacements {
                self.visible_documents
                    .insert(document_id, document.into_document());
                touched_documents.push(document_id);
                let heads = self.visible_document_heads.entry(document_id).or_default();
                let dependencies = &updates[&document_id].dependency_heads;
                heads.retain(|head| dependencies.binary_search(head).is_err());
                heads.insert(batch_id);
            }
            let keep_single_shard =
                touched_documents.len() == 1 && touched_documents[0] != self.catalog_document_id;
            for document_id in touched_documents {
                if self.scratch.is_some() || keep_single_shard {
                    self.retain_hot_document(document_id);
                } else if document_id != self.catalog_document_id {
                    self.visible_documents.remove(&document_id);
                    self.visible_document_lru
                        .retain(|current| *current != document_id);
                }
            }
        }
        Ok(BatchApplication::Accepted {
            no_op: declared_effect.is_empty(),
            evidence: status_evidence,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_and_apply_pending_author(
        &mut self,
        batch_id: BatchId,
        causal_dot: BatchCausalDot,
        allow_publication: bool,
        frontier: &FrontierV2,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
        semantic_payload: &[u8],
        pending_documents: BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<Option<BatchApplication>, EngineError> {
        if !self.dependency_witnesses_are_current(frontier, updates, self.is_blocked())?
            || pending_documents
                .keys()
                .copied()
                .ne(updates.keys().copied())
        {
            return Ok(None);
        }

        let mut before_documents = BTreeMap::new();
        for document_id in updates.keys() {
            let document = if let Some(document) = self.visible_documents.get(document_id) {
                document.clone()
            } else if self
                .document_dependency_heads(*document_id, self.is_blocked())?
                .is_empty()
            {
                LoroDoc::new()
            } else {
                return Ok(None);
            };
            validate_update_base(*document_id, &document, &updates[document_id].raw_update)?;
            before_documents.insert(*document_id, document);
        }
        let before_snapshots =
            snapshot_documents_with_validation(self.catalog_document_id, &before_documents, false)?;
        let after_snapshots = snapshot_documents(self.catalog_document_id, &pending_documents)?;
        let declared_effect = SemanticEffect::decode(semantic_payload)?;
        let derived_catalog_pages = compare_declared_effect_against_snapshots_with_catalog(
            &declared_effect,
            &before_snapshots,
            &after_snapshots,
        )?;

        let mut new_exact_shards = BTreeSet::new();
        let mut validated_catalog_pages = None;
        for (document_id, replacement) in &pending_documents {
            let exact_before = &before_documents[document_id];
            if *document_id == self.catalog_document_id {
                validated_catalog_pages = derived_catalog_pages.cloned();
            } else if exact_before.oplog_vv().is_empty() {
                validate_shard_metadata_shape(*document_id, replacement)?;
                new_exact_shards.insert(*document_id);
            } else {
                let current_page_id = shard_page_id(exact_before)?;
                validate_shard(self.catalog_document_id, *document_id, replacement)?;
                validate_immutable_shard_identity(*document_id, current_page_id, replacement)?;
            }
        }
        let pending_engine_documents = pending_documents
            .iter()
            .map(|(document_id, document)| {
                (*document_id, EngineDocument::InMemory(document.clone()))
            })
            .collect();
        self.validate_prospective_references(
            &pending_engine_documents,
            &declared_effect,
            &new_exact_shards,
            validated_catalog_pages.as_ref(),
        )?;
        let dependencies = if self.scratch.is_none()
            && declared_effect
                .blocks()
                .iter()
                .any(|delta| delta.before.is_none() && delta.after.is_some())
        {
            self.collect_batch_ancestry(&declared_batch_heads(frontier), self.is_blocked())?
                .into_keys()
                .collect()
        } else {
            BTreeSet::new()
        };
        let identity = self.validate_and_record_semantic_roles_and_block_homes(
            &self.scratch_roots.clone(),
            batch_id,
            causal_dot,
            &dependencies,
            &declared_effect,
        )?;
        if identity.blocked || !allow_publication || self.is_blocked() {
            self.commit_terminal_replacements(
                batch_id,
                updates,
                pending_documents
                    .into_iter()
                    .map(|(document_id, document)| {
                        (document_id, EngineDocument::InMemory(document))
                    })
                    .collect(),
            )?;
            return Ok(Some(BatchApplication::Quarantined));
        }
        let (post_documents, accepted_evidence, candidate_roots) = self
            .prepare_acceptance_evidence(
                batch_id,
                updates,
                &pending_engine_documents,
                &BTreeMap::new(),
                &identity.scratch_roots,
            )?;
        if self.scratch.is_some() {
            self.block_claim_root = identity.block_claim_root;
            self.fatal_handle = identity.fatal_handle;
        }
        let status_evidence = accepted_evidence.clone();
        self.commit_acceptance_evidence(post_documents, accepted_evidence, candidate_roots);

        let mut work = self.history_work.get();
        work.stage_structural_buffer_reuses = work
            .stage_structural_buffer_reuses
            .saturating_add(pending_documents.len());
        self.history_work.set(work);
        let mut touched_documents = Vec::with_capacity(pending_documents.len());
        for (document_id, document) in pending_documents {
            self.visible_documents.insert(document_id, document);
            touched_documents.push(document_id);
            let heads = self.visible_document_heads.entry(document_id).or_default();
            let dependencies = &updates[&document_id].dependency_heads;
            heads.retain(|head| dependencies.binary_search(head).is_err());
            heads.insert(batch_id);
        }
        let keep_single_shard =
            touched_documents.len() == 1 && touched_documents[0] != self.catalog_document_id;
        for document_id in &touched_documents {
            if self.scratch.is_some() || keep_single_shard {
                self.retain_hot_document(*document_id);
            } else if *document_id != self.catalog_document_id {
                self.visible_documents.remove(document_id);
                self.spare_documents.borrow_mut().remove(document_id);
                self.visible_document_lru
                    .retain(|current| current != document_id);
            }
        }

        // Visible publication is complete before recycling the former visible
        // buffers. A failed optimization import only discards that spare; it
        // cannot roll back or partially expose the accepted semantic state.
        for (document_id, before) in before_documents {
            if (document_id == self.catalog_document_id || keep_single_shard)
                && import_complete(
                    document_id,
                    &before,
                    std::slice::from_ref(&updates[&document_id].raw_update),
                )
                .is_ok()
            {
                self.spare_documents
                    .borrow_mut()
                    .insert(document_id, before);
            }
        }
        Ok(Some(BatchApplication::Accepted {
            no_op: declared_effect.is_empty(),
            evidence: status_evidence,
        }))
    }

    fn reconstruct_frontier(
        &self,
        frontier: &FrontierV2,
    ) -> Result<BTreeMap<DocumentId, LoroDoc>, EngineError> {
        let direct_heads = declared_batch_heads(frontier);
        let ancestry = self.collect_batch_ancestry(&direct_heads, self.is_blocked())?;
        validate_maximal_document_heads(frontier, &ancestry)?;
        let mut documents = BTreeMap::new();
        for dependencies in frontier.documents() {
            let document_id = dependencies.document_id();
            let mut updates = Vec::new();
            for (dependency_id, manifest) in &ancestry {
                if manifest.required_objects().iter().any(|descriptor| {
                    descriptor.kind() == ObjectKind::CrdtUpdate
                        && descriptor.document_id() == document_id
                }) {
                    let update =
                        self.load_archive_document_object(*dependency_id, manifest, document_id)?;
                    updates.push(
                        decode_crdt_update_payload(*dependency_id, document_id, update.payload())?
                            .raw_update,
                    );
                }
            }
            let document = LoroDoc::new();
            import_complete(document_id, &document, &updates)?;
            let actual = canonical_peer_counters(&document.oplog_vv())?;
            if actual != dependencies.peer_counters() {
                return Err(EngineError::FrontierVectorMismatch(document_id));
            }
            documents.insert(document_id, document);
        }
        Ok(documents)
    }

    fn clone_visible_document(
        &self,
        document_id: DocumentId,
        peer: u64,
    ) -> Result<LoroDoc, EngineError> {
        if let Some(store) = &self.scratch {
            let document = match super::document_state::load_external_current(
                store,
                &self.scratch_roots,
                super::document_state::DocumentLane::Visible,
                document_id,
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?
            {
                Some((record, document, state_work)) => {
                    self.record_document_state_work(state_work);
                    self.validate_external_record_anchor(document_id, &record)?;
                    document.into_document()
                }
                None => LoroDoc::new(),
            };
            document.set_peer_id(peer).map_err(loro_error)?;
            return Ok(document);
        }
        match self.visible_documents.get(&document_id) {
            Some(document) => clone_doc(document, peer),
            None => {
                let document = if self
                    .visible_document_heads
                    .get(&document_id)
                    .is_none_or(BTreeSet::is_empty)
                {
                    LoroDoc::new()
                } else {
                    self.reconstruct_document_from_heads(document_id, false)?
                };
                document.set_peer_id(peer).map_err(loro_error)?;
                Ok(document)
            }
        }
    }

    fn clone_validation_document(
        &self,
        document_id: DocumentId,
        peer: u64,
    ) -> Result<LoroDoc, EngineError> {
        if let Some(store) = &self.scratch {
            let lane = if self.is_blocked() {
                super::document_state::DocumentLane::Terminal
            } else {
                super::document_state::DocumentLane::Visible
            };
            let mut loaded = super::document_state::load_external_current(
                store,
                &self.scratch_roots,
                lane,
                document_id,
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?;
            if loaded.is_none() && lane == super::document_state::DocumentLane::Terminal {
                loaded = super::document_state::load_external_current(
                    store,
                    &self.scratch_roots,
                    super::document_state::DocumentLane::Visible,
                    document_id,
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            }
            if let Some((record, _, state_work)) = loaded.as_ref() {
                self.record_document_state_work(*state_work);
                self.validate_external_record_anchor(document_id, record)?;
            }
            let document = loaded
                .map(|(_, document, _)| document.into_document())
                .unwrap_or_else(LoroDoc::new);
            document.set_peer_id(peer).map_err(loro_error)?;
            return Ok(document);
        }
        match self.terminal_documents.get(&document_id) {
            Some(document) => clone_doc(document, peer),
            None => {
                if !self.terminal_document_heads.contains_key(&document_id) {
                    return self.clone_visible_document(document_id, peer);
                }
                let document = self.reconstruct_document_from_heads(document_id, true)?;
                document.set_peer_id(peer).map_err(loro_error)?;
                Ok(document)
            }
        }
    }

    fn load_external_validation_document(
        &self,
        document_id: DocumentId,
    ) -> Result<(EngineDocument, BTreeSet<BatchId>), EngineError> {
        let store = self
            .scratch
            .as_ref()
            .expect("external validation requires scratch");
        let lane = if self.is_blocked() {
            super::document_state::DocumentLane::Terminal
        } else {
            super::document_state::DocumentLane::Visible
        };
        let mut loaded = super::document_state::load_external_current(
            store,
            &self.scratch_roots,
            lane,
            document_id,
        )
        .map_err(|error| EngineError::Archive(error.to_string()))?;
        if loaded.is_none() && lane == super::document_state::DocumentLane::Terminal {
            loaded = super::document_state::load_external_current(
                store,
                &self.scratch_roots,
                super::document_state::DocumentLane::Visible,
                document_id,
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        }
        if let Some((record, document, state_work)) = loaded {
            self.record_document_state_work(state_work);
            self.validate_external_record_anchor(document_id, &record)?;
            let heads = record.exact_direct_heads().iter().copied().collect();
            return Ok((EngineDocument::External(document), heads));
        }
        Ok((
            EngineDocument::External(
                super::document_state::ExternalDocument::empty(Arc::clone(store))
                    .map_err(|error| EngineError::Archive(error.to_string()))?,
            ),
            BTreeSet::new(),
        ))
    }

    fn validate_external_record_anchor(
        &self,
        document_id: DocumentId,
        record: &super::document_state::ExternalDocumentStateRecord,
    ) -> Result<(), EngineError> {
        let anchor = (
            document_id,
            record.latest_source_batch(),
            record.latest_manifest_fingerprint(),
            record.latest_update_digest(),
        );
        if self.external_anchor_point_cache.borrow().contains(&anchor) {
            return Ok(());
        }
        let manifest = self.load_observed_manifest(record.latest_source_batch())?;
        let object = self.load_archive_document_object(
            record.latest_source_batch(),
            &manifest,
            document_id,
        )?;
        let descriptor = object.descriptor().map_err(EngineError::from)?;
        if descriptor.content_digest() != record.latest_update_digest()
            || batch_fingerprint_from_manifest(&manifest) != record.latest_manifest_fingerprint()
        {
            return Err(EngineError::Archive(
                "external document checkpoint archive anchor mismatch".into(),
            ));
        }
        self.external_anchor_point_cache.borrow_mut().insert(anchor);
        Ok(())
    }

    fn commit_terminal_replacements(
        &mut self,
        batch_id: BatchId,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
        replacements: BTreeMap<DocumentId, EngineDocument>,
    ) -> Result<(), EngineError> {
        for (document_id, document) in replacements {
            self.terminal_documents
                .insert(document_id, document.into_document());
            self.visible_documents.remove(&document_id);
            let heads = self
                .terminal_document_heads
                .entry(document_id)
                .or_insert_with(|| {
                    self.visible_document_heads
                        .get(&document_id)
                        .cloned()
                        .unwrap_or_default()
                });
            let dependencies = &updates[&document_id].dependency_heads;
            heads.retain(|head| dependencies.binary_search(head).is_err());
            heads.insert(batch_id);
            self.retain_hot_document(document_id);
        }
        self.enforce_shared_document_lru();
        Ok(())
    }

    fn prepare_external_document_checkpoints(
        &self,
        roots: &ScratchRoots,
        batch_id: BatchId,
        replacements: &BTreeMap<DocumentId, EngineDocument>,
        replacement_heads: &BTreeMap<DocumentId, BTreeSet<BatchId>>,
        lane: super::document_state::DocumentLane,
    ) -> Result<ScratchRoots, EngineError> {
        let Some(store) = self.scratch.as_ref().cloned() else {
            return Ok(roots.clone());
        };
        let fingerprint = self
            .archive_fingerprints
            .get(&batch_id)
            .copied()
            .ok_or_else(|| EngineError::Archive("missing staged fingerprint".into()))?;
        let update_digests = self.archive[&batch_id]
            .objects()
            .iter()
            .filter(|object| object.kind() == ObjectKind::CrdtUpdate)
            .map(|object| {
                Ok((
                    object.document_id(),
                    object
                        .descriptor()
                        .map_err(EngineError::from)?
                        .content_digest(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, EngineError>>()?;
        let mut inputs = Vec::with_capacity(replacements.len());
        for (document_id, document) in replacements {
            let external = document.external().ok_or_else(|| {
                EngineError::Archive(
                    "store-backed publication lost authenticated document control".into(),
                )
            })?;
            #[cfg(test)]
            if self.external_publication_failure_index == Some(inputs.len()) {
                external.poison_store_for_test("injected late external publication failure");
            }
            inputs.push(super::document_state::ExternalCheckpointInput {
                document_id: *document_id,
                document: external,
                exact_direct_heads: replacement_heads
                    .get(document_id)
                    .ok_or_else(|| {
                        EngineError::Archive(
                            "store-backed publication lost authenticated current heads".into(),
                        )
                    })?
                    .iter()
                    .copied()
                    .collect(),
                latest_update_digest: update_digests[document_id],
            });
        }
        let (candidate, state_work) = super::document_state::commit_external_current_batch(
            &store,
            roots,
            lane,
            batch_id,
            fingerprint,
            inputs,
        )
        .map_err(|error| EngineError::Archive(error.to_string()))?;
        self.record_document_state_work(state_work);
        Ok(candidate)
    }

    fn prepare_exact_document_checkpoints(
        &self,
        roots: &ScratchRoots,
        batch_id: BatchId,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
        exact_after: &BTreeMap<DocumentId, EngineDocument>,
        lane: super::document_state::DocumentLane,
    ) -> Result<ScratchRoots, EngineError> {
        let Some(store) = &self.scratch else {
            return Ok(roots.clone());
        };
        let fingerprint = self
            .archive_fingerprints
            .get(&batch_id)
            .copied()
            .ok_or_else(|| EngineError::Archive("missing staged fingerprint".into()))?;
        let update_digests = self.archive[&batch_id]
            .objects()
            .iter()
            .filter(|object| object.kind() == ObjectKind::CrdtUpdate)
            .map(|object| {
                Ok((
                    object.document_id(),
                    object
                        .descriptor()
                        .map_err(EngineError::from)?
                        .content_digest(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, EngineError>>()?;
        let mut inputs = Vec::new();
        for (document_id, document) in exact_after {
            if !updates.contains_key(document_id) {
                continue;
            }
            inputs.push(super::document_state::ExternalCheckpointInput {
                document_id: *document_id,
                document: document.external().ok_or_else(|| {
                    EngineError::Archive(
                        "store-backed exact publication lost authenticated document control".into(),
                    )
                })?,
                exact_direct_heads: vec![batch_id],
                latest_update_digest: update_digests[document_id],
            });
        }
        let (candidate, state_work) = super::document_state::commit_external_exact_batch(
            store,
            roots,
            lane,
            batch_id,
            fingerprint,
            inputs,
        )
        .map_err(|error| EngineError::Archive(error.to_string()))?;
        self.record_document_state_work(state_work);
        Ok(candidate)
    }

    fn enforce_shared_document_lru(&mut self) {
        if self.scratch.is_none() {
            return;
        }
        let mut non_catalog = self
            .visible_documents
            .keys()
            .filter(|document_id| **document_id != self.catalog_document_id)
            .copied()
            .map(|document_id| (false, document_id))
            .chain(
                self.terminal_documents
                    .keys()
                    .filter(|document_id| **document_id != self.catalog_document_id)
                    .copied()
                    .map(|document_id| (true, document_id)),
            )
            .collect::<Vec<_>>();
        non_catalog.sort_unstable_by_key(|(_, document_id)| *document_id);
        while non_catalog.len() > MAX_HOT_NON_CATALOG_DOCUMENTS {
            let (terminal, document_id) = non_catalog.remove(0);
            if terminal {
                self.terminal_documents.remove(&document_id);
                self.terminal_document_heads.remove(&document_id);
            } else {
                self.visible_documents.remove(&document_id);
                self.visible_document_heads.remove(&document_id);
            }
            self.spare_documents.borrow_mut().remove(&document_id);
            self.visible_document_lru
                .retain(|current| *current != document_id);
        }
    }

    fn document_dependency_heads(
        &self,
        document_id: DocumentId,
        terminal: bool,
    ) -> Result<BTreeSet<BatchId>, EngineError> {
        if let Some(store) = &self.scratch {
            let lane = if terminal {
                super::document_state::DocumentLane::Terminal
            } else {
                super::document_state::DocumentLane::Visible
            };
            if let Some((record, _, state_work)) = super::document_state::load_external_current(
                store,
                &self.scratch_roots,
                lane,
                document_id,
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?
            {
                self.record_document_state_work(state_work);
                self.validate_external_record_anchor(document_id, &record)?;
                return Ok(record.exact_direct_heads().iter().copied().collect());
            }
            if terminal {
                if let Some((record, _, state_work)) = super::document_state::load_external_current(
                    store,
                    &self.scratch_roots,
                    super::document_state::DocumentLane::Visible,
                    document_id,
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?
                {
                    self.record_document_state_work(state_work);
                    self.validate_external_record_anchor(document_id, &record)?;
                    return Ok(record.exact_direct_heads().iter().copied().collect());
                }
            }
            return Ok(BTreeSet::new());
        }
        let heads = if terminal {
            self.terminal_document_heads
                .get(&document_id)
                .or_else(|| self.visible_document_heads.get(&document_id))
        } else {
            self.visible_document_heads.get(&document_id)
        };
        Ok(heads.cloned().unwrap_or_default())
    }

    fn retain_hot_document(&mut self, document_id: DocumentId) {
        if document_id == self.catalog_document_id {
            return;
        }
        self.visible_document_lru
            .retain(|current| *current != document_id);
        self.visible_document_lru.push_back(document_id);
        while self.visible_document_lru.len() > MAX_HOT_NON_CATALOG_DOCUMENTS {
            if let Some(evicted) = self.visible_document_lru.pop_front() {
                self.visible_documents.remove(&evicted);
                self.terminal_documents.remove(&evicted);
                self.visible_document_heads.remove(&evicted);
                self.terminal_document_heads.remove(&evicted);
                self.spare_documents.borrow_mut().remove(&evicted);
            }
        }
    }

    fn load_observed_manifest(&self, batch_id: BatchId) -> Result<OperationBatch, EngineError> {
        if let Some(batch) = self.archive.get(&batch_id) {
            return Ok(batch.manifest().clone());
        }
        let store = self
            .archive_store
            .as_ref()
            .ok_or(EngineError::MissingDependency(batch_id))?;
        let expected_fingerprint =
            if let Some(fingerprint) = self.archive_fingerprints.get(&batch_id) {
                *fingerprint
            } else {
                self.cold_history_record(batch_id)?
                    .map(|record| record.manifest_fingerprint)
                    .ok_or(EngineError::MissingDependency(batch_id))?
            };
        let manifest = store
            .reload_accepted_manifest(batch_id, expected_fingerprint)
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        if manifest.lineage_digest() != self.lineage_digest {
            return Err(EngineError::LineageMismatch {
                expected: self.lineage_digest,
                found: manifest.lineage_digest(),
            });
        }
        Ok(manifest)
    }

    fn load_archive_document_object(
        &self,
        batch_id: BatchId,
        manifest: &OperationBatch,
        document_id: DocumentId,
    ) -> Result<OperationObject, EngineError> {
        if let Some(batch) = self.archive.get(&batch_id) {
            return batch
                .objects()
                .iter()
                .find(|object| {
                    object.kind() == ObjectKind::CrdtUpdate && object.document_id() == document_id
                })
                .cloned()
                .ok_or(EngineError::MissingDocumentUpdate {
                    document_id,
                    dependency: batch_id,
                });
        }
        let store = self
            .archive_store
            .as_ref()
            .ok_or(EngineError::MissingDependency(batch_id))?;
        store
            .reload_accepted_document_object(manifest, document_id)
            .map_err(|error| EngineError::Archive(error.to_string()))
    }

    fn reconstruct_document_from_heads(
        &self,
        document_id: DocumentId,
        terminal: bool,
    ) -> Result<LoroDoc, EngineError> {
        let heads = self.document_dependency_heads(document_id, terminal)?;
        let ancestry = self.collect_batch_ancestry(&heads, terminal)?;
        let mut updates = Vec::new();
        for (batch_id, manifest) in ancestry {
            if manifest.required_objects().iter().any(|descriptor| {
                descriptor.kind() == ObjectKind::CrdtUpdate
                    && descriptor.document_id() == document_id
            }) {
                let update = self.load_archive_document_object(batch_id, &manifest, document_id)?;
                updates.push(
                    decode_crdt_update_payload(batch_id, document_id, update.payload())?.raw_update,
                );
            }
        }
        let document = LoroDoc::new();
        import_complete(document_id, &document, &updates)?;
        Ok(document)
    }

    fn collect_batch_ancestry(
        &self,
        direct_heads: &BTreeSet<BatchId>,
        allow_quarantined: bool,
    ) -> Result<BTreeMap<BatchId, OperationBatch>, EngineError> {
        let mut work = self.history_work.get();
        work.ancestry_traversals = work.ancestry_traversals.saturating_add(1);
        self.history_work.set(work);
        let mut ancestry = BTreeMap::new();
        let mut stack: Vec<_> = direct_heads.iter().copied().collect();
        while let Some(batch_id) = stack.pop() {
            if ancestry.contains_key(&batch_id) {
                continue;
            }
            match self.archive_status(batch_id)? {
                Some(ArchiveStatus::Accepted { .. }) => {}
                Some(ArchiveStatus::Quarantined) if allow_quarantined => {}
                Some(ArchiveStatus::Rejected(_)) => {
                    return Err(EngineError::RejectedDependency(batch_id));
                }
                Some(ArchiveStatus::Staged) | Some(ArchiveStatus::Quarantined) | None => {
                    return Err(EngineError::MissingDependency(batch_id));
                }
            }
            let manifest = self.load_observed_manifest(batch_id)?;
            let manifest_parents = declared_batch_heads(manifest.dependency_frontier());
            if manifest_parents.contains(&batch_id) {
                return Err(EngineError::SelfDependency(batch_id));
            }
            stack.extend(manifest_parents.iter().copied());
            ancestry.insert(batch_id, manifest);
        }

        Ok(ancestry)
    }

    fn archive_read_stats(&self) -> super::object_store::AcceptedReadStats {
        self.archive_store
            .as_ref()
            .map(|store| store.accepted_read_stats())
            .unwrap_or_default()
    }

    fn referenced_home<'a>(
        &self,
        replacements: &'a BTreeMap<DocumentId, EngineDocument>,
        loaded: &'a mut BTreeMap<DocumentId, LoroDoc>,
        document_id: DocumentId,
    ) -> Result<&'a LoroDoc, EngineError> {
        if let Some(home) = replacements.get(&document_id) {
            return Ok(home.document());
        }
        Ok(match loaded.entry(document_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let home = self.clone_validation_document(document_id, 1)?;
                validate_shard(self.catalog_document_id, document_id, &home)?;
                entry.insert(home)
            }
        })
    }

    fn validate_prospective_references(
        &self,
        replacements: &BTreeMap<DocumentId, EngineDocument>,
        effect: &SemanticEffect,
        new_exact_shards: &BTreeSet<DocumentId>,
        validated_catalog_pages: Option<&BTreeMap<PageId, PageState>>,
    ) -> Result<(), EngineError> {
        let catalog;
        let loaded_pages;
        let pages = if let Some(pages) = validated_catalog_pages {
            pages
        } else if let Some(replacement) = replacements.get(&self.catalog_document_id) {
            loaded_pages = validate_catalog(self.catalog_document_id, replacement.document())?;
            &loaded_pages
        } else {
            catalog = self.clone_validation_document(self.catalog_document_id, 1)?;
            loaded_pages = validate_catalog(self.catalog_document_id, &catalog)?;
            &loaded_pages
        };

        // A changed catalog entry must name an extant immutable home whose
        // retained shard identity agrees with that entry. This is scoped to
        // changed pages; ordinary catalog edits do not enumerate every shard.
        for delta in effect.pages() {
            let Some(state) = &delta.after else {
                return Err(EngineError::MalformedDocument {
                    document_id: self.catalog_document_id,
                    reason: format!("page {} was removed instead of tombstoned", delta.page_id),
                });
            };
            let home_document_id = state.home_document_id();
            let loaded_home;
            let home = if let Some(home) = replacements.get(&home_document_id) {
                home.document()
            } else {
                loaded_home = self.clone_validation_document(home_document_id, 1)?;
                validate_shard(self.catalog_document_id, home_document_id, &loaded_home)?;
                &loaded_home
            };
            if shard_page_id(home)? != Some(delta.page_id) {
                return Err(EngineError::MalformedDocument {
                    document_id: home_document_id,
                    reason: format!(
                        "catalog page {} does not match its immutable home shard identity",
                        delta.page_id
                    ),
                });
            }
        }

        // These are validation-only membership indexes: canonical ordering is
        // neither observed nor serialized. Hash lookup keeps a large batch
        // linear instead of paying a log factor for every block membership.
        let new_blocks: AHashSet<(DocumentId, BlockId)> = effect
            .blocks()
            .iter()
            .filter_map(|delta| {
                delta
                    .after
                    .as_ref()
                    .map(|_| (delta.home_document_id, delta.block_id))
            })
            .collect();
        let mut new_memberships = AHashMap::<PageId, Vec<(BlockId, MembershipClaim)>>::new();
        for delta in effect.memberships() {
            for claim in [&delta.before, &delta.after].into_iter().flatten() {
                if claim.home_document_id == self.catalog_document_id {
                    return Err(EngineError::MalformedDocument {
                        document_id: self.catalog_document_id,
                        reason: format!(
                            "catalog cannot be the membership home of block {}",
                            delta.block_id
                        ),
                    });
                }
            }
            if let Some(claim) = &delta.after {
                new_memberships
                    .entry(delta.page_id)
                    .or_default()
                    .push((delta.block_id, claim.clone()));
            }
        }
        let mut referenced_homes = BTreeMap::<DocumentId, LoroDoc>::new();
        for (document_id, shard) in replacements {
            let shard = shard.document();
            if *document_id == self.catalog_document_id {
                continue;
            }
            let page_id = shard_page_id(shard)?.ok_or_else(|| EngineError::MalformedDocument {
                document_id: *document_id,
                reason: "shard has no page identity".into(),
            })?;
            let Some(page_state) = pages.get(&page_id) else {
                return Err(EngineError::MalformedDocument {
                    document_id: *document_id,
                    reason: format!("shard identity references missing catalog page {page_id}"),
                });
            };
            if page_state.home_document_id() != *document_id {
                return Err(EngineError::MalformedDocument {
                    document_id: *document_id,
                    reason: format!("shard identity {page_id} is not its catalog home"),
                });
            }

            if new_exact_shards.contains(document_id) {
                for (block_id, claim) in new_memberships.get(&page_id).into_iter().flatten() {
                    if !new_blocks.contains(&(claim.home_document_id, *block_id)) {
                        let home = self.referenced_home(
                            replacements,
                            &mut referenced_homes,
                            claim.home_document_id,
                        )?;
                        if !has_block_state(home, *block_id)? {
                            return Err(EngineError::MalformedDocument {
                                document_id: *document_id,
                                reason: format!(
                                    "membership {block_id} references missing home content {}",
                                    claim.home_document_id
                                ),
                            });
                        }
                    }
                }
                continue;
            }

            for (block_id, claim) in read_memberships(*document_id, shard)? {
                let home = self.referenced_home(
                    replacements,
                    &mut referenced_homes,
                    claim.home_document_id,
                )?;
                if !has_block_state(home, block_id)? {
                    return Err(EngineError::MalformedDocument {
                        document_id: *document_id,
                        reason: format!(
                            "membership {block_id} references missing home content {}",
                            claim.home_document_id
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn ensure_working_document<'a>(
        &'a self,
        working: &'a mut BTreeMap<DocumentId, EngineDocument>,
        before_vectors: &mut BTreeMap<DocumentId, VersionVector>,
        before_snapshots: &mut BTreeMap<DocumentId, SemanticDocumentSnapshot>,
        document_id: DocumentId,
        peer_id: CrdtPeerId,
    ) -> Result<&'a LoroDoc, EngineError> {
        if let Entry::Vacant(entry) = working.entry(document_id) {
            let document = if self.scratch.is_some() {
                let (document, _) = self.load_external_validation_document(document_id)?;
                document
                    .document()
                    .set_peer_id(peer_id.as_u64())
                    .map_err(loro_error)?;
                document
            } else {
                let spare = self.spare_documents.borrow_mut().remove(&document_id);
                let visible_vector = self
                    .visible_documents
                    .get(&document_id)
                    .map(LoroDoc::oplog_vv);
                let document = match spare {
                    Some(spare)
                        if visible_vector
                            .as_ref()
                            .is_some_and(|vector| *vector == spare.oplog_vv())
                            || (visible_vector.is_none()
                                && self
                                    .visible_document_heads
                                    .get(&document_id)
                                    .is_none_or(BTreeSet::is_empty)
                                && spare.oplog_vv().is_empty()) =>
                    {
                        spare
                    }
                    _ => {
                        let document =
                            self.clone_visible_document(document_id, peer_id.as_u64())?;
                        self.record_author_snapshot_clone(&document);
                        document
                    }
                };
                document.set_peer_id(peer_id.as_u64()).map_err(loro_error)?;
                EngineDocument::InMemory(document)
            };
            before_vectors.insert(document_id, document.document().oplog_vv());
            before_snapshots.insert(
                document_id,
                snapshot_document(
                    self.catalog_document_id,
                    document_id,
                    document.document(),
                    false,
                )?,
            );
            entry.insert(document);
        }
        Ok(working
            .get(&document_id)
            .expect("inserted working document")
            .document())
    }

    fn validate_and_record_semantic_roles_and_block_homes(
        &mut self,
        scratch_roots: &ScratchRoots,
        batch_id: BatchId,
        causal_dot: BatchCausalDot,
        dependencies: &BTreeSet<BatchId>,
        effect: &SemanticEffect,
    ) -> Result<IdentityPublicationCandidate, EngineError> {
        let validation_started = Instant::now();
        for delta in effect.pages() {
            for state in [&delta.before, &delta.after].into_iter().flatten() {
                if state.home_document_id() == self.catalog_document_id {
                    return Err(EngineError::MalformedDocument {
                        document_id: self.catalog_document_id,
                        reason: format!(
                            "catalog cannot be the immutable home of page {}",
                            delta.page_id
                        ),
                    });
                }
            }
        }
        // Only None -> Some transitions are immutable identity claims. Edits
        // and owner changes retain the creation claim's home and provenance.
        // Reject a batch that claims one ID more than once before touching any
        // retained evidence.
        let mut candidate_keys = AHashSet::with_capacity(effect.blocks().len());
        let mut candidates =
            Vec::<(u128, BlockId, ImmutableHomeClaim)>::with_capacity(effect.blocks().len());
        for delta in effect.blocks() {
            if delta.home_document_id == self.catalog_document_id {
                return Err(EngineError::MalformedDocument {
                    document_id: self.catalog_document_id,
                    reason: format!(
                        "catalog cannot contain authoritative block {}",
                        delta.block_id
                    ),
                });
            }
            if delta.before.is_some() || delta.after.is_none() {
                continue;
            }
            let block_key = delta.block_id.as_uuid().as_u128();
            let claim = if self.scratch.is_some() {
                ImmutableHomeClaim::with_causal_dot(batch_id, delta.home_document_id, causal_dot)
            } else {
                ImmutableHomeClaim::new(batch_id, delta.home_document_id)
            };
            if !candidate_keys.insert(block_key) {
                return Err(EngineError::BlockAlreadyExists(delta.block_id));
            }
            candidates.push((block_key, delta.block_id, claim));
        }

        // Any causal reuse, including the same home, is malformed rather than
        // ambiguous. Receiver delivery order is never consulted.
        let candidate_block_ids: Vec<_> = candidates
            .iter()
            .map(|(_, block_id, _)| *block_id)
            .collect();
        let lookup_started = Instant::now();
        let mut existing_by_key = match self.block_home_claims_many(&candidate_block_ids) {
            Ok(existing) => existing,
            Err(error) => {
                self.history_failure = Some(error.clone());
                return Err(error);
            }
        };
        let lookup_nanos =
            usize::try_from(lookup_started.elapsed().as_nanos()).unwrap_or(usize::MAX);
        let candidate_clock = if let Some(store) = &self.scratch {
            Some(
                super::causal_index::batch_record(store, scratch_roots, batch_id)
                    .map_err(|error| EngineError::Archive(error.to_string()))?
                    .ok_or_else(|| {
                        EngineError::Archive("missing tentative causal batch record".into())
                    })?,
            )
        } else {
            None
        };
        for (block_key, block_id, _) in &candidates {
            let causally_reused = existing_by_key.get(block_key).is_some_and(|existing| {
                existing.iter().any(|existing| {
                    if let Some(clock) = &candidate_clock {
                        existing.causal_dot().is_none_or(|dot| clock.contains(dot))
                    } else {
                        dependencies.contains(&existing.batch_id)
                    }
                })
            });
            if causally_reused {
                return Err(EngineError::BlockAlreadyExists(*block_id));
            }
        }

        // Commit every candidate only after the complete batch has passed
        // causal classification. This includes novel candidates sharing a
        // batch with the claim that causes the terminal latch and novel IDs
        // first observed after it.
        drop(candidate_keys);
        let store_backed = self.block_claim_index.is_some();
        let encode_started = Instant::now();
        let mut changed = Vec::with_capacity(candidates.len());
        let mut changed_claims =
            Vec::with_capacity(if store_backed { 0 } else { candidates.len() });
        for (block_key, block_id, claim) in candidates {
            let mut claims = existing_by_key.remove(&block_key).unwrap_or_default();
            if store_backed && claims.is_empty() {
                changed.push((
                    block_id.as_uuid().into_bytes(),
                    encode_inline_block_claim_index_value(block_id, claim)?,
                ));
                continue;
            }
            claims.insert(claim);
            changed.push((
                block_id.as_uuid().into_bytes(),
                BlockClaimIndexValue::from_vec(encode_block_claim_record(block_id, &claims)?),
            ));
            changed_claims.push((block_id, claims));
        }
        changed.sort_unstable_by_key(|(key, _)| *key);
        let encode_nanos =
            usize::try_from(encode_started.elapsed().as_nanos()).unwrap_or(usize::MAX);
        let insert_started = Instant::now();
        let mut candidate_block_claim_root = self.block_claim_root;
        if let Some(index) = &self.block_claim_index {
            candidate_block_claim_root = match index.insert_many(self.block_claim_root, &changed) {
                Ok(root) => root,
                Err(error) => {
                    let error = EngineError::Archive(error.to_string());
                    self.history_failure = Some(error.clone());
                    return Err(error);
                }
            };
        } else {
            let novel = changed_claims
                .iter()
                .filter(|(block_id, _)| {
                    !self
                        .ephemeral_block_claims
                        .contains_key(&block_id.as_uuid().as_u128())
                })
                .count();
            if self.ephemeral_block_claims.len().saturating_add(novel) > MAX_EPHEMERAL_BLOCK_CLAIMS
            {
                return Err(EngineError::InvalidTransaction(
                    "no-store block-claim test index reached its fixed capacity".into(),
                ));
            }
            for (block_id, claims) in &changed_claims {
                self.ephemeral_block_claims
                    .insert(block_id.as_uuid().as_u128(), claims.clone());
            }
        }
        let insert_nanos =
            usize::try_from(insert_started.elapsed().as_nanos()).unwrap_or(usize::MAX);

        let novel_conflicts: Vec<_> = changed_claims
            .into_iter()
            .filter_map(|(block_id, claims)| {
                let homes: BTreeSet<_> =
                    claims.iter().map(|claim| claim.home_document_id).collect();
                (homes.len() > 1).then(|| ImmutableHomeConflict::from_claims(block_id, claims))
            })
            .collect();
        let mut candidate_roots = scratch_roots.clone();
        let mut candidate_fatal_handle = self.fatal_handle;
        if let Some(store) = self.scratch.as_ref().cloned() {
            let mut roots = candidate_roots;
            let mut handle = self.fatal_handle;
            for conflict in novel_conflicts {
                let (next_roots, next_handle) =
                    super::evidence_index::upsert_conflict(&store, &roots, handle, conflict)
                        .map_err(|error| {
                            let error = EngineError::Archive(error.to_string());
                            self.history_failure = Some(error.clone());
                            error
                        })?;
                roots = next_roots;
                handle = Some(next_handle);
            }
            candidate_roots = roots;
            candidate_fatal_handle = handle;
        } else {
            let mut conflicts: BTreeMap<_, _> = self
                .fatal_evidence
                .as_ref()
                .into_iter()
                .flat_map(|evidence| evidence.conflicts())
                .cloned()
                .map(|conflict| (conflict.block_id(), conflict))
                .collect();
            for conflict in novel_conflicts {
                conflicts.insert(conflict.block_id(), conflict);
            }
            if !conflicts.is_empty() {
                let evidence = ImmutableHomeEvidence::new(conflicts.into_values().collect());
                self.fatal_handle = Some(in_memory_evidence_handle(&evidence));
                self.fatal_evidence = Some(evidence);
            }
        }
        let mut work = self.history_work.get();
        work.block_claim_validation_nanos = work.block_claim_validation_nanos.saturating_add(
            usize::try_from(validation_started.elapsed().as_nanos()).unwrap_or(usize::MAX),
        );
        work.block_claim_lookup_nanos = work.block_claim_lookup_nanos.saturating_add(lookup_nanos);
        work.block_claim_encode_nanos = work.block_claim_encode_nanos.saturating_add(encode_nanos);
        work.block_claim_insert_nanos = work.block_claim_insert_nanos.saturating_add(insert_nanos);
        self.history_work.set(work);
        Ok(IdentityPublicationCandidate {
            blocked: candidate_fatal_handle.is_some() || self.fatal_evidence.is_some(),
            scratch_roots: candidate_roots,
            block_claim_root: candidate_block_claim_root,
            fatal_handle: candidate_fatal_handle,
        })
    }

    fn block_home_claims_many(
        &self,
        block_ids: &[BlockId],
    ) -> Result<AHashMap<u128, BTreeSet<ImmutableHomeClaim>>, EngineError> {
        if block_ids.is_empty() {
            return Ok(AHashMap::new());
        }
        if let Some(index) = &self.block_claim_index {
            let mut by_key: Vec<_> = block_ids
                .iter()
                .map(|block_id| (block_id.as_uuid().into_bytes(), *block_id))
                .collect();
            by_key.sort_unstable_by_key(|(key, _)| *key);
            let keys: Vec<_> = by_key.iter().map(|(key, _)| *key).collect();
            let found = index
                .lookup_many(self.block_claim_root, &keys)
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            return found
                .into_iter()
                .map(|(key, bytes)| {
                    let position = by_key
                        .binary_search_by_key(&key, |(candidate, _)| *candidate)
                        .expect("found claim key came from the requested set");
                    let block_id = by_key[position].1;
                    decode_block_claim_record(block_id, bytes.as_slice()).map(|record| {
                        (
                            block_id.as_uuid().as_u128(),
                            record.claims.into_iter().collect(),
                        )
                    })
                })
                .collect();
        }
        Ok(block_ids
            .iter()
            .filter_map(|block_id| {
                let key = block_id.as_uuid().as_u128();
                self.ephemeral_block_claims
                    .get(&key)
                    .cloned()
                    .map(|claims| (key, claims))
            })
            .collect())
    }

    fn apply_author_operation(
        &self,
        working: &mut BTreeMap<DocumentId, EngineDocument>,
        before_vectors: &mut BTreeMap<DocumentId, VersionVector>,
        before_snapshots: &mut BTreeMap<DocumentId, SemanticDocumentSnapshot>,
        peer_id: CrdtPeerId,
        operation: &SemanticOperation,
    ) -> Result<(), EngineError> {
        match operation {
            SemanticOperation::CreatePage {
                page_id,
                home_document_id,
                path,
            } => {
                if *home_document_id == self.catalog_document_id {
                    return Err(EngineError::InvalidTransaction(
                        "catalog and page-home document roles must be disjoint".into(),
                    ));
                }
                let catalog = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    self.catalog_document_id,
                    peer_id,
                )?;
                if read_page_state(catalog, *page_id)?.is_some() {
                    return Err(EngineError::PageAlreadyExists(*page_id));
                }
                insert_page_state(
                    catalog,
                    *page_id,
                    &PageState::Live {
                        path: path.clone(),
                        home_document_id: *home_document_id,
                    },
                )?;
                let shard = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    *home_document_id,
                    peer_id,
                )?;
                if shard_page_id(shard)?.is_some() {
                    return Err(EngineError::MalformedDocument {
                        document_id: *home_document_id,
                        reason: "home shard is already assigned".into(),
                    });
                }
                shard
                    .get_map(SHARD_META)
                    .insert(SHARD_PAGE_ID, page_id.to_string())
                    .map_err(loro_error)?;
            }
            SemanticOperation::EditPagePath { page_id, path } => {
                let catalog = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    self.catalog_document_id,
                    peer_id,
                )?;
                let state = require_live_page(catalog, *page_id)?;
                insert_page_state(
                    catalog,
                    *page_id,
                    &PageState::Live {
                        path: path.clone(),
                        home_document_id: state.home_document_id(),
                    },
                )?;
            }
            SemanticOperation::DeletePage { page_id } => {
                let catalog = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    self.catalog_document_id,
                    peer_id,
                )?;
                let state = require_live_page(catalog, *page_id)?;
                insert_page_state(
                    catalog,
                    *page_id,
                    &PageState::Tombstone {
                        home_document_id: state.home_document_id(),
                    },
                )?;
            }
            SemanticOperation::CreateBlock {
                block,
                page_id,
                parent,
                order,
                content,
            } => {
                let page_home = self.page_home_from_working(working, *page_id)?;
                if block.home_document_id != page_home {
                    return Err(EngineError::InvalidTransaction(
                        "new block home must be its creation page shard".into(),
                    ));
                }
                let claim = MembershipClaim::new(block.home_document_id, *parent, order.clone())?;
                let shard = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    block.home_document_id,
                    peer_id,
                )?;
                if read_block_state(block.home_document_id, shard, block.block_id)?.is_some() {
                    return Err(EngineError::BlockAlreadyExists(block.block_id));
                }
                if content.len() > super::semantic::MAX_BLOCK_CONTENT_BYTES {
                    return Err(EngineError::InvalidTransaction(
                        "block content exceeds the semantic bound".into(),
                    ));
                }
                shard
                    .get_map(SHARD_OWNERS)
                    .insert(&block.block_id.to_string(), page_id.to_string())
                    .map_err(loro_error)?;
                shard
                    .get_map(SHARD_CONTENT)
                    .ensure_mergeable_text(&block.block_id.to_string())
                    .map_err(loro_error)?
                    .insert(0, content)
                    .map_err(loro_error)?;
                insert_membership(shard, block.block_id, &claim)?;
            }
            SemanticOperation::EditBlockContent { block, content } => {
                let shard = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    block.home_document_id,
                    peer_id,
                )?;
                let text = block_text(shard, block.block_id)
                    .ok_or(EngineError::BlockNotFound(block.block_id))?;
                text.update(content, UpdateOptions::default())
                    .map_err(|error| EngineError::InvalidCrdt(error.to_string()))?;
            }
            SemanticOperation::MoveSubtree {
                root,
                from_page_id,
                to_page_id,
                parent,
                order,
            } => {
                let source_id = self.page_home_from_working(working, *from_page_id)?;
                let destination_id = self.page_home_from_working(working, *to_page_id)?;
                let source = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    source_id,
                    peer_id,
                )?;
                let all = read_memberships(source_id, source)?;
                let root_claim = all
                    .get(&root.block_id)
                    .ok_or(EngineError::BlockNotFound(root.block_id))?;
                if root_claim.home_document_id != root.home_document_id {
                    return Err(EngineError::HomeShardMismatch(root.block_id));
                }
                let subtree = subtree_claims(root.block_id, &all);
                let mut moved = Vec::with_capacity(subtree.len());
                for block_id in subtree {
                    let mut claim = all
                        .get(&block_id)
                        .expect("subtree claim came from membership map")
                        .clone();
                    if block_id == root.block_id {
                        claim.parent = *parent;
                        claim.order = order.clone();
                        claim.validate()?;
                    }
                    moved.push((block_id, claim));
                }
                for (block_id, claim) in &moved {
                    let home = self.ensure_working_document(
                        working,
                        before_vectors,
                        before_snapshots,
                        claim.home_document_id,
                        peer_id,
                    )?;
                    set_owner(home, *block_id, BlockOwner::Page(*to_page_id))?;
                }
                let source = working
                    .get(&source_id)
                    .expect("source is working")
                    .document();
                for (block_id, _) in &moved {
                    source
                        .get_map(SHARD_MEMBERS)
                        .delete(&block_id.to_string())
                        .map_err(loro_error)?;
                }
                let destination = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    destination_id,
                    peer_id,
                )?;
                for (block_id, claim) in moved {
                    insert_membership(destination, block_id, &claim)?;
                }
            }
            SemanticOperation::ReorderBlock {
                block_id,
                page_id,
                parent,
                order,
            } => {
                let page_document_id = self.page_home_from_working(working, *page_id)?;
                let page = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    page_document_id,
                    peer_id,
                )?;
                let mut claim = read_membership(page, *block_id)?
                    .ok_or(EngineError::BlockNotFound(*block_id))?;
                claim.parent = *parent;
                claim.order = order.clone();
                claim.validate()?;
                insert_membership(page, *block_id, &claim)?;
            }
            SemanticOperation::DeleteSubtree {
                root_block_id,
                page_id,
            } => {
                let page_document_id = self.page_home_from_working(working, *page_id)?;
                let page = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    page_document_id,
                    peer_id,
                )?;
                let all = read_memberships(page_document_id, page)?;
                if !all.contains_key(root_block_id) {
                    return Err(EngineError::BlockNotFound(*root_block_id));
                }
                let subtree = subtree_claims(*root_block_id, &all);
                for block_id in &subtree {
                    let claim = all.get(block_id).expect("subtree membership exists");
                    let home = self.ensure_working_document(
                        working,
                        before_vectors,
                        before_snapshots,
                        claim.home_document_id,
                        peer_id,
                    )?;
                    set_owner(home, *block_id, BlockOwner::Tombstone)?;
                }
                let page = working
                    .get(&page_document_id)
                    .expect("page is working")
                    .document();
                for block_id in subtree {
                    page.get_map(SHARD_MEMBERS)
                        .delete(&block_id.to_string())
                        .map_err(loro_error)?;
                }
            }
            SemanticOperation::RenamePageAndRewriteReferrers {
                page_id,
                path,
                referrers,
            } => {
                self.apply_author_operation(
                    working,
                    before_vectors,
                    before_snapshots,
                    peer_id,
                    &SemanticOperation::EditPagePath {
                        page_id: *page_id,
                        path: path.clone(),
                    },
                )?;
                for (block, content) in referrers {
                    self.apply_author_operation(
                        working,
                        before_vectors,
                        before_snapshots,
                        peer_id,
                        &SemanticOperation::EditBlockContent {
                            block: *block,
                            content: content.clone(),
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn page_home_from_working(
        &self,
        working: &BTreeMap<DocumentId, EngineDocument>,
        page_id: PageId,
    ) -> Result<DocumentId, EngineError> {
        let loaded;
        let catalog = if let Some(catalog) = working.get(&self.catalog_document_id) {
            catalog.document()
        } else if self.scratch.is_some() {
            loaded = self.clone_visible_document(self.catalog_document_id, 1)?;
            &loaded
        } else {
            self.visible_documents
                .get(&self.catalog_document_id)
                .ok_or(EngineError::PageNotFound(page_id))?
        };
        Ok(require_live_page(catalog, page_id)?.home_document_id())
    }
}

fn batch_fingerprint(batch: &ValidatedBatch) -> ContentDigest {
    batch_fingerprint_from_manifest(batch.manifest())
}

fn batch_fingerprint_from_manifest(manifest: &OperationBatch) -> ContentDigest {
    ContentDigest::of(
        &manifest
            .encode()
            .expect("validated batch manifest remains encodable"),
    )
}

fn prepared_manifest_fingerprint(batch: &PreparedBatch) -> ContentDigest {
    ContentDigest::of(
        &batch
            .manifest()
            .encode()
            .expect("prepared batch manifest remains encodable"),
    )
}

fn frontier_contains_batch(frontier: &FrontierV2, batch_id: BatchId) -> bool {
    frontier.documents().iter().any(|document| {
        document
            .direct_dependency_heads()
            .binary_search(&batch_id)
            .is_ok()
    })
}

fn declared_batch_heads(frontier: &FrontierV2) -> BTreeSet<BatchId> {
    frontier
        .documents()
        .iter()
        .flat_map(|document| document.direct_dependency_heads().iter().copied())
        .collect()
}

fn validate_maximal_document_heads(
    frontier: &FrontierV2,
    ancestry: &BTreeMap<BatchId, OperationBatch>,
) -> Result<(), EngineError> {
    for document in frontier.documents() {
        let direct_heads: BTreeSet<_> =
            document.direct_dependency_heads().iter().copied().collect();
        for root in &direct_heads {
            let mut pending = ancestry
                .get(root)
                .map(|manifest| declared_batch_heads(manifest.dependency_frontier()))
                .unwrap_or_default();
            let mut visited = BTreeSet::new();
            while let Some(ancestor) = pending.pop_first() {
                if !visited.insert(ancestor) {
                    continue;
                }
                if direct_heads.contains(&ancestor) {
                    return Err(EngineError::NonMaximalDependencyHead {
                        redundant: ancestor,
                        descendant: *root,
                    });
                }
                if let Some(manifest) = ancestry.get(&ancestor) {
                    pending.extend(declared_batch_heads(manifest.dependency_frontier()));
                }
            }
        }

        // The declared heads must not merely form an antichain. Derive the
        // canonical frontier from the immutable atomic DAG: a relevant batch
        // is one that actually carries a CRDT update for this document, and a
        // canonical head is a relevant batch with no relevant descendant.
        // Multi-source propagation visits every ancestry edge at most once,
        // including paths through cross-document-only atomic batches.
        let document_id = document.document_id();
        let relevant: BTreeSet<_> = ancestry
            .iter()
            .filter_map(|(batch_id, manifest)| {
                manifest
                    .required_objects()
                    .iter()
                    .any(|descriptor| {
                        descriptor.kind() == ObjectKind::CrdtUpdate
                            && descriptor.document_id() == document_id
                    })
                    .then_some(*batch_id)
            })
            .collect();
        let mut has_relevant_descendant = BTreeSet::new();
        let mut pending = BTreeSet::new();
        for batch_id in &relevant {
            if let Some(manifest) = ancestry.get(batch_id) {
                pending.extend(declared_batch_heads(manifest.dependency_frontier()));
            }
        }
        while let Some(batch_id) = pending.pop_first() {
            if !has_relevant_descendant.insert(batch_id) {
                continue;
            }
            if let Some(manifest) = ancestry.get(&batch_id) {
                pending.extend(declared_batch_heads(manifest.dependency_frontier()));
            }
        }
        let canonical: BTreeSet<_> = relevant
            .difference(&has_relevant_descendant)
            .copied()
            .collect();
        if direct_heads != canonical {
            return Err(EngineError::InexactDocumentDependencyHeads { document_id });
        }
    }
    Ok(())
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn encode_history_record(record: &ColdHistoryRecord) -> Result<Vec<u8>, EngineError> {
    postcard::to_allocvec(record).map_err(|error| EngineError::Archive(error.to_string()))
}

fn encode_archive_status(status: &ArchiveStatus) -> Result<Vec<u8>, EngineError> {
    postcard::to_allocvec(status).map_err(|error| EngineError::Archive(error.to_string()))
}

fn decode_archive_status(bytes: &[u8]) -> Result<ArchiveStatus, EngineError> {
    let status: ArchiveStatus =
        postcard::from_bytes(bytes).map_err(|error| EngineError::Archive(error.to_string()))?;
    if encode_archive_status(&status)? != bytes {
        return Err(EngineError::Archive(
            "non-canonical scratch batch status".into(),
        ));
    }
    Ok(status)
}

fn empty_accepted_frontier_root() -> AcceptedFrontierRoot {
    AcceptedFrontierRoot {
        schema_version: ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION,
        acceptance_sequence: 0,
        document_count: 0,
        state_digest: ContentDigest::of(b"tine/oplog/accepted-frontier/v1/empty"),
        scratch_root: None,
    }
}

fn next_accepted_frontier_root(
    prior: &AcceptedFrontierRoot,
    event_binding_digest: ContentDigest,
    acceptance_sequence: u64,
    document_count: u64,
    affected_documents: &[DocumentDependencies],
    scratch_root: Option<super::scratch_store::ScratchLsmRoot>,
) -> Result<AcceptedFrontierRoot, EngineError> {
    validate_accepted_frontier_root(prior)?;
    if acceptance_sequence != prior.acceptance_sequence.saturating_add(1) {
        return Err(EngineError::Archive(
            "accepted frontier sequence is not contiguous".into(),
        ));
    }
    let mut bytes = b"tine/oplog/accepted-frontier/v1\0".to_vec();
    bytes.extend_from_slice(prior.state_digest.as_bytes());
    bytes.extend_from_slice(event_binding_digest.as_bytes());
    bytes.extend_from_slice(&acceptance_sequence.to_be_bytes());
    bytes.extend_from_slice(&document_count.to_be_bytes());
    bytes.extend_from_slice(&(affected_documents.len() as u64).to_be_bytes());
    for document in affected_documents {
        let encoded = encode_accepted_document(document)?;
        bytes.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&encoded);
    }
    Ok(AcceptedFrontierRoot {
        schema_version: ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION,
        acceptance_sequence,
        document_count,
        state_digest: ContentDigest::of(&bytes),
        scratch_root,
    })
}

fn validate_accepted_frontier_root(root: &AcceptedFrontierRoot) -> Result<(), EngineError> {
    if root.schema_version != ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION {
        return Err(EngineError::Archive(format!(
            "unknown accepted-frontier root schema {}",
            root.schema_version
        )));
    }
    if root.acceptance_sequence == 0 {
        if root.document_count != 0
            || root.state_digest != empty_accepted_frontier_root().state_digest
            || root.scratch_root.is_some()
        {
            return Err(EngineError::Archive(
                "malformed empty accepted-frontier root".into(),
            ));
        }
    }
    Ok(())
}

fn validate_accepted_evidence(evidence: &AcceptedBatchEvidence) -> Result<(), EngineError> {
    if evidence.schema_version != ACCEPTED_EVIDENCE_SCHEMA_VERSION {
        return Err(EngineError::Archive(format!(
            "unknown accepted-evidence schema {}",
            evidence.schema_version
        )));
    }
    validate_accepted_frontier_root(&evidence.prior_frontier_root)?;
    validate_accepted_frontier_root(&evidence.post_frontier_root)?;
    if evidence.acceptance_sequence != evidence.post_frontier_root.acceptance_sequence
        || evidence.acceptance_sequence
            != evidence
                .prior_frontier_root
                .acceptance_sequence
                .saturating_add(1)
    {
        return Err(EngineError::Archive(format!(
            "accepted batch {} has a non-contiguous frontier transition",
            evidence.batch_id
        )));
    }
    if evidence
        .affected_documents
        .windows(2)
        .any(|pair| pair[0].document_id() >= pair[1].document_id())
    {
        return Err(EngineError::Archive(
            "accepted frontier affected documents are not canonical".into(),
        ));
    }
    let expected = next_accepted_frontier_root(
        &evidence.prior_frontier_root,
        evidence.event_binding_digest,
        evidence.acceptance_sequence,
        evidence.post_frontier_root.document_count,
        &evidence.affected_documents,
        evidence.post_frontier_root.scratch_root.clone(),
    )?;
    if expected != evidence.post_frontier_root {
        return Err(EngineError::Archive(format!(
            "accepted batch {} frontier root digest mismatch",
            evidence.batch_id
        )));
    }
    Ok(())
}

fn encode_accepted_document(dependencies: &DocumentDependencies) -> Result<Vec<u8>, EngineError> {
    postcard::to_allocvec(dependencies).map_err(|error| EngineError::Archive(error.to_string()))
}

fn decode_accepted_document(
    expected_document_id: DocumentId,
    bytes: &[u8],
) -> Result<DocumentDependencies, EngineError> {
    let dependencies: DocumentDependencies =
        postcard::from_bytes(bytes).map_err(|error| EngineError::Archive(error.to_string()))?;
    if dependencies.document_id() != expected_document_id {
        return Err(EngineError::Archive(format!(
            "accepted-frontier identity mismatch: expected {expected_document_id}, found {}",
            dependencies.document_id()
        )));
    }
    if encode_accepted_document(&dependencies)? != bytes {
        return Err(EngineError::Archive(
            "accepted-frontier bytes are not canonical".into(),
        ));
    }
    Ok(dependencies)
}

fn materialize_accepted_frontier(
    store: &ScratchStore,
    root: &super::scratch_store::ScratchLsmRoot,
) -> Result<FrontierV2, EngineError> {
    let records = store
        .materialize(
            root,
            super::scratch_store::ScratchPageKind::AcceptedFrontier,
        )
        .map_err(|error| EngineError::Archive(error.to_string()))?;
    let mut documents = Vec::with_capacity(records.len());
    for (key, bytes) in records {
        let document_id = Uuid::from_slice(&key)
            .map(DocumentId::from_uuid)
            .map_err(|error| {
                EngineError::Archive(format!("invalid accepted-frontier key: {error}"))
            })?;
        documents.push(decode_accepted_document(document_id, &bytes)?);
    }
    FrontierV2::new(documents).map_err(EngineError::from)
}

fn encode_block_claim_record(
    block_id: BlockId,
    claims: &BTreeSet<ImmutableHomeClaim>,
) -> Result<Vec<u8>, EngineError> {
    let claims: Vec<_> = claims.iter().copied().collect();
    encode_block_claim_record_slice(block_id, &claims)
}

fn encode_block_claim_record_slice(
    block_id: BlockId,
    claims: &[ImmutableHomeClaim],
) -> Result<Vec<u8>, EngineError> {
    if claims.is_empty() || !strictly_sorted(claims) {
        return Err(EngineError::Archive(
            "block-claim record claims must be nonempty and canonical".into(),
        ));
    }
    postcard::to_allocvec(&BlockClaimRecordRef {
        schema_version: BLOCK_CLAIM_RECORD_SCHEMA_VERSION,
        block_id,
        claims,
    })
    .map_err(|error| EngineError::Archive(error.to_string()))
}

fn encode_inline_block_claim_index_value(
    block_id: BlockId,
    claim: ImmutableHomeClaim,
) -> Result<BlockClaimIndexValue, EngineError> {
    let mut buffer = [0_u8; 128];
    let encoded = postcard::to_slice(
        &BlockClaimRecordRef {
            schema_version: BLOCK_CLAIM_RECORD_SCHEMA_VERSION,
            block_id,
            claims: &[claim],
        },
        &mut buffer,
    )
    .map_err(|error| EngineError::Archive(error.to_string()))?;
    Ok(BlockClaimIndexValue::from_slice(encoded))
}

fn decode_block_claim_record(
    expected_block_id: BlockId,
    bytes: &[u8],
) -> Result<BlockClaimRecord, EngineError> {
    let record: BlockClaimRecord =
        postcard::from_bytes(bytes).map_err(|error| EngineError::Archive(error.to_string()))?;
    if record.schema_version != BLOCK_CLAIM_RECORD_SCHEMA_VERSION
        || record.block_id != expected_block_id
        || record.claims.is_empty()
        || !strictly_sorted(&record.claims)
        || postcard::to_allocvec(&record)
            .map_err(|error| EngineError::Archive(error.to_string()))?
            != bytes
    {
        return Err(EngineError::Archive(
            "non-canonical or misbound block-claim record".into(),
        ));
    }
    Ok(record)
}

fn new_history_record(
    generation: u64,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    status: ArchiveStatus,
) -> ColdHistoryRecord {
    ColdHistoryRecord {
        schema_version: ENGINE_HISTORY_SCHEMA_VERSION,
        generation,
        batch_id,
        manifest_fingerprint,
        status,
    }
}

fn decode_history_record(
    expected_batch_id: BatchId,
    bytes: &[u8],
) -> Result<ColdHistoryRecord, EngineError> {
    let record: ColdHistoryRecord =
        postcard::from_bytes(bytes).map_err(|error| EngineError::Archive(error.to_string()))?;
    if record.schema_version != ENGINE_HISTORY_SCHEMA_VERSION
        || record.batch_id != expected_batch_id
        || encode_history_record(&record)? != bytes
    {
        return Err(EngineError::Archive(
            "non-canonical or misbound engine history record".into(),
        ));
    }
    Ok(record)
}

fn validate_history_catalog(
    records: &[ColdHistoryRecord],
    through_generation: u64,
) -> Result<(), EngineError> {
    for (index, record) in records.iter().enumerate() {
        if record.generation != index as u64 + 1 {
            return Err(EngineError::Archive(
                "engine history catalog is incomplete or has duplicate generations".into(),
            ));
        }
    }
    if records.len() as u64 != through_generation {
        return Err(EngineError::Archive(
            "engine history catalog does not match its authenticated generation".into(),
        ));
    }
    Ok(())
}

fn validated_history_records(
    store: &super::object_store::EngineHistoryStore,
    through_generation: u64,
    history_root: ContentDigest,
) -> Result<Vec<ColdHistoryRecord>, EngineError> {
    let mut records = store
        .materialize(history_root)
        .map_err(|error| EngineError::Archive(error.to_string()))?
        .into_iter()
        .map(|(batch_id, bytes)| {
            store.note_history_decode();
            decode_history_record(batch_id, &bytes)
        })
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_unstable_by_key(|record| record.generation);
    validate_history_catalog(&records, through_generation)?;
    Ok(records)
}

fn status_history_from_records(records: Vec<ColdHistoryRecord>) -> StatusHistory {
    let mut history = StatusHistory::default();
    for record in records {
        history.offered_batches.push(record.batch_id);
        match record.status {
            ArchiveStatus::Accepted { no_op, .. } => {
                history.accepted_batches.push(AcceptedBatch {
                    batch_id: record.batch_id,
                    no_op,
                });
            }
            ArchiveStatus::Quarantined => {
                history.validated_unpublished_batches.push(record.batch_id);
            }
            ArchiveStatus::Staged | ArchiveStatus::Rejected(_) => {}
        }
    }
    history
        .accepted_batches
        .sort_unstable_by_key(|accepted| accepted.batch_id);
    history.validated_unpublished_batches.sort_unstable();
    history.offered_batches.sort_unstable();
    history.offered_batches.dedup();
    history
}

fn disposition_from_final_status(status: ArchiveStatus, duplicate: bool) -> BatchDisposition {
    match status {
        ArchiveStatus::Accepted { no_op, .. } if duplicate => {
            BatchDisposition::DuplicateAccepted { no_op }
        }
        ArchiveStatus::Accepted { no_op, .. } => BatchDisposition::Accepted { no_op },
        ArchiveStatus::Quarantined => BatchDisposition::Quarantined,
        ArchiveStatus::Rejected(error) => BatchDisposition::Rejected { error },
        ArchiveStatus::Staged => unreachable!("cold engine history never stores staged status"),
    }
}

fn encode_crdt_update_payload(
    batch_id: BatchId,
    document_id: DocumentId,
    mut dependency_heads: Vec<BatchId>,
    mut batch_dependency_heads: Vec<BatchId>,
    causal_state_digest: Option<DocumentCausalDigest>,
    raw_update: Vec<u8>,
) -> Result<Vec<u8>, EngineError> {
    if raw_update.is_empty() {
        return Err(EngineError::InvalidCrdt("empty CRDT update payload".into()));
    }
    dependency_heads.sort_unstable();
    dependency_heads.dedup();
    batch_dependency_heads.sort_unstable();
    batch_dependency_heads.dedup();
    postcard::to_allocvec(&CrdtUpdatePayload {
        schema_version: CRDT_UPDATE_PAYLOAD_SCHEMA_VERSION,
        batch_id,
        document_id,
        dependency_heads,
        batch_dependency_heads,
        causal_state_digest,
        raw_update,
    })
    .map_err(|error| EngineError::InvalidCrdt(error.to_string()))
}

fn decode_crdt_update_payload(
    expected_batch_id: BatchId,
    expected_document_id: DocumentId,
    bytes: &[u8],
) -> Result<CrdtUpdatePayload, EngineError> {
    let payload: CrdtUpdatePayload = postcard::from_bytes(bytes).map_err(|error| {
        EngineError::InvalidCrdt(format!("invalid CRDT payload envelope: {error}"))
    })?;
    if payload.schema_version != CRDT_UPDATE_PAYLOAD_SCHEMA_VERSION {
        return Err(EngineError::InvalidCrdt(format!(
            "unknown CRDT payload schema {}",
            payload.schema_version
        )));
    }
    if payload.batch_id != expected_batch_id || payload.document_id != expected_document_id {
        return Err(EngineError::CrdtPayloadIdentityMismatch {
            expected_batch_id,
            expected_document_id,
            found_batch_id: payload.batch_id,
            found_document_id: payload.document_id,
        });
    }
    if payload.raw_update.is_empty() {
        return Err(EngineError::InvalidCrdt("empty CRDT update payload".into()));
    }
    if !strictly_sorted(&payload.dependency_heads)
        || !strictly_sorted(&payload.batch_dependency_heads)
    {
        return Err(EngineError::InvalidCrdt(
            "non-canonical CRDT dependency witness".into(),
        ));
    }
    let canonical = postcard::to_allocvec(&payload)
        .map_err(|error| EngineError::InvalidCrdt(error.to_string()))?;
    if canonical != bytes {
        return Err(EngineError::InvalidCrdt(
            "non-canonical CRDT payload envelope".into(),
        ));
    }
    Ok(payload)
}

fn validate_update_base(
    document_id: DocumentId,
    before: &LoroDoc,
    update: &[u8],
) -> Result<(), EngineError> {
    let metadata = LoroDoc::decode_import_blob_meta(update, true).map_err(loro_error)?;
    if metadata.mode != EncodedBlobMode::Updates {
        return Err(EngineError::InvalidCrdt(format!(
            "CRDT payload for {document_id} uses {}, expected update mode",
            metadata.mode
        )));
    }
    if metadata.start_frontiers != before.oplog_frontiers() {
        return Err(EngineError::CrdtUpdateBaseMismatch(document_id));
    }
    Ok(())
}

fn validate_immutable_shard_identity(
    document_id: DocumentId,
    prior_page_id: Option<PageId>,
    replacement: &LoroDoc,
) -> Result<(), EngineError> {
    let replacement_page_id = shard_page_id(replacement)?;
    if let Some(expected) = prior_page_id.filter(|prior| Some(*prior) != replacement_page_id) {
        return Err(EngineError::ShardPageIdentityChanged {
            document_id,
            expected,
            found: replacement_page_id,
        });
    }
    Ok(())
}

fn canonical_peer_counters(vv: &VersionVector) -> Result<Vec<CrdtPeerCounter>, EngineError> {
    let mut counters = Vec::new();
    for (peer, end) in vv.iter() {
        if *end <= 0 {
            continue;
        }
        let max_counter = u64::try_from(*end - 1)
            .map_err(|_| EngineError::InvalidCrdt("negative version-vector counter".into()))?;
        counters.push(CrdtPeerCounter::new(
            CrdtPeerId::from_u64(*peer),
            max_counter,
        ));
    }
    counters.sort_unstable_by_key(|counter| counter.peer_id());
    Ok(counters)
}

fn clone_doc(document: &LoroDoc, peer: u64) -> Result<LoroDoc, EngineError> {
    let bytes = document
        .export(ExportMode::all_updates())
        .map_err(|error| EngineError::InvalidCrdt(error.to_string()))?;
    let clone = LoroDoc::new();
    if !bytes.is_empty() {
        import_complete(DocumentId::from_uuid(uuid::Uuid::nil()), &clone, &[bytes])?;
    }
    clone.set_peer_id(peer).map_err(loro_error)?;
    Ok(clone)
}

fn import_complete(
    document_id: DocumentId,
    document: &LoroDoc,
    updates: &[Vec<u8>],
) -> Result<(), EngineError> {
    if updates.is_empty() {
        return Ok(());
    }
    let status = document.import_batch(updates).map_err(loro_error)?;
    if status.pending.is_some() {
        return Err(EngineError::MissingCrdtDependencies(document_id));
    }
    Ok(())
}

fn validate_document_roots(
    document_id: DocumentId,
    document: &LoroDoc,
    allowed: &[&str],
) -> Result<(), EngineError> {
    let LoroValue::Map(roots) = document.get_value() else {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "document root is not a map".into(),
        });
    };
    for (name, value) in roots.iter() {
        if !allowed.contains(&name.as_str()) {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("unexpected root container {name:?}"),
            });
        }
        if !matches!(
            value,
            LoroValue::Container(container_id)
                if container_id.container_type() == ContainerType::Map
        ) {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("root container {name:?} is not a map"),
            });
        }
    }
    Ok(())
}

fn validate_catalog(
    catalog_document_id: DocumentId,
    document: &LoroDoc,
) -> Result<BTreeMap<PageId, PageState>, EngineError> {
    validate_document_roots(catalog_document_id, document, &[CATALOG_PAGES])?;
    let pages = read_all_pages(document)?;
    for (page_id, state) in &pages {
        if state.home_document_id() == catalog_document_id {
            return Err(EngineError::MalformedDocument {
                document_id: catalog_document_id,
                reason: format!("catalog cannot be the immutable home of page {page_id}"),
            });
        }
    }
    Ok(pages)
}

fn validate_shard(
    catalog_document_id: DocumentId,
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<(), EngineError> {
    validate_shard_metadata(catalog_document_id, document_id, document)?;
    read_all_blocks(document_id, document)?;
    read_memberships(document_id, document)?;
    Ok(())
}

fn validate_shard_metadata(
    catalog_document_id: DocumentId,
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<(), EngineError> {
    if document_id == catalog_document_id {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "catalog cannot be used as a page shard".into(),
        });
    }
    validate_document_roots(
        document_id,
        document,
        &[SHARD_META, SHARD_OWNERS, SHARD_MEMBERS, SHARD_CONTENT],
    )?;
    validate_shard_metadata_shape(document_id, document)
}

fn validate_shard_metadata_shape(
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<(), EngineError> {
    let metadata = document.get_map(SHARD_META);
    if metadata.len() != 1
        || metadata
            .keys()
            .next()
            .is_none_or(|key| key.as_str() != SHARD_PAGE_ID)
    {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "shard metadata must contain only its page identity".into(),
        });
    }
    shard_page_id(document)?.ok_or_else(|| EngineError::MalformedDocument {
        document_id,
        reason: "shard has no page identity".into(),
    })?;
    Ok(())
}

#[cfg(test)]
fn derive_effect(
    catalog_document_id: DocumentId,
    before: &BTreeMap<DocumentId, LoroDoc>,
    after: &BTreeMap<DocumentId, LoroDoc>,
) -> Result<SemanticEffect, EngineError> {
    derive_effect_with_catalog(catalog_document_id, before, after).map(|(effect, _)| effect)
}

#[cfg(test)]
fn derive_effect_with_catalog(
    catalog_document_id: DocumentId,
    before: &BTreeMap<DocumentId, LoroDoc>,
    after: &BTreeMap<DocumentId, LoroDoc>,
) -> Result<(SemanticEffect, Option<BTreeMap<PageId, PageState>>), EngineError> {
    let before = snapshot_documents_with_validation(catalog_document_id, before, false)?;
    let after = snapshot_documents_with_validation(catalog_document_id, after, true)?;
    derive_effect_from_snapshots_with_catalog(&before, &after)
}

#[derive(Clone, Debug)]
enum SemanticDocumentSnapshot {
    Catalog(BTreeMap<PageId, PageState>),
    Shard {
        page_id: Option<PageId>,
        blocks: BTreeMap<BlockId, BlockState>,
        memberships: BTreeMap<BlockId, MembershipClaim>,
    },
}

#[cfg(test)]
thread_local! {
    static OWNED_SEMANTIC_SNAPSHOT_ENTRIES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn reset_owned_semantic_snapshot_entries() {
    OWNED_SEMANTIC_SNAPSHOT_ENTRIES.set(0);
}

#[cfg(test)]
fn owned_semantic_snapshot_entries() -> usize {
    OWNED_SEMANTIC_SNAPSHOT_ENTRIES.get()
}

#[cfg(test)]
fn record_owned_semantic_snapshot_entries(entries: usize) {
    OWNED_SEMANTIC_SNAPSHOT_ENTRIES.set(
        OWNED_SEMANTIC_SNAPSHOT_ENTRIES
            .get()
            .saturating_add(entries),
    );
}

fn snapshot_document(
    catalog_document_id: DocumentId,
    document_id: DocumentId,
    document: &LoroDoc,
    validate_shape: bool,
) -> Result<SemanticDocumentSnapshot, EngineError> {
    if document_id == catalog_document_id {
        if validate_shape {
            validate_document_roots(catalog_document_id, document, &[CATALOG_PAGES])?;
        }
        let pages = read_all_pages(document)?;
        for (page_id, state) in &pages {
            if state.home_document_id() == catalog_document_id {
                return Err(EngineError::MalformedDocument {
                    document_id: catalog_document_id,
                    reason: format!("catalog cannot be the immutable home of page {page_id}"),
                });
            }
        }
        Ok(SemanticDocumentSnapshot::Catalog(pages))
    } else {
        if validate_shape {
            validate_document_roots(
                document_id,
                document,
                &[SHARD_META, SHARD_OWNERS, SHARD_MEMBERS, SHARD_CONTENT],
            )?;
        }
        let blocks = read_all_blocks(document_id, document)?;
        let memberships = read_memberships(document_id, document)?;
        #[cfg(test)]
        record_owned_semantic_snapshot_entries(blocks.len().saturating_add(memberships.len()));
        Ok(SemanticDocumentSnapshot::Shard {
            page_id: shard_page_id(document)?,
            blocks,
            memberships,
        })
    }
}

fn snapshot_documents(
    catalog_document_id: DocumentId,
    documents: &BTreeMap<DocumentId, LoroDoc>,
) -> Result<BTreeMap<DocumentId, SemanticDocumentSnapshot>, EngineError> {
    snapshot_documents_with_validation(catalog_document_id, documents, true)
}

fn snapshot_engine_documents(
    catalog_document_id: DocumentId,
    documents: &BTreeMap<DocumentId, EngineDocument>,
    validate_shape: bool,
) -> Result<BTreeMap<DocumentId, SemanticDocumentSnapshot>, EngineError> {
    snapshot_engine_documents_excluding(
        catalog_document_id,
        documents,
        validate_shape,
        &BTreeSet::new(),
    )
}

fn snapshot_engine_documents_excluding(
    catalog_document_id: DocumentId,
    documents: &BTreeMap<DocumentId, EngineDocument>,
    validate_shape: bool,
    excluded: &BTreeSet<DocumentId>,
) -> Result<BTreeMap<DocumentId, SemanticDocumentSnapshot>, EngineError> {
    documents
        .iter()
        .filter(|(document_id, _)| !excluded.contains(document_id))
        .map(|(document_id, document)| {
            Ok((
                *document_id,
                snapshot_document(
                    catalog_document_id,
                    *document_id,
                    document.document(),
                    validate_shape,
                )?,
            ))
        })
        .collect()
}

fn validate_new_exact_shards_against_declared(
    catalog_document_id: DocumentId,
    documents: &BTreeMap<DocumentId, EngineDocument>,
    candidates: &BTreeSet<DocumentId>,
    declared: &SemanticEffect,
) -> Result<ValidatedNewShardEffects, EngineError> {
    let mut page_source_counts = BTreeMap::<PageId, usize>::new();
    for (document_id, document) in documents {
        if *document_id == catalog_document_id {
            continue;
        }
        if let Some(page_id) = shard_page_id(document.document())? {
            let count = page_source_counts.entry(page_id).or_default();
            *count = count.saturating_add(1);
        }
    }

    let mut validated = ValidatedNewShardEffects::default();
    for document_id in candidates {
        let document = documents
            .get(document_id)
            .expect("new exact shard candidate has an imported document")
            .document();
        let Some(page_id) = shard_page_id(document)? else {
            continue;
        };
        if page_source_counts.get(&page_id) != Some(&1) {
            // Duplicate page sources require the general merged-membership
            // comparator, which detects overlapping and disjoint key streams.
            continue;
        }
        validate_new_exact_shard_against_declared(
            catalog_document_id,
            *document_id,
            page_id,
            document,
            declared,
        )?;
        validated.documents.insert(*document_id);
        validated.pages.insert(page_id);
    }
    Ok(validated)
}

fn prime_empty_shard_roots(document: &LoroDoc) {
    // Root handles are arena-local structural identities even while the exact
    // VV is empty. Register them in the same order as a full empty snapshot so
    // imported mergeable-text containers retain checkpoint-stable indices.
    document.get_map(SHARD_META);
    document.get_map(SHARD_OWNERS);
    document.get_map(SHARD_CONTENT);
    document.get_map(SHARD_MEMBERS);
}

fn validate_new_exact_shard_against_declared(
    catalog_document_id: DocumentId,
    document_id: DocumentId,
    page_id: PageId,
    document: &LoroDoc,
    declared: &SemanticEffect,
) -> Result<(), EngineError> {
    validate_shard_metadata(catalog_document_id, document_id, document)?;

    let owners = document.get_map(SHARD_OWNERS);
    let content = document.get_map(SHARD_CONTENT);
    if owners.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "block entry bound exceeded".into(),
        });
    }
    if content.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "content entry bound exceeded".into(),
        });
    }
    if owners.len() != content.len() {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "owner and content key coverage differs".into(),
        });
    }

    let declared_blocks = declared.blocks();
    let block_start = declared_blocks.partition_point(|delta| delta.home_document_id < document_id);
    let block_end = declared_blocks.partition_point(|delta| delta.home_document_id <= document_id);
    let declared_blocks = &declared_blocks[block_start..block_end];
    if declared_blocks.len() != owners.len() {
        return Err(EngineError::SemanticEffectMismatch);
    }
    for key in owners.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        let owner = map_string(&owners, &key)?
            .ok_or_else(|| EngineError::InvalidCrdt(format!("owner {block_id} is not a string")))
            .and_then(|owner| parse_owner(&owner))?;
        let content =
            block_text(document, block_id).ok_or_else(|| EngineError::MalformedDocument {
                document_id,
                reason: format!("content {block_id} is missing or not mergeable text"),
            })?;
        let content = content.to_string();
        if content.len() > super::semantic::MAX_BLOCK_CONTENT_BYTES {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("content {block_id} exceeds the semantic bound"),
            });
        }
        let Ok(index) = declared_blocks.binary_search_by_key(&block_id, |delta| delta.block_id)
        else {
            return Err(EngineError::SemanticEffectMismatch);
        };
        let delta = &declared_blocks[index];
        let Some(after) = delta.after.as_ref() else {
            return Err(EngineError::SemanticEffectMismatch);
        };
        if delta.before.is_some()
            || delta.home_document_id != document_id
            || after.block_id != block_id
            || after.home_document_id != document_id
            || after.owner != owner
            || after.content != content
        {
            return Err(EngineError::SemanticEffectMismatch);
        }
    }
    for key in content.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if map_string(&owners, &key)?.is_none() || block_text(document, block_id).is_none() {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("content {block_id} has no matching owner and mergeable text"),
            });
        }
    }

    let members = document.get_map(SHARD_MEMBERS);
    if members.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "membership entry bound exceeded".into(),
        });
    }
    let declared_memberships = declared.memberships();
    let membership_start = declared_memberships.partition_point(|delta| delta.page_id < page_id);
    let membership_end = declared_memberships.partition_point(|delta| delta.page_id <= page_id);
    let declared_memberships = &declared_memberships[membership_start..membership_end];
    if declared_memberships.len() != members.len() {
        return Err(EngineError::SemanticEffectMismatch);
    }
    for key in members.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        let encoded = map_string(&members, &key)?.ok_or_else(|| {
            EngineError::InvalidCrdt(format!("membership {block_id} is not a string"))
        })?;
        let claim: MembershipClaim = decode_canonical(&encoded)?;
        let Ok(index) =
            declared_memberships.binary_search_by_key(&block_id, |delta| delta.block_id)
        else {
            return Err(EngineError::SemanticEffectMismatch);
        };
        let delta = &declared_memberships[index];
        if delta.before.is_some() || delta.after.as_ref() != Some(&claim) {
            return Err(EngineError::SemanticEffectMismatch);
        }
    }
    Ok(())
}

fn snapshot_documents_with_validation(
    catalog_document_id: DocumentId,
    documents: &BTreeMap<DocumentId, LoroDoc>,
    validate_shape: bool,
) -> Result<BTreeMap<DocumentId, SemanticDocumentSnapshot>, EngineError> {
    documents
        .iter()
        .map(|(document_id, document)| {
            Ok((
                *document_id,
                snapshot_document(catalog_document_id, *document_id, document, validate_shape)?,
            ))
        })
        .collect()
}

fn derive_effect_from_snapshots(
    before: &BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    after: &BTreeMap<DocumentId, SemanticDocumentSnapshot>,
) -> Result<SemanticEffect, EngineError> {
    derive_effect_from_snapshots_with_catalog(before, after).map(|(effect, _)| effect)
}

fn derive_effect_from_snapshots_with_catalog(
    before: &BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    after: &BTreeMap<DocumentId, SemanticDocumentSnapshot>,
) -> Result<(SemanticEffect, Option<BTreeMap<PageId, PageState>>), EngineError> {
    let mut pages = Vec::new();
    let mut blocks = Vec::new();
    let mut memberships = Vec::new();
    let mut catalog_after_pages = None;
    let mut document_ids = BTreeSet::new();
    document_ids.extend(before.keys().copied());
    document_ids.extend(after.keys().copied());
    for document_id in document_ids {
        match (before.get(&document_id), after.get(&document_id)) {
            (
                None | Some(SemanticDocumentSnapshot::Catalog(_)),
                None | Some(SemanticDocumentSnapshot::Catalog(_)),
            ) => {
                let before_pages = match before.get(&document_id) {
                    Some(SemanticDocumentSnapshot::Catalog(pages)) => pages,
                    _ => &BTreeMap::new(),
                };
                let after_pages = match after.get(&document_id) {
                    Some(SemanticDocumentSnapshot::Catalog(pages)) => pages,
                    _ => &BTreeMap::new(),
                };
                if before_pages.is_empty() {
                    pages.extend(after_pages.iter().map(|(page_id, state)| PageDelta {
                        page_id: *page_id,
                        before: None,
                        after: Some(state.clone()),
                    }));
                } else if after_pages.is_empty() {
                    pages.extend(before_pages.iter().map(|(page_id, state)| PageDelta {
                        page_id: *page_id,
                        before: Some(state.clone()),
                        after: None,
                    }));
                } else {
                    let keys: BTreeSet<PageId> = before_pages
                        .keys()
                        .chain(after_pages.keys())
                        .copied()
                        .collect();
                    for page_id in keys {
                        let before_state = before_pages.get(&page_id).cloned();
                        let after_state = after_pages.get(&page_id).cloned();
                        if before_state != after_state {
                            pages.push(PageDelta {
                                page_id,
                                before: before_state,
                                after: after_state,
                            });
                        }
                    }
                }
                catalog_after_pages = Some(after_pages.clone());
            }
            (
                None
                | Some(SemanticDocumentSnapshot::Shard {
                    page_id: _,
                    blocks: _,
                    memberships: _,
                }),
                None
                | Some(SemanticDocumentSnapshot::Shard {
                    page_id: _,
                    blocks: _,
                    memberships: _,
                }),
            ) => {
                let (before_page_id, before_blocks, before_members) = match before.get(&document_id)
                {
                    Some(SemanticDocumentSnapshot::Shard {
                        page_id,
                        blocks,
                        memberships,
                    }) => (*page_id, blocks, memberships),
                    _ => (None, &BTreeMap::new(), &BTreeMap::new()),
                };
                let (after_page_id, after_blocks, after_members) = match after.get(&document_id) {
                    Some(SemanticDocumentSnapshot::Shard {
                        page_id,
                        blocks,
                        memberships,
                    }) => (*page_id, blocks, memberships),
                    _ => (None, &BTreeMap::new(), &BTreeMap::new()),
                };
                if before_blocks.is_empty() {
                    blocks.extend(after_blocks.iter().map(|(block_id, state)| BlockDelta {
                        block_id: *block_id,
                        home_document_id: document_id,
                        before: None,
                        after: Some(state.clone()),
                    }));
                } else if after_blocks.is_empty() {
                    blocks.extend(before_blocks.iter().map(|(block_id, state)| BlockDelta {
                        block_id: *block_id,
                        home_document_id: document_id,
                        before: Some(state.clone()),
                        after: None,
                    }));
                } else {
                    let keys: BTreeSet<BlockId> = before_blocks
                        .keys()
                        .chain(after_blocks.keys())
                        .copied()
                        .collect();
                    for block_id in keys {
                        let before_state = before_blocks.get(&block_id).cloned();
                        let after_state = after_blocks.get(&block_id).cloned();
                        if before_state != after_state {
                            blocks.push(BlockDelta {
                                block_id,
                                home_document_id: document_id,
                                before: before_state,
                                after: after_state,
                            });
                        }
                    }
                }
                if before_page_id.is_some() && before_page_id != after_page_id {
                    return Err(EngineError::MalformedDocument {
                        document_id,
                        reason: "stable shard page identity changed".into(),
                    });
                }
                let page_id = after_page_id.or(before_page_id);
                if let Some(page_id) = page_id {
                    if before_members.is_empty() {
                        memberships.extend(after_members.iter().map(|(block_id, claim)| {
                            MembershipDelta {
                                page_id,
                                block_id: *block_id,
                                before: None,
                                after: Some(claim.clone()),
                            }
                        }));
                    } else if after_members.is_empty() {
                        memberships.extend(before_members.iter().map(|(block_id, claim)| {
                            MembershipDelta {
                                page_id,
                                block_id: *block_id,
                                before: Some(claim.clone()),
                                after: None,
                            }
                        }));
                    } else {
                        let member_keys: BTreeSet<BlockId> = before_members
                            .keys()
                            .chain(after_members.keys())
                            .copied()
                            .collect();
                        for block_id in member_keys {
                            let before_claim = before_members.get(&block_id).cloned();
                            let after_claim = after_members.get(&block_id).cloned();
                            if before_claim != after_claim {
                                memberships.push(MembershipDelta {
                                    page_id,
                                    block_id,
                                    before: before_claim,
                                    after: after_claim,
                                });
                            }
                        }
                    }
                }
            }
            _ => {
                return Err(EngineError::MalformedDocument {
                    document_id,
                    reason: "document changed between catalog and shard roles".into(),
                });
            }
        }
    }
    let effect = SemanticEffect::new(pages, blocks, memberships).map_err(EngineError::from)?;
    Ok((effect, catalog_after_pages))
}

struct MembershipSnapshotSource<'a> {
    before: Option<&'a BTreeMap<BlockId, MembershipClaim>>,
    after: Option<&'a BTreeMap<BlockId, MembershipClaim>>,
}

#[derive(Default)]
struct ValidatedNewShardEffects {
    documents: BTreeSet<DocumentId>,
    pages: BTreeSet<PageId>,
}

/// Verifies the decoded canonical declaration against the independently read
/// CRDT snapshots without constructing another owned semantic effect.
fn compare_declared_effect_against_snapshots_with_catalog<'a>(
    declared: &SemanticEffect,
    before: &'a BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    after: &'a BTreeMap<DocumentId, SemanticDocumentSnapshot>,
) -> Result<Option<&'a BTreeMap<PageId, PageState>>, EngineError> {
    compare_declared_effect_against_snapshots_with_catalog_skipping(
        declared,
        before,
        after,
        &ValidatedNewShardEffects::default(),
    )
}

fn compare_declared_effect_against_snapshots_with_catalog_skipping<'a>(
    declared: &SemanticEffect,
    before: &'a BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    after: &'a BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    validated_new_shards: &ValidatedNewShardEffects,
) -> Result<Option<&'a BTreeMap<PageId, PageState>>, EngineError> {
    let mut declared_pages = declared.pages().iter().peekable();
    let mut declared_blocks = declared.blocks().iter().peekable();
    let mut declared_memberships = declared.memberships().iter().peekable();
    let mut before_documents = before.iter().peekable();
    let mut after_documents = after.iter().peekable();
    let mut catalog_after_pages = None;
    let mut membership_sources: BTreeMap<PageId, Vec<MembershipSnapshotSource<'a>>> =
        BTreeMap::new();

    loop {
        let ordering = match (before_documents.peek(), after_documents.peek()) {
            (Some((before_id, _)), Some((after_id, _))) => Some(before_id.cmp(after_id)),
            (Some(_), None) => Some(std::cmp::Ordering::Less),
            (None, Some(_)) => Some(std::cmp::Ordering::Greater),
            (None, None) => None,
        };
        let Some(ordering) = ordering else {
            break;
        };
        let (document_id, before_snapshot, after_snapshot) = match ordering {
            std::cmp::Ordering::Less => {
                let (document_id, snapshot) = before_documents.next().expect("peeked before");
                (*document_id, Some(snapshot), None)
            }
            std::cmp::Ordering::Greater => {
                let (document_id, snapshot) = after_documents.next().expect("peeked after");
                (*document_id, None, Some(snapshot))
            }
            std::cmp::Ordering::Equal => {
                let (document_id, before_snapshot) =
                    before_documents.next().expect("peeked before");
                let (_, after_snapshot) = after_documents.next().expect("peeked after");
                (*document_id, Some(before_snapshot), Some(after_snapshot))
            }
        };

        skip_validated_new_shard_block_deltas(
            &mut declared_blocks,
            &validated_new_shards.documents,
        );
        match (before_snapshot, after_snapshot) {
            (
                None | Some(SemanticDocumentSnapshot::Catalog(_)),
                None | Some(SemanticDocumentSnapshot::Catalog(_)),
            ) => {
                let before_pages = match before_snapshot {
                    Some(SemanticDocumentSnapshot::Catalog(pages)) => Some(pages),
                    None => None,
                    Some(SemanticDocumentSnapshot::Shard { .. }) => unreachable!(),
                };
                let after_pages = match after_snapshot {
                    Some(SemanticDocumentSnapshot::Catalog(pages)) => Some(pages),
                    None => None,
                    Some(SemanticDocumentSnapshot::Shard { .. }) => unreachable!(),
                };
                if !compare_page_deltas(&mut declared_pages, before_pages, after_pages) {
                    return Err(EngineError::SemanticEffectMismatch);
                }
                catalog_after_pages = after_pages;
            }
            (
                None
                | Some(SemanticDocumentSnapshot::Shard {
                    page_id: _,
                    blocks: _,
                    memberships: _,
                }),
                None
                | Some(SemanticDocumentSnapshot::Shard {
                    page_id: _,
                    blocks: _,
                    memberships: _,
                }),
            ) => {
                let (before_page_id, before_blocks, before_memberships) = match before_snapshot {
                    Some(SemanticDocumentSnapshot::Shard {
                        page_id,
                        blocks,
                        memberships,
                    }) => (*page_id, Some(blocks), Some(memberships)),
                    None => (None, None, None),
                    Some(SemanticDocumentSnapshot::Catalog(_)) => unreachable!(),
                };
                let (after_page_id, after_blocks, after_memberships) = match after_snapshot {
                    Some(SemanticDocumentSnapshot::Shard {
                        page_id,
                        blocks,
                        memberships,
                    }) => (*page_id, Some(blocks), Some(memberships)),
                    None => (None, None, None),
                    Some(SemanticDocumentSnapshot::Catalog(_)) => unreachable!(),
                };
                if !compare_block_deltas(
                    &mut declared_blocks,
                    document_id,
                    before_blocks,
                    after_blocks,
                ) {
                    return Err(EngineError::SemanticEffectMismatch);
                }
                if before_page_id.is_some() && before_page_id != after_page_id {
                    return Err(EngineError::MalformedDocument {
                        document_id,
                        reason: "stable shard page identity changed".into(),
                    });
                }
                if let Some(page_id) = after_page_id.or(before_page_id) {
                    let source = MembershipSnapshotSource {
                        before: before_memberships,
                        after: after_memberships,
                    };
                    membership_sources.entry(page_id).or_default().push(source);
                }
            }
            _ => {
                return Err(EngineError::MalformedDocument {
                    document_id,
                    reason: "document changed between catalog and shard roles".into(),
                });
            }
        }
    }

    skip_validated_new_shard_block_deltas(&mut declared_blocks, &validated_new_shards.documents);
    if declared_pages.next().is_some() || declared_blocks.next().is_some() {
        return Err(EngineError::SemanticEffectMismatch);
    }
    for (page_id, sources) in membership_sources {
        skip_validated_new_shard_membership_deltas(
            &mut declared_memberships,
            &validated_new_shards.pages,
        );
        if !matches!(
            compare_membership_sources(&mut declared_memberships, page_id, sources,),
            MembershipComparison::Matches
        ) {
            return Err(EngineError::SemanticEffectMismatch);
        }
    }
    skip_validated_new_shard_membership_deltas(
        &mut declared_memberships,
        &validated_new_shards.pages,
    );
    if declared_memberships.next().is_some() {
        return Err(EngineError::SemanticEffectMismatch);
    }
    Ok(catalog_after_pages)
}

fn skip_validated_new_shard_block_deltas(
    declared: &mut std::iter::Peekable<std::slice::Iter<'_, BlockDelta>>,
    documents: &BTreeSet<DocumentId>,
) {
    while declared
        .peek()
        .is_some_and(|delta| documents.contains(&delta.home_document_id))
    {
        declared.next();
    }
}

fn skip_validated_new_shard_membership_deltas(
    declared: &mut std::iter::Peekable<std::slice::Iter<'_, MembershipDelta>>,
    pages: &BTreeSet<PageId>,
) {
    while declared
        .peek()
        .is_some_and(|delta| pages.contains(&delta.page_id))
    {
        declared.next();
    }
}

fn compare_page_deltas(
    declared: &mut std::iter::Peekable<std::slice::Iter<'_, PageDelta>>,
    before: Option<&BTreeMap<PageId, PageState>>,
    after: Option<&BTreeMap<PageId, PageState>>,
) -> bool {
    let mut before = before.into_iter().flat_map(BTreeMap::iter).peekable();
    let mut after = after.into_iter().flat_map(BTreeMap::iter).peekable();
    while before.peek().is_some() || after.peek().is_some() {
        let page_id = match (before.peek(), after.peek()) {
            (Some((before_id, _)), Some((after_id, _))) => (**before_id).min(**after_id),
            (Some((page_id, _)), None) | (None, Some((page_id, _))) => **page_id,
            (None, None) => unreachable!(),
        };
        let before_state = if before
            .peek()
            .is_some_and(|(candidate, _)| **candidate == page_id)
        {
            Some(before.next().expect("peeked before page").1)
        } else {
            None
        };
        let after_state = if after
            .peek()
            .is_some_and(|(candidate, _)| **candidate == page_id)
        {
            Some(after.next().expect("peeked after page").1)
        } else {
            None
        };
        if before_state == after_state {
            continue;
        }
        let Some(delta) = declared.next() else {
            return false;
        };
        if delta.page_id != page_id
            || delta.before.as_ref() != before_state
            || delta.after.as_ref() != after_state
        {
            return false;
        }
    }
    true
}

fn compare_block_deltas(
    declared: &mut std::iter::Peekable<std::slice::Iter<'_, BlockDelta>>,
    home_document_id: DocumentId,
    before: Option<&BTreeMap<BlockId, BlockState>>,
    after: Option<&BTreeMap<BlockId, BlockState>>,
) -> bool {
    let mut before = before.into_iter().flat_map(BTreeMap::iter).peekable();
    let mut after = after.into_iter().flat_map(BTreeMap::iter).peekable();
    while before.peek().is_some() || after.peek().is_some() {
        let block_id = match (before.peek(), after.peek()) {
            (Some((before_id, _)), Some((after_id, _))) => (**before_id).min(**after_id),
            (Some((block_id, _)), None) | (None, Some((block_id, _))) => **block_id,
            (None, None) => unreachable!(),
        };
        let before_state = if before
            .peek()
            .is_some_and(|(candidate, _)| **candidate == block_id)
        {
            Some(before.next().expect("peeked before block").1)
        } else {
            None
        };
        let after_state = if after
            .peek()
            .is_some_and(|(candidate, _)| **candidate == block_id)
        {
            Some(after.next().expect("peeked after block").1)
        } else {
            None
        };
        if before_state == after_state {
            continue;
        }
        let Some(delta) = declared.next() else {
            return false;
        };
        if delta.block_id != block_id
            || delta.home_document_id != home_document_id
            || delta.before.as_ref() != before_state
            || delta.after.as_ref() != after_state
        {
            return false;
        }
    }
    true
}

enum MembershipComparison {
    Matches,
    Mismatch,
    DuplicateDerivedKey,
}

struct DerivedMembershipDelta<'a> {
    block_id: BlockId,
    before: Option<&'a MembershipClaim>,
    after: Option<&'a MembershipClaim>,
}

struct MembershipDeltaStream<'a> {
    before: Option<std::collections::btree_map::Iter<'a, BlockId, MembershipClaim>>,
    after: Option<std::collections::btree_map::Iter<'a, BlockId, MembershipClaim>>,
    before_current: Option<(&'a BlockId, &'a MembershipClaim)>,
    after_current: Option<(&'a BlockId, &'a MembershipClaim)>,
}

impl<'a> MembershipDeltaStream<'a> {
    fn new(source: MembershipSnapshotSource<'a>) -> Self {
        let mut stream = Self {
            before: source.before.map(BTreeMap::iter),
            after: source.after.map(BTreeMap::iter),
            before_current: None,
            after_current: None,
        };
        stream.advance_before();
        stream.advance_after();
        stream
    }

    fn advance_before(&mut self) {
        self.before_current = self.before.as_mut().and_then(Iterator::next);
    }

    fn advance_after(&mut self) {
        self.after_current = self.after.as_mut().and_then(Iterator::next);
    }

    fn next(&mut self) -> Option<DerivedMembershipDelta<'a>> {
        loop {
            let block_id = match (self.before_current, self.after_current) {
                (Some((before_id, _)), Some((after_id, _))) => (*before_id).min(*after_id),
                (Some((block_id, _)), None) | (None, Some((block_id, _))) => *block_id,
                (None, None) => return None,
            };
            let before = if self
                .before_current
                .is_some_and(|(candidate, _)| *candidate == block_id)
            {
                let claim = self.before_current.expect("checked before membership").1;
                self.advance_before();
                Some(claim)
            } else {
                None
            };
            let after = if self
                .after_current
                .is_some_and(|(candidate, _)| *candidate == block_id)
            {
                let claim = self.after_current.expect("checked after membership").1;
                self.advance_after();
                Some(claim)
            } else {
                None
            };
            if before != after {
                return Some(DerivedMembershipDelta {
                    block_id,
                    before,
                    after,
                });
            }
        }
    }
}

fn compare_membership_sources(
    declared: &mut std::iter::Peekable<std::slice::Iter<'_, MembershipDelta>>,
    page_id: PageId,
    sources: Vec<MembershipSnapshotSource<'_>>,
) -> MembershipComparison {
    let mut streams: Vec<MembershipDeltaStream<'_>> = sources
        .into_iter()
        .map(MembershipDeltaStream::new)
        .collect();
    let mut pending: Vec<Option<DerivedMembershipDelta<'_>>> = streams
        .iter_mut()
        .map(MembershipDeltaStream::next)
        .collect();
    loop {
        let mut selected: Option<usize> = None;
        for (index, candidate) in pending.iter().enumerate() {
            let Some(candidate) = candidate else {
                continue;
            };
            if let Some(selected_index) = selected {
                let selected_delta = pending[selected_index].as_ref().expect("selected pending");
                if candidate.block_id == selected_delta.block_id {
                    return MembershipComparison::DuplicateDerivedKey;
                }
                if candidate.block_id < selected_delta.block_id {
                    selected = Some(index);
                }
            } else {
                selected = Some(index);
            }
        }
        let Some(selected) = selected else {
            return MembershipComparison::Matches;
        };
        let derived = pending[selected].take().expect("selected pending");
        let Some(delta) = declared.next() else {
            return MembershipComparison::Mismatch;
        };
        if delta.page_id != page_id
            || delta.block_id != derived.block_id
            || delta.before.as_ref() != derived.before
            || delta.after.as_ref() != derived.after
        {
            return MembershipComparison::Mismatch;
        }
        pending[selected] = streams[selected].next();
    }
}

fn read_all_pages(document: &LoroDoc) -> Result<BTreeMap<PageId, PageState>, EngineError> {
    let pages = document.get_map(CATALOG_PAGES);
    if pages.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::InvalidCrdt(
            "catalog entry bound exceeded".into(),
        ));
    }
    let mut result = BTreeMap::new();
    for key in pages.keys() {
        let page_id = PageId::from_str(&key)
            .map_err(|_| EngineError::InvalidCrdt(format!("invalid page key {key:?}")))?;
        let encoded = map_string(&pages, &key)?
            .ok_or_else(|| EngineError::InvalidCrdt("page register is not a string".into()))?;
        result.insert(page_id, decode_canonical(&encoded)?);
    }
    Ok(result)
}

fn read_page_state(document: &LoroDoc, page_id: PageId) -> Result<Option<PageState>, EngineError> {
    let pages = document.get_map(CATALOG_PAGES);
    map_string(&pages, &page_id.to_string())?
        .map(|encoded| decode_canonical(&encoded))
        .transpose()
}

fn require_live_page(document: &LoroDoc, page_id: PageId) -> Result<PageState, EngineError> {
    match read_page_state(document, page_id)? {
        Some(state @ PageState::Live { .. }) => Ok(state),
        Some(PageState::Tombstone { .. }) => Err(EngineError::PageDeleted(page_id)),
        None => Err(EngineError::PageNotFound(page_id)),
    }
}

fn insert_page_state(
    document: &LoroDoc,
    page_id: PageId,
    state: &PageState,
) -> Result<(), EngineError> {
    document
        .get_map(CATALOG_PAGES)
        .insert(&page_id.to_string(), encode_canonical(state)?)
        .map_err(loro_error)
}

fn shard_page_id(document: &LoroDoc) -> Result<Option<PageId>, EngineError> {
    map_string(&document.get_map(SHARD_META), SHARD_PAGE_ID)?
        .map(|value| {
            PageId::from_str(&value)
                .map_err(|_| EngineError::InvalidCrdt(format!("invalid shard page ID {value:?}")))
        })
        .transpose()
}

fn read_all_blocks(
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<BTreeMap<BlockId, BlockState>, EngineError> {
    let owners = document.get_map(SHARD_OWNERS);
    let content = document.get_map(SHARD_CONTENT);
    if owners.len() > MAX_DOCUMENT_ENTRIES || content.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "shard entry bound exceeded".into(),
        });
    }
    let mut result = BTreeMap::new();
    for key in owners.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if let Some(state) = read_block_state(document_id, document, block_id)? {
            result.insert(block_id, state);
        }
    }
    for key in content.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if !result.contains_key(&block_id) {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("content {block_id} has no owner register"),
            });
        }
        if block_text(document, block_id).is_none() {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("content {block_id} is not mergeable text"),
            });
        }
    }
    Ok(result)
}

fn read_block_state(
    document_id: DocumentId,
    document: &LoroDoc,
    block_id: BlockId,
) -> Result<Option<BlockState>, EngineError> {
    let Some(owner) = map_string(&document.get_map(SHARD_OWNERS), &block_id.to_string())? else {
        return Ok(None);
    };
    let content = block_text(document, block_id)
        .ok_or_else(|| EngineError::InvalidCrdt(format!("block {block_id} has no text")))?
        .to_string();
    Ok(Some(BlockState {
        block_id,
        home_document_id: document_id,
        owner: parse_owner(&owner)?,
        content,
    }))
}

fn has_block_state(document: &LoroDoc, block_id: BlockId) -> Result<bool, EngineError> {
    if map_string(&document.get_map(SHARD_OWNERS), &block_id.to_string())?.is_none() {
        return Ok(false);
    }
    Ok(block_text(document, block_id).is_some())
}

fn parse_owner(value: &str) -> Result<BlockOwner, EngineError> {
    if value == TOMBSTONE {
        Ok(BlockOwner::Tombstone)
    } else {
        PageId::from_str(value)
            .map(BlockOwner::Page)
            .map_err(|_| EngineError::InvalidCrdt(format!("invalid owner register {value:?}")))
    }
}

fn set_owner(document: &LoroDoc, block_id: BlockId, owner: BlockOwner) -> Result<(), EngineError> {
    if block_text(document, block_id).is_none() {
        return Err(EngineError::BlockNotFound(block_id));
    }
    let value = match owner {
        BlockOwner::Page(page_id) => page_id.to_string(),
        BlockOwner::Tombstone => TOMBSTONE.into(),
    };
    document
        .get_map(SHARD_OWNERS)
        .insert(&block_id.to_string(), value)
        .map_err(loro_error)
}

fn block_text(document: &LoroDoc, block_id: BlockId) -> Option<loro::LoroText> {
    match document.get_map(SHARD_CONTENT).get(&block_id.to_string()) {
        Some(ValueOrContainer::Container(Container::Text(text))) => Some(text),
        _ => None,
    }
}

fn read_memberships(
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<BTreeMap<BlockId, MembershipClaim>, EngineError> {
    let members = document.get_map(SHARD_MEMBERS);
    if members.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "membership entry bound exceeded".into(),
        });
    }
    let mut result = BTreeMap::new();
    for key in members.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        let encoded = map_string(&members, &key)?
            .ok_or_else(|| EngineError::InvalidCrdt("membership is not a string".into()))?;
        result.insert(block_id, decode_canonical(&encoded)?);
    }
    Ok(result)
}

fn read_membership(
    document: &LoroDoc,
    block_id: BlockId,
) -> Result<Option<MembershipClaim>, EngineError> {
    map_string(&document.get_map(SHARD_MEMBERS), &block_id.to_string())?
        .map(|encoded| decode_canonical(&encoded))
        .transpose()
}

fn insert_membership(
    document: &LoroDoc,
    block_id: BlockId,
    claim: &MembershipClaim,
) -> Result<(), EngineError> {
    claim.validate()?;
    document
        .get_map(SHARD_MEMBERS)
        .insert(&block_id.to_string(), encode_canonical(claim)?)
        .map_err(loro_error)
}

fn subtree_claims(root: BlockId, claims: &BTreeMap<BlockId, MembershipClaim>) -> Vec<BlockId> {
    let mut selected = BTreeSet::from([root]);
    let mut queue = VecDeque::from([root]);
    while let Some(parent) = queue.pop_front() {
        for (block_id, claim) in claims {
            if claim.parent == Some(parent) && selected.insert(*block_id) {
                queue.push_back(*block_id);
            }
        }
    }
    selected.into_iter().collect()
}

fn parse_block_key(document_id: DocumentId, key: &str) -> Result<BlockId, EngineError> {
    BlockId::from_str(key).map_err(|_| EngineError::MalformedDocument {
        document_id,
        reason: format!("invalid block key {key:?}"),
    })
}

fn map_string(map: &LoroMap, key: &str) -> Result<Option<String>, EngineError> {
    match map.get(key) {
        None => Ok(None),
        Some(ValueOrContainer::Value(LoroValue::String(value))) => Ok(Some((*value).clone())),
        Some(_) => Err(EngineError::InvalidCrdt(format!(
            "map value {key:?} is not a string"
        ))),
    }
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<String, EngineError> {
    serde_json::to_string(value).map_err(|error| EngineError::InvalidCrdt(error.to_string()))
}

fn decode_canonical<T>(value: &str) -> Result<T, EngineError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let decoded: T =
        serde_json::from_str(value).map_err(|error| EngineError::InvalidCrdt(error.to_string()))?;
    if encode_canonical(&decoded)? != value {
        return Err(EngineError::InvalidCrdt(
            "non-canonical embedded JSON register".into(),
        ));
    }
    Ok(decoded)
}

fn loro_error(error: loro::LoroError) -> EngineError {
    EngineError::InvalidCrdt(error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EngineError {
    Archive(String),
    Batch(String),
    Semantic(String),
    Receipt(String),
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    LineageMismatch {
        expected: LineageDigest,
        found: LineageDigest,
    },
    BatchCollision(BatchId),
    SelfDependency(BatchId),
    MissingDependency(BatchId),
    RejectedDependency(BatchId),
    NonMaximalDependencyHead {
        redundant: BatchId,
        descendant: BatchId,
    },
    InexactDocumentDependencyHeads {
        document_id: DocumentId,
    },
    CausalWitnessMismatch {
        document_id: DocumentId,
    },
    MissingDocumentUpdate {
        document_id: DocumentId,
        dependency: BatchId,
    },
    FrontierVectorMismatch(DocumentId),
    MissingCrdtDependencies(DocumentId),
    CrdtUpdateBaseMismatch(DocumentId),
    CrdtPayloadIdentityMismatch {
        expected_batch_id: BatchId,
        expected_document_id: DocumentId,
        found_batch_id: BatchId,
        found_document_id: DocumentId,
    },
    DuplicateDocumentUpdate(DocumentId),
    SemanticEffectMismatch,
    InvalidCrdt(String),
    InvalidTransaction(String),
    MalformedDocument {
        document_id: DocumentId,
        reason: String,
    },
    MissingDocument(DocumentId),
    PageAlreadyExists(PageId),
    PageNotFound(PageId),
    PageDeleted(PageId),
    BlockAlreadyExists(BlockId),
    BlockNotFound(BlockId),
    HomeShardMismatch(BlockId),
    WorkspaceBlocked(FatalEvidenceHandle),
    ShardPageIdentityChanged {
        document_id: DocumentId,
        expected: PageId,
        found: Option<PageId>,
    },
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Archive(error) => write!(f, "immutable archive error: {error}"),
            Self::Batch(error) => write!(f, "batch error: {error}"),
            Self::Semantic(error) => write!(f, "semantic effect error: {error}"),
            Self::Receipt(error) => write!(f, "frontier error: {error}"),
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::LineageMismatch { expected, found } => {
                write!(f, "lineage mismatch: expected {expected}, found {found}")
            }
            Self::BatchCollision(batch_id) => write!(f, "batch collision for {batch_id}"),
            Self::SelfDependency(batch_id) => write!(f, "batch {batch_id} depends on itself"),
            Self::MissingDependency(batch_id) => write!(f, "missing dependency {batch_id}"),
            Self::RejectedDependency(batch_id) => write!(f, "dependency {batch_id} was rejected"),
            Self::NonMaximalDependencyHead {
                redundant,
                descendant,
            } => write!(
                f,
                "direct dependency head {redundant} is already an ancestor of {descendant}"
            ),
            Self::InexactDocumentDependencyHeads { document_id } => write!(
                f,
                "direct dependency heads are not the exact relevant frontier for {document_id}"
            ),
            Self::CausalWitnessMismatch { document_id } => write!(
                f,
                "CRDT witness disagrees with compact causal frontier for {document_id}"
            ),
            Self::MissingDocumentUpdate {
                document_id,
                dependency,
            } => write!(
                f,
                "dependency {dependency} has no CRDT update for document {document_id}"
            ),
            Self::FrontierVectorMismatch(document_id) => {
                write!(f, "reconstructed CRDT frontier mismatch for {document_id}")
            }
            Self::MissingCrdtDependencies(document_id) => {
                write!(f, "CRDT update for {document_id} has missing dependencies")
            }
            Self::CrdtUpdateBaseMismatch(document_id) => {
                write!(f, "CRDT update for {document_id} was not exported from its declared base")
            }
            Self::CrdtPayloadIdentityMismatch {
                expected_batch_id,
                expected_document_id,
                found_batch_id,
                found_document_id,
            } => write!(
                f,
                "CRDT payload identity mismatch: expected batch {expected_batch_id} document {expected_document_id}, found batch {found_batch_id} document {found_document_id}"
            ),
            Self::DuplicateDocumentUpdate(document_id) => {
                write!(f, "duplicate CRDT update for {document_id}")
            }
            Self::SemanticEffectMismatch => {
                f.write_str("declared semantic effect does not match CRDT transitions")
            }
            Self::InvalidCrdt(error) => write!(f, "invalid CRDT update/state: {error}"),
            Self::InvalidTransaction(error) => write!(f, "invalid transaction: {error}"),
            Self::MalformedDocument {
                document_id,
                reason,
            } => write!(f, "malformed document {document_id}: {reason}"),
            Self::MissingDocument(document_id) => write!(f, "missing document {document_id}"),
            Self::PageAlreadyExists(page_id) => write!(f, "page {page_id} already exists"),
            Self::PageNotFound(page_id) => write!(f, "page {page_id} was not found"),
            Self::PageDeleted(page_id) => write!(f, "page {page_id} is deleted"),
            Self::BlockAlreadyExists(block_id) => write!(f, "block {block_id} already exists"),
            Self::BlockNotFound(block_id) => write!(f, "block {block_id} was not found"),
            Self::HomeShardMismatch(block_id) => {
                write!(f, "stable home shard mismatch for block {block_id}")
            }
            Self::WorkspaceBlocked(handle) => {
                write!(
                    f,
                    "workspace is fatally blocked: {} conflicting blocks, {} claims, evidence {}",
                    handle.conflicting_block_count(),
                    handle.claim_count(),
                    handle.canonical_digest()
                )
            }
            Self::ShardPageIdentityChanged {
                document_id,
                expected,
                found,
            } => write!(
                f,
                "shard {document_id} page identity changed from {expected} to {found:?}"
            ),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<super::BatchError> for EngineError {
    fn from(error: super::BatchError) -> Self {
        Self::Batch(error.to_string())
    }
}

impl From<SemanticError> for EngineError {
    fn from(error: SemanticError) -> Self {
        Self::Semantic(error.to_string())
    }
}

impl From<super::ReceiptError> for EngineError {
    fn from(error: super::ReceiptError) -> Self {
        Self::Receipt(error.to_string())
    }
}

#[cfg(test)]
mod validation_tests {
    use loro::LoroText;
    use uuid::Uuid;

    use super::*;

    fn validated_transition(
        engine: &ShardedHotEngine,
        author: AuthorBatch,
        before: &BTreeMap<DocumentId, LoroDoc>,
        after: &BTreeMap<DocumentId, LoroDoc>,
        frontier: FrontierV2,
    ) -> ValidatedBatch {
        let effect = derive_effect(engine.catalog_document_id, before, after).unwrap();
        validated_transition_with_effect(engine, author, before, after, frontier, effect)
    }

    fn validated_transition_with_effect(
        engine: &ShardedHotEngine,
        author: AuthorBatch,
        before: &BTreeMap<DocumentId, LoroDoc>,
        after: &BTreeMap<DocumentId, LoroDoc>,
        frontier: FrontierV2,
        effect: SemanticEffect,
    ) -> ValidatedBatch {
        let effect_bytes = effect.encode().unwrap();
        validated_transition_with_payload(engine, author, before, after, frontier, effect_bytes)
    }

    fn validated_transition_with_payload(
        engine: &ShardedHotEngine,
        author: AuthorBatch,
        before: &BTreeMap<DocumentId, LoroDoc>,
        after: &BTreeMap<DocumentId, LoroDoc>,
        frontier: FrontierV2,
        effect_bytes: Vec<u8>,
    ) -> ValidatedBatch {
        let mut objects = vec![OperationObject::new(
            engine.workspace_id,
            engine.catalog_document_id,
            ObjectKind::SemanticEffect,
            effect_bytes.clone(),
        )
        .unwrap()];
        let batch_dependency_heads: Vec<_> = frontier
            .documents()
            .iter()
            .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for (document_id, document) in after {
            let start = before
                .get(document_id)
                .map(LoroDoc::oplog_vv)
                .unwrap_or_default();
            let raw_update = document.export(ExportMode::updates(&start)).unwrap();
            let dependencies = frontier
                .documents()
                .iter()
                .find(|dependencies| dependencies.document_id() == *document_id);
            objects.push(
                OperationObject::new(
                    engine.workspace_id,
                    *document_id,
                    ObjectKind::CrdtUpdate,
                    encode_crdt_update_payload(
                        author.batch_id,
                        *document_id,
                        dependencies
                            .into_iter()
                            .flat_map(|dependencies| {
                                dependencies.direct_dependency_heads().iter().copied()
                            })
                            .collect(),
                        batch_dependency_heads.clone(),
                        dependencies.map(DocumentDependencies::causal_state_digest),
                        raw_update,
                    )
                    .unwrap(),
                )
                .unwrap(),
            );
        }
        let descriptors = objects
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let manifest = OperationBatch::new_with_causality(
            engine.workspace_id,
            engine.lineage_digest,
            author.batch_id,
            author.author_device_id,
            author.author_session_id,
            BatchCausalDot::new(CausalPeerId::from_device_id(author.author_device_id), 1).unwrap(),
            batch_dependency_heads,
            frontier,
            SemanticEffectDigest::of(&effect_bytes),
            descriptors,
        )
        .unwrap();
        ValidatedBatch::new(PreparedBatch::new(manifest, objects).unwrap())
    }

    #[derive(Serialize)]
    struct RawSemanticEffectWire {
        semantic_effect_schema_version: u32,
        pages: Vec<PageDelta>,
        blocks: Vec<BlockDelta>,
        memberships: Vec<MembershipDelta>,
    }

    fn raw_semantic_effect(
        pages: Vec<PageDelta>,
        blocks: Vec<BlockDelta>,
        memberships: Vec<MembershipDelta>,
    ) -> Vec<u8> {
        let body = postcard::to_allocvec(&RawSemanticEffectWire {
            semantic_effect_schema_version: crate::oplog::SEMANTIC_EFFECT_SCHEMA_VERSION,
            pages,
            blocks,
            memberships,
        })
        .unwrap();
        let mut bytes = b"TINESEM1".to_vec();
        bytes.extend(body);
        bytes
    }

    fn test_author(batch: u128, peer: u64) -> AuthorBatch {
        AuthorBatch {
            batch_id: BatchId::from_uuid(Uuid::from_u128(batch)),
            author_device_id: DeviceId::from_uuid(Uuid::from_u128(batch + 1_000)),
            author_session_id: SessionId::from_uuid(Uuid::from_u128(batch + 2_000)),
            crdt_peer_id: CrdtPeerId::from_u64(peer),
        }
    }

    fn dependencies_for(
        engine: &ShardedHotEngine,
        document_id: DocumentId,
        peer: u64,
    ) -> DocumentDependencies {
        let document = engine.clone_visible_document(document_id, peer).unwrap();
        DocumentDependencies::new(
            document_id,
            canonical_peer_counters(&document.oplog_vv()).unwrap(),
            engine
                .document_dependency_heads(document_id, false)
                .unwrap()
                .into_iter()
                .collect(),
        )
        .unwrap()
    }

    fn live_page(home_document_id: DocumentId, path: &str) -> PageState {
        PageState::Live {
            path: ManagedPath::parse(path).unwrap(),
            home_document_id,
        }
    }

    fn block_state(
        block_id: BlockId,
        home_document_id: DocumentId,
        owner: BlockOwner,
        content: &str,
    ) -> BlockState {
        BlockState {
            block_id,
            home_document_id,
            owner,
            content: content.into(),
        }
    }

    struct NewExactShardFixture {
        engine: ShardedHotEngine,
        author: AuthorBatch,
        before: BTreeMap<DocumentId, LoroDoc>,
        after: BTreeMap<DocumentId, LoroDoc>,
        effect: SemanticEffect,
        home_id: DocumentId,
        page_id: PageId,
        block_id: BlockId,
    }

    fn new_exact_shard_fixture(seed: u128) -> NewExactShardFixture {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(seed));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(seed + 1));
        let home_id = DocumentId::from_uuid(Uuid::from_u128(seed + 2));
        let page_id = PageId::from_uuid(Uuid::from_u128(seed + 3));
        let block_id = BlockId::from_uuid(Uuid::from_u128(seed + 4));
        let engine = ShardedHotEngine::new(
            workspace,
            LineageDigest::of(&seed.to_be_bytes()),
            catalog_id,
        );
        let catalog = LoroDoc::new();
        catalog.set_peer_id(seed as u64).unwrap();
        insert_page_state(&catalog, page_id, &live_page(home_id, "pages/Fast Path.md")).unwrap();
        let shard = LoroDoc::new();
        shard.set_peer_id(seed as u64).unwrap();
        shard
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, page_id.to_string())
            .unwrap();
        shard
            .get_map(SHARD_OWNERS)
            .insert(&block_id.to_string(), page_id.to_string())
            .unwrap();
        shard
            .get_map(SHARD_CONTENT)
            .insert_container(&block_id.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "fast content")
            .unwrap();
        insert_membership(
            &shard,
            block_id,
            &MembershipClaim::new(home_id, None, "a").unwrap(),
        )
        .unwrap();
        let before = BTreeMap::from([(catalog_id, LoroDoc::new()), (home_id, LoroDoc::new())]);
        let after = BTreeMap::from([(catalog_id, catalog), (home_id, shard)]);
        let effect = derive_effect(catalog_id, &before, &after).unwrap();
        NewExactShardFixture {
            engine,
            author: test_author(seed + 5, seed as u64),
            before,
            after,
            effect,
            home_id,
            page_id,
            block_id,
        }
    }

    fn assert_new_exact_shard_rejected(mut fixture: NewExactShardFixture) {
        let batch = validated_transition_with_effect(
            &fixture.engine,
            fixture.author,
            &fixture.before,
            &fixture.after,
            FrontierV2::new(Vec::new()).unwrap(),
            fixture.effect,
        );
        assert!(matches!(
            fixture.engine.stage_ready(batch).disposition(),
            BatchDisposition::Rejected { .. }
        ));
        assert!(fixture.engine.visible_documents.is_empty());
        assert!(fixture
            .engine
            .canonical_snapshot()
            .unwrap()
            .pages
            .is_empty());
    }

    fn catalog_snapshot(entries: Vec<(PageId, PageState)>) -> SemanticDocumentSnapshot {
        SemanticDocumentSnapshot::Catalog(entries.into_iter().collect())
    }

    fn shard_snapshot(
        page_id: Option<PageId>,
        blocks: Vec<BlockState>,
        memberships: Vec<(BlockId, MembershipClaim)>,
    ) -> SemanticDocumentSnapshot {
        SemanticDocumentSnapshot::Shard {
            page_id,
            blocks: blocks
                .into_iter()
                .map(|state| (state.block_id, state))
                .collect(),
            memberships: memberships.into_iter().collect(),
        }
    }

    fn comparator_fixture(
        transition: u128,
    ) -> (
        BTreeMap<DocumentId, SemanticDocumentSnapshot>,
        BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    ) {
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(70_000 + transition));
        let home_id = DocumentId::from_uuid(Uuid::from_u128(71_000 + transition));
        let page_id = PageId::from_uuid(Uuid::from_u128(72_000 + transition));
        let block_id = BlockId::from_uuid(Uuid::from_u128(73_000 + transition));
        let before_claim = MembershipClaim::new(home_id, None, "before").unwrap();
        let after_claim = MembershipClaim::new(home_id, None, "after").unwrap();
        let before_block =
            block_state(block_id, home_id, BlockOwner::Page(page_id), "before block");
        let after_block = block_state(block_id, home_id, BlockOwner::Page(page_id), "after block");

        let (
            before_pages,
            after_pages,
            before_blocks,
            after_blocks,
            before_memberships,
            after_memberships,
        ) = match transition % 3 {
            0 => (
                Vec::new(),
                vec![(page_id, live_page(home_id, "pages/Inserted.md"))],
                Vec::new(),
                vec![after_block],
                Vec::new(),
                vec![(block_id, after_claim)],
            ),
            1 => (
                vec![(page_id, live_page(home_id, "pages/Before.md"))],
                vec![(page_id, live_page(home_id, "pages/After.md"))],
                vec![before_block],
                vec![after_block],
                vec![(block_id, before_claim)],
                vec![(block_id, after_claim)],
            ),
            _ => (
                vec![(page_id, live_page(home_id, "pages/Removed.md"))],
                Vec::new(),
                vec![before_block.clone()],
                vec![before_block],
                vec![(block_id, before_claim)],
                Vec::new(),
            ),
        };
        (
            BTreeMap::from([
                (catalog_id, catalog_snapshot(before_pages)),
                (
                    home_id,
                    shard_snapshot(Some(page_id), before_blocks, before_memberships),
                ),
            ]),
            BTreeMap::from([
                (catalog_id, catalog_snapshot(after_pages)),
                (
                    home_id,
                    shard_snapshot(Some(page_id), after_blocks, after_memberships),
                ),
            ]),
        )
    }

    fn assert_comparator_mismatch(
        declared: &SemanticEffect,
        before: &BTreeMap<DocumentId, SemanticDocumentSnapshot>,
        after: &BTreeMap<DocumentId, SemanticDocumentSnapshot>,
    ) {
        assert!(matches!(
            compare_declared_effect_against_snapshots_with_catalog(declared, before, after),
            Err(EngineError::SemanticEffectMismatch)
        ));
    }

    #[test]
    fn borrowed_comparator_matches_owned_derivation_for_generated_valid_transitions() {
        for transition in 0..9 {
            let (before, after) = comparator_fixture(transition);
            let expected = derive_effect_from_snapshots(&before, &after).unwrap();
            assert!(!expected.is_empty());
            for declared in [
                expected.clone(),
                SemanticEffect::new(Vec::new(), Vec::new(), Vec::new()).unwrap(),
            ] {
                assert_eq!(
                    expected == declared,
                    compare_declared_effect_against_snapshots_with_catalog(
                        &declared, &before, &after
                    )
                    .is_ok(),
                    "transition {transition} diverged from owned derivation"
                );
            }
        }
    }

    #[test]
    fn borrowed_comparator_rejects_mismatch_and_exhaustion_for_each_delta_class() {
        let (before, after) = comparator_fixture(0);
        let expected = derive_effect_from_snapshots(&before, &after).unwrap();
        assert_eq!(expected.pages().len(), 1);
        assert_eq!(expected.blocks().len(), 1);
        assert_eq!(expected.memberships().len(), 1);

        let no_pages = SemanticEffect::new(
            Vec::new(),
            expected.blocks().to_vec(),
            expected.memberships().to_vec(),
        )
        .unwrap();
        let no_blocks = SemanticEffect::new(
            expected.pages().to_vec(),
            Vec::new(),
            expected.memberships().to_vec(),
        )
        .unwrap();
        let no_memberships = SemanticEffect::new(
            expected.pages().to_vec(),
            expected.blocks().to_vec(),
            Vec::new(),
        )
        .unwrap();
        for declared in [&no_pages, &no_blocks, &no_memberships] {
            assert_comparator_mismatch(declared, &before, &after);
        }

        let extra_page_id = PageId::from_uuid(Uuid::from_u128(80_001));
        let extra_home_id = DocumentId::from_uuid(Uuid::from_u128(80_002));
        let extra_block_id = BlockId::from_uuid(Uuid::from_u128(80_003));
        let mut extra_pages = expected.pages().to_vec();
        extra_pages.push(PageDelta {
            page_id: extra_page_id,
            before: None,
            after: Some(live_page(extra_home_id, "pages/Extra.md")),
        });
        let mut extra_blocks = expected.blocks().to_vec();
        extra_blocks.push(BlockDelta {
            block_id: extra_block_id,
            home_document_id: extra_home_id,
            before: None,
            after: Some(block_state(
                extra_block_id,
                extra_home_id,
                BlockOwner::Tombstone,
                "extra block",
            )),
        });
        let mut extra_memberships = expected.memberships().to_vec();
        extra_memberships.push(MembershipDelta {
            page_id: extra_page_id,
            block_id: extra_block_id,
            before: None,
            after: Some(MembershipClaim::new(extra_home_id, None, "extra").unwrap()),
        });
        for declared in [
            SemanticEffect::new(
                extra_pages,
                expected.blocks().to_vec(),
                expected.memberships().to_vec(),
            )
            .unwrap(),
            SemanticEffect::new(
                expected.pages().to_vec(),
                extra_blocks,
                expected.memberships().to_vec(),
            )
            .unwrap(),
            SemanticEffect::new(
                expected.pages().to_vec(),
                expected.blocks().to_vec(),
                extra_memberships,
            )
            .unwrap(),
        ] {
            assert_comparator_mismatch(&declared, &before, &after);
        }

        let mut mismatched_pages = expected.pages().to_vec();
        mismatched_pages[0].after = Some(live_page(
            mismatched_pages[0]
                .after
                .as_ref()
                .unwrap()
                .home_document_id(),
            "pages/Different.md",
        ));
        let mut mismatched_blocks = expected.blocks().to_vec();
        mismatched_blocks[0].after.as_mut().unwrap().content = "different block".into();
        let mut mismatched_memberships = expected.memberships().to_vec();
        mismatched_memberships[0].after.as_mut().unwrap().order = "different".into();
        for declared in [
            SemanticEffect::new(
                mismatched_pages,
                expected.blocks().to_vec(),
                expected.memberships().to_vec(),
            )
            .unwrap(),
            SemanticEffect::new(
                expected.pages().to_vec(),
                mismatched_blocks,
                expected.memberships().to_vec(),
            )
            .unwrap(),
            SemanticEffect::new(
                expected.pages().to_vec(),
                expected.blocks().to_vec(),
                mismatched_memberships,
            )
            .unwrap(),
        ] {
            assert_comparator_mismatch(&declared, &before, &after);
        }
    }

    #[test]
    fn borrowed_comparator_uses_page_order_and_retains_disjoint_duplicate_page_sources() {
        let low_document_id = DocumentId::from_uuid(Uuid::from_u128(81_001));
        let high_document_id = DocumentId::from_uuid(Uuid::from_u128(81_002));
        let low_page_id = PageId::from_uuid(Uuid::from_u128(81_003));
        let high_page_id = PageId::from_uuid(Uuid::from_u128(81_004));
        let low_block_id = BlockId::from_uuid(Uuid::from_u128(81_005));
        let high_block_id = BlockId::from_uuid(Uuid::from_u128(81_006));
        let before = BTreeMap::new();
        let after = BTreeMap::from([
            (
                low_document_id,
                shard_snapshot(
                    Some(high_page_id),
                    Vec::new(),
                    vec![(
                        high_block_id,
                        MembershipClaim::new(low_document_id, None, "a").unwrap(),
                    )],
                ),
            ),
            (
                high_document_id,
                shard_snapshot(
                    Some(low_page_id),
                    Vec::new(),
                    vec![(
                        low_block_id,
                        MembershipClaim::new(high_document_id, None, "b").unwrap(),
                    )],
                ),
            ),
        ]);
        let declared = derive_effect_from_snapshots(&before, &after).unwrap();
        assert_eq!(
            declared
                .memberships()
                .iter()
                .map(|delta| delta.page_id)
                .collect::<Vec<_>>(),
            vec![low_page_id, high_page_id]
        );
        assert!(
            compare_declared_effect_against_snapshots_with_catalog(&declared, &before, &after)
                .is_ok()
        );

        let duplicate_page_id = PageId::from_uuid(Uuid::from_u128(81_007));
        let disjoint_duplicate_after = BTreeMap::from([
            (
                low_document_id,
                shard_snapshot(
                    Some(duplicate_page_id),
                    Vec::new(),
                    vec![(
                        low_block_id,
                        MembershipClaim::new(low_document_id, None, "a").unwrap(),
                    )],
                ),
            ),
            (
                high_document_id,
                shard_snapshot(
                    Some(duplicate_page_id),
                    Vec::new(),
                    vec![(
                        high_block_id,
                        MembershipClaim::new(high_document_id, None, "b").unwrap(),
                    )],
                ),
            ),
        ]);
        let disjoint_declared =
            derive_effect_from_snapshots(&before, &disjoint_duplicate_after).unwrap();
        assert_eq!(disjoint_declared.memberships().len(), 2);
        assert!(compare_declared_effect_against_snapshots_with_catalog(
            &disjoint_declared,
            &before,
            &disjoint_duplicate_after
        )
        .is_ok());

        let duplicate_key_after = BTreeMap::from([
            (
                low_document_id,
                shard_snapshot(
                    Some(duplicate_page_id),
                    Vec::new(),
                    vec![(
                        low_block_id,
                        MembershipClaim::new(low_document_id, None, "a").unwrap(),
                    )],
                ),
            ),
            (
                high_document_id,
                shard_snapshot(
                    Some(duplicate_page_id),
                    Vec::new(),
                    vec![(
                        low_block_id,
                        MembershipClaim::new(high_document_id, None, "b").unwrap(),
                    )],
                ),
            ),
        ]);
        assert!(derive_effect_from_snapshots(&before, &duplicate_key_after).is_err());
        let empty = SemanticEffect::new(Vec::new(), Vec::new(), Vec::new()).unwrap();
        assert!(matches!(
            compare_declared_effect_against_snapshots_with_catalog(
                &empty,
                &before,
                &duplicate_key_after
            ),
            Err(EngineError::SemanticEffectMismatch)
        ));
    }

    #[test]
    fn borrowed_comparator_preserves_absent_page_identity_and_role_rejection() {
        let document_id = DocumentId::from_uuid(Uuid::from_u128(82_001));
        let page_id = PageId::from_uuid(Uuid::from_u128(82_002));
        let block_id = BlockId::from_uuid(Uuid::from_u128(82_003));
        let before = BTreeMap::from([(document_id, shard_snapshot(None, Vec::new(), Vec::new()))]);
        let after = BTreeMap::from([(
            document_id,
            shard_snapshot(
                None,
                Vec::new(),
                vec![(
                    block_id,
                    MembershipClaim::new(document_id, None, "a").unwrap(),
                )],
            ),
        )]);
        let declared = derive_effect_from_snapshots(&before, &after).unwrap();
        assert!(declared.memberships().is_empty());
        assert!(
            compare_declared_effect_against_snapshots_with_catalog(&declared, &before, &after)
                .is_ok()
        );

        let role_after = BTreeMap::from([(document_id, catalog_snapshot(Vec::new()))]);
        let empty = SemanticEffect::new(Vec::new(), Vec::new(), Vec::new()).unwrap();
        assert!(matches!(
            compare_declared_effect_against_snapshots_with_catalog(&empty, &before, &role_after),
            Err(EngineError::MalformedDocument { .. })
        ));

        let changed_page_after = BTreeMap::from([(
            document_id,
            shard_snapshot(Some(page_id), Vec::new(), Vec::new()),
        )]);
        let stable_before = BTreeMap::from([(
            document_id,
            shard_snapshot(
                Some(PageId::from_uuid(Uuid::from_u128(82_004))),
                Vec::new(),
                Vec::new(),
            ),
        )]);
        assert!(matches!(
            compare_declared_effect_against_snapshots_with_catalog(
                &empty,
                &stable_before,
                &changed_page_after
            ),
            Err(EngineError::MalformedDocument { .. })
        ));
    }

    #[test]
    fn import_batch_pending_is_rejected() {
        let source = LoroDoc::new();
        source.set_peer_id(41).unwrap();
        source.get_map("m").insert("first", "dependency").unwrap();
        let start = source.oplog_vv();
        source.get_map("m").insert("second", "suffix").unwrap();
        let suffix = source.export(ExportMode::updates(&start)).unwrap();
        let target = LoroDoc::new();
        assert!(matches!(
            import_complete(
                DocumentId::from_uuid(Uuid::from_u128(41)),
                &target,
                &[suffix]
            ),
            Err(EngineError::MissingCrdtDependencies(_))
        ));
    }

    #[test]
    fn block_claim_records_are_canonical_sorted_and_key_bound() {
        let block_id = BlockId::from_uuid(Uuid::from_u128(42));
        let other_block_id = BlockId::from_uuid(Uuid::from_u128(43));
        let claim_a = ImmutableHomeClaim::new(
            BatchId::from_uuid(Uuid::from_u128(44)),
            DocumentId::from_uuid(Uuid::from_u128(45)),
        );
        let claim_b = ImmutableHomeClaim::new(
            BatchId::from_uuid(Uuid::from_u128(46)),
            DocumentId::from_uuid(Uuid::from_u128(47)),
        );
        let claims = BTreeSet::from([claim_a, claim_b]);
        let bytes = encode_block_claim_record(block_id, &claims).unwrap();
        assert_eq!(
            decode_block_claim_record(block_id, &bytes).unwrap().claims,
            vec![claim_a, claim_b]
        );
        assert!(decode_block_claim_record(other_block_id, &bytes).is_err());

        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_block_claim_record(block_id, &trailing).is_err());
        for malformed_claims in [vec![], vec![claim_a, claim_a], vec![claim_b, claim_a]] {
            let malformed = postcard::to_allocvec(&BlockClaimRecord {
                schema_version: BLOCK_CLAIM_RECORD_SCHEMA_VERSION,
                block_id,
                claims: malformed_claims,
            })
            .unwrap();
            assert!(decode_block_claim_record(block_id, &malformed).is_err());
        }
    }

    #[test]
    fn pending_author_collision_never_exposes_the_wrong_speculative_state() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(100));
        let lineage = LineageDigest::of(b"pending-collision");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(101));
        let page_a = PageId::from_uuid(Uuid::from_u128(102));
        let page_b = PageId::from_uuid(Uuid::from_u128(103));
        let home_a = DocumentId::from_uuid(Uuid::from_u128(104));
        let home_b = DocumentId::from_uuid(Uuid::from_u128(105));
        let author = test_author(106, 106);
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        let original = engine
            .prepare_transaction(
                author,
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page_a,
                    home_document_id: home_a,
                    path: ManagedPath::parse("pages/A.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        let foreign_engine = ShardedHotEngine::new(workspace, lineage, catalog);
        let foreign = foreign_engine
            .prepare_transaction(
                author,
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page_b,
                    home_document_id: home_b,
                    path: ManagedPath::parse("pages/B.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();

        let before = engine.instrumentation();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(foreign))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let after = engine.instrumentation();
        assert_eq!(
            after.stage_structural_buffer_reuses - before.stage_structural_buffer_reuses,
            0
        );
        let snapshot = engine.canonical_snapshot().unwrap();
        assert!(!snapshot
            .pages
            .iter()
            .any(|(candidate, _)| *candidate == page_a));
        assert_eq!(
            snapshot
                .pages
                .iter()
                .find(|(candidate, _)| *candidate == page_b)
                .unwrap()
                .1
                .path(),
            Some(&ManagedPath::parse("pages/B.md").unwrap())
        );
        let before_collision = snapshot;
        assert!(matches!(
            engine.stage_ready(ValidatedBatch::new(original)).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::BatchCollision(found),
            } if found == author.batch_id
        ));
        assert_eq!(engine.canonical_snapshot().unwrap(), before_collision);
    }

    #[test]
    fn rejected_candidate_cannot_publish_matching_pending_author_documents() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(110));
        let foreign_workspace = WorkspaceId::from_uuid(Uuid::from_u128(111));
        let lineage = LineageDigest::of(b"pending-rejected");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(112));
        let page = PageId::from_uuid(Uuid::from_u128(113));
        let home = DocumentId::from_uuid(Uuid::from_u128(114));
        let author = test_author(115, 115);
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        let _pending = engine
            .prepare_transaction(
                author,
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page,
                    home_document_id: home,
                    path: ManagedPath::parse("pages/Must Not Appear.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        let foreign_engine = ShardedHotEngine::new(foreign_workspace, lineage, catalog);
        let foreign = foreign_engine
            .prepare_transaction(
                author,
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(116)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(117)),
                    path: ManagedPath::parse("pages/Foreign.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        engine
            .pending_author_documents
            .borrow_mut()
            .as_mut()
            .unwrap()
            .manifest_fingerprint = prepared_manifest_fingerprint(&foreign);

        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(foreign))
                .disposition(),
            BatchDisposition::Rejected {
                error: EngineError::WorkspaceMismatch { .. },
            }
        ));
        assert!(engine.canonical_snapshot().unwrap().pages.is_empty());
        assert!(engine.pending_author_documents.borrow().is_none());
        assert_eq!(engine.instrumentation().stage_structural_buffer_reuses, 0);
    }

    #[test]
    fn stale_pending_author_falls_back_and_quarantines_concurrent_home_conflict() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(120));
        let lineage = LineageDigest::of(b"pending-concurrent");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(121));
        let page_a = PageId::from_uuid(Uuid::from_u128(122));
        let page_b = PageId::from_uuid(Uuid::from_u128(123));
        let home_a = DocumentId::from_uuid(Uuid::from_u128(124));
        let home_b = DocumentId::from_uuid(Uuid::from_u128(125));
        let block_id = BlockId::from_uuid(Uuid::from_u128(126));
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        let genesis = engine
            .prepare_transaction(
                test_author(127, 127),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: page_a,
                        home_document_id: home_a,
                        path: ManagedPath::parse("pages/A.md").unwrap(),
                    },
                    SemanticOperation::CreatePage {
                        page_id: page_b,
                        home_document_id: home_b,
                        path: ManagedPath::parse("pages/B.md").unwrap(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        let genesis = ValidatedBatch::new(genesis);
        engine.stage_ready(genesis.clone());

        let local = engine
            .prepare_transaction(
                test_author(128, 128),
                &OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: home_a,
                    },
                    page_id: page_a,
                    parent: None,
                    order: "a".into(),
                    content: "local".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        let mut remote = ShardedHotEngine::new(workspace, lineage, catalog);
        remote.stage_ready(genesis);
        let remote_claim = remote
            .prepare_transaction(
                test_author(129, 129),
                &OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: home_b,
                    },
                    page_id: page_b,
                    parent: None,
                    order: "b".into(),
                    content: "remote".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(remote_claim))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let before = engine.instrumentation();
        assert!(matches!(
            engine.stage_ready(ValidatedBatch::new(local)).disposition(),
            BatchDisposition::Quarantined
        ));
        let after = engine.instrumentation();
        assert_eq!(
            after.stage_structural_buffer_reuses - before.stage_structural_buffer_reuses,
            0
        );
        assert!(matches!(
            engine.status().workspace(),
            WorkspaceStatus::Blocked(_)
        ));
        assert_eq!(engine.status().accepted_batches().unwrap().len(), 2);
        assert_eq!(
            engine
                .status()
                .validated_unpublished_batch_ids()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn failed_prepare_and_corrupt_pending_buffer_can_only_force_full_validation() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(140));
        let lineage = LineageDigest::of(b"pending-negative-paths");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(141));
        let page = PageId::from_uuid(Uuid::from_u128(142));
        let home = DocumentId::from_uuid(Uuid::from_u128(143));
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        let prepared = engine
            .prepare_transaction(
                test_author(144, 144),
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page,
                    home_document_id: home,
                    path: ManagedPath::parse("pages/Safe.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            engine.prepare_transaction(
                test_author(145, 0),
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(146)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(147)),
                    path: ManagedPath::parse("pages/Never.md").unwrap(),
                }])
                .unwrap(),
            ),
            Err(EngineError::InvalidTransaction(_))
        ));
        assert!(engine.pending_author_documents.borrow().is_none());
        let before = engine.instrumentation();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(prepared))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let after = engine.instrumentation();
        assert_eq!(
            after.stage_structural_buffer_reuses - before.stage_structural_buffer_reuses,
            0
        );

        let edited = engine
            .prepare_transaction(
                test_author(148, 148),
                &OperationTransaction::new(vec![SemanticOperation::EditPagePath {
                    page_id: page,
                    path: ManagedPath::parse("pages/Validated.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        engine
            .pending_author_documents
            .borrow_mut()
            .as_mut()
            .unwrap()
            .documents
            .get(&catalog)
            .unwrap()
            .get_map("unexpected_root")
            .insert("poison", true)
            .unwrap();
        let before = engine.instrumentation();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(edited))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let after = engine.instrumentation();
        assert_eq!(
            after.stage_structural_buffer_reuses - before.stage_structural_buffer_reuses,
            0
        );
        let snapshot = engine.canonical_snapshot().unwrap();
        assert_eq!(
            snapshot
                .pages
                .iter()
                .find(|(candidate, _)| *candidate == page)
                .unwrap()
                .1
                .path(),
            Some(&ManagedPath::parse("pages/Validated.md").unwrap())
        );
    }

    #[test]
    fn pending_buffer_eviction_never_skips_validation_of_either_batch() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(150));
        let lineage = LineageDigest::of(b"pending-eviction");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(151));
        let page_a = PageId::from_uuid(Uuid::from_u128(152));
        let page_b = PageId::from_uuid(Uuid::from_u128(153));
        let home_a = DocumentId::from_uuid(Uuid::from_u128(154));
        let home_b = DocumentId::from_uuid(Uuid::from_u128(155));
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        let first = engine
            .prepare_transaction(
                test_author(156, 156),
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page_a,
                    home_document_id: home_a,
                    path: ManagedPath::parse("pages/A.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        let second = engine
            .prepare_transaction(
                test_author(157, 157),
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page_b,
                    home_document_id: home_b,
                    path: ManagedPath::parse("pages/B.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        let before = engine.instrumentation();
        assert!(matches!(
            engine.stage_ready(ValidatedBatch::new(first)).disposition(),
            BatchDisposition::Accepted { .. }
        ));
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(second))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let after = engine.instrumentation();
        assert_eq!(
            after.stage_structural_buffer_reuses - before.stage_structural_buffer_reuses,
            0
        );
        let snapshot = engine.canonical_snapshot().unwrap();
        assert_eq!(snapshot.pages.len(), 2);
        assert!(snapshot.pages.iter().any(|(page, _)| *page == page_a));
        assert!(snapshot.pages.iter().any(|(page, _)| *page == page_b));
    }

    #[test]
    fn new_exact_shard_fast_path_avoids_owned_block_and_membership_snapshots() {
        let mut fixture = new_exact_shard_fixture(90_000);
        let batch = validated_transition_with_effect(
            &fixture.engine,
            fixture.author,
            &fixture.before,
            &fixture.after,
            FrontierV2::new(Vec::new()).unwrap(),
            fixture.effect,
        );
        reset_owned_semantic_snapshot_entries();
        assert!(matches!(
            fixture.engine.stage_ready(batch).disposition(),
            BatchDisposition::Accepted { .. }
        ));
        assert_eq!(
            owned_semantic_snapshot_entries(),
            0,
            "new exact shard constructed an owned block or membership snapshot"
        );
        assert_eq!(
            fixture
                .engine
                .materialize_page(fixture.page_id)
                .unwrap()
                .blocks[0]
                .content,
            "fast content"
        );
    }

    #[test]
    fn new_exact_shard_fast_path_rejects_undeclared_and_extra_state_atomically() {
        let mut undeclared_block = new_exact_shard_fixture(90_100);
        undeclared_block.effect = SemanticEffect::new(
            undeclared_block.effect.pages().to_vec(),
            Vec::new(),
            undeclared_block.effect.memberships().to_vec(),
        )
        .unwrap();
        assert_new_exact_shard_rejected(undeclared_block);

        let undeclared_content = new_exact_shard_fixture(90_200);
        let extra_content_id = BlockId::from_uuid(Uuid::from_u128(90_299));
        undeclared_content.after[&undeclared_content.home_id]
            .get_map(SHARD_CONTENT)
            .insert_container(&extra_content_id.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "undeclared content")
            .unwrap();
        assert_new_exact_shard_rejected(undeclared_content);

        let undeclared_member = new_exact_shard_fixture(90_300);
        let extra_member_id = BlockId::from_uuid(Uuid::from_u128(90_399));
        insert_membership(
            &undeclared_member.after[&undeclared_member.home_id],
            extra_member_id,
            &MembershipClaim::new(undeclared_member.home_id, None, "b").unwrap(),
        )
        .unwrap();
        assert_new_exact_shard_rejected(undeclared_member);

        let mut extra_declaration = new_exact_shard_fixture(90_400);
        let extra_block_id = BlockId::from_uuid(Uuid::from_u128(90_499));
        let mut blocks = extra_declaration.effect.blocks().to_vec();
        blocks.push(BlockDelta {
            block_id: extra_block_id,
            home_document_id: extra_declaration.home_id,
            before: None,
            after: Some(block_state(
                extra_block_id,
                extra_declaration.home_id,
                BlockOwner::Page(extra_declaration.page_id),
                "extra declaration",
            )),
        });
        extra_declaration.effect = SemanticEffect::new(
            extra_declaration.effect.pages().to_vec(),
            blocks,
            extra_declaration.effect.memberships().to_vec(),
        )
        .unwrap();
        assert_new_exact_shard_rejected(extra_declaration);
    }

    #[test]
    fn new_exact_shard_fast_path_rejects_wrong_declared_fields_atomically() {
        let mut wrong_before = new_exact_shard_fixture(90_500);
        let mut blocks = wrong_before.effect.blocks().to_vec();
        blocks[0].before = Some(block_state(
            wrong_before.block_id,
            wrong_before.home_id,
            BlockOwner::Tombstone,
            "before",
        ));
        wrong_before.effect = SemanticEffect::new(
            wrong_before.effect.pages().to_vec(),
            blocks,
            wrong_before.effect.memberships().to_vec(),
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_before);

        let mut wrong_membership_before = new_exact_shard_fixture(90_550);
        let mut memberships = wrong_membership_before.effect.memberships().to_vec();
        memberships[0].before =
            Some(MembershipClaim::new(wrong_membership_before.home_id, None, "before").unwrap());
        wrong_membership_before.effect = SemanticEffect::new(
            wrong_membership_before.effect.pages().to_vec(),
            wrong_membership_before.effect.blocks().to_vec(),
            memberships,
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_membership_before);

        let mut wrong_owner = new_exact_shard_fixture(90_600);
        let mut blocks = wrong_owner.effect.blocks().to_vec();
        blocks[0].after.as_mut().unwrap().owner = BlockOwner::Tombstone;
        wrong_owner.effect = SemanticEffect::new(
            wrong_owner.effect.pages().to_vec(),
            blocks,
            wrong_owner.effect.memberships().to_vec(),
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_owner);

        let mut wrong_content = new_exact_shard_fixture(90_700);
        let mut blocks = wrong_content.effect.blocks().to_vec();
        blocks[0].after.as_mut().unwrap().content = "wrong content".into();
        wrong_content.effect = SemanticEffect::new(
            wrong_content.effect.pages().to_vec(),
            blocks,
            wrong_content.effect.memberships().to_vec(),
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_content);

        let mut wrong_claim = new_exact_shard_fixture(90_800);
        let mut memberships = wrong_claim.effect.memberships().to_vec();
        memberships[0].after.as_mut().unwrap().order = "wrong".into();
        wrong_claim.effect = SemanticEffect::new(
            wrong_claim.effect.pages().to_vec(),
            wrong_claim.effect.blocks().to_vec(),
            memberships,
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_claim);

        let mut wrong_page = new_exact_shard_fixture(90_900);
        let mut memberships = wrong_page.effect.memberships().to_vec();
        memberships[0].page_id = PageId::from_uuid(Uuid::from_u128(90_999));
        wrong_page.effect = SemanticEffect::new(
            wrong_page.effect.pages().to_vec(),
            wrong_page.effect.blocks().to_vec(),
            memberships,
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_page);

        let mut wrong_home = new_exact_shard_fixture(91_000);
        let mut memberships = wrong_home.effect.memberships().to_vec();
        memberships[0].after.as_mut().unwrap().home_document_id =
            DocumentId::from_uuid(Uuid::from_u128(91_099));
        wrong_home.effect = SemanticEffect::new(
            wrong_home.effect.pages().to_vec(),
            wrong_home.effect.blocks().to_vec(),
            memberships,
        )
        .unwrap();
        assert_new_exact_shard_rejected(wrong_home);
    }

    #[test]
    fn new_exact_shard_fast_path_rejects_malformed_roots_and_value_types_atomically() {
        let malformed_root = new_exact_shard_fixture(91_100);
        malformed_root.after[&malformed_root.home_id]
            .get_map("unexpected_root")
            .insert("poison", true)
            .unwrap();
        assert_new_exact_shard_rejected(malformed_root);

        let malformed_owner = new_exact_shard_fixture(91_200);
        malformed_owner.after[&malformed_owner.home_id]
            .get_map(SHARD_OWNERS)
            .insert(&malformed_owner.block_id.to_string(), true)
            .unwrap();
        assert_new_exact_shard_rejected(malformed_owner);

        let malformed_content = new_exact_shard_fixture(91_300);
        malformed_content.after[&malformed_content.home_id]
            .get_map(SHARD_CONTENT)
            .insert(&malformed_content.block_id.to_string(), "scalar content")
            .unwrap();
        assert_new_exact_shard_rejected(malformed_content);

        let malformed_member = new_exact_shard_fixture(91_400);
        malformed_member.after[&malformed_member.home_id]
            .get_map(SHARD_MEMBERS)
            .insert(&malformed_member.block_id.to_string(), true)
            .unwrap();
        assert_new_exact_shard_rejected(malformed_member);
    }

    #[test]
    fn mixed_new_and_existing_shards_do_not_skip_existing_declarations() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(91_500));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(91_501));
        let old_home = DocumentId::from_uuid(Uuid::from_u128(91_502));
        let old_page = PageId::from_uuid(Uuid::from_u128(91_503));
        let old_block = BlockId::from_uuid(Uuid::from_u128(91_504));
        let new_home = DocumentId::from_uuid(Uuid::from_u128(91_505));
        let new_page = PageId::from_uuid(Uuid::from_u128(91_506));
        let new_block = BlockId::from_uuid(Uuid::from_u128(91_507));
        let mut engine =
            ShardedHotEngine::new(workspace, LineageDigest::of(b"mixed-fast-path"), catalog_id);
        let genesis = engine
            .prepare_transaction(
                test_author(91_510, 1),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: old_page,
                        home_document_id: old_home,
                        path: ManagedPath::parse("pages/Existing.md").unwrap(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: old_block,
                            home_document_id: old_home,
                        },
                        page_id: old_page,
                        parent: None,
                        order: "a".into(),
                        content: "original".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(genesis))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));

        let before_catalog = engine.clone_visible_document(catalog_id, 2).unwrap();
        let before_old = engine.clone_visible_document(old_home, 2).unwrap();
        let after_catalog = clone_doc(&before_catalog, 2).unwrap();
        insert_page_state(
            &after_catalog,
            new_page,
            &live_page(new_home, "pages/New.md"),
        )
        .unwrap();
        let after_old = clone_doc(&before_old, 2).unwrap();
        block_text(&after_old, old_block)
            .unwrap()
            .update("edited", UpdateOptions::default())
            .unwrap();
        let after_new = LoroDoc::new();
        after_new.set_peer_id(2).unwrap();
        after_new
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, new_page.to_string())
            .unwrap();
        after_new
            .get_map(SHARD_OWNERS)
            .insert(&new_block.to_string(), new_page.to_string())
            .unwrap();
        after_new
            .get_map(SHARD_CONTENT)
            .insert_container(&new_block.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "new")
            .unwrap();
        insert_membership(
            &after_new,
            new_block,
            &MembershipClaim::new(new_home, None, "a").unwrap(),
        )
        .unwrap();
        let before = BTreeMap::from([
            (catalog_id, before_catalog),
            (old_home, before_old),
            (new_home, LoroDoc::new()),
        ]);
        let after = BTreeMap::from([
            (catalog_id, after_catalog),
            (old_home, after_old),
            (new_home, after_new),
        ]);
        let mut effect = derive_effect(catalog_id, &before, &after).unwrap();
        let mut blocks = effect.blocks().to_vec();
        blocks
            .iter_mut()
            .find(|delta| delta.home_document_id == old_home)
            .unwrap()
            .after
            .as_mut()
            .unwrap()
            .content = "incorrect existing declaration".into();
        effect = SemanticEffect::new(
            effect.pages().to_vec(),
            blocks,
            effect.memberships().to_vec(),
        )
        .unwrap();
        let frontier = FrontierV2::new(vec![
            dependencies_for(&engine, catalog_id, 2),
            dependencies_for(&engine, old_home, 2),
        ])
        .unwrap();
        let batch = validated_transition_with_effect(
            &engine,
            test_author(91_511, 2),
            &before,
            &after,
            frontier,
            effect,
        );
        assert!(matches!(
            engine.stage_ready(batch).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::SemanticEffectMismatch,
                ..
            }
        ));
        assert_eq!(
            engine.materialize_page(old_page).unwrap().blocks[0].content,
            "original"
        );
        assert!(matches!(
            engine.materialize_page(new_page),
            Err(EngineError::PageNotFound(_))
        ));
    }

    #[test]
    fn crdt_witness_decode_rejects_duplicate_unsorted_and_noncanonical_heads() {
        let batch_id = BatchId::from_uuid(Uuid::from_u128(45));
        let document_id = DocumentId::from_uuid(Uuid::from_u128(46));
        let head_a = BatchId::from_uuid(Uuid::from_u128(47));
        let head_b = BatchId::from_uuid(Uuid::from_u128(48));
        for dependency_heads in [vec![head_a, head_a], vec![head_b, head_a]] {
            let bytes = postcard::to_allocvec(&CrdtUpdatePayload {
                schema_version: CRDT_UPDATE_PAYLOAD_SCHEMA_VERSION,
                batch_id,
                document_id,
                dependency_heads,
                batch_dependency_heads: vec![head_a],
                causal_state_digest: None,
                raw_update: vec![1],
            })
            .unwrap();
            assert!(matches!(
                decode_crdt_update_payload(batch_id, document_id, &bytes),
                Err(EngineError::InvalidCrdt(_))
            ));
        }

        let mut canonical = encode_crdt_update_payload(
            batch_id,
            document_id,
            vec![head_a],
            vec![head_a],
            None,
            vec![1],
        )
        .unwrap();
        canonical.push(0);
        assert!(matches!(
            decode_crdt_update_payload(batch_id, document_id, &canonical),
            Err(EngineError::InvalidCrdt(_))
        ));

        let future = postcard::to_allocvec(&CrdtUpdatePayload {
            schema_version: CRDT_UPDATE_PAYLOAD_SCHEMA_VERSION + 1,
            batch_id,
            document_id,
            dependency_heads: vec![head_a],
            batch_dependency_heads: vec![head_a],
            causal_state_digest: None,
            raw_update: vec![1],
        })
        .unwrap();
        assert!(matches!(
            decode_crdt_update_payload(batch_id, document_id, &future),
            Err(EngineError::InvalidCrdt(_))
        ));
    }

    #[test]
    fn malformed_and_referentially_incomplete_replacements_reject_atomically() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(2));
        let home_id = DocumentId::from_uuid(Uuid::from_u128(20));
        let page_id = PageId::from_uuid(Uuid::from_u128(10));
        let block_id = BlockId::from_uuid(Uuid::from_u128(30));
        let mut engine =
            ShardedHotEngine::new(workspace, LineageDigest::of(b"lineage"), catalog_id);

        let catalog = LoroDoc::new();
        catalog.set_peer_id(50).unwrap();
        insert_page_state(
            &catalog,
            page_id,
            &PageState::Live {
                path: ManagedPath::parse("pages/A.md").unwrap(),
                home_document_id: home_id,
            },
        )
        .unwrap();
        let shard = LoroDoc::new();
        shard.set_peer_id(50).unwrap();
        shard
            .get_map(SHARD_OWNERS)
            .insert(&block_id.to_string(), page_id.to_string())
            .unwrap();
        shard
            .get_map(SHARD_CONTENT)
            .insert_container(&block_id.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "coherent content without page identity")
            .unwrap();

        let before = BTreeMap::from([(catalog_id, LoroDoc::new()), (home_id, LoroDoc::new())]);
        let after = BTreeMap::from([(catalog_id, catalog), (home_id, shard)]);
        let batch = validated_transition(
            &engine,
            test_author(50, 50),
            &before,
            &after,
            FrontierV2::new(Vec::new()).unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(batch).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::MalformedDocument { .. },
                ..
            }
        ));
        assert!(engine.canonical_snapshot().unwrap().pages.is_empty());
        assert!(engine.visible_documents.is_empty());

        let catalog_only = LoroDoc::new();
        catalog_only.set_peer_id(51).unwrap();
        insert_page_state(
            &catalog_only,
            page_id,
            &PageState::Live {
                path: ManagedPath::parse("pages/Catalog Only.md").unwrap(),
                home_document_id: home_id,
            },
        )
        .unwrap();
        let catalog_only = validated_transition(
            &engine,
            test_author(51, 51),
            &BTreeMap::from([(catalog_id, LoroDoc::new())]),
            &BTreeMap::from([(catalog_id, catalog_only)]),
            FrontierV2::new(Vec::new()).unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(catalog_only).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::MalformedDocument { .. },
                ..
            }
        ));
        assert!(engine.visible_documents.is_empty());

        let orphan_shard = LoroDoc::new();
        orphan_shard.set_peer_id(52).unwrap();
        orphan_shard
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, page_id.to_string())
            .unwrap();
        let orphan_shard = validated_transition(
            &engine,
            test_author(52, 52),
            &BTreeMap::from([(home_id, LoroDoc::new())]),
            &BTreeMap::from([(home_id, orphan_shard)]),
            FrontierV2::new(Vec::new()).unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(orphan_shard).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::MalformedDocument { .. },
                ..
            }
        ));
        assert!(engine.visible_documents.is_empty());

        let missing_home_id = DocumentId::from_uuid(Uuid::from_u128(21));
        let referenced_block_id = BlockId::from_uuid(Uuid::from_u128(31));
        let catalog = LoroDoc::new();
        catalog.set_peer_id(53).unwrap();
        insert_page_state(
            &catalog,
            page_id,
            &PageState::Live {
                path: ManagedPath::parse("pages/Missing Home.md").unwrap(),
                home_document_id: home_id,
            },
        )
        .unwrap();
        let shard = LoroDoc::new();
        shard.set_peer_id(53).unwrap();
        shard
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, page_id.to_string())
            .unwrap();
        insert_membership(
            &shard,
            referenced_block_id,
            &MembershipClaim::new(missing_home_id, None, "a").unwrap(),
        )
        .unwrap();
        let missing_home = validated_transition(
            &engine,
            test_author(53, 53),
            &BTreeMap::from([(catalog_id, LoroDoc::new()), (home_id, LoroDoc::new())]),
            &BTreeMap::from([(catalog_id, catalog), (home_id, shard)]),
            FrontierV2::new(Vec::new()).unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(missing_home).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::MalformedDocument { .. },
                ..
            }
        ));
        assert!(engine.visible_documents.is_empty());

        // None of the ordinary validation failures above may leave provisional
        // immutable-home evidence behind. The same identities remain valid for
        // a later coherent batch.
        let coherent = engine
            .prepare_transaction(
                test_author(54, 54),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id,
                        home_document_id: home_id,
                        path: ManagedPath::parse("pages/Coherent.md").unwrap(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id,
                            home_document_id: home_id,
                        },
                        page_id,
                        parent: None,
                        order: "a".into(),
                        content: "accepted after ordinary rejections".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(coherent))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        assert!(engine.fatal_evidence().is_none());
        assert_eq!(
            engine.materialize_page(page_id).unwrap().blocks[0].content,
            "accepted after ordinary rejections"
        );
    }

    #[test]
    fn raw_catalog_cannot_smuggle_shard_state_past_the_semantic_effect() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(61));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(62));
        let page_id = PageId::from_uuid(Uuid::from_u128(63));
        let block_id = BlockId::from_uuid(Uuid::from_u128(64));
        let mut engine =
            ShardedHotEngine::new(workspace, LineageDigest::of(b"disjoint-roles"), catalog_id);

        let catalog = LoroDoc::new();
        catalog.set_peer_id(65).unwrap();
        insert_page_state(
            &catalog,
            page_id,
            &PageState::Live {
                path: ManagedPath::parse("pages/Aliased.md").unwrap(),
                home_document_id: catalog_id,
            },
        )
        .unwrap();
        catalog
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, page_id.to_string())
            .unwrap();
        catalog
            .get_map(SHARD_OWNERS)
            .insert(&block_id.to_string(), page_id.to_string())
            .unwrap();
        catalog
            .get_map(SHARD_CONTENT)
            .insert_container(&block_id.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "semantically omitted content")
            .unwrap();
        insert_membership(
            &catalog,
            block_id,
            &MembershipClaim::new(catalog_id, None, "a").unwrap(),
        )
        .unwrap();

        let before = BTreeMap::from([(catalog_id, LoroDoc::new())]);
        let after = BTreeMap::from([(catalog_id, catalog)]);
        let declared_page_only_effect = SemanticEffect::new(
            vec![PageDelta {
                page_id,
                before: None,
                after: Some(PageState::Live {
                    path: ManagedPath::parse("pages/Aliased.md").unwrap(),
                    home_document_id: catalog_id,
                }),
            }],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let batch = validated_transition_with_effect(
            &engine,
            test_author(65, 65),
            &before,
            &after,
            FrontierV2::new(Vec::new()).unwrap(),
            declared_page_only_effect,
        );
        let outcome = engine.stage_ready(batch);
        assert!(
            matches!(
                outcome.disposition(),
                BatchDisposition::Rejected {
                    error: EngineError::MalformedDocument { .. },
                    ..
                }
            ),
            "unexpected aliased-role outcome: {outcome:?}"
        );
        assert!(engine.canonical_snapshot().unwrap().pages.is_empty());
        assert!(engine.visible_documents.is_empty());
    }

    #[test]
    fn accepted_shard_page_identity_cannot_change() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(101));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(102));
        let home_id = DocumentId::from_uuid(Uuid::from_u128(120));
        let page_a = PageId::from_uuid(Uuid::from_u128(110));
        let page_b = PageId::from_uuid(Uuid::from_u128(111));
        let mut engine =
            ShardedHotEngine::new(workspace, LineageDigest::of(b"identity"), catalog_id);
        let genesis = engine
            .prepare_transaction(
                test_author(200, 200),
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: page_a,
                    home_document_id: home_id,
                    path: ManagedPath::parse("pages/A.md").unwrap(),
                }])
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(genesis))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));

        let before_doc = engine.clone_visible_document(home_id, 201).unwrap();
        let changed = clone_doc(&before_doc, 201).unwrap();
        changed
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, page_b.to_string())
            .unwrap();
        let before = BTreeMap::from([(home_id, before_doc)]);
        let after = BTreeMap::from([(home_id, changed)]);
        let dependencies = DocumentDependencies::new(
            home_id,
            canonical_peer_counters(&before[&home_id].oplog_vv()).unwrap(),
            engine
                .document_dependency_heads(home_id, false)
                .unwrap()
                .into_iter()
                .collect(),
        )
        .unwrap();
        let batch = validated_transition_with_effect(
            &engine,
            test_author(201, 201),
            &before,
            &after,
            FrontierV2::new(vec![dependencies]).unwrap(),
            SemanticEffect::new(Vec::new(), Vec::new(), Vec::new()).unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(batch).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::ShardPageIdentityChanged { .. }
                    | EngineError::MalformedDocument { .. },
                ..
            }
        ));
        assert_eq!(engine.materialize_page(page_a).unwrap().page_id, page_a);
    }

    #[test]
    fn raw_block_removal_rejects_before_merge_in_both_delivery_orders() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(301));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(302));
        let home_id = DocumentId::from_uuid(Uuid::from_u128(320));
        let page_id = PageId::from_uuid(Uuid::from_u128(310));
        let block_id = BlockId::from_uuid(Uuid::from_u128(330));
        let mut engine =
            ShardedHotEngine::new(workspace, LineageDigest::of(b"merged-residue"), catalog_id);
        let genesis = engine
            .prepare_transaction(
                test_author(400, 400),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id,
                        home_document_id: home_id,
                        path: ManagedPath::parse("pages/Merge.md").unwrap(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id,
                            home_document_id: home_id,
                        },
                        page_id,
                        parent: None,
                        order: "a".into(),
                        content: "baseline".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        let genesis = ValidatedBatch::new(genesis);
        assert!(matches!(
            engine.stage_ready(genesis.clone()).disposition(),
            BatchDisposition::Accepted { .. }
        ));

        let dependencies = DocumentDependencies::new(
            home_id,
            canonical_peer_counters(
                &engine
                    .clone_visible_document(home_id, 401)
                    .unwrap()
                    .oplog_vv(),
            )
            .unwrap(),
            engine
                .document_dependency_heads(home_id, false)
                .unwrap()
                .into_iter()
                .collect(),
        )
        .unwrap();
        let frontier = FrontierV2::new(vec![dependencies]).unwrap();

        let removed_before = engine.clone_visible_document(home_id, 401).unwrap();
        let removed_after = clone_doc(&removed_before, 401).unwrap();
        removed_after
            .get_map(SHARD_OWNERS)
            .delete(&block_id.to_string())
            .unwrap();
        removed_after
            .get_map(SHARD_CONTENT)
            .delete(&block_id.to_string())
            .unwrap();
        removed_after
            .get_map(SHARD_MEMBERS)
            .delete(&block_id.to_string())
            .unwrap();
        let before_state = read_block_state(home_id, &removed_before, block_id)
            .unwrap()
            .unwrap();
        let removed_effect = raw_semantic_effect(
            Vec::new(),
            vec![BlockDelta {
                block_id,
                home_document_id: home_id,
                before: Some(before_state),
                after: None,
            }],
            vec![MembershipDelta {
                page_id,
                block_id,
                before: Some(MembershipClaim::new(home_id, None, "a").unwrap()),
                after: None,
            }],
        );
        let removed = validated_transition_with_payload(
            &engine,
            test_author(401, 401),
            &BTreeMap::from([(home_id, removed_before)]),
            &BTreeMap::from([(home_id, removed_after)]),
            frontier.clone(),
            removed_effect,
        );

        let edited_before = engine.clone_visible_document(home_id, 402).unwrap();
        let edited_after = clone_doc(&edited_before, 402).unwrap();
        edited_after
            .get_map(SHARD_CONTENT)
            .delete(&block_id.to_string())
            .unwrap();
        edited_after
            .get_map(SHARD_CONTENT)
            .insert_container(&block_id.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "concurrent replacement")
            .unwrap();
        let edited = validated_transition(
            &engine,
            test_author(402, 402),
            &BTreeMap::from([(home_id, edited_before)]),
            &BTreeMap::from([(home_id, edited_after)]),
            frontier,
        );

        let mut expected_accepted = None;
        let mut expected_rejected = None;
        let mut expected_snapshot = None;
        for removal_first in [true, false] {
            let mut receiver =
                ShardedHotEngine::new(workspace, LineageDigest::of(b"merged-residue"), catalog_id);
            assert!(matches!(
                receiver.stage_ready(genesis.clone()).disposition(),
                BatchDisposition::Accepted { .. }
            ));
            let ordered = if removal_first {
                [removed.clone(), edited.clone()]
            } else {
                [edited.clone(), removed.clone()]
            };
            let mut rejected = Vec::new();
            for batch in ordered {
                let batch_id = batch.manifest().batch_id();
                if matches!(
                    receiver.stage_ready(batch).disposition(),
                    BatchDisposition::Rejected { .. }
                ) {
                    rejected.push(batch_id);
                }
            }
            let accepted = receiver.status().accepted_batch_ids().unwrap();
            let snapshot = receiver.canonical_snapshot().unwrap();
            assert_eq!(
                receiver.materialize_page(page_id).unwrap().blocks[0].content,
                "concurrent replacement"
            );
            if let Some(expected) = &expected_accepted {
                assert_eq!(&accepted, expected);
                assert_eq!(&rejected, expected_rejected.as_ref().unwrap());
                assert_eq!(&snapshot, expected_snapshot.as_ref().unwrap());
            } else {
                expected_accepted = Some(accepted);
                expected_rejected = Some(rejected);
                expected_snapshot = Some(snapshot);
            }
        }
        assert_eq!(
            expected_accepted.unwrap(),
            vec![
                BatchId::from_uuid(Uuid::from_u128(400)),
                BatchId::from_uuid(Uuid::from_u128(402)),
            ]
        );
        assert_eq!(
            expected_rejected.unwrap(),
            vec![BatchId::from_uuid(Uuid::from_u128(401))]
        );
    }

    #[test]
    fn existing_block_id_cannot_be_duplicated_or_recreated_in_another_home() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(501));
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(502));
        let home_a = DocumentId::from_uuid(Uuid::from_u128(520));
        let home_b = DocumentId::from_uuid(Uuid::from_u128(521));
        let page_a = PageId::from_uuid(Uuid::from_u128(510));
        let page_b = PageId::from_uuid(Uuid::from_u128(511));
        let block_id = BlockId::from_uuid(Uuid::from_u128(530));
        let lineage = LineageDigest::of(b"immutable-block-home");
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog_id);
        let genesis = ValidatedBatch::new(
            engine
                .prepare_transaction(
                    test_author(600, 600),
                    &OperationTransaction::new(vec![
                        SemanticOperation::CreatePage {
                            page_id: page_a,
                            home_document_id: home_a,
                            path: ManagedPath::parse("pages/A.md").unwrap(),
                        },
                        SemanticOperation::CreatePage {
                            page_id: page_b,
                            home_document_id: home_b,
                            path: ManagedPath::parse("pages/B.md").unwrap(),
                        },
                        SemanticOperation::CreateBlock {
                            block: BlockLocation {
                                block_id,
                                home_document_id: home_a,
                            },
                            page_id: page_a,
                            parent: None,
                            order: "a".into(),
                            content: "immutable home A".into(),
                        },
                    ])
                    .unwrap(),
                )
                .unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(genesis.clone()).disposition(),
            BatchDisposition::Accepted { .. }
        ));

        let duplicate_before = engine.clone_visible_document(home_b, 601).unwrap();
        let duplicate_after = clone_doc(&duplicate_before, 601).unwrap();
        duplicate_after
            .get_map(SHARD_OWNERS)
            .insert(&block_id.to_string(), page_b.to_string())
            .unwrap();
        duplicate_after
            .get_map(SHARD_CONTENT)
            .ensure_mergeable_text(&block_id.to_string())
            .unwrap()
            .insert(0, "duplicate home B")
            .unwrap();
        insert_membership(
            &duplicate_after,
            block_id,
            &MembershipClaim::new(home_b, None, "b").unwrap(),
        )
        .unwrap();
        let duplicate_author = test_author(601, 601);
        let duplicate = validated_transition(
            &engine,
            duplicate_author,
            &BTreeMap::from([(home_b, duplicate_before)]),
            &BTreeMap::from([(home_b, duplicate_after)]),
            FrontierV2::new(vec![dependencies_for(&engine, home_b, 601)]).unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(duplicate).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::BlockAlreadyExists(found),
                ..
            } if found == block_id
        ));
        assert_eq!(engine.fatal_evidence(), None);

        for (home_document_id, page_id) in [(home_a, page_a), (home_b, page_b)] {
            assert!(matches!(
                engine.prepare_transaction(
                    test_author(603, 603),
                    &OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id,
                            home_document_id,
                        },
                        page_id,
                        parent: None,
                        order: "c".into(),
                        content: "author must consult global immutable-home evidence".into(),
                    }])
                    .unwrap(),
                ),
                Err(EngineError::BlockAlreadyExists(found)) if found == block_id
            ));
        }

        let same_batch_id = BlockId::from_uuid(Uuid::from_u128(531));
        let same_before_a = engine.clone_visible_document(home_a, 604).unwrap();
        let same_before_b = engine.clone_visible_document(home_b, 604).unwrap();
        let same_after_a = clone_doc(&same_before_a, 604).unwrap();
        let same_after_b = clone_doc(&same_before_b, 604).unwrap();
        for (document_id, page_id, document, order) in [
            (home_a, page_a, &same_after_a, "same-a"),
            (home_b, page_b, &same_after_b, "same-b"),
        ] {
            document
                .get_map(SHARD_OWNERS)
                .insert(&same_batch_id.to_string(), page_id.to_string())
                .unwrap();
            document
                .get_map(SHARD_CONTENT)
                .ensure_mergeable_text(&same_batch_id.to_string())
                .unwrap()
                .insert(0, order)
                .unwrap();
            insert_membership(
                document,
                same_batch_id,
                &MembershipClaim::new(document_id, None, order).unwrap(),
            )
            .unwrap();
        }
        let same_batch_duplicate = validated_transition(
            &engine,
            test_author(604, 604),
            &BTreeMap::from([(home_a, same_before_a), (home_b, same_before_b)]),
            &BTreeMap::from([(home_a, same_after_a), (home_b, same_after_b)]),
            FrontierV2::new(vec![
                dependencies_for(&engine, home_a, 604),
                dependencies_for(&engine, home_b, 604),
            ])
            .unwrap(),
        );
        assert!(matches!(
            engine.stage_ready(same_batch_duplicate).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::BlockAlreadyExists(found),
            } if found == same_batch_id
        ));
        assert_eq!(engine.fatal_evidence(), None);
        assert!(engine
            .recover_block_state(home_a, same_batch_id)
            .unwrap()
            .is_none());
        assert!(engine
            .recover_block_state(home_b, same_batch_id)
            .unwrap()
            .is_none());
        let accepted_after_rollback = engine
            .prepare_transaction(
                test_author(605, 605),
                &OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: same_batch_id,
                        home_document_id: home_a,
                    },
                    page_id: page_a,
                    parent: None,
                    order: "accepted-after-rollback".into(),
                    content: "no provisional identity evidence remained".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            engine
                .stage_ready(ValidatedBatch::new(accepted_after_rollback))
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));

        let mut relocation_engine = ShardedHotEngine::new(workspace, lineage, catalog_id);
        assert!(matches!(
            relocation_engine.stage_ready(genesis).disposition(),
            BatchDisposition::Accepted { .. }
        ));

        let before_a = relocation_engine
            .clone_visible_document(home_a, 602)
            .unwrap();
        let before_b = relocation_engine
            .clone_visible_document(home_b, 602)
            .unwrap();
        let after_a = clone_doc(&before_a, 602).unwrap();
        after_a
            .get_map(SHARD_OWNERS)
            .delete(&block_id.to_string())
            .unwrap();
        after_a
            .get_map(SHARD_CONTENT)
            .delete(&block_id.to_string())
            .unwrap();
        after_a
            .get_map(SHARD_MEMBERS)
            .delete(&block_id.to_string())
            .unwrap();
        let after_b = clone_doc(&before_b, 602).unwrap();
        after_b
            .get_map(SHARD_OWNERS)
            .insert(&block_id.to_string(), page_b.to_string())
            .unwrap();
        after_b
            .get_map(SHARD_CONTENT)
            .ensure_mergeable_text(&block_id.to_string())
            .unwrap()
            .insert(0, "recreated home B")
            .unwrap();
        insert_membership(
            &after_b,
            block_id,
            &MembershipClaim::new(home_b, None, "b").unwrap(),
        )
        .unwrap();
        let removed_state = read_block_state(home_a, &before_a, block_id)
            .unwrap()
            .unwrap();
        let recreated_state = read_block_state(home_b, &after_b, block_id)
            .unwrap()
            .unwrap();
        let relocation_effect = raw_semantic_effect(
            Vec::new(),
            vec![
                BlockDelta {
                    block_id,
                    home_document_id: home_a,
                    before: Some(removed_state),
                    after: None,
                },
                BlockDelta {
                    block_id,
                    home_document_id: home_b,
                    before: None,
                    after: Some(recreated_state),
                },
            ],
            vec![
                MembershipDelta {
                    page_id: page_a,
                    block_id,
                    before: Some(MembershipClaim::new(home_a, None, "a").unwrap()),
                    after: None,
                },
                MembershipDelta {
                    page_id: page_b,
                    block_id,
                    before: None,
                    after: Some(MembershipClaim::new(home_b, None, "b").unwrap()),
                },
            ],
        );
        let relocation = validated_transition_with_payload(
            &relocation_engine,
            test_author(602, 602),
            &BTreeMap::from([(home_a, before_a), (home_b, before_b)]),
            &BTreeMap::from([(home_a, after_a), (home_b, after_b)]),
            FrontierV2::new(vec![
                dependencies_for(&relocation_engine, home_a, 602),
                dependencies_for(&relocation_engine, home_b, 602),
            ])
            .unwrap(),
            relocation_effect,
        );
        assert!(matches!(
            relocation_engine.stage_ready(relocation).disposition(),
            BatchDisposition::Rejected {
                error: EngineError::Semantic(_),
                ..
            }
        ));
        assert_eq!(
            relocation_engine.materialize_page(page_a).unwrap().blocks[0].content,
            "immutable home A"
        );
        assert!(relocation_engine
            .materialize_page(page_b)
            .unwrap()
            .blocks
            .is_empty());
        assert!(relocation_engine
            .recover_block_state(home_b, block_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn compact_direct_parent_preserves_cross_document_atomic_ancestry() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(7_000));
        let catalog = DocumentId::from_uuid(Uuid::from_u128(7_001));
        let page_a = PageId::from_uuid(Uuid::from_u128(7_002));
        let page_b = PageId::from_uuid(Uuid::from_u128(7_003));
        let page_c = PageId::from_uuid(Uuid::from_u128(7_004));
        let home_a = DocumentId::from_uuid(Uuid::from_u128(7_005));
        let home_b = DocumentId::from_uuid(Uuid::from_u128(7_006));
        let home_c = DocumentId::from_uuid(Uuid::from_u128(7_007));
        let duplicate = BlockId::from_uuid(Uuid::from_u128(7_008));
        let support = BlockId::from_uuid(Uuid::from_u128(7_009));
        let mut engine =
            ShardedHotEngine::new(workspace, LineageDigest::of(b"cross-document"), catalog);
        let genesis = engine
            .prepare_transaction(
                test_author(7_010, 710),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: page_a,
                        home_document_id: home_a,
                        path: ManagedPath::parse("pages/A.md").unwrap(),
                    },
                    SemanticOperation::CreatePage {
                        page_id: page_b,
                        home_document_id: home_b,
                        path: ManagedPath::parse("pages/B.md").unwrap(),
                    },
                    SemanticOperation::CreatePage {
                        page_id: page_c,
                        home_document_id: home_c,
                        path: ManagedPath::parse("pages/C.md").unwrap(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        engine.stage_ready(ValidatedBatch::new(genesis));
        let ancestor = engine
            .prepare_transaction(
                test_author(7_011, 711),
                &OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: duplicate,
                        home_document_id: home_a,
                    },
                    page_id: page_a,
                    parent: None,
                    order: "a".into(),
                    content: "ancestor identity".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        engine.stage_ready(ValidatedBatch::new(ancestor));
        let atomic_parent = engine
            .prepare_transaction(
                test_author(7_012, 712),
                &OperationTransaction::new(vec![
                    SemanticOperation::EditBlockContent {
                        block: BlockLocation {
                            block_id: duplicate,
                            home_document_id: home_a,
                        },
                        content: "parent touched ancestor document".into(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: support,
                            home_document_id: home_b,
                        },
                        page_id: page_b,
                        parent: None,
                        order: "support".into(),
                        content: "parent also touched child document".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        engine.stage_ready(ValidatedBatch::new(atomic_parent));

        let authored_descendant = engine
            .prepare_transaction(
                test_author(7_014, 714),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: support,
                        home_document_id: home_b,
                    },
                    content: "legitimate cross-document descendant".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        assert!(authored_descendant
            .manifest()
            .dependency_frontier()
            .documents()
            .iter()
            .any(|entry| {
                entry.document_id() == home_b
                    && entry.direct_dependency_heads()
                        == [BatchId::from_uuid(Uuid::from_u128(7_012))]
            }));
        assert!(!authored_descendant
            .manifest()
            .dependency_frontier()
            .documents()
            .iter()
            .any(|entry| entry.document_id() == home_a));

        let before_c = engine.clone_visible_document(home_c, 713).unwrap();
        let after_c = clone_doc(&before_c, 713).unwrap();
        after_c
            .get_map(SHARD_OWNERS)
            .insert(&duplicate.to_string(), page_c.to_string())
            .unwrap();
        after_c
            .get_map(SHARD_CONTENT)
            .ensure_mergeable_text(&duplicate.to_string())
            .unwrap()
            .insert(0, "omitted cross-document ancestor")
            .unwrap();
        insert_membership(
            &after_c,
            duplicate,
            &MembershipClaim::new(home_c, None, "duplicate").unwrap(),
        )
        .unwrap();
        let malformed = validated_transition(
            &engine,
            test_author(7_013, 713),
            &BTreeMap::from([(home_c, before_c)]),
            &BTreeMap::from([(home_c, after_c)]),
            FrontierV2::new(vec![
                dependencies_for(&engine, home_b, 713),
                dependencies_for(&engine, home_c, 713),
            ])
            .unwrap(),
        );
        let malformed_outcome = engine.stage_ready(malformed).disposition();
        assert!(
            matches!(
                malformed_outcome,
                BatchDisposition::Rejected {
                    error: EngineError::BlockAlreadyExists(found),
                }
                if found == duplicate
            ),
            "unexpected malformed ancestry outcome: {malformed_outcome:?}"
        );
        assert_eq!(engine.fatal_evidence(), None);
    }

    #[test]
    fn sequential_same_page_chain_has_bounded_authorship_manifest_and_stage_work() {
        const BATCHES: usize = 192;
        const MAX_COMPACT_MANIFEST_BYTES: usize = 4_096;
        const MAX_HISTORY_INDEX_READS_PER_STAGE: usize = 100;
        const MAX_HISTORY_INDEX_WRITES_PER_STAGE: usize = 33;
        const MAX_HISTORY_RECORD_READS_PER_STAGE: usize = 1;

        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(8_000));
        let catalog = DocumentId::from_uuid(Uuid::from_u128(8_001));
        let page = PageId::from_uuid(Uuid::from_u128(8_002));
        let home = DocumentId::from_uuid(Uuid::from_u128(8_003));
        let block = BlockId::from_uuid(Uuid::from_u128(8_004));
        let lineage = LineageDigest::of(b"bounded-batch-history");
        let root =
            std::env::temp_dir().join(format!("tine-oplog-bounded-history-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("archive");
        let writer = ObjectStore::open(&archive_path, workspace).unwrap();
        let reader = ObjectStore::open(&archive_path, workspace).unwrap();
        let mut engine = ShardedHotEngine::with_archive_store(reader, lineage, catalog);
        let mut max_candidate_visits = 0;
        let mut max_status_lookups = 0;
        let mut late_manifest_sizes = Vec::new();
        let mut early_stage_costs = Vec::<[usize; 10]>::new();
        let mut late_stage_costs = Vec::<[usize; 10]>::new();
        let mut early_author_clone_costs = Vec::<[usize; 2]>::new();
        let mut late_author_clone_costs = Vec::<[usize; 2]>::new();

        for index in 0..BATCHES {
            let operation = if index == 0 {
                vec![
                    SemanticOperation::CreatePage {
                        page_id: page,
                        home_document_id: home,
                        path: ManagedPath::parse("pages/History.md").unwrap(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: block,
                            home_document_id: home,
                        },
                        page_id: page,
                        parent: None,
                        order: "a".into(),
                        content: "0".into(),
                    },
                ]
            } else {
                vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: block,
                        home_document_id: home,
                    },
                    content: index.to_string(),
                }]
            };
            let prepare_before = engine.instrumentation();
            let prepared = engine
                .prepare_transaction(
                    // A device keeps one stable Loro peer for its sequential
                    // edits. Peer cardinality is a separate legitimate
                    // frontier dimension and is not page-history growth.
                    test_author(8_100 + index as u128, 8_100),
                    &OperationTransaction::new(operation).unwrap(),
                )
                .unwrap();
            let prepare_after = engine.instrumentation();
            assert_eq!(
                prepare_after.prepare_transactions - prepare_before.prepare_transactions,
                1
            );
            assert!(
                prepare_after.prepare_document_head_visits
                    - prepare_before.prepare_document_head_visits
                    <= 2
            );
            let accepted_manifest_reads = prepare_after.store.accepted_manifest_reads
                - prepare_before.store.accepted_manifest_reads;
            assert!(
                accepted_manifest_reads <= 2,
                "ordinary authorship used {accepted_manifest_reads} archive reads at batch {index}"
            );
            assert!(
                prepare_after.store.dag_manifest_reads - prepare_before.store.dag_manifest_reads
                    <= 2,
                "ordinary authorship exceeded its bounded DAG-read ceiling at batch {index}"
            );
            let author_clone_cost = [
                prepare_after.author_snapshot_clones - prepare_before.author_snapshot_clones,
                prepare_after.author_snapshot_clone_ops - prepare_before.author_snapshot_clone_ops,
            ];
            if index < 16 {
                early_author_clone_costs.push(author_clone_cost);
            }
            if index >= BATCHES - 16 {
                late_author_clone_costs.push(author_clone_cost);
            }
            let manifest_size = prepared.manifest().encode().unwrap().len();
            assert!(
                manifest_size <= MAX_COMPACT_MANIFEST_BYTES,
                "manifest {index} is {manifest_size} bytes"
            );
            if index >= BATCHES - 16 {
                late_manifest_sizes.push(manifest_size);
            }
            writer.publish_prepared(&prepared).unwrap();
            let work_before = engine.history_work.get();
            let stage_before = engine.instrumentation();
            assert!(matches!(
                engine
                    .stage_archive_batch(prepared.manifest().batch_id())
                    .unwrap()
                    .disposition(),
                BatchDisposition::Accepted { .. }
            ));
            let work_after = engine.history_work.get();
            let stage_after = engine.instrumentation();
            assert_eq!(
                stage_after.store.directory_enumerations
                    - stage_before.store.directory_enumerations,
                0,
                "stage {index} enumerated an object-store directory"
            );
            assert!(
                stage_after.store.dag_manifest_reads - stage_before.store.dag_manifest_reads <= 2,
                "exact-current stage {index} exceeded its bounded DAG-read ceiling"
            );
            assert!(
                stage_after.store.history_index_reads - stage_before.store.history_index_reads
                    <= MAX_HISTORY_INDEX_READS_PER_STAGE,
                "stage {index} exceeded the authenticated point-lookup read bound: {} reads",
                stage_after.store.history_index_reads - stage_before.store.history_index_reads
            );
            assert!(
                stage_after.store.history_index_writes - stage_before.store.history_index_writes
                    <= MAX_HISTORY_INDEX_WRITES_PER_STAGE,
                "stage {index} exceeded the authenticated index write bound"
            );
            assert!(
                stage_after.store.history_record_reads - stage_before.store.history_record_reads
                    <= MAX_HISTORY_RECORD_READS_PER_STAGE,
                "stage {index} exceeded the terminal-record read bound"
            );
            assert!(
                stage_after.store.history_decodes - stage_before.store.history_decodes
                    <= MAX_HISTORY_RECORD_READS_PER_STAGE,
                "stage {index} exceeded the terminal-record decode bound"
            );
            max_candidate_visits = max_candidate_visits.max(
                work_after
                    .drain_candidate_visits
                    .saturating_sub(work_before.drain_candidate_visits),
            );
            max_status_lookups = max_status_lookups.max(
                work_after
                    .dependency_status_lookups
                    .saturating_sub(work_before.dependency_status_lookups),
            );
            let stage_cost = [
                stage_after.store.history_index_reads - stage_before.store.history_index_reads,
                stage_after.store.history_index_writes - stage_before.store.history_index_writes,
                stage_after.store.history_record_reads - stage_before.store.history_record_reads,
                stage_after.store.history_decodes - stage_before.store.history_decodes,
                stage_after.store.dag_manifest_reads - stage_before.store.dag_manifest_reads,
                work_after
                    .drain_candidate_visits
                    .saturating_sub(work_before.drain_candidate_visits),
                work_after
                    .dependency_status_lookups
                    .saturating_sub(work_before.dependency_status_lookups),
                stage_after.stage_snapshot_clones - stage_before.stage_snapshot_clones,
                stage_after.stage_snapshot_clone_ops - stage_before.stage_snapshot_clone_ops,
                stage_after.stage_structural_buffer_reuses
                    - stage_before.stage_structural_buffer_reuses,
            ];
            if index < 16 {
                early_stage_costs.push(stage_cost);
            }
            if index >= BATCHES - 16 {
                late_stage_costs.push(stage_cost);
            }
        }

        let component_max = |costs: &[[usize; 10]]| {
            costs.iter().fold([0; 10], |mut maxima, cost| {
                for (maximum, value) in maxima.iter_mut().zip(cost) {
                    *maximum = (*maximum).max(*value);
                }
                maxima
            })
        };
        let early_max = component_max(&early_stage_costs);
        let late_max = component_max(&late_stage_costs);
        assert!(
            late_max
                .iter()
                .zip(early_max)
                .all(|(late, early)| *late <= early),
            "late point/DAG/status work grew with page age: early={early_max:?}, late={late_max:?}"
        );
        eprintln!("compact_history_stage_cost early_max={early_max:?} late_max={late_max:?}");
        let component_max_2 = |costs: &[[usize; 2]]| {
            costs.iter().fold([0; 2], |mut maxima, cost| {
                for (maximum, value) in maxima.iter_mut().zip(cost) {
                    *maximum = (*maximum).max(*value);
                }
                maxima
            })
        };
        let early_author_max = component_max_2(&early_author_clone_costs);
        let late_author_max = component_max_2(&late_author_clone_costs);
        assert!(
            late_author_max
                .iter()
                .zip(early_author_max)
                .all(|(late, early)| *late <= early),
            "late author snapshot/history work grew with page age: early={early_author_max:?}, late={late_author_max:?}"
        );
        assert!(
            late_manifest_sizes.iter().copied().max().unwrap()
                - late_manifest_sizes.iter().copied().min().unwrap()
                <= 64,
            "late compact manifest sizes grew with page age: {late_manifest_sizes:?}"
        );

        assert!(
            engine.archive_fingerprints.len() <= 4,
            "finalized fingerprints remained hot: {}",
            engine.archive_fingerprints.len()
        );
        assert!(
            engine.persisted_staged.len() <= 4,
            "persisted batch IDs remained hot: {}",
            engine.persisted_staged.len()
        );
        assert!(
            engine.statuses.len() <= 4,
            "finalized statuses remained hot: {}",
            engine.statuses.len()
        );
        assert!(
            engine
                .visible_document_heads
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
                <= 8,
            "document direct-head frontier exceeded its compact bound"
        );
        assert!(
            engine.accepted_frontier.is_empty(),
            "store-backed accepted frontier leaked into graph-wide heap state"
        );
        assert!(
            engine.accepted_sequence.is_empty(),
            "store-backed accepted sequence index leaked into graph-wide heap state"
        );
        assert_eq!(engine.exact_frontier().unwrap().documents().len(), 2);
        assert!(
            max_candidate_visits <= 2,
            "one stage revisited {max_candidate_visits} active candidates"
        );
        assert!(
            max_status_lookups <= 6,
            "one stage performed {max_status_lookups} historical status lookups"
        );
        let instrumentation = engine.instrumentation();
        assert!(instrumentation.external_flushes > 0);
        assert!(instrumentation.external_history_page_reads > 0);
        assert_eq!(instrumentation.scratch_syncs, 0);
        assert_eq!(instrumentation.batch_status_hot_entries, 0);
        assert_eq!(instrumentation.ready_payload_hot_entries, 0);
        assert!(instrumentation.document_hot_entries <= 65);
        engine
            .scratch
            .as_ref()
            .expect("store-backed engine scratch")
            .truncate_pages_for_test();
        let corrupt_left = engine.status();
        let corrupt_right = engine.status();
        assert!(matches!(
            corrupt_left.try_eq(&corrupt_right),
            Err(EngineError::Archive(_))
        ));
        assert!(matches!(
            engine.status().accepted_batch_ids(),
            Err(EngineError::Archive(_))
        ));
        engine.visible_documents.remove(&home);
        let tampered_materialization = engine.materialize_page(page);
        assert!(
            matches!(tampered_materialization, Err(EngineError::Archive(_))),
            "unexpected tampered materialization: {tampered_materialization:?}"
        );
        drop(engine);
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn late_external_publication_failure_preserves_all_engine_visible_state_roots() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(88_000));
        let lineage = LineageDigest::of(b"late-external-publication");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(88_001));
        let page_a = PageId::from_uuid(Uuid::from_u128(88_002));
        let home_a = DocumentId::from_uuid(Uuid::from_u128(88_003));
        let block_a = BlockId::from_uuid(Uuid::from_u128(88_004));
        let root = std::env::temp_dir().join(format!("tine-oplog-late-publish-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("archive");
        let writer = ObjectStore::open(&archive_path, workspace).unwrap();
        let reader = ObjectStore::open(&archive_path, workspace).unwrap();
        let mut engine = ShardedHotEngine::with_archive_store(reader, lineage, catalog);

        let baseline = engine
            .prepare_transaction(
                test_author(88_100, 1),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: page_a,
                        home_document_id: home_a,
                        path: ManagedPath::parse("pages/Baseline.md").unwrap(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: block_a,
                            home_document_id: home_a,
                        },
                        page_id: page_a,
                        parent: None,
                        order: "a".into(),
                        content: "baseline".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        writer.publish_prepared(&baseline).unwrap();
        assert!(matches!(
            engine
                .stage_archive_batch(baseline.manifest().batch_id())
                .unwrap()
                .disposition,
            BatchDisposition::Accepted { .. }
        ));
        let prior_snapshot = engine.canonical_snapshot().unwrap();
        let prior_accepted = engine.status().accepted_batch_ids().unwrap();
        let prior_roots = engine.scratch_roots.clone();
        let prior_claim_root = engine.block_claim_root;
        let prior_fatal_handle = engine.fatal_handle;
        let prior_fatal_evidence = engine.fatal_evidence.clone();

        let mut operations = Vec::new();
        for offset in 0..2_u128 {
            let page_id = PageId::from_uuid(Uuid::from_u128(88_010 + offset));
            let home_document_id = DocumentId::from_uuid(Uuid::from_u128(88_020 + offset));
            let block_id = BlockId::from_uuid(Uuid::from_u128(88_030 + offset));
            operations.push(SemanticOperation::CreatePage {
                page_id,
                home_document_id,
                path: ManagedPath::parse(format!("pages/Rejected {offset}.md")).unwrap(),
            });
            operations.push(SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id,
                },
                page_id,
                parent: None,
                order: "a".into(),
                content: format!("rejected {offset}"),
            });
        }
        let rejected = engine
            .prepare_transaction(
                test_author(88_101, 2),
                &OperationTransaction::new(operations).unwrap(),
            )
            .unwrap();
        writer.publish_prepared(&rejected).unwrap();
        engine.external_publication_failure_index = Some(1);
        let outcome = engine
            .stage_archive_batch(rejected.manifest().batch_id())
            .unwrap();
        assert!(matches!(
            outcome.disposition,
            BatchDisposition::Rejected {
                error: EngineError::Archive(_),
            }
        ));

        assert_eq!(engine.canonical_snapshot().unwrap(), prior_snapshot);
        assert_eq!(
            engine.status().accepted_batch_ids().unwrap(),
            prior_accepted
        );
        assert_eq!(engine.block_claim_root, prior_claim_root);
        assert_eq!(engine.fatal_handle, prior_fatal_handle);
        assert_eq!(engine.fatal_evidence, prior_fatal_evidence);
        assert_eq!(engine.workspace_status(), WorkspaceStatus::Operational);
        assert!(engine.history_failure.is_none());
        assert_eq!(
            engine.scratch_roots.external_document_current_root,
            prior_roots.external_document_current_root
        );
        assert_eq!(
            engine.scratch_roots.external_document_state_root,
            prior_roots.external_document_state_root
        );
        assert_eq!(
            engine.scratch_roots.blob_dedup_root,
            prior_roots.blob_dedup_root
        );
        assert_eq!(
            engine.scratch_roots.conflict_root,
            prior_roots.conflict_root
        );
        assert_eq!(engine.scratch_roots.causal_root, prior_roots.causal_root);
        assert_eq!(
            engine.scratch_roots.causal_dot_root,
            prior_roots.causal_dot_root
        );
        assert_eq!(
            engine.scratch_roots.causal_peer_root,
            prior_roots.causal_peer_root
        );
        assert!(engine
            .batch_statuses()
            .unwrap()
            .iter()
            .any(|(batch_id, status)| {
                *batch_id == rejected.manifest().batch_id()
                    && matches!(status, BatchDisposition::Rejected { .. })
            }));
        drop(engine);
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod replay_benchmark {
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use uuid::Uuid;

    use super::*;

    const REPLAY_CHILD_ARCHIVE_ENV: &str = "TINE_OPLOG_REPLAY_CHILD_ARCHIVE";

    struct FixtureCleanup(PathBuf);

    impl Drop for FixtureCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Reproducible P1 sealed-operation replay gate. The fixture contains
    /// exactly 1,000,000 blocks across 10,000 pages; page creation operations
    /// are additional. Four hundred pages are sealed per atomic batch. Every
    /// batch is authored through the evolving store-backed engine, so catalog
    /// updates carry a nonempty causal FrontierV2 after the first batch.
    ///
    /// Run with:
    /// `cargo test --release -p tine-core oplog_hot_replay_million -- --ignored --nocapture`
    #[test]
    #[ignore = "one-million-operation performance gate"]
    fn oplog_hot_replay_million() {
        const BLOCK_TARGET: usize = 1_000_000;
        const BLOCKS_PER_PAGE: usize = 100;
        const PAGES_PER_BATCH: usize = 400;
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let lineage = LineageDigest::of(b"p1-hot-replay");
        let catalog = DocumentId::from_uuid(Uuid::from_u128(2));
        let pages = BLOCK_TARGET.div_ceil(BLOCKS_PER_PAGE);
        if let Some(archive_root) = std::env::var_os(REPLAY_CHILD_ARCHIVE_ENV) {
            replay_million_child(
                PathBuf::from(archive_root),
                workspace,
                lineage,
                catalog,
                pages,
            );
            return;
        }
        let fixture_root =
            std::env::temp_dir().join(format!("tine-oplog-hot-replay-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&fixture_root).unwrap();
        let _cleanup = FixtureCleanup(fixture_root.clone());
        let archive_root = fixture_root.join("archive");
        let writer = ObjectStore::open(&archive_root, workspace).unwrap();
        let author_store = ObjectStore::open(&archive_root, workspace).unwrap();
        let mut evolving = ShardedHotEngine::with_archive_store(author_store, lineage, catalog);
        let mut blocks_built = 0usize;

        for batch_start in (0..pages).step_by(PAGES_PER_BATCH) {
            let batch_end = (batch_start + PAGES_PER_BATCH).min(pages);
            let batch_index = batch_start / PAGES_PER_BATCH;
            let peer = CrdtPeerId::from_u64(batch_index as u64 + 10);
            let author = AuthorBatch {
                batch_id: BatchId::from_uuid(Uuid::from_u128(4_000_000 + batch_index as u128)),
                author_device_id: DeviceId::from_uuid(Uuid::from_u128(
                    5_000_000 + batch_index as u128,
                )),
                author_session_id: SessionId::from_uuid(Uuid::from_u128(
                    6_000_000 + batch_index as u128,
                )),
                crdt_peer_id: peer,
            };
            let mut operations =
                Vec::with_capacity((batch_end - batch_start) * (BLOCKS_PER_PAGE + 1));
            for page_index in batch_start..batch_end {
                let page_id = PageId::from_uuid(Uuid::from_u128(1_000_000 + page_index as u128));
                let home = DocumentId::from_uuid(Uuid::from_u128(2_000_000 + page_index as u128));
                operations.push(SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    path: ManagedPath::parse(format!("pages/Replay {page_index:08}.md")).unwrap(),
                });
                for order in 0..BLOCKS_PER_PAGE {
                    if blocks_built >= BLOCK_TARGET {
                        break;
                    }
                    let block_id =
                        BlockId::from_uuid(Uuid::from_u128(3_000_000 + blocks_built as u128));
                    blocks_built += 1;
                    operations.push(SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id,
                            home_document_id: home,
                        },
                        page_id,
                        parent: None,
                        order: format!("{order:08x}"),
                        content: format!("sealed replay block {blocks_built:08}"),
                    });
                }
            }
            let prepared = evolving
                .prepare_transaction(author, &OperationTransaction::new(operations).unwrap())
                .unwrap();
            writer.publish_prepared(&prepared).unwrap();
            assert!(matches!(
                evolving
                    .stage_archive_batch(author.batch_id)
                    .unwrap()
                    .disposition(),
                BatchDisposition::Accepted { .. }
            ));
        }
        assert_eq!(blocks_built, BLOCK_TARGET);
        assert_eq!(pages, 10_000);
        drop(evolving);
        drop(writer);

        let status = Command::new(std::env::current_exe().unwrap())
            .arg("oplog_hot_replay_million")
            .arg("--ignored")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(REPLAY_CHILD_ARCHIVE_ENV, &archive_root)
            .status()
            .unwrap();
        assert!(status.success(), "isolated replay child failed: {status}");
    }

    fn replay_million_child(
        archive_root: PathBuf,
        workspace: WorkspaceId,
        lineage: LineageDigest,
        catalog: DocumentId,
        pages: usize,
    ) {
        const BLOCKS_PER_PAGE: usize = 100;
        const PAGES_PER_BATCH: usize = 400;
        // This is an authenticated offline replay/rebuild ceiling, not the
        // normal SQLite-backed startup target. The measured optimized baseline
        // is about 38 seconds on the reference host; 45 seconds retains a
        // regression margin without conflating rebuild work with app startup.
        const MAX_COLD_REPLAY_SECONDS: f64 = 45.0;
        let mut inspection_elapsed = Duration::ZERO;
        let mut validation_elapsed = Duration::ZERO;
        let replay_store = ObjectStore::open(&archive_root, workspace).unwrap();
        let mut replay = ShardedHotEngine::with_archive_store(replay_store, lineage, catalog);
        replay.ensure_history_store().unwrap();
        reset_owned_semantic_snapshot_entries();
        // Store construction performs fail-closed namespace preflight, not
        // operation replay. It remains in this fresh process's conservative
        // VmHWM evidence but outside the established 25-batch replay timer.
        let started = Instant::now();
        for batch_index in 0..pages.div_ceil(PAGES_PER_BATCH) {
            let batch_id = BatchId::from_uuid(Uuid::from_u128(4_000_000 + batch_index as u128));
            let inspection_started = Instant::now();
            let batch = match replay
                .archive_store
                .as_ref()
                .expect("replay store exists")
                .inspect_batch(batch_id)
                .unwrap()
            {
                BatchInspection::Ready(batch) => batch,
                other => panic!("replay batch is not ready: {other:?}"),
            };
            inspection_elapsed += inspection_started.elapsed();
            let validation_started = Instant::now();
            assert!(matches!(
                replay.stage_ready_internal(batch, true).disposition(),
                BatchDisposition::Accepted { .. }
            ));
            replay.prune_persisted_archive_cache();
            validation_elapsed += validation_started.elapsed();
        }
        let elapsed = started.elapsed();
        // This child performs no fixture authorship. Linux maintains VmHWM in
        // the kernel over the entire fresh process lifetime, including store
        // open, inspection, frontier reconstruction, semantic derivation,
        // replacement staging, and pruning. A transient replay allocation
        // therefore cannot evade the measurement between userspace samples.
        let replay_peak_rss_kib = linux_peak_rss_kib();
        assert_eq!(
            replay.status().accepted_batch_ids().unwrap().len(),
            pages.div_ceil(PAGES_PER_BATCH)
        );
        let instrumentation = replay.instrumentation();
        assert_eq!(
            instrumentation.block_claim_hot_entries, 0,
            "store-backed replay retained per-block claim evidence in hot memory"
        );
        assert_eq!(
            owned_semantic_snapshot_entries(),
            0,
            "one-million new-shard replay constructed owned block or membership snapshots"
        );
        eprintln!(
            "oplog_hot_replay blocks=1000000 page_operations={pages} batches={} elapsed_ms={:.3} inspection_ms={:.3} validation_ms={:.3} replay_peak_rss_kib={} claim_validation_ms={:.3} claim_lookup_ms={:.3} claim_encode_ms={:.3} claim_insert_ms={:.3} claim_index_reads={} claim_index_writes={} claim_index_syncs={} claim_hot_entries={} owned_semantic_snapshot_entries={}",
            replay.status().accepted_batch_ids().unwrap().len(),
            elapsed.as_secs_f64() * 1_000.0,
            inspection_elapsed.as_secs_f64() * 1_000.0,
            validation_elapsed.as_secs_f64() * 1_000.0,
            replay_peak_rss_kib.map_or_else(|| "unsupported".into(), |value| value.to_string()),
            instrumentation.block_claim_validation_nanos as f64 / 1_000_000.0,
            instrumentation.block_claim_lookup_nanos as f64 / 1_000_000.0,
            instrumentation.block_claim_encode_nanos as f64 / 1_000_000.0,
            instrumentation.block_claim_insert_nanos as f64 / 1_000_000.0,
            instrumentation.store.block_claim_index_reads,
            instrumentation.store.block_claim_index_writes,
            instrumentation.store.block_claim_index_syncs,
            instrumentation.block_claim_hot_entries,
            owned_semantic_snapshot_entries(),
        );
        eprintln!(
            "oplog_hot_replay_phases decode_ms={:.3} frontier_load_ms={:.3} before_snapshot_ms={:.3} exact_import_ms={:.3} after_snapshot_ms={:.3} semantic_compare_ms={:.3} replacement_validation_ms={:.3} identity_ms={:.3} exact_publication_ms={:.3} current_publication_ms={:.3} external_flushes={} external_point_reads={} external_range_scans={} external_history_page_reads={} external_history_blob_reads={}",
            replay.validation_phase_nanos[0] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[1] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[2] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[3] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[4] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[5] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[6] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[7] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[8] as f64 / 1_000_000.0,
            replay.validation_phase_nanos[9] as f64 / 1_000_000.0,
            instrumentation.external_flushes,
            instrumentation.external_point_reads,
            instrumentation.external_range_scans,
            instrumentation.external_history_page_reads,
            instrumentation.external_history_blob_reads,
        );
        assert!(
            elapsed.as_secs_f64() <= MAX_COLD_REPLAY_SECONDS,
            "one-million-block replay exceeded {MAX_COLD_REPLAY_SECONDS} seconds: {elapsed:?}"
        );
        #[cfg(target_os = "linux")]
        {
            let peak_rss_kib = replay_peak_rss_kib
                .expect("Linux replay child must expose /proc/self/status VmHWM");
            assert!(
                peak_rss_kib <= 1_048_576,
                "one-million-block replay exceeded 1 GiB RSS: {peak_rss_kib} KiB"
            );
        }

        let materialize_started = Instant::now();
        let page = replay
            .materialize_page(PageId::from_uuid(Uuid::from_u128(1_000_000)))
            .unwrap();
        let materialize_elapsed = materialize_started.elapsed();
        assert_eq!(page.blocks.len(), BLOCKS_PER_PAGE);
        assert_eq!(page.stats.physical_manifest_reads, 1);
        assert_eq!(page.stats.physical_object_reads, 1);
        eprintln!(
            "oplog_sparse_materialize batch_shards={PAGES_PER_BATCH} blocks={} elapsed_us={} manifest_reads={} object_reads={}",
            page.blocks.len(),
            materialize_elapsed.as_micros(),
            page.stats.physical_manifest_reads,
            page.stats.physical_object_reads,
        );
        drop(replay);
    }

    fn linux_peak_rss_kib() -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/self/status")
                .ok()?
                .lines()
                .find_map(|line| {
                    line.strip_prefix("VmHWM:")
                        .and_then(|value| value.split_whitespace().next())
                        .and_then(|value| value.parse().ok())
                })
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    #[test]
    #[ignore = "documents the Loro 1.13 shallow-boundary concurrent-import limitation"]
    fn correction11_shallow_checkpoint_concurrent_import_probe() {
        let base = LoroDoc::new();
        base.set_peer_id(10).unwrap();
        base.get_map("probe").insert("value", "base").unwrap();
        base.commit();
        let base_vv = base.oplog_vv();

        let left = clone_doc(&base, 11).unwrap();
        left.get_map("probe").insert("value", "left").unwrap();
        left.commit();
        let left_update = left.export(ExportMode::updates(&base_vv)).unwrap();

        let right = clone_doc(&base, 12).unwrap();
        right.get_map("probe").insert("value", "right").unwrap();
        right.commit();
        let right_update = right.export(ExportMode::updates(&base_vv)).unwrap();

        let expected = clone_doc(&base, 13).unwrap();
        assert!(expected.import(&left_update).unwrap().pending.is_none());
        assert!(expected.import(&right_update).unwrap().pending.is_none());

        let checkpoint = left
            .export(ExportMode::shallow_snapshot(&left.oplog_frontiers()))
            .unwrap();
        let restored = LoroDoc::new();
        assert!(restored.import(&checkpoint).unwrap().pending.is_none());
        let status = restored.import(&right_update).unwrap();
        assert!(
            status.pending.is_none(),
            "concurrent update remained pending behind shallow boundary: {:?}",
            status.pending
        );
        assert_eq!(restored.get_deep_value(), expected.get_deep_value());
    }
}
