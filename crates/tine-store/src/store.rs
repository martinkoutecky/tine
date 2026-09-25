//! Interim read boundary. `Store` owns the legacy graph. `page` reads and
//! parses one page in O(page bytes + its blocks), updating the live cache when
//! it observes an external edit. `read` returns raw bytes in O(file bytes),
//! optionally bounded by `max_bytes`; `open_read` returns an open handle and
//! length in O(1) metadata; `path_for_os_handoff` validates an absolute path
//! for an OS opener in O(1) metadata. Invalid identities, missing files,
//! undecodable pages, oversized reads, and I/O errors are typed `StoreError`s.
//! Callers need no cache state, disk layout, or path for file reads.
//!
//! Whole-graph questions below read the live cache. A first call can build
//! that cache in O(P + B + disk). The selected reads are bounded here.
//! `WholeGraph` does not yet pin an immutable generation: two calls on one view
//! may observe different states. Immutable snapshots arrive with B7.
//! `trash_stats` scans recoverable entries in O(trash entries), returning typed
//! counts and bytes or an I/O error. `purge_asset_trash` irreversibly removes
//! asset and legacy-asset entries in O(asset trash entries + metadata), leaving
//! other kinds intact. On an error it returns completed removal counts and
//! bytes. Callers need no trash layout or legacy name classifier.
//!
//! Questions and costs after cache construction (`P` pages, `B` blocks):
//! `rev` O(1); `backlinks`, `unlinked_references`, `block_referrers`,
//! `block_ref_counts`, `find_blocks`, `property_facets`, and `templates` O(B)
//! worst case; `backlink_filter_context` O(B) with selected roots;
//! `blocks` and `preview_block` O(blocks of hinted pages), O(B) without a hint;
//! `complete_page_names` O(P + aliases + referenced names);
//! `export_query_subtrees` O(64 × B + selected nodes); `page_icons`
//! O(names + aliases); `journal_content_days` O(journals + their blocks).
//! Calls that exceed fixed request or result limits return `QueryError`; a
//! cancelled block search returns `Cancelled`, never a partial answer. Callers
//! need no cache state, budget constants, lane IDs, or disk paths.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::model::{classify_legacy_trash_entry, trash_dir_kind, trash_root, TrashEntryKind};

use serde::{Deserialize, Serialize};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockPreview, BoundedRefGroups, PageDto,
    PageEntry, PageKind, RefGroup, TemplateDto,
};
pub use tine_core::model::{FileId, PageId};
use tine_core::query::{AdvancedResult, QueryExportBatch, QueryExportSpec};
use tine_core::query_plan::QueryExecution;

use crate::model::Graph;

const RESULT_BRIDGE_MAX_ROWS: usize = 20_000;
const RESULT_BRIDGE_MAX_BYTES: usize = 32 * 1024 * 1024;
const AUTOCOMPLETE_FACET_MAX_ITEMS: usize = 2_000;
const AUTOCOMPLETE_FACET_MAX_BYTES: usize = 2 * 1024 * 1024;
const QUERY_EXPORT_MAX_QUERIES: usize = 64;
const QUERY_EXPORT_REQUEST_MAX_QUERIES: usize = 1_024;
const QUERY_EXPORT_MAX_QUERY_BYTES: usize = 64 * 1024;
const QUERY_EXPORT_MAX_ROOTS: usize = 50;
const QUERY_EXPORT_MAX_NODES: usize = 2_000;
const QUERY_EXPORT_MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PREVIEW_NODES: usize = 2_000;
const PREVIEW_MAX_BYTES: usize = RESULT_BRIDGE_MAX_BYTES - 4 * 1024;

/// Interim owner of a legacy graph. Constructing it is O(1); reads can build
/// the whole cache in O(P + B + disk) on first use.
pub struct Store {
    graph: Arc<Graph>,
}

/// Trash categories. Legacy covers entries with no recognized recoverable type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrashKind {
    Asset,
    Page,
    Journal,
    Conflict,
    Legacy,
}

impl From<TrashEntryKind> for TrashKind {
    fn from(kind: TrashEntryKind) -> Self {
        match kind {
            TrashEntryKind::Asset => Self::Asset,
            TrashEntryKind::Page => Self::Page,
            TrashEntryKind::Journal => Self::Journal,
            TrashEntryKind::Conflict => Self::Conflict,
            TrashEntryKind::Other => Self::Legacy,
        }
    }
}

