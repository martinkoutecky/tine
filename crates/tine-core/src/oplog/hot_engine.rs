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
use super::portable_path_index::{
    PortablePathIndexRoot, PortablePathIndexStore, PortablePathOccupied, PortablePathRecord,
    PortablePathReleased,
};
use super::scratch_store::{ScratchRoots, ScratchStore};
use super::semantic::{
    LogseqIdentityOrigin, PagePreambleDelta, PagePreambleState, PolicyGeneratedAnchorReason,
};
use super::uuid_claim_index::{LogseqClaimIndexRoot, LogseqClaimIndexStore};
use super::{
    AnnotatedProjectionBase, BatchCausalDot, BatchId, BatchInspection, BatchOrigin, BlockDelta,
    BlockId, BlockOwner, BlockState, CausalPeerId, ContentDigest, CrdtPeerCounter, CrdtPeerId,
    DeviceId, DocumentCausalDigest, DocumentDependencies, DocumentId, FrontierV2, LineageDigest,
    LogseqUuid, ManagedPath, ManifestObjectRef, ManifestProjectionPrecondition,
    ManifestProjectionTarget, ManifestedProjectionIntent, MembershipClaim, MembershipDelta,
    ObjectKind, ObjectStore, OperationBatch, OperationObject, PageDelta, PageId, PageState,
    PortablePathKeyDigest, PreparedBatch, ProjectionClaimEvidence, ProjectionClaimParticipant,
    ProjectionCompletion, ProjectionEndpointId, ProjectionIntent, ProjectionReceiptStore,
    ProjectionWork, ProjectionWorkIndex, ProjectionWorkTarget, SemanticEffect,
    SemanticEffectDigest, SemanticError, SessionId, ValidatedBatch, WorkspaceId,
};
use crate::Graph;

const CATALOG_PAGES: &str = "pages";
const SHARD_META: &str = "shard_meta";
const SHARD_PAGE_ID: &str = "page_id";
const SHARD_OWNERS: &str = "owners";
const SHARD_MEMBERS: &str = "members";
const SHARD_CONTENT: &str = "content";
const SHARD_LOGSEQ_UUIDS: &str = "logseq_uuids";
const SHARD_LOGSEQ_IDENTITY_ORIGINS: &str = "logseq_identity_origins";
const SHARD_PAGE_PREAMBLE: &str = "page_preamble";
const SHARD_PAGE_PREAMBLE_VALUE: &str = "value";
const TOMBSTONE: &str = "tombstone";
const MAX_TRANSACTION_OPERATIONS: usize = 100_000;
const MAX_DOCUMENT_ENTRIES: usize = 1_000_000;
const MAX_HOT_NON_CATALOG_DOCUMENTS: usize = 64;
const CRDT_UPDATE_PAYLOAD_SCHEMA_VERSION: u32 = 6;
const ENGINE_HISTORY_SCHEMA_VERSION: u32 = 6;
const BLOCK_CLAIM_RECORD_SCHEMA_VERSION: u32 = 2;
const LOGSEQ_CLAIM_RECORD_SCHEMA_VERSION: u32 = 1;
const ACCEPTED_EVIDENCE_SCHEMA_VERSION: u32 = 4;
const ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION: u32 = 3;
const MAX_EPHEMERAL_BLOCK_CLAIMS: usize = 4_096;
const MAX_EPHEMERAL_LOGSEQ_CLAIMS: usize = 4_096;
const MAX_EPHEMERAL_PORTABLE_PATHS: usize = 4_096;

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

