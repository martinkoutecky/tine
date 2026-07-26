//! Exact, read-only external inventory and conservative identity matching.
//!
//! This module plans reconciliation only. It does not publish semantic
//! operations, write a graph, consult SQLite, or activate managed sync.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::external_import::{
    ExternalImportObservationEntry, ExternalImportObservationMaterial,
    ExternalImportObservationMaterialError, ExternalImportObservationState,
};
use super::hot_engine::{AcceptedFrontierRoot, MAX_TRANSACTION_OPERATIONS};
use super::{
    plan_projection, AnnotatedIdentity, BatchId, BatchOrigin, BlobDescription, BlockId,
    BlockLocation, ContentDigest, CurrentPageAtPath, DocumentId, ImportId, ImportInventoryEntry,
    ImportInventoryState, ImportLocator, LogicalCompletionId, LogicalPageName,
    LogseqIdentityMutation, LogseqUuid, ManagedPath, ManagedTextKind, OperationTransaction, PageId,
    ProjectionCompletedReceipt, ProjectionCompletion, ProjectionIntent, ProjectionReceiptStore,
    ProjectionStoreError, SemanticOperation, ShardedHotEngine, StructuralLocator, StructuralSpan,
    WorkspaceId, DIFF_SCHEMA_VERSION,
};
use crate::model::{path_is_sync_conflict, Graph, PageKind};