fn add_trash_count(counts: &mut [(u64, u64); 5], kind: TrashKind, bytes: u64) {
    let index = match kind {
        TrashKind::Asset => 0,
        TrashKind::Page => 1,
        TrashKind::Journal => 2,
        TrashKind::Conflict => 3,
        TrashKind::Legacy => 4,
    };
    counts[index].0 += 1;
    counts[index].1 += bytes;
}

fn trash_counts(counts: [(u64, u64); 5]) -> Vec<(TrashKind, u64, u64)> {
    [
        TrashKind::Asset,
        TrashKind::Page,
        TrashKind::Journal,
        TrashKind::Conflict,
        TrashKind::Legacy,
    ]
    .into_iter()
    .zip(counts)
    .map(|(kind, (count, bytes))| (kind, count, bytes))
    .collect()
}

fn trash_entry_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    let kind = fs::symlink_metadata(path)?.file_type();
    if kind.is_file() {
        return Ok(fs::metadata(path)?.len());
    }
    if !kind.is_dir() {
        return Ok(0);
    }
    let mut bytes = 0;
    for child in fs::read_dir(path)? {
        bytes += trash_entry_bytes(&child?.path())?;
    }
    Ok(bytes)
}

impl Store {
    /// Adopt the current graph without loading it. O(1). Removed in B7.
    pub fn from_legacy(graph: Arc<Graph>) -> Self {
        Self { graph }
    }