struct PreparedTransactionParts {
    prepared: PreparedBatch,
    semantic_effect: SemanticEffect,
    prospective_documents: BTreeMap<DocumentId, LoroDoc>,
    portable_path_root: PortablePathIndexRoot,
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

struct PortablePathPublicationCandidate {
    root: PortablePathIndexRoot,
    changed: Vec<(PortablePathKeyDigest, PortablePathRecord)>,
    conflicts: Vec<PortablePathConflict>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePathConflictParticipant {
    page_id: PageId,
    exact_path: ManagedPath,
    introducing_batch: BatchId,
}

impl PortablePathConflictParticipant {
    fn new(page_id: PageId, exact_path: ManagedPath, introducing_batch: BatchId) -> Self {
        Self {
            page_id,
            exact_path,
            introducing_batch,
        }
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub fn exact_path(&self) -> &ManagedPath {
        &self.exact_path
    }

    pub const fn introducing_batch(&self) -> BatchId {
        self.introducing_batch
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePathConflict {
    key_version: u32,
    key_digest: PortablePathKeyDigest,
    participants: Vec<PortablePathConflictParticipant>,
}

impl PortablePathConflict {
    fn new(
        key_digest: PortablePathKeyDigest,
        mut participants: Vec<PortablePathConflictParticipant>,
    ) -> Self {
        participants.sort_unstable();
        participants.dedup();
        Self {
            key_version: super::PORTABLE_PATH_KEY_VERSION,
            key_digest,
            participants,
        }
    }

    pub const fn key_version(&self) -> u32 {
        self.key_version
    }

    pub const fn key_digest(&self) -> PortablePathKeyDigest {
        self.key_digest
    }

    pub fn participants(&self) -> &[PortablePathConflictParticipant] {
        &self.participants
    }
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
struct LogseqClaimIntroduction {
    block_id: BlockId,
    home_document_id: DocumentId,
    batch_id: BatchId,
    causal_dot: BatchCausalDot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct LogseqClaimIntroductionRecord {
    schema_version: u32,
    introduction: LogseqClaimIntroduction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct LogseqClaimRecord {
    schema_version: u32,
    logseq_uuid: LogseqUuid,
    introductions: Vec<LogseqClaimIntroduction>,
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

fn portable_path_evidence_handle(
    conflicts: &BTreeMap<PortablePathKeyDigest, PortablePathConflict>,
) -> FatalEvidenceHandle {
    let bytes = postcard::to_allocvec(&(
        super::PORTABLE_PATH_KEY_VERSION,
        conflicts.values().collect::<Vec<_>>(),
    ))
    .expect("portable-path evidence has an infallible canonical encoding");
    let conflict_root = ContentDigest::of(&bytes);
    let conflicting_block_count = conflicts.len() as u64;
    let claim_count = conflicts
        .values()
        .map(|conflict| conflict.participants.len() as u64)
        .sum();
    let summary = postcard::to_allocvec(&(
        b"tine/portable-path-conflict-evidence/v1".as_slice(),
        super::PORTABLE_PATH_KEY_VERSION,
        conflict_root,
        conflicting_block_count,
        claim_count,
    ))
    .expect("portable-path evidence summary has an infallible canonical encoding");
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
    SetPagePreamble {
        page_id: PageId,
        preamble: Option<String>,
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
    MutateBlockLogseqIdentity {
        block: BlockLocation,
        mutation: LogseqIdentityMutation,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogseqIdentityTrigger {
    BlockReference { referrer: BlockLocation },
    BlockEmbed { referrer: BlockLocation },
    ExportUserAction,
    CopiedDeepLinkUserAction,
}

impl LogseqIdentityTrigger {
    const fn policy_reason(self) -> PolicyGeneratedAnchorReason {
        match self {
            Self::BlockReference { .. } => PolicyGeneratedAnchorReason::BlockReference,
            Self::BlockEmbed { .. } => PolicyGeneratedAnchorReason::BlockEmbed,
            Self::ExportUserAction => PolicyGeneratedAnchorReason::Export,
            Self::CopiedDeepLinkUserAction => PolicyGeneratedAnchorReason::CopiedDeepLink,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogseqIdentityMutation {
    AssignExternal {
        logseq_uuid: LogseqUuid,
    },
    ReplaceExternal {
        logseq_uuid: LogseqUuid,
    },
    RemoveExternal,
    Generate {
        logseq_uuid: LogseqUuid,
        trigger: LogseqIdentityTrigger,
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

fn validate_logseq_identity_mutation_shape(
    transaction: &OperationTransaction,
) -> Result<(), EngineError> {
    let mut mutated_blocks = BTreeSet::new();
    for operation in &transaction.operations {
        let SemanticOperation::MutateBlockLogseqIdentity { block, .. } = operation else {
            continue;
        };
        if !mutated_blocks.insert(block.block_id) {
            return Err(EngineError::InvalidTransaction(format!(
                "block {} has more than one Logseq identity mutation",
                block.block_id
            )));
        }
    }
    Ok(())
}

fn operation_content_block(operation: &SemanticOperation) -> Option<BlockLocation> {
    match operation {
        SemanticOperation::CreateBlock { block, .. }
        | SemanticOperation::EditBlockContent { block, .. } => Some(*block),
        _ => None,
    }
}

fn content_has_logseq_trigger(
    content: &str,
    logseq_uuid: LogseqUuid,
    trigger: LogseqIdentityTrigger,
    is_org: bool,
) -> bool {
    let expected = logseq_uuid.to_string();
    let projection = crate::render::parse_projection(content, is_org);
    let mut found_reference = false;
    let mut found_embed = false;
    classify_logseq_triggers(
        &projection.blocks,
        &expected,
        is_org,
        &mut found_reference,
        &mut found_embed,
    );
    match trigger {
        LogseqIdentityTrigger::BlockReference { .. } => found_reference,
        LogseqIdentityTrigger::BlockEmbed { .. } => found_embed,
        LogseqIdentityTrigger::ExportUserAction
        | LogseqIdentityTrigger::CopiedDeepLinkUserAction => true,
    }
}

fn classify_logseq_triggers(
    blocks: &[lsdoc::ast::Block],
    expected: &str,
    is_org: bool,
    found_reference: &mut bool,
    found_embed: &mut bool,
) {
    use lsdoc::ast::Block;
    for block in blocks {
        match block {
            Block::Paragraph { inline, .. }
            | Block::Heading { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::FootnoteDef { inline, .. } => {
                classify_logseq_inline_triggers(inline, expected, found_reference, found_embed);
            }
            Block::Quote { children, .. } | Block::Custom { children, .. } => {
                classify_logseq_triggers(children, expected, is_org, found_reference, found_embed);
            }
            Block::List { items, .. } => {
                for item in items {
                    classify_logseq_list_item_triggers(
                        item,
                        expected,
                        is_org,
                        found_reference,
                        found_embed,
                    );
                }
            }
            Block::Table { header, rows, .. } => {
                for cell in header
                    .iter()
                    .flat_map(|row| row.iter())
                    .chain(rows.iter().flat_map(|row| row.iter()))
                {
                    classify_logseq_inline_triggers(cell, expected, found_reference, found_embed);
                }
            }
            Block::Properties { props, .. } => {
                for property in props {
                    let projection =
                        lsdoc::parse_format(&property.1, if is_org { "org" } else { "md" });
                    classify_logseq_triggers(
                        &projection.blocks,
                        expected,
                        is_org,
                        found_reference,
                        found_embed,
                    );
                }
            }
            _ => {}
        }
    }
}

fn classify_logseq_list_item_triggers(
    item: &lsdoc::ast::ListItem,
    expected: &str,
    is_org: bool,
    found_reference: &mut bool,
    found_embed: &mut bool,
) {
    classify_logseq_inline_triggers(&item.name, expected, found_reference, found_embed);
    classify_logseq_triggers(
        &item.content,
        expected,
        is_org,
        found_reference,
        found_embed,
    );
    for child in &item.items {
        classify_logseq_list_item_triggers(child, expected, is_org, found_reference, found_embed);
    }
}

fn classify_logseq_inline_triggers(
    inlines: &[lsdoc::ast::Inline],
    expected: &str,
    found_reference: &mut bool,
    found_embed: &mut bool,
) {
    use lsdoc::ast::{Inline, Url};
    for inline in inlines {
        match inline {
            Inline::Link {
                url: Url::BlockRef { v },
                label,
                ..
            } => {
                *found_reference |= v == expected;
                classify_logseq_inline_triggers(label, expected, found_reference, found_embed);
            }
            Inline::Link { label, .. } => {
                classify_logseq_inline_triggers(label, expected, found_reference, found_embed)
            }
            Inline::Macro { name, args, .. } if name == "embed" => {
                *found_embed |= args.first().is_some_and(|argument| {
                    argument
                        .trim()
                        .strip_prefix("((")
                        .and_then(|value| value.strip_suffix("))"))
                        .is_some_and(|value| value == expected)
                });
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. } => {
                classify_logseq_inline_triggers(children, expected, found_reference, found_embed)
            }
            Inline::Fnref { definition, .. } => {
                classify_logseq_inline_triggers(definition, expected, found_reference, found_embed)
            }
            _ => {}
        }
    }
}

fn page_logseq_references(
    path: &ManagedPath,
    preamble: Option<&str>,
    blocks: &[MaterializedBlock],
) -> BTreeSet<LogseqUuid> {
    let is_org = path.as_str().ends_with(".org");
    let mut references = BTreeSet::new();
    if let Some(preamble) = preamble {
        let format = if is_org { "org" } else { "md" };
        references.extend(
            lsdoc::parse_format(preamble, format)
                .refs
                .block
                .into_iter()
                .filter_map(|value| LogseqUuid::parse(&value).ok()),
        );
    }
    for block in blocks {
        references.extend(
            crate::render::block_refs(&block.content, is_org)
                .block
                .into_iter()
                .filter_map(|value| LogseqUuid::parse(&value).ok()),
        );
    }
    references
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorBatch {
    pub batch_id: BatchId,
    pub author_device_id: DeviceId,
    pub author_session_id: SessionId,
    pub crdt_peer_id: CrdtPeerId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionEndpointBinding {
    pub(crate) endpoint_id: ProjectionEndpointId,
    pub(crate) device_id: DeviceId,
    pub(crate) graph_resource_id: super::CanonicalGraphResourceId,
}

impl ProjectionEndpointBinding {
    pub fn enroll_graph(
        graph: &Graph,
        endpoint_id: ProjectionEndpointId,
        device_id: DeviceId,
    ) -> std::io::Result<Self> {
        Ok(Self {
            endpoint_id,
            device_id,
            graph_resource_id: graph.canonical_resource_id()?,
        })
    }

    pub const fn endpoint_id(self) -> ProjectionEndpointId {
        self.endpoint_id
    }

    pub const fn device_id(self) -> DeviceId {
        self.device_id
    }

    pub const fn graph_resource_id(self) -> super::CanonicalGraphResourceId {
        self.graph_resource_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CurrentPageAtPath {
    ExactOwner(PortablePathOccupied),
    Released(PortablePathReleased),
    Unowned,
    PortableCollision(PortablePathOccupied),
    ReleasedPortableCollision(PortablePathReleased),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionStorageBinding {
    pub(crate) endpoint: ProjectionEndpointBinding,
    pub(crate) receipt_store_id: super::ProjectionReceiptStoreId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionRequirementState {
    Absent,
    Present,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionRequirement {
    page_id: PageId,
    path: ManagedPath,
    precondition: ProjectionRequirementState,
    target: ProjectionRequirementState,
    render_base_path: Option<ManagedPath>,
}

impl ProjectionRequirement {
    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub const fn precondition(&self) -> ProjectionRequirementState {
        self.precondition
    }

    pub const fn target(&self) -> ProjectionRequirementState {
        self.target
    }

    pub fn render_base_path(&self) -> Option<&ManagedPath> {
        self.render_base_path.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityCapturedProjectionState {
    Absent,
    Present {
        bytes: Vec<u8>,
        prior_intent: ProjectionIntent,
        prior_completion: ProjectionCompletion,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityCapturedProjectionInput {
    path: ManagedPath,
    endpoint: ProjectionEndpointBinding,
    state: CapabilityCapturedProjectionState,
}

impl CapabilityCapturedProjectionInput {
    pub(crate) fn from_graph_capability(
        path: ManagedPath,
        endpoint: ProjectionEndpointBinding,
        state: CapabilityCapturedProjectionState,
    ) -> Self {
        Self {
            path,
            endpoint,
            state,
        }
    }

    pub fn path(&self) -> &ManagedPath {
        &self.path
    }
}

struct DraftProjectionPage {
    before: Option<ProjectionPageState>,
    after: Option<ProjectionPageState>,
    post_frontier: FrontierV2,
}

/// Speculative author state. Only bounded path requirements are observable;
/// CRDT buffers, semantic effects, and the prospective object set remain
/// private until exact endpoint inputs finalize the closed batch.
pub struct AuthorTransactionDraft {
    author: AuthorBatch,
    origin: BatchOrigin,
    generation: u64,
    root_token: ContentDigest,
    prepared_core: PreparedBatch,
    semantic_effect: SemanticEffect,
    portable_path_root: PortablePathIndexRoot,
    prospective_documents: BTreeMap<DocumentId, LoroDoc>,
    requirements: Vec<ProjectionRequirement>,
    pages: BTreeMap<PageId, DraftProjectionPage>,
}

impl AuthorTransactionDraft {
    pub fn requirements(&self) -> &[ProjectionRequirement] {
        &self.requirements
    }
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
/// Device-local evidence for a derived accepted-frontier projection.
///
/// This value authenticates local engine and SQLite projection transitions. It
/// is deliberately not a protocol frontier, interchange format, or portable
/// export representation: its state is acceptance-order dependent and may
/// contain run-local scratch-store references. Its Serde encoding exists only
/// for Tine's validated local evidence/archive paths and carries no wire-format
/// stability claim.
#[non_exhaustive]
pub struct AcceptedFrontierRoot {
    schema_version: u32,
    acceptance_sequence: u64,
    document_count: u64,
    retained_bytes_total: u64,
    document_map_root_key: Option<[u8; 16]>,
    document_map_root_digest: ContentDigest,
    // Commits every accepted BatchId to its immutable manifest/event binding,
    // causal dot, and authenticated sparse-clock root.
    batch_map_root_key: Option<[u8; 16]>,
    batch_map_root_digest: ContentDigest,
    state_digest: ContentDigest,
    scratch_root: Option<super::scratch_store::ScratchLsmRoot>,
}

impl AcceptedBatchEvidence {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_test(
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        event_binding_digest: ContentDigest,
        prior_frontier_root: AcceptedFrontierRoot,
        affected_documents: Vec<DocumentDependencies>,
        all_documents: Vec<DocumentDependencies>,
        accepted_batch_entries: Vec<(BatchId, ContentDigest)>,
        retained_bytes: u64,
    ) -> Self {
        let acceptance_sequence = prior_frontier_root.acceptance_sequence.saturating_add(1);
        let (document_map_root_key, document_map_root_digest) =
            authenticated_document_map_root(&all_documents)
                .expect("canonical test authenticated document map");
        let (batch_map_root_key, batch_map_root_digest) =
            authenticated_map_root(&accepted_batch_entries)
                .expect("canonical test authenticated batch map");
        let post_frontier_root = next_accepted_frontier_root(
            &prior_frontier_root,
            event_binding_digest,
            acceptance_sequence,
            all_documents.len() as u64,
            retained_bytes,
            &affected_documents,
            document_map_root_key,
            document_map_root_digest,
            batch_map_root_key,
            batch_map_root_digest,
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

    pub(crate) fn validate(&self) -> Result<(), EngineError> {
        validate_accepted_evidence(self)
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

    pub const fn retained_bytes_total(&self) -> u64 {
        self.retained_bytes_total
    }

    pub const fn document_map_root_key(&self) -> Option<[u8; 16]> {
        self.document_map_root_key
    }

    pub const fn document_map_root_digest(&self) -> ContentDigest {
        self.document_map_root_digest
    }

    pub const fn batch_map_root_key(&self) -> Option<[u8; 16]> {
        self.batch_map_root_key
    }

    pub const fn batch_map_root_digest(&self) -> ContentDigest {
        self.batch_map_root_digest
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
        retained_bytes: u64,
        affected_documents: &[DocumentDependencies],
        post: &Self,
    ) -> Result<bool, EngineError> {
        Ok(next_accepted_frontier_root(
            self,
            event_binding_digest,
            acceptance_sequence,
            document_count,
            retained_bytes,
            affected_documents,
            post.document_map_root_key,
            post.document_map_root_digest,
            post.batch_map_root_key,
            post.batch_map_root_digest,
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
        store: Arc<super::object_store::DurableEngineHistoryStore>,
        through_generation: u64,
        history_root: ContentDigest,
        active: Vec<ColdHistoryRecord>,
    },
    DurableAndScratch {
        history_store: Arc<super::object_store::DurableEngineHistoryStore>,
        through_generation: u64,
        history_root: ContentDigest,
        scratch_store: Arc<ScratchStore>,
        scratch_roots: ScratchRoots,
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
            Self::DurableAndScratch {
                history_store,
                through_generation,
                history_root,
                scratch_store,
                scratch_roots,
            } => {
                let durable =
                    validated_history_records(history_store, *through_generation, *history_root)?;
                let scratch = super::dependency_queue::all_records(scratch_store, scratch_roots)
                    .map_err(|error| EngineError::Archive(error.to_string()))?
                    .into_iter()
                    .map(|record| {
                        let status = match record.status() {
                            super::dependency_queue::CompactBatchStatus::Final => {
                                decode_archive_status(record.final_status().ok_or_else(|| {
                                    EngineError::Archive(
                                        "final scratch status has no result".into(),
                                    )
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
                            portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
                            portable_path_root: PortablePathIndexRoot::empty(),
                            catalog_checkpoint_binding: ContentDigest::of(
                                b"tine/transient-scratch-catalog-binding/v1",
                            ),
                            portable_path_conflicts: Vec::new(),
                            status,
                        })
                    })
                    .collect::<Result<Vec<_>, EngineError>>()?;
                let mut records = durable
                    .into_iter()
                    .map(|record| (record.batch_id, record))
                    .collect::<BTreeMap<_, _>>();
                for record in scratch {
                    records.entry(record.batch_id).or_insert(record);
                }
                records.into_values().collect()
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
                        portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
                        portable_path_root: PortablePathIndexRoot::empty(),
                        catalog_checkpoint_binding: ContentDigest::of(
                            b"tine/transient-scratch-catalog-binding/v1",
                        ),
                        portable_path_conflicts: Vec::new(),
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
    portable_path_key_version: u32,
    portable_path_root: PortablePathIndexRoot,
    catalog_checkpoint_binding: ContentDigest,
    portable_path_conflicts: Vec<PortablePathConflict>,
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
    pub logseq_uuid: Option<LogseqUuid>,
    pub logseq_identity_origin: Option<LogseqIdentityOrigin>,
    pub content: String,
}

impl MaterializedBlock {
    pub const fn policy_generated_logseq_uuid(&self) -> Option<LogseqUuid> {
        match (self.logseq_uuid, self.logseq_identity_origin) {
            (Some(uuid), Some(LogseqIdentityOrigin::PolicyGenerated { .. })) => Some(uuid),
            _ => None,
        }
    }
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
    pub preamble: Option<String>,
    pub blocks: Vec<MaterializedBlock>,
    pub stats: MaterializationStats,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPageState {
    pub page: MaterializedPage,
    pub frontier: FrontierV2,
    pub claim_evidence: Vec<ProjectionClaimEvidence>,
}

/// Engine-issued proof that projection state came from accepted durable
/// batches at the exact returned frontier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWriteAuthorization {
    state: ProjectionPageState,
    claim_root: LogseqClaimIndexRoot,
}

impl ProjectionWriteAuthorization {
    pub const fn state(&self) -> &ProjectionPageState {
        &self.state
    }

    pub fn into_state(self) -> ProjectionPageState {
        self.state
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogseqUuidClaim {
    pub logseq_uuid: LogseqUuid,
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub page_id: PageId,
    pub origin: LogseqIdentityOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogseqUuidResolution {
    Unclaimed,
    Unique(LogseqUuidClaim),
    Ambiguous { claim_count: usize },
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
    pub scratch_page_reads: usize,
    pub scratch_page_bytes_read: usize,
    pub scratch_max_page_bytes_read: usize,
    pub scratch_syncs: usize,
    pub stale_scratch_runs_reclaimed: usize,
    pub live_scratch_runs_skipped: usize,
    pub batch_status_hot_entries: usize,
    pub ready_payload_hot_entries: usize,
    pub document_hot_entries: usize,
    pub conflict_hot_entries: usize,
    pub block_claim_hot_entries: usize,
    pub logseq_claim_hot_entries: usize,
    pub logseq_claim_index_reads: usize,
    pub logseq_claim_index_writes: usize,
    pub portable_path_index_reads: usize,
    pub portable_path_index_writes: usize,
    pub portable_path_index_bytes_read: usize,
    pub portable_path_index_bytes_written: usize,
    pub projection_work_node_reads: usize,
    pub projection_work_root_reads: usize,
    pub projection_work_prepared_reads: usize,
    pub projection_pending_entries_read: usize,
    pub block_claim_validation_nanos: usize,
    pub block_claim_lookup_nanos: usize,
    pub block_claim_encode_nanos: usize,
    pub block_claim_insert_nanos: usize,
    pub store: super::ObjectStoreStats,
}

pub(crate) enum AcceptedBatchCursor<'a> {
    Scratch(super::scratch_store::ScratchAcceptedSequenceCursor<'a>),
    Inline(std::collections::btree_map::Iter<'a, u64, BatchId>),
}

impl AcceptedBatchCursor<'_> {
    pub(crate) fn next_batch(
        &mut self,
    ) -> Result<Option<(u64, BatchId, Option<AcceptedBatchEvidence>)>, EngineError> {
        match self {
            Self::Scratch(cursor) => cursor
                .next_batch()
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .map(|(sequence, entry)| {
                    let evidence = decode_accepted_evidence(&entry.evidence)?;
                    if evidence.batch_id != entry.batch_id
                        || evidence.acceptance_sequence != sequence
                    {
                        return Err(EngineError::Archive(
                            "accepted sequence entry is misbound".into(),
                        ));
                    }
                    Ok((sequence, entry.batch_id, Some(evidence)))
                })
                .transpose(),
            Self::Inline(iter) => Ok(iter
                .next()
                .map(|(sequence, batch_id)| (*sequence, *batch_id, None))),
        }
    }

    pub(crate) fn page_stats(&self) -> (usize, usize, usize) {
        match self {
            Self::Scratch(cursor) => cursor.page_stats(),
            Self::Inline(_) => (0, 0, 0),
        }
    }
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
    projection_endpoint: Option<ProjectionEndpointBinding>,
    projection_receipt_store_id: Option<super::ProjectionReceiptStoreId>,
    projection_work_index: Option<Arc<ProjectionWorkIndex>>,
    scratch: Option<Arc<ScratchStore>>,
    scratch_roots: ScratchRoots,
    ephemeral_causal_chain: RefCell<BTreeMap<CausalPeerId, (u64, BatchId)>>,
    history_store: Option<Arc<super::object_store::DurableEngineHistoryStore>>,
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
    logseq_claim_index: Option<Arc<LogseqClaimIndexStore>>,
    logseq_claim_root: LogseqClaimIndexRoot,
    ephemeral_logseq_claims: BTreeMap<LogseqUuid, LogseqClaimRecord>,
    portable_path_index: Option<Arc<PortablePathIndexStore>>,
    portable_path_root: PortablePathIndexRoot,
    ephemeral_portable_paths: BTreeMap<PortablePathKeyDigest, PortablePathRecord>,
    portable_path_conflicts: BTreeMap<PortablePathKeyDigest, PortablePathConflict>,
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
    ephemeral_causal_clocks: BTreeMap<BatchId, Vec<(CausalPeerId, u64)>>,
    ephemeral_accepted_batch_entries: BTreeMap<BatchId, ContentDigest>,
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
            projection_endpoint: None,
            projection_receipt_store_id: None,
            projection_work_index: None,
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
            logseq_claim_index: None,
            logseq_claim_root: LogseqClaimIndexRoot::empty(),
            ephemeral_logseq_claims: BTreeMap::new(),
            portable_path_index: None,
            portable_path_root: PortablePathIndexRoot::empty(),
            ephemeral_portable_paths: BTreeMap::new(),
            portable_path_conflicts: BTreeMap::new(),
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
            ephemeral_causal_clocks: BTreeMap::new(),
            ephemeral_accepted_batch_entries: BTreeMap::new(),
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
        match store.open_logseq_claim_index() {
            Ok(index) => engine.logseq_claim_index = Some(Arc::new(index)),
            Err(error) => engine.history_failure = Some(EngineError::Archive(error.to_string())),
        }
        match store.open_portable_path_index() {
            Ok(index) => engine.portable_path_index = Some(Arc::new(index)),
            Err(error) => engine.history_failure = Some(EngineError::Archive(error.to_string())),
        }
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

    /// Enroll manifested projection work only after the archive, receipt
    /// namespace, retained graph capability, endpoint, and durable history all
    /// agree. The resulting engine owns the work-index capability.
    pub fn with_enrolled_projection(
        store: ObjectStore,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
    ) -> Self {
        let workspace_id = store.workspace_id();
        let endpoint = receipts.endpoint_binding();
        let receipt_store_id = receipts.store_id();
        let enrollment_error = if receipts.workspace_id() != workspace_id {
            Some("projection receipt workspace does not match archive workspace".to_owned())
        } else if endpoint.is_none() {
            Some("projection receipt namespace has no endpoint enrollment".to_owned())
        } else {
            match graph.canonical_resource_id() {
                Ok(graph_resource_id)
                    if Some(graph_resource_id)
                        == endpoint.map(|binding| binding.graph_resource_id) =>
                {
                    None
                }
                Ok(_) => {
                    Some("projection receipt enrollment does not match graph capability".to_owned())
                }
                Err(error) => Some(format!(
                    "projection graph capability identity could not be verified: {error}"
                )),
            }
        };
        if let Some(error) = enrollment_error {
            let mut engine = Self::with_archive_store(store, lineage_digest, catalog_document_id);
            engine.history_failure = Some(EngineError::Archive(error));
            return engine;
        }
        Self::with_archive_store_for_endpoint(
            store,
            lineage_digest,
            catalog_document_id,
            ProjectionStorageBinding {
                endpoint: endpoint.expect("validated enrolled endpoint"),
                receipt_store_id,
            },
        )
    }

    pub(crate) fn with_archive_store_for_endpoint(
        store: ObjectStore,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
        binding: ProjectionStorageBinding,
    ) -> Self {
        let endpoint = binding.endpoint;
        let mut engine = Self::with_archive_store(store, lineage_digest, catalog_document_id);
        let history = engine
            .archive_store
            .as_ref()
            .expect("archive store was installed")
            .open_engine_history(binding);
        match history {
            Ok(history) => match history.current_with_binding() {
                Ok((generation, root, _, binding)) => {
                    engine.history_generation = generation;
                    engine.history_root = root;
                    engine.history_store = Some(Arc::new(history));
                    if binding.portable_path_key_version != super::PORTABLE_PATH_KEY_VERSION {
                        engine.history_failure = Some(EngineError::Archive(
                            "durable history portable-path key version mismatch".into(),
                        ));
                    } else {
                        let portable_root =
                            PortablePathIndexRoot::from_digest(binding.portable_path_root);
                        match engine
                            .portable_path_index
                            .as_ref()
                            .ok_or_else(|| {
                                EngineError::Archive(
                                    "durable history has no portable-path index".into(),
                                )
                            })
                            .and_then(|index| {
                                index
                                    .validate_root(portable_root)
                                    .map_err(|error| EngineError::Archive(error.to_string()))
                            }) {
                            Ok(()) => engine.portable_path_root = portable_root,
                            Err(error) => engine.history_failure = Some(error),
                        }
                    }
                    if let Some(terminal) = binding.terminal_evidence {
                        engine.fatal_handle = Some(FatalEvidenceHandle {
                            conflict_root: terminal.conflict_root,
                            conflicting_block_count: terminal.conflict_count,
                            claim_count: terminal.participant_count,
                            canonical_digest: terminal.canonical_digest,
                        });
                    }
                    engine.portable_path_conflicts = binding
                        .portable_path_conflicts
                        .into_iter()
                        .map(|conflict| (conflict.key_digest(), conflict))
                        .collect();
                    if !engine.portable_path_conflicts.is_empty() {
                        let expected =
                            portable_path_evidence_handle(&engine.portable_path_conflicts);
                        if engine.fatal_handle != Some(expected) {
                            engine.history_failure = Some(EngineError::Archive(
                                "durable portable-path evidence binding mismatch".into(),
                            ));
                        }
                    }
                }
                Err(error) => {
                    engine.history_failure = Some(EngineError::Archive(error.to_string()))
                }
            },
            Err(error) => engine.history_failure = Some(EngineError::Archive(error.to_string())),
        }
        if engine.history_failure.is_none() {
            let result = engine
                .archive_store
                .as_ref()
                .expect("archive store was installed")
                .open_projection_work_index(binding);
            match result {
                Ok(index) => {
                    engine.projection_endpoint = Some(endpoint);
                    engine.projection_receipt_store_id = Some(binding.receipt_store_id);
                    engine.projection_work_index = Some(Arc::new(index));
                    if engine.fatal_handle.is_none() {
                        if let Err(error) = engine.reconcile_pending_projection_work() {
                            engine.history_failure = Some(error);
                        }
                    }
                }
                Err(error) => {
                    engine.history_failure = Some(EngineError::Archive(error.to_string()))
                }
            }
        }
        engine
    }

    pub(crate) fn enrolled_projection_runtime(
        &self,
    ) -> Result<(Arc<ObjectStore>, Arc<ProjectionWorkIndex>), EngineError> {
        let archive = self.archive_store.as_ref().ok_or_else(|| {
            EngineError::ProjectionWork("engine has no enrolled projection archive".into())
        })?;
        let index = self.projection_work_index.as_ref().ok_or_else(|| {
            EngineError::ProjectionWork("engine has no enrolled projection work index".into())
        })?;
        let endpoint = self.projection_endpoint.ok_or_else(|| {
            EngineError::ProjectionWork("engine has no enrolled projection endpoint".into())
        })?;
        if archive.workspace_id() != self.workspace_id
            || index.workspace_id() != self.workspace_id
            || index.endpoint_id() != endpoint.endpoint_id
            || index.graph_resource_id() != endpoint.graph_resource_id
            || index.receipt_store_id()
                != self.projection_receipt_store_id.ok_or_else(|| {
                    EngineError::ProjectionWork(
                        "engine has no enrolled projection receipt store".into(),
                    )
                })?
        {
            return Err(EngineError::ProjectionWork(
                "engine-owned projection runtime binding mismatch".into(),
            ));
        }
        Ok((Arc::clone(archive), Arc::clone(index)))
    }

    pub fn projection_work_index(&self) -> Option<&ProjectionWorkIndex> {
        self.projection_work_index.as_deref()
    }

    pub const fn projection_endpoint_binding(&self) -> Option<ProjectionEndpointBinding> {
        self.projection_endpoint
    }

    pub(crate) const fn projection_receipt_store_id(
        &self,
    ) -> Option<super::ProjectionReceiptStoreId> {
        self.projection_receipt_store_id
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
        let logseq_claims = self
            .logseq_claim_index
            .as_ref()
            .map(|index| index.stats())
            .unwrap_or_default();
        let portable_paths = self
            .portable_path_index
            .as_ref()
            .map(|index| index.stats())
            .unwrap_or_default();
        let scratch = self
            .scratch
            .as_ref()
            .map(|store| store.stats())
            .unwrap_or_default();
        let projection_work = self
            .projection_work_index
            .as_ref()
            .map(|index| index.stats())
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
            scratch_page_reads: scratch.page_reads,
            scratch_page_bytes_read: scratch.page_bytes_read,
            scratch_max_page_bytes_read: scratch.max_page_bytes_read,
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
            logseq_claim_hot_entries: self.ephemeral_logseq_claims.len(),
            logseq_claim_index_reads: logseq_claims.reads,
            logseq_claim_index_writes: logseq_claims.writes,
            portable_path_index_reads: portable_paths.reads,
            portable_path_index_writes: portable_paths.writes,
            portable_path_index_bytes_read: portable_paths.bytes_read,
            portable_path_index_bytes_written: portable_paths.bytes_written,
            projection_work_node_reads: projection_work.node_reads,
            projection_work_root_reads: projection_work.root_reads,
            projection_work_prepared_reads: projection_work.prepared_reads,
            projection_pending_entries_read: projection_work.pending_entries_read,
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
                    self.portable_path_root,
                    self.catalog_checkpoint_binding(),
                    self.portable_path_conflicts.values().cloned().collect(),
                    status.clone(),
                )
            })
            .collect();
        let history_source = match (&self.history_failure, &self.scratch, &self.history_store) {
            (Some(error), _, _) => StatusHistorySource::Failed(error.clone()),
            (None, Some(scratch_store), Some(history_store)) => {
                StatusHistorySource::DurableAndScratch {
                    history_store: Arc::clone(history_store),
                    through_generation: self.history_generation,
                    history_root: self.history_root,
                    scratch_store: Arc::clone(scratch_store),
                    scratch_roots: self.scratch_roots.clone(),
                }
            }
            (None, Some(store), None) => StatusHistorySource::Scratch {
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
        Ok(self
            .accepted_batch_entry_at(sequence)?
            .map(|(batch_id, _)| batch_id))
    }

    pub(crate) fn accepted_batch_entry_at(
        &self,
        sequence: u64,
    ) -> Result<Option<(BatchId, Option<AcceptedBatchEvidence>)>, EngineError> {
        self.ensure_not_blocked()?;
        if sequence == 0 || sequence > self.next_acceptance_sequence {
            return Ok(None);
        }
        if let Some(store) = &self.scratch {
            return store
                .lookup_accepted_sequence(&self.scratch_roots.accepted_sequence_root, sequence)
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .map(|entry| {
                    let evidence = decode_accepted_evidence(&entry.evidence)?;
                    if evidence.batch_id != entry.batch_id
                        || evidence.acceptance_sequence != sequence
                    {
                        return Err(EngineError::Archive(
                            "accepted sequence point entry is misbound".into(),
                        ));
                    }
                    Ok((entry.batch_id, Some(evidence)))
                })
                .transpose();
        }
        Ok(self
            .accepted_sequence
            .get(&sequence)
            .copied()
            .map(|batch_id| (batch_id, None)))
    }

    pub(crate) fn accepted_batch_cursor(&self) -> Result<AcceptedBatchCursor<'_>, EngineError> {
        self.ensure_not_blocked()?;
        if let Some(store) = &self.scratch {
            return Ok(AcceptedBatchCursor::Scratch(
                store
                    .accepted_sequence_cursor(&self.scratch_roots.accepted_sequence_root)
                    .map_err(|error| EngineError::Archive(error.to_string()))?,
            ));
        }
        Ok(AcceptedBatchCursor::Inline(self.accepted_sequence.iter()))
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
        let (post_documents, scratch_root, document_map_root_key, document_map_root_digest) =
            if let Some(store) = &self.scratch {
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
                for (document_id, dependencies) in &changed_documents {
                    let value_digest = ContentDigest::of(&encode_accepted_document(dependencies)?);
                    roots.accepted_document_map_root = store
                        .authenticated_map_upsert(
                            &roots.accepted_document_map_root,
                            document_id.as_uuid().into_bytes(),
                            value_digest,
                        )
                        .map_err(|error| EngineError::Archive(error.to_string()))?;
                }
                if roots.accepted_document_map_root.count() != new_document_count {
                    return Err(EngineError::Archive(
                        "authenticated document map count differs from accepted frontier".into(),
                    ));
                }
                (
                    None,
                    Some(roots.accepted_frontier_root.clone()),
                    roots.accepted_document_map_root.root_key(),
                    roots.accepted_document_map_root.root_digest(),
                )
            } else {
                let mut post_documents = self.accepted_frontier.clone();
                post_documents.extend(changed_documents.clone());
                let all_documents = post_documents.values().cloned().collect::<Vec<_>>();
                let (root_key, root_digest) = authenticated_document_map_root(&all_documents)?;
                (Some(post_documents), None, root_key, root_digest)
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
        let causal_clock = if let Some(store) = &self.scratch {
            super::causal_index::batch_record(store, &roots, batch_id)
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .ok_or_else(|| {
                    EngineError::Archive(format!(
                        "accepted batch {batch_id} has no authenticated causal record"
                    ))
                })?
                .clock()
                .to_vec()
        } else {
            self.derive_ephemeral_causal_clock(manifest)?
        };
        let (clock_root_key, clock_root_digest) = authenticated_causal_clock_root(&causal_clock)?;
        let causal_record_digest = accepted_causal_record_digest(
            batch_id,
            manifest_fingerprint,
            event_binding_digest,
            manifest.causal_dot(),
            clock_root_key,
            clock_root_digest,
        );
        let (batch_map_root_key, batch_map_root_digest) = if let Some(store) = &self.scratch {
            roots.accepted_batch_map_root = store
                .accepted_batch_map_upsert(
                    &roots.accepted_batch_map_root,
                    batch_id.as_uuid().into_bytes(),
                    causal_record_digest,
                )
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            if roots.accepted_batch_map_root.count() != acceptance_sequence {
                return Err(EngineError::Archive(
                    "authenticated batch map count differs from accepted sequence".into(),
                ));
            }
            (
                roots.accepted_batch_map_root.root_key(),
                roots.accepted_batch_map_root.root_digest(),
            )
        } else {
            let mut entries = self.ephemeral_accepted_batch_entries.clone();
            entries.insert(batch_id, causal_record_digest);
            authenticated_map_root(&entries.into_iter().collect::<Vec<_>>())?
        };
        let affected_documents = changed_documents.into_values().collect::<Vec<_>>();
        let retained_bytes = accepted_batch_retained_bytes(&self.archive[&batch_id])?;
        let post_frontier_root = next_accepted_frontier_root(
            &prior_frontier_root,
            event_binding_digest,
            acceptance_sequence,
            new_document_count,
            retained_bytes,
            &affected_documents,
            document_map_root_key,
            document_map_root_digest,
            batch_map_root_key,
            batch_map_root_digest,
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
            roots.accepted_sequence_root = store
                .append_accepted_sequence(
                    &roots.accepted_sequence_root,
                    acceptance_sequence,
                    batch_id,
                    postcard::to_allocvec(&evidence)
                        .map_err(|error| EngineError::Archive(error.to_string()))?,
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
            let manifest = self.archive[&evidence.batch_id].manifest();
            let clock = self
                .derive_ephemeral_causal_clock(manifest)
                .expect("accepted inline causal clock was validated");
            let (clock_root_key, clock_root_digest) =
                authenticated_causal_clock_root(&clock).expect("canonical accepted causal clock");
            let record_digest = accepted_causal_record_digest(
                evidence.batch_id,
                evidence.manifest_fingerprint,
                evidence.event_binding_digest,
                manifest.causal_dot(),
                clock_root_key,
                clock_root_digest,
            );
            self.ephemeral_causal_clocks
                .insert(evidence.batch_id, clock);
            self.ephemeral_accepted_batch_entries
                .insert(evidence.batch_id, record_digest);
            self.accepted_frontier = post_documents.expect("inline accepted frontier");
            self.accepted_sequence
                .insert(evidence.acceptance_sequence, evidence.batch_id);
        }
    }

    fn derive_ephemeral_causal_clock(
        &self,
        manifest: &OperationBatch,
    ) -> Result<Vec<(CausalPeerId, u64)>, EngineError> {
        let mut clock = BTreeMap::<CausalPeerId, u64>::new();
        for parent in manifest.causal_dependency_heads() {
            let parent_clock = self
                .ephemeral_causal_clocks
                .get(parent)
                .ok_or(EngineError::MissingDependency(*parent))?;
            for (peer, counter) in parent_clock {
                clock
                    .entry(*peer)
                    .and_modify(|current| *current = (*current).max(*counter))
                    .or_insert(*counter);
            }
        }
        let dot = manifest.causal_dot();
        let expected = clock
            .get(&dot.peer_id())
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| EngineError::Archive("causal counter overflowed".into()))?;
        if dot.counter() != expected {
            return Err(EngineError::InvalidCrdt(format!(
                "causal dot {:?}:{} is not gap-free; expected {expected}",
                dot.peer_id(),
                dot.counter()
            )));
        }
        clock.insert(dot.peer_id(), dot.counter());
        Ok(clock.into_iter().collect())
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

    pub const fn portable_path_index_root(&self) -> PortablePathIndexRoot {
        self.portable_path_root
    }

    /// Authenticated point lookup used by import scope capture.
    ///
    /// The portable key is only an index key; exact case-preserved path
    /// equality is rechecked before returning an owner.
    pub fn current_page_at_path(
        &self,
        path: &ManagedPath,
    ) -> Result<CurrentPageAtPath, EngineError> {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let key = path.portable_key().digest();
        let record = self.portable_path_records_many(&[key])?.remove(&key);
        Ok(match record {
            Some(record) => match (record.occupied().cloned(), record.latest_release().cloned()) {
                (Some(occupied), _) if occupied.exact_path() == path => {
                    CurrentPageAtPath::ExactOwner(occupied)
                }
                (Some(occupied), _) => CurrentPageAtPath::PortableCollision(occupied),
                (None, Some(released)) if released.prior_exact_path() == path => {
                    CurrentPageAtPath::Released(released)
                }
                (None, Some(released)) => CurrentPageAtPath::ReleasedPortableCollision(released),
                (None, None) => CurrentPageAtPath::Unowned,
            },
            None => CurrentPageAtPath::Unowned,
        })
    }

    pub fn portable_path_conflicts(&self) -> Vec<PortablePathConflict> {
        self.portable_path_conflicts.values().cloned().collect()
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
            if matches!(existing.status, ArchiveStatus::Accepted { .. }) {
                if let Err(error) = self.prepare_projection_work_for_batch(&batch) {
                    self.history_failure = Some(error.clone());
                    return self.outcome(
                        batch_id,
                        BatchDisposition::Rejected { error },
                        Vec::new(),
                    );
                }
                if let Err(error) = self.activate_projection_work(batch_id, fingerprint) {
                    self.history_failure = Some(error.clone());
                    return self.outcome(
                        batch_id,
                        BatchDisposition::Rejected { error },
                        Vec::new(),
                    );
                }
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
                    if let Err(error) = self.prepare_projection_work(batch_id) {
                        self.history_failure = Some(error.clone());
                        return self.outcome(
                            batch_id,
                            BatchDisposition::Rejected { error },
                            Vec::new(),
                        );
                    }
                    if let Err(error) = self.activate_projection_work(batch_id, fingerprint) {
                        self.history_failure = Some(error.clone());
                        return self.outcome(
                            batch_id,
                            BatchDisposition::Rejected { error },
                            Vec::new(),
                        );
                    }
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
        match self.cold_history_record(offered_batch_id) {
            Ok(Some(existing)) => {
                if existing.manifest_fingerprint != fingerprint {
                    let error = EngineError::BatchCollision(offered_batch_id);
                    return self.outcome(
                        offered_batch_id,
                        BatchDisposition::Rejected { error },
                        Vec::new(),
                    );
                }
                if matches!(
                    existing.status,
                    ArchiveStatus::Rejected(_) | ArchiveStatus::Quarantined
                ) {
                    return self.outcome(
                        offered_batch_id,
                        disposition_from_final_status(existing.status, true),
                        Vec::new(),
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.history_failure = Some(error.clone());
                return self.outcome(
                    offered_batch_id,
                    BatchDisposition::Rejected { error },
                    Vec::new(),
                );
            }
        }
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
            if let Err(error) =
                self.persist_durable_final_status(offered_batch_id, fingerprint, status.clone())
            {
                self.history_failure = Some(error.clone());
                return self.outcome(
                    offered_batch_id,
                    BatchDisposition::Rejected { error },
                    Vec::new(),
                );
            }
            if matches!(status, ArchiveStatus::Accepted { .. }) {
                if let Err(error) = self.prepare_projection_work_for_batch(&batch) {
                    self.history_failure = Some(error.clone());
                    return self.outcome(
                        offered_batch_id,
                        BatchDisposition::Rejected { error },
                        Vec::new(),
                    );
                }
                if let Err(error) = self.activate_projection_work(offered_batch_id, fingerprint) {
                    self.history_failure = Some(error.clone());
                    return self.outcome(
                        offered_batch_id,
                        BatchDisposition::Rejected { error },
                        Vec::new(),
                    );
                }
            }
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
            if let Err(error) =
                self.persist_durable_final_status(batch_id, ready_fingerprint, final_status.clone())
            {
                self.history_failure = Some(error);
                break;
            }
            if matches!(final_status, ArchiveStatus::Accepted { .. }) {
                if let Err(error) = self.activate_projection_work(batch_id, ready_fingerprint) {
                    self.history_failure = Some(error);
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

    /// Origin-explicit compatibility helper for bootstrap/import fixtures.
    ///
    /// Normal local mutation authors must use the speculative draft/finalize
    /// API so projection evidence is part of the closed manifest object set.
    pub fn prepare_bootstrap_transaction(
        &self,
        author: AuthorBatch,
        transaction: &OperationTransaction,
    ) -> Result<PreparedBatch, EngineError> {
        Ok(self
            .prepare_transaction_core(
                author,
                super::BatchOrigin::BootstrapImport,
                transaction,
                false,
            )?
            .prepared)
    }

    pub fn draft_author_transaction(
        &self,
        author: AuthorBatch,
        origin: BatchOrigin,
        transaction: &OperationTransaction,
    ) -> Result<AuthorTransactionDraft, EngineError> {
        if origin == BatchOrigin::BootstrapImport {
            return Err(EngineError::InvalidTransaction(
                "bootstrap import must use the origin-explicit bootstrap helper".into(),
            ));
        }
        let generation = self.history_generation;
        let root_token = self.author_generation_root()?;
        let parts = self.prepare_transaction_core(author, origin, transaction, true)?;
        let affected_pages = affected_projection_pages(&parts.semantic_effect);
        let mut pages = BTreeMap::new();
        for page_id in affected_pages {
            let before = match self.materialize_page_for_projection(page_id) {
                Ok(state) => Some(state),
                Err(EngineError::PageNotFound(_) | EngineError::PageDeleted(_)) => None,
                Err(error) => return Err(error),
            };
            let after = self.prospective_projection_page(
                page_id,
                author.batch_id,
                &parts.prospective_documents,
                &parts.semantic_effect,
            )?;
            let post_frontier = match &after {
                Some(after) => after.frontier.clone(),
                None => self.prospective_absent_frontier(
                    before.as_ref(),
                    author.batch_id,
                    &parts.prospective_documents,
                )?,
            };
            pages.insert(
                page_id,
                DraftProjectionPage {
                    before,
                    after,
                    post_frontier,
                },
            );
        }
        let requirements = projection_requirements(&pages)?;
        if origin == BatchOrigin::LocalMutation && requirements.is_empty() {
            // This is valid only for a semantic transaction whose exact
            // projection transition set is empty. Closed-set acceptance
            // independently derives and verifies the same condition.
        }
        Ok(AuthorTransactionDraft {
            author,
            origin,
            generation,
            root_token,
            prepared_core: parts.prepared,
            semantic_effect: parts.semantic_effect,
            portable_path_root: parts.portable_path_root,
            prospective_documents: parts.prospective_documents,
            requirements,
            pages,
        })
    }

    pub fn finalize_author_transaction(
        &self,
        draft: AuthorTransactionDraft,
        source: ProjectionEndpointBinding,
        mut captured_inputs: Vec<CapabilityCapturedProjectionInput>,
    ) -> Result<PreparedBatch, EngineError> {
        if source.device_id != draft.author.author_device_id {
            return Err(EngineError::ProjectionManifest(
                "source endpoint device does not match batch author".into(),
            ));
        }
        if captured_inputs.iter().any(|input| input.endpoint != source) {
            return Err(EngineError::ProjectionManifest(
                "captured projection input is not bound to the source graph capability".into(),
            ));
        }
        if draft.generation != self.history_generation
            || draft.root_token != self.author_generation_root()?
        {
            return Err(EngineError::AuthorDraftStale);
        }
        captured_inputs.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        if !captured_inputs
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        {
            return Err(EngineError::ProjectionManifest(
                "captured projection inputs contain a duplicate path".into(),
            ));
        }
        let expected_paths = draft
            .requirements
            .iter()
            .flat_map(|requirement| {
                std::iter::once(requirement.path.clone())
                    .chain(requirement.render_base_path.iter().cloned())
            })
            .collect::<BTreeSet<_>>();
        let actual_paths = captured_inputs
            .iter()
            .map(|input| input.path.clone())
            .collect::<BTreeSet<_>>();
        if actual_paths != expected_paths {
            return Err(EngineError::ProjectionManifest(
                "captured projection inputs do not exactly cover requirements".into(),
            ));
        }
        let inputs = captured_inputs
            .into_iter()
            .map(|input| (input.path, input.state))
            .collect::<BTreeMap<_, _>>();

        let mut objects = draft.prepared_core.objects().to_vec();
        let mut bases =
            BTreeMap::<ManagedPath, (ManifestObjectRef, AnnotatedProjectionBase)>::new();
        for path in &expected_paths {
            let state = &inputs[path];
            let requirement = draft
                .requirements
                .iter()
                .find(|requirement| {
                    requirement.path == *path || requirement.render_base_path.as_ref() == Some(path)
                })
                .expect("expected path came from a requirement");
            let page = &draft.pages[&requirement.page_id];
            match state {
                CapabilityCapturedProjectionState::Absent => {
                    let path_requires_present = draft.requirements.iter().any(|candidate| {
                        (candidate.path == *path
                            && candidate.precondition == ProjectionRequirementState::Present)
                            || candidate.render_base_path.as_ref() == Some(path)
                    });
                    if path_requires_present {
                        return Err(EngineError::ProjectionManifest(format!(
                            "captured path {path} is absent but an exact base is required"
                        )));
                    }
                }
                CapabilityCapturedProjectionState::Present {
                    bytes,
                    prior_intent,
                    prior_completion,
                } => {
                    prior_completion
                        .validate_against(prior_intent)
                        .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    if prior_intent.workspace_id() != self.workspace_id
                        || prior_intent.page_id() != requirement.page_id
                        || prior_intent.path() != path
                        || prior_intent.frontier()
                            != page
                                .before
                                .as_ref()
                                .map(|before| &before.frontier)
                                .ok_or_else(|| {
                                    EngineError::ProjectionManifest(format!(
                                        "captured path {path} is present without semantic pre-state"
                                    ))
                                })?
                        || prior_intent.claim_evidence()
                            != page
                                .before
                                .as_ref()
                                .map(|before| before.claim_evidence.as_slice())
                                .unwrap_or_default()
                        || prior_intent.target() != super::BlobDescription::of(bytes)
                    {
                        return Err(EngineError::ProjectionManifest(format!(
                            "captured path {path} completion is not its intended semantic predecessor"
                        )));
                    }
                    let before = page.before.as_ref().ok_or_else(|| {
                        EngineError::ProjectionManifest(format!(
                            "captured path {path} is present without semantic pre-state"
                        ))
                    })?;
                    if before.page.path != *path {
                        return Err(EngineError::ProjectionManifest(format!(
                            "captured path {path} is not the semantic source path"
                        )));
                    }
                    let replay =
                        super::projection::plan_projection(self.workspace_id, before, Some(bytes))
                            .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    if replay.target() != bytes {
                        return Err(EngineError::ProjectionManifest(format!(
                            "captured path {path} is not the exact semantic pre-state"
                        )));
                    }
                    let base = AnnotatedProjectionBase::new(
                        self.workspace_id,
                        source.endpoint_id,
                        before.page.page_id,
                        path.clone(),
                        Some(prior_completion.logical_completion_id()),
                        before.frontier.clone(),
                        bytes.clone(),
                        replay.intent().annotations().to_vec(),
                        before.claim_evidence.clone(),
                    )
                    .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    let payload = base
                        .encode()
                        .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    let document_id = base
                        .descriptor_document_id()
                        .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    let object = OperationObject::new(
                        self.workspace_id,
                        document_id,
                        ObjectKind::AnnotatedBaseBlob,
                        payload,
                    )?;
                    let reference = ManifestObjectRef::from_descriptor(&object.descriptor()?);
                    bases.insert(path.clone(), (reference, base));
                    objects.push(object);
                }
            }
        }

        for requirement in &draft.requirements {
            let page = &draft.pages[&requirement.page_id];
            let precondition = match requirement.precondition {
                ProjectionRequirementState::Absent => ManifestProjectionPrecondition::Absent,
                ProjectionRequirementState::Present => ManifestProjectionPrecondition::Present {
                    base: bases
                        .get(&requirement.path)
                        .ok_or_else(|| {
                            EngineError::ProjectionManifest(
                                "required precondition base was not captured".into(),
                            )
                        })?
                        .0
                        .clone(),
                },
            };
            let render_base = requirement
                .render_base_path
                .as_ref()
                .map(|path| {
                    bases
                        .get(path)
                        .map(|(reference, _)| reference.clone())
                        .ok_or_else(|| {
                            EngineError::ProjectionManifest(
                                "required rename render base was not captured".into(),
                            )
                        })
                })
                .transpose()?;
            let target = match requirement.target {
                ProjectionRequirementState::Absent => ManifestProjectionTarget::Absent,
                ProjectionRequirementState::Present => {
                    let after = page.after.as_ref().ok_or_else(|| {
                        EngineError::ProjectionManifest(
                            "Present requirement has no semantic post-state".into(),
                        )
                    })?;
                    let render_bytes = requirement
                        .render_base_path
                        .as_ref()
                        .or_else(|| {
                            (requirement.precondition == ProjectionRequirementState::Present)
                                .then_some(&requirement.path)
                        })
                        .and_then(|path| match &inputs[path] {
                            CapabilityCapturedProjectionState::Present { bytes, .. } => {
                                Some(bytes.as_slice())
                            }
                            CapabilityCapturedProjectionState::Absent => None,
                        });
                    let plan =
                        super::projection::plan_projection(self.workspace_id, after, render_bytes)
                            .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    ManifestProjectionTarget::present(
                        plan.target().to_vec(),
                        plan.intent().annotations().to_vec(),
                    )
                    .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?
                }
            };
            let claim_evidence = match requirement.target {
                ProjectionRequirementState::Absent => Vec::new(),
                ProjectionRequirementState::Present => page
                    .after
                    .as_ref()
                    .map(|after| after.claim_evidence.clone())
                    .unwrap_or_default(),
            };
            let intent = ManifestedProjectionIntent::new(
                self.workspace_id,
                draft.author.batch_id,
                draft.author.author_device_id,
                draft.author.author_session_id,
                source.endpoint_id,
                requirement.page_id,
                requirement.path.clone(),
                draft.portable_path_root,
                precondition,
                render_base,
                target,
                page.post_frontier.clone(),
                claim_evidence,
            )
            .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
            let object = OperationObject::new(
                self.workspace_id,
                intent.descriptor_document_id(),
                ObjectKind::ProjectionIntent,
                intent
                    .encode()
                    .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?,
            )?;
            objects.push(object);
        }
        objects.sort_unstable_by_key(|object| {
            object
                .descriptor()
                .expect("bounded finalized object remains encodable")
        });
        objects.dedup_by(|left, right| {
            left.descriptor().expect("encodable").content_digest()
                == right.descriptor().expect("encodable").content_digest()
        });
        let descriptors = objects
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()?;
        let core = draft.prepared_core.manifest();
        let manifest = OperationBatch::new_with_causality(
            core.workspace_id(),
            core.lineage_digest(),
            core.batch_id(),
            core.author_device_id(),
            core.author_session_id(),
            draft.origin,
            core.causal_dot(),
            core.causal_dependency_heads().to_vec(),
            core.dependency_frontier().clone(),
            core.semantic_effect_digest(),
            descriptors,
        )?;
        let prepared = PreparedBatch::new(manifest, objects)?;
        if self.scratch.is_none() {
            *self.pending_author_documents.borrow_mut() = Some(PendingAuthorDocuments {
                batch_id: draft.author.batch_id,
                manifest_fingerprint: prepared_manifest_fingerprint(&prepared),
                documents: draft.prospective_documents,
            });
        }
        let _ = draft.semantic_effect;
        Ok(prepared)
    }

    fn prepare_transaction_core(
        &self,
        author: AuthorBatch,
        origin: super::BatchOrigin,
        transaction: &OperationTransaction,
        capture_prospective_documents: bool,
    ) -> Result<PreparedTransactionParts, EngineError> {
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
        validate_logseq_identity_mutation_shape(transaction)?;

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
        self.validate_logseq_identity_triggers(transaction, &working)?;

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
                origin,
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
                origin,
                BatchCausalDot::new(peer, counter)?,
                causal_dependency_heads,
                frontier,
                SemanticEffectDigest::of(&effect_bytes),
                descriptors,
            )?
        };
        let portable_path_root = if !effect.pages().is_empty() {
            let catalog = working
                .get(&self.catalog_document_id)
                .ok_or_else(|| {
                    EngineError::InvalidTransaction(
                        "page effect has no prospective catalog document".into(),
                    )
                })?
                .document();
            let prospective_pages = validate_catalog(self.catalog_document_id, catalog)?;
            let candidate = self.prepare_portable_path_updates(
                &self.scratch_roots,
                author.batch_id,
                manifest.causal_dot(),
                manifest.dependency_frontier(),
                &effect,
                Some(&prospective_pages),
                true,
            )?;
            if !candidate.conflicts.is_empty() {
                return Err(EngineError::InvalidTransaction(
                    "locally authored transaction would create a portable path conflict".into(),
                ));
            }
            candidate.root
        } else {
            self.portable_path_root
        };
        let prepared = PreparedBatch::new(manifest, objects).map_err(EngineError::from)?;
        let prospective_documents = if capture_prospective_documents {
            working
                .iter()
                .map(|(document_id, document)| {
                    clone_doc(document.document(), 1).map(|copy| (*document_id, copy))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?
        } else {
            BTreeMap::new()
        };
        if self.scratch.is_none() && !capture_prospective_documents {
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
        Ok(PreparedTransactionParts {
            prepared,
            semantic_effect: effect,
            prospective_documents,
            portable_path_root,
        })
    }

    fn author_generation_root(&self) -> Result<ContentDigest, EngineError> {
        let mut bytes = postcard::to_allocvec(&self.scratch_roots)
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        bytes.extend_from_slice(&super::PORTABLE_PATH_KEY_VERSION.to_be_bytes());
        bytes.extend_from_slice(self.portable_path_root.digest().as_bytes());
        bytes.extend_from_slice(&self.history_generation.to_be_bytes());
        bytes.extend_from_slice(self.history_root.as_bytes());
        for (document_id, heads) in &self.visible_document_heads {
            bytes.extend_from_slice(document_id.as_uuid().as_bytes());
            for head in heads {
                bytes.extend_from_slice(head.as_uuid().as_bytes());
            }
        }
        for (batch_id, fingerprint) in &self.archive_fingerprints {
            bytes.extend_from_slice(batch_id.as_uuid().as_bytes());
            bytes.extend_from_slice(fingerprint.as_bytes());
        }
        Ok(ContentDigest::of(&bytes))
    }

    fn catalog_checkpoint_binding(&self) -> ContentDigest {
        let bytes = postcard::to_allocvec(&(
            super::PORTABLE_PATH_KEY_VERSION,
            &self.scratch_roots.external_document_current_root,
            &self.scratch_roots.external_document_state_root,
            self.visible_document_heads
                .get(&self.catalog_document_id)
                .cloned()
                .unwrap_or_default(),
        ))
        .expect("catalog checkpoint binding has an infallible canonical encoding");
        ContentDigest::of(&bytes)
    }

    fn durable_history_binding(&self) -> super::object_store::EngineHistoryBinding {
        super::object_store::EngineHistoryBinding {
            portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
            portable_path_root: self.portable_path_root.digest(),
            catalog_checkpoint_binding: self.catalog_checkpoint_binding(),
            portable_path_conflicts: self.portable_path_conflicts.values().cloned().collect(),
            terminal_evidence: self.fatal_handle.map(|handle| {
                super::object_store::EngineTerminalEvidenceBinding {
                    conflict_root: handle.conflict_root,
                    conflict_count: handle.conflicting_block_count,
                    participant_count: handle.claim_count,
                    canonical_digest: handle.canonical_digest,
                }
            }),
        }
    }

    fn prospective_document(
        &self,
        document_id: DocumentId,
        prospective: &BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<LoroDoc, EngineError> {
        match prospective.get(&document_id) {
            Some(document) => clone_doc(document, 1),
            None => self.clone_visible_document(document_id, 1),
        }
    }

    fn prospective_document_dependencies(
        &self,
        document_id: DocumentId,
        document: &LoroDoc,
        batch_id: BatchId,
        prospective: &BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<DocumentDependencies, EngineError> {
        if prospective.contains_key(&document_id) {
            DocumentDependencies::new(
                document_id,
                canonical_peer_counters(&document.oplog_vv())?,
                vec![batch_id],
            )
            .map_err(Into::into)
        } else {
            self.current_document_dependencies(document_id, document)
        }
    }

    fn prospective_projection_page(
        &self,
        page_id: PageId,
        batch_id: BatchId,
        prospective: &BTreeMap<DocumentId, LoroDoc>,
        effect: &SemanticEffect,
    ) -> Result<Option<ProjectionPageState>, EngineError> {
        let catalog = self.prospective_document(self.catalog_document_id, prospective)?;
        let Some(page_state) = validate_catalog_page(self.catalog_document_id, &catalog, page_id)?
        else {
            return Ok(None);
        };
        let PageState::Live {
            home_document_id: page_document_id,
            ..
        } = page_state
        else {
            return Ok(None);
        };
        let page_document = self.prospective_document(page_document_id, prospective)?;
        let members = read_memberships(page_document_id, &page_document)?;
        let mut documents = BTreeMap::from([
            (self.catalog_document_id, catalog),
            (page_document_id, page_document),
        ]);
        for claim in members.values() {
            if !documents.contains_key(&claim.home_document_id) {
                documents.insert(
                    claim.home_document_id,
                    self.prospective_document(claim.home_document_id, prospective)?,
                );
            }
        }
        let page = self.materialize_page_from_documents(page_id, &documents)?;
        let mut requested =
            page_logseq_references(&page.path, page.preamble.as_deref(), &page.blocks);
        requested.extend(page.blocks.iter().filter_map(|block| block.logseq_uuid));
        let introduced = effect
            .blocks()
            .iter()
            .filter_map(|delta| {
                let before_uuid = delta.before.as_ref().and_then(|state| state.logseq_uuid);
                let after_uuid = delta.after.as_ref().and_then(|state| state.logseq_uuid);
                (after_uuid != before_uuid)
                    .then_some(after_uuid)
                    .flatten()
                    .map(|uuid| {
                        (
                            uuid,
                            ProjectionClaimParticipant::new(delta.block_id, delta.home_document_id),
                        )
                    })
            })
            .collect::<BTreeMap<_, _>>();
        let mut claim_evidence = Vec::new();
        for logseq_uuid in requested {
            let record = self.logseq_claim_record(self.logseq_claim_root, logseq_uuid)?;
            let mut participants = record
                .introductions
                .iter()
                .map(|claim| {
                    ProjectionClaimParticipant::new(claim.block_id, claim.home_document_id)
                })
                .collect::<BTreeSet<_>>();
            if let Some(participant) = introduced.get(&logseq_uuid) {
                participants.insert(*participant);
            }
            if participants.is_empty() {
                continue;
            }
            for participant in &participants {
                let document_id = participant.home_document_id();
                if !documents.contains_key(&document_id) {
                    documents.insert(
                        document_id,
                        self.prospective_document(document_id, prospective)?,
                    );
                }
            }
            let evidence =
                ProjectionClaimEvidence::new(logseq_uuid, participants.into_iter().collect())?;
            match self.resolve_logseq_uuid_from_documents(logseq_uuid, &evidence, &documents)? {
                LogseqUuidResolution::Ambiguous { claim_count } => {
                    return Err(EngineError::AmbiguousLogseqUuid {
                        logseq_uuid,
                        claim_count,
                    })
                }
                LogseqUuidResolution::Unclaimed | LogseqUuidResolution::Unique(_) => {}
            }
            claim_evidence.push(evidence);
        }
        claim_evidence.sort_unstable_by_key(ProjectionClaimEvidence::logseq_uuid);
        let frontier = FrontierV2::new(
            documents
                .iter()
                .map(|(document_id, document)| {
                    self.prospective_document_dependencies(
                        *document_id,
                        document,
                        batch_id,
                        prospective,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )?;
        Ok(Some(ProjectionPageState {
            page,
            frontier,
            claim_evidence,
        }))
    }

    fn prospective_absent_frontier(
        &self,
        before: Option<&ProjectionPageState>,
        batch_id: BatchId,
        prospective: &BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<FrontierV2, EngineError> {
        let Some(before) = before else {
            return Ok(FrontierV2::new(Vec::new())?);
        };
        FrontierV2::new(
            before
                .frontier
                .documents()
                .iter()
                .map(|dependencies| {
                    let document_id = dependencies.document_id();
                    let document = self.prospective_document(document_id, prospective)?;
                    self.prospective_document_dependencies(
                        document_id,
                        &document,
                        batch_id,
                        prospective,
                    )
                })
                .collect::<Result<Vec<_>, EngineError>>()?,
        )
        .map_err(Into::into)
    }

    fn validate_manifested_projection_transition(
        &self,
        batch_id: BatchId,
        effect: &SemanticEffect,
        after: &BTreeMap<DocumentId, EngineDocument>,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
    ) -> Result<(), EngineError> {
        let batch = &self.archive[&batch_id];
        let projection = super::projection_manifest::validate_projection_object_set(
            batch.manifest(),
            batch.objects(),
        )
        .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
        let affected = affected_projection_pages(effect);
        if projection.intents().is_empty() {
            return match batch.manifest().origin() {
                BatchOrigin::BootstrapImport => Ok(()),
                BatchOrigin::LocalMutation if affected.is_empty() => Ok(()),
                BatchOrigin::LocalMutation | BatchOrigin::ExternalReconciliation { .. } => {
                    Err(EngineError::ProjectionManifest(
                        "origin requires complete affected-path projection intents".into(),
                    ))
                }
            };
        }
        if batch.manifest().origin() == BatchOrigin::BootstrapImport {
            // A bootstrap is allowed to carry complete intents once a live
            // endpoint exists, but any carried set is validated identically.
        }
        let endpoints = projection
            .intents()
            .iter()
            .map(ManifestedProjectionIntent::source_endpoint_id)
            .collect::<BTreeSet<_>>();
        if endpoints.len() != 1 {
            return Err(EngineError::ProjectionManifest(
                "one batch cannot bind more than one source endpoint".into(),
            ));
        }
        let intent_pages = projection
            .intents()
            .iter()
            .map(ManifestedProjectionIntent::page_id)
            .collect::<BTreeSet<_>>();
        if intent_pages != affected {
            return Err(EngineError::ProjectionManifest(
                "projection intents do not exactly cover affected pages".into(),
            ));
        }
        for page_id in &affected {
            let intents = projection
                .intents()
                .iter()
                .filter(|intent| intent.page_id() == *page_id)
                .collect::<Vec<_>>();
            validate_intent_directions(effect, *page_id, &intents)?;
        }

        for intent in projection.intents() {
            let documents =
                self.manifest_post_documents(intent.post_frontier(), batch_id, after, updates)?;
            let page = match self.materialize_page_from_documents(intent.page_id(), &documents) {
                Ok(page) => Some(page),
                Err(EngineError::PageNotFound(_) | EngineError::PageDeleted(_)) => None,
                Err(error) => return Err(error),
            };
            match intent.target() {
                ManifestProjectionTarget::Absent => {
                    if !intent.claim_evidence().is_empty() {
                        return Err(EngineError::ProjectionManifest(
                            "Absent target carries claim evidence".into(),
                        ));
                    }
                    if page
                        .as_ref()
                        .is_some_and(|page| page.path == *intent.path())
                    {
                        return Err(EngineError::ProjectionManifest(
                            "Absent target path remains present in semantic post-state".into(),
                        ));
                    }
                }
                ManifestProjectionTarget::Present {
                    bytes, annotations, ..
                } => {
                    let page = page.ok_or_else(|| {
                        EngineError::ProjectionManifest(
                            "Present target page is absent from semantic post-state".into(),
                        )
                    })?;
                    if page.path != *intent.path() {
                        return Err(EngineError::ProjectionManifest(
                            "Present target path does not match semantic post-state".into(),
                        ));
                    }
                    let expected_evidence = self.projection_claim_evidence_after_batch(
                        batch.manifest().dependency_frontier(),
                        effect,
                        &page,
                        &documents,
                    )?;
                    if expected_evidence != intent.claim_evidence() {
                        return Err(EngineError::ProjectionClaimEvidenceMismatch);
                    }
                    let required_documents = projection_page_document_ids(
                        self.catalog_document_id,
                        &page,
                        &expected_evidence,
                        &documents,
                    )?;
                    let declared_documents = intent
                        .post_frontier()
                        .documents()
                        .iter()
                        .map(DocumentDependencies::document_id)
                        .collect::<BTreeSet<_>>();
                    if required_documents != declared_documents {
                        return Err(EngineError::ProjectionManifest(
                            "projection post frontier has missing or extra documents".into(),
                        ));
                    }
                    let render_bytes = intent
                        .render_base()
                        .or_else(|| intent.precondition().base())
                        .map(|reference| {
                            projection
                                .bases()
                                .get(&reference.document_id())
                                .map(AnnotatedProjectionBase::bytes)
                                .ok_or_else(|| {
                                    EngineError::ProjectionManifest(
                                        "render base object is unavailable".into(),
                                    )
                                })
                        })
                        .transpose()?;
                    let state = ProjectionPageState {
                        page,
                        frontier: intent.post_frontier().clone(),
                        claim_evidence: expected_evidence,
                    };
                    let plan =
                        super::projection::plan_projection(self.workspace_id, &state, render_bytes)
                            .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
                    if plan.target() != bytes || plan.intent().annotations() != annotations {
                        return Err(EngineError::ProjectionManifest(
                            "projection target bytes/annotations do not match semantic post-state"
                                .into(),
                        ));
                    }
                }
            }
            for reference in intent
                .precondition()
                .base()
                .into_iter()
                .chain(intent.render_base())
            {
                let base = projection
                    .bases()
                    .get(&reference.document_id())
                    .ok_or_else(|| {
                        EngineError::ProjectionManifest("referenced base is unavailable".into())
                    })?;
                self.validate_manifested_base(base)?;
            }
        }
        Ok(())
    }

    fn validate_manifested_portable_path_binding(
        &self,
        batch_id: BatchId,
        candidate_root: PortablePathIndexRoot,
        terminal_conflict: bool,
    ) -> Result<(), EngineError> {
        let batch = &self.archive[&batch_id];
        let projection = super::projection_manifest::validate_projection_object_set(
            batch.manifest(),
            batch.objects(),
        )
        .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
        for intent in projection.intents() {
            if intent.portable_path_key_version() != super::PORTABLE_PATH_KEY_VERSION
                || intent.portable_path_key_digest() != intent.path().portable_key().digest()
                || (!terminal_conflict && intent.portable_path_index_root() != candidate_root)
            {
                return Err(EngineError::ProjectionManifest(
                    "projection intent portable-path index binding mismatch".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_manifested_base(&self, base: &AnnotatedProjectionBase) -> Result<(), EngineError> {
        let state = match self.authorize_projection_recovery(
            base.source_page_id(),
            base.prior_frontier(),
            base.claim_evidence(),
        ) {
            Ok(authorization) => authorization.into_state(),
            Err(EngineError::ProjectionAuthorizationUnavailable) if self.scratch.is_none() => {
                self.materialize_page_for_projection(base.source_page_id())?
            }
            Err(error) => return Err(error),
        };
        if state.page.path != *base.source_path()
            || state.frontier != *base.prior_frontier()
            || state.claim_evidence != base.claim_evidence()
        {
            return Err(EngineError::ProjectionManifest(
                "annotated base prior binding does not match exact semantic state".into(),
            ));
        }
        let replay =
            super::projection::plan_projection(self.workspace_id, &state, Some(base.bytes()))
                .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
        if replay.target() != base.bytes() || replay.intent().annotations() != base.annotations() {
            return Err(EngineError::ProjectionManifest(
                "annotated base bytes/annotations are not the exact semantic pre-state".into(),
            ));
        }
        Ok(())
    }

    fn manifest_post_documents(
        &self,
        frontier: &FrontierV2,
        batch_id: BatchId,
        after: &BTreeMap<DocumentId, EngineDocument>,
        updates: &BTreeMap<DocumentId, CrdtUpdatePayload>,
    ) -> Result<BTreeMap<DocumentId, LoroDoc>, EngineError> {
        let accepted = FrontierV2::new(
            frontier
                .documents()
                .iter()
                .filter(|dependencies| {
                    dependencies
                        .direct_dependency_heads()
                        .binary_search(&batch_id)
                        .is_err()
                })
                .cloned()
                .collect(),
        )?;
        let mut documents = if accepted.documents().is_empty() {
            BTreeMap::new()
        } else {
            self.reconstruct_projection_frontier(&accepted)?
        };
        for dependencies in frontier.documents() {
            let has_current = dependencies
                .direct_dependency_heads()
                .binary_search(&batch_id)
                .is_ok();
            if !has_current {
                continue;
            }
            if dependencies.direct_dependency_heads() != [batch_id]
                || !updates.contains_key(&dependencies.document_id())
            {
                return Err(EngineError::ProjectionManifest(
                    "post frontier current-batch head is not exact".into(),
                ));
            }
            let document = after
                .get(&dependencies.document_id())
                .ok_or_else(|| {
                    EngineError::ProjectionManifest(
                        "post frontier current-batch document is missing".into(),
                    )
                })?
                .document();
            if canonical_peer_counters(&document.oplog_vv())? != dependencies.peer_counters() {
                return Err(EngineError::FrontierVectorMismatch(
                    dependencies.document_id(),
                ));
            }
            documents.insert(dependencies.document_id(), clone_doc(document, 1)?);
        }
        Ok(documents)
    }

    fn projection_claim_evidence_after_batch(
        &self,
        dependency_frontier: &FrontierV2,
        effect: &SemanticEffect,
        page: &MaterializedPage,
        documents: &BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<Vec<ProjectionClaimEvidence>, EngineError> {
        let mut requested =
            page_logseq_references(&page.path, page.preamble.as_deref(), &page.blocks);
        requested.extend(page.blocks.iter().filter_map(|block| block.logseq_uuid));
        let mut additions = BTreeMap::<LogseqUuid, BTreeSet<ProjectionClaimParticipant>>::new();
        for delta in effect.blocks() {
            let before = delta.before.as_ref().and_then(|state| state.logseq_uuid);
            let after = delta.after.as_ref().and_then(|state| state.logseq_uuid);
            if before != after {
                if let Some(uuid) = after {
                    additions
                        .entry(uuid)
                        .or_default()
                        .insert(ProjectionClaimParticipant::new(
                            delta.block_id,
                            delta.home_document_id,
                        ));
                }
            }
        }
        let mut evidence = Vec::new();
        for uuid in requested {
            let mut participants: BTreeSet<ProjectionClaimParticipant> = self
                .logseq_claim_evidence_at_frontier(uuid, dependency_frontier)?
                .map(|evidence| evidence.participants().iter().copied().collect())
                .unwrap_or_default();
            participants.extend(additions.remove(&uuid).unwrap_or_default());
            if participants.is_empty() {
                continue;
            }
            let entry = ProjectionClaimEvidence::new(uuid, participants.into_iter().collect())?;
            match self.resolve_logseq_uuid_from_documents(uuid, &entry, documents)? {
                LogseqUuidResolution::Ambiguous { claim_count } => {
                    return Err(EngineError::AmbiguousLogseqUuid {
                        logseq_uuid: uuid,
                        claim_count,
                    })
                }
                LogseqUuidResolution::Unclaimed | LogseqUuidResolution::Unique(_) => {}
            }
            evidence.push(entry);
        }
        evidence.sort_unstable_by_key(ProjectionClaimEvidence::logseq_uuid);
        Ok(evidence)
    }

    fn prepare_projection_work(&self, batch_id: BatchId) -> Result<(), EngineError> {
        self.prepare_projection_work_for_batch(&self.archive[&batch_id])
    }

    fn prepare_projection_work_for_batch(&self, batch: &ValidatedBatch) -> Result<(), EngineError> {
        let (Some(endpoint), Some(index)) = (
            self.projection_endpoint,
            self.projection_work_index.as_ref(),
        ) else {
            return Ok(());
        };
        let batch_id = batch.manifest().batch_id();
        let projection = super::projection_manifest::validate_projection_object_set(
            batch.manifest(),
            batch.objects(),
        )
        .map_err(|error| EngineError::ProjectionManifest(error.to_string()))?;
        let mut work = Vec::new();
        let mut superseded = Vec::new();
        for intent in projection
            .intents()
            .iter()
            .filter(|intent| intent.source_endpoint_id() == endpoint.endpoint_id)
        {
            if intent.source_author_device_id() != endpoint.device_id {
                return Err(EngineError::ProjectionManifest(
                    "source endpoint is not bound to the author device".into(),
                ));
            }
            let descriptor = batch
                .manifest()
                .required_objects()
                .iter()
                .find(|descriptor| {
                    descriptor.kind() == ObjectKind::ProjectionIntent
                        && descriptor.document_id() == intent.descriptor_document_id()
                })
                .ok_or_else(|| {
                    EngineError::ProjectionManifest(
                        "projection intent descriptor disappeared".into(),
                    )
                })?;
            let target = intent
                .target()
                .description()
                .map_or(ProjectionWorkTarget::Absent, ProjectionWorkTarget::Present);
            let row = ProjectionWork::new(
                self.workspace_id,
                endpoint.endpoint_id,
                endpoint.graph_resource_id,
                batch_id,
                intent.page_id(),
                intent.path().clone(),
                intent.portable_path_index_root(),
                ManifestObjectRef::from_descriptor(descriptor),
                intent.post_frontier().clone(),
                target,
            );
            for older in index
                .pending_for_path(intent.path())
                .map_err(|error| EngineError::ProjectionWork(error.to_string()))?
            {
                if older.work_id() == row.work_id() {
                    continue;
                }
                if self
                    .projection_frontier_dominates(intent.post_frontier(), older.post_frontier())?
                {
                    superseded.push(older.work_id());
                }
            }
            work.push(row);
        }
        superseded.sort_unstable();
        superseded.dedup();
        index
            .prepare_batch(batch_id, batch_fingerprint(batch), &work, &superseded)
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))
    }

    fn activate_projection_work(
        &self,
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
    ) -> Result<(), EngineError> {
        let Some(index) = self.projection_work_index.as_ref() else {
            return Ok(());
        };
        let authority = self
            .history_store
            .as_ref()
            .ok_or_else(|| {
                EngineError::ProjectionWork(
                    "projection activation has no durable engine history".into(),
                )
            })?
            .current_authority()
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        index
            .accept_batch_at_history(
                batch_id,
                manifest_fingerprint,
                authority.generation,
                authority.index_root,
            )
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))
    }

    fn reconcile_pending_projection_work(&mut self) -> Result<(), EngineError> {
        let Some(index) = self.projection_work_index.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        self.begin_point_operation();
        let authority = self
            .history_store
            .as_ref()
            .ok_or_else(|| {
                EngineError::ProjectionWork(
                    "projection reconciliation has no durable engine history".into(),
                )
            })?
            .current_authority()
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        let mut cursor = None;
        loop {
            let page = index
                .pending_activation_page(cursor.as_ref(), 256)
                .map_err(|error| EngineError::ProjectionWork(error.to_string()))?;
            for pending in page.pending() {
                match self.durable_endpoint_history_record(pending.batch_id())? {
                    Some(record) => {
                        if record.manifest_fingerprint != pending.manifest_fingerprint() {
                            return Err(EngineError::ProjectionWork(format!(
                                "pending projection batch {} does not match durable engine history",
                                pending.batch_id()
                            )));
                        }
                        match record.status {
                            ArchiveStatus::Accepted { .. } => index
                                .accept_batch_at_history(
                                    pending.batch_id(),
                                    pending.manifest_fingerprint(),
                                    authority.generation,
                                    authority.index_root,
                                )
                                .map_err(|error| EngineError::ProjectionWork(error.to_string()))?,
                            ArchiveStatus::Rejected(_)
                            | ArchiveStatus::Quarantined
                            | ArchiveStatus::Staged => index
                                .retire_pending_activation_at_history(
                                    pending,
                                    authority.generation,
                                    authority.index_root,
                                )
                                .map_err(|error| EngineError::ProjectionWork(error.to_string()))?,
                        }
                    }
                    None => index
                        .retire_pending_activation_at_history(
                            pending,
                            authority.generation,
                            authority.index_root,
                        )
                        .map_err(|error| EngineError::ProjectionWork(error.to_string()))?,
                }
            }
            cursor = page.next().cloned();
            if cursor.is_none() {
                break;
            }
        }
        index
            .require_current_history_binding(authority.generation, authority.index_root)
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))?;
        Ok(())
    }

    pub(crate) fn authorize_projection_work(
        &mut self,
        index: &ProjectionWorkIndex,
        work: &ProjectionWork,
    ) -> Result<(), EngineError> {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let endpoint = self.projection_endpoint.ok_or_else(|| {
            EngineError::ProjectionWork("engine has no enrolled projection endpoint".into())
        })?;
        if endpoint.endpoint_id != work.endpoint_id()
            || endpoint.graph_resource_id != work.graph_resource_id()
            || index.workspace_id() != self.workspace_id
            || index.endpoint_id() != endpoint.endpoint_id
            || index.graph_resource_id() != endpoint.graph_resource_id
            || Some(index.receipt_store_id()) != self.projection_receipt_store_id
            || work.workspace_id() != self.workspace_id
        {
            return Err(EngineError::ProjectionWork(
                "projection work endpoint/workspace binding mismatch".into(),
            ));
        }
        let authority = self
            .history_store
            .as_ref()
            .ok_or_else(|| {
                EngineError::ProjectionWork(
                    "projection authorization has no durable engine history".into(),
                )
            })?
            .current_authority()
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        index
            .require_current_history_binding(authority.generation, authority.index_root)
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))?;
        let record = self
            .durable_endpoint_history_record(work.batch_id())?
            .ok_or_else(|| {
                EngineError::ProjectionWork(
                    "projection work batch has no authenticated durable status".into(),
                )
            })?;
        if !matches!(record.status, ArchiveStatus::Accepted { .. }) {
            return Err(EngineError::ProjectionWork(
                "projection work batch is not accepted durable state".into(),
            ));
        }
        if self.visible_documents.is_empty() {
            self.portable_path_index
                .as_ref()
                .ok_or_else(|| {
                    EngineError::ProjectionWork(
                        "projection authorization has no portable-path index".into(),
                    )
                })?
                .validate_root(record.portable_path_root)
                .map_err(|error| EngineError::ProjectionWork(error.to_string()))?;
            self.portable_path_root = record.portable_path_root;
        }
        if record.portable_path_key_version != super::PORTABLE_PATH_KEY_VERSION
            || record.portable_path_root != work.portable_path_index_root()
            || work.portable_path_key_version() != super::PORTABLE_PATH_KEY_VERSION
            || work.portable_path_key_digest() != work.path().portable_key().digest()
        {
            return Err(EngineError::ProjectionWork(
                "projection work portable-path history binding mismatch".into(),
            ));
        }
        let current = self
            .portable_path_records_many(&[work.portable_path_key_digest()])?
            .remove(&work.portable_path_key_digest());
        let currently_owned = current.as_ref().and_then(PortablePathRecord::occupied);
        match work.target() {
            ProjectionWorkTarget::Present(_)
                if currently_owned.is_none_or(|occupied| {
                    occupied.page_id() != work.page_id() || occupied.exact_path() != work.path()
                }) =>
            {
                return Err(EngineError::ProjectionWork(
                    "projection work path is not currently owned by its page".into(),
                ))
            }
            ProjectionWorkTarget::Absent if currently_owned.is_some() => {
                return Err(EngineError::ProjectionWork(
                    "projection deletion path is currently owned".into(),
                ))
            }
            ProjectionWorkTarget::Absent | ProjectionWorkTarget::Present(_) => {}
        }
        index
            .require_accepted_ready(work, record.manifest_fingerprint)
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))
    }

    pub(crate) fn authorize_projected_release(
        &self,
        index: &ProjectionWorkIndex,
        release: &PortablePathReleased,
    ) -> Result<ProjectionWork, EngineError> {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let endpoint = self.projection_endpoint.ok_or_else(|| {
            EngineError::ProjectionWork("engine has no enrolled projection endpoint".into())
        })?;
        let receipt_store_id = self.projection_receipt_store_id.ok_or_else(|| {
            EngineError::ProjectionWork("engine has no enrolled projection receipt store".into())
        })?;
        if index.workspace_id() != self.workspace_id
            || index.endpoint_id() != endpoint.endpoint_id
            || index.graph_resource_id() != endpoint.graph_resource_id
            || index.receipt_store_id() != receipt_store_id
        {
            return Err(EngineError::ProjectionWork(
                "projection release runtime binding mismatch".into(),
            ));
        }
        let history = self.history_store.as_ref().ok_or_else(|| {
            EngineError::ProjectionWork(
                "projection release authorization has no durable engine history".into(),
            )
        })?;
        let (generation, history_root) = history
            .current()
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        index
            .require_current_history_binding(generation, history_root)
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))?;
        let bytes = history
            .lookup(history_root, release.release_batch())
            .map_err(|error| EngineError::Archive(error.to_string()))?
            .ok_or_else(|| {
                EngineError::ProjectionWork(
                    "projection release batch has no authenticated durable status".into(),
                )
            })?;
        let record = decode_history_record(release.release_batch(), &bytes)?;
        if record.generation == 0
            || record.generation > generation
            || !matches!(record.status, ArchiveStatus::Accepted { .. })
        {
            return Err(EngineError::ProjectionWork(
                "projection release batch is not accepted durable state".into(),
            ));
        }
        let key = release.prior_exact_path().portable_key().digest();
        let current = self.portable_path_records_many(&[key])?.remove(&key);
        if current
            .as_ref()
            .and_then(PortablePathRecord::occupied)
            .is_some()
            || current
                .as_ref()
                .and_then(PortablePathRecord::latest_release)
                != Some(release)
        {
            return Err(EngineError::ProjectionWork(
                "projection release is not the authenticated current path fence".into(),
            ));
        }
        index
            .completed_release(
                release.release_batch(),
                record.manifest_fingerprint,
                release.prior_page_id(),
                release.prior_exact_path(),
            )
            .map_err(|error| EngineError::ProjectionWork(error.to_string()))
    }

    fn projection_frontier_dominates(
        &self,
        newer: &FrontierV2,
        older: &FrontierV2,
    ) -> Result<bool, EngineError> {
        for old in older.documents() {
            let Ok(index) = newer
                .documents()
                .binary_search_by_key(&old.document_id(), DocumentDependencies::document_id)
            else {
                return Ok(false);
            };
            let new = &newer.documents()[index];
            for old_counter in old.peer_counters() {
                let Some(new_counter) = new
                    .peer_counters()
                    .iter()
                    .find(|counter| counter.peer_id() == old_counter.peer_id())
                else {
                    return Ok(false);
                };
                if new_counter.max_counter() < old_counter.max_counter() {
                    return Ok(false);
                }
            }
            let ancestry = self.collect_batch_ancestry(
                &new.direct_dependency_heads().iter().copied().collect(),
                self.is_blocked(),
            )?;
            if old
                .direct_dependency_heads()
                .iter()
                .any(|head| !ancestry.contains_key(head))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn materialize_page(&self, page_id: PageId) -> Result<MaterializedPage, EngineError> {
        self.materialize_page_inner(page_id, false)
            .map(|(page, _, _)| page)
    }

    pub fn materialize_page_for_projection(
        &self,
        page_id: PageId,
    ) -> Result<ProjectionPageState, EngineError> {
        let (page, frontier, claim_evidence) = self.materialize_page_inner(page_id, true)?;
        Ok(ProjectionPageState {
            page,
            frontier: frontier.expect("projection materialization requested a frontier"),
            claim_evidence,
        })
    }

    pub fn authorize_projection_write(
        &self,
        page_id: PageId,
    ) -> Result<ProjectionWriteAuthorization, EngineError> {
        let store = self
            .archive_store
            .as_ref()
            .ok_or(EngineError::ProjectionAuthorizationUnavailable)?;
        let state = self.materialize_page_for_projection(page_id)?;
        let mut accepted_heads = 0_usize;
        for document in state.frontier.documents() {
            for batch_id in document.direct_dependency_heads() {
                accepted_heads = accepted_heads.saturating_add(1);
                if !matches!(
                    self.archive_status(*batch_id)?,
                    Some(ArchiveStatus::Accepted { .. })
                ) || !matches!(
                    store
                        .inspect_batch(*batch_id)
                        .map_err(|error| EngineError::Archive(error.to_string()))?,
                    BatchInspection::Ready(_)
                ) {
                    return Err(EngineError::ProjectionFrontierNotDurable(*batch_id));
                }
            }
        }
        if accepted_heads == 0 {
            return Err(EngineError::ProjectionAuthorizationUnavailable);
        }
        Ok(ProjectionWriteAuthorization {
            state,
            claim_root: self.logseq_claim_root,
        })
    }

    pub(crate) fn authorize_projection_recovery(
        &self,
        page_id: PageId,
        frontier: &FrontierV2,
        expected_claim_evidence: &[ProjectionClaimEvidence],
    ) -> Result<ProjectionWriteAuthorization, EngineError> {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let store = self
            .archive_store
            .as_ref()
            .ok_or(EngineError::ProjectionAuthorizationUnavailable)?;
        let documents = self.reconstruct_projection_frontier(frontier)?;
        let page = self.materialize_page_from_documents(page_id, &documents)?;
        let mut requested =
            page_logseq_references(&page.path, page.preamble.as_deref(), &page.blocks);
        requested.extend(page.blocks.iter().filter_map(|block| block.logseq_uuid));
        let mut actual_evidence = Vec::new();
        for logseq_uuid in requested {
            let evidence = self.logseq_claim_evidence_at_frontier(logseq_uuid, frontier)?;
            if let Some(evidence) = evidence {
                let resolution =
                    self.resolve_logseq_uuid_from_documents(logseq_uuid, &evidence, &documents)?;
                if let LogseqUuidResolution::Ambiguous { claim_count } = resolution {
                    return Err(EngineError::AmbiguousLogseqUuid {
                        logseq_uuid,
                        claim_count,
                    });
                }
                actual_evidence.push(evidence);
            }
        }
        if actual_evidence != expected_claim_evidence {
            return Err(EngineError::ProjectionClaimEvidenceMismatch);
        }
        for block in &page.blocks {
            let Some(logseq_uuid) = block.logseq_uuid else {
                continue;
            };
            let evidence = actual_evidence
                .binary_search_by_key(&logseq_uuid, ProjectionClaimEvidence::logseq_uuid)
                .ok()
                .map(|index| &actual_evidence[index])
                .ok_or(EngineError::ProjectionIdentityAuthorityUnavailable {
                    logseq_uuid,
                    block_id: block.block_id,
                })?;
            match self.resolve_logseq_uuid_from_documents(logseq_uuid, evidence, &documents)? {
                LogseqUuidResolution::Unique(claim)
                    if claim.block_id == block.block_id
                        && claim.home_document_id == block.home_document_id
                        && claim.page_id == page_id
                        && Some(claim.origin) == block.logseq_identity_origin => {}
                LogseqUuidResolution::Ambiguous { claim_count } => {
                    return Err(EngineError::AmbiguousLogseqUuid {
                        logseq_uuid,
                        claim_count,
                    });
                }
                LogseqUuidResolution::Unclaimed | LogseqUuidResolution::Unique(_) => {
                    return Err(EngineError::ProjectionIdentityAuthorityUnavailable {
                        logseq_uuid,
                        block_id: block.block_id,
                    });
                }
            }
        }
        let mut accepted_heads = 0_usize;
        for document in frontier.documents() {
            for batch_id in document.direct_dependency_heads() {
                accepted_heads = accepted_heads.saturating_add(1);
                if !matches!(
                    self.archive_status(*batch_id)?,
                    Some(ArchiveStatus::Accepted { .. })
                ) || !matches!(
                    store
                        .inspect_batch(*batch_id)
                        .map_err(|error| EngineError::Archive(error.to_string()))?,
                    BatchInspection::Ready(_)
                ) {
                    return Err(EngineError::ProjectionFrontierNotDurable(*batch_id));
                }
            }
        }
        if accepted_heads == 0 {
            return Err(EngineError::ProjectionAuthorizationUnavailable);
        }
        Ok(ProjectionWriteAuthorization {
            state: ProjectionPageState {
                page,
                frontier: frontier.clone(),
                claim_evidence: actual_evidence,
            },
            claim_root: self.logseq_claim_root,
        })
    }

    pub fn resolve_logseq_uuid(&self, logseq_uuid: LogseqUuid) -> LogseqUuidResolution {
        self.resolve_logseq_uuid_current(logseq_uuid)
            .map(|(resolution, _, _)| resolution)
            .expect("authenticated Logseq claim lookup remains readable")
    }

    fn logseq_claim_record(
        &self,
        root: LogseqClaimIndexRoot,
        logseq_uuid: LogseqUuid,
    ) -> Result<LogseqClaimRecord, EngineError> {
        let mut introductions = match &self.logseq_claim_index {
            Some(index) => index
                .lookup_prefix(root, logseq_uuid.as_uuid().as_bytes())
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .into_iter()
                .map(|(key, bytes)| decode_logseq_claim_introduction(logseq_uuid, &key, &bytes))
                .collect::<Result<Vec<_>, _>>()?,
            None => self
                .ephemeral_logseq_claims
                .get(&logseq_uuid)
                .map(|record| record.introductions.clone())
                .unwrap_or_default(),
        };
        introductions.sort_unstable();
        if !strictly_sorted(&introductions) {
            return Err(EngineError::Archive(
                "duplicate or non-canonical Logseq claim introductions".into(),
            ));
        }
        Ok(LogseqClaimRecord {
            schema_version: LOGSEQ_CLAIM_RECORD_SCHEMA_VERSION,
            logseq_uuid,
            introductions,
        })
    }

    fn resolve_logseq_uuid_current(
        &self,
        logseq_uuid: LogseqUuid,
    ) -> Result<
        (
            LogseqUuidResolution,
            Option<ProjectionClaimEvidence>,
            BTreeMap<DocumentId, LoroDoc>,
        ),
        EngineError,
    > {
        let record = self.logseq_claim_record(self.logseq_claim_root, logseq_uuid)?;
        let participants: Vec<_> = record
            .introductions
            .iter()
            .map(|claim| ProjectionClaimParticipant::new(claim.block_id, claim.home_document_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if participants.is_empty() {
            return Ok((LogseqUuidResolution::Unclaimed, None, BTreeMap::new()));
        }

        let catalog = self
            .visible_documents
            .get(&self.catalog_document_id)
            .ok_or(EngineError::ProjectionAuthorizationUnavailable)?;
        let mut homes = BTreeMap::new();
        let mut live = BTreeMap::<BlockId, LogseqUuidClaim>::new();
        for participant in &participants {
            let home_document_id = participant.home_document_id();
            if !homes.contains_key(&home_document_id) {
                let home = self.clone_visible_document(home_document_id, 1)?;
                validate_shard(self.catalog_document_id, home_document_id, &home)?;
                homes.insert(home_document_id, home);
            }
            let Some(state) = read_block_state(
                home_document_id,
                &homes[&home_document_id],
                participant.block_id(),
            )?
            else {
                continue;
            };
            let (BlockOwner::Page(page_id), Some(current_uuid), Some(origin)) =
                (state.owner, state.logseq_uuid, state.logseq_identity_origin)
            else {
                continue;
            };
            if current_uuid != logseq_uuid
                || !matches!(
                    validate_catalog_page(self.catalog_document_id, catalog, page_id)?,
                    Some(PageState::Live { .. })
                )
            {
                continue;
            }
            live.insert(
                state.block_id,
                LogseqUuidClaim {
                    logseq_uuid,
                    block_id: state.block_id,
                    home_document_id,
                    page_id,
                    origin,
                },
            );
        }
        let resolution = match live.len() {
            0 => LogseqUuidResolution::Unclaimed,
            1 => LogseqUuidResolution::Unique(*live.values().next().expect("one live claim")),
            claim_count => LogseqUuidResolution::Ambiguous { claim_count },
        };
        let evidence = ProjectionClaimEvidence::new(logseq_uuid, participants)?;
        Ok((resolution, Some(evidence), homes))
    }

    fn logseq_claim_evidence_at_frontier(
        &self,
        logseq_uuid: LogseqUuid,
        frontier: &FrontierV2,
    ) -> Result<Option<ProjectionClaimEvidence>, EngineError> {
        let record = self.logseq_claim_record(self.logseq_claim_root, logseq_uuid)?;
        let heads = declared_batch_heads(frontier);
        let mut participants = BTreeSet::new();
        if let Some(store) = &self.scratch {
            let mut head_records = Vec::with_capacity(heads.len());
            for head in &heads {
                let record = super::causal_index::batch_record(store, &self.scratch_roots, *head)
                    .map_err(|error| EngineError::Archive(error.to_string()))?
                    .ok_or(EngineError::MissingDependency(*head))?;
                head_records.push((*head, record));
            }
            for introduction in record.introductions {
                if head_records.iter().any(|(head, clock)| {
                    *head == introduction.batch_id || clock.contains(introduction.causal_dot)
                }) {
                    participants.insert(ProjectionClaimParticipant::new(
                        introduction.block_id,
                        introduction.home_document_id,
                    ));
                }
            }
        } else {
            let ancestry = self.collect_batch_ancestry(&heads, self.is_blocked())?;
            for introduction in record.introductions {
                if ancestry.contains_key(&introduction.batch_id) {
                    participants.insert(ProjectionClaimParticipant::new(
                        introduction.block_id,
                        introduction.home_document_id,
                    ));
                }
            }
        }
        if participants.is_empty() {
            return Ok(None);
        }
        ProjectionClaimEvidence::new(logseq_uuid, participants.into_iter().collect())
            .map(Some)
            .map_err(Into::into)
    }

    fn resolve_logseq_uuid_from_documents(
        &self,
        logseq_uuid: LogseqUuid,
        evidence: &ProjectionClaimEvidence,
        documents: &BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<LogseqUuidResolution, EngineError> {
        let catalog = documents
            .get(&self.catalog_document_id)
            .ok_or(EngineError::MissingDocument(self.catalog_document_id))?;
        let mut live = BTreeMap::<BlockId, LogseqUuidClaim>::new();
        for participant in evidence.participants() {
            let home_document_id = participant.home_document_id();
            let home = documents
                .get(&home_document_id)
                .ok_or(EngineError::MissingDocument(home_document_id))?;
            validate_shard(self.catalog_document_id, home_document_id, home)?;
            let Some(state) = read_block_state(home_document_id, home, participant.block_id())?
            else {
                continue;
            };
            let (BlockOwner::Page(page_id), Some(uuid), Some(origin)) =
                (state.owner, state.logseq_uuid, state.logseq_identity_origin)
            else {
                continue;
            };
            if uuid != logseq_uuid
                || !matches!(
                    validate_catalog_page(self.catalog_document_id, catalog, page_id)?,
                    Some(PageState::Live { .. })
                )
            {
                continue;
            }
            live.insert(
                state.block_id,
                LogseqUuidClaim {
                    logseq_uuid,
                    block_id: state.block_id,
                    home_document_id,
                    page_id,
                    origin,
                },
            );
        }
        Ok(match live.len() {
            0 => LogseqUuidResolution::Unclaimed,
            1 => LogseqUuidResolution::Unique(*live.values().next().expect("one live claim")),
            claim_count => LogseqUuidResolution::Ambiguous { claim_count },
        })
    }

    fn materialize_page_from_documents(
        &self,
        page_id: PageId,
        documents: &BTreeMap<DocumentId, LoroDoc>,
    ) -> Result<MaterializedPage, EngineError> {
        let catalog = documents
            .get(&self.catalog_document_id)
            .ok_or(EngineError::MissingDocument(self.catalog_document_id))?;
        let page_state = validate_catalog_page(self.catalog_document_id, catalog, page_id)?
            .ok_or(EngineError::PageNotFound(page_id))?;
        let PageState::Live {
            path,
            home_document_id: page_document_id,
        } = page_state
        else {
            return Err(EngineError::PageDeleted(page_id));
        };
        let page_document = documents
            .get(&page_document_id)
            .ok_or(EngineError::MissingDocument(page_document_id))?;
        validate_shard(self.catalog_document_id, page_document_id, page_document)?;
        if shard_page_id(page_document)? != Some(page_id) {
            return Err(EngineError::MalformedDocument {
                document_id: page_document_id,
                reason: "membership shard page identity mismatch".into(),
            });
        }
        let preamble = read_page_preamble(page_document_id, page_document)?;
        let members = read_memberships(page_document_id, page_document)?;
        let mut by_home = BTreeMap::<DocumentId, Vec<(BlockId, MembershipClaim)>>::new();
        for (block_id, claim) in members {
            by_home
                .entry(claim.home_document_id)
                .or_default()
                .push((block_id, claim));
        }
        let mut blocks = Vec::new();
        for (home_document_id, claims) in &by_home {
            let home = documents
                .get(home_document_id)
                .ok_or(EngineError::MissingDocument(*home_document_id))?;
            validate_shard(self.catalog_document_id, *home_document_id, home)?;
            for (block_id, claim) in claims {
                let Some(state) = read_block_state(*home_document_id, home, *block_id)? else {
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
                        logseq_uuid: state.logseq_uuid,
                        logseq_identity_origin: state.logseq_identity_origin,
                        content: state.content,
                    });
                }
            }
        }
        blocks.sort_unstable_by(|left, right| {
            (&left.order, left.block_id).cmp(&(&right.order, right.block_id))
        });
        Ok(MaterializedPage {
            page_id,
            path,
            preamble,
            blocks,
            stats: MaterializationStats {
                catalog_documents_loaded: 1,
                membership_documents_loaded: 1,
                home_documents_loaded: by_home.len(),
                distinct_home_documents: by_home.keys().copied().collect(),
                physical_manifest_reads: 0,
                physical_object_reads: 0,
            },
        })
    }

    fn materialize_page_inner(
        &self,
        page_id: PageId,
        include_frontier: bool,
    ) -> Result<
        (
            MaterializedPage,
            Option<FrontierV2>,
            Vec<ProjectionClaimEvidence>,
        ),
        EngineError,
    > {
        self.begin_point_operation();
        self.ensure_not_blocked()?;
        let reads_before = self.archive_read_stats();
        let catalog = self
            .visible_documents
            .get(&self.catalog_document_id)
            .ok_or(EngineError::PageNotFound(page_id))?;
        let mut frontier_documents = BTreeMap::new();
        if include_frontier {
            frontier_documents.insert(
                self.catalog_document_id,
                self.current_document_dependencies(self.catalog_document_id, catalog)?,
            );
        }
        let page_state = validate_catalog_page(self.catalog_document_id, catalog, page_id)?
            .ok_or(EngineError::PageNotFound(page_id))?;
        let PageState::Live {
            path,
            home_document_id: page_document_id,
        } = page_state
        else {
            return Err(EngineError::PageDeleted(page_id));
        };
        let page_document = self.clone_visible_document(page_document_id, 1)?;
        validate_shard(self.catalog_document_id, page_document_id, &page_document)?;
        if include_frontier {
            frontier_documents.insert(
                page_document_id,
                self.current_document_dependencies(page_document_id, &page_document)?,
            );
        }
        if shard_page_id(&page_document)? != Some(page_id) {
            return Err(EngineError::MalformedDocument {
                document_id: page_document_id,
                reason: "membership shard page identity mismatch".into(),
            });
        }
        let preamble = read_page_preamble(page_document_id, &page_document)?;

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
                            logseq_uuid: state.logseq_uuid,
                            logseq_identity_origin: state.logseq_identity_origin,
                            content: state.content,
                        });
                    }
                }
                continue;
            }
            let home = self.clone_visible_document(*home_document_id, 1)?;
            validate_shard(self.catalog_document_id, *home_document_id, &home)?;
            if include_frontier {
                frontier_documents.insert(
                    *home_document_id,
                    self.current_document_dependencies(*home_document_id, &home)?,
                );
            }
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
                        logseq_uuid: state.logseq_uuid,
                        logseq_identity_origin: state.logseq_identity_origin,
                        content: state.content,
                    });
                }
            }
        }
        blocks.sort_unstable_by(|left, right| {
            (&left.order, left.block_id).cmp(&(&right.order, right.block_id))
        });
        let mut claim_evidence = Vec::new();
        if include_frontier {
            let block_claims: BTreeMap<_, _> = blocks
                .iter()
                .filter_map(|block| {
                    block.logseq_uuid.map(|uuid| {
                        (
                            uuid,
                            (
                                block.block_id,
                                block.home_document_id,
                                block.logseq_identity_origin,
                            ),
                        )
                    })
                })
                .collect();
            let mut referenced = page_logseq_references(&path, preamble.as_deref(), &blocks);
            referenced.extend(block_claims.keys().copied());
            for logseq_uuid in referenced {
                let (resolution, evidence, homes) =
                    self.resolve_logseq_uuid_current(logseq_uuid)?;
                if let Some((block_id, home_document_id, origin)) =
                    block_claims.get(&logseq_uuid).copied()
                {
                    match resolution {
                        LogseqUuidResolution::Unique(claim)
                            if claim.block_id == block_id
                                && claim.home_document_id == home_document_id
                                && claim.page_id == page_id
                                && Some(claim.origin) == origin => {}
                        LogseqUuidResolution::Ambiguous { claim_count } => {
                            return Err(EngineError::AmbiguousLogseqUuid {
                                logseq_uuid,
                                claim_count,
                            });
                        }
                        LogseqUuidResolution::Unclaimed | LogseqUuidResolution::Unique(_) => {
                            return Err(EngineError::ProjectionIdentityAuthorityUnavailable {
                                logseq_uuid,
                                block_id,
                            });
                        }
                    }
                } else if let LogseqUuidResolution::Ambiguous { claim_count } = resolution {
                    return Err(EngineError::AmbiguousLogseqUuid {
                        logseq_uuid,
                        claim_count,
                    });
                }
                for (home_document_id, home) in homes {
                    frontier_documents
                        .entry(home_document_id)
                        .or_insert(self.current_document_dependencies(home_document_id, &home)?);
                }
                if let Some(evidence) = evidence {
                    claim_evidence.push(evidence);
                }
            }
        }
        let reads_after = self.archive_read_stats();
        let page = MaterializedPage {
            page_id,
            path,
            preamble,
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
        };
        let frontier = include_frontier
            .then(|| FrontierV2::new(frontier_documents.into_values().collect()))
            .transpose()?;
        Ok((page, frontier, claim_evidence))
    }

    pub fn canonical_snapshot(&self) -> Result<super::CanonicalSnapshot, EngineError> {
        self.ensure_not_blocked()?;
        let Some(catalog) = self.visible_documents.get(&self.catalog_document_id) else {
            return Ok(super::CanonicalSnapshot::default());
        };
        let all_pages = validate_catalog(self.catalog_document_id, catalog)?;
        let mut pages = Vec::new();
        let mut page_preambles = Vec::new();
        let mut blocks = Vec::new();
        let mut memberships = Vec::new();
        let mut paths = BTreeMap::<ManagedPath, Vec<PageId>>::new();
        for (page_id, state) in all_pages {
            if let PageState::Live { path, .. } = &state {
                paths.entry(path.clone()).or_default().push(page_id);
                let materialized = self.materialize_page(page_id)?;
                page_preambles.push(PagePreambleState {
                    page_id,
                    home_document_id: state.home_document_id(),
                    preamble: materialized.preamble,
                });
                for block in materialized.blocks {
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
                        logseq_uuid: block.logseq_uuid,
                        logseq_identity_origin: block.logseq_identity_origin,
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
            page_preambles,
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
                        let manifest_fingerprint = self.archive_fingerprints[&batch_id];
                        self.set_final_status(
                            batch_id,
                            ArchiveStatus::Accepted { no_op, evidence },
                        );
                        if self.history_failure.is_some() {
                            break 'drain;
                        }
                        if let Err(error) =
                            self.activate_projection_work(batch_id, manifest_fingerprint)
                        {
                            self.history_failure = Some(error);
                            break 'drain;
                        }
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
        let Some(manifest_fingerprint) = self.archive_fingerprints.get(&batch_id).copied() else {
            return;
        };
        if let Err(error) =
            self.persist_durable_final_status(batch_id, manifest_fingerprint, status)
        {
            self.history_failure = Some(error);
            return;
        }
        if !self.persisted_staged.contains(&batch_id) {
            return;
        }
        self.persisted_staged.remove(&batch_id);
        self.statuses.remove(&batch_id);
        self.archive_fingerprints.remove(&batch_id);
        self.archive.remove(&batch_id);
    }

    fn persist_durable_final_status(
        &mut self,
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        status: ArchiveStatus,
    ) -> Result<(), EngineError> {
        if matches!(status, ArchiveStatus::Staged) {
            return Err(EngineError::Archive(
                "staged status cannot enter durable final history".into(),
            ));
        }
        let Some(store) = self.history_store.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        let (generation, root) = store
            .current()
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        self.history_generation = generation;
        self.history_root = root;
        if let Some(bytes) = store
            .lookup(root, batch_id)
            .map_err(|error| EngineError::Archive(error.to_string()))?
        {
            store.note_history_decode();
            let existing = decode_history_record(batch_id, &bytes)?;
            if existing.manifest_fingerprint != manifest_fingerprint
                || existing.status != status
                || existing.portable_path_key_version != super::PORTABLE_PATH_KEY_VERSION
                || existing.portable_path_root != self.portable_path_root
                || existing.catalog_checkpoint_binding != self.catalog_checkpoint_binding()
                || existing.portable_path_conflicts
                    != self
                        .portable_path_conflicts
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
            {
                return Err(EngineError::Archive(format!(
                    "durable engine history collision for batch {batch_id}"
                )));
            }
            self.status_point_cache
                .borrow_mut()
                .insert(batch_id, Some(existing));
            return Ok(());
        }
        let next_generation = generation.checked_add(1).ok_or_else(|| {
            EngineError::Archive("durable engine history generation overflow".into())
        })?;
        let record = new_history_record(
            next_generation,
            batch_id,
            manifest_fingerprint,
            self.portable_path_root,
            self.catalog_checkpoint_binding(),
            self.portable_path_conflicts.values().cloned().collect(),
            status,
        );
        let bytes = encode_history_record(&record)?;
        let (published_generation, published_root) = store
            .publish(batch_id, &bytes, self.durable_history_binding())
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        if published_generation != next_generation {
            return Err(EngineError::Archive(
                "durable engine history generation changed during publication".into(),
            ));
        }
        self.history_generation = published_generation;
        self.history_root = published_root;
        let mut point_cache = self.status_point_cache.borrow_mut();
        point_cache.clear();
        point_cache.insert(batch_id, Some(record));
        Ok(())
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
            if let Some(record) =
                super::dependency_queue::lookup(store, &self.scratch_roots, batch_id)
                    .map_err(|error| EngineError::Archive(error.to_string()))?
            {
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
                return Ok(Some(ColdHistoryRecord {
                    schema_version: ENGINE_HISTORY_SCHEMA_VERSION,
                    generation: 0,
                    batch_id,
                    manifest_fingerprint: record.manifest_fingerprint(),
                    portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
                    portable_path_root: self.portable_path_root,
                    catalog_checkpoint_binding: self.catalog_checkpoint_binding(),
                    portable_path_conflicts: self
                        .portable_path_conflicts
                        .values()
                        .cloned()
                        .collect(),
                    status,
                }));
            }
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

    /// Exact authorization witness from the endpoint's current durable head.
    ///
    /// This deliberately bypasses run-local scratch, the cached engine root,
    /// and the general point cache. Those remain appropriate for reconstruction
    /// and causal validation, but none can grant projection write authority.
    fn durable_endpoint_history_record(
        &mut self,
        batch_id: BatchId,
    ) -> Result<Option<ColdHistoryRecord>, EngineError> {
        self.ensure_not_blocked()?;
        let result = match self.history_store.as_ref().map(Arc::clone) {
            Some(store) => (|| {
                let (generation, root) = store
                    .current()
                    .map_err(|error| EngineError::Archive(error.to_string()))?;
                let Some(bytes) = store
                    .lookup(root, batch_id)
                    .map_err(|error| EngineError::Archive(error.to_string()))?
                else {
                    return Ok(None);
                };
                store.note_history_decode();
                let record = decode_history_record(batch_id, &bytes)?;
                if record.generation == 0 || record.generation > generation {
                    return Err(EngineError::Archive(
                        "engine history record is not bound to the current durable generation"
                            .into(),
                    ));
                }
                Ok(Some(record))
            })(),
            None => Err(EngineError::Archive(
                "projection authorization requires durable endpoint history".into(),
            )),
        };
        if let Err(error) = &result {
            self.history_failure = Some(error.clone());
        }
        result
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
        self.validate_manifested_projection_transition(
            batch_id,
            &declared_effect,
            &after,
            &updates,
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
        let portable_paths = self.prepare_portable_path_updates(
            &starting_roots,
            batch_id,
            self.archive[&batch_id].manifest().causal_dot(),
            &frontier,
            &declared_effect,
            validated_catalog_pages.as_ref(),
            true,
        )?;
        self.validate_manifested_portable_path_binding(
            batch_id,
            portable_paths.root,
            !portable_paths.conflicts.is_empty(),
        )?;
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
        let portable_path_blocked = !portable_paths.conflicts.is_empty();
        if portable_path_blocked {
            for conflict in &portable_paths.conflicts {
                self.portable_path_conflicts
                    .insert(conflict.key_digest(), conflict.clone());
            }
            self.fatal_handle = Some(portable_path_evidence_handle(&self.portable_path_conflicts));
        }
        let quarantined =
            identity.blocked || portable_path_blocked || !allow_publication || self.is_blocked();
        let logseq_claim_candidate = if quarantined {
            None
        } else {
            Some(self.prepare_logseq_claim_updates(
                batch_id,
                self.archive[&batch_id].manifest().causal_dot(),
                &declared_effect,
            )?)
        };
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
        if !quarantined {
            self.prepare_projection_work(batch_id)?;
        }
        if quarantined {
            if self.scratch.is_some() {
                self.scratch_roots = candidate_roots;
                self.block_claim_root = identity.block_claim_root;
                if !portable_path_blocked {
                    self.fatal_handle = identity.fatal_handle;
                }
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
            if !portable_path_blocked {
                self.fatal_handle = identity.fatal_handle;
            }
        }
        self.commit_logseq_claim_updates(
            logseq_claim_candidate.expect("visible batch prepared Logseq claim updates"),
        );
        self.commit_portable_path_updates(portable_paths);
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

    fn prepare_logseq_claim_updates(
        &self,
        batch_id: BatchId,
        causal_dot: BatchCausalDot,
        effect: &SemanticEffect,
    ) -> Result<
        (
            LogseqClaimIndexRoot,
            Vec<(LogseqUuid, LogseqClaimIntroduction)>,
        ),
        EngineError,
    > {
        let mut additions = Vec::new();
        for delta in effect.blocks() {
            let before = delta.before.as_ref().and_then(|state| state.logseq_uuid);
            let Some(logseq_uuid) = delta.after.as_ref().and_then(|state| state.logseq_uuid) else {
                continue;
            };
            if before == Some(logseq_uuid) {
                continue;
            }
            additions.push((
                logseq_uuid,
                LogseqClaimIntroduction {
                    block_id: delta.block_id,
                    home_document_id: delta.home_document_id,
                    batch_id,
                    causal_dot,
                },
            ));
        }
        if additions.is_empty() {
            return Ok((self.logseq_claim_root, Vec::new()));
        }
        additions.sort_unstable();
        additions.dedup();

        let mut encoded = BTreeMap::new();
        for (logseq_uuid, introduction) in &additions {
            encoded.insert(
                logseq_claim_introduction_key(*logseq_uuid, *introduction),
                encode_logseq_claim_introduction(*introduction)?,
            );
        }
        if self.logseq_claim_index.is_none() {
            let existing = self
                .ephemeral_logseq_claims
                .values()
                .map(|record| record.introductions.len())
                .sum::<usize>();
            let added = additions
                .iter()
                .filter(|(uuid, introduction)| {
                    !self
                        .ephemeral_logseq_claims
                        .get(uuid)
                        .is_some_and(|record| {
                            record.introductions.binary_search(introduction).is_ok()
                        })
                })
                .count();
            if existing.saturating_add(added) > MAX_EPHEMERAL_LOGSEQ_CLAIMS {
                return Err(EngineError::InvalidTransaction(
                    "no-store Logseq claim test index reached its fixed capacity".into(),
                ));
            }
            return Ok((self.logseq_claim_root, additions));
        }
        let root = self
            .logseq_claim_index
            .as_ref()
            .expect("checked store-backed claim index")
            .insert_many(self.logseq_claim_root, &encoded)
            .map_err(|error| EngineError::Archive(error.to_string()))?;
        Ok((root, additions))
    }

    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    fn prepare_portable_path_updates(
        &self,
        scratch_roots: &ScratchRoots,
        batch_id: BatchId,
        causal_dot: BatchCausalDot,
        frontier: &FrontierV2,
        effect: &SemanticEffect,
        prospective_pages: Option<&BTreeMap<PageId, PageState>>,
        publish_index: bool,
    ) -> Result<PortablePathPublicationCandidate, EngineError> {
        if effect.pages().is_empty() {
            return Ok(PortablePathPublicationCandidate {
                root: self.portable_path_root,
                changed: Vec::new(),
                conflicts: Vec::new(),
            });
        }
        let prospective_pages = prospective_pages.ok_or_else(|| {
            EngineError::InvalidTransaction(
                "affected catalog pages have no validated prospective state".into(),
            )
        })?;
        let affected = effect
            .pages()
            .iter()
            .map(|delta| delta.page_id)
            .collect::<BTreeSet<_>>();
        let mut desired = BTreeMap::<PageId, Option<ManagedPath>>::new();
        let mut keys = BTreeSet::new();
        for delta in effect.pages() {
            for state in [&delta.before, &delta.after].into_iter().flatten() {
                if let Some(path) = state.path() {
                    keys.insert(path.portable_key().digest());
                }
            }
            let path = prospective_pages
                .get(&delta.page_id)
                .and_then(PageState::path)
                .cloned();
            if let Some(path) = &path {
                keys.insert(path.portable_key().digest());
            }
            desired.insert(delta.page_id, path);
        }

        let requested = keys.iter().copied().collect::<Vec<_>>();
        let mut records = self.portable_path_records_many(&requested)?;
        let current_catalog = if records.is_empty() {
            None
        } else {
            Some(self.clone_validation_document(self.catalog_document_id, 1)?)
        };
        for (key, record) in &records {
            let Some(occupied) = record.occupied() else {
                continue;
            };
            let state = current_catalog
                .as_ref()
                .and_then(|catalog| read_page_state(catalog, occupied.page_id()).transpose())
                .transpose()?;
            if state.as_ref().and_then(PageState::path) != Some(occupied.exact_path())
                || occupied.exact_path().portable_key().digest() != *key
            {
                return Err(EngineError::Archive(
                    "portable-path index occupancy is misbound to current catalog state".into(),
                ));
            }
        }

        let candidate_clock = if let Some(store) = &self.scratch {
            super::causal_index::batch_record(store, scratch_roots, batch_id)
                .map_err(|error| EngineError::Archive(error.to_string()))?
        } else {
            None
        };
        let dependency_batches = if candidate_clock.is_none() {
            self.collect_batch_ancestry(&declared_batch_heads(frontier), self.is_blocked())?
                .into_keys()
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        let contains = |dot: BatchCausalDot, batch: BatchId| {
            candidate_clock
                .as_ref()
                .is_some_and(|clock| clock.contains(dot))
                || dependency_batches.contains(&batch)
                || batch == batch_id
        };

        let mut changed = BTreeMap::<PortablePathKeyDigest, PortablePathRecord>::new();
        for key in &requested {
            let Some(record) = records.get_mut(key) else {
                continue;
            };
            let release = record.occupied().is_some_and(|occupied| {
                affected.contains(&occupied.page_id())
                    && desired[&occupied.page_id()].as_ref().is_none_or(|path| {
                        path.portable_key().digest() != *key || path != occupied.exact_path()
                    })
            });
            if !release {
                continue;
            }
            let occupied = record
                .occupied()
                .cloned()
                .expect("checked portable-path occupancy");
            // A successful acquisition is admitted only after its frontier
            // contains the prior release fence. Any later release of that
            // acquisition therefore causally dominates every older release;
            // retaining the newest fence is complete and keeps the authenticated
            // point value bounded for an unbounded path lifetime.
            let latest_release = PortablePathReleased::new(
                occupied.page_id(),
                occupied.exact_path().clone(),
                occupied.acquisition_batch(),
                batch_id,
                causal_dot,
            );
            let replacement = PortablePathRecord::new(*key, None, Some(latest_release))
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            *record = replacement.clone();
            changed.insert(*key, replacement);
        }

        let mut acquisitions = desired
            .iter()
            .filter_map(|(page_id, path)| {
                path.as_ref()
                    .map(|path| (path.portable_key().digest(), *page_id, path.clone()))
            })
            .collect::<Vec<_>>();
        acquisitions.sort_unstable();
        let mut conflicts =
            BTreeMap::<PortablePathKeyDigest, Vec<PortablePathConflictParticipant>>::new();
        for (key, page_id, path) in acquisitions {
            let existing = records.get(&key);
            if existing
                .and_then(PortablePathRecord::occupied)
                .is_some_and(|occupied| {
                    occupied.page_id() == page_id && occupied.exact_path() == &path
                })
            {
                continue;
            }
            if let Some(occupied) = existing.and_then(PortablePathRecord::occupied) {
                if contains(occupied.causal_dot(), occupied.acquisition_batch()) {
                    return Err(EngineError::InvalidTransaction(format!(
                        "portable path {} conflicts at the declared dependency frontier",
                        path
                    )));
                }
                conflicts.entry(key).or_default().extend([
                    PortablePathConflictParticipant::new(
                        occupied.page_id(),
                        occupied.exact_path().clone(),
                        occupied.acquisition_batch(),
                    ),
                    PortablePathConflictParticipant::new(page_id, path, batch_id),
                ]);
                continue;
            }
            if let Some(release) = existing
                .and_then(PortablePathRecord::latest_release)
                .filter(|release| {
                    release.release_batch() != batch_id
                        && !contains(release.causal_dot(), release.release_batch())
                })
            {
                conflicts.entry(key).or_default().extend([
                    PortablePathConflictParticipant::new(
                        release.prior_page_id(),
                        release.prior_exact_path().clone(),
                        release.prior_acquisition_batch(),
                    ),
                    PortablePathConflictParticipant::new(page_id, path, batch_id),
                ]);
                continue;
            }
            let latest_release = existing
                .and_then(PortablePathRecord::latest_release)
                .cloned();
            let replacement = PortablePathRecord::new(
                key,
                Some(PortablePathOccupied::new(
                    page_id, path, batch_id, causal_dot,
                )),
                latest_release,
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?;
            records.insert(key, replacement.clone());
            changed.insert(key, replacement);
        }

        let conflicts = conflicts
            .into_iter()
            .map(|(key, participants)| PortablePathConflict::new(key, participants))
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            return Ok(PortablePathPublicationCandidate {
                root: self.portable_path_root,
                changed: Vec::new(),
                conflicts,
            });
        }
        if self.portable_path_index.is_none()
            && self
                .ephemeral_portable_paths
                .len()
                .saturating_add(changed.len())
                > MAX_EPHEMERAL_PORTABLE_PATHS
        {
            return Err(EngineError::InvalidTransaction(
                "no-store portable-path test index reached its fixed capacity".into(),
            ));
        }
        let root = if publish_index {
            self.portable_path_index
                .as_ref()
                .map(|index| index.insert_many(self.portable_path_root, &changed))
                .transpose()
                .map_err(|error| EngineError::Archive(error.to_string()))?
                .unwrap_or(self.portable_path_root)
        } else {
            self.portable_path_root
        };
        Ok(PortablePathPublicationCandidate {
            root,
            changed: changed.into_iter().collect(),
            conflicts,
        })
    }

    fn portable_path_records_many(
        &self,
        keys: &[PortablePathKeyDigest],
    ) -> Result<BTreeMap<PortablePathKeyDigest, PortablePathRecord>, EngineError> {
        match &self.portable_path_index {
            Some(index) => index
                .lookup_many(self.portable_path_root, keys)
                .map_err(|error| EngineError::Archive(error.to_string())),
            None => Ok(keys
                .iter()
                .filter_map(|key| {
                    self.ephemeral_portable_paths
                        .get(key)
                        .cloned()
                        .map(|record| (*key, record))
                })
                .collect()),
        }
    }

    fn commit_portable_path_updates(&mut self, candidate: PortablePathPublicationCandidate) {
        debug_assert!(candidate.conflicts.is_empty());
        self.portable_path_root = candidate.root;
        if self.portable_path_index.is_none() {
            self.ephemeral_portable_paths.extend(candidate.changed);
        }
    }

    fn commit_logseq_claim_updates(
        &mut self,
        (root, additions): (
            LogseqClaimIndexRoot,
            Vec<(LogseqUuid, LogseqClaimIntroduction)>,
        ),
    ) {
        self.logseq_claim_root = root;
        if self.logseq_claim_index.is_none() {
            for (logseq_uuid, introduction) in additions {
                let record = self
                    .ephemeral_logseq_claims
                    .entry(logseq_uuid)
                    .or_insert_with(|| LogseqClaimRecord {
                        schema_version: LOGSEQ_CLAIM_RECORD_SCHEMA_VERSION,
                        logseq_uuid,
                        introductions: Vec::new(),
                    });
                match record.introductions.binary_search(&introduction) {
                    Ok(_) => {}
                    Err(index) => record.introductions.insert(index, introduction),
                }
            }
        }
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
        let pending_engine_documents = pending_documents
            .iter()
            .map(|(document_id, document)| {
                (*document_id, EngineDocument::InMemory(document.clone()))
            })
            .collect();
        self.validate_manifested_projection_transition(
            batch_id,
            &declared_effect,
            &pending_engine_documents,
            updates,
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
        self.validate_prospective_references(
            &pending_engine_documents,
            &declared_effect,
            &new_exact_shards,
            validated_catalog_pages.as_ref(),
        )?;
        let portable_paths = self.prepare_portable_path_updates(
            &self.scratch_roots,
            batch_id,
            causal_dot,
            frontier,
            &declared_effect,
            validated_catalog_pages.as_ref(),
            true,
        )?;
        self.validate_manifested_portable_path_binding(
            batch_id,
            portable_paths.root,
            !portable_paths.conflicts.is_empty(),
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
        let portable_path_blocked = !portable_paths.conflicts.is_empty();
        if portable_path_blocked {
            for conflict in &portable_paths.conflicts {
                self.portable_path_conflicts
                    .insert(conflict.key_digest(), conflict.clone());
            }
            self.fatal_handle = Some(portable_path_evidence_handle(&self.portable_path_conflicts));
        }
        if identity.blocked || portable_path_blocked || !allow_publication || self.is_blocked() {
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
        let logseq_claim_candidate =
            self.prepare_logseq_claim_updates(batch_id, causal_dot, &declared_effect)?;
        self.prepare_projection_work(batch_id)?;
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
            if !portable_path_blocked {
                self.fatal_handle = identity.fatal_handle;
            }
        }
        self.commit_logseq_claim_updates(logseq_claim_candidate);
        self.commit_portable_path_updates(portable_paths);
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

    fn reconstruct_projection_frontier(
        &self,
        frontier: &FrontierV2,
    ) -> Result<BTreeMap<DocumentId, LoroDoc>, EngineError> {
        let Some(store) = &self.scratch else {
            return self.reconstruct_frontier(frontier);
        };
        let mut documents = BTreeMap::new();
        for dependencies in frontier.documents() {
            let (record, document, state_work) = super::document_state::load_external_exact(
                store,
                &self.scratch_roots,
                super::document_state::DocumentLane::Visible,
                dependencies.document_id(),
                dependencies.causal_state_digest(),
            )
            .map_err(|error| EngineError::Archive(error.to_string()))?
            .ok_or(EngineError::FrontierVectorMismatch(
                dependencies.document_id(),
            ))?;
            self.record_document_state_work(state_work);
            self.validate_external_record_anchor(dependencies.document_id(), &record)?;
            if record.peer_counters() != dependencies.peer_counters()
                || record.exact_direct_heads() != dependencies.direct_dependency_heads()
            {
                return Err(EngineError::FrontierVectorMismatch(
                    dependencies.document_id(),
                ));
            }
            documents.insert(dependencies.document_id(), document.into_document());
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

    fn current_document_dependencies(
        &self,
        document_id: DocumentId,
        document: &LoroDoc,
    ) -> Result<DocumentDependencies, EngineError> {
        Ok(DocumentDependencies::new(
            document_id,
            canonical_peer_counters(&document.oplog_vv())?,
            self.document_dependency_heads(document_id, false)?
                .into_iter()
                .collect(),
        )?)
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
            SemanticOperation::SetPagePreamble { page_id, preamble } => {
                if preamble
                    .as_ref()
                    .is_some_and(|value| value.len() > super::semantic::MAX_PAGE_PREAMBLE_BYTES)
                {
                    return Err(EngineError::InvalidTransaction(
                        "page preamble exceeds the semantic bound".into(),
                    ));
                }
                let page_document_id = self.page_home_from_working(working, *page_id)?;
                let was_working = working.contains_key(&page_document_id);
                let shard = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    page_document_id,
                    peer_id,
                )?;
                let current = read_page_preamble(page_document_id, shard)?;
                if current == *preamble {
                    if !was_working {
                        working.remove(&page_document_id);
                        before_vectors.remove(&page_document_id);
                        before_snapshots.remove(&page_document_id);
                    }
                    return Ok(());
                }
                let preamble_map = shard.get_map(SHARD_PAGE_PREAMBLE);
                match preamble {
                    Some(preamble) => preamble_map
                        .insert(SHARD_PAGE_PREAMBLE_VALUE, preamble.clone())
                        .map_err(loro_error)?,
                    None => preamble_map
                        .delete(SHARD_PAGE_PREAMBLE_VALUE)
                        .map_err(loro_error)?,
                }
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
            SemanticOperation::MutateBlockLogseqIdentity { block, mutation } => {
                let shard = self.ensure_working_document(
                    working,
                    before_vectors,
                    before_snapshots,
                    block.home_document_id,
                    peer_id,
                )?;
                let state = read_block_state(block.home_document_id, shard, block.block_id)?
                    .ok_or(EngineError::BlockNotFound(block.block_id))?;
                let (logseq_uuid, origin) = match mutation {
                    LogseqIdentityMutation::AssignExternal { logseq_uuid } => {
                        if state.logseq_uuid.is_some() {
                            return Err(EngineError::InvalidTransaction(
                                "external Logseq UUID assignment requires an unclaimed block"
                                    .into(),
                            ));
                        }
                        (
                            Some(*logseq_uuid),
                            Some(LogseqIdentityOrigin::ExternalImported),
                        )
                    }
                    LogseqIdentityMutation::ReplaceExternal { logseq_uuid } => {
                        if state.logseq_uuid.is_none() {
                            return Err(EngineError::InvalidTransaction(
                                "external Logseq UUID replacement requires an existing identity"
                                    .into(),
                            ));
                        }
                        (
                            Some(*logseq_uuid),
                            Some(LogseqIdentityOrigin::ExternalImported),
                        )
                    }
                    LogseqIdentityMutation::RemoveExternal => {
                        if state.logseq_uuid.is_none() {
                            return Err(EngineError::InvalidTransaction(
                                "external Logseq UUID removal requires an existing identity".into(),
                            ));
                        }
                        (None, None)
                    }
                    LogseqIdentityMutation::Generate {
                        logseq_uuid,
                        trigger,
                    } => {
                        if state.logseq_uuid.is_some() {
                            return Err(EngineError::InvalidTransaction(
                                "policy-generated Logseq UUID requires an unclaimed block".into(),
                            ));
                        }
                        (
                            Some(*logseq_uuid),
                            Some(LogseqIdentityOrigin::PolicyGenerated {
                                reason: trigger.policy_reason(),
                            }),
                        )
                    }
                };
                let uuids = shard.get_map(SHARD_LOGSEQ_UUIDS);
                let origins = shard.get_map(SHARD_LOGSEQ_IDENTITY_ORIGINS);
                match logseq_uuid {
                    Some(logseq_uuid) => uuids
                        .insert(&block.block_id.to_string(), logseq_uuid.to_string())
                        .map_err(loro_error)?,
                    None => uuids
                        .delete(&block.block_id.to_string())
                        .map_err(loro_error)?,
                }
                match origin {
                    Some(origin) => origins
                        .insert(&block.block_id.to_string(), encode_canonical(&origin)?)
                        .map_err(loro_error)?,
                    None => origins
                        .delete(&block.block_id.to_string())
                        .map_err(loro_error)?,
                }
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

    fn validate_logseq_identity_triggers(
        &self,
        transaction: &OperationTransaction,
        working: &BTreeMap<DocumentId, EngineDocument>,
    ) -> Result<(), EngineError> {
        let mut content_blocks = BTreeSet::new();
        for operation in &transaction.operations {
            if let SemanticOperation::RenamePageAndRewriteReferrers { referrers, .. } = operation {
                content_blocks.extend(
                    referrers
                        .iter()
                        .map(|(block, _)| (block.home_document_id, block.block_id)),
                );
            } else if let Some(block) = operation_content_block(operation) {
                content_blocks.insert((block.home_document_id, block.block_id));
            }
        }
        let has_typed_trigger = transaction.operations.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::Generate {
                        trigger: LogseqIdentityTrigger::BlockReference { .. }
                            | LogseqIdentityTrigger::BlockEmbed { .. },
                        ..
                    },
                    ..
                }
            )
        });
        if !has_typed_trigger {
            return Ok(());
        }

        let loaded_catalog;
        let catalog = if let Some(catalog) = working.get(&self.catalog_document_id) {
            catalog.document()
        } else {
            loaded_catalog = self.clone_visible_document(self.catalog_document_id, 1)?;
            &loaded_catalog
        };
        let before_catalog = self.visible_documents.get(&self.catalog_document_id);

        for operation in &transaction.operations {
            let SemanticOperation::MutateBlockLogseqIdentity {
                block,
                mutation:
                    LogseqIdentityMutation::Generate {
                        logseq_uuid,
                        trigger:
                            trigger @ (LogseqIdentityTrigger::BlockReference { .. }
                            | LogseqIdentityTrigger::BlockEmbed { .. }),
                    },
            } = operation
            else {
                continue;
            };
            let referrer = match trigger {
                LogseqIdentityTrigger::BlockReference { referrer }
                | LogseqIdentityTrigger::BlockEmbed { referrer } => referrer,
                LogseqIdentityTrigger::ExportUserAction
                | LogseqIdentityTrigger::CopiedDeepLinkUserAction => unreachable!(),
            };
            let has_trigger = (|| -> Result<bool, EngineError> {
                if !content_blocks.contains(&(referrer.home_document_id, referrer.block_id)) {
                    return Ok(false);
                }
                let Some(document) = working.get(&referrer.home_document_id) else {
                    return Ok(false);
                };
                let Some(state) = read_block_state(
                    referrer.home_document_id,
                    document.document(),
                    referrer.block_id,
                )?
                else {
                    return Ok(false);
                };
                let BlockOwner::Page(page_id) = state.owner else {
                    return Ok(false);
                };
                let Some(PageState::Live { path, .. }) = read_page_state(catalog, page_id)? else {
                    return Ok(false);
                };
                Ok(content_has_logseq_trigger(
                    &state.content,
                    *logseq_uuid,
                    *trigger,
                    path.as_str().ends_with(".org"),
                ))
            })()?;
            let had_trigger = (|| -> Result<bool, EngineError> {
                let Some(before_catalog) = before_catalog else {
                    return Ok(false);
                };
                let before_document = self.clone_visible_document(referrer.home_document_id, 1)?;
                let Some(state) = read_block_state(
                    referrer.home_document_id,
                    &before_document,
                    referrer.block_id,
                )?
                else {
                    return Ok(false);
                };
                let BlockOwner::Page(page_id) = state.owner else {
                    return Ok(false);
                };
                let Some(PageState::Live { path, .. }) = read_page_state(before_catalog, page_id)?
                else {
                    return Ok(false);
                };
                Ok(content_has_logseq_trigger(
                    &state.content,
                    *logseq_uuid,
                    *trigger,
                    path.as_str().ends_with(".org"),
                ))
            })()?;
            if !has_trigger || had_trigger {
                return Err(EngineError::MissingLogseqIdentityTrigger {
                    block_id: block.block_id,
                    logseq_uuid: *logseq_uuid,
                });
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

fn affected_projection_pages(effect: &SemanticEffect) -> BTreeSet<PageId> {
    let mut pages = BTreeSet::new();
    for delta in effect.pages() {
        pages.insert(delta.page_id);
    }
    for delta in effect.page_preambles() {
        pages.insert(delta.page_id);
    }
    for delta in effect.blocks() {
        for state in [&delta.before, &delta.after].into_iter().flatten() {
            if let BlockOwner::Page(page_id) = state.owner {
                pages.insert(page_id);
            }
        }
    }
    for delta in effect.memberships() {
        pages.insert(delta.page_id);
    }
    pages
}

fn projection_requirements(
    pages: &BTreeMap<PageId, DraftProjectionPage>,
) -> Result<Vec<ProjectionRequirement>, EngineError> {
    let mut requirements = Vec::new();
    for (page_id, transition) in pages {
        match (&transition.before, &transition.after) {
            (None, None) => {}
            (None, Some(after)) => requirements.push(ProjectionRequirement {
                page_id: *page_id,
                path: after.page.path.clone(),
                precondition: ProjectionRequirementState::Absent,
                target: ProjectionRequirementState::Present,
                render_base_path: None,
            }),
            (Some(before), None) => requirements.push(ProjectionRequirement {
                page_id: *page_id,
                path: before.page.path.clone(),
                precondition: ProjectionRequirementState::Present,
                target: ProjectionRequirementState::Absent,
                render_base_path: None,
            }),
            (Some(before), Some(after)) if before.page.path == after.page.path => {
                requirements.push(ProjectionRequirement {
                    page_id: *page_id,
                    path: after.page.path.clone(),
                    precondition: ProjectionRequirementState::Present,
                    target: ProjectionRequirementState::Present,
                    render_base_path: None,
                });
            }
            (Some(before), Some(after)) => {
                requirements.push(ProjectionRequirement {
                    page_id: *page_id,
                    path: before.page.path.clone(),
                    precondition: ProjectionRequirementState::Present,
                    target: ProjectionRequirementState::Absent,
                    render_base_path: None,
                });
                requirements.push(ProjectionRequirement {
                    page_id: *page_id,
                    path: after.page.path.clone(),
                    precondition: ProjectionRequirementState::Absent,
                    target: ProjectionRequirementState::Present,
                    render_base_path: Some(before.page.path.clone()),
                });
            }
        }
    }
    requirements.sort_unstable_by(|left, right| {
        (&left.path, left.page_id).cmp(&(&right.path, right.page_id))
    });
    if !requirements
        .windows(2)
        .all(|pair| pair[0].path < pair[1].path)
    {
        return Err(EngineError::ProjectionManifest(
            "semantic batch has conflicting projection paths".into(),
        ));
    }
    Ok(requirements)
}

fn validate_intent_directions(
    effect: &SemanticEffect,
    page_id: PageId,
    intents: &[&ManifestedProjectionIntent],
) -> Result<(), EngineError> {
    let page_delta = effect.pages().iter().find(|delta| delta.page_id == page_id);
    let before_path = page_delta
        .and_then(|delta| delta.before.as_ref())
        .and_then(PageState::path);
    let after_path = page_delta
        .and_then(|delta| delta.after.as_ref())
        .and_then(PageState::path);
    let matches_direction = |intent: &ManifestedProjectionIntent,
                             path: Option<&ManagedPath>,
                             precondition: ProjectionRequirementState,
                             target: ProjectionRequirementState| {
        path.is_none_or(|path| intent.path() == path)
            && matches!(
                (intent.precondition(), precondition),
                (
                    ManifestProjectionPrecondition::Absent,
                    ProjectionRequirementState::Absent
                ) | (
                    ManifestProjectionPrecondition::Present { .. },
                    ProjectionRequirementState::Present
                )
            )
            && matches!(
                (intent.target(), target),
                (
                    ManifestProjectionTarget::Absent,
                    ProjectionRequirementState::Absent
                ) | (
                    ManifestProjectionTarget::Present { .. },
                    ProjectionRequirementState::Present
                )
            )
    };
    match (page_delta, before_path, after_path) {
        (Some(_), None, Some(after)) => {
            if intents.len() != 1
                || !matches_direction(
                    intents[0],
                    Some(after),
                    ProjectionRequirementState::Absent,
                    ProjectionRequirementState::Present,
                )
            {
                return Err(EngineError::ProjectionManifest(
                    "page creation projection direction/path mismatch".into(),
                ));
            }
        }
        (Some(_), Some(before), None) => {
            if intents.len() != 1
                || !matches_direction(
                    intents[0],
                    Some(before),
                    ProjectionRequirementState::Present,
                    ProjectionRequirementState::Absent,
                )
            {
                return Err(EngineError::ProjectionManifest(
                    "page deletion projection direction/path mismatch".into(),
                ));
            }
        }
        (Some(_), Some(before), Some(after)) if before != after => {
            if intents.len() != 2
                || !intents.iter().any(|intent| {
                    matches_direction(
                        intent,
                        Some(before),
                        ProjectionRequirementState::Present,
                        ProjectionRequirementState::Absent,
                    )
                })
                || !intents.iter().any(|intent| {
                    matches_direction(
                        intent,
                        Some(after),
                        ProjectionRequirementState::Absent,
                        ProjectionRequirementState::Present,
                    ) && intent.render_base().is_some()
                })
            {
                return Err(EngineError::ProjectionManifest(
                    "page rename projection coverage/direction/render base mismatch".into(),
                ));
            }
        }
        _ => {
            if intents.len() != 1
                || !matches_direction(
                    intents[0],
                    before_path.or(after_path),
                    ProjectionRequirementState::Present,
                    ProjectionRequirementState::Present,
                )
            {
                return Err(EngineError::ProjectionManifest(
                    "affected page verification projection mismatch".into(),
                ));
            }
        }
    }
    Ok(())
}

fn projection_page_document_ids(
    catalog_document_id: DocumentId,
    page: &MaterializedPage,
    evidence: &[ProjectionClaimEvidence],
    documents: &BTreeMap<DocumentId, LoroDoc>,
) -> Result<BTreeSet<DocumentId>, EngineError> {
    let page_document_id = documents
        .iter()
        .filter(|(document_id, _)| **document_id != catalog_document_id)
        .find_map(|(document_id, document)| {
            (shard_page_id(document).ok().flatten() == Some(page.page_id)).then_some(*document_id)
        })
        .ok_or(EngineError::MissingDocument(catalog_document_id))?;
    let mut required = BTreeSet::from([catalog_document_id, page_document_id]);
    required.extend(page.stats.distinct_home_documents.iter().copied());
    required.extend(
        evidence
            .iter()
            .flat_map(ProjectionClaimEvidence::participants)
            .map(|participant| participant.home_document_id()),
    );
    Ok(required)
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

fn decode_accepted_evidence(bytes: &[u8]) -> Result<AcceptedBatchEvidence, EngineError> {
    let evidence: AcceptedBatchEvidence =
        postcard::from_bytes(bytes).map_err(|error| EngineError::Archive(error.to_string()))?;
    if postcard::to_allocvec(&evidence).map_err(|error| EngineError::Archive(error.to_string()))?
        != bytes
    {
        return Err(EngineError::Archive(
            "non-canonical accepted sequence evidence".into(),
        ));
    }
    validate_accepted_evidence(&evidence)?;
    Ok(evidence)
}

fn empty_accepted_frontier_root() -> AcceptedFrontierRoot {
    AcceptedFrontierRoot {
        schema_version: ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION,
        acceptance_sequence: 0,
        document_count: 0,
        retained_bytes_total: 0,
        document_map_root_key: None,
        document_map_root_digest: super::scratch_store::authenticated_map_empty_digest(),
        batch_map_root_key: None,
        batch_map_root_digest: super::scratch_store::authenticated_map_empty_digest(),
        state_digest: ContentDigest::of(b"tine/oplog/accepted-frontier/v3/empty"),
        scratch_root: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn next_accepted_frontier_root(
    prior: &AcceptedFrontierRoot,
    event_binding_digest: ContentDigest,
    acceptance_sequence: u64,
    document_count: u64,
    retained_bytes: u64,
    affected_documents: &[DocumentDependencies],
    document_map_root_key: Option<[u8; 16]>,
    document_map_root_digest: ContentDigest,
    batch_map_root_key: Option<[u8; 16]>,
    batch_map_root_digest: ContentDigest,
    scratch_root: Option<super::scratch_store::ScratchLsmRoot>,
) -> Result<AcceptedFrontierRoot, EngineError> {
    validate_accepted_frontier_root(prior)?;
    if acceptance_sequence != prior.acceptance_sequence.saturating_add(1) {
        return Err(EngineError::Archive(
            "accepted frontier sequence is not contiguous".into(),
        ));
    }
    let mut bytes = b"tine/oplog/accepted-frontier/v3\0".to_vec();
    bytes.extend_from_slice(prior.state_digest.as_bytes());
    bytes.extend_from_slice(event_binding_digest.as_bytes());
    bytes.extend_from_slice(&acceptance_sequence.to_be_bytes());
    bytes.extend_from_slice(&document_count.to_be_bytes());
    let retained_bytes_total = prior
        .retained_bytes_total
        .checked_add(retained_bytes)
        .ok_or_else(|| EngineError::Archive("accepted retained-byte total overflowed".into()))?;
    bytes.extend_from_slice(&retained_bytes_total.to_be_bytes());
    match document_map_root_key {
        Some(key) => {
            bytes.push(1);
            bytes.extend_from_slice(&key);
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(document_map_root_digest.as_bytes());
    match batch_map_root_key {
        Some(key) => {
            bytes.push(1);
            bytes.extend_from_slice(&key);
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(batch_map_root_digest.as_bytes());
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
        retained_bytes_total,
        document_map_root_key,
        document_map_root_digest,
        batch_map_root_key,
        batch_map_root_digest,
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
            || root.retained_bytes_total != 0
            || root.document_map_root_key.is_some()
            || root.document_map_root_digest
                != super::scratch_store::authenticated_map_empty_digest()
            || root.batch_map_root_key.is_some()
            || root.batch_map_root_digest != super::scratch_store::authenticated_map_empty_digest()
            || root.state_digest != empty_accepted_frontier_root().state_digest
            || root.scratch_root.is_some()
        {
            return Err(EngineError::Archive(
                "malformed empty accepted-frontier root".into(),
            ));
        }
    } else if root.batch_map_root_key.is_none()
        || root.batch_map_root_digest == super::scratch_store::authenticated_map_empty_digest()
        || (root.document_count == 0
            && (root.document_map_root_key.is_some()
                || root.document_map_root_digest
                    != super::scratch_store::authenticated_map_empty_digest()))
        || (root.document_count > 0
            && (root.document_map_root_key.is_none()
                || root.document_map_root_digest
                    == super::scratch_store::authenticated_map_empty_digest()))
    {
        return Err(EngineError::Archive(
            "malformed nonempty accepted-frontier document map".into(),
        ));
    }
    Ok(())
}

fn accepted_batch_retained_bytes(batch: &ValidatedBatch) -> Result<u64, EngineError> {
    let manifest_bytes = batch
        .manifest()
        .encode()
        .map_err(|error| EngineError::Archive(error.to_string()))?;
    batch
        .objects()
        .iter()
        .try_fold(manifest_bytes.len() as u64, |total, object| {
            let encoded = object
                .encode()
                .map_err(|error| EngineError::Archive(error.to_string()))?;
            total
                .checked_add(encoded.len() as u64)
                .ok_or_else(|| EngineError::Archive("accepted retained bytes overflowed".into()))
        })
}

fn authenticated_document_map_root(
    documents: &[DocumentDependencies],
) -> Result<(Option<[u8; 16]>, ContentDigest), EngineError> {
    let canonical = FrontierV2::new(documents.to_vec())?;
    if canonical.documents() != documents {
        return Err(EngineError::Archive(
            "authenticated document map is not canonically ordered".into(),
        ));
    }
    let entries = documents
        .iter()
        .map(|document| {
            Ok((
                document.document_id().as_uuid().into_bytes(),
                ContentDigest::of(&encode_accepted_document(document)?),
            ))
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    Ok(authenticated_map_subtree(&entries))
}

fn authenticated_map_root(
    entries: &[(BatchId, ContentDigest)],
) -> Result<(Option<[u8; 16]>, ContentDigest), EngineError> {
    if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(EngineError::Archive(
            "authenticated batch map is not canonically ordered".into(),
        ));
    }
    Ok(authenticated_map_subtree(
        &entries
            .iter()
            .map(|(batch_id, digest)| (batch_id.as_uuid().into_bytes(), *digest))
            .collect::<Vec<_>>(),
    ))
}

pub(crate) fn causal_clock_counter_digest(peer: CausalPeerId, counter: u64) -> ContentDigest {
    let mut bytes = b"tine/oplog/causal-clock-entry/v1\0".to_vec();
    bytes.extend_from_slice(peer.as_device_id().as_uuid().as_bytes());
    bytes.extend_from_slice(&counter.to_be_bytes());
    ContentDigest::of(&bytes)
}

pub(crate) fn authenticated_causal_clock_root(
    clock: &[(CausalPeerId, u64)],
) -> Result<(Option<[u8; 16]>, ContentDigest), EngineError> {
    if clock.is_empty()
        || clock.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || clock.iter().any(|(_, counter)| *counter == 0)
    {
        return Err(EngineError::Archive(
            "authenticated causal clock is not canonical".into(),
        ));
    }
    let entries = clock
        .iter()
        .map(|(peer, counter)| {
            (
                peer.as_device_id().as_uuid().into_bytes(),
                causal_clock_counter_digest(*peer, *counter),
            )
        })
        .collect::<Vec<_>>();
    Ok(authenticated_map_subtree(&entries))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn accepted_causal_record_digest(
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    event_binding_digest: ContentDigest,
    dot: BatchCausalDot,
    clock_root_key: Option<[u8; 16]>,
    clock_root_digest: ContentDigest,
) -> ContentDigest {
    let mut bytes = b"tine/oplog/accepted-causal-record/v1\0".to_vec();
    bytes.extend_from_slice(batch_id.as_uuid().as_bytes());
    bytes.extend_from_slice(manifest_fingerprint.as_bytes());
    bytes.extend_from_slice(event_binding_digest.as_bytes());
    bytes.extend_from_slice(dot.peer_id().as_device_id().as_uuid().as_bytes());
    bytes.extend_from_slice(&dot.counter().to_be_bytes());
    match clock_root_key {
        Some(key) => {
            bytes.push(1);
            bytes.extend_from_slice(&key);
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(clock_root_digest.as_bytes());
    ContentDigest::of(&bytes)
}

fn authenticated_map_subtree(
    entries: &[([u8; 16], ContentDigest)],
) -> (Option<[u8; 16]>, ContentDigest) {
    let Some((root_index, (key, value_digest))) =
        entries
            .iter()
            .enumerate()
            .min_by(|(_, (left, _)), (_, (right, _))| {
                super::scratch_store::authenticated_map_priority_order(*left, *right)
            })
    else {
        return (None, super::scratch_store::authenticated_map_empty_digest());
    };
    let left = authenticated_map_subtree(&entries[..root_index]);
    let right = authenticated_map_subtree(&entries[root_index + 1..]);
    (
        Some(*key),
        super::scratch_store::authenticated_map_node_digest(
            *key,
            *value_digest,
            left.0.map(|child_key| (child_key, left.1)),
            right.0.map(|child_key| (child_key, right.1)),
        ),
    )
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
        evidence
            .post_frontier_root
            .retained_bytes_total
            .checked_sub(evidence.prior_frontier_root.retained_bytes_total)
            .ok_or_else(|| EngineError::Archive("accepted retained bytes regressed".into()))?,
        &evidence.affected_documents,
        evidence.post_frontier_root.document_map_root_key,
        evidence.post_frontier_root.document_map_root_digest,
        evidence.post_frontier_root.batch_map_root_key,
        evidence.post_frontier_root.batch_map_root_digest,
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

fn logseq_claim_introduction_key(
    logseq_uuid: LogseqUuid,
    introduction: LogseqClaimIntroduction,
) -> Vec<u8> {
    let mut key = Vec::with_capacity(88);
    key.extend_from_slice(logseq_uuid.as_uuid().as_bytes());
    key.extend_from_slice(introduction.block_id.as_uuid().as_bytes());
    key.extend_from_slice(introduction.home_document_id.as_uuid().as_bytes());
    key.extend_from_slice(introduction.batch_id.as_uuid().as_bytes());
    key.extend_from_slice(
        introduction
            .causal_dot
            .peer_id()
            .as_device_id()
            .as_uuid()
            .as_bytes(),
    );
    key.extend_from_slice(&introduction.causal_dot.counter().to_be_bytes());
    key
}

fn encode_logseq_claim_introduction(
    introduction: LogseqClaimIntroduction,
) -> Result<Vec<u8>, EngineError> {
    postcard::to_allocvec(&LogseqClaimIntroductionRecord {
        schema_version: LOGSEQ_CLAIM_RECORD_SCHEMA_VERSION,
        introduction,
    })
    .map_err(|error| EngineError::Archive(error.to_string()))
}

fn decode_logseq_claim_introduction(
    expected_uuid: LogseqUuid,
    key: &[u8],
    bytes: &[u8],
) -> Result<LogseqClaimIntroduction, EngineError> {
    let record: LogseqClaimIntroductionRecord =
        postcard::from_bytes(bytes).map_err(|error| EngineError::Archive(error.to_string()))?;
    if record.schema_version != LOGSEQ_CLAIM_RECORD_SCHEMA_VERSION
        || logseq_claim_introduction_key(expected_uuid, record.introduction) != key
        || encode_logseq_claim_introduction(record.introduction)?.as_slice() != bytes
    {
        return Err(EngineError::Archive(
            "non-canonical or misbound Logseq claim introduction".into(),
        ));
    }
    Ok(record.introduction)
}

fn new_history_record(
    generation: u64,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    portable_path_root: PortablePathIndexRoot,
    catalog_checkpoint_binding: ContentDigest,
    portable_path_conflicts: Vec<PortablePathConflict>,
    status: ArchiveStatus,
) -> ColdHistoryRecord {
    ColdHistoryRecord {
        schema_version: ENGINE_HISTORY_SCHEMA_VERSION,
        generation,
        batch_id,
        manifest_fingerprint,
        portable_path_key_version: super::PORTABLE_PATH_KEY_VERSION,
        portable_path_root,
        catalog_checkpoint_binding,
        portable_path_conflicts,
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
        || record.portable_path_key_version != super::PORTABLE_PATH_KEY_VERSION
        || record
            .portable_path_conflicts
            .windows(2)
            .any(|pair| pair[0].key_digest() >= pair[1].key_digest())
        || record.portable_path_conflicts.iter().any(|conflict| {
            conflict.key_version() != super::PORTABLE_PATH_KEY_VERSION
                || conflict.participants().len() < 2
                || conflict
                    .participants()
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
        })
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
    store: &super::object_store::DurableEngineHistoryStore,
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

fn validate_catalog_page(
    catalog_document_id: DocumentId,
    document: &LoroDoc,
    page_id: PageId,
) -> Result<Option<PageState>, EngineError> {
    validate_document_roots(catalog_document_id, document, &[CATALOG_PAGES])?;
    if document.get_map(CATALOG_PAGES).len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::InvalidCrdt(
            "catalog entry bound exceeded".into(),
        ));
    }
    let state = read_page_state(document, page_id)?;
    if state
        .as_ref()
        .is_some_and(|state| state.home_document_id() == catalog_document_id)
    {
        return Err(EngineError::MalformedDocument {
            document_id: catalog_document_id,
            reason: format!("catalog cannot be the immutable home of page {page_id}"),
        });
    }
    Ok(state)
}

fn validate_shard(
    catalog_document_id: DocumentId,
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<(), EngineError> {
    validate_shard_metadata(catalog_document_id, document_id, document)?;
    read_page_preamble(document_id, document)?;
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
        &[
            SHARD_META,
            SHARD_OWNERS,
            SHARD_MEMBERS,
            SHARD_CONTENT,
            SHARD_LOGSEQ_UUIDS,
            SHARD_LOGSEQ_IDENTITY_ORIGINS,
            SHARD_PAGE_PREAMBLE,
        ],
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
        page_preamble: Option<PagePreambleState>,
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
                &[
                    SHARD_META,
                    SHARD_OWNERS,
                    SHARD_MEMBERS,
                    SHARD_CONTENT,
                    SHARD_LOGSEQ_UUIDS,
                    SHARD_LOGSEQ_IDENTITY_ORIGINS,
                    SHARD_PAGE_PREAMBLE,
                ],
            )?;
        }
        let page_id = shard_page_id(document)?;
        let page_preamble = page_id
            .map(|page_id| {
                read_page_preamble(document_id, document).map(|preamble| PagePreambleState {
                    page_id,
                    home_document_id: document_id,
                    preamble,
                })
            })
            .transpose()?;
        let blocks = read_all_blocks(document_id, document)?;
        let memberships = read_memberships(document_id, document)?;
        #[cfg(test)]
        record_owned_semantic_snapshot_entries(blocks.len().saturating_add(memberships.len()));
        Ok(SemanticDocumentSnapshot::Shard {
            page_id,
            page_preamble,
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
    document.get_map(SHARD_LOGSEQ_UUIDS);
    document.get_map(SHARD_LOGSEQ_IDENTITY_ORIGINS);
    document.get_map(SHARD_PAGE_PREAMBLE);
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
    let logseq_uuids = document.get_map(SHARD_LOGSEQ_UUIDS);
    let logseq_origins = document.get_map(SHARD_LOGSEQ_IDENTITY_ORIGINS);
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
    if logseq_uuids.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "Logseq UUID entry bound exceeded".into(),
        });
    }
    if logseq_origins.len() > MAX_DOCUMENT_ENTRIES {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "Logseq identity-origin entry bound exceeded".into(),
        });
    }
    if logseq_uuids.len() != logseq_origins.len() {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "Logseq UUID and identity-origin key coverage differs".into(),
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
        let logseq_uuid = read_logseq_uuid(document_id, document, block_id)?;
        let logseq_identity_origin = read_logseq_identity_origin(document_id, document, block_id)?;
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
            || after.logseq_uuid != logseq_uuid
            || after.logseq_identity_origin != logseq_identity_origin
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
    for key in logseq_uuids.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if map_string(&owners, &key)?.is_none() {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("Logseq UUID {block_id} has no owner register"),
            });
        }
        read_logseq_uuid(document_id, document, block_id)?;
        read_logseq_identity_origin(document_id, document, block_id)?;
    }
    for key in logseq_origins.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if map_string(&logseq_uuids, &key)?.is_none() {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("Logseq identity origin {block_id} has no UUID register"),
            });
        }
        read_logseq_identity_origin(document_id, document, block_id)?;
    }

    let page_preamble = PagePreambleState {
        page_id,
        home_document_id: document_id,
        preamble: read_page_preamble(document_id, document)?,
    };
    let declared_preambles = declared.page_preambles();
    let preamble_start =
        declared_preambles.partition_point(|delta| delta.home_document_id < document_id);
    let preamble_end =
        declared_preambles.partition_point(|delta| delta.home_document_id <= document_id);
    let declared_preambles = &declared_preambles[preamble_start..preamble_end];
    if declared_preambles.len() != 1
        || declared_preambles[0].page_id != page_id
        || declared_preambles[0].before.is_some()
        || declared_preambles[0].after.as_ref() != Some(&page_preamble)
    {
        return Err(EngineError::SemanticEffectMismatch);
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
    let mut page_preambles = Vec::new();
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
                    page_preamble: _,
                    blocks: _,
                    memberships: _,
                }),
                None
                | Some(SemanticDocumentSnapshot::Shard {
                    page_id: _,
                    page_preamble: _,
                    blocks: _,
                    memberships: _,
                }),
            ) => {
                let (before_page_id, before_preamble, before_blocks, before_members) =
                    match before.get(&document_id) {
                        Some(SemanticDocumentSnapshot::Shard {
                            page_id,
                            page_preamble,
                            blocks,
                            memberships,
                        }) => (*page_id, page_preamble.as_ref(), blocks, memberships),
                        _ => (None, None, &BTreeMap::new(), &BTreeMap::new()),
                    };
                let (after_page_id, after_preamble, after_blocks, after_members) =
                    match after.get(&document_id) {
                        Some(SemanticDocumentSnapshot::Shard {
                            page_id,
                            page_preamble,
                            blocks,
                            memberships,
                        }) => (*page_id, page_preamble.as_ref(), blocks, memberships),
                        _ => (None, None, &BTreeMap::new(), &BTreeMap::new()),
                    };
                if before_page_id.is_some() && before_page_id != after_page_id {
                    return Err(EngineError::MalformedDocument {
                        document_id,
                        reason: "stable shard page identity changed".into(),
                    });
                }
                if before_preamble != after_preamble {
                    let page_id = after_preamble
                        .or(before_preamble)
                        .expect("changed preamble has one semantic state")
                        .page_id;
                    page_preambles.push(PagePreambleDelta {
                        page_id,
                        home_document_id: document_id,
                        before: before_preamble.cloned(),
                        after: after_preamble.cloned(),
                    });
                }
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
    let effect =
        SemanticEffect::new_with_page_preambles(pages, page_preambles, blocks, memberships)
            .map_err(EngineError::from)?;
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
    let mut declared_page_preambles = declared.page_preambles().iter().peekable();
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
        skip_validated_new_shard_preamble_deltas(
            &mut declared_page_preambles,
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
                    page_preamble: _,
                    blocks: _,
                    memberships: _,
                }),
                None
                | Some(SemanticDocumentSnapshot::Shard {
                    page_id: _,
                    page_preamble: _,
                    blocks: _,
                    memberships: _,
                }),
            ) => {
                let (before_page_id, before_preamble, before_blocks, before_memberships) =
                    match before_snapshot {
                        Some(SemanticDocumentSnapshot::Shard {
                            page_id,
                            page_preamble,
                            blocks,
                            memberships,
                        }) => (
                            *page_id,
                            page_preamble.as_ref(),
                            Some(blocks),
                            Some(memberships),
                        ),
                        None => (None, None, None, None),
                        Some(SemanticDocumentSnapshot::Catalog(_)) => unreachable!(),
                    };
                let (after_page_id, after_preamble, after_blocks, after_memberships) =
                    match after_snapshot {
                        Some(SemanticDocumentSnapshot::Shard {
                            page_id,
                            page_preamble,
                            blocks,
                            memberships,
                        }) => (
                            *page_id,
                            page_preamble.as_ref(),
                            Some(blocks),
                            Some(memberships),
                        ),
                        None => (None, None, None, None),
                        Some(SemanticDocumentSnapshot::Catalog(_)) => unreachable!(),
                    };
                if before_page_id.is_some() && before_page_id != after_page_id {
                    return Err(EngineError::MalformedDocument {
                        document_id,
                        reason: "stable shard page identity changed".into(),
                    });
                }
                if before_preamble != after_preamble {
                    let Some(delta) = declared_page_preambles.next() else {
                        return Err(EngineError::SemanticEffectMismatch);
                    };
                    let page_id = after_preamble
                        .or(before_preamble)
                        .expect("changed preamble has one semantic state")
                        .page_id;
                    if delta.page_id != page_id
                        || delta.home_document_id != document_id
                        || delta.before.as_ref() != before_preamble
                        || delta.after.as_ref() != after_preamble
                    {
                        return Err(EngineError::SemanticEffectMismatch);
                    }
                }
                if !compare_block_deltas(
                    &mut declared_blocks,
                    document_id,
                    before_blocks,
                    after_blocks,
                ) {
                    return Err(EngineError::SemanticEffectMismatch);
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
    skip_validated_new_shard_preamble_deltas(
        &mut declared_page_preambles,
        &validated_new_shards.documents,
    );
    if declared_pages.next().is_some()
        || declared_page_preambles.next().is_some()
        || declared_blocks.next().is_some()
    {
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

fn skip_validated_new_shard_preamble_deltas(
    declared: &mut std::iter::Peekable<std::slice::Iter<'_, PagePreambleDelta>>,
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
    let logseq_uuids = document.get_map(SHARD_LOGSEQ_UUIDS);
    let logseq_origins = document.get_map(SHARD_LOGSEQ_IDENTITY_ORIGINS);
    if owners.len() > MAX_DOCUMENT_ENTRIES
        || content.len() > MAX_DOCUMENT_ENTRIES
        || logseq_uuids.len() > MAX_DOCUMENT_ENTRIES
        || logseq_origins.len() > MAX_DOCUMENT_ENTRIES
    {
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
    for key in logseq_uuids.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if !result.contains_key(&block_id) {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("Logseq UUID {block_id} has no owner register"),
            });
        }
        read_logseq_uuid(document_id, document, block_id)?;
    }
    for key in logseq_origins.keys() {
        let block_id = parse_block_key(document_id, &key)?;
        if !result.contains_key(&block_id) {
            return Err(EngineError::MalformedDocument {
                document_id,
                reason: format!("Logseq identity origin {block_id} has no owner register"),
            });
        }
        read_logseq_identity_origin(document_id, document, block_id)?;
    }
    if logseq_uuids.len() != logseq_origins.len() {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "Logseq UUID and identity-origin key coverage differs".into(),
        });
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
    let logseq_uuid = read_logseq_uuid(document_id, document, block_id)?;
    let logseq_identity_origin = read_logseq_identity_origin(document_id, document, block_id)?;
    if logseq_uuid.is_some() != logseq_identity_origin.is_some() {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: format!(
                "block {block_id} must carry its Logseq UUID and identity origin together"
            ),
        });
    }
    Ok(Some(BlockState {
        block_id,
        home_document_id: document_id,
        owner: parse_owner(&owner)?,
        logseq_uuid,
        logseq_identity_origin,
        content,
    }))
}

fn read_logseq_identity_origin(
    document_id: DocumentId,
    document: &LoroDoc,
    block_id: BlockId,
) -> Result<Option<LogseqIdentityOrigin>, EngineError> {
    map_string(
        &document.get_map(SHARD_LOGSEQ_IDENTITY_ORIGINS),
        &block_id.to_string(),
    )?
    .map(|value| {
        decode_canonical(&value).map_err(|_| EngineError::MalformedDocument {
            document_id,
            reason: format!("invalid Logseq identity-origin register for block {block_id}"),
        })
    })
    .transpose()
}

fn read_page_preamble(
    document_id: DocumentId,
    document: &LoroDoc,
) -> Result<Option<String>, EngineError> {
    let preamble = document.get_map(SHARD_PAGE_PREAMBLE);
    if preamble.len() > 1
        || preamble
            .keys()
            .next()
            .is_some_and(|key| key.as_str() != SHARD_PAGE_PREAMBLE_VALUE)
    {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "page preamble root must contain only its optional value".into(),
        });
    }
    let value = map_string(&preamble, SHARD_PAGE_PREAMBLE_VALUE)?;
    if value
        .as_ref()
        .is_some_and(|value| value.len() > super::semantic::MAX_PAGE_PREAMBLE_BYTES)
    {
        return Err(EngineError::MalformedDocument {
            document_id,
            reason: "page preamble exceeds the semantic bound".into(),
        });
    }
    Ok(value)
}

fn read_logseq_uuid(
    document_id: DocumentId,
    document: &LoroDoc,
    block_id: BlockId,
) -> Result<Option<LogseqUuid>, EngineError> {
    map_string(&document.get_map(SHARD_LOGSEQ_UUIDS), &block_id.to_string())?
        .map(|value| {
            LogseqUuid::parse(&value).map_err(|_| EngineError::MalformedDocument {
                document_id,
                reason: format!("invalid Logseq UUID register for block {block_id}"),
            })
        })
        .transpose()
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
    ProjectionManifest(String),
    ProjectionWork(String),
    AuthorDraftStale,
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
    MissingLogseqIdentityTrigger {
        block_id: BlockId,
        logseq_uuid: LogseqUuid,
    },
    AmbiguousLogseqUuid {
        logseq_uuid: LogseqUuid,
        claim_count: usize,
    },
    ProjectionIdentityAuthorityUnavailable {
        logseq_uuid: LogseqUuid,
        block_id: BlockId,
    },
    ProjectionClaimEvidenceMismatch,
    ProjectionAuthorizationUnavailable,
    ProjectionFrontierNotDurable(BatchId),
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
            Self::ProjectionManifest(error) => {
                write!(f, "projection manifest validation failed: {error}")
            }
            Self::ProjectionWork(error) => write!(f, "projection work index failed: {error}"),
            Self::AuthorDraftStale => {
                f.write_str("author transaction draft generation/root is stale")
            }
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
            Self::MissingLogseqIdentityTrigger {
                block_id,
                logseq_uuid,
            } => write!(
                f,
                "policy-generated Logseq UUID {logseq_uuid} for block {block_id} has no same-transaction content trigger"
            ),
            Self::AmbiguousLogseqUuid {
                logseq_uuid,
                claim_count,
            } => write!(
                f,
                "Logseq UUID {logseq_uuid} has {claim_count} live authoritative claims"
            ),
            Self::ProjectionIdentityAuthorityUnavailable {
                logseq_uuid,
                block_id,
            } => write!(
                f,
                "Logseq UUID {logseq_uuid} does not uniquely authorize materialized block {block_id}"
            ),
            Self::ProjectionClaimEvidenceMismatch => f.write_str(
                "projection claim evidence does not match authenticated historical participants",
            ),
            Self::ProjectionAuthorizationUnavailable => f.write_str(
                "projection write authorization requires accepted durable archive state",
            ),
            Self::ProjectionFrontierNotDurable(batch_id) => write!(
                f,
                "projection frontier batch {batch_id} is not accepted durable state"
            ),
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
            crate::oplog::BatchOrigin::BootstrapImport,
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
        page_preambles: Vec<PagePreambleDelta>,
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
            page_preambles: Vec::new(),
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
            logseq_uuid: None,
            logseq_identity_origin: None,
            content: content.into(),
        }
    }

    #[test]
    fn malformed_or_misbound_logseq_uuid_registers_fail_closed() {
        let catalog_id = DocumentId::from_uuid(Uuid::from_u128(67_000));
        let home_id = DocumentId::from_uuid(Uuid::from_u128(67_001));
        let page_id = PageId::from_uuid(Uuid::from_u128(67_002));
        let block_id = BlockId::from_uuid(Uuid::from_u128(67_003));
        let document = LoroDoc::new();
        document.set_peer_id(67_000).unwrap();
        document
            .get_map(SHARD_META)
            .insert(SHARD_PAGE_ID, page_id.to_string())
            .unwrap();
        document
            .get_map(SHARD_OWNERS)
            .insert(&block_id.to_string(), page_id.to_string())
            .unwrap();
        document
            .get_map(SHARD_CONTENT)
            .insert_container(&block_id.to_string(), LoroText::new())
            .unwrap()
            .insert(0, "content")
            .unwrap();
        document
            .get_map(SHARD_LOGSEQ_UUIDS)
            .insert(&block_id.to_string(), "not-a-uuid")
            .unwrap();
        assert!(matches!(
            validate_shard(catalog_id, home_id, &document),
            Err(EngineError::MalformedDocument { document_id, .. })
                if document_id == home_id
        ));

        document
            .get_map(SHARD_LOGSEQ_UUIDS)
            .insert(
                &block_id.to_string(),
                LogseqUuid::from_uuid(Uuid::from_u128(67_004)).to_string(),
            )
            .unwrap();
        let misbound = BlockId::from_uuid(Uuid::from_u128(67_005));
        document
            .get_map(SHARD_LOGSEQ_UUIDS)
            .insert(
                &misbound.to_string(),
                LogseqUuid::from_uuid(Uuid::from_u128(67_006)).to_string(),
            )
            .unwrap();
        assert!(matches!(
            validate_shard(catalog_id, home_id, &document),
            Err(EngineError::MalformedDocument { document_id, .. })
                if document_id == home_id
        ));
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
        document_id: DocumentId,
        page_id: Option<PageId>,
        blocks: Vec<BlockState>,
        memberships: Vec<(BlockId, MembershipClaim)>,
    ) -> SemanticDocumentSnapshot {
        SemanticDocumentSnapshot::Shard {
            page_id,
            page_preamble: page_id.map(|page_id| PagePreambleState {
                page_id,
                home_document_id: document_id,
                preamble: None,
            }),
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
                    shard_snapshot(home_id, Some(page_id), before_blocks, before_memberships),
                ),
            ]),
            BTreeMap::from([
                (catalog_id, catalog_snapshot(after_pages)),
                (
                    home_id,
                    shard_snapshot(home_id, Some(page_id), after_blocks, after_memberships),
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
                    low_document_id,
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
                    high_document_id,
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
                    low_document_id,
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
                    high_document_id,
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
                    low_document_id,
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
                    high_document_id,
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
        let before = BTreeMap::from([(
            document_id,
            shard_snapshot(document_id, None, Vec::new(), Vec::new()),
        )]);
        let after = BTreeMap::from([(
            document_id,
            shard_snapshot(
                document_id,
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
            shard_snapshot(document_id, Some(page_id), Vec::new(), Vec::new()),
        )]);
        let stable_before = BTreeMap::from([(
            document_id,
            shard_snapshot(
                document_id,
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            engine.prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
                .prepare_bootstrap_transaction(
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
                engine.prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
                .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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
                .prepare_bootstrap_transaction(
                    author,
                    &OperationTransaction::new(operations).unwrap(),
                )
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