#[cfg(test)]
thread_local! {
    static SNAPSHOT_REVALIDATION_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static POST_FRONTIER_OVERRIDE:
        std::cell::RefCell<Option<AcceptedFrontierRoot>> = const { std::cell::RefCell::new(None) };
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
    completed: ProjectionCompletedReceipt,
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
            .saturating_add(self.rejected_raw_id_occurrences)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageMatchBasis {
    SamePathCompletion,
    ReceiptBackedExactRename,
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
        match capture_affected_catalog(receipts, engine, &paths, &mut instrumentation) {
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
        match capture_affected_catalog(receipts, engine, &paths, &mut instrumentation) {
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
    plan_import(inventory, scope, engine, instrumentation)
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
                completed,
                intent,
                completion,
            });
            captured_entries = captured_entries.saturating_add(1);
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
                let work = engine
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
                        || entry.intent.frontier() != work.post_frontier()
                        || entry.intent.target() != BlobDescription::of(&[])
                        || entry.completed.page_id() != work.page_id()
                        || entry.completed.frontier() != work.post_frontier()
                        || entry.completed.target() != super::ProjectionWorkTarget::Absent
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

        let mut exact = None;
        let mut replay_cache =
            BTreeMap::<Option<BlobDescription>, (ProjectionIntent, Vec<u8>)>::new();
        for entry in catalog_entries {
            if entry.intent.workspace_id() != engine.workspace_id()
                || entry.intent.page_id() != page_id
                || entry.intent.frontier() != &current.state().frontier
                || entry.intent.claim_evidence() != current.state().claim_evidence
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
                let base = receipts.load_base(&entry.intent).map_err(|error| {
                    authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        format!("canonical base evidence is unavailable: {error}"),
                    )
                })?;
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
            if replayed_intent == &entry.intent {
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
    inventory: RawInventory,
    scope: ImportScopeSnapshot,
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
    if let Err(block) = match_blocks(&inventory, &completed, &mut matches, &mut instrumentation) {
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }

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
    let derivation_entries = match inventory.derivation_entries(&scope.path_identities) {
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

    let page_transition =
        match build_desired_page_transition(&inventory, &matches, &scope, import_id) {
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
    let execution = if changed {
        match build_execution_material(
            import_id,
            &inventory,
            &matches,
            &scope,
            &page_transition,
            &mut instrumentation,
        ) {
            Ok(execution) => Some(execution),
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
        None
    };
    ImportPlan {
        status: if changed {
            ImportPlanStatus::Reconcile
        } else {
            ImportPlanStatus::Noop
        },
        import_id: Some(import_id),
        inventory: Some(inventory),
        matches: Some(matches),
        scope: changed.then_some(scope),
        execution,
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

fn build_desired_page_transition(
    inventory: &RawInventory,
    matches: &ImportMatches,
    scope: &ImportScopeSnapshot,
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
        let path_identity = scope.path_identities.get(path).ok_or_else(|| {
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
    instrumentation: &mut ImportInstrumentation,
) -> Result<ImportExecutionMaterial, ImportExecutionError> {
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

    let mut trees = BTreeMap::<ManagedPath, ParsedTree>::new();
    for (path, observation) in inventory.entries() {
        let RawObservation::Present(bytes) = observation else {
            continue;
        };
        if std::str::from_utf8(bytes.bytes()).is_err() {
            return Err(ImportExecutionError::InvalidMaterial(format!(
                "sealed inventory path {path} is not valid UTF-8"
            )));
        }
        trees.insert(
            path.clone(),
            parse_nodes(path, bytes.bytes(), instrumentation)
                .map_err(|block| ImportExecutionError::InvalidMaterial(block.detail))?,
        );
    }

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
        let current_kind = scope
            .path_identities
            .get(&current.path)
            .ok_or(ImportExecutionError::IncompletePlan(
                "sealed existing page has no Graph-decoded current managed kind",
            ))?
            .kind;
        if current.name != page.name || current.path != page.path || current_kind != page.kind {
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

    let transaction = OperationTransaction::new(operations)
        .map_err(|error| ImportExecutionError::InvalidMaterial(error.to_string()))?;
    Ok(ImportExecutionMaterial {
        import_id,
        transaction,
        observation,
    })
}

fn imported_order(sibling_position: u32) -> String {
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

fn parse_nodes(
    path: &ManagedPath,
    bytes: &[u8],
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedTree, ImportBlock> {
    let text = std::str::from_utf8(bytes).expect("UTF-8 checked before semantic parsing");
    preflight_depth(path, text, instrumentation.parsed_nodes)?;
    let parsed = if path.as_str().ends_with(".org") {
        crate::org::parse_org_with_source_spans(text)
    } else {
        crate::doc::parse_with_source_spans(text)
    };
    flatten_document(path, parsed, instrumentation)
}

fn preflight_depth(path: &ManagedPath, text: &str, parsed_nodes: usize) -> Result<(), ImportBlock> {
    let mut candidate_nodes = 0_usize;
    for line in text.lines() {
        let is_org = path.as_str().ends_with(".org");
        let depth = if is_org {
            let stars = line
                .as_bytes()
                .iter()
                .take_while(|byte| **byte == b'*')
                .count();
            if stars > 0 && line.as_bytes().get(stars) == Some(&b' ') {
                candidate_nodes = candidate_nodes.saturating_add(1);
            }
            stars
        } else {
            let tabs = line
                .as_bytes()
                .iter()
                .take_while(|byte| **byte == b'\t')
                .count();
            let spaces = line
                .as_bytes()
                .iter()
                .skip(tabs)
                .take_while(|byte| **byte == b' ')
                .count();
            let content = &line[tabs + spaces..];
            if content == "-" || content.starts_with("- ") {
                candidate_nodes = candidate_nodes.saturating_add(1);
            }
            tabs.saturating_add(spaces / 2).saturating_add(1)
        };
        if depth > MAX_IMPORT_DEPTH {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!(
                    "document nesting depth exceeds import limit {MAX_IMPORT_DEPTH} before parsing"
                ),
            ));
        }
    }
    let observed = parsed_nodes.saturating_add(candidate_nodes);
    if observed > MAX_IMPORT_PARSED_NODES {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "parsed-node budget would be exceeded before parsing: observed {observed}, limit {MAX_IMPORT_PARSED_NODES}"
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
    inventory: &RawInventory,
    completed: &[&ReceiptBackedPage],
    matches: &mut ImportMatches,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    let mut external_by_path = BTreeMap::<ManagedPath, ParsedTree>::new();
    for (path, observation) in inventory.entries() {
        if let RawObservation::Present(bytes) = observation {
            external_by_path.insert(
                path.clone(),
                parse_nodes(path, bytes.bytes(), instrumentation)?,
            );
        }
    }
    let mut base_by_path = BTreeMap::<ManagedPath, ParsedTree>::new();
    for page in completed {
        base_by_path.insert(
            page.path().clone(),
            parse_nodes(page.path(), page.bytes(), instrumentation)?,
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
    Ok(())
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

    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        write_projection_exact, AuthorBatch, BatchId, BlockLocation, CrdtPeerId, DeviceId,
        DocumentId, LineageDigest, ManagedTextKind, ObjectStore, OperationTransaction,
        PortablePathIndexRoot, ProjectionEndpointBinding, ProjectionEndpointId, SemanticOperation,
        SessionId,
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
            Self::new_with_initial_uuid_and_config(label, paths, None, None)
        }

        fn new_with_initial_uuid(
            label: &str,
            paths: &[&str],
            initial_uuid: Option<LogseqUuid>,
        ) -> Self {
            Self::new_with_initial_uuid_and_config(label, paths, initial_uuid, None)
        }

        fn new_with_graph_config(label: &str, paths: &[&str], config: &str) -> Self {
            Self::new_with_initial_uuid_and_config(label, paths, None, Some(config))
        }

        fn new_with_initial_uuid_and_config(
            label: &str,
            paths: &[&str],
            initial_uuid: Option<LogseqUuid>,
            config: Option<&str>,
        ) -> Self {
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
            let author = ShardedHotEngine::new(workspace, lineage, catalog);
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
                    name: crate::oplog::LogicalPageName::parse(format!("Snapshot Page {index}"))
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
                    content: match (index, initial_uuid) {
                        (0, Some(logseq_uuid)) => format!("page {index}\nid:: {logseq_uuid}"),
                        _ => format!("page {index}"),
                    },
                });
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
            let prepared = author
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id,
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(6)),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(7)),
                        crdt_peer_id: CrdtPeerId::from_u64(8),
                    },
                    &transaction,
                )
                .unwrap();
            let archive = root.path().join("archive");
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
    fn promoted_collapsed_preamble_heading_has_an_exact_parser_owned_span() {
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
        assert_eq!(tree.roots, vec![0]);
        assert_eq!(tree.nodes[0].children, vec![1, 2]);
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
            capture_affected_catalog(&receipts, &reopened, &requested, &mut instrumentation)
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
        assert_eq!(
            preflight_depth(&path, "- one more\n", MAX_IMPORT_PARSED_NODES)
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
}