    /// Count entries and bytes by kind in the recoverable trash. Cost: O(trash entries).
    pub fn trash_stats(&self) -> Result<Vec<(TrashKind, u64, u64)>, StoreError> {
        let trash = trash_root(&self.graph.root);
        let mut counts = [(0, 0); 5];
        let entries = match fs::read_dir(trash) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(trash_counts(counts))
            }
            Err(error) => return Err(StoreError::from_io(error)),
        };
        for entry in entries {
            let entry = entry.map_err(StoreError::from_io)?;
            let kind = entry.file_type().map_err(StoreError::from_io)?;
            if kind.is_dir() {
                if let Some(typed) = trash_dir_kind(&entry.path()) {
                    for child in fs::read_dir(entry.path()).map_err(StoreError::from_io)? {
                        let child = child.map_err(StoreError::from_io)?;
                        let file_type = child.file_type().map_err(StoreError::from_io)?;
                        let bytes = if file_type.is_file() {
                            child.metadata().map_err(StoreError::from_io)?.len()
                        } else {
                            0
                        };
                        add_trash_count(&mut counts, TrashKind::from(typed), bytes);
                    }
                } else {
                    add_trash_count(&mut counts, TrashKind::Legacy, 0);
                }
            } else {
                let bytes = if kind.is_file() {
                    entry.metadata().map_err(StoreError::from_io)?.len()
                } else {
                    0
                };
                add_trash_count(
                    &mut counts,
                    TrashKind::from(classify_legacy_trash_entry(&entry.path(), kind)),
                    bytes,
                );
            }
        }
        Ok(trash_counts(counts))
    }

    /// Permanently remove asset and legacy-asset entries, including directories.
    /// Other kinds stay recoverable. On error, returns counts already removed.
    /// Cost: O(asset trash entries + bytes).
    pub fn purge_asset_trash(&self) -> Result<(u64, u64), (StoreError, u64, u64)> {
        let trash = trash_root(&self.graph.root);
        self.graph
            .ensure_write_target(&trash)
            .map_err(|error| (StoreError::from_io(error), 0, 0))?;
        let mut removed = (0, 0);
        let entries = match fs::read_dir(trash) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
            Err(error) => return Err((StoreError::from_io(error), 0, 0)),
        };
        for entry in entries {
            let entry =
                entry.map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
            let file_type = entry
                .file_type()
                .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
            if file_type.is_dir() {
                if trash_dir_kind(&entry.path()) != Some(TrashEntryKind::Asset) {
                    continue;
                }
                let assets = fs::read_dir(entry.path())
                    .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                for asset in assets {
                    let asset = asset
                        .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                    let kind = asset
                        .file_type()
                        .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                    let bytes = trash_entry_bytes(&asset.path())
                        .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                    let result = if kind.is_dir() {
                        fs::remove_dir_all(asset.path())
                    } else {
                        fs::remove_file(asset.path())
                    };
                    result.map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                    removed.0 += 1;
                    removed.1 += bytes;
                }
            } else if classify_legacy_trash_entry(&entry.path(), file_type) == TrashEntryKind::Asset
            {
                let bytes = entry
                    .metadata()
                    .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?
                    .len();
                fs::remove_file(entry.path())
                    .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                removed.0 += 1;
                removed.1 += bytes;
            }
        }
        Ok(removed)
    }

    /// Type a file name within one configured graph area. Validation repeats
    /// whenever an id is used, including after deserialization.
    pub fn file_id(&self, area: Area, rel: &str) -> Result<FileId, StoreError> {
        let directory = match area {
            Area::Pages => &self.graph.config.pages_dir,
            Area::Journals => &self.graph.config.journals_dir,
            Area::Assets => "assets",
            Area::Meta => "logseq",
            Area::Trash => "logseq/.tine-trash",
        };
        let id = FileId::from(format!("{directory}/{rel}"));
        self.validate_file(&id)?;
        Ok(id)
    }

    pub fn as_page(&self, file: &FileId) -> Option<PageId> {
        self.validate_file(file).ok()?;
        let path = file.as_str();
        if !path.starts_with(&format!("{}/", self.graph.config.pages_dir))
            && !path.starts_with(&format!("{}/", self.graph.config.journals_dir))
        {
            return None;
        }
        let stem = std::path::Path::new(path).file_stem()?.to_str()?;
        if !matches!(
            std::path::Path::new(path).extension()?.to_str(),
            Some("md" | "org")
        ) {
            return None;
        }
        if tine_core::model::is_sync_conflict(stem) {
            return None;
        }
        Some(PageId::from(path))
    }

    fn validate_file(&self, file: &FileId) -> Result<(), StoreError> {
        let path = file.as_str();
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        if !path.starts_with(&format!("{}/", self.graph.config.pages_dir))
            && !path.starts_with(&format!("{}/", self.graph.config.journals_dir))
            && !path.starts_with("assets/")
            && !path.starts_with("logseq/")
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        Ok(())
    }

    fn area_root(&self, file: &FileId) -> Result<PathBuf, StoreError> {
        self.validate_file(file)?;
        let path = file.as_str();
        let area = if path.starts_with(&format!("{}/", self.graph.config.pages_dir)) {
            self.graph.config.pages_dir.as_str()
        } else if path.starts_with(&format!("{}/", self.graph.config.journals_dir)) {
            self.graph.config.journals_dir.as_str()
        } else {
            path.split('/').next().unwrap_or_default()
        };
        Ok(self.graph.root.join(area))
    }

    /// A validated OS path. A missing final file is allowed; existing ancestors
    /// must still remain inside the selected area. Callers requiring an existing
    /// regular file must check that separately.
    pub fn path_for_os_handoff(&self, file: &FileId) -> Result<PathBuf, StoreError> {
        let area = self.area_root(file)?;
        let (area, candidate) = if let Some(rel) = file.as_str().strip_prefix("assets/") {
            let approved = self.graph.assets_path();
            let live =
                fs::canonicalize(self.graph.root.join("assets")).map_err(StoreError::from_io)?;
            if live != approved {
                return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
            }
            (approved.clone(), approved.join(rel))
        } else {
            (area, self.graph.root.join(file.as_str()))
        };
        let area_canonical = fs::canonicalize(&area).map_err(StoreError::from_io)?;
        let mut existing = candidate.as_path();
        loop {
            match fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    existing = existing
                        .parent()
                        .ok_or_else(|| StoreError::InvalidTarget(file.as_str().to_owned()))?;
                }
                Err(error) => return Err(StoreError::from_io(error)),
            }
        }
        let resolved = fs::canonicalize(existing).map_err(StoreError::from_io)?;
        if !resolved.starts_with(&area_canonical) {
            return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
        }
        let suffix = candidate
            .strip_prefix(existing)
            .expect("candidate ancestor");
        if suffix.as_os_str().is_empty() {
            Ok(resolved)
        } else {
            Ok(resolved.join(suffix))
        }
    }

    /// Read one file's bytes, with an optional limit checked before and after
    /// reading. Cost: O(file bytes).
    pub fn read(
        &self,
        file: &FileId,
        max_bytes: Option<u64>,
    ) -> Result<(Vec<u8>, FileRev), StoreError> {
        let path = self.path_for_os_handoff(file)?;
        let mut input = File::open(path).map_err(StoreError::from_io)?;
        let meta = input.metadata().map_err(StoreError::from_io)?;
        if !meta.is_file() {
            return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
        }
        let len = meta.len();
        if let Some(limit) = max_bytes {
            if len > limit {
                return Err(StoreError::TooLarge { limit, len });
            }
        }
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).map_err(StoreError::from_io)?;
        if let Some(limit) = max_bytes {
            if bytes.len() as u64 > limit {
                return Err(StoreError::TooLarge {
                    limit,
                    len: bytes.len() as u64,
                });
            }
        }
        let rev = FileRev::from_bytes(&bytes);
        Ok((bytes, rev))
    }

    /// Open a validated file for streaming and return its length. The final
    /// component must not be a symlink. Cost: O(1) metadata.
    pub fn open_read(&self, file: &FileId) -> Result<(File, u64), StoreError> {
        self.validate_file(file)?;
        let path = self.path_for_os_handoff(file)?;
        let raw = if let Some(rel) = file.as_str().strip_prefix("assets/") {
            self.graph.assets_path().join(rel)
        } else {
            self.graph.root.join(file.as_str())
        };
        if fs::symlink_metadata(&raw)
            .map_err(StoreError::from_io)?
            .file_type()
            .is_symlink()
        {
            return Err(StoreError::InvalidTarget(format!(
                "symlink:{}",
                file.as_str()
            )));
        }
        let input = File::open(path).map_err(StoreError::from_io)?;
        let meta = input.metadata().map_err(StoreError::from_io)?;
        if !meta.is_file() {
            return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
        }
        Ok((input, meta.len()))
    }

    /// Read and parse one page. This can advance the live cache when its bytes
    /// differ from the cached copy (interim behavior, before immutable D3).
    /// Cost: O(page bytes + its blocks).
    pub fn page(&self, id: &PageId) -> Result<PageRead, StoreError> {
        if self.as_page(&id.file()).is_none() {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        // v0.6.5's page walker never indexes a symlinked page file (it could
        // expose a file outside the graph), so no listing hands out such an
        // id; refuse one here too. Ancestors must stay inside the area. The
        // read itself uses the lexical path, which is the page's identity.
        let path = self.graph.root.join(id.as_str());
        self.path_for_os_handoff(&id.file())?;
        if fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        let entry = self
            .graph
            .entry_for_path(&path)
            .ok_or_else(|| StoreError::InvalidTarget(id.as_str().to_owned()))?;
        let canonical = self
            .graph
            .find_entry(&entry.name, entry.kind)
            .is_some_and(|found| found.path == path);
        let doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if canonical {
                self.graph.load_page(&entry).map(Some)
            } else {
                self.graph.load_by_validated_path(&path)
            }
        }))
        .map_err(|panic| {
            let reason = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "page parser panicked".to_owned());
            StoreError::Unparseable(reason)
        })?
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                StoreError::Undecodable
            } else {
                StoreError::from_io(error)
            }
        })?
        .ok_or(StoreError::NotFound)?;
        let rev = FileRev(doc.rev.clone().ok_or(StoreError::NotFound)?);
        let read_only = doc
            .read_only
            .then(|| "Org file does not round-trip".to_owned());
        Ok(PageRead {
            id: id.clone(),
            doc,
            rev,
            read_only,
        })
    }

    /// Get a live-cache read view and record its current generation. O(1).
    /// This interim implementation has no load failure or wait; the first
    /// question on the view may build the cache in O(P + B + disk).
    pub fn whole_graph(&self) -> Result<WholeGraph, LoadError> {
        Ok(WholeGraph {
            graph: Arc::clone(&self.graph),
            rev: GraphRev(self.graph.cache_generation()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Pages,
    Journals,
    Assets,
    Meta,
    Trash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Day(pub i64);

#[derive(Debug)]
pub enum StoreError {
    NotFound,
    InvalidTarget(String),
    Undecodable,
    Unparseable(String),
    TooLarge {
        limit: u64,
        len: u64,
    },
    Io(std::io::Error),
    /// Reserved for B7 lifecycle.
    Closed,
}

impl StoreError {
    fn from_io(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            _ => Self::Io(error),
        }
    }
}

#[cfg(test)]
#[test]
fn invalid_data_io_error_is_not_a_page_decode_error() {
    let error = std::io::Error::new(std::io::ErrorKind::InvalidData, "unrelated I/O data");
    assert!(matches!(StoreError::from_io(error), StoreError::Io(_)));
}

/// Opaque FNV-1a/64 content revision with the v0.6.5 hex string on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileRev(String);

impl FileRev {
    fn from_bytes(bytes: &[u8]) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(format!("{hash:016x}"))
    }
}

impl From<FileRev> for String {
    fn from(rev: FileRev) -> Self {
        rev.0
    }
}

pub struct PageRead {
    pub id: PageId,
    pub doc: PageDto,
    pub rev: FileRev,
    pub read_only: Option<String>,
}

pub enum Resolved {
    Existing { id: PageId, others: Vec<PageId> },
    Alias { owners: Vec<PageId> },
    Absent { id: PageId },
}
pub struct SearchRequest {
    pub text: String,
    pub within: Option<PageId>,
    pub page_limit: usize,
    pub block_limit: usize,
    pub explain: bool,
}
pub enum QueryDialect {
    Simple,
    Advanced,
}
pub enum QueryResult {
    Simple(Arc<Vec<RefGroup>>),
    Advanced(AdvancedResult),
}

/// Cache generation seen at `whole_graph()`. It is not a consistency guard.
/// Ordered and serialized as a decimal string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GraphRev(u64);

impl TryFrom<String> for GraphRev {
    type Error = std::num::ParseIntError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse().map(Self)
    }
}
impl From<GraphRev> for String {
    fn from(value: GraphRev) -> Self {
        value.0.to_string()
    }
}

/// Initial load failure. The interim `Store::whole_graph` cannot produce one.
#[derive(Debug)]
pub enum LoadError {
    /// Background load failed; introduced with B7.
    Failed { reason: String },
    /// Store closed; introduced with B7.
    Closed,
}

/// A whole-graph request failed before returning a partial answer.
#[derive(Debug)]
pub enum QueryError {
    InvalidTarget(String),
    /// Request exceeds the store's fixed input budget.
    RequestTooLarge {
        what: Budget,
        count: usize,
        limit: usize,
    },
    /// Export request exceeds the fixed macro or source-byte budget. Extra
    /// counts retain the existing frontend error text during this migration.
    ExportRequestTooLarge {
        macros: usize,
        bytes: usize,
        macro_limit: usize,
        byte_limit: usize,
        processing_cap: usize,
    },
    /// Evaluation reached the store's fixed result budget.
    ResultTooLarge {
        what: Budget,
        count: usize,
        limit: usize,
        bytes: Option<usize>,
        byte_limit: usize,
    },
    /// Query syntax error (reserved for later batches).
    Parse(String),
    /// Caller set the cancellation flag; no partial answer is returned.
    Cancelled,
}

#[derive(Clone, Copy, Debug)]
pub enum Budget {
    BacklinkFilterRoots,
    MatchingBlocks,
    BridgeMatchingBlocks,
    RequestedBlockRefs,
    ResolvedBlockRows,
    ExportBytes,
    PropertyFacets,
    AdvancedQueryMatches,
    SearchHits,
}

impl QueryError {
    /// Check the final transport estimate after the adapter has assembled groups.
    pub fn bridge_matching_blocks(rows: usize, bytes: usize) -> Option<Self> {
        (rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES).then_some(
            Self::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count: rows,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            },
        )
    }

    /// Check the final transport estimate after search serialization fields are known.
    pub fn bridge_search_hits(hits: usize, bytes: usize) -> Option<Self> {
        (hits > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES).then_some(
            Self::ResultTooLarge {
                what: Budget::SearchHits,
                count: hits,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            },
        )
    }
}

