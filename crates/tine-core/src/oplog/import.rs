//! Exact, read-only external inventory and conservative identity matching.
//!
//! This module plans reconciliation only. It does not publish semantic
//! operations, write a graph, consult SQLite, or activate managed sync.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::hot_engine::AcceptedFrontierRoot;
use super::{
    plan_projection, AnnotatedIdentity, BlobDescription, BlockId, ContentDigest, CurrentPageAtPath,
    ImportId, ImportInventoryEntry, ImportInventoryState, LogicalCompletionId, LogseqUuid,
    ManagedPath, PageId, ProjectionCompletion, ProjectionIntent, ProjectionReceiptStore,
    ShardedHotEngine, StructuralLocator, WorkspaceId, DIFF_SCHEMA_VERSION,
};
use crate::doc::Document;
use crate::model::{path_is_sync_conflict, Graph};

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

    fn derivation_entries(&self) -> Vec<ImportInventoryEntry> {
        self.entries
            .iter()
            .map(|(path, observation)| {
                ImportInventoryEntry::new(
                    path.clone(),
                    match observation {
                        RawObservation::Present(bytes) => {
                            ImportInventoryState::Present(bytes.description())
                        }
                        RawObservation::Absent => ImportInventoryState::Absent,
                    },
                )
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

fn require_default_layout(graph: &Graph) -> Result<(), InventoryError> {
    let (pages_directory, journals_directory) = graph.raw_managed_text_layout();
    if pages_directory == "pages" && journals_directory == "journals" {
        Ok(())
    } else {
        Err(InventoryError::UnsupportedManagedLayout {
            pages_directory: pages_directory.to_owned(),
            journals_directory: journals_directory.to_owned(),
        })
    }
}

/// Read only the explicitly named affected paths. No directory enumeration is
/// performed, including when a requested path is absent.
pub fn inventory_affected(
    graph: &Graph,
    requested_paths: &[&str],
) -> Result<RawInventory, InventoryError> {
    require_default_layout(graph)?;
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
    require_default_layout(graph)?;
    let captured = graph
        .initial_shadow_raw_managed_text_inventory()
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InventoryPathFingerprint {
    state: ImportInventoryState,
    file_resource_id: Option<ContentDigest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CatalogAuthority {
    generation: u64,
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
    let paths = match parse_requested_paths(graph, requested_paths) {
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
    let (catalog, catalog_authority) = match capture_catalog(receipts, &mut instrumentation) {
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
    let (_, post_catalog_authority) = match capture_catalog(receipts, &mut instrumentation) {
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
                "inventory, receipt catalog, or accepted frontier changed between snapshot passes",
            ),
            instrumentation,
        );
    };
    // Equal bounded collections detect stale diagnostic input only under the
    // explicit quiescent-writer boundary. They are neither a portable atomic
    // filesystem snapshot nor authority for later publication.
    plan_import(inventory, scope, instrumentation)
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

fn parse_requested_paths(
    graph: &Graph,
    requested_paths: &[&str],
) -> Result<Vec<ManagedPath>, InventoryError> {
    require_default_layout(graph)?;
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

fn capture_catalog(
    receipts: &ProjectionReceiptStore,
    instrumentation: &mut ImportInstrumentation,
) -> Result<
    (
        Vec<super::projection_store::ProjectionCatalogEntry>,
        CatalogAuthority,
    ),
    ImportBlock,
> {
    let catalog = receipts.validated_catalog().map_err(|error| {
        authority_block(
            if matches!(&error, super::ProjectionStoreError::EvidenceTooLarge { .. }) {
                ImportBlockReason::ResourceLimit
            } else {
                ImportBlockReason::CorruptBase
            },
            None,
            format!("durable receipt catalog is invalid: {error}"),
        )
    })?;
    if catalog.len() > MAX_IMPORT_CATALOG_ENTRIES {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            None,
            format!(
                "receipt catalog entry budget exceeded: observed {}, limit {}",
                catalog.len(),
                MAX_IMPORT_CATALOG_ENTRIES
            ),
        ));
    }
    instrumentation.catalog_entries = instrumentation
        .catalog_entries
        .saturating_add(catalog.len());
    let mut hasher = Sha256::new();
    hasher.update(b"tine/import-receipt-catalog-snapshot/v1\0");
    hasher.update(receipts.store_id().as_bytes());
    for entry in &catalog {
        let intent = entry.intent.encode().map_err(|error| {
            authority_block(ImportBlockReason::CorruptBase, None, error.to_string())
        })?;
        instrumentation.catalog_bytes_hashed = instrumentation
            .catalog_bytes_hashed
            .saturating_add(intent.len() as u64);
        hasher.update((intent.len() as u64).to_be_bytes());
        hasher.update(&intent);
        match &entry.completion {
            Some(completion) => {
                let completion = completion.encode().map_err(|error| {
                    authority_block(ImportBlockReason::CorruptBase, None, error.to_string())
                })?;
                instrumentation.catalog_bytes_hashed = instrumentation
                    .catalog_bytes_hashed
                    .saturating_add(completion.len() as u64);
                hasher.update([1]);
                hasher.update((completion.len() as u64).to_be_bytes());
                hasher.update(completion);
            }
            None => hasher.update([0]),
        }
    }
    let authority = CatalogAuthority {
        generation: catalog.len().saturating_add(
            catalog
                .iter()
                .filter(|entry| entry.completion.is_some())
                .count(),
        ) as u64,
        digest: ContentDigest::from_bytes(hasher.finalize().into()),
    };
    Ok((catalog, authority))
}

fn capture_import_scope(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    requested_paths: &[ManagedPath],
    catalog: Vec<super::projection_store::ProjectionCatalogEntry>,
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

    let mut catalog_by_path = BTreeMap::<ManagedPath, Vec<_>>::new();
    for entry in catalog {
        instrumentation.catalog_path_inserts =
            instrumentation.catalog_path_inserts.saturating_add(1);
        catalog_by_path
            .entry(entry.intent.path().clone())
            .or_default()
            .push(entry);
    }

    let mut paths = BTreeMap::new();
    for path in requested_paths {
        instrumentation.catalog_path_lookups =
            instrumentation.catalog_path_lookups.saturating_add(1);
        let catalog_entries = catalog_by_path.get(path).map(Vec::as_slice).unwrap_or(&[]);
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
                    {
                        continue;
                    }
                    let authority = receipts
                        .completed_work_authority(&work, &entry.intent)
                        .map_err(|error| {
                            authority_block(
                                ImportBlockReason::ConflictingLocalTail,
                                Some(path),
                                format!("released path completion is unavailable: {error}"),
                            )
                        })?;
                    work_index.require_completed(&authority).map_err(|error| {
                        authority_block(
                            ImportBlockReason::ConflictingLocalTail,
                            Some(path),
                            format!("released path completion is stale: {error}"),
                        )
                    })?;
                    let logical = entry
                        .completion
                        .as_ref()
                        .expect("completion authority requires completion")
                        .logical_completion_id();
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
        let completion = match &entry.completion {
            Some(completion) => {
                completion
                    .validate_against(&entry.intent)
                    .map_err(|error| {
                        authority_block(
                            ImportBlockReason::CorruptBase,
                            Some(path),
                            error.to_string(),
                        )
                    })?;
                completion.clone()
            }
            None => {
                return Err(authority_block(
                    ImportBlockReason::MissingBase,
                    Some(path),
                    "current accepted projection has no durable Graph-proved completion",
                ));
            }
        };
        paths.insert(
            path.clone(),
            ScopedPathEvidence::Existing(ReceiptBackedPage {
                intent: entry.intent.clone(),
                completion,
                replayed_target: ExactBytes::from_description(
                    replayed_target,
                    entry.intent.target(),
                ),
            }),
        );
    }

    Ok(ImportScopeSnapshot {
        workspace_id: engine.workspace_id(),
        paths,
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
    mut instrumentation: ImportInstrumentation,
) -> ImportPlan {
    if scope.paths.len() != inventory.entries().len()
        || scope
            .paths
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
    let import_id = match ImportId::derive(
        scope.workspace_id,
        &completion_ids,
        &inventory.derivation_entries(),
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
    ImportPlan {
        status: if changed {
            ImportPlanStatus::Reconcile
        } else {
            ImportPlanStatus::Noop
        },
        import_id: Some(import_id),
        inventory: Some(inventory),
        matches: Some(matches),
        blocks: Vec::new(),
        instrumentation,
    }
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
    raw: String,
    raw_ids: Vec<String>,
}

struct ParsedTree {
    path: ManagedPath,
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
    let document = if path.as_str().ends_with(".org") {
        crate::org::parse_org(text)
    } else {
        crate::doc::parse(text)
    };
    flatten_document(path, &document, instrumentation)
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
    document: &Document,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedTree, ImportBlock> {
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
        nodes.push(ParsedNode {
            parent,
            sibling_position,
            depth,
            children: Vec::with_capacity(block.children.len()),
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
    Ok(ParsedTree {
        path: path.clone(),
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
        ProjectionEndpointBinding, ProjectionEndpointId, SemanticOperation, SessionId,
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
    }

    impl SnapshotFixture {
        fn new(label: &str, paths: &[&str]) -> Self {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            let graph = Graph::open(&graph_root);
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
                let kind = match path.split_once('/') {
                    Some(("pages", rest)) if !rest.is_empty() => ManagedTextKind::Page,
                    Some(("journals", rest)) if !rest.is_empty() => ManagedTextKind::Journal,
                    _ => panic!("snapshot fixture path is outside the guarded default layout"),
                };
                page_ids.push(page_id);
                operations.push(SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    name: crate::oplog::LogicalPageName::parse(format!("Snapshot Page {index}"))
                        .unwrap(),
                    path: ManagedPath::parse((*path).to_owned()).unwrap(),
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
                    content: format!("page {index}"),
                });
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
            roots: vec![0],
            nodes: vec![ParsedNode {
                parent: None,
                sibling_position: 0,
                depth: 1,
                children: Vec::new(),
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
                raw: "node".into(),
                raw_ids: vec!["duplicate".into(), "duplicate".into()],
            });
        }
        let deep = ParsedTree {
            path,
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
                    roots: vec![0],
                    nodes: vec![ParsedNode {
                        parent: None,
                        sibling_position: 0,
                        depth: 1,
                        children: Vec::new(),
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