/// Caller-owned cancellation flag, checked before each search page and block.
pub struct Cancel(pub Arc<AtomicBool>);

/// Facet answer policy: reject oversized query-builder results or return the
/// editor's bounded autocomplete prefix. Both cost O(B) on a cold cache.
pub enum FacetPolicy {
    Budgeted,
    Truncated,
}

/// Live-cache graph-wide questions. Clone is O(1). A first question may build
/// the cache in O(P + B + disk); later calls do not wait for a load. This is
/// not yet an immutable snapshot: successive calls can see different states.
#[derive(Clone)]
pub struct WholeGraph {
    graph: Arc<Graph>,
    rev: GraphRev,
}

fn bounded(result: BoundedRefGroups, what: Budget) -> Result<Arc<Vec<RefGroup>>, QueryError> {
    if result.exceeded {
        Err(QueryError::ResultTooLarge {
            what,
            count: result.total,
            limit: RESULT_BRIDGE_MAX_ROWS,
            bytes: None,
            byte_limit: RESULT_BRIDGE_MAX_BYTES,
        })
    } else {
        Ok(result.groups)
    }
}

impl WholeGraph {
    /// Resolve a name using the configured file naming rules. Real files win
    /// before aliases; all claimants share the same deterministic order.
    pub fn resolve(&self, name: &str, is_journal: bool) -> Resolved {
        let kind = if is_journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let entries = self.graph.find_claimants(name, kind);
        if !entries.is_empty() {
            let mut ids = entries
                .into_iter()
                .map(|entry| entry.rel_path.expect("file claimant has a path"));
            return Resolved::Existing {
                id: ids.next().unwrap(),
                others: ids.collect(),
            };
        }
        if !is_journal {
            let owners: Vec<_> = self
                .graph
                .page_aliases_with_owners()
                .into_iter()
                .filter(|(alias, _, _)| tine_core::refs::same_page(alias, name))
                .map(|(_, _, path)| PageId::from(path))
                .collect();
            if !owners.is_empty() {
                return Resolved::Alias { owners };
            }
        }
        Resolved::Absent {
            id: PageId::from(self.graph.rel_path(&self.graph.path_for(name, kind))),
        }
    }

    fn validated_page(&self, id: &PageId) -> Result<(), QueryError> {
        let store = Store {
            graph: Arc::clone(&self.graph),
        };
        let file = id.file();
        if store.as_page(&file).is_none() {
            return Err(QueryError::InvalidTarget(id.as_str().to_owned()));
        }
        Ok(())
    }

    /// Execute one query macro with the same bounded evaluator as v0.6.5.
    /// The current evaluator ignores the current page; the id is validated.
    pub fn query(
        &self,
        source: &str,
        dialect: QueryDialect,
        current_page: Option<&PageId>,
    ) -> Result<QueryResult, QueryError> {
        if let Some(id) = current_page {
            self.validated_page(id)?;
        }
        match dialect {
            QueryDialect::Simple => bounded(
                self.graph.run_query_bounded(
                    source,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                ),
                Budget::MatchingBlocks,
            )
            .map(QueryResult::Simple),
            QueryDialect::Advanced => {
                let (result, exceeded, total) = self.graph.run_advanced_query_bounded_cached(
                    source,
                    None,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err(QueryError::ResultTooLarge {
                        what: Budget::AdvancedQueryMatches,
                        count: total,
                        limit: RESULT_BRIDGE_MAX_ROWS,
                        bytes: None,
                        byte_limit: RESULT_BRIDGE_MAX_BYTES,
                    })
                } else {
                    Ok(QueryResult::Advanced(result))
                }
            }
        }
    }

    /// Graph search, including an exact file scope and caller cancellation.
    pub fn search(
        &self,
        req: &SearchRequest,
        cancel: &Cancel,
    ) -> Result<QueryExecution, QueryError> {
        let scope = match &req.within {
            Some(id) => {
                self.validated_page(id)?;
                Some(crate::query_plan::QueryPageScope {
                    name: String::new(),
                    page_kind: PageKind::Page,
                    path: Some(id.as_str().to_owned()),
                })
            }
            None => None,
        };
        let page_limit = req.page_limit.min(RESULT_BRIDGE_MAX_ROWS);
        let block_limit = req.block_limit.min(RESULT_BRIDGE_MAX_ROWS - page_limit);
        let result = self.graph.run_graph_search_latest_scoped(
            cancel,
            &req.text,
            page_limit,
            block_limit,
            scope,
            req.explain,
        );
        if result.cancelled {
            Err(QueryError::Cancelled)
        } else {
            Ok(result)
        }
    }

    /// Cache generation at acquisition, O(1); later reads may see newer data.
    pub fn rev(&self) -> GraphRev {
        self.rev
    }

    /// Backlinks, worst case O(B), with early stop at fixed row/byte limits.
    pub fn backlinks(&self, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph
                .backlinks_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES),
            Budget::MatchingBlocks,
        )
    }

    /// Unlinked mentions, worst case O(B), with fixed row/byte limits.
    pub fn unlinked_references(&self, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph
                .unlinked_refs_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES),
            Budget::MatchingBlocks,
        )
    }

    /// Metadata for selected backlink roots, O(B) in the worst case. Refuses
    /// more than 20,000 targets before scanning.
    pub fn backlink_filter_context(
        &self,
        name: &str,
        targets: &[BacklinkFilterTarget],
    ) -> Result<BacklinkFilterContext, QueryError> {
        if targets.len() > RESULT_BRIDGE_MAX_ROWS {
            return Err(QueryError::RequestTooLarge {
                what: Budget::BacklinkFilterRoots,
                count: targets.len(),
                limit: RESULT_BRIDGE_MAX_ROWS,
            });
        }
        Ok(crate::query::backlink_filter_context(
            &self.graph,
            name,
            targets,
        ))
    }

    /// Resolve block identities in request order; unknown ids yield `None`.
    /// Cost: hinted pages' blocks, or O(B) for unhinted ids. Fixed result cap.
    pub fn blocks(&self, uuids: &[String]) -> Result<Vec<Option<RefGroup>>, QueryError> {
        if uuids.len() > RESULT_BRIDGE_MAX_ROWS {
            return Err(QueryError::ResultTooLarge {
                what: Budget::RequestedBlockRefs,
                count: uuids.len(),
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: None,
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            });
        }
        let (groups, exceeded, total) = crate::query::resolve_blocks_bounded(
            &self.graph,
            uuids,
            RESULT_BRIDGE_MAX_ROWS,
            RESULT_BRIDGE_MAX_BYTES,
        );
        if exceeded {
            Err(QueryError::ResultTooLarge {
                what: Budget::ResolvedBlockRows,
                count: total,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: None,
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            })
        } else {
            Ok(groups)
        }
    }

    /// Bounded subtree preview; 1..=2000 nodes and a fixed byte cap. Cost:
    /// one hinted page's blocks, or O(B) if no hint. Unknown id yields `None`.
    pub fn preview_block(
        &self,
        uuid: &str,
        max_nodes: usize,
    ) -> Result<Option<BlockPreview>, QueryError> {
        let preview = self.graph.preview_block_with_budget(
            uuid,
            max_nodes.clamp(1, MAX_PREVIEW_NODES),
            PREVIEW_MAX_BYTES,
        );
        if let Some(value) = &preview {
            let groups = std::slice::from_ref(&value.group);
            let rows = groups.iter().map(|g| g.blocks.len()).sum::<usize>();
            let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
            if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
                return Err(QueryError::ResultTooLarge {
                    what: Budget::BridgeMatchingBlocks,
                    count: rows,
                    limit: RESULT_BRIDGE_MAX_ROWS,
                    bytes: Some(bytes),
                    byte_limit: RESULT_BRIDGE_MAX_BYTES,
                });
            }
        }
        Ok(preview)
    }

    /// Block referrers, worst case O(B), with fixed row/byte limits.
    pub fn block_referrers(&self, uuid: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph.block_referrers_bounded(
                uuid,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            ),
            Budget::MatchingBlocks,
        )
    }

    /// Referenced block counts; O(B) on a cache miss.
    pub fn block_ref_counts(&self) -> Arc<HashMap<String, usize>> {
        self.graph.block_ref_counts()
    }

    /// `[[` completion over pages, journals, aliases and referenced names.
    /// Cost O(P + aliases + referenced names), at most `limit` entries.
    pub fn complete_page_names(&self, text: &str, limit: usize) -> Vec<PageEntry> {
        self.graph.quick_switch(text, limit)
    }

    /// Literal `((` block search, O(B), at most `limit` blocks. Checks `cancel`
    /// before each page and block; cancellation returns no partial result.
    pub fn find_blocks(
        &self,
        text: &str,
        limit: usize,
        cancel: &Cancel,
    ) -> Result<Vec<RefGroup>, QueryError> {
        let result = crate::query::search_cancellable_result(
            &self.graph,
            text,
            limit.min(RESULT_BRIDGE_MAX_ROWS),
            || cancel.0.load(Ordering::Acquire),
        );
        let groups = result.ok_or(QueryError::Cancelled)?;
        let rows = groups.iter().map(|g| g.blocks.len()).sum::<usize>();
        let bytes = tine_core::model::ref_groups_estimated_bytes(&groups);
        if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
            Err(QueryError::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count: rows,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            })
        } else {
            Ok(groups)
        }
    }

    /// Selected query subtrees, O(64 × B) plus selected nodes. Fixed request,
    /// root, node and byte limits; oversized requests fail before evaluation.
    pub fn export_query_subtrees(
        &self,
        specs: &[QueryExportSpec],
    ) -> Result<QueryExportBatch, QueryError> {
        let query_bytes = specs.iter().fold(0usize, |n, s| {
            n.saturating_add(s.key.len()).saturating_add(s.query.len())
        });
        if specs.len() > QUERY_EXPORT_REQUEST_MAX_QUERIES
            || query_bytes > QUERY_EXPORT_MAX_QUERY_BYTES
        {
            return Err(QueryError::ExportRequestTooLarge {
                macros: specs.len(),
                bytes: query_bytes,
                macro_limit: QUERY_EXPORT_REQUEST_MAX_QUERIES,
                byte_limit: QUERY_EXPORT_MAX_QUERY_BYTES,
                processing_cap: QUERY_EXPORT_MAX_QUERIES,
            });
        }
        let batch = crate::query::export_query_subtrees(
            &self.graph,
            specs,
            QUERY_EXPORT_MAX_QUERIES,
            QUERY_EXPORT_MAX_ROOTS,
            QUERY_EXPORT_MAX_NODES,
            QUERY_EXPORT_MAX_BYTES,
        );
        let bytes = batch
            .results
            .iter()
            .map(|r| {
                r.key.len()
                    + r.groups
                        .iter()
                        .map(|g| {
                            tine_core::model::ref_groups_estimated_bytes(std::slice::from_ref(g))
                        })
                        .sum::<usize>()
                    + 128
            })
            .sum::<usize>();
        if bytes > QUERY_EXPORT_MAX_BYTES {
            Err(QueryError::ResultTooLarge {
                what: Budget::ExportBytes,
                count: bytes,
                limit: QUERY_EXPORT_MAX_BYTES,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            })
        } else {
            Ok(batch)
        }
    }

    /// Query-builder facets reject overflow; editor autocomplete returns a
    /// bounded prefix. O(B), fixed item and byte limits.
    pub fn property_facets(
        &self,
        policy: FacetPolicy,
    ) -> Result<Vec<(String, Vec<String>)>, QueryError> {
        match policy {
            FacetPolicy::Budgeted => {
                let (facets, exceeded) = crate::query::property_facets_bounded(
                    &self.graph,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err(QueryError::ResultTooLarge {
                        what: Budget::PropertyFacets,
                        count: 0,
                        limit: RESULT_BRIDGE_MAX_ROWS,
                        bytes: None,
                        byte_limit: RESULT_BRIDGE_MAX_BYTES,
                    })
                } else {
                    Ok(facets)
                }
            }
            FacetPolicy::Truncated => Ok(crate::query::autocomplete_property_facets_bounded(
                &self.graph,
                AUTOCOMPLETE_FACET_MAX_ITEMS,
                AUTOCOMPLETE_FACET_MAX_BYTES,
            )
            .0),
        }
    }

    /// Template blocks, O(B) on a cache miss.
    pub fn templates(&self) -> Vec<TemplateDto> {
        self.graph.templates()
    }

    /// Icons for requested names, O(names + aliases).
    pub fn page_icons(&self, names: &[String]) -> HashMap<String, String> {
        self.graph.page_icons(names)
    }

    /// Journal days with content, O(journals + their blocks).
    pub fn journal_content_days(&self) -> Vec<Day> {
        self.graph
            .journal_content_days()
            .into_iter()
            .map(Day)
            .collect()
    }
}
