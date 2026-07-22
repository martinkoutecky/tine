//! Graph model: opening a graph directory, listing/loading/saving pages, and
//! the DTOs that cross the Tauri IPC boundary.
//!
//! For M0/M1 the canonical state is the on-disk files; Rust loads a page into a
//! [`PageDto`] tree and writes it back from one. The frontend owns the live
//! editing tree (see plan). File-backed runtime UUIDs are deterministic structural
//! locators; persisted `id::` values remain a separate external reference identity.

use crate::config::{Config, FileNameFormat};
use crate::crdt::{
    BlockId as CrdtBlockId, BlockSnapshot as CrdtBlockSnapshot, CrdtGraph, CrdtStatus,
    ManagedSyncStoreState, PageId as CrdtPageId, PageSnapshot as CrdtPageSnapshot,
    ProjectionPrecondition,
};
use crate::date::{JournalDate, JournalFormat};
use crate::doc::{self, DocBlock, Document};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    Journal,
    Page,
}

/// On-disk file format of a page. Markdown (`.md`) is the default; Logseq org
/// graphs use `.org`. A graph may mix the two — format is decided per file by
/// extension, never graph-wide (matching OG, which stores `:block/format` per
/// page). The graph's `:preferred-format` only chooses the extension for NEW
/// files (see [`Graph::preferred_format`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Md,
    Org,
}

impl Format {
    /// Format of a page file by its extension (`.org` → Org, else Md).
    pub fn from_path(p: &Path) -> Format {
        match p.extension().and_then(|e| e.to_str()) {
            Some("org") => Format::Org,
            _ => Format::Md,
        }
    }
    /// File extension (no dot) for this format.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Md => "md",
            Format::Org => "org",
        }
    }
}

/// Whether `path` is a page file Tine reads (markdown or org).
fn is_page_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("org")
    )
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn rel_under_dir(rel_dir: &str, dir: &Path, path: &Path) -> String {
    let tail = path.strip_prefix(dir).unwrap_or(path);
    if tail.as_os_str().is_empty() {
        rel_dir.to_string()
    } else {
        format!("{rel_dir}/{}", slash_path(tail))
    }
}

/// If `stem` is a sync tool's conflict copy of another file, return the base file
/// stem it shadows. Recognises Syncthing
/// (`name.sync-conflict-YYYYMMDD-HHMMSS-XXXXXXX`) and Dropbox
/// (`name (conflicted copy …)` / `name (<user>'s conflicted copy …)`).
///
/// A conflict copy is NOT a real page — it must be kept out of the page list and
/// the `(kind,name)` cache (otherwise it shows as a garbage page and its shared
/// `id::` values churn the id space), yet remain loadable by path for the
/// conflict-merge UI. So this is threaded through the *listing* sites, never
/// through `is_page_file`/`entry_for_path`/`resolve_rel` (which the merge UI's
/// path-addressed load relies on).
pub fn sync_conflict_base(stem: &str) -> Option<&str> {
    if let Some(i) = stem.find(".sync-conflict-") {
        return Some(&stem[..i]);
    }
    // Dropbox: "<base> (conflicted copy …)" or "<base> (<user>'s conflicted copy …)".
    if let Some(i) = stem.find(" (") {
        if stem[i..].contains("conflicted copy") {
            return Some(&stem[..i]);
        }
    }
    None
}

/// Whether `stem` names a sync-tool conflict copy (see [`sync_conflict_base`]).
pub fn is_sync_conflict(stem: &str) -> bool {
    sync_conflict_base(stem).is_some()
}

/// Whether `path`'s file stem names a sync-tool conflict copy — the `Path`-level
/// convenience used by the watcher (which works in paths, not stems).
pub fn path_is_sync_conflict(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(is_sync_conflict)
}

/// Error for an ambiguous page that exists as both a `.md` and a `.org` file.
/// Deliberately NOT the `AlreadyExists`/"conflict" signal, so the UI surfaces it
/// as a plain error (a toast) instead of a keep-mine/use-disk conflict prompt.
fn twin_error(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Other,
        format!(
            "\"{name}\" exists as both a .md and a .org file — remove one (e.g. in Logseq) to edit it in Tine"
        ),
    )
}

/// The error for a path-addressed op (#21) whose graph-root-relative path is
/// invalid — outside `journals/`/`pages/`, a traversal, or the wrong extension.
fn bad_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid file path")
}

/// Parse a page file's bytes into a [`Document`] using the parser for its
/// format (org headlines vs markdown bullets), chosen by the path's extension.
fn parse_doc(path: &Path, content: &str) -> Document {
    match Format::from_path(path) {
        Format::Md => doc::parse(content),
        Format::Org => crate::org::parse_org(content),
    }
}

/// Lightweight entry for the page list / sidebar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageEntry {
    pub name: String,
    pub kind: PageKind,
    /// Sort key `yyyymmdd` for journals; `None` for ordinary pages.
    pub date_key: Option<i64>,
    /// Graph-root-relative path exposed to the frontend so duplicate basenames
    /// can be opened by file, not by ambiguous `(kind,name)`.
    #[serde(rename = "path", default)]
    pub rel_path: String,
    #[serde(skip)]
    pub path: PathBuf,
}

/// A block as sent to / received from the frontend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockDto {
    pub id: String,
    pub raw: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub children: Vec<BlockDto>,
    /// Ancestor first-lines (page-relative path) for search/reference results;
    /// empty for normal page loads. Lets the UI show a "parent › child" trail.
    #[serde(default)]
    pub breadcrumb: Vec<String>,
    /// Synthetic, read-only result row representing references from the source
    /// page's property pre-block rather than an editable outline block.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_property: bool,
    // --- M1: block-header facets, computed ONCE off the lsdoc projection (the one
    // grammar source) and shipped so the frontend never re-derives them with its
    // own scanner. Derived (not authoritative — `raw` round-trips); the frontend
    // recomputes locally only for the block it is actively editing. Omitted from the
    // wire when empty to keep the payload small (most blocks have no marker/dates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<(String, String)>,
}

/// A group of blocks from one source page — used for both Linked References
/// (backlinks) and `{{query}}` results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefGroup {
    pub page: String,
    pub kind: PageKind,
    pub blocks: Vec<BlockDto>,
    /// Result-only source evidence keyed by block id. Empty for ordinary query
    /// groups and older callers; never crosses the block write boundary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<ReferenceBlockEvidence>,
}

/// One backlink root whose visible subtree can be searched and whose OG-style
/// co-reference facets came from the cached lsdoc projection. This is fetched
/// only when the Linked References filter opens; ordinary backlink DTOs remain
/// shallow so their lazy-loading and bridge cost do not change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkFilterTarget {
    pub page: String,
    pub kind: PageKind,
    pub block_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkFilterEntry {
    pub page: String,
    pub kind: PageKind,
    pub block_id: String,
    pub text: String,
    pub facets: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacklinkFilterContext {
    pub entries: Vec<BacklinkFilterEntry>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Cache-friendly bounded result metadata. The groups stay behind one `Arc` so
/// routine frontend refreshes can reuse the generation-scoped native result
/// without a deep clone while preserving the construction ceiling's outcome.
#[derive(Debug, Clone)]
pub struct BoundedRefGroups {
    pub groups: Arc<Vec<RefGroup>>,
    pub total: usize,
    pub exceeded: bool,
}

/// A deliberately bounded block-reference hover preview. Ordinary query,
/// reference, and batched-resolution results carry shallow block identities;
/// callers that genuinely need a subtree must ask for one explicitly and give
/// it node and byte budgets so an outline cannot be multiplied across the IPC
/// bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockPreview {
    pub group: RefGroup,
    /// Number of nodes omitted after either construction budget was reached.
    pub truncated: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Explicit,
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceSpan {
    /// UTF-16 code-unit offsets into the matching `BlockDto.raw`.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceOccurrence {
    pub matched_name: String,
    pub canonical: String,
    pub kind: ReferenceKind,
    pub span: ReferenceSpan,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceBlockEvidence {
    pub block_id: String,
    pub occurrences: Vec<ReferenceOccurrence>,
    /// Total parser-owned matches before the bounded evidence cap.
    #[serde(default)]
    pub total: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDiagnosticTrace {
    pub page: String,
    pub kind: PageKind,
    pub block_id: String,
    pub occurrences: Vec<ReferenceOccurrence>,
    pub included_linked: bool,
    pub included_unlinked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDiagnostics {
    pub engine_version: String,
    pub target: String,
    pub traces: Vec<ReferenceDiagnosticTrace>,
}

/// A named template (a block with `template:: <name>`) and the blocks to insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDto {
    pub name: String,
    pub blocks: Vec<BlockDto>,
    /// Page the template's defining block lives on (so the UI can jump to edit it).
    pub page: String,
    /// Kind of that page (journal/page), for navigation.
    pub kind: PageKind,
}

/// An orphaned asset file (no block references it) — surfaced so the user can
/// review + trash unused media. `size` in bytes; `modified` is the file's
/// last-modified time as Unix seconds (≈ when it entered the graph), or `None`
/// if the filesystem doesn't report it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub name: String,
    pub size: u64,
    pub modified: Option<u64>,
}

/// Count + total bytes of recoverable asset trash. `count`/`bytes` are asset
/// entries only; the other counters are protected non-asset recovery files that
/// share `logseq/.tine-trash` for backward compatibility.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TrashStats {
    pub count: u64,
    pub bytes: u64,
    pub pages: u64,
    pub journals: u64,
    pub conflicts: u64,
    pub other: u64,
}

/// One file participating in a journal-day conflict: its on-disk filename, a
/// graph-root-relative path (so the UI can navigate straight to THIS file even
/// when it shares a date with the canonical one, #21), a one-line content
/// preview, and whether its name is the canonical date stem (`yyyy_MM_dd`, the
/// one normally kept).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalFile {
    pub name: String,
    pub path: String,
    pub preview: String,
    pub canonical: bool,
}

/// A journal day that resolves to more than one file (e.g. a canonical
/// `2026_06_26.org` plus a title-named `Friday, 26-06-2026.org`, or a `.md`+`.org`
/// twin). These can't be auto-merged, so they're surfaced for the user to
/// reconcile (delete the redundant one / copy content across).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalConflict {
    pub title: String,
    pub files: Vec<JournalFile>,
}

/// A sync-tool conflict copy left in the graph (Syncthing/Dropbox) — a
/// `*.sync-conflict-*.md` (or Dropbox `(conflicted copy)`) file that shadows a
/// real page. Surfaced so the user can review + reconcile it instead of it
/// rotting as a garbage page. See [`sync_conflict_base`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    /// Graph-root-relative path of the conflict copy file.
    pub path: String,
    /// Display name of the page it shadows (decoded page name / journal title).
    pub base_name: String,
    /// Graph-root-relative path of the winning (base) file, if it still exists.
    pub base_path: Option<String>,
    /// Kind of the shadowed page (journal/page).
    pub kind: PageKind,
    /// The device/timestamp suffix from the conflict filename (best-effort label).
    pub tag: String,
    /// One-line content preview of the conflict copy.
    pub preview: String,
}

/// A full page as sent to / received from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDto {
    pub name: String,
    pub kind: PageKind,
    pub title: String,
    /// Raw page-property pre-block (if any).
    pub pre_block: Option<String>,
    pub blocks: Vec<BlockDto>,
    /// Hash of the on-disk file content when this page was loaded — the editor's
    /// baseline. Sent back on save so we conflict against the version the editor
    /// actually loaded (not the mutable cache, which the watcher can advance).
    /// `None` for a page with no file yet.
    #[serde(default)]
    pub rev: Option<String>,
    /// On-disk format of this page (markdown vs org), so the editor renders org
    /// inline syntax and shows the right bullet. New pages default to markdown.
    #[serde(default)]
    pub format: Format,
    /// True for an org page Tine can't round-trip byte-for-byte: the editor shows
    /// it but disables editing, so Tine never rewrites (and risks corrupting) it.
    #[serde(default)]
    pub read_only: bool,
    /// Graph-root-relative path of the file this page was loaded from
    /// (`journals/2026_06_26.org`), forward-slashed. Echoed back on save so a page
    /// pinned to a SPECIFIC file — a duplicate-day stray that shares a `(kind,name)`
    /// with the canonical file — saves to its own file instead of being re-resolved
    /// by name to the canonical one (#21). Empty for a brand-new page with no file
    /// yet; then save resolves the path by name, exactly as before.
    #[serde(default)]
    pub path: String,
    /// True for bundled in-app Guide pages. Guide pages are ephemeral/read-only
    /// virtual pages and must never be persisted into the user's graph by the
    /// normal save/writeback path.
    #[serde(default)]
    pub guide: bool,
}

/// What enabling managed sync would change in the plain-text projection.
///
/// The migration is deliberately inspectable before it writes: every block needs
/// a durable Logseq-compatible id so a later edit made by OG Logseq or a plain
/// editor can be reconciled against the CRDT without guessing its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncIdentityPlan {
    pub pages: usize,
    pub blocks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncIdentityMigration {
    pub pages_changed: usize,
    pub blocks_changed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedSyncEnableResult {
    pub migration: SyncIdentityMigration,
    pub status: CrdtStatus,
}

#[derive(Debug, Clone)]
pub struct ManagedSyncProjectionChange {
    pub entry: PageEntry,
    pub created: bool,
    pub removed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ManagedSyncPull {
    pub imported_chunks: usize,
    pub changes: Vec<ManagedSyncProjectionChange>,
    pub conflicts_changed: bool,
}

pub struct Graph {
    pub root: PathBuf,
    /// The canonical filesystem capability used for every asset operation. For
    /// ordinary graphs this is `<root>/assets`; when the runtime has explicitly
    /// approved an external assets symlink/junction it is that exact resolved
    /// directory. No other managed graph path may use this capability.
    assets_root: PathBuf,
    pub config: Config,
    /// Journal date formats (filename + title) resolved from `config.edn`, used to
    /// recognize journal files in the user's format and render new ones. Built once
    /// at open (config changes need a reopen, as in OG).
    pub journal_format: JournalFormat,
    /// In-memory cache of every parsed page, keyed implicitly by position.
    /// Built once on first whole-graph query and kept in sync by edits, so
    /// search / backlinks / `{{query}}` scan memory instead of re-reading and
    /// re-parsing the entire tree on every keystroke. `None` = not yet built.
    // `Arc<Document>` so a cache snapshot or a save's scoped-invalidation copy is
    // an O(1) refcount bump, not a deep clone of the whole page (see cache_upsert).
    cache: RwLock<Option<Arc<Vec<(PageEntry, Arc<Document>)>>>>,
    /// Graph-relative paths of pages skipped by the latest whole-graph cache
    /// build because their parse/projection panicked. Kept retrievable so an
    /// lsdoc ownership gap can never degrade search completeness invisibly.
    page_index_failures: RwLock<Vec<String>>,
    /// Companion indexes for `cache`: the logical `(kind, page_key(name)) -> Vec
    /// slot` index preserves deterministic first-wins lookup, while the exact-path
    /// index keeps cache ownership physical. The Vec stays the source of truth for
    /// whole-graph iteration. `None` means "rebuild from the Vec on next lookup"
    /// and is preferred over risking a stale slot after broad mutations.
    cache_index: RwLock<Option<PageCacheIndex>>,
    /// Bumped on every cache mutation (upsert/remove). The lock-free cache build
    /// captures this before reading disk and rebuilds if a mutation raced it
    /// (which would otherwise install stale content over a concurrent save).
    cache_gen: std::sync::atomic::AtomicU64,
    /// Serializes whole-graph cache builds so a racing warmup/search/query parses
    /// the graph ONCE, not once per caller. Held only during the build (not the
    /// cache lock), so it never blocks readers of an already-built cache.
    build_lock: std::sync::Mutex<()>,
    /// Cached `alias:: → canonical + owning path` records, derived from the page cache. Rebuilt
    /// lazily and dropped whenever the page cache mutates (the only time aliases
    /// can change). Avoids re-scanning the whole graph for aliases on every page
    /// load / backlink lookup.
    alias_cache: RwLock<Option<Vec<(String, String, String)>>>,
    /// `block uuid / id:: → page name` hint, derived from the page cache and keyed
    /// by `cache_gen` so it self-invalidates on any cache mutation (same pattern as
    /// `alias_cache`). Lets `((uuid))` ref / embed resolution jump straight to the
    /// owning page instead of walking every block of every page. A stale hint is
    /// harmless: resolution falls back to a full scan when the block isn't found.
    block_index: RwLock<Option<(u64, std::collections::HashMap<String, String>)>>,
    /// Reconstructible, generation-keyed candidate index for page references.
    /// Explicit postings are parser-owned; the fixed-size per-page signature is
    /// only a no-false-negative prefilter for safely tokenizable plain mentions.
    /// Exact reference verification remains authoritative in `query`.
    reference_candidate_index: RwLock<Option<ReferenceCandidateIndex>>,
    /// `block uuid → # of distinct blocks that reference it` (`((uuid))`, labeled
    /// `[..](((uuid)))`, `{{embed ((uuid))}}`), keyed by `cache_gen` so it self-
    /// invalidates on any cache mutation (same pattern as `block_index`). Drives the
    /// per-block reference-count badge; `Arc` so handing the whole map to the
    /// frontend is a refcount bump, not a clone. Only referenced uuids appear, so
    /// the map is small.
    block_ref_count_cache: RwLock<Option<(u64, Arc<std::collections::HashMap<String, usize>>)>>,
    /// Memoized results of the pervasive whole-graph scans (run_query / backlinks /
    /// unlinked_refs), keyed by `(cache_gen, today)` so it self-invalidates on ANY
    /// cache mutation and on a date rollover (relative-date queries depend on
    /// today). Lets a re-render, a second component showing the same query, or
    /// navigating back to a page recompute nothing; never serves a stale result.
    derived_cache: RwLock<Option<DerivedCache>>,
    /// Memoized advanced-query results. Kept separate from `derived_cache` because
    /// advanced queries return clause metadata as well as groups.
    advanced_cache: RwLock<Option<AdvancedCache>>,
    /// Memoized `list_pages()` (the journals//pages/ directory scan), keyed by
    /// cache_gen — which bumps on every page create/delete/rename (Tine or watcher)
    /// — so quick-switch / [[ ]] autocomplete don't re-read both dirs on every
    /// keystroke. An externally-created page not yet seen by the watcher is at most
    /// one watcher tick (≤3s) stale here.
    page_list_cache: RwLock<Option<(u64, Vec<PageEntry>)>>,
    /// Memoized exact `find_entry(name, kind)` resolution, keyed by `cache_gen`.
    /// Unlike `list_pages()`, this index is built from raw `list_md` output so it
    /// preserves `find_entry`'s duplicate selection: date-stem file first, else
    /// first directory-walk match.
    find_entry_cache: RwLock<Option<(u64, FindEntryIndex)>>,
    /// `path → content_rev` of the bytes Tine last wrote to each page file,
    /// recorded *before* the write lands on disk. The file watcher reads files
    /// outside the cache lock, so during the window between a save's atomic rename
    /// and its `cache_upsert` it can read disk-ahead-of-cache and mistake Tine's
    /// own write for an external change. This lets the watcher recognize the exact
    /// bytes we wrote and suppress that false positive (the parse-cache comparison
    /// alone races that window). See `write_page` / `sync_file_content`.
    recent_writes: std::sync::Mutex<std::collections::HashMap<PathBuf, String>>,
    /// `path → content_rev` of the on-disk bytes the cached page's
    /// `Document` was parsed from. Invariant: an entry exists IFF the page is in
    /// the cache, and `disk_revs[path] == content_rev(current disk bytes)` ⟹ the
    /// cached doc reflects disk (is fresh). Lets `sync_file_content` skip the
    /// parse→serialize→parse freshness comparison when a file is unchanged — the
    /// common case on every page navigation and most watcher polls. A missing or
    /// mismatched entry always falls through to the correct parse-compare path, so
    /// the worst a desync can cause is redundant work, never a stale serve.
    disk_revs: RwLock<std::collections::HashMap<PathBuf, String>>,
    /// All page names referenced anywhere — `[[link]]`/`#tag`/`#[[..]]` plus
    /// `tags::`/`alias::` property values — in their as-written display case,
    /// keyed by `cache_gen`. Like OG, a page that is only referenced (never given
    /// its own file) still "exists" — this lets quick-switch / `[[ ]]`/`#`
    /// autocomplete surface such a page instead of offering a misleading
    /// "Create …". Built from the page cache, and only when it's already warm
    /// (never force-built on a keystroke); empty until then.
    referenced_names_cache: RwLock<Option<(u64, Vec<String>)>>,
    /// Per-resolved-path write locks. The same page file has TWO in-process
    /// writers — the editor (`save_page`/`write_page`) and the PDF highlight path
    /// (`write_highlights`, for an `hls__` page) — and a rename rewrites many
    /// files at once. Holding the per-path lock across the whole
    /// read→conflict-check→write→`cache_upsert` makes same-page writes serialize,
    /// so they can't clobber each other or leave a stale self-write marker.
    /// Lock order is ALWAYS page_lock → cache → disk_revs; never the reverse.
    page_locks:
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
    /// Present only for an opt-in `.tine-sync/v1` workspace. The mutex serializes
    /// Loro transactions and immutable-chunk replay; callers must never hold it
    /// while acquiring a page lock (save uses page-lock → sync-lock, while remote
    /// replay releases sync-lock before projecting pages).
    managed_sync: std::sync::Mutex<Option<CrdtGraph>>,
    /// Per-UI-lane cancellation epochs for whole-graph text searches. Starting a
    /// newer search makes its superseded prefix stop promptly.
    search_lanes: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicU64>>,
    >,
}

struct PageCacheIndex {
    by_name: std::collections::HashMap<(PageKind, String), usize>,
    by_path: std::collections::HashMap<PathBuf, usize>,
}

const REFERENCE_SIGNATURE_WORDS: usize = 64; // 4096 bits = 512 bytes/page

#[derive(Clone)]
struct ReferenceTokenSignature([u64; REFERENCE_SIGNATURE_WORDS]);

impl Default for ReferenceTokenSignature {
    fn default() -> Self {
        Self([0; REFERENCE_SIGNATURE_WORDS])
    }
}

impl ReferenceTokenSignature {
    fn token_hash(token: &[u8], seed: u64) -> usize {
        let mut hash = seed;
        for byte in token {
            hash ^= u64::from(byte.to_ascii_lowercase());
            hash = hash.wrapping_mul(0x100000001b3);
        }
        (hash as usize) & (REFERENCE_SIGNATURE_WORDS * 64 - 1)
    }

    fn insert_token(&mut self, token: &[u8]) {
        for seed in [0xcbf29ce484222325, 0x9e3779b97f4a7c15] {
            let bit = Self::token_hash(token, seed);
            self.0[bit / 64] |= 1u64 << (bit % 64);
        }
    }

    fn insert_text(&mut self, text: &str) {
        // Exact plain-reference matching compares Unicode-lowercased, NFC text.
        // Fold the source the same way before extracting ASCII tokens so a
        // character such as the Kelvin sign (`K`) cannot match page `K` exactly
        // while being absent from this no-false-negative prefilter.
        let folded: String = text.to_lowercase().nfc().collect();
        let bytes = folded.as_bytes();
        let mut start = None;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte.is_ascii_alphanumeric() {
                if start.is_none() {
                    start = Some(index);
                }
            } else if let Some(begin) = start.take() {
                self.insert_token(&bytes[begin..index]);
            }
        }
        if let Some(begin) = start {
            self.insert_token(&bytes[begin..]);
        }
    }

    /// `None` means tokenization is not provably safe, so callers must full-scan.
    fn may_contain_name(&self, normalized_name: &str) -> Option<bool> {
        if !normalized_name.is_ascii() {
            return None;
        }
        let tokens = normalized_name
            .as_bytes()
            .split(|byte| !byte.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            return None;
        }
        Some(tokens.into_iter().all(|token| {
            [0xcbf29ce484222325, 0x9e3779b97f4a7c15]
                .into_iter()
                .all(|seed| {
                    let bit = Self::token_hash(token, seed);
                    self.0[bit / 64] & (1u64 << (bit % 64)) != 0
                })
        }))
    }
}

#[derive(Clone)]
struct ReferencePageProjection {
    explicit: Vec<String>,
    signature: ReferenceTokenSignature,
    name_key: String,
    name: String,
}

struct ReferenceCandidateIndex {
    generation: u64,
    complete: bool,
    pages: std::collections::HashMap<PathBuf, ReferencePageProjection>,
    explicit: std::collections::HashMap<String, std::collections::BTreeSet<PathBuf>>,
    real_pages: std::collections::HashMap<String, std::collections::BTreeMap<PathBuf, String>>,
}

impl ReferenceCandidateIndex {
    fn page_projection(entry: &PageEntry, doc: &Document) -> ReferencePageProjection {
        fn add_blocks(signature: &mut ReferenceTokenSignature, blocks: &[DocBlock]) {
            for block in blocks {
                signature.insert_text(&block.raw);
                add_blocks(signature, &block.children);
            }
        }
        let mut signature = ReferenceTokenSignature::default();
        if let Some(pre) = doc.pre_block.as_deref() {
            signature.insert_text(pre);
        }
        add_blocks(&mut signature, &doc.roots);
        ReferencePageProjection {
            explicit: crate::query::document_explicit_reference_names(entry, doc),
            signature,
            name_key: crate::refs::page_key(&entry.name),
            name: entry.name.clone(),
        }
    }

    fn build(generation: u64, pages: &[(PageEntry, Arc<Document>)]) -> Self {
        let mut index = Self {
            generation,
            complete: true,
            pages: std::collections::HashMap::with_capacity(pages.len()),
            explicit: std::collections::HashMap::new(),
            real_pages: std::collections::HashMap::new(),
        };
        for (entry, doc) in pages {
            index.insert(entry, doc);
        }
        index
    }

    fn remove(&mut self, path: &Path) {
        let Some(previous) = self.pages.remove(path) else {
            return;
        };
        for target in previous.explicit {
            let remove_posting = self.explicit.get_mut(&target).is_some_and(|paths| {
                paths.remove(path);
                paths.is_empty()
            });
            if remove_posting {
                self.explicit.remove(&target);
            }
        }
        let remove_name = self
            .real_pages
            .get_mut(&previous.name_key)
            .is_some_and(|owners| {
                owners.remove(path);
                owners.is_empty()
            });
        if remove_name {
            self.real_pages.remove(&previous.name_key);
        }
    }

    fn insert(&mut self, entry: &PageEntry, doc: &Document) {
        self.remove(&entry.path);
        let projection = Self::page_projection(entry, doc);
        for target in &projection.explicit {
            self.explicit
                .entry(target.clone())
                .or_default()
                .insert(entry.path.clone());
        }
        self.real_pages
            .entry(projection.name_key.clone())
            .or_default()
            .insert(entry.path.clone(), projection.name.clone());
        self.pages.insert(entry.path.clone(), projection);
    }

    #[cfg(test)]
    fn estimated_bytes(&self) -> usize {
        let page_bytes = self
            .pages
            .iter()
            .map(|(path, page)| {
                path.as_os_str().len()
                    + std::mem::size_of::<ReferencePageProjection>()
                    + page.explicit.iter().map(String::len).sum::<usize>()
                    + page.name_key.len()
                    + page.name.len()
            })
            .sum::<usize>();
        let posting_bytes = self
            .explicit
            .iter()
            .map(|(target, paths)| {
                target.len()
                    + paths
                        .iter()
                        .map(|path| path.as_os_str().len())
                        .sum::<usize>()
            })
            .sum::<usize>();
        let real_page_bytes = self
            .real_pages
            .iter()
            .map(|(key, owners)| {
                key.len()
                    + owners
                        .iter()
                        .map(|(path, name)| path.as_os_str().len() + name.len())
                        .sum::<usize>()
            })
            .sum::<usize>();
        page_bytes + posting_bytes + real_page_bytes
    }
}

pub(crate) struct ReferenceCandidatePages {
    pub pages: Vec<(PageEntry, Arc<Document>)>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub indexed: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub full_page_count: usize,
}

#[derive(Default)]
struct PageCacheBuild {
    pages: Vec<ParsedPage>,
    failures: Vec<String>,
}

type ParsedPage = (PageEntry, Document, String);
type PageParseResult = Result<Option<ParsedPage>, String>;

impl PageCacheBuild {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            pages: Vec::with_capacity(capacity),
            failures: Vec::new(),
        }
    }

    fn append(&mut self, mut other: Self) {
        self.pages.append(&mut other.pages);
        self.failures.append(&mut other.failures);
    }

    fn collect(&mut self, parsed: PageParseResult) -> bool {
        match parsed {
            Ok(Some(page)) => {
                self.pages.push(page);
                true
            }
            Ok(None) => false,
            Err(path) => {
                self.failures.push(path);
                false
            }
        }
    }
}

fn page_cache_key(kind: PageKind, name: &str) -> (PageKind, String) {
    (kind, crate::refs::page_key(name))
}

fn document_block_ref_counts(doc: &Document) -> std::collections::HashMap<String, usize> {
    fn walk(blocks: &[DocBlock], counts: &mut std::collections::HashMap<String, usize>) {
        for block in blocks {
            // projection().block_refs is already de-duplicated per referrer block,
            // matching the badge's OG-compatible counting semantics.
            for id in &block.projection().block_refs {
                *counts.entry(id.clone()).or_insert(0) += 1;
            }
            walk(&block.children, counts);
        }
    }

    let mut counts = std::collections::HashMap::new();
    walk(&doc.roots, &mut counts);
    counts
}

fn build_page_cache_index(pages: &[(PageEntry, Arc<Document>)]) -> PageCacheIndex {
    let mut by_name = std::collections::HashMap::with_capacity(pages.len());
    let mut by_path = std::collections::HashMap::with_capacity(pages.len());
    for (i, (entry, _)) in pages.iter().enumerate() {
        // Preserve Vec `.find` semantics if duplicates ever slip in: first wins.
        by_name
            .entry(page_cache_key(entry.kind, &entry.name))
            .or_insert(i);
        by_path.insert(entry.path.clone(), i);
    }
    PageCacheIndex { by_name, by_path }
}

fn is_date_stem_entry(entry: &PageEntry) -> bool {
    entry
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| crate::date::JournalDate::from_file_stem(s).is_some())
}

/// Gen+today-tagged cache of derived scan results. Reset wholesale whenever the
/// tag no longer matches — so every entry is always consistent with the current
/// graph state (no per-entry invalidation to get wrong).
struct DerivedCache {
    gen: u64,
    today: i64,
    // `Arc<Vec<RefGroup>>` so serving a memoized result (every dataRev re-render)
    // is a refcount bump, not a deep clone of every matched block (see derived_memo).
    results: std::collections::HashMap<String, (BoundedRefGroups, usize)>,
    lru: std::collections::VecDeque<String>,
    bytes: usize,
}

struct AdvancedCache {
    gen: u64,
    today: i64,
    results: std::collections::HashMap<String, (CachedAdvancedResult, usize)>,
    lru: std::collections::VecDeque<String>,
    bytes: usize,
}

#[derive(Clone)]
struct CachedAdvancedResult {
    result: Arc<crate::query::AdvancedResult>,
    total: usize,
    exceeded: bool,
}

// Query results contain owned DTO subtrees and can be close to graph-sized. A
// graph-lifetime, key-unbounded memo turns ordinary navigation through many
// pages' Linked References into unbounded retained memory. Oversized results are
// returned to their caller but deliberately not retained here.
const DERIVED_CACHE_MAX_ENTRIES: usize = 64;
const DERIVED_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
const DERIVED_CACHE_MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

fn result_cache_key_estimated_bytes(key: &str) -> usize {
    // The HashMap owns one key and the LRU owns another. Account both copies so
    // a result with an enormous query source cannot bypass the payload budget.
    key.len().saturating_mul(2).saturating_add(128)
}

pub fn block_dto_estimated_bytes(block: &BlockDto) -> usize {
    block.id.len()
        + block.raw.len()
        + block.breadcrumb.iter().map(String::len).sum::<usize>()
        + block.tags.iter().map(String::len).sum::<usize>()
        + block
            .properties
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>()
        + block
            .children
            .iter()
            .map(block_dto_estimated_bytes)
            .sum::<usize>()
        + 128
}

/// Conservative owned-memory estimate for a result payload. Tauri commands use
/// this before serialization as a second guard beside the row cap; derived
/// caches use the same accounting so transport and retention budgets cannot
/// drift apart.
pub fn ref_groups_estimated_bytes(groups: &[RefGroup]) -> usize {
    groups
        .iter()
        .map(|group| {
            group.page.len()
                + group
                    .blocks
                    .iter()
                    .map(block_dto_estimated_bytes)
                    .sum::<usize>()
                + group
                    .evidence
                    .iter()
                    .map(|evidence| {
                        evidence.block_id.len()
                            + evidence
                                .occurrences
                                .iter()
                                .map(|occurrence| {
                                    occurrence.matched_name.len()
                                        + occurrence.canonical.len()
                                        + occurrence.rule.len()
                                        + std::mem::size_of::<ReferenceOccurrence>()
                                })
                                .sum::<usize>()
                    })
                    .sum::<usize>()
                + std::mem::size_of::<RefGroup>()
        })
        .sum()
}

fn touch_lru(lru: &mut std::collections::VecDeque<String>, key: &str) {
    if let Some(pos) = lru.iter().position(|candidate| candidate == key) {
        lru.remove(pos);
    }
    lru.push_back(key.to_owned());
}

fn prune_result_cache<T>(
    results: &mut std::collections::HashMap<String, (T, usize)>,
    lru: &mut std::collections::VecDeque<String>,
    bytes: &mut usize,
) {
    while results.len() > DERIVED_CACHE_MAX_ENTRIES || *bytes > DERIVED_CACHE_MAX_BYTES {
        let Some(oldest) = lru.pop_front() else { break };
        if let Some((_, removed_bytes)) = results.remove(&oldest) {
            *bytes = bytes.saturating_sub(removed_bytes);
        }
    }
}

struct FindEntryIndex {
    entries: std::collections::HashMap<(PageKind, String), PageEntry>,
    pages_loaded: bool,
    journals_loaded: bool,
}

impl FindEntryIndex {
    fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            pages_loaded: false,
            journals_loaded: false,
        }
    }

    fn has_kind(&self, kind: PageKind) -> bool {
        match kind {
            PageKind::Journal => self.journals_loaded,
            PageKind::Page => self.pages_loaded,
        }
    }

    fn mark_kind_loaded(&mut self, kind: PageKind) {
        match kind {
            PageKind::Journal => self.journals_loaded = true,
            PageKind::Page => self.pages_loaded = true,
        }
    }
}

/// Validate one config-controlled graph directory. Logseq permits nested relative
/// directories, but an absolute path, traversal component, or symlinked existing
/// ancestor outside the graph would turn ordinary save/delete/restore operations
/// into writes against unrelated files.
fn validate_managed_dir(root: &Path, raw: &str, label: &str) -> io::Result<()> {
    if raw.is_empty() || raw.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {label} directory: {raw:?}"),
        ));
    }
    let rel = Path::new(raw);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} directory must be a safe relative path: {raw:?}"),
        ));
    }
    let candidate = root.join(rel);
    if !path_stays_within_root(root, &candidate) || path_uses_managed_alias(root, &candidate) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} directory escapes graph root: {raw:?}"),
        ));
    }
    Ok(())
}

/// Containment check for both existing and not-yet-created targets. Canonicalize
/// the deepest existing ancestor so a symlink in the path cannot smuggle a later
/// filename outside the graph. The runtime root is already canonical, while the
/// fallback keeps disposable direct-`Graph::open` fixtures working as before.
fn path_stays_within_root(root: &Path, target: &Path) -> bool {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut existing = target;
    while fs::symlink_metadata(existing).is_err() {
        let Some(parent) = existing.parent() else {
            return false;
        };
        existing = parent;
    }
    fs::canonicalize(existing)
        .map(|p| p.starts_with(&canonical_root))
        .unwrap_or(false)
}

/// Managed graph directories must retain their own identity, not merely land
/// somewhere under the graph after canonicalization. An in-graph symlink such as
/// `publish -> assets` passes a plain containment check but redirects generated
/// output onto user assets. Compare the deepest existing ancestor with its
/// expected canonical lexical location to reject any such alias.
fn path_uses_managed_alias(root: &Path, target: &Path) -> bool {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut existing = target;
    while fs::symlink_metadata(existing).is_err() {
        let Some(parent) = existing.parent() else {
            return true;
        };
        existing = parent;
    }
    let Ok(relative) = existing.strip_prefix(root) else {
        return true;
    };
    fs::canonicalize(existing)
        .map(|actual| actual != canonical_root.join(relative))
        .unwrap_or(true)
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_RENAME_SOURCE_REMOVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WITHDRAW_RACE_REPLACEMENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static GUIDE_TWIN_RACE_CONTENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn rename_source_remove_failpoint() -> io::Result<()> {
    FAIL_NEXT_RENAME_SOURCE_REMOVE.with(|flag| {
        if flag.replace(false) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected source remove failure",
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
fn withdrawal_race_hook(path: &Path) -> io::Result<()> {
    WITHDRAW_RACE_REPLACEMENT.with(|replacement| {
        if let Some(bytes) = replacement.borrow_mut().take() {
            fs::write(path, bytes)?;
        }
        Ok(())
    })
}

#[cfg(not(test))]
fn withdrawal_race_hook(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn guide_twin_race_hook(path: &Path) -> io::Result<()> {
    GUIDE_TWIN_RACE_CONTENT.with(|content| {
        if let Some(bytes) = content.borrow_mut().take() {
            fs::write(path.with_extension("org"), bytes)?;
        }
        Ok(())
    })
}

#[cfg(not(test))]
fn guide_twin_race_hook(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn rename_source_remove_failpoint() -> io::Result<()> {
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    pub root: String,
    pub journals_dir: String,
    pub pages_dir: String,
    /// "now" (LATER/NOW) or "todo" (TODO/DOING) — drives the task cycle.
    pub preferred_workflow: String,
    pub shortcuts: std::collections::HashMap<String, String>,
    /// First day of week for the date picker (0=Sunday … 6=Saturday).
    pub start_of_week: u32,
    /// Extra property keys to hide from the rendered properties area.
    pub block_hidden_properties: Vec<String>,
    /// Template name applied to a new, empty journal page (if configured).
    pub default_journal_template: Option<String>,
    /// Favorited page names (read from config.edn `:favorites`).
    pub favorites: Vec<String>,
    /// Effective journal title format (`:journal/page-title-format`, default
    /// `MMM do, yyyy`) — so the frontend formats "today" to match the backend.
    pub journal_page_title_format: String,
    /// Effective journal filename format (`:journal/file-name-format`, default
    /// `yyyy_MM_dd`).
    pub journal_file_name_format: String,
    /// Format new pages/journals are created in (`"md"` or `"org"`), from
    /// `:preferred-format`. The frontend uses it to label the toggle and pick the
    /// new-page extension.
    pub preferred_format: String,
    /// User-defined `:macros {"name" "template"}` — the frontend substitutes
    /// `$1..$N` args into the template and renders the result as markdown.
    pub macros: std::collections::HashMap<String, String>,
    /// `:feature/enable-timetracking?` effective value; default true.
    pub enable_timetracking: bool,
    /// `:ui/show-brackets?` effective value; default true.
    pub show_brackets: bool,
    /// `:shortcut/doc-mode-enter-for-new-block?` effective value; default false.
    pub doc_mode_enter_for_new_block: bool,
    /// `:editor/logical-outdenting?` effective value; default false.
    pub logical_outdenting: bool,
    /// `:logbook/settings :with-second-support?` effective value; default true.
    pub logbook_with_second_support: bool,
    /// `:logbook/settings :enabled-in-timestamped-blocks` effective value.
    pub logbook_enabled_in_timestamped_blocks: bool,
    /// `:logbook/settings :enabled-in-all-blocks` effective value.
    pub logbook_enabled_in_all_blocks: bool,
    /// Tine-owned graph-local flag: whether this graph has already seen the
    /// one-time in-app Guide announcement.
    pub guide_announced: bool,
}

impl Graph {
    /// Open a graph for use by the application, rejecting any configured page or
    /// journal directory that can escape the selected graph. `Graph::open` stays
    /// available for the many in-crate disposable fixtures, but runtime graph
    /// binding must use this checked entry point.
    pub fn open_checked(root: impl AsRef<Path>) -> io::Result<Graph> {
        Self::open_checked_with_assets(root, None)
    }

    /// Resolve an `assets` link/junction that lands outside the graph. The
    /// returned path is canonical and therefore suitable for showing to the user
    /// and binding a device-local approval. An in-graph directory (or a missing
    /// directory that Tine may create normally) returns `None`.
    pub fn external_assets_target(root: impl AsRef<Path>) -> io::Result<Option<PathBuf>> {
        let root = fs::canonicalize(root.as_ref())?;
        let assets = root.join("assets");
        match fs::symlink_metadata(&assets) {
            Ok(_) => {
                let resolved = fs::canonicalize(&assets)?;
                if !resolved.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("assets path is not a directory: {}", assets.display()),
                    ));
                }
                Ok((!resolved.starts_with(&root)).then_some(resolved))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Checked runtime open with one narrowly-scoped exception to the graph-root
    /// boundary: an external `assets` link/junction is accepted only when its
    /// current canonical target exactly matches the caller's approved target.
    /// This makes a retargeted link fail closed instead of inheriting old trust.
    pub fn open_checked_with_assets(
        root: impl AsRef<Path>,
        approved_assets: Option<&Path>,
    ) -> io::Result<Graph> {
        let mut graph = Self::open(root);
        validate_managed_dir(&graph.root, &graph.config.journals_dir, "journals")?;
        validate_managed_dir(&graph.root, &graph.config.pages_dir, "pages")?;
        validate_managed_dir(&graph.root, "logseq", "logseq")?;
        validate_managed_dir(&graph.root, "publish", "publish")?;
        validate_managed_dir(&graph.root, ".tine-sync", "managed sync")?;
        validate_managed_dir(&graph.root, ".tine-sync/v1", "managed sync store")?;
        if let Some(resolved) = Self::external_assets_target(&graph.root)? {
            let approved = approved_assets.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "external assets directory requires approval: {}",
                        resolved.display()
                    ),
                )
            })?;
            let approved = fs::canonicalize(approved).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("approved assets directory is unavailable: {error}"),
                )
            })?;
            if approved != resolved {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "external assets directory changed; approved {} but graph now resolves to {}",
                        approved.display(),
                        resolved.display()
                    ),
                ));
            }
            graph.assets_root = resolved;
        } else {
            validate_managed_dir(&graph.root, "assets", "assets")?;
            graph.assets_root = graph.root.join("assets");
        }
        Ok(graph)
    }

    pub(crate) fn ensure_write_target(&self, target: &Path) -> io::Result<()> {
        if path_stays_within_root(&self.root, target)
            && !path_uses_managed_alias(&self.root, target)
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("write target escapes graph root: {}", target.display()),
            ))
        }
    }

    /// Asset writes have their own capability boundary. Keeping this separate
    /// from `ensure_write_target` means approving external assets cannot widen a
    /// page/config/publish write into the same directory.
    fn ensure_asset_write_target(&self, target: &Path) -> io::Result<()> {
        if self.assets_root == self.root.join("assets") {
            return self.ensure_write_target(target);
        }
        if path_stays_within_root(&self.assets_root, target)
            && !path_uses_managed_alias(&self.assets_root, target)
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "write target escapes approved assets root: {}",
                    target.display()
                ),
            ))
        }
    }

    /// Open a graph directory, reading `logseq/config.edn` if present.
    pub fn open(root: impl AsRef<Path>) -> Graph {
        let root = root.as_ref().to_path_buf();
        let config = fs::read_to_string(root.join("logseq").join("config.edn"))
            .map(|s| Config::parse(&s))
            .unwrap_or_default();
        let journal_format = JournalFormat::new(
            config.journal_file_name_format.as_deref(),
            config.journal_page_title_format.as_deref(),
        );
        Graph {
            assets_root: root.join("assets"),
            root,
            config,
            journal_format,
            cache: RwLock::new(None),
            page_index_failures: RwLock::new(Vec::new()),
            cache_index: RwLock::new(None),
            cache_gen: std::sync::atomic::AtomicU64::new(0),
            build_lock: std::sync::Mutex::new(()),
            alias_cache: RwLock::new(None),
            block_index: RwLock::new(None),
            reference_candidate_index: RwLock::new(None),
            block_ref_count_cache: RwLock::new(None),
            derived_cache: RwLock::new(None),
            advanced_cache: RwLock::new(None),
            page_list_cache: RwLock::new(None),
            find_entry_cache: RwLock::new(None),
            recent_writes: std::sync::Mutex::new(std::collections::HashMap::new()),
            disk_revs: RwLock::new(std::collections::HashMap::new()),
            referenced_names_cache: RwLock::new(None),
            page_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
            managed_sync: std::sync::Mutex::new(None),
            search_lanes: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Construct a read-only graph projection from one caller-owned document
    /// snapshot. The empty `root` is only a fail-closed fallback: whole-graph
    /// consumers use the preinstalled cache and page list, so they can never
    /// mix these documents with a later revision from the live graph.
    pub(crate) fn from_page_snapshot(
        root: impl AsRef<Path>,
        mut pages: Vec<(PageEntry, Arc<Document>)>,
    ) -> Graph {
        for (entry, document) in &mut pages {
            assign_doc_runtime_ids(&mut Arc::make_mut(document).roots, &entry.rel_path);
        }
        let graph = Graph::open(root);
        let entries = pages.iter().map(|(entry, _)| entry.clone()).collect();
        let index = build_page_cache_index(&pages);
        let reference_index = ReferenceCandidateIndex::build(0, &pages);
        *graph.cache.write().unwrap() = Some(Arc::new(pages));
        *graph.cache_index.write().unwrap() = Some(index);
        *graph.reference_candidate_index.write().unwrap() = Some(reference_index);
        *graph.page_list_cache.write().unwrap() = Some((0, entries));
        graph
    }

    /// The write lock for a resolved page path (see `page_locks`). Returns an
    /// `Arc` the caller holds (`let _g = lock.lock().unwrap();`) for the critical
    /// section. The `page_locks` map mutex is released before the per-page lock is
    /// taken, so callers never serialize on the map. Opportunistically prunes
    /// entries no caller still holds (strong_count == 1) to bound growth.
    fn page_lock(&self, path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
        let mut map = self.page_locks.lock().unwrap();
        if map.len() >= 64 {
            map.retain(|_, v| std::sync::Arc::strong_count(v) > 1);
        }
        map.entry(path.to_path_buf())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
            .clone()
    }

    pub fn meta(&self) -> GraphMeta {
        GraphMeta {
            root: self.root.display().to_string(),
            journals_dir: self.config.journals_dir.clone(),
            pages_dir: self.config.pages_dir.clone(),
            preferred_workflow: match self.config.preferred_workflow {
                crate::config::Workflow::Todo => "todo".into(),
                crate::config::Workflow::Now => "now".into(),
            },
            shortcuts: self.config.shortcuts.clone(),
            start_of_week: self.config.start_of_week,
            block_hidden_properties: self.config.block_hidden_properties.clone(),
            default_journal_template: self.config.default_journal_template.clone(),
            favorites: self.config.favorites.clone(),
            journal_page_title_format: self.journal_format.title_format().to_string(),
            journal_file_name_format: self.journal_format.file_format().to_string(),
            preferred_format: self.config.preferred_format.ext().to_string(),
            macros: self.config.macros.clone(),
            enable_timetracking: self.config.enable_timetracking,
            show_brackets: self.config.show_brackets,
            doc_mode_enter_for_new_block: self.config.doc_mode_enter_for_new_block,
            logical_outdenting: self.config.logical_outdenting,
            logbook_with_second_support: self.config.logbook.with_second_support,
            logbook_enabled_in_timestamped_blocks: self
                .config
                .logbook
                .enabled_in_timestamped_blocks,
            logbook_enabled_in_all_blocks: self.config.logbook.enabled_in_all_blocks,
            guide_announced: self.config.guide_announced,
        }
    }

    /// Current cache generation — bumped on every cache-mutating page change, and
    /// the key that memoized queries/backlinks/derived results invalidate against.
    /// Exposed for observability and tests (e.g. asserting a no-op save doesn't
    /// needlessly invalidate everything).
    pub fn cache_generation(&self) -> u64 {
        self.cache_gen.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Pages skipped by the latest whole-graph search-cache build because their
    /// parse/projection panicked. Paths are graph-relative and safe to surface.
    pub fn page_index_failures(&self) -> Vec<String> {
        self.page_index_failures.read().unwrap().clone()
    }

    pub fn journals_path(&self) -> PathBuf {
        self.root.join(&self.config.journals_dir)
    }

    pub fn pages_path(&self) -> PathBuf {
        self.root.join(&self.config.pages_dir)
    }

    pub fn managed_sync_store_path(&self) -> PathBuf {
        self.root.join(".tine-sync").join("v1")
    }

    pub fn managed_sync_configured(&self) -> bool {
        self.managed_sync_store_state()
            .is_ok_and(|state| state == ManagedSyncStoreState::Initialized)
    }

    pub fn managed_sync_store_state(&self) -> io::Result<ManagedSyncStoreState> {
        CrdtGraph::store_state(&self.root).map_err(crdt_io_error)
    }

    /// Replay an existing managed-sync workspace for this process. A configured
    /// but invalid workspace fails closed: callers must not let the graph accept
    /// projection-only edits while its operation truth is unavailable.
    pub fn start_managed_sync(&self, device_id: Uuid, session_id: Uuid) -> io::Result<bool> {
        if self.managed_sync_store_state()? != ManagedSyncStoreState::Initialized {
            return Ok(false);
        }
        let crdt = CrdtGraph::open(&self.root, device_id, session_id).map_err(crdt_io_error)?;
        *self.managed_sync.lock().unwrap() = Some(crdt);
        Ok(true)
    }

    /// One-time compatible-mode activation. Identity writes happen first through
    /// the normal guarded save path; genesis is published only after every page
    /// was re-read and its exact revision rechecked.
    pub fn enable_managed_sync(
        &self,
        device_id: Uuid,
        session_id: Uuid,
    ) -> io::Result<ManagedSyncEnableResult> {
        let store_state = self.managed_sync_store_state()?;
        if store_state == ManagedSyncStoreState::Initialized {
            self.start_managed_sync(device_id, session_id)?;
            let status = self
                .managed_sync_status()
                .ok_or_else(|| io::Error::other("managed sync did not start"))?;
            return Ok(ManagedSyncEnableResult {
                migration: SyncIdentityMigration {
                    pages_changed: 0,
                    blocks_changed: 0,
                },
                status,
            });
        }
        if store_state == ManagedSyncStoreState::Claimed {
            CrdtGraph::validate_resume_device(&self.root, device_id).map_err(crdt_io_error)?;
        }

        let migration = self.migrate_sync_identities()?;
        let mut pages = Vec::new();
        let mut baselines = Vec::new();
        for entry in self.list_pages() {
            let page = self.load_page(&entry)?;
            if page.read_only {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is read-only and cannot join managed sync", page.path),
                ));
            }
            let rev = page
                .rev
                .clone()
                .ok_or_else(|| io::Error::other("loaded page has no disk revision"))?;
            pages.push(crdt_snapshot_for_page(&page, CrdtPageId::new())?);
            baselines.push((entry.path, rev));
        }
        let mut projection_contents = Vec::new();
        for (path, expected) in baselines {
            let current = fs::read_to_string(&path)?;
            if content_rev(&current) != expected {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{} changed while managed sync was being enabled",
                        path.display()
                    ),
                ));
            }
            projection_contents.push((self.rel_path(&path), current));
        }

        let crdt = CrdtGraph::initialize(&self.root, device_id, session_id, pages)
            .map_err(crdt_io_error)?;
        for (path, content) in projection_contents {
            crdt.record_projection(&path, &content)
                .map_err(crdt_io_error)?;
        }
        let status = crdt.status().map_err(crdt_io_error)?;
        *self.managed_sync.lock().unwrap() = Some(crdt);
        Ok(ManagedSyncEnableResult { migration, status })
    }

    pub fn managed_sync_status(&self) -> Option<CrdtStatus> {
        self.managed_sync
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|sync| sync.status().ok())
    }

    /// Commit a verified backup's complete graph-text set as the new operation
    /// truth before the caller copies any projection files. Every block must
    /// already carry a persisted UUID; accepting an older pre-migration backup
    /// would make a crash choose fresh identities nondeterministically.
    pub fn commit_managed_restore(&self, files: &[(String, String)]) -> io::Result<bool> {
        let mut guard = self.managed_sync.lock().unwrap();
        let Some(sync) = guard.as_mut() else {
            return Ok(false);
        };
        let current_pages = sync.materialize_pages().map_err(crdt_io_error)?;
        let current_by_path: std::collections::HashMap<String, CrdtPageId> = current_pages
            .iter()
            .cloned()
            .into_iter()
            .map(|page| (page.path, page.id))
            .collect();

        let mut snapshots = Vec::with_capacity(files.len());
        let mut graph_ids = std::collections::HashSet::new();
        let mut seen_paths = std::collections::HashSet::new();
        for (rel, content) in files {
            if !seen_paths.insert(rel.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("backup contains duplicate graph path {rel}"),
                ));
            }
            let path = self.resolve_rel(rel).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("backup contains invalid graph path {rel}"),
                )
            })?;
            let entry = self.entry_for_path(&path).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("backup path is not a page or journal: {rel}"),
                )
            })?;
            let mut doc = parse_doc(&path, content);
            assign_doc_runtime_ids(&mut doc.roots, rel);
            let mut page = page_dto(&entry, &doc);
            page.path = rel.clone();
            if page.read_only {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("backup contains read-only Org content at {rel}"),
                ));
            }
            if count_missing_sync_ids(&page.blocks, page.format) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "backup {rel} predates managed-sync identities and cannot be restored while managed sync is active"
                    ),
                ));
            }
            let page = page_with_persisted_sync_ids(&page)?;
            validate_graph_sync_ids(&page.blocks, page.format, &mut graph_ids)?;
            let page_id = current_by_path
                .get(rel)
                .copied()
                .unwrap_or_else(CrdtPageId::new);
            snapshots.push(crdt_snapshot_for_page(&page, page_id)?);
        }
        let affected_paths: std::collections::BTreeSet<String> = current_pages
            .iter()
            .map(|page| page.path.clone())
            .chain(files.iter().map(|(path, _)| path.clone()))
            .collect();
        let mut projection_preconditions = Vec::with_capacity(affected_paths.len());
        for rel in affected_paths {
            let path = self.resolve_rel(&rel).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("managed-sync restore contains invalid path {rel}"),
                )
            })?;
            let expected_content = match fs::read_to_string(path) {
                Ok(content) => Some(content),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            projection_preconditions.push(ProjectionPrecondition {
                path: rel,
                expected_content,
            });
        }
        sync.replace_pages_with_projection_preconditions(snapshots, projection_preconditions)
            .map(|report| report.changed)
            .map_err(crdt_io_error)
    }

    /// Import newly delivered immutable chunks, project their affected pages,
    /// and clean only conflict copies proven to be old generated projections.
    pub fn pull_managed_sync(&self) -> io::Result<ManagedSyncPull> {
        let report = {
            let mut guard = self.managed_sync.lock().unwrap();
            let Some(sync) = guard.as_mut() else {
                return Ok(ManagedSyncPull::default());
            };
            sync.import_pending().map_err(crdt_io_error)?
        };

        let mut changes = Vec::new();
        for affected in &report.affected_pages {
            changes.extend(self.project_managed_page(affected.page_id, &affected.paths)?);
        }
        let conflicts_changed = self.cleanup_known_projection_conflicts()?;
        {
            let mut guard = self.managed_sync.lock().unwrap();
            guard
                .as_mut()
                .ok_or_else(|| io::Error::other("managed sync is not active"))?
                .acknowledge_pending_projection();
        }
        Ok(ManagedSyncPull {
            imported_chunks: report.imported_chunks,
            changes,
            conflicts_changed,
        })
    }

    /// Reconcile every current CRDT page against its projection after opening a
    /// workspace. This covers the case where provider chunks arrived while Tine
    /// was closed, so `open()` already replayed them and `import_pending()` has no
    /// newly-seen chunk to report.
    pub fn project_all_managed_sync(&self) -> io::Result<ManagedSyncPull> {
        let affected_pages = {
            let guard = self.managed_sync.lock().unwrap();
            let Some(sync) = guard.as_ref() else {
                return Ok(ManagedSyncPull::default());
            };
            sync.affected_pages_history().map_err(crdt_io_error)?
        };
        let mut changes = Vec::new();
        for affected in affected_pages {
            changes.extend(self.project_managed_page(affected.page_id, &affected.paths)?);
        }
        let conflicts_changed = self.cleanup_known_projection_conflicts()?;
        Ok(ManagedSyncPull {
            imported_chunks: 0,
            changes,
            conflicts_changed,
        })
    }

    fn project_managed_page(
        &self,
        page_id: CrdtPageId,
        affected_paths: &[String],
    ) -> io::Result<Vec<ManagedSyncProjectionChange>> {
        let snapshot = {
            let guard = self.managed_sync.lock().unwrap();
            guard
                .as_ref()
                .ok_or_else(|| io::Error::other("managed sync is not active"))?
                .materialize_page(page_id)
                .map_err(crdt_io_error)?
        };
        let Some(snapshot) = snapshot else {
            return self.project_managed_deletion(page_id, affected_paths);
        };
        let path = self.resolve_rel(&snapshot.path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid synced page path")
        })?;
        let lock = self.page_lock(&path);
        let _guard = lock.lock().unwrap();
        let before = fs::read_to_string(&path).ok();
        if let Some(content) = before.as_deref() {
            let authorized = {
                let guard = self.managed_sync.lock().unwrap();
                guard
                    .as_ref()
                    .ok_or_else(|| io::Error::other("managed sync is not active"))?
                    .is_projection_authorized(&snapshot.path, before.as_deref())
                    .map_err(crdt_io_error)?
            };
            if !authorized
                && self.reconcile_managed_external_locked(&path, content, Some(page_id))?
            {
                let mut changes = self
                    .entry_for_path(&path)
                    .map(|entry| {
                        vec![ManagedSyncProjectionChange {
                            entry,
                            created: false,
                            removed: false,
                        }]
                    })
                    .unwrap_or_default();
                drop(_guard);
                changes
                    .extend(self.cleanup_superseded_managed_paths(&snapshot.path, affected_paths)?);
                return Ok(changes);
            }
        }

        // Re-materialize after the external check: it may have committed a local
        // operation on another path while this pull was in progress.
        let snapshot = {
            let guard = self.managed_sync.lock().unwrap();
            guard
                .as_ref()
                .ok_or_else(|| io::Error::other("managed sync is not active"))?
                .materialize_page(page_id)
                .map_err(crdt_io_error)?
                .ok_or_else(|| io::Error::other("synced page vanished during projection"))?
        };
        let page = page_dto_from_crdt(&snapshot)?;
        let cache = self.path_is_cacheable(&path);
        self.write_page(&page, &path, before.as_deref(), true, cache)?;
        self.record_managed_projection(&path);
        let after = fs::read_to_string(&path).ok();
        let mut changes = Vec::new();
        if before == after {
            drop(_guard);
            changes.extend(self.cleanup_superseded_managed_paths(&snapshot.path, affected_paths)?);
            return Ok(changes);
        }
        if let Some(entry) = self.entry_for_path(&path) {
            changes.push(ManagedSyncProjectionChange {
                entry,
                created: before.is_none(),
                removed: false,
            });
        }
        drop(_guard);
        changes.extend(self.cleanup_superseded_managed_paths(&snapshot.path, affected_paths)?);
        Ok(changes)
    }

    fn cleanup_superseded_managed_paths(
        &self,
        current_path: &str,
        affected_paths: &[String],
    ) -> io::Result<Vec<ManagedSyncProjectionChange>> {
        let mut changes = Vec::new();
        for rel in affected_paths {
            if rel == current_path {
                continue;
            }
            let Some(path) = self.resolve_rel(rel) else {
                continue;
            };
            let lock = self.page_lock(&path);
            let _guard = lock.lock().unwrap();
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let sync_guard = self.managed_sync.lock().unwrap();
            let sync = sync_guard
                .as_ref()
                .ok_or_else(|| io::Error::other("managed sync is not active"))?;
            // A historical old path may already be the current projection of a
            // different live identity (for example, a rename swap). Keep the
            // ownership check and trash move atomic against CRDT mutations.
            if sync
                .materialize_page(rel.as_str())
                .map_err(crdt_io_error)?
                .is_some()
            {
                continue;
            }
            let removable = sync
                .is_projection_authorized(rel, Some(&content))
                .map_err(crdt_io_error)?
                || sync
                    .is_known_projection(rel, &content)
                    .map_err(crdt_io_error)?;
            if !removable {
                continue; // unexplained bytes are evidence, never auto-removed
            }
            let Some(entry) = self.entry_for_path(&path) else {
                continue;
            };
            let trash = typed_trash_dir(
                &self.root,
                match entry.kind {
                    PageKind::Journal => TrashEntryKind::Journal,
                    PageKind::Page => TrashEntryKind::Page,
                },
            );
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("page");
            let dest = trash.join(format!("{}__{name}", trash_stamp()));
            move_to_trash(&path, &dest, &trash)?;
            drop(sync_guard);
            self.cache_remove(&entry.name, entry.kind);
            changes.push(ManagedSyncProjectionChange {
                entry,
                created: false,
                removed: true,
            });
        }
        Ok(changes)
    }

    fn project_managed_deletion(
        &self,
        page_id: CrdtPageId,
        affected_paths: &[String],
    ) -> io::Result<Vec<ManagedSyncProjectionChange>> {
        let mut changes = Vec::new();
        for rel in affected_paths {
            let Some(path) = self.resolve_rel(rel) else {
                continue;
            };
            let lock = self.page_lock(&path);
            let _guard = lock.lock().unwrap();
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let sync_guard = self.managed_sync.lock().unwrap();
            let sync = sync_guard
                .as_ref()
                .ok_or_else(|| io::Error::other("managed sync is not active"))?;
            // Chunk envelopes retain deleted identities and historical paths.
            // They may request deletion after another live identity has already
            // materialized at the same path; ownership always wins over a receipt.
            if sync
                .materialize_page(rel.as_str())
                .map_err(crdt_io_error)?
                .is_some()
            {
                continue;
            }
            let removable = sync
                .is_projection_authorized(rel, Some(&content))
                .map_err(crdt_io_error)?
                || sync
                    .is_known_projection(rel, &content)
                    .map_err(crdt_io_error)?;
            if !removable {
                drop(sync_guard);
                if self.reconcile_managed_external_locked(&path, &content, Some(page_id))? {
                    if let Some(entry) = self.entry_for_path(&path) {
                        changes.push(ManagedSyncProjectionChange {
                            entry,
                            created: false,
                            removed: false,
                        });
                    }
                }
                continue;
            }
            let Some(entry) = self.entry_for_path(&path) else {
                continue;
            };
            let trash = typed_trash_dir(
                &self.root,
                match entry.kind {
                    PageKind::Journal => TrashEntryKind::Journal,
                    PageKind::Page => TrashEntryKind::Page,
                },
            );
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("page");
            let dest = trash.join(format!("{}__{name}", trash_stamp()));
            move_to_trash(&path, &dest, &trash)?;
            drop(sync_guard);
            self.cache_remove(&entry.name, entry.kind);
            changes.push(ManagedSyncProjectionChange {
                entry,
                created: false,
                removed: true,
            });
        }
        Ok(changes)
    }

    pub fn cleanup_known_projection_conflicts(&self) -> io::Result<bool> {
        let mut changed = false;
        for conflict in self.list_sync_conflicts() {
            let Some(base_path) = conflict.base_path.as_deref() else {
                continue;
            };
            let Some(conflict_path) = self.resolve_rel(&conflict.path) else {
                continue;
            };
            let content = fs::read_to_string(&conflict_path)?;
            let known = {
                let guard = self.managed_sync.lock().unwrap();
                let Some(sync) = guard.as_ref() else {
                    return Ok(changed);
                };
                sync.is_known_projection(base_path, &content)
                    .map_err(crdt_io_error)?
            };
            if known {
                self.trash_sync_conflict(&conflict.path)?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Graph-root-relative, forward-slashed path for an absolute file path inside
    /// the graph (`…/journals/2026_06_26.org` → `journals/2026_06_26.org`). The
    /// stable, machine-portable id Tine hands the frontend so a page can be pinned
    /// to a SPECIFIC file (#21). Falls back to the input lossily if it's somehow
    /// outside the root (shouldn't happen for graph files).
    pub fn rel_path(&self, abs: &Path) -> String {
        slash_path(abs.strip_prefix(&self.root).unwrap_or(abs))
    }

    /// Resolve a graph-root-relative path (as produced by [`rel_path`]) back to an
    /// absolute file path, validating it points at a real graph text file. This is
    /// the security gate for every path-addressed command (#21): it accepts ONLY
    /// `.md`/`.org` files under `<journals-dir>/` or `<pages-dir>/`, with nested
    /// sub-directories allowed but no `..`/`.`/absolute/empty/backslash segments.
    /// Anything else returns `None`, so a path-addressed read/save can never
    /// escape the graph.
    pub fn resolve_rel(&self, rel: &str) -> Option<PathBuf> {
        let rel = rel.trim();
        if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') {
            return None;
        }
        let mut parts = rel.split('/');
        let dir = parts.next()?;
        let base = if dir == self.config.journals_dir {
            self.journals_path()
        } else if dir == self.config.pages_dir {
            self.pages_path()
        } else {
            return None;
        };
        // The remaining segments are the file's path UNDER that dir. Nested
        // sub-directories are allowed (#21) but the can't-escape-the-graph
        // invariant is kept lexically: every segment must be a plain name — no
        // empty segment (`a//b`, a trailing `/`), no `.`/`..` traversal. With no
        // `..` and no absolute/backslash (rejected above), `base.join(tail)`
        // provably stays within `base`; there must be at least one segment (a bare
        // `pages` is a dir, not a file).
        let mut tail = PathBuf::new();
        for seg in parts {
            if seg.is_empty() || seg == "." || seg == ".." {
                return None;
            }
            tail.push(seg);
        }
        if tail.as_os_str().is_empty() {
            return None;
        }
        let abs = base.join(tail);
        if !path_stays_within_root(&self.root, &abs) || path_uses_managed_alias(&self.root, &abs) {
            return None;
        }
        match abs.extension().and_then(|e| e.to_str()) {
            Some("md") | Some("org") => Some(abs),
            _ => None,
        }
    }

    /// Resolve the exact on-disk source file for an explicit user file action.
    /// A loaded page's recorded relative path always wins (including nested and
    /// duplicate-name files); a newly saved page without a refreshed path may
    /// fall back to normal name resolution. The final canonical-file check keeps
    /// symlinks from escaping the managed pages/journals directories.
    pub fn page_source_file(
        &self,
        name: &str,
        kind: PageKind,
        recorded_path: Option<&str>,
    ) -> io::Result<PathBuf> {
        let candidate = recorded_path
            .filter(|path| !path.trim().is_empty())
            .map(|path| {
                self.resolve_rel(path)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid page path"))
            })
            .unwrap_or_else(|| Ok(self.path_for(name, kind)))?;
        let canonical = candidate.canonicalize()?;
        if !canonical.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "page source is not a file",
            ));
        }
        let pages = self.pages_path().canonicalize()?;
        let journals = self.journals_path().canonicalize()?;
        if !canonical.starts_with(&pages) && !canonical.starts_with(&journals) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "page source escapes graph directories",
            ));
        }
        Ok(canonical)
    }

    /// Whether a journal file is a "shadow": a non-date-stem file (e.g. a leftover
    /// title-named `Friday, 26-06-2026.org`) that coexists with a canonical
    /// date-stem file (`2026_06_26.{md,org}`) for the SAME day. The `(kind,name)`
    /// cache slot belongs to the canonical file, so a shadow must never be folded
    /// into it (that would make name-resolution serve the shadow's content). A
    /// shadow is loaded fresh by path on demand instead (#21). Twins (two date-stem
    /// files of the same day in different extensions) are deliberately NOT shadows —
    /// that case keeps its existing `has_twin`/dedup handling.
    fn is_shadow_journal(&self, path: &Path, date: crate::date::JournalDate) -> bool {
        let is_date_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| crate::date::JournalDate::from_file_stem(s).is_some());
        if is_date_stem {
            return false;
        }
        let canon = self.journal_format.file_stem(date);
        let dir = self.journals_path();
        dir.join(format!("{canon}.md")).is_file() || dir.join(format!("{canon}.org")).is_file()
    }

    /// The format (`Md`/`Org`) new pages and journals are created in, from
    /// `config.edn`'s `:preferred-format`. Existing files keep their own format.
    pub fn preferred_format(&self) -> Format {
        self.config.preferred_format
    }

    /// List all pages and journals in the graph.
    pub fn list_pages(&self) -> Vec<PageEntry> {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if let Some((g, entries)) = self.page_list_cache.read().unwrap().as_ref() {
            if *g == gen {
                return entries.clone();
            }
        }
        let mut entries = Vec::new();
        let nf = self.config.file_name_format;
        entries.extend(list_md(
            &self.journals_path(),
            PageKind::Journal,
            &self.journal_format,
            nf,
            &self.config.journals_dir,
        ));
        entries.extend(list_md(
            &self.pages_path(),
            PageKind::Page,
            &self.journal_format,
            nf,
            &self.config.pages_dir,
        ));
        // A duplicate-day journal (canonical + leftover title-named file) must show
        // once in quick-switch / All-Pages, not twice (both resolve to one page).
        let entries = dedup_journal_days(entries);
        *self.page_list_cache.write().unwrap() = Some((gen, entries.clone()));
        entries
    }

    /// Page names referenced anywhere in the graph — inline `[[link]]`/`#tag`/
    /// `#[[..]]` plus `tags::`/`alias::` property values (block- and page-level) —
    /// display case preserved, deduped case-insensitively. These are the pages
    /// that "exist" by reference even without a file of their own (OG semantics),
    /// so autocomplete/quick-switch can offer them instead of "Create …".
    ///
    /// Computed from the whole-graph cache and memoized by `cache_gen`. If the
    /// cache isn't warm yet it returns empty and memoizes nothing — we never force
    /// a full-graph parse from here (this runs on autocomplete keystrokes).
    pub fn referenced_page_names(&self) -> Vec<String> {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if let Some((g, names)) = self.referenced_names_cache.read().unwrap().as_ref() {
            if *g == gen {
                return names.clone();
            }
        }
        let guard = self.cache.read().unwrap();
        let Some(pages) = guard.as_ref() else {
            return Vec::new(); // cache not warm — don't force a parse, don't memoize
        };
        fn add(seen: &mut std::collections::HashMap<String, String>, name: String) {
            if !name.is_empty() {
                seen.entry(crate::refs::page_key(&name)).or_insert(name);
            }
        }
        // `tags::` / `alias::` property values are page references in OG too —
        // comma-separated, written bare or as `[[..]]`/`#..` — so a page named
        // only in a `tags::`/`alias::` list still "exists". Strip any wrapping
        // down to the page name. (Line-based, like DocBlock::property.)
        fn add_property_refs(seen: &mut std::collections::HashMap<String, String>, text: &str) {
            for line in text.lines() {
                let Some((k, v)) = crate::doc::parse_property_line(line) else {
                    continue;
                };
                if !(k.eq_ignore_ascii_case("tags")
                    || k.eq_ignore_ascii_case("alias")
                    || k.eq_ignore_ascii_case("aliases"))
                {
                    continue;
                }
                let quoted = v.trim();
                if quoted.len() >= 2 && quoted.starts_with('"') && quoted.ends_with('"') {
                    continue;
                }
                for val in v.split([',', '，']) {
                    let t = val.trim();
                    let t = t.strip_prefix('#').unwrap_or(t).trim();
                    let t = t
                        .strip_prefix("[[")
                        .and_then(|x| x.strip_suffix("]]"))
                        .unwrap_or(t);
                    add(seen, t.trim().to_string());
                }
            }
        }
        fn visit(b: &DocBlock, seen: &mut std::collections::HashMap<String, String>) {
            // Read the memoized projection's original-case page refs instead of a fresh
            // `block_refs` parse — this runs over the WHOLE graph on every `[[`/`#`/Ctrl-K
            // keystroke after a save (each bumps cache_gen), so re-parsing every block was
            // a ~0.5s keystroke stall on a large graph (audit F1).
            for name in &b.projection().refs_page {
                add(seen, name.clone());
            }
            add_property_refs(seen, &b.raw); // block-level tags::/alias::
            for c in &b.children {
                visit(c, seen);
            }
        }
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (_, doc) in pages.iter() {
            if let Some(pre) = &doc.pre_block {
                add_property_refs(&mut seen, pre); // page-level tags::/alias::
            }
            for b in &doc.roots {
                visit(b, &mut seen);
            }
        }
        drop(guard);
        let names: Vec<String> = seen.into_values().collect();
        *self.referenced_names_cache.write().unwrap() = Some((gen, names.clone()));
        names
    }

    /// Journals sorted newest-first.
    pub fn journals_desc(&self) -> Vec<PageEntry> {
        // Prefer the warmed whole-graph cache — its PageEntry list is kept current
        // by cache_upsert/cache_remove, so we avoid a directory read + parse on
        // every infinite-scroll feed append. Fall back to scanning the dir while
        // the cache isn't built yet.
        let raw: Vec<PageEntry> = match self.cache.read().unwrap().as_ref() {
            Some(pages) => pages
                .iter()
                .filter(|(e, _)| e.kind == PageKind::Journal && e.date_key.is_some())
                .map(|(e, _)| e.clone())
                .collect(),
            None => list_md(
                &self.journals_path(),
                PageKind::Journal,
                &self.journal_format,
                self.config.file_name_format,
                &self.config.journals_dir,
            )
            .into_iter()
            .filter(|e| e.date_key.is_some())
            .collect(),
        };
        // A day with more than one file (e.g. a leftover title-named duplicate of
        // a `yyyy_MM_dd` file) must appear ONCE — both files resolve to the same
        // page name, so otherwise the day renders twice. The stray stays visible
        // via journal_conflicts() for reconciliation.
        let mut js = dedup_journal_days(raw);
        js.sort_by_key(|e| std::cmp::Reverse(e.date_key.unwrap_or(0)));
        js
    }

    /// Feed membership is narrower than the raw journal inventory: future
    /// journals remain directly reachable graph pages, but are not in Journals.
    pub fn feed_journals_desc_through(&self, cutoff: JournalDate) -> Vec<PageEntry> {
        let cutoff = cutoff.ordinal_key();
        self.journals_desc()
            .into_iter()
            .filter(|entry| entry.date_key.is_some_and(|day| day <= cutoff))
            .collect()
    }

    pub fn feed_journals_desc(&self) -> Vec<PageEntry> {
        self.feed_journals_desc_through(JournalDate::today())
    }

    /// Journal `date_key`s (yyyymmdd) whose page has real content — i.e. at
    /// least one block with a non-empty, non-property line. Drives the calendar
    /// picker's empty/non-empty day marking. Served from the cache.
    pub fn journal_content_days(&self) -> Vec<i64> {
        self.with_pages(|pages| {
            pages
                .iter()
                .filter(|(e, _)| e.kind == PageKind::Journal)
                .filter_map(|(e, d)| e.date_key.filter(|_| doc_has_content(&d.roots)))
                .collect()
        })
    }

    /// One-time recovery: a journal that was saved under its display title
    /// ("Jun 18th, 2026.md") instead of its date stem ("2026_06_18.md") can't be
    /// parsed back to a date, so it drops out of the feed and the day looks
    /// empty. Rename such files to their stem — but only when the stem file
    /// doesn't already exist (never clobber/merge). Returns how many were fixed.
    pub fn has_journal_filename_migrations(&self) -> bool {
        let dir = self.journals_path();
        let Ok(rd) = fs::read_dir(&dir) else {
            return false;
        };
        for e in rd.flatten() {
            let p = e.path();
            if self.journal_filename_migration_target(&p).is_some() {
                return true;
            }
        }
        false
    }

    fn journal_filename_migration_target(&self, p: &std::path::Path) -> Option<PathBuf> {
        // Both formats — an org graph's title-named journals are `.org`.
        let ext = match p.extension().and_then(|x| x.to_str()) {
            Some(e @ ("md" | "org")) => e,
            _ => return None,
        };
        let stem = p.file_stem().and_then(|s| s.to_str())?;
        if JournalDate::from_file_stem(stem).is_some() {
            return None; // already a plausible date stem (yyyy_MM_dd / yyyy-MM-dd) — leave it
        }
        // A title-named ("Jun 18th, 2026.md", "Thursday, 25-06-2026.org") or
        // otherwise non-stem journal file: normalize it to the graph's filename
        // format so it round-trips with OG and is recognized in the feed.
        let d = self.journal_format.parse(stem)?;
        let want = self.journal_format.file_stem(d);
        if want == stem {
            return None; // already in the graph's filename format
        }
        let target = self.journals_path().join(format!("{want}.{ext}"));
        if target.exists() {
            return None; // don't clobber an existing stem file
        }
        Some(target)
    }

    pub fn migrate_journal_filenames(&self) -> usize {
        self.migrate_journal_filenames_checked().unwrap_or(0)
    }

    pub fn migrate_journal_filenames_checked(&self) -> io::Result<usize> {
        let dir = self.journals_path();
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        let mut n = 0;
        for e in rd {
            let e = e?;
            let p = e.path();
            if let Some(target) = self.journal_filename_migration_target(&p) {
                let managed = self.managed_sync.lock().unwrap().is_some();
                if !managed {
                    if move_file_noreplace(&p, &target).is_ok() {
                        n += 1;
                    }
                    continue;
                }

                let mut lock_paths = [p.clone(), target.clone()];
                lock_paths.sort();
                let locks: Vec<_> = lock_paths.iter().map(|path| self.page_lock(path)).collect();
                let guards: Vec<_> = locks.iter().map(|lock| lock.lock().unwrap()).collect();
                if target.exists()
                    || self.journal_filename_migration_target(&p).as_ref() != Some(&target)
                {
                    continue;
                }
                let content = fs::read_to_string(&p)?;
                let source_rel = self.rel_path(&p);
                let target_rel = self.rel_path(&target);
                let (page_id, report) = {
                    let mut sync_guard = self.managed_sync.lock().unwrap();
                    let sync = sync_guard
                        .as_mut()
                        .ok_or_else(|| io::Error::other("managed sync stopped during migration"))?;
                    let existing = sync
                        .materialize_page(source_rel.as_str())
                        .map_err(crdt_io_error)?
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "managed-sync operation truth has no journal at {source_rel}"
                                ),
                            )
                        })?;
                    let entry = self.entry_for_path(&target).ok_or_else(bad_path)?;
                    let mut doc = parse_doc(&target, &content);
                    assign_doc_runtime_ids(&mut doc.roots, &target_rel);
                    let mut page = page_dto(&entry, &doc);
                    page.path = target_rel;
                    page.format = Format::from_path(&target);
                    page = page_with_persisted_sync_ids(&page)?;
                    let page_id = existing.id;
                    let report = sync
                        .commit_page(crdt_snapshot_for_page(&page, page_id)?)
                        .map_err(crdt_io_error)?;
                    (page_id, report)
                };
                drop(guards);
                self.project_managed_page(page_id, &report.affected_paths)?;
                if target.exists() && !p.exists() {
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// Journal days that resolve to more than one file — the migration leaves these
    /// alone (it never clobbers), so they're reported for the user to reconcile.
    /// Each file gets a one-line preview and a `canonical` flag (date-stem name).
    pub fn journal_conflicts(&self) -> Vec<JournalConflict> {
        let dir = self.journals_path();
        let mut by_date: std::collections::BTreeMap<i64, Vec<(String, PathBuf, bool)>> =
            std::collections::BTreeMap::new();
        walk_page_files(&dir, |p| {
            let ext = match p.extension().and_then(|x| x.to_str()) {
                Some(x @ ("md" | "org")) => x.to_string(),
                _ => return,
            };
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                return;
            };
            // A date-stem file is canonical; otherwise try to parse its title.
            let canonical = JournalDate::from_file_stem(stem).is_some();
            let date =
                JournalDate::from_file_stem(stem).or_else(|| self.journal_format.parse(stem));
            if let Some(d) = date {
                by_date.entry(d.ordinal_key()).or_default().push((
                    format!("{stem}.{ext}"),
                    p,
                    canonical,
                ));
            }
        });
        let mut out = Vec::new();
        for (key, files) in by_date {
            if files.len() < 2 {
                continue;
            }
            let date = JournalDate::from_ordinal(key);
            let mut jfiles: Vec<JournalFile> = files
                .into_iter()
                .map(|(name, path, canonical)| {
                    let preview = fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| {
                            c.lines()
                                .map(|l| {
                                    l.trim_start_matches(|ch| {
                                        ch == '*' || ch == '-' || ch == ' ' || ch == '\t'
                                    })
                                    .trim()
                                    .to_string()
                                })
                                .find(|l| !l.is_empty())
                        })
                        .map(|l| l.chars().take(80).collect::<String>())
                        .unwrap_or_default();
                    let rel = self.rel_path(&path);
                    JournalFile {
                        name,
                        path: rel,
                        preview,
                        canonical,
                    }
                })
                .collect();
            // Canonical first (the keeper), then alphabetical.
            jfiles.sort_by(|a, b| {
                b.canonical
                    .cmp(&a.canonical)
                    .then_with(|| a.name.cmp(&b.name))
            });
            out.push(JournalConflict {
                title: self.journal_format.title(date),
                files: jfiles,
            });
        }
        out
    }

    /// Sync-tool conflict copies (`*.sync-conflict-*`, Dropbox `(conflicted copy)`)
    /// sitting in `journals/` or `pages/`. Each carries the winning page it shadows,
    /// that winner's path (if it still exists), a device/timestamp tag, and a
    /// one-line preview — everything the conflicts panel needs to offer a merge.
    /// These files are deliberately excluded from `list_pages`/the cache
    /// (see [`is_sync_conflict`]); this is the ONLY place they're surfaced.
    pub fn list_sync_conflicts(&self) -> Vec<SyncConflict> {
        let mut out = Vec::new();
        for (dir, kind) in [
            (self.journals_path(), PageKind::Journal),
            (self.pages_path(), PageKind::Page),
        ] {
            walk_page_files(&dir, |p| {
                let ext = match p.extension().and_then(|x| x.to_str()) {
                    Some(x @ ("md" | "org")) => x,
                    _ => return,
                };
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                    return;
                };
                let Some(base_stem) = sync_conflict_base(stem) else {
                    return;
                };
                // The winner it shadows: same dir, same extension, base stem.
                let base_file = p
                    .parent()
                    .unwrap_or(&dir)
                    .join(format!("{base_stem}.{ext}"));
                let base_path = base_file.is_file().then(|| self.rel_path(&base_file));
                let base_name = match kind {
                    PageKind::Journal => self
                        .journal_format
                        .parse(base_stem)
                        .map(|d| self.journal_format.title(d))
                        .unwrap_or_else(|| base_stem.to_string()),
                    PageKind::Page => decode_page_name(base_stem, self.config.file_name_format),
                };
                let tag = stem[base_stem.len()..]
                    .trim_matches(|c: char| c == '.' || c == ' ' || c == '(' || c == ')')
                    .to_string();
                let preview = fs::read_to_string(&p)
                    .ok()
                    .and_then(|c| {
                        c.lines()
                            .map(|l| {
                                l.trim_start_matches(|ch| {
                                    ch == '*' || ch == '-' || ch == ' ' || ch == '\t'
                                })
                                .trim()
                                .to_string()
                            })
                            .find(|l| !l.is_empty())
                    })
                    .map(|l| l.chars().take(80).collect::<String>())
                    .unwrap_or_default();
                out.push(SyncConflict {
                    path: self.rel_path(&p),
                    base_name,
                    base_path,
                    kind,
                    tag,
                    preview,
                });
            });
        }
        out.sort_by(|a, b| {
            a.base_name
                .cmp(&b.base_name)
                .then_with(|| a.path.cmp(&b.path))
        });
        out
    }

    /// Structural block-level diff of a conflict copy against its winner (both
    /// graph-root-relative paths). Loads each file directly by path — the conflict
    /// copy is deliberately not in the page cache — and aligns the two block trees
    /// (see [`crate::sync_diff`]). This is a READ; nothing is written. `Ok(None)`
    /// if either path is invalid or the file is gone.
    pub fn sync_conflict_diff(
        &self,
        winner_rel: &str,
        conflict_rel: &str,
    ) -> io::Result<Option<crate::sync_diff::SyncConflictDiff>> {
        let (Some(win), Some(conf)) =
            (self.resolve_rel(winner_rel), self.resolve_rel(conflict_rel))
        else {
            return Ok(None);
        };
        let (win_c, conf_c) = match (fs::read_to_string(&win), fs::read_to_string(&conf)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            (Err(e), _) | (_, Err(e)) => return Err(e),
        };
        let mine = parse_doc(&win, &win_c);
        let theirs = parse_doc(&conf, &conf_c);
        let mut diff = crate::sync_diff::diff_docs(&mine, &theirs);
        diff.base_rev = content_rev(&win_c);
        diff.conflict_rev = content_rev(&conf_c);
        Ok(Some(diff))
    }

    /// Resolve a sync-conflict copy: build the merged winner from the user's
    /// per-row `decisions` (row id → `"mine"`/`"theirs"`/`"both"`, see
    /// [`crate::sync_diff::merge_blocks`]), write it through the NORMAL round-
    /// tripping save path, and move the conflict copy to the recoverable trash.
    ///
    /// Data-safety invariants (ADR 0012 one-writer + ADR 0007 never-silently-
    /// overwrite), mirroring [`merge_pages`]:
    /// - Everything runs under the winner's `page_lock`.
    /// - `base_rev` guard: if the winner changed on disk since the UI diffed it,
    ///   returns `AlreadyExists` ("conflict") WITHOUT writing, so the UI re-diffs
    ///   against fresh content instead of merging a stale alignment.
    /// - Org round-trip firewall: if either side is a non-round-trippable `.org`,
    ///   refuses rather than risk corrupting it.
    /// - Stage-before-commit: the conflict copy is moved to trash BEFORE the
    ///   merged winner is written, and the move is rolled back if the write fails
    ///   — so a retry can never duplicate content, and nothing is lost.
    ///
    /// `pre_choice` decides the page-property pre-block: `"mine"`, `"theirs"`, or
    /// `"union"` (default; markdown only — keep the winner's and add any property
    /// the conflict defines that the winner doesn't, so an `alias::`/`tags::` from
    /// the other device isn't dropped; org keeps the winner's, gated by the
    /// firewall).
    pub fn resolve_sync_conflict(
        &self,
        winner_rel: &str,
        conflict_rel: &str,
        decisions: &std::collections::HashMap<String, String>,
        base_rev: &str,
        conflict_rev: &str,
        pre_choice: &str,
    ) -> io::Result<()> {
        let win = self.resolve_rel(winner_rel).ok_or_else(bad_path)?;
        let conf = self.resolve_rel(conflict_rel).ok_or_else(bad_path)?;
        if win == conf {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "winner and conflict are the same file",
            ));
        }
        let win_entry = self.entry_for_path(&win).ok_or_else(bad_path)?;
        // Lock the winner so a concurrent editor/watcher write can't race the merge.
        let lock = self.page_lock(&win);
        let _guard = lock.lock().unwrap();
        let win_content = fs::read_to_string(&win)?;
        let conf_content = fs::read_to_string(&conf)?;
        // base_rev guard — the winner must still be what the UI diffed against.
        if content_rev(&win_content) != base_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "winner changed on disk",
            ));
        }
        if content_rev(&conf_content) != conflict_rev {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "conflict copy changed on disk",
            ));
        }
        // Org round-trip firewall (same as merge_pages).
        if Format::from_path(&win) == Format::Org
            && (!crate::org::org_editable(&win_content) || !crate::org::org_editable(&conf_content))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "an org file in this pair does not round-trip; not merging",
            ));
        }
        let mine_doc = parse_doc(&win, &win_content);
        let theirs_doc = parse_doc(&conf, &conf_content);
        let merged_roots =
            crate::sync_diff::merge_blocks(&mine_doc.roots, &theirs_doc.roots, decisions);
        let pre_block = match pre_choice {
            "theirs" => theirs_doc.pre_block.clone(),
            "mine" => mine_doc.pre_block.clone(),
            _ if Format::from_path(&win) == Format::Md => union_pre(
                mine_doc.pre_block.as_deref(),
                theirs_doc.pre_block.as_deref(),
            ),
            _ => mine_doc.pre_block.clone(),
        };
        let mut merged = Document {
            pre_block,
            roots: merged_roots,
        };
        assign_doc_runtime_ids(&mut merged.roots, &win_entry.rel_path);
        let dto = page_dto(&win_entry, &merged);
        let win_cacheable = self.path_is_cacheable(&win);
        // Stage-before-commit (L5): move the conflict copy out first, then write the
        // merged winner; roll the move back if the write fails.
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Conflict);
        self.ensure_write_target(&trash)?;
        fs::create_dir_all(&trash)?;
        let conf_name = conf.file_name().and_then(|s| s.to_str()).unwrap_or("file");
        let staged = trash.join(format!("{}__{conf_name}", trash_stamp()));
        move_file_noreplace(&conf, &staged)?;
        if fs::read_to_string(&staged)? != conf_content {
            let _ = move_file_noreplace(&staged, &conf);
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "conflict copy changed during merge",
            ));
        }
        if let Err(e) = self.write_page(&dto, &win, Some(&win_content), true, win_cacheable) {
            let _ = move_file_noreplace(&staged, &conf); // rollback: restore the conflict copy
            return Err(e);
        }
        Ok(())
    }

    /// Move a sync-conflict copy to the recoverable trash WITHOUT merging (the
    /// "I've reviewed it, the winner is fine, discard the copy" affordance). Guards
    /// that the target actually IS a conflict copy so this can never trash a real
    /// page. Recoverable in `logseq/.tine-trash` (ADR 0007).
    pub fn trash_sync_conflict(&self, conflict_rel: &str) -> io::Result<()> {
        let conf = self.resolve_rel(conflict_rel).ok_or_else(bad_path)?;
        if !path_is_sync_conflict(&conf) {
            return Err(bad_path()); // refuse anything that isn't a conflict copy
        }
        if !conf.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no such conflict file",
            ));
        }
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Conflict);
        self.ensure_write_target(&trash)?;
        let name = conf.file_name().and_then(|s| s.to_str()).unwrap_or("file");
        let dest = trash.join(format!("{}__{name}", trash_stamp()));
        move_to_trash(&conf, &dest, &trash)
    }

    /// Raw contents of ONE journal file (by exact filename) — lets the UI show a
    /// duplicate day's individual files (which can't be navigated to separately,
    /// as pages are keyed by date) so the user can inspect before reconciling.
    pub fn read_journal_file(&self, name: &str) -> io::Result<String> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "bad journal file name",
            ));
        }
        fs::read_to_string(self.journals_path().join(name))
    }

    /// Move ONE journal file (by its exact filename) to the recoverable trash —
    /// the affordance for reconciling a duplicate day. Refuses a path separator so
    /// it can't reach outside `journals/`.
    pub fn trash_journal_file(&self, name: &str) -> io::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "bad journal file name",
            ));
        }
        let src = self.journals_path().join(name);
        if !src.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no such journal file",
            ));
        }
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Journal);
        self.ensure_write_target(&trash)?;
        let dest = trash.join(format!("{}__{name}", trash_stamp()));
        move_to_trash(&src, &dest, &trash)?;
        Ok(())
    }

    /// Whether a file participates in the `(kind,name)` page cache. False only for a
    /// shadow journal (a title-named duplicate of a canonical date-stem file, #21),
    /// whose cache slot belongs to the canonical file.
    fn path_is_cacheable(&self, path: &Path) -> bool {
        if let Some(entry) = self.entry_for_path(path) {
            if entry.kind == PageKind::Journal {
                if let Some(date) = entry.date_key.map(crate::date::JournalDate::from_ordinal) {
                    return !self.is_shadow_journal(path, date);
                }
            }
        }
        true
    }

    /// Reconcile a duplicate-day pair: append every block of `src_rel` to the end of
    /// `dst_rel`, then move `src_rel` to the recoverable trash (#21). Both must be
    /// real graph text files of the SAME format (we don't transcode md⇄org), and an
    /// org file that can't be round-tripped is refused so the merge can never
    /// corrupt it (both files are left untouched on any error). `src`'s page
    /// PROPERTIES that `dst` doesn't already define are carried into `dst` (md only;
    /// dst wins on a clash) so an alias/tags/icon isn't silently lost; src free-text
    /// in the pre-block is dropped. The src is trashed ONLY after `dst` is durably
    /// written.
    pub fn merge_pages(&self, src_rel: &str, dst_rel: &str) -> io::Result<()> {
        let src = self.resolve_rel(src_rel).ok_or_else(bad_path)?;
        let dst = self.resolve_rel(dst_rel).ok_or_else(bad_path)?;
        if src == dst {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot merge a file into itself",
            ));
        }
        if Format::from_path(&src) != Format::from_path(&dst) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "files are in different formats",
            ));
        }
        let dst_entry = self.entry_for_path(&dst).ok_or_else(bad_path)?;
        // Write `dst` under its page lock so a concurrent editor/PDF write can't
        // race the merge; read both files inside the lock (the dst baseline must be
        // current for write_page's recheck).
        let lock = self.page_lock(&dst);
        let _guard = lock.lock().unwrap();
        let src_content = fs::read_to_string(&src)?;
        let dst_content = fs::read_to_string(&dst)?;
        if Format::from_path(&dst) == Format::Org
            && (!crate::org::org_editable(&dst_content) || !crate::org::org_editable(&src_content))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "an org file in this pair does not round-trip; not merging",
            ));
        }
        let src_doc = parse_doc(&src, &src_content);
        let mut merged = parse_doc(&dst, &dst_content);
        // Preserve src's page PROPERTIES that dst doesn't already define
        // (alias::/tags::/icon::/…). Dropping them silently is real data loss — a
        // lost `alias::` breaks every inbound link that used the alias. dst's value
        // wins on a key clash (no duplicate property line); markdown only (org
        // pre-blocks are header/drawer-structured and gated by the round-trip
        // firewall, so we don't risk a non-round-tripping merge there). Free text in
        // src's pre-block is still dropped — rare, and src is trashed-recoverable.
        if Format::from_path(&dst) == Format::Md {
            if let Some(src_pre) = src_doc.pre_block.as_deref() {
                let dst_pre = merged.pre_block.clone().unwrap_or_default();
                let dst_keys: std::collections::HashSet<String> = dst_pre
                    .lines()
                    .filter_map(|l| {
                        doc::parse_property_line(l).map(|(k, _)| k.to_ascii_lowercase())
                    })
                    .collect();
                let extra: Vec<&str> = src_pre
                    .lines()
                    .filter(|l| {
                        doc::parse_property_line(l)
                            .is_some_and(|(k, _)| !dst_keys.contains(&k.to_ascii_lowercase()))
                    })
                    .collect();
                if !extra.is_empty() {
                    let mut pre = dst_pre;
                    if !pre.is_empty() && !pre.ends_with('\n') {
                        pre.push('\n');
                    }
                    pre.push_str(&extra.join("\n"));
                    merged.pre_block = Some(pre);
                }
            }
        }
        merged.roots.extend(src_doc.roots);
        assign_doc_runtime_ids(&mut merged.roots, &dst_entry.rel_path);
        let dto = page_dto(&dst_entry, &merged);
        let dst_cacheable = self.path_is_cacheable(&dst);
        // L5: stage `src` into the trash BEFORE committing the merged `dst`. The old
        // order (write dst, then trash src) duplicated blocks on a retry when
        // trashing failed: dst already held src's blocks while src survived on disk,
        // so a second merge re-appended them. Now we move src out first — a staging
        // failure aborts the merge cleanly before any write — and if the dst write
        // then fails we roll the move back, so neither the merge nor the source is
        // lost. On success src sits in the recoverable trash.
        let trash = typed_trash_dir(
            &self.root,
            match self.entry_for_path(&src).map(|e| e.kind) {
                Some(PageKind::Journal) => TrashEntryKind::Journal,
                _ => TrashEntryKind::Page,
            },
        );
        self.ensure_write_target(&trash)?;
        fs::create_dir_all(&trash)?;
        let src_name = src.file_name().and_then(|s| s.to_str()).unwrap_or("file");
        let staged = trash.join(format!("{}__{src_name}", trash_stamp()));
        move_file_noreplace(&src, &staged)?;
        if fs::read_to_string(&staged)? != src_content {
            let _ = move_file_noreplace(&staged, &src);
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "source changed during merge",
            ));
        }
        // The page lock excludes other Tine writers, but not Logseq/Syncthing.
        // Recheck the baseline at commit so an external edit arriving after our
        // read is not silently overwritten.
        if let Err(e) = self.write_page(&dto, &dst, Some(&dst_content), true, dst_cacheable) {
            let _ = move_file_noreplace(&staged, &src); // rollback: restore the source file
            return Err(e);
        }
        Ok(())
    }

    /// Turn a stray file into a normal, uniquely-named page by moving it to
    /// `pages/<encoded new_name>.<its ext>` (#21) — the way to rescue a duplicate-day
    /// leftover whose name collides with the canonical day. Refuses if a page for
    /// `new_name` already exists in EITHER extension (never clobbers) or the name is
    /// empty. Inbound references are NOT rewritten (a stray rarely has any); the
    /// file's own content is unchanged.
    pub fn rename_file_to_page(&self, src_rel: &str, new_name: &str) -> io::Result<()> {
        let src = self.resolve_rel(src_rel).ok_or_else(bad_path)?;
        let name = new_name.trim();
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty page name",
            ));
        }
        let ext = match src.extension().and_then(|e| e.to_str()) {
            Some(e @ ("md" | "org")) => e.to_string(),
            _ => return Err(bad_path()),
        };
        let enc = encode_page_name(name, self.config.file_name_format);
        let dir = self.pages_path();
        if dir.join(format!("{enc}.md")).exists() || dir.join(format!("{enc}.org")).exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a page with that name already exists",
            ));
        }
        fs::create_dir_all(&dir)?;
        move_file_noreplace(&src, &dir.join(format!("{enc}.{ext}")))?;
        // The page SET changed — drop the list memo so the new page (and the stray's
        // disappearance from journals/) show up immediately, and discard the
        // parsed snapshot so its page/index set is rebuilt coherently on next use.
        *self.page_list_cache.write().unwrap() = None;
        *self.find_entry_cache.write().unwrap() = None;
        self.invalidate_cache();
        Ok(())
    }

    /// Resolve a page name to a file path. Journals match by date title;
    /// pages match by filename stem.
    pub(crate) fn path_for(&self, name: &str, kind: PageKind) -> PathBuf {
        let pref = self.preferred_format();
        match kind {
            PageKind::Journal => self
                .journals_desc()
                .into_iter()
                .find(|e| crate::refs::same_page(&e.name, name))
                .map(|e| e.path)
                .unwrap_or_else(|| {
                    // New journal: name it by its date stem in the graph's filename
                    // format ("2026_06_18.org"), not the display title — a
                    // title-named file can't be parsed back to a date, so
                    // journals_desc would drop it and the day would look empty. The
                    // extension follows the graph's :preferred-format.
                    let stem = self
                        .journal_format
                        .parse(name)
                        .map(|d| self.journal_format.file_stem(d))
                        .unwrap_or_else(|| name.to_string());
                    self.journals_path().join(format!("{stem}.{}", pref.ext()))
                }),
            PageKind::Page => {
                // Resolve to an EXISTING file (any format) so a save updates it in
                // place rather than creating a second file in the other extension;
                // a brand-new page is created in the graph's preferred format.
                // Cheap `exists()` probes (the common hit needs one), no dir scan.
                let enc = encode_page_name(name, self.config.file_name_format);
                let dir = self.pages_path();
                let primary = dir.join(format!("{enc}.{}", pref.ext()));
                if primary.exists() {
                    return primary;
                }
                let alt_ext = if pref == Format::Org { "md" } else { "org" };
                let alt = dir.join(format!("{enc}.{alt_ext}"));
                if alt.exists() {
                    return alt;
                }
                primary
            }
        }
    }

    /// Create a Markdown page file with `content` if that logical page does not
    /// already exist. Used by the explicit guide-copy action:
    /// it is intentionally raw Markdown, not a serialized DTO, so copied guide
    /// pages stay ordinary Logseq template pages byte-for-byte.
    ///
    /// Returns `true` when a file was created and `false` when an existing page
    /// won. Existing content is never overwritten.
    pub fn create_markdown_page_if_absent(&self, name: &str, content: &str) -> io::Result<bool> {
        if self.find_entry(name, PageKind::Page).is_some() {
            return Ok(false);
        }
        let path = self.pages_path().join(format!(
            "{}.md",
            encode_page_name(name, self.config.file_name_format)
        ));
        let lock = self.page_lock(&path);
        let _guard = lock.lock().unwrap();
        if self.find_entry(name, PageKind::Page).is_some() || path.exists() {
            return Ok(false);
        }
        self.ensure_write_target(&path)?;
        fs::create_dir_all(self.pages_path())?;
        match atomic_write_new(&path, content.as_bytes()) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) => return Err(e),
        }
        guide_twin_race_hook(&path)?;
        let alt = self.pages_path().join(format!(
            "{}.org",
            encode_page_name(name, self.config.file_name_format)
        ));
        if alt.exists() {
            // An Org twin appeared during publication. Withdraw only the exact
            // guide inode we just created. Stage the currently named inode first
            // and verify it in recovery, so an external replacement that wins at
            // the syscall boundary is restored or retained rather than unlinked.
            let _ = self.withdraw_file_to_conflict_if_exact(
                &path,
                content.as_bytes(),
                "guide-twin-withdrawal",
            )?;
            return Ok(false);
        }
        *self.page_list_cache.write().unwrap() = None;
        *self.find_entry_cache.write().unwrap() = None;
        self.invalidate_cache();
        Ok(true)
    }

    /// Create one named top-level asset without replacing an existing file. The
    /// approved asset capability is revalidated at the actual write target so a
    /// managed-directory symlink/junction swap cannot redirect this creation.
    pub(crate) fn create_asset_if_absent(&self, name: &str, bytes: &[u8]) -> io::Result<bool> {
        top_level_asset_name(name)?;
        let path = self.assets_path().join(name);
        self.ensure_asset_write_target(&path)?;
        fs::create_dir_all(self.assets_path())?;
        match atomic_write_new(&path, bytes) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Whether BOTH a `.md` and a `.org` file exist for the same logical page —
    /// an ambiguous identity, since Tine keys pages by `(kind, name)`. Writes
    /// (save/rename/delete) are refused on such a page so a save can't serve one
    /// twin's content with the other's baseline and clobber the wrong file. This
    /// is an interim guard; the full fix is path/format in page identity (#21).
    /// `.org` is probed first so a markdown-only graph short-circuits after one
    /// stat. A journal whose name doesn't parse to a date stem isn't guarded.
    fn has_twin(&self, name: &str, kind: PageKind) -> bool {
        let (dir, stem) = match kind {
            PageKind::Page => (
                self.pages_path(),
                Some(encode_page_name(name, self.config.file_name_format)),
            ),
            PageKind::Journal => (
                self.journals_path(),
                self.journal_format
                    .parse(name)
                    .map(|d| self.journal_format.file_stem(d)),
            ),
        };
        match stem {
            Some(s) => {
                dir.join(format!("{s}.org")).exists() && dir.join(format!("{s}.md")).exists()
            }
            None => false,
        }
    }

    /// Find a page/journal entry by display name. When several files share the
    /// name — a duplicate journal day, a canonical `2026_06_26.org` plus a
    /// title-named stray `Friday, 26-06-2026.org` (#21) — prefer the canonical
    /// date-stem file, so opening the day by name (a `[[link]]`, quick-switch, or
    /// `get_page`) is deterministic AND lands on the same file a save resolves to
    /// (`path_for`), instead of whichever the directory listing happened to yield
    /// first (which could mismatch the save target and raise a phantom conflict).
    /// The stray is reached by path via `load_by_path`.
    pub fn find_entry(&self, name: &str, kind: PageKind) -> Option<PageEntry> {
        let key = (kind, crate::refs::page_key(name));
        loop {
            let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            if let Some((g, index)) = self.find_entry_cache.read().unwrap().as_ref() {
                if *g == gen && index.has_kind(kind) {
                    return index.entries.get(&key).cloned();
                }
            }

            let dir = match kind {
                PageKind::Journal => self.journals_path(),
                PageKind::Page => self.pages_path(),
            };
            let rel_dir = match kind {
                PageKind::Journal => &self.config.journals_dir,
                PageKind::Page => &self.config.pages_dir,
            };
            let mut built = FindEntryIndex::new();
            for entry in list_md(
                &dir,
                kind,
                &self.journal_format,
                self.config.file_name_format,
                rel_dir,
            ) {
                let entry_key = (kind, crate::refs::page_key(&entry.name));
                match built.entries.get_mut(&entry_key) {
                    Some(winner) => {
                        if !is_date_stem_entry(winner) && is_date_stem_entry(&entry) {
                            *winner = entry;
                        }
                    }
                    None => {
                        built.entries.insert(entry_key, entry);
                    }
                }
            }
            built.mark_kind_loaded(kind);

            let found = {
                let mut guard = self.find_entry_cache.write().unwrap();
                match guard.as_mut() {
                    Some((g, index)) if *g == gen => {
                        if !index.has_kind(kind) {
                            index.entries.extend(built.entries);
                            index.mark_kind_loaded(kind);
                        }
                        index.entries.get(&key).cloned()
                    }
                    _ => {
                        let found = built.entries.get(&key).cloned();
                        *guard = Some((gen, built));
                        found
                    }
                }
            };
            if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == gen {
                return found;
            }
        }
    }

    /// Load a page by name; returns `None` if it doesn't exist on disk. Falls
    /// back to alias resolution (`alias::`) for named pages.
    pub fn load_named(&self, name: &str, kind: PageKind) -> io::Result<Option<PageDto>> {
        // A file that vanished between listing and load (external delete) reports
        // NotFound from load_page — map it to "no page" rather than an error, so
        // the page is treated as absent (never resurrected) and the get_page
        // contract (Ok(None) = doesn't exist) holds.
        let load = |entry: &PageEntry| -> io::Result<Option<PageDto>> {
            match self.load_page(entry) {
                Ok(dto) => Ok(Some(dto)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        };
        if let Some(entry) = self.find_entry(name, kind) {
            return load(&entry);
        }
        if kind == PageKind::Page {
            let tnorm = crate::refs::page_key(name);
            if let Some((_, canon)) = self
                .page_aliases()
                .into_iter()
                .find(|(alias, _)| crate::refs::page_key(alias) == tnorm)
            {
                if let Some(entry) = self.find_entry(&canon, kind) {
                    return load(&entry);
                }
            }
        }
        Ok(None)
    }

    /// The `icon::` property value of each named page that has one (for rendering
    /// page icons next to titles / in the namespace tree, like OG). Scans the
    /// cached pages once, then answers the requested names; only pages WITH an
    /// icon appear in the result. On-demand (e.g. a `{{namespace}}` macro), not at
    /// index time.
    pub fn page_icons(&self, names: &[String]) -> std::collections::HashMap<String, String> {
        let (mut icons_by_name, real_page_names) = self.with_pages(|pages| {
            let mut icons = std::collections::HashMap::new();
            let mut real = std::collections::HashSet::new();
            for (entry, doc) in pages {
                if entry.kind != PageKind::Page {
                    continue;
                }
                let key = crate::refs::page_key(&entry.name);
                real.insert(key.clone());
                if let Some(icon) = doc.pre_block.as_deref().and_then(pre_block_icon) {
                    icons.entry(key).or_insert(icon);
                }
            }
            (icons, real)
        });
        for (alias, canon) in self.page_aliases() {
            if real_page_names.contains(&alias) {
                continue; // `load_named` prefers a real page over an alias fallback.
            }
            if let Some(icon) = icons_by_name.get(&crate::refs::page_key(&canon)).cloned() {
                icons_by_name.entry(alias).or_insert(icon);
            }
        }
        let mut out = std::collections::HashMap::new();
        for name in names {
            if let Some(icon) = icons_by_name.get(&crate::refs::page_key(name)) {
                out.insert(name.clone(), icon.clone());
            }
        }
        out
    }

    /// Alias → canonical-page-name pairs (for the UI to resolve links/navigation).
    pub fn page_aliases(&self) -> Vec<(String, String)> {
        self.page_aliases_with_owners()
            .into_iter()
            .map(|(alias, canonical, _)| (alias, canonical))
            .collect()
    }

    pub(crate) fn page_aliases_with_owners(&self) -> Vec<(String, String, String)> {
        if let Some(a) = self.alias_cache.read().unwrap().as_ref() {
            return a.clone();
        }
        let aliases = crate::query::page_aliases_with_owners(self);
        *self.alias_cache.write().unwrap() = Some(aliases.clone());
        aliases
    }

    /// The page that owns a block uuid / `id::`, via a `cache_gen`-keyed index, or
    /// `None` if unknown. A hint only — callers must verify (the index can lag a
    /// concurrent edit). O(graph) to (re)build once per cache change, then O(1).
    pub fn block_page_hint(&self, uuid: &str) -> Option<String> {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        if let Some((idx_gen, map)) = self.block_index.read().unwrap().as_ref() {
            if *idx_gen == gen {
                return map.get(uuid).cloned();
            }
        }
        fn walk_idx(
            blocks: &[DocBlock],
            name: &str,
            m: &mut std::collections::HashMap<String, String>,
        ) {
            for b in blocks {
                if !b.uuid.is_empty() {
                    m.entry(b.uuid.clone()).or_insert_with(|| name.to_string());
                }
                if let Some(id) = b.property("id") {
                    if !id.is_empty() {
                        m.entry(id).or_insert_with(|| name.to_string());
                    }
                }
                walk_idx(&b.children, name, m);
            }
        }
        let map = self.with_pages(|pages| {
            let mut m = std::collections::HashMap::new();
            for (entry, doc) in pages {
                walk_idx(&doc.roots, &entry.name, &mut m);
            }
            m
        });
        let result = map.get(uuid).cloned();
        *self.block_index.write().unwrap() = Some((gen, map));
        result
    }

    /// Resolve a bounded set of physical cached pages that could contain one of
    /// `names_norm`. An unusable/stale/incomplete index returns the complete
    /// snapshot, so callers preserve exact full-scan correctness.
    pub(crate) fn reference_candidate_pages(
        &self,
        names_norm: &[String],
        kind: ReferenceKind,
    ) -> ReferenceCandidatePages {
        // Force the ordinary single-flight cache build. `install_built` publishes
        // the matching reference index alongside the parsed snapshot.
        self.with_pages(|_| ());
        for _ in 0..2 {
            let (snapshot, generation) = {
                let guard = self.cache.read().unwrap();
                let snapshot = guard.as_ref().map(Arc::clone).unwrap();
                let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
                (snapshot, generation)
            };
            let full_page_count = snapshot.len();
            let selected_paths = {
                let guard = self.reference_candidate_index.read().unwrap();
                let Some(index) = guard.as_ref().filter(|index| {
                    index.complete
                        && index.generation == generation
                        && index.pages.len() == full_page_count
                }) else {
                    if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
                        continue;
                    }
                    return ReferenceCandidatePages {
                        pages: snapshot.iter().cloned().collect(),
                        indexed: false,
                        full_page_count,
                    };
                };
                match kind {
                    ReferenceKind::Explicit => {
                        let mut paths = std::collections::BTreeSet::new();
                        for name in names_norm {
                            if let Some(postings) = index.explicit.get(name) {
                                paths.extend(postings.iter().cloned());
                            }
                        }
                        Some(paths)
                    }
                    ReferenceKind::Plain => {
                        let mut paths = std::collections::BTreeSet::new();
                        let mut safe = true;
                        for (path, projection) in &index.pages {
                            let mut any_name = false;
                            for name in names_norm {
                                let Some(maybe) = projection.signature.may_contain_name(name)
                                else {
                                    safe = false;
                                    break;
                                };
                                any_name |= maybe;
                            }
                            if !safe {
                                break;
                            }
                            if any_name {
                                paths.insert(path.clone());
                            }
                        }
                        safe.then_some(paths)
                    }
                }
            };
            let Some(selected_paths) = selected_paths else {
                if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
                    continue;
                }
                return ReferenceCandidatePages {
                    pages: snapshot.iter().cloned().collect(),
                    indexed: false,
                    full_page_count,
                };
            };
            let selected = {
                let cache_index = self.cache_index.read().unwrap();
                let Some(cache_index) = cache_index.as_ref() else {
                    if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
                        continue;
                    }
                    return ReferenceCandidatePages {
                        pages: snapshot.iter().cloned().collect(),
                        indexed: false,
                        full_page_count,
                    };
                };
                let mut selected = Vec::with_capacity(selected_paths.len());
                let mut coherent = true;
                for path in selected_paths {
                    let Some(slot) = cache_index.by_path.get(&path).copied() else {
                        coherent = false;
                        break;
                    };
                    let Some(page) = snapshot.get(slot) else {
                        coherent = false;
                        break;
                    };
                    selected.push(page.clone());
                }
                coherent.then_some(selected)
            };
            let Some(selected) = selected else {
                if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
                    continue;
                }
                return ReferenceCandidatePages {
                    pages: snapshot.iter().cloned().collect(),
                    indexed: false,
                    full_page_count,
                };
            };
            if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation {
                return ReferenceCandidatePages {
                    pages: selected,
                    indexed: true,
                    full_page_count,
                };
            }
        }
        let pages = self.with_pages(|pages| pages.iter().cloned().collect::<Vec<_>>());
        ReferenceCandidatePages {
            full_page_count: pages.len(),
            pages,
            indexed: false,
        }
    }

    pub(crate) fn reference_real_page_names(&self) -> Option<crate::query::RealPageNames> {
        self.with_pages(|_| ());
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let guard = self.reference_candidate_index.read().unwrap();
        let index = guard.as_ref()?;
        if !index.complete || index.generation != generation {
            return None;
        }
        let names = index
            .real_pages
            .iter()
            .filter_map(|(key, owners)| {
                owners
                    .first_key_value()
                    .map(|(path, name)| (key.clone(), (path.clone(), name.clone())))
            })
            .collect();
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(names)
    }

    /// Candidate paths for rename after validating the index covers the exact
    /// page-list/collision snapshot rename already collected. `None` means the
    /// caller must retain its correct whole-list scan.
    fn reference_candidate_paths_for_entries(
        &self,
        names_norm: &[String],
        entries: &[PageEntry],
    ) -> Option<std::collections::BTreeSet<PathBuf>> {
        self.with_pages(|_| ());
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let paths = {
            let guard = self.reference_candidate_index.read().unwrap();
            let index = guard.as_ref()?;
            if !index.complete
                || index.generation != generation
                || index.pages.len() != entries.len()
                || entries
                    .iter()
                    .any(|entry| !index.pages.contains_key(&entry.path))
            {
                return None;
            }
            let mut paths = std::collections::BTreeSet::new();
            for name in names_norm {
                if let Some(postings) = index.explicit.get(name) {
                    paths.extend(postings.iter().cloned());
                }
            }
            paths
        };

        // Generation/path coherence proves only that the index matches the
        // cached snapshot. Before using it to skip physical files, also prove
        // every listed file still has the bytes that snapshot was parsed from.
        // This catches external edits that landed before watcher reconciliation;
        // failures or mismatches retain the existing full-scan transaction.
        // Clone the reconstructible revisions so disk I/O does not hold a graph
        // lock. No document is reparsed on this validation path.
        let disk_revs = self.disk_revs.read().unwrap().clone();
        if disk_revs.len() != entries.len() {
            return None;
        }
        for entry in entries {
            let expected = disk_revs.get(&entry.path)?;
            let current = fs::read_to_string(&entry.path).ok()?;
            if content_rev(&current) != *expected {
                return None;
            }
        }
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(paths)
    }

    /// `block uuid → # of distinct referrer blocks`, over the whole graph, via a
    /// `cache_gen`-keyed index whose generation is advanced across edits that do
    /// not change the edited page's block-reference projection. A referrer is a
    /// block whose text references the uuid (`((uuid))`, `[..](((uuid)))`, or
    /// `{{embed ((uuid))}}`); multiple refs from one block count once (OG semantics).
    /// O(graph) to build initially or after a reference-bearing edit, then O(1)
    /// reuse across ordinary edits.
    pub fn block_ref_counts(&self) -> Arc<std::collections::HashMap<String, usize>> {
        use std::sync::atomic::Ordering;
        loop {
            let gen = self.cache_gen.load(Ordering::Acquire);
            if let Some((idx_gen, map)) = self.block_ref_count_cache.read().unwrap().as_ref() {
                if *idx_gen == gen {
                    return Arc::clone(map);
                }
            }

            let map = self.with_pages(|pages| {
                let mut counts = std::collections::HashMap::new();
                for (_entry, doc) in pages {
                    for (id, count) in document_block_ref_counts(doc) {
                        *counts.entry(id).or_insert(0) += count;
                    }
                }
                counts
            });
            // A save can race the scan. Never publish its old snapshot under the
            // new generation; retry against the current cache instead.
            if self.cache_gen.load(Ordering::Acquire) != gen {
                continue;
            }
            let arc = Arc::new(map);
            let mut cache = self.block_ref_count_cache.write().unwrap();
            if self.cache_gen.load(Ordering::Acquire) == gen {
                *cache = Some((gen, Arc::clone(&arc)));
                return arc;
            }
        }
    }

    /// Locate a page in the parsed-doc cache by its resolved physical path.
    /// Callers must already hold either `cache.read()` or `cache.write()`; this
    /// function only touches the companion index, preserving the lock order
    /// cache -> cache_index.
    fn cached_page_index_for_path(
        &self,
        pages: &[(PageEntry, Arc<Document>)],
        path: &Path,
    ) -> Option<usize> {
        {
            let guard = self.cache_index.read().unwrap();
            if let Some(index) = guard.as_ref() {
                if let Some(&i) = index.by_path.get(path) {
                    if pages.get(i).is_some_and(|(e, _)| e.path == path) {
                        return Some(i);
                    }
                    // A mismatched slot means a previous mutation dropped/shifted
                    // entries without rebuilding. Rebuild below rather than
                    // serving whatever the stale slot now points at.
                } else {
                    return None;
                }
            }
        }

        let mut guard = self.cache_index.write().unwrap();
        let rebuild = match guard.as_ref() {
            Some(index) => index
                .by_path
                .get(path)
                .is_some_and(|&i| !pages.get(i).is_some_and(|(e, _)| e.path == path)),
            None => true,
        };
        if rebuild {
            #[cfg(test)]
            count_cache_linear_scan(pages.len());
            *guard = Some(build_page_cache_index(pages));
        }
        guard
            .as_ref()
            .and_then(|index| index.by_path.get(path).copied())
            .filter(|&i| pages.get(i).is_some_and(|(e, _)| e.path == path))
    }

    /// A page DTO from the cache ONLY if the cache is already built — never
    /// triggers a (synchronous, whole-graph) build. `None` on a cold cache or a
    /// page not yet cached, so latency-path callers can parse just one file.
    fn peek_cached_page(&self, entry: &PageEntry) -> Option<PageDto> {
        let guard = self.cache.read().unwrap();
        let pages = guard.as_ref()?;
        let i = self.cached_page_index_for_path(pages, &entry.path)?;
        pages.get(i).map(|(e, d)| page_dto(e, d))
    }

    /// Load a page by entry. Served from the in-memory cache so block uuids are
    /// stable and consistent with queries / refs / the sidebar. Falls back to a
    /// disk parse for a page not yet in the cache (e.g. just created externally).
    pub fn load_page(&self, entry: &PageEntry) -> io::Result<PageDto> {
        // Reconcile any external change into the cache FIRST. Otherwise a stale
        // cache (an edit the 3s watcher hasn't folded in yet) would be served as
        // the editor's content while the rev below reflects the NEW disk bytes —
        // and the editor's save would then clobber the external edit with the rev
        // matching. sync_file is a no-op when the cache already matches disk.
        // Read the file ONCE: reconcile the cache against it, derive the save
        // baseline (rev) from the SAME bytes (so rev and the served content can't
        // disagree via a write landing between two reads), and — on a cache miss —
        // parse it below.
        let read = fs::read_to_string(&entry.path);
        if let Ok(content) = &read {
            self.sync_file_content(&entry.path, content, false);
        } else if read
            .as_ref()
            .err()
            .is_some_and(|e| e.kind() == io::ErrorKind::NotFound)
        {
            // The file is gone (external delete) but may still sit in the warm
            // cache. Serving that cached copy below — with rev = None — would make
            // it a null-baseline page, so a later edit + save would treat it as
            // brand-new and silently RESURRECT the externally-deleted file. Evict
            // the stale entry and report NotFound; callers treat the page as
            // absent (the feed skips it, get_page returns None).
            self.forget_file(&entry.path);
            return Err(read.unwrap_err());
        }
        let rev = read.as_ref().ok().map(|s| content_rev(s));
        // Serve from the cache if it's ALREADY built, but never trigger a build
        // here: a cold-cache `with_pages` would synchronously parse the entire
        // graph just to return one page, making first paint scale with graph size
        // (and defeating the background warm). On a cold cache, parse only this
        // file; `warm_cache_async` builds the rest. (Non-ref blocks then get fresh
        // uuids that may differ from the warm cache until the page is reloaded —
        // benign: id:: ref targets are stable, and live-ref views fall back to a
        // read-only render for an unmatched uuid, never losing edits.)
        if let Some(mut dto) = self.peek_cached_page(entry) {
            if let Ok(c) = &read {
                dto.read_only = read_only_org(&entry.path, c);
            }
            dto.rev = rev;
            dto.path = self.rel_path(&entry.path);
            return Ok(dto);
        }
        // Cache miss: parse the bytes we already read (propagate the original read
        // error if it failed).
        let content = read?;
        let mut doc = parse_doc(&entry.path, &content);
        assign_doc_runtime_ids(&mut doc.roots, &entry.rel_path);
        let mut dto = page_dto(entry, &doc);
        dto.read_only = read_only_org(&entry.path, &content);
        dto.rev = rev;
        dto.path = self.rel_path(&entry.path);
        Ok(dto)
    }

    /// Load a page from a SPECIFIC file by its graph-root-relative path, parsing it
    /// directly and bypassing the `(kind,name)` page cache + `disk_revs`. This is
    /// how a duplicate-day stray (`journals/Friday, 26-06-2026.org`) — which shares
    /// a `(kind,name)` with the canonical `2026_06_26.org` and so is unreachable by
    /// name — gets opened and edited (#21). The direct parse is deliberate: the
    /// cache slot for that `(kind,name)` holds the CANONICAL file, so a cache lookup
    /// here would serve the wrong file's content. Returns `Ok(None)` if the path is
    /// invalid (see [`resolve_rel`]) or the file is gone.
    pub fn load_by_path(&self, rel: &str) -> io::Result<Option<PageDto>> {
        let Some(abs) = self.resolve_rel(rel) else {
            return Ok(None);
        };
        let Some(entry) = self.entry_for_path(&abs) else {
            return Ok(None);
        };
        let content = match fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut doc = parse_doc(&abs, &content);
        assign_doc_runtime_ids(&mut doc.roots, &entry.rel_path);
        let mut dto = page_dto(&entry, &doc);
        dto.read_only = read_only_org(&abs, &content);
        dto.rev = Some(content_rev(&content));
        dto.path = self.rel_path(&abs);
        Ok(Some(dto))
    }

    /// Read and parse a page file into a [`Document`].
    pub fn read_document(&self, entry: &PageEntry) -> io::Result<Document> {
        let content = fs::read_to_string(&entry.path)?;
        let mut doc = parse_doc(&entry.path, &content);
        assign_doc_runtime_ids(&mut doc.roots, &entry.rel_path);
        Ok(doc)
    }

    /// Read+parse every page from disk (skipping unreadable files). Used to build
    /// the in-memory cache on first use — the on-demand `with_pages` build a user
    /// ACTIVELY WAITS ON when they navigate before the background warm finishes
    /// (NOT the paced thermal `warm_cache`, which keeps its own serial loop).
    ///
    /// The per-file work (read → content_rev → parse → assign uuids) is independent,
    /// so on a large graph we fan it across cores. Result order is irrelevant: the
    /// cache is searched by `(kind, name)`, never by position.
    fn load_all_pages(&self) -> PageCacheBuild {
        let entries = self.list_pages();
        let entry_count = entries.len();
        let workers = page_cache_worker_count();
        // Small graphs (or a single core): serial — the parse is fast and thread
        // spawn isn't worth it. Big graphs: split across `workers` threads.
        if workers <= 1 || entries.len() < 64 {
            let mut built = PageCacheBuild::with_capacity(entries.len());
            for entry in entries {
                built.collect(parse_page_entry_isolated(entry));
            }
            return built;
        }
        let per = (entries.len() + workers - 1) / workers;
        // Drain into owned contiguous chunks (no clone of PageEntry).
        let mut chunks: Vec<Vec<PageEntry>> = Vec::with_capacity(workers);
        let mut it = entries.into_iter();
        loop {
            let chunk: Vec<PageEntry> = it.by_ref().take(per).collect();
            if chunk.is_empty() {
                break;
            }
            chunks.push(chunk);
        }
        std::thread::scope(|s| {
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    s.spawn(move || {
                        let mut built = PageCacheBuild::with_capacity(chunk.len());
                        for entry in chunk {
                            built.collect(parse_page_entry_isolated(entry));
                        }
                        built
                    })
                })
                .collect();
            let mut built = PageCacheBuild::with_capacity(entry_count);
            for (worker, handle) in handles.into_iter().enumerate() {
                match handle.join() {
                    Ok(shard) => built.append(shard),
                    Err(_) => eprintln!(
                        "Tine search index worker {worker} panicked after per-page isolation; its shard was not indexed"
                    ),
                }
            }
            built
        })
    }

    /// Install a freshly-built whole-graph snapshot atomically: the parsed pages
    /// into the cache, their on-disk revs into `disk_revs`. Cache set BEFORE
    /// disk_revs so a reader never observes a fresh rev paired with a stale cache.
    fn install_built(&self, built: PageCacheBuild) {
        let PageCacheBuild {
            pages: built,
            failures,
        } = built;
        let revs: std::collections::HashMap<PathBuf, String> = built
            .iter()
            .map(|(e, _, r)| (e.path.clone(), r.clone()))
            .collect();
        let pages: Vec<(PageEntry, Arc<Document>)> = built
            .into_iter()
            .map(|(e, d, _)| (e, Arc::new(d)))
            .collect();
        let index = build_page_cache_index(&pages);
        // Publish cache + revs atomically under the cache lock (cache → disk_revs
        // order), so no reader observes a fresh rev paired with a stale cache.
        let mut guard = self.cache.write().unwrap();
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let reference_index = ReferenceCandidateIndex::build(generation, &pages);
        *guard = Some(Arc::new(pages));
        *self.page_index_failures.write().unwrap() = failures;
        *self.cache_index.write().unwrap() = Some(index);
        *self.reference_candidate_index.write().unwrap() = Some(reference_index);
        *self.disk_revs.write().unwrap() = revs;
        drop(guard);
    }

    /// Run `f` over every parsed page, building the cache on first use.
    ///
    /// `f` scans a consistent snapshot: a concurrent save/delete may or may not be
    /// visible depending on whether it published before this method cloned the
    /// snapshot Arc, but the scan never sees torn or partially-mutated cache
    /// contents. The cache read lock is held only while cloning the Arc; mutations
    /// use copy-on-write under `cache.write()` when a scan still holds an older
    /// snapshot.
    pub fn with_pages<T>(&self, f: impl FnOnce(&[(PageEntry, Arc<Document>)]) -> T) -> T {
        let snapshot = {
            let guard = self.cache.read().unwrap();
            guard.as_ref().map(Arc::clone)
        };
        if let Some(snapshot) = snapshot {
            return f(snapshot.as_slice());
        }
        // Single-flight build: serialize builders on `build_lock` (NOT the cache
        // lock) so the whole-graph parse happens once, not once per racing caller,
        // and so we never hold the cache write lock during the slow parse.
        use std::sync::atomic::Ordering;
        let _bl = self.build_lock.lock().unwrap();
        if self.cache.read().unwrap().is_none() {
            loop {
                let gen0 = self.cache_gen.load(Ordering::Acquire);
                let built = self.load_all_pages();
                // If a save/remove raced our read (its cache mutation no-op'd
                // because the cache was still None), its disk write is already
                // done — rebuild so we don't install a stale snapshot. We hold
                // build_lock, so no other builder competes.
                if self.cache_gen.load(Ordering::Acquire) == gen0 {
                    self.install_built(built);
                    break;
                }
            }
        }
        drop(_bl);
        let snapshot = {
            let guard = self.cache.read().unwrap();
            guard.as_ref().map(Arc::clone).unwrap()
        };
        f(snapshot.as_slice())
    }

    /// Eagerly build the page cache plus graph-open derived maps (call once after
    /// opening, off the hot path).
    pub fn warm_cache(&self) {
        let _ = self.warm_cache_cancellable(|| false);
    }

    /// Build graph-open caches while allowing a revoked window binding to stop
    /// between files and derived-map phases. Returns false when cancelled.
    pub fn warm_cache_cancellable(&self, cancelled: impl Fn() -> bool) -> bool {
        if !self.warm_page_cache_cancellable(&cancelled) || cancelled() {
            return false;
        }
        // Warm the derived maps the frontend fetches right after `warm-cache-done`
        // (aliases + block-ref counts), so those fetches are pure cache hits.
        let _ = self.page_aliases();
        if cancelled() {
            return false;
        }
        let _ = self.block_ref_counts();
        !cancelled()
    }

    fn warm_page_cache_cancellable(&self, cancelled: &impl Fn() -> bool) -> bool {
        use std::sync::atomic::Ordering;
        if cancelled() {
            return false;
        }
        if self.cache.read().unwrap().is_some() {
            return true; // already built (e.g. by a query) — nothing to warm
        }
        // Build PACED and WITHOUT holding build_lock during the parse: on a
        // thermally throttled laptop the warm would otherwise peg a core in one
        // burst right after launch, competing with first scrolling/typing/the
        // first agenda query. Parse into a LOCAL vec in small chunks with a brief
        // yield between them, so the load is spread out and an on-demand
        // `with_pages` (a user query) can still take build_lock and build fast
        // without waiting on our sleeps. If it wins, we discard our work.
        let gen0 = self.cache_gen.load(Ordering::Acquire);
        let entries = self.list_pages();
        let mut built = PageCacheBuild::with_capacity(entries.len());
        // Record each file's mtime BEFORE reading it, so a re-stat before install
        // catches any external edit that landed during the paced parse (external
        // writers don't bump cache_gen, so the gen check below can't see them).
        let mut mtimes: Vec<(PathBuf, Option<std::time::SystemTime>)> =
            Vec::with_capacity(entries.len());
        for (i, e) in entries.into_iter().enumerate() {
            if cancelled() {
                return false;
            }
            let mtime = fs::metadata(&e.path).and_then(|m| m.modified()).ok();
            if let Ok(content) = fs::read_to_string(&e.path) {
                let path = e.path.clone();
                let indexed = built.collect(isolate_page_parse(e, |entry| {
                    Some(parse_page_content(entry, &content))
                }));
                if indexed {
                    mtimes.push((path, mtime));
                }
            }
            if i % 24 == 23 {
                if self.cache.read().unwrap().is_some() {
                    return true; // a query built the cache while we parsed
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        // If any built file changed during the paced parse, our snapshot may be
        // stale and the watcher might not yet baseline-track it — discard and let
        // the next on-demand build read fresh. (A false positive just rebuilds.)
        if mtimes
            .iter()
            .any(|(p, m)| fs::metadata(p).and_then(|md| md.modified()).ok() != *m)
        {
            return false;
        }
        if cancelled() {
            return false;
        }
        // Install only if nobody else built it and no Tine save/remove raced our
        // reads (its cache mutation would have no-op'd against the None cache, so
        // its disk write must be folded in by a rebuild — defer to the next
        // on-demand build rather than install a stale snapshot).
        let _bl = self.build_lock.lock().unwrap();
        if self.cache.read().unwrap().is_none() && self.cache_gen.load(Ordering::Acquire) == gen0 {
            self.install_built(built);
        }
        !cancelled()
    }

    /// Discard the cache; it rebuilds on the next whole-graph query. Use when an
    /// external change may have touched many files.
    pub fn invalidate_cache(&self) {
        let mut guard = self.cache.write().unwrap();
        *guard = None;
        self.page_index_failures.write().unwrap().clear();
        *self.cache_index.write().unwrap() = None;
        *self.reference_candidate_index.write().unwrap() = None;
        self.disk_revs.write().unwrap().clear(); // under the cache lock (cache → disk_revs)
                                                 // Bump the generation AFTER discarding the cache (under the cache lock), so
                                                 // a reader that loads the new gen then reads the cache sees None (and
                                                 // rebuilds from disk) rather than the stale pre-invalidation content — same
                                                 // gen-after-content ordering as cache_upsert. The gen-keyed block index
                                                 // then rebuilds against fresh content too.
        self.cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        drop(guard);
        *self.alias_cache.write().unwrap() = None;
        *self.block_index.write().unwrap() = None;
        *self.block_ref_count_cache.write().unwrap() = None;
        *self.advanced_cache.write().unwrap() = None;
    }

    /// Update one page in the cache after we write it (no full rebuild). A no-op
    /// if the cache hasn't been built yet. `disk_rev` is `content_rev` of the
    /// exact on-disk bytes `doc` was produced from (the freshness key — see
    /// `disk_revs`).
    fn cache_upsert(&self, entry: PageEntry, mut doc: Document, disk_rev: String) {
        // Fill runtime ids for any block that lacks one (e.g. PDF-highlight writes)
        // from this physical owner. Blocks saved from the frontend already carry
        // live ids, which are deliberately kept through the in-memory save path.
        assign_doc_runtime_ids(&mut doc.roots, &entry.rel_path);
        // Only the alias map needs dropping when an `alias::` was added/changed/
        // removed — invalidating on every save would make a normal edit an O(P)
        // alias rescan on the next navigation.
        let new_aliases = crate::query::document_aliases(&doc);
        let new_block_refs = document_block_ref_counts(&doc);
        let mut block_refs_touched = false;
        let mut alias_touched = !new_aliases.is_empty();
        let path_key = entry.path.clone();
        let doc = Arc::new(doc);
        // Keep the new content + identity for the scoped derived-cache pass below
        // (the original is moved into the cache slot; this clone is a refcount bump).
        let evict_doc = Arc::clone(&doc);
        let evict_entry = entry.clone();
        let mut previous_doc: Option<Arc<Document>> = None;
        let mut is_new_page = false;
        let mut guard = self.cache.write().unwrap();
        let cache_built = guard.is_some();
        if let Some(pages) = guard.as_mut() {
            let pages = Arc::make_mut(pages);
            match self.cached_page_index_for_path(pages, &entry.path) {
                Some(i) => {
                    let slot = &mut pages[i];
                    alias_touched = new_aliases != crate::query::document_aliases(&slot.1);
                    block_refs_touched = document_block_ref_counts(&slot.1) != new_block_refs;
                    previous_doc = Some(Arc::clone(&slot.1));
                    slot.1 = doc;
                }
                None => {
                    is_new_page = true;
                    block_refs_touched = !new_block_refs.is_empty();
                    let name_key = page_cache_key(entry.kind, &entry.name);
                    pages.push((entry, doc));
                    if let Some(index) = self.cache_index.write().unwrap().as_mut() {
                        let slot = pages.len() - 1;
                        index.by_path.insert(path_key.clone(), slot);
                        // Exact-path additions must never repoint the stable
                        // logical duplicate winner.
                        index.by_name.entry(name_key).or_insert(slot);
                    }
                }
            }
            // Update disk_revs WHILE STILL HOLDING the cache write lock, so the
            // cached doc and its freshness rev are published atomically and can
            // never diverge across concurrent same-page writers (e.g. an editor
            // save racing a PDF write_highlights on an hls__ page). If they could
            // diverge, the sync_file_content fast-path could match disk against a
            // rev that isn't the cached doc's and serve a stale doc. Lock order is
            // always cache → disk_revs; readers never hold disk_revs while taking
            // the cache lock, so this nesting can't deadlock. Sets only when the
            // page is actually cached (preserves "entry exists IFF cached").
            self.disk_revs.write().unwrap().insert(path_key, disk_rev);
        }
        // Bump cache_gen AFTER publishing the new content (and disk_revs), still
        // under the cache write lock. A reader loads cache_gen (Acquire) then takes
        // the cache read lock; because the bump (Release) happens-after the slot
        // write and before the lock is dropped, observing the new gen guarantees
        // the new doc is visible. So any derived result computed at gen G reflects
        // every edit whose gen is <= G — it can never be a stale whole-graph scan
        // that reads the OLD doc yet gets tagged (and served) at the fresh gen.
        // (Bumping FIRST left a window where the gen was new but the doc still old.)
        // The bump is unconditional — even on a cold cache (no slot to update) — so
        // a concurrent lock-free with_pages build still detects the race and retries.
        let newgen = self
            .cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        if cache_built {
            let mut index_guard = self.reference_candidate_index.write().unwrap();
            match index_guard.as_mut() {
                Some(index) if index.complete && index.generation + 1 == newgen => {
                    index.insert(&evict_entry, &evict_doc);
                    index.generation = newgen;
                }
                _ => {
                    if let Some(pages) = guard.as_ref() {
                        *index_guard = Some(ReferenceCandidateIndex::build(newgen, pages));
                    }
                }
            }
        }
        drop(guard);
        {
            let mut counts = self.block_ref_count_cache.write().unwrap();
            if block_refs_touched {
                *counts = None;
            } else if let Some((generation, _)) = counts.as_mut() {
                *generation = newgen;
            }
        }
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
        // Scoped query/backlink invalidation (#52): a content edit to one page
        // can't change a derived result the page doesn't participate in, so keep
        // those (advancing their generation) and recompute only the entries this
        // page is in or now matches. An alias change, a new page, or a cold cache
        // has graph-wide effects → drop everything. Guarded by the differential
        // fuzz oracle in tests/derived_cache_fuzz.rs.
        let scoped = cache_built && !alias_touched && !is_new_page;
        self.scope_derived_invalidation(
            &evict_entry,
            previous_doc.as_deref(),
            &evict_doc,
            newgen,
            scoped,
        );
    }

    /// See `cache_upsert`. When `scoped`, evict only derived entries the edited
    /// page (`entry`, `doc`) participates in and re-tag the survivors to `newgen`;
    /// otherwise drop the whole derived cache.
    fn scope_derived_invalidation(
        &self,
        entry: &PageEntry,
        previous_doc: Option<&Document>,
        doc: &Document,
        newgen: u64,
        scoped: bool,
    ) {
        // Resolve aliases BEFORE taking the derived lock (page_aliases may take the
        // cache lock); never hold derived while taking cache.
        let (aliases, real_pages) = if scoped {
            (self.page_aliases(), crate::query::real_page_names(self))
        } else {
            (Vec::new(), crate::query::RealPageNames::new())
        };
        let today = crate::date::JournalDate::today().ordinal_key();
        // Hold the derived write lock across the WHOLE prune+re-tag. This is
        // deliberately atomic: the keep/evict test (page_affects_*) is re-evaluated
        // against whatever entry is CURRENTLY in the map, so a result a concurrent
        // query inserted (possibly from an older page-doc) is re-judged and evicted
        // if this edit affects it — never kept on pointer identity. Combined with
        // the gen-after-content bump (cache_upsert), an entry that survives the
        // prune is provably unaffected by this edit and consistent at `newgen`.
        // (An earlier version evaluated off the lock and kept entries by Arc
        // ptr_eq; that could bless a stale concurrent recompute — reverted.)
        let pname = &entry.name;
        {
            let mut g = self.derived_cache.write().unwrap();
            let Some(dc) = g.as_mut() else {
                drop(g);
                self.scope_advanced_invalidation(entry, previous_doc, doc, newgen, scoped, today);
                return;
            };
            if !scoped || dc.today != today {
                *g = None; // full invalidate (alias/page-set/cold-cache, or day rollover)
                drop(g);
                self.scope_advanced_invalidation(entry, previous_doc, doc, newgen, scoped, today);
                return;
            }
            let mut removed_bytes = 0usize;
            dc.results.retain(|key, (result, result_bytes)| {
                // Evict iff this page is already in the result OR matches the key's
                // predicate in either the old or new page; keep (still correct)
                // otherwise. Comparing both parsed documents makes omitted overflow
                // matches visible without retaining a graph-sized membership set.
                if result
                    .groups
                    .iter()
                    .any(|grp| crate::refs::same_page(&grp.page, pname))
                {
                    removed_bytes = removed_bytes.saturating_add(*result_bytes);
                    return false;
                }
                let page_affects = |candidate: &Document| match key.split_once('\0') {
                    Some(("b", target)) => crate::query::page_affects_backlinks(
                        &real_pages,
                        &aliases,
                        target,
                        entry,
                        candidate,
                    ),
                    Some(("u", target)) => crate::query::page_affects_unlinked(
                        &real_pages,
                        &aliases,
                        target,
                        entry,
                        candidate,
                    ),
                    Some(("br", uuid)) => {
                        crate::query::page_affects_block_referrers(uuid, candidate)
                    }
                    Some(("q", source)) => {
                        crate::query::page_affects_query(source, entry, candidate)
                    }
                    Some(("B", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|target| {
                        crate::query::page_affects_backlinks(
                            &real_pages,
                            &aliases,
                            target,
                            entry,
                            candidate,
                        )
                    }),
                    Some(("U", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|target| {
                        crate::query::page_affects_unlinked(
                            &real_pages,
                            &aliases,
                            target,
                            entry,
                            candidate,
                        )
                    }),
                    Some(("Q", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|source| {
                        crate::query::page_affects_query(source, entry, candidate)
                    }),
                    Some(("R", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|uuid| {
                        crate::query::page_affects_block_referrers(uuid, candidate)
                    }),
                    _ => true, // unknown key shape → evict to stay safe
                };
                let affects = page_affects(doc) || previous_doc.is_some_and(&page_affects);
                if affects {
                    removed_bytes = removed_bytes.saturating_add(*result_bytes);
                }
                !affects
            });
            dc.bytes = dc.bytes.saturating_sub(removed_bytes);
            dc.lru.retain(|key| dc.results.contains_key(key));
            dc.gen = newgen; // survivors are valid for the post-bump generation
        }
        self.scope_advanced_invalidation(entry, previous_doc, doc, newgen, scoped, today);
    }

    fn scope_advanced_invalidation(
        &self,
        entry: &PageEntry,
        previous_doc: Option<&Document>,
        doc: &Document,
        newgen: u64,
        scoped: bool,
        today: i64,
    ) {
        let mut cache = self.advanced_cache.write().unwrap();
        let Some(advanced) = cache.as_mut() else {
            return;
        };
        if !scoped || advanced.today != today {
            *cache = None;
            return;
        }
        let mut removed_bytes = 0usize;
        advanced.results.retain(|key, (result, result_bytes)| {
            if result
                .result
                .groups
                .iter()
                .any(|group| crate::refs::same_page(&group.page, &entry.name))
            {
                removed_bytes = removed_bytes.saturating_add(*result_bytes);
                return false;
            }
            // Split only structural fields; the final query source is opaque and
            // may itself contain NUL bytes. Treating it as another delimiter used
            // to make warm invalidation evaluate a truncated query.
            let (page_key, query_src) = if let Some(rest) = key.strip_prefix("aq\0") {
                rest.split_once('\0')
                    .map_or((None, None), |(page, query)| (Some(page), Some(query)))
            } else if let Some(rest) = key.strip_prefix("AQ\0") {
                let mut parts = rest.splitn(4, '\0');
                let _max_rows = parts.next();
                let _max_bytes = parts.next();
                (parts.next(), parts.next())
            } else {
                (None, None)
            };
            let page_affects = |candidate: &Document| {
                page_key.zip(query_src).is_none_or(|(page_key, query_src)| {
                    let current_page = page_key.strip_prefix("p:");
                    crate::query::page_affects_advanced_query(
                        query_src,
                        current_page,
                        entry,
                        candidate,
                    )
                })
            };
            let affects = page_affects(doc) || previous_doc.is_some_and(page_affects);
            if affects {
                removed_bytes = removed_bytes.saturating_add(*result_bytes);
            }
            !affects
        });
        advanced.bytes = advanced.bytes.saturating_sub(removed_bytes);
        advanced
            .lru
            .retain(|key| advanced.results.contains_key(key));
        advanced.gen = newgen;
    }

    /// Drop one page from the cache after deleting its file.
    fn cache_remove(&self, name: &str, kind: PageKind) {
        // A page delete is a page-set change (affects namespaces, exists-by-ref,
        // every backlink/query) — drop the whole derived cache.
        *self.derived_cache.write().unwrap() = None;
        *self.advanced_cache.write().unwrap() = None;
        let mut guard = self.cache.write().unwrap();
        let mut alias_touched = false;
        let mut block_refs_touched = false;
        if let Some(pages) = guard.as_mut() {
            let pages = Arc::make_mut(pages);
            let removed_paths = pages
                .iter()
                .filter(|(e, _)| e.kind == kind && crate::refs::same_page(&e.name, name))
                .map(|(e, doc)| {
                    alias_touched |= !crate::query::document_aliases(doc).is_empty();
                    block_refs_touched |= !document_block_ref_counts(doc).is_empty();
                    e.path.clone()
                })
                .collect::<Vec<_>>();
            pages.retain(|(e, _)| !(e.kind == kind && crate::refs::same_page(&e.name, name)));
            // Drop all exact revisions removed by this ambiguity-validated logical
            // delete under the cache lock (same cache → disk_revs order as
            // cache_upsert) so the two never diverge.
            let mut revs = self.disk_revs.write().unwrap();
            for path in removed_paths {
                revs.remove(&path);
            }
        }
        *self.cache_index.write().unwrap() =
            guard.as_ref().map(|pages| build_page_cache_index(pages));
        // Bump AFTER the removal is published (under the cache lock), so a reader
        // that loads the new gen is guaranteed to see the page gone — see the
        // gen-after-content note in cache_upsert.
        let newgen = self
            .cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        if let Some(pages) = guard.as_ref() {
            *self.reference_candidate_index.write().unwrap() =
                Some(ReferenceCandidateIndex::build(newgen, pages));
        } else {
            *self.reference_candidate_index.write().unwrap() = None;
        }
        drop(guard);
        {
            let mut counts = self.block_ref_count_cache.write().unwrap();
            if block_refs_touched {
                *counts = None;
            } else if let Some((generation, _)) = counts.as_mut() {
                *generation = newgen;
            }
        }
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
    }

    /// Drop one physical page from the cache after its file disappears. Unlike
    /// `cache_remove`, this preserves same-name siblings and rebuilds the logical
    /// first-wins index from the surviving entries.
    fn cache_remove_path(&self, entry: &PageEntry) {
        // A page delete is a page-set change (affects namespaces, exists-by-ref,
        // every backlink/query) — drop the whole derived cache.
        *self.derived_cache.write().unwrap() = None;
        *self.advanced_cache.write().unwrap() = None;
        let mut guard = self.cache.write().unwrap();
        let mut alias_touched = false;
        let mut block_refs_touched = false;
        if let Some(pages) = guard.as_mut() {
            let pages = Arc::make_mut(pages);
            if let Some(i) = self.cached_page_index_for_path(pages, &entry.path) {
                alias_touched = !crate::query::document_aliases(&pages[i].1).is_empty();
                block_refs_touched = !document_block_ref_counts(&pages[i].1).is_empty();
                pages.remove(i);
                // Drop the rev under the cache lock (same cache → disk_revs order
                // as cache_upsert) so the two never diverge.
                self.disk_revs.write().unwrap().remove(&entry.path);
            }
            *self.cache_index.write().unwrap() = Some(build_page_cache_index(pages));
        }
        // Bump AFTER the removal is published (under the cache lock), so a reader
        // that loads the new gen is guaranteed to see the page gone — see the
        // gen-after-content note in cache_upsert.
        let newgen = self
            .cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        if let Some(pages) = guard.as_ref() {
            *self.reference_candidate_index.write().unwrap() =
                Some(ReferenceCandidateIndex::build(newgen, pages));
        } else {
            *self.reference_candidate_index.write().unwrap() = None;
        }
        drop(guard);
        {
            let mut counts = self.block_ref_count_cache.write().unwrap();
            if block_refs_touched {
                *counts = None;
            } else if let Some((generation, _)) = counts.as_mut() {
                *generation = newgen;
            }
        }
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
    }

    /// Memoize a derived whole-graph scan result, keyed by `(cache_gen, today)` +
    /// `key`. On a tag mismatch the whole cache is dropped, so a hit is always
    /// consistent with the current graph. `compute` runs with NO lock held (it
    /// takes the cache read lock itself), so it can't deadlock against `with_pages`.
    fn derived_memo_bounded(
        &self,
        key: String,
        compute: impl FnOnce() -> crate::query::BoundedGroups,
    ) -> BoundedRefGroups {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        let today = crate::date::JournalDate::today().ordinal_key();
        {
            let mut g = self.derived_cache.write().unwrap();
            if let Some(dc) = g.as_mut() {
                if dc.gen == gen && dc.today == today {
                    if let Some((r, _)) = dc.results.get(&key) {
                        let result = r.clone();
                        touch_lru(&mut dc.lru, &key);
                        return result;
                    }
                }
            }
        }
        let computed = compute();
        let result = BoundedRefGroups {
            groups: Arc::new(computed.groups),
            total: computed.total,
            exceeded: computed.exceeded,
        };
        let result_bytes = ref_groups_estimated_bytes(result.groups.as_slice())
            .saturating_add(result_cache_key_estimated_bytes(&key));
        if result_bytes > DERIVED_CACHE_MAX_ENTRY_BYTES {
            return result;
        }
        let mut g = self.derived_cache.write().unwrap();
        match g.as_mut() {
            Some(dc) if dc.gen == gen && dc.today == today => {
                if let Some((_, old_bytes)) = dc
                    .results
                    .insert(key.clone(), (result.clone(), result_bytes))
                {
                    dc.bytes = dc.bytes.saturating_sub(old_bytes);
                }
                dc.bytes = dc.bytes.saturating_add(result_bytes);
                touch_lru(&mut dc.lru, &key);
                prune_result_cache(&mut dc.results, &mut dc.lru, &mut dc.bytes);
            }
            _ => {
                let mut results = std::collections::HashMap::new();
                results.insert(key.clone(), (result.clone(), result_bytes));
                *g = Some(DerivedCache {
                    gen,
                    today,
                    results,
                    lru: std::collections::VecDeque::from([key]),
                    bytes: result_bytes,
                });
            }
        }
        result
    }

    fn derived_memo(
        &self,
        key: String,
        compute: impl FnOnce() -> Vec<RefGroup>,
    ) -> Arc<Vec<RefGroup>> {
        self.derived_memo_bounded(key, || {
            let groups = compute();
            let total = groups.iter().map(|group| group.blocks.len()).sum();
            crate::query::BoundedGroups {
                groups,
                total,
                exceeded: false,
            }
        })
        .groups
    }

    fn advanced_memo_bounded(
        &self,
        key: String,
        compute: impl FnOnce() -> (crate::query::AdvancedResult, bool, usize),
    ) -> CachedAdvancedResult {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        let today = crate::date::JournalDate::today().ordinal_key();
        {
            let mut g = self.advanced_cache.write().unwrap();
            if let Some(dc) = g.as_mut() {
                if dc.gen == gen && dc.today == today {
                    if let Some((r, _)) = dc.results.get(&key) {
                        let result = r.clone();
                        touch_lru(&mut dc.lru, &key);
                        return result;
                    }
                }
            }
        }
        let (computed, exceeded, total) = compute();
        let result = CachedAdvancedResult {
            result: Arc::new(computed),
            total,
            exceeded,
        };
        let result_bytes = ref_groups_estimated_bytes(&result.result.groups)
            .saturating_add(result.result.ran.iter().map(String::len).sum::<usize>())
            .saturating_add(result.result.ignored.iter().map(String::len).sum::<usize>())
            .saturating_add(result_cache_key_estimated_bytes(&key));
        if result_bytes > DERIVED_CACHE_MAX_ENTRY_BYTES {
            return result;
        }
        let mut g = self.advanced_cache.write().unwrap();
        match g.as_mut() {
            Some(dc) if dc.gen == gen && dc.today == today => {
                if let Some((_, old_bytes)) = dc
                    .results
                    .insert(key.clone(), (result.clone(), result_bytes))
                {
                    dc.bytes = dc.bytes.saturating_sub(old_bytes);
                }
                dc.bytes = dc.bytes.saturating_add(result_bytes);
                touch_lru(&mut dc.lru, &key);
                prune_result_cache(&mut dc.results, &mut dc.lru, &mut dc.bytes);
            }
            _ => {
                let mut results = std::collections::HashMap::new();
                results.insert(key.clone(), (result.clone(), result_bytes));
                *g = Some(AdvancedCache {
                    gen,
                    today,
                    results,
                    lru: std::collections::VecDeque::from([key]),
                    bytes: result_bytes,
                });
            }
        }
        result
    }

    fn advanced_memo(
        &self,
        key: String,
        compute: impl FnOnce() -> crate::query::AdvancedResult,
    ) -> Arc<crate::query::AdvancedResult> {
        self.advanced_memo_bounded(key, || {
            let result = compute();
            let total = result.groups.iter().map(|group| group.blocks.len()).sum();
            (result, false, total)
        })
        .result
    }

    fn run_advanced_query_cached(
        &self,
        query_src: &str,
        current_page: Option<&str>,
    ) -> Arc<crate::query::AdvancedResult> {
        if !crate::query::query_source_within_limit(query_src) {
            return Arc::new(crate::query::rejected_advanced_query("query-too-large"));
        }
        if !crate::query::query_nesting_within_limit(query_src) {
            return Arc::new(crate::query::rejected_advanced_query(
                "query-nesting-too-deep",
            ));
        }
        let page_key = current_page
            .map(|p| format!("p:{}", crate::refs::page_key(p)))
            .unwrap_or_else(|| "n:".to_string());
        self.advanced_memo(format!("aq\0{page_key}\0{query_src}"), || {
            crate::query::run_advanced_query(self, query_src, current_page)
        })
    }

    pub fn run_advanced_query_bounded_cached(
        &self,
        query_src: &str,
        current_page: Option<&str>,
        max_rows: usize,
        max_bytes: usize,
    ) -> (crate::query::AdvancedResult, bool, usize) {
        if !crate::query::query_source_within_limit(query_src) {
            return (
                crate::query::rejected_advanced_query("query-too-large"),
                false,
                0,
            );
        }
        if !crate::query::query_nesting_within_limit(query_src) {
            return (
                crate::query::rejected_advanced_query("query-nesting-too-deep"),
                false,
                0,
            );
        }
        let page_key = current_page
            .map(|page| format!("p:{}", crate::refs::page_key(page)))
            .unwrap_or_else(|| "n:".to_string());
        let cached = self.advanced_memo_bounded(
            format!("AQ\0{max_rows}\0{max_bytes}\0{page_key}\0{query_src}"),
            || {
                crate::query::run_advanced_query_bounded(
                    self,
                    query_src,
                    current_page,
                    max_rows,
                    max_bytes,
                )
            },
        );
        (
            cached.result.as_ref().clone(),
            cached.exceeded,
            cached.total,
        )
    }

    /// Backlinks for a page: blocks across the graph that reference it,
    /// grouped by source page. Delegates to the query module (memoized).
    pub fn backlinks(&self, target: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("b\0{}", crate::refs::normalize(target)), || {
            crate::query::backlinks(self, target)
        })
    }

    pub fn backlinks_bounded(
        &self,
        target: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        let normalized = crate::refs::normalize(target);
        self.derived_memo_bounded(format!("B\0{max_rows}\0{max_bytes}\0{normalized}"), || {
            crate::query::backlinks_bounded(self, target, max_rows, max_bytes)
        })
    }

    /// Block-level referrers for a block uuid: every block across the graph that
    /// references it, grouped by source page (memoized). Includes same-page
    /// referrers (see `query::block_referrers`).
    pub fn block_referrers(&self, uuid: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("br\0{}", uuid.trim()), || {
            crate::query::block_referrers(self, uuid)
        })
    }

    pub fn block_referrers_bounded(
        &self,
        uuid: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        let uuid = uuid.trim();
        self.derived_memo_bounded(format!("R\0{max_rows}\0{max_bytes}\0{uuid}"), || {
            crate::query::block_referrers_bounded(self, uuid, max_rows, max_bytes)
        })
    }

    /// Evaluate a `{{query ...}}` body over the graph (memoized).
    pub fn run_query(&self, query_src: &str) -> Arc<Vec<RefGroup>> {
        if !crate::query::query_source_within_limit(query_src)
            || !crate::query::query_nesting_within_limit(query_src)
        {
            return Arc::new(Vec::new());
        }
        self.derived_memo(format!("q\0{query_src}"), || {
            crate::query::run_query(self, query_src)
        })
    }

    pub fn run_query_bounded(
        &self,
        query_src: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        if !crate::query::query_source_within_limit(query_src)
            || !crate::query::query_nesting_within_limit(query_src)
        {
            return BoundedRefGroups {
                groups: Arc::new(Vec::new()),
                total: 0,
                exceeded: false,
            };
        }
        self.derived_memo_bounded(format!("Q\0{max_rows}\0{max_bytes}\0{query_src}"), || {
            crate::query::run_query_bounded(self, query_src, max_rows, max_bytes)
        })
    }

    /// Evaluate an advanced (datalog-subset) query, returning the matched groups
    /// plus which clauses ran vs were ignored. Memoized by query text, effective
    /// current page, cache generation, and today.
    pub fn run_advanced_query(
        &self,
        query_src: &str,
        current_page: Option<&str>,
    ) -> crate::query::AdvancedResult {
        self.run_advanced_query_cached(query_src, current_page)
            .as_ref()
            .clone()
    }

    /// Unlinked references: plain-text mentions of a page that aren't links
    /// (memoized).
    pub fn unlinked_refs(&self, target: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("u\0{}", crate::refs::normalize(target)), || {
            crate::query::unlinked_refs(self, target)
        })
    }

    pub fn unlinked_refs_bounded(
        &self,
        target: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        let normalized = crate::refs::normalize(target);
        self.derived_memo_bounded(format!("U\0{max_rows}\0{max_bytes}\0{normalized}"), || {
            crate::query::unlinked_refs_bounded(self, target, max_rows, max_bytes)
        })
    }

    /// Explicit, uncached target-scoped trace of the exact reference engine.
    /// Intended for local diagnostics; callers must anonymize before export.
    pub fn reference_diagnostics(&self, target: &str) -> ReferenceDiagnostics {
        crate::query::reference_diagnostics(self, target)
    }

    /// Export the whole graph to static HTML under `<root>/publish/`.
    pub fn publish_html(&self) -> io::Result<(String, usize)> {
        crate::publish::publish_graph(self)
    }

    /// Render a single page to a self-contained HTML document for print-to-PDF
    /// (assets inlined, no sidebar/scripts). `Ok(None)` if the page doesn't exist.
    pub fn page_print_html(
        &self,
        name: &str,
        opts: crate::publish::PrintOpts,
    ) -> io::Result<Option<String>> {
        crate::publish::page_print_html(self, name, opts)
    }

    /// Rename a page, OG-style. Moves its file to the new name and rewrites every
    /// reference across pages AND journals — inline `[[old]]`/`#old`, the page's
    /// OWN self/sibling refs, and bare `tags:: old` property refs — and CASCADES
    /// to the whole `old/*` namespace subtree (each `old/child` page moves to
    /// `new/child`, its refs rewritten), matching Logseq's `rename-namespace-pages!`.
    /// Journals can't be renamed (their name is their date). Transactional: locks
    /// every touched file, re-verifies each is unchanged since collection, commits,
    /// and rolls back every write on any failure. Aborts (no change) if a target
    /// name already exists or a touched file changed under us.
    pub fn rename_page(&self, old: &str, new: &str) -> io::Result<()> {
        self.rename_page_expected(old, new, None)
    }

    pub fn rename_page_expected(
        &self,
        old: &str,
        new: &str,
        expected_path: Option<&str>,
    ) -> io::Result<()> {
        let old = old.trim();
        let new = new.trim();
        if new.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty name"));
        }
        if old.is_empty() || crate::refs::same_page(old, new) {
            return Ok(()); // nothing to do (case-only rename is intentionally a no-op)
        }
        self.validate_page_mutation_target(old, PageKind::Page, expected_path)?;
        // M1: refuse to rename an ambiguous page (both .md and .org on disk) — which
        // twin moves, and which content is authoritative, is undecidable here.
        if self.has_twin(old, PageKind::Page) || self.has_twin(new, PageKind::Page) {
            return Err(twin_error(old));
        }
        let old_n = crate::refs::normalize(old);
        let ns_prefix = format!("{old_n}/");
        let skip = old.chars().count();
        let entries = self.list_pages();

        // Phase 0a — the rename SET: the page itself plus every file-backed
        // namespace descendant (`old/*`). Each contributes a file move and an
        // (old_name -> new_name) ref-rewrite pair applied graph-wide. We only match
        // the exact name or the `old/` prefix (never a bare substring), so renaming
        // `work` -> `work1` turns `work/log` into `work1/log`, not `work1/work1log`.
        let mut rename_pairs: Vec<(String, String)> = Vec::new();
        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut move_destinations: std::collections::HashSet<PathBuf> =
            std::collections::HashSet::new();
        let mut move_identities: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut primary_is_file = false;
        for entry in &entries {
            if entry.kind != PageKind::Page {
                continue; // journals aren't namespaced pages; their refs still get rewritten in 0b
            }
            let en = crate::refs::normalize(&entry.name);
            let is_primary = en == old_n;
            if !is_primary && !en.starts_with(&ns_prefix) {
                continue;
            }
            let new_name = if is_primary {
                new.to_string()
            } else {
                // replace the `old` prefix, preserving the descendant's own casing
                let suffix: String = entry.name.chars().skip(skip).collect();
                format!("{new}{suffix}")
            };
            // Keep the page's own format on rename (an .org page stays .org).
            let encoded_new = encode_page_name(&new_name, self.config.file_name_format);
            let entry_format = Format::from_path(&entry.path);
            let new_path = self
                .pages_path()
                .join(format!("{encoded_new}.{}", entry_format.ext()));
            let other_ext = if entry_format == Format::Org {
                "md"
            } else {
                "org"
            };
            let other_format_target = self.pages_path().join(format!("{encoded_new}.{other_ext}"));
            if entries.iter().any(|other| {
                other.kind == PageKind::Page
                    && other.path != entry.path
                    && crate::refs::same_page(&other.name, &new_name)
            }) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "target page identity already exists elsewhere in the graph",
                ));
            }
            if new_path != entry.path && new_path.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "target page exists",
                ));
            }
            if other_format_target != entry.path && other_format_target.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "target page exists in the other format",
                ));
            }
            // Recursive graph directories can contain two distinct files with
            // the same basename/page identity. Both would map to the same flat
            // rename destination; allowing the transaction to continue would let
            // the later atomic rename overwrite the earlier page and remove both
            // sources. Refuse the ambiguous rename before collecting any edits.
            if !move_destinations.insert(new_path.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "multiple pages map to the same rename target",
                ));
            }
            if !move_identities.insert(crate::refs::normalize(&new_name)) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "multiple pages map to the same logical rename target",
                ));
            }
            if is_primary {
                primary_is_file = true;
            }
            rename_pairs.push((entry.name.clone(), new_name));
            moves.push((entry.path.clone(), new_path));
        }
        // A page can exist only via references (no file of its own); still rewrite
        // refs to it.
        if !primary_is_file {
            rename_pairs.push((old.to_string(), new.to_string()));
        }

        // Phase 0b — compute every file edit (inline refs + bare `tags::`), across
        // pages AND journals. A moved page's OWN content is rewritten too (self /
        // sibling refs) and lands at its new path.
        struct Edit {
            src: PathBuf,
            dst: PathBuf,
            orig: String,
            new_content: String,
            base_rev: String,
            is_move: bool,
        }
        let move_dst: std::collections::HashMap<PathBuf, PathBuf> = moves.into_iter().collect();
        // The whole rename SET as a normalized(old) -> new map, so each graph file
        // is rewritten ONCE against every descendant in a single pass — not K passes
        // (one per `(old,new)` pair), which made a namespace rename O(graph_text * K)
        // and recomputed code ranges twice per pair per file (perf Codex#2).
        let rename_map: std::collections::HashMap<String, String> = rename_pairs
            .iter()
            .map(|(o, n)| (crate::refs::normalize(o), n.clone()))
            .collect();
        let candidate_names = rename_pairs
            .iter()
            .map(|(old, _)| crate::refs::page_key(old))
            .collect::<Vec<_>>();
        let candidate_paths = self
            .reference_candidate_paths_for_entries(&candidate_names, &entries)
            .map(|mut candidates| {
                // A moved page must be read and staged even when it has no refs.
                candidates.extend(move_dst.keys().cloned());
                candidates
            });
        let mut edits: Vec<Edit> = Vec::new();
        for entry in &entries {
            if candidate_paths
                .as_ref()
                .is_some_and(|paths| !paths.contains(&entry.path))
            {
                continue;
            }
            let Ok(content) = fs::read_to_string(&entry.path) else {
                continue;
            };
            let is_org = Format::from_path(&entry.path) == Format::Org;
            // One inline-ref pass + one `tags::` pass per file (each computes code
            // ranges once), regardless of how many descendants are being renamed.
            let updated = crate::refs::rename_tags_property_multi(
                &crate::refs::rename_refs_multi(&content, &rename_map, is_org),
                &rename_map,
                is_org,
            );
            // H1: a rename must never rewrite a read-only (non-round-tripping) .org
            // file. Abort the whole rename (all-or-nothing) so the user resolves it
            // in Logseq first. A pure file move with no content change (updated ==
            // content) is still allowed — it preserves bytes exactly.
            if is_org && updated != content && !crate::org::org_editable(&content) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "cannot rename: {} is a read-only .org file (does not round-trip)",
                        entry.path.display()
                    ),
                ));
            }
            match move_dst.get(&entry.path) {
                Some(dst) => edits.push(Edit {
                    src: entry.path.clone(),
                    dst: dst.clone(),
                    base_rev: content_rev(&content),
                    orig: content,
                    new_content: updated,
                    is_move: true,
                }),
                None if updated != content => edits.push(Edit {
                    src: entry.path.clone(),
                    dst: entry.path.clone(),
                    base_rev: content_rev(&content),
                    orig: content,
                    new_content: updated,
                    is_move: false,
                }),
                None => {}
            }
        }
        if edits.is_empty() {
            return Ok(()); // page doesn't exist / nothing references it
        }

        // Phase 1 — lock every touched path (src + move dst), sorted + deduped
        // (deadlock-free against a single-page save, which only ever holds ONE lock).
        let mut lock_paths: Vec<PathBuf> = Vec::new();
        for e in &edits {
            lock_paths.push(e.src.clone());
            if e.is_move {
                lock_paths.push(e.dst.clone());
            }
        }
        lock_paths.sort();
        lock_paths.dedup();
        let locks: Vec<_> = lock_paths.iter().map(|p| self.page_lock(p)).collect();
        let _guards: Vec<_> = locks.iter().map(|l| l.lock().unwrap()).collect();

        // Phase 2 — re-verify nothing changed under us since Phase 0; abort (no
        // change) on any mismatch (an external editor / Syncthing pull landed).
        for e in &edits {
            if e.is_move && e.dst != e.src && e.dst.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "target page exists",
                ));
            }
            // A hard read failure is not the same thing as an empty file. Treating
            // it as empty could let an actually-empty baseline pass verification,
            // after which the transaction would overwrite a file we could no
            // longer inspect.
            if content_rev(&fs::read_to_string(&e.src)?) != e.base_rev {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
            }
        }

        // Phase 2.5 — operation truth first. The whole rename set (moved page
        // paths plus every reference rewrite) is one Loro transaction/chunk.
        // If a later projection write fails, the durable operation remains
        // recoverable and the watcher can finish projecting it.
        {
            let mut sync_guard = self.managed_sync.lock().unwrap();
            if let Some(sync) = sync_guard.as_mut() {
                let mut snapshots = Vec::with_capacity(edits.len());
                for e in &edits {
                    let src_rel = self.rel_path(&e.src);
                    let dst_rel = self.rel_path(&e.dst);
                    let existing = sync
                        .materialize_page(src_rel.as_str())
                        .map_err(crdt_io_error)?;
                    let existing = match existing {
                        Some(page) => Some(page),
                        None => sync
                            .materialize_page(dst_rel.as_str())
                            .map_err(crdt_io_error)?,
                    };
                    let page_id = existing.map(|page| page.id).unwrap_or_else(CrdtPageId::new);
                    let entry = self.entry_for_path(&e.dst).ok_or_else(bad_path)?;
                    let mut doc = parse_doc(&e.dst, &e.new_content);
                    assign_doc_runtime_ids(&mut doc.roots, &dst_rel);
                    let mut page = page_dto(&entry, &doc);
                    page.path = dst_rel;
                    page.format = Format::from_path(&e.dst);
                    page = page_with_persisted_sync_ids(&page)?;
                    snapshots.push(crdt_snapshot_for_page(&page, page_id)?);
                }
                sync.commit_pages(snapshots).map_err(crdt_io_error)?;
            }
        }

        // Phase 3 — commit, tracking writes for rollback. Move sources are
        // atomically staged into recoverable trash rather than unlinked, so an
        // external replacement at the syscall boundary is preserved as an inode.
        let mut written: Vec<(&Edit, Option<PathBuf>)> = Vec::new();
        let result: io::Result<()> = (|| {
            for e in &edits {
                // Phase 2 can be far in the past for a large graph. Recheck this
                // exact file immediately before its write so an external editor or
                // sync pull that landed while earlier edits committed is preserved.
                if content_rev(&fs::read_to_string(&e.src)?) != e.base_rev {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
                self.note_self_write(&e.dst, content_rev(&e.new_content));
                if e.is_move && e.dst != e.src {
                    if let Some(parent) = e.dst.parent() {
                        fs::create_dir_all(parent)?;
                    }
                }
                if e.is_move && e.dst != e.src {
                    atomic_write_new(&e.dst, e.new_content.as_bytes())?;
                } else {
                    atomic_write(&e.dst, e.new_content.as_bytes())?;
                }
                written.push((e, None));
                if e.is_move && e.dst != e.src {
                    rename_source_remove_failpoint()?;
                    let trash = typed_trash_dir(&self.root, TrashEntryKind::Page);
                    self.ensure_write_target(&trash)?;
                    fs::create_dir_all(&trash)?;
                    let src_name = e.src.file_name().and_then(|s| s.to_str()).unwrap_or("page");
                    let staged = trash.join(format!("{}__rename__{src_name}", trash_stamp()));
                    move_file_noreplace(&e.src, &staged)?;
                    written.last_mut().unwrap().1 = Some(staged.clone());
                    // If a sync replacement won just before the atomic move, the
                    // staged bytes no longer match our baseline. Abort and restore
                    // that exact inode instead of completing from stale content.
                    if content_rev(&fs::read_to_string(&staged)?) != e.base_rev {
                        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                    }
                }
            }
            Ok(())
        })();
        if let Err(err) = result {
            // Roll back in reverse, and drop the self-write markers for bytes that
            // won't survive the rollback so they can't later suppress a real
            // external change (M1).
            for (e, staged_source) in written.iter().rev() {
                if e.is_move && e.dst != e.src {
                    let source_restored = match staged_source {
                        Some(staged) => {
                            move_file_noreplace(staged, &e.src).is_ok() || e.src.exists()
                        }
                        None => e.src.exists(),
                    };
                    if source_restored {
                        // Never compare and unlink the live destination. Detach
                        // whichever inode currently owns the name, inspect it in
                        // recovery, and restore/retain any external replacement.
                        let _ = self.withdraw_file_to_conflict_if_exact(
                            &e.dst,
                            e.new_content.as_bytes(),
                            "rename-rollback-destination",
                        );
                    }
                    self.recent_writes.lock().unwrap().remove(&e.dst);
                } else {
                    let ours = content_rev(&e.new_content);
                    if fs::read_to_string(&e.dst).is_ok_and(|disk| content_rev(&disk) == ours) {
                        self.note_self_write(&e.dst, content_rev(&e.orig));
                        let _ = atomic_write(&e.dst, e.orig.as_bytes());
                    } else {
                        self.recent_writes.lock().unwrap().remove(&e.dst);
                    }
                }
            }
            self.invalidate_cache();
            return Err(err);
        }
        self.invalidate_cache();
        for edit in &edits {
            self.record_managed_projection(&edit.dst);
        }
        Ok(())
    }

    /// Delete a page/journal file. Rather than unlinking, the file is moved to a
    /// graph-local trash (`logseq/.tine-trash/`, outside journals//pages/ so it's
    /// never re-loaded) — so a delete that races an unseen external edit, or a
    /// simple misclick, is recoverable. If the trash move fails, the live file is
    /// left in place and the error is returned.
    pub fn delete_page(&self, name: &str, kind: PageKind) -> io::Result<()> {
        self.delete_page_expected(name, kind, None)
    }

    pub fn delete_page_expected(
        &self,
        name: &str,
        kind: PageKind,
        expected_path: Option<&str>,
    ) -> io::Result<()> {
        // M1: with both a .md and a .org twin, "which file?" is ambiguous — refuse
        // rather than trash an arbitrary one.
        if self.has_twin(name, kind) {
            return Err(twin_error(name));
        }
        let matching: Vec<_> = self
            .list_pages()
            .into_iter()
            .filter(|entry| entry.kind == kind && crate::refs::same_page(&entry.name, name))
            .collect();
        if matching.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "multiple files share this page identity; delete by name is ambiguous",
            ));
        }
        self.validate_page_mutation_target(name, kind, expected_path)?;
        if let Some(entry) = matching.into_iter().next() {
            let lock = self.page_lock(&entry.path);
            let _guard = lock.lock().unwrap();
            self.commit_managed_delete(&entry.path)?;
            let trash = typed_trash_dir(
                &self.root,
                match entry.kind {
                    PageKind::Journal => TrashEntryKind::Journal,
                    PageKind::Page => TrashEntryKind::Page,
                },
            );
            self.ensure_write_target(&trash)?;
            let fname = entry
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("page.md");
            let dest = trash.join(format!("{}__{fname}", trash_stamp()));
            move_to_trash(&entry.path, &dest, &trash)?;
        }
        self.cache_remove(name, kind);
        Ok(())
    }

    /// Validate the snapshot captured by a page menu/title before any mutation.
    /// Even an exact path does not authorize choosing one logical duplicate: the
    /// semantics of rewriting `[[page]]` references remain ambiguous.
    fn validate_page_mutation_target(
        &self,
        name: &str,
        kind: PageKind,
        expected_path: Option<&str>,
    ) -> io::Result<()> {
        let matching: Vec<_> = self
            .list_pages()
            .into_iter()
            .filter(|entry| entry.kind == kind && crate::refs::same_page(&entry.name, name))
            .collect();
        if matching.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "multiple files share this page identity; mutation is ambiguous",
            ));
        }
        let Some(expected) = expected_path.filter(|path| !path.trim().is_empty()) else {
            return Ok(());
        };
        let expected_abs = self.resolve_rel(expected).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid expected page path")
        })?;
        let Some(entry) = matching.first() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "stale page target"));
        };
        if entry.path != expected_abs {
            return Err(io::Error::new(io::ErrorKind::NotFound, "stale page target"));
        }
        Ok(())
    }

    /// Full-text search across all blocks.
    pub fn search(&self, query: &str, limit: usize) -> Vec<RefGroup> {
        crate::query::search(self, query, limit)
    }

    /// Execute the typed, combined graph-search plan (page names + block text).
    /// Commands and page creation remain frontend providers and are deliberately
    /// outside this graph query result.
    pub fn run_graph_search(
        &self,
        source: &str,
        page_limit: usize,
        block_limit: usize,
        explain: bool,
    ) -> crate::query_plan::QueryExecution {
        self.run_graph_search_scoped(source, page_limit, block_limit, None, explain)
    }

    pub fn run_graph_search_scoped(
        &self,
        source: &str,
        page_limit: usize,
        block_limit: usize,
        scope: Option<crate::query_plan::QueryPageScope>,
        explain: bool,
    ) -> crate::query_plan::QueryExecution {
        match scope {
            Some(scope) => {
                crate::query_plan::QueryPlan::friendly_for_page(source, block_limit, scope)
            }
            None => crate::query_plan::QueryPlan::friendly(source, page_limit, block_limit),
        }
        .execute_with_explain(self, || false, explain)
    }

    /// Interactive search lane: a newer request in the same lane cooperatively
    /// cancels the older whole-graph scan. Separate lanes keep the Ctrl-K
    /// switcher and in-editor block picker from canceling one another.
    pub fn search_latest(&self, lane: &str, query: &str, limit: usize) -> Vec<RefGroup> {
        use std::sync::atomic::Ordering;
        let epoch = {
            let mut lanes = self.search_lanes.lock().unwrap();
            lanes
                .entry(lane.to_owned())
                .or_insert_with(|| Arc::new(std::sync::atomic::AtomicU64::new(0)))
                .clone()
        };
        let mine = epoch.fetch_add(1, Ordering::AcqRel) + 1;
        crate::query::search_cancellable(self, query, limit, || {
            epoch.load(Ordering::Acquire) != mine
        })
    }

    /// Latest-wins combined graph search.  It shares the same lane epochs as the
    /// legacy block-search adapter, so migrating a consumer cannot leave an older
    /// request from either API running in that logical lane.
    pub fn run_graph_search_latest(
        &self,
        lane: &str,
        source: &str,
        page_limit: usize,
        block_limit: usize,
        explain: bool,
    ) -> crate::query_plan::QueryExecution {
        self.run_graph_search_latest_scoped(lane, source, page_limit, block_limit, None, explain)
    }

    pub fn run_graph_search_latest_scoped(
        &self,
        lane: &str,
        source: &str,
        page_limit: usize,
        block_limit: usize,
        scope: Option<crate::query_plan::QueryPageScope>,
        explain: bool,
    ) -> crate::query_plan::QueryExecution {
        use std::sync::atomic::Ordering;
        let epoch = {
            let mut lanes = self.search_lanes.lock().unwrap();
            lanes
                .entry(lane.to_owned())
                .or_insert_with(|| Arc::new(std::sync::atomic::AtomicU64::new(0)))
                .clone()
        };
        let mine = epoch.fetch_add(1, Ordering::AcqRel) + 1;
        match scope {
            Some(scope) => {
                crate::query_plan::QueryPlan::friendly_for_page(source, block_limit, scope)
            }
            None => crate::query_plan::QueryPlan::friendly(source, page_limit, block_limit),
        }
        .execute_with_explain(self, || epoch.load(Ordering::Acquire) != mine, explain)
    }

    /// Fuzzy page-name matches for the quick switcher.
    pub fn quick_switch(&self, query: &str, limit: usize) -> Vec<PageEntry> {
        crate::query::quick_switch(self, query, limit)
    }

    /// All `template:: <name>` templates across the graph, with the blocks to
    /// insert (ids and template properties stripped).
    pub fn templates(&self) -> Vec<TemplateDto> {
        crate::query::templates(self)
    }

    /// Resolve a `((uuid))` block reference to its shallow identity row.
    pub fn resolve_block(&self, uuid: &str) -> Option<RefGroup> {
        crate::query::resolve_block(self, uuid)
    }

    /// Resolve many block references in one call (for a page full of `((uuid))`
    /// refs / embeds) — one IPC instead of N, and one graph pass instead of N:
    /// hinted ids are grouped + each hinted page scanned once, with a single
    /// whole-graph fallback for hint misses.
    pub fn resolve_blocks(&self, uuids: &[String]) -> Vec<Option<RefGroup>> {
        crate::query::resolve_blocks(self, uuids)
    }

    /// Resolve a bounded subtree for an explicitly expanded preview/export.
    pub fn preview_block(&self, uuid: &str, max_nodes: usize) -> Option<BlockPreview> {
        crate::query::preview_block(self, uuid, max_nodes)
    }

    pub fn preview_block_with_budget(
        &self,
        uuid: &str,
        max_nodes: usize,
        max_bytes: usize,
    ) -> Option<BlockPreview> {
        crate::query::preview_block_with_budget(self, uuid, max_nodes, max_bytes)
    }

    /// The graph's `logseq/custom.css`, if present (for user theming).
    pub fn custom_css(&self) -> String {
        std::fs::read_to_string(self.root.join("logseq").join("custom.css")).unwrap_or_default()
    }

    /// Property keys (with their distinct values) used across the graph, for the
    /// query builder's property-filter autocomplete. Excludes internal/metadata
    /// properties (id, collapsed, hl-*, …).
    pub fn property_facets(&self) -> Vec<(String, Vec<String>)> {
        crate::query::property_facets(self)
    }

    // ---- Assets & PDF highlights ----

    pub fn assets_path(&self) -> PathBuf {
        self.assets_root.clone()
    }

    /// Top-level `assets/` files that NO block references — orphans the user may
    /// want to trash. Tine never auto-deletes assets (a deleted block keeps its
    /// media as a safety net), so this is the discovery half of "find unused
    /// media". Conservative: scans every block's `raw` + page `pre_block` for any
    /// `assets/<name>` mention; skips subdirectories (PDF area-image stores) and
    /// `.edn`/dotfiles (sidecars, not media) so nothing in use is ever flagged.
    pub fn orphan_assets(&self) -> Vec<AssetInfo> {
        let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
        self.with_pages(|pages| {
            for (_e, doc) in pages {
                if let Some(pre) = &doc.pre_block {
                    collect_asset_refs(pre, &mut referenced);
                }
                for b in &doc.roots {
                    collect_block_asset_refs(b, &mut referenced);
                }
            }
        });
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir(self.assets_path()) else {
            return out;
        };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_file() {
                continue; // skip subdirs (PDF area-image stores, tied to a PDF)
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Sidecars/hidden files aren't user media; never flag them as orphans.
            if name.starts_with('.') || name.ends_with(".edn") {
                continue;
            }
            if referenced.contains(name) {
                continue;
            }
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            out.push(AssetInfo {
                name: name.to_string(),
                size,
                modified,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Move an asset file to `logseq/.tine-trash` (recoverable), never a hard
    /// delete by default. Refuses any name with a path separator (top-level
    /// assets only) so it can't reach outside `assets/`.
    pub fn trash_asset(&self, name: &str) -> io::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "bad asset name",
            ));
        }
        let src = self.assets_path().join(name);
        if !src.is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such asset"));
        }
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Asset);
        self.ensure_asset_write_target(&src)?;
        self.ensure_write_target(&trash)?;
        let dest = trash.join(format!("{}__{name}", trash_stamp()));
        move_to_trash(&src, &dest, &trash)?;
        Ok(())
    }

    /// File count + total bytes currently in the asset trash. Non-asset recovery
    /// entries are counted separately so an asset cleanup cannot silently sweep
    /// pages, journals, or sync-conflict copies.
    pub fn asset_trash_stats(&self) -> TrashStats {
        trash_stats(&trash_root(&self.root))
    }

    /// Permanently delete asset-type entries in the asset trash. Returns the
    /// number of entries removed. Page, journal, conflict, and unknown legacy
    /// entries stay recoverable in `logseq/.tine-trash`.
    pub fn empty_asset_trash(&self) -> io::Result<u64> {
        let trash = trash_root(&self.root);
        self.ensure_write_target(&trash)?;
        let mut removed = 0;
        match fs::read_dir(&trash) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    let Ok(ft) = entry.file_type() else { continue };
                    if ft.is_dir() {
                        if trash_dir_kind(&entry.path()) == Some(TrashEntryKind::Asset) {
                            for asset_entry in fs::read_dir(entry.path())?.flatten() {
                                let path = asset_entry.path();
                                let ok = match asset_entry.file_type() {
                                    Ok(ft) if ft.is_dir() => fs::remove_dir_all(&path).is_ok(),
                                    Ok(_) => fs::remove_file(&path).is_ok(),
                                    Err(_) => false,
                                };
                                if ok {
                                    removed += 1;
                                }
                            }
                        }
                    } else if classify_legacy_trash_entry(&entry.path(), ft)
                        == TrashEntryKind::Asset
                        && fs::remove_file(entry.path()).is_ok()
                    {
                        removed += 1;
                    }
                }
                Ok(removed)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Read raw bytes of an asset (e.g. a PDF) for the viewer.
    pub fn read_asset(&self, name: &str) -> io::Result<Vec<u8>> {
        fs::read(self.asset_file_for_read(name)?)
    }

    /// Resolve an existing top-level regular asset through the canonical asset
    /// capability. A symlink may point elsewhere inside that approved root, but
    /// can never turn a read/open into access outside it.
    pub fn asset_file_for_read(&self, name: &str) -> io::Result<PathBuf> {
        top_level_asset_name(name)?;
        let assets = fs::canonicalize(self.assets_path())?;
        let path = fs::canonicalize(self.assets_path().join(name))?;
        if !path.starts_with(&assets) || !fs::metadata(&path)?.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid asset"));
        }
        Ok(path)
    }

    /// Canonical, regular-file path for the native asset protocol. This is used
    /// for audio/video so WebView range requests read at most a small chunk
    /// instead of copying a multi-gigabyte file through Rust Vec → IPC → Blob.
    pub fn stream_asset_path(&self, name: &str) -> io::Result<PathBuf> {
        top_level_asset_name(name)?;
        let candidate = self.assets_path().join(name);
        if fs::symlink_metadata(&candidate)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "asset symlinks cannot be streamed",
            ));
        }
        self.asset_file_for_read(name)
    }

    /// Read an asset only if its current on-disk size is within `max_bytes`.
    /// The post-read check closes the metadata/read race if another process grows
    /// the file between those operations.
    pub fn read_asset_limited(&self, name: &str, max_bytes: u64) -> io::Result<Vec<u8>> {
        top_level_asset_name(name)?;
        let path = self.asset_file_for_read(name)?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "asset is not a regular file",
            ));
        }
        if metadata.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("asset exceeds {} byte limit", max_bytes),
            ));
        }
        let bytes = fs::read(path)?;
        if bytes.len() as u64 > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("asset exceeds {} byte limit", max_bytes),
            ));
        }
        Ok(bytes)
    }

    /// Write raw bytes (e.g. a pasted image) into `assets/`, returning the
    /// stored filename (de-duplicated if it already exists).
    pub fn save_asset(&self, name: &str, bytes: &[u8]) -> io::Result<String> {
        let assets = self.assets_path();
        self.ensure_asset_write_target(&assets)?;
        fs::create_dir_all(&assets)?;
        top_level_asset_name(name)?;
        let (stem, ext) = split_asset_stem_ext(name);
        for i in 0usize.. {
            let final_name = if i == 0 {
                name.to_string()
            } else {
                format!("{stem}_{i}{ext}")
            };
            match atomic_write_new(&assets.join(&final_name), bytes) {
                Ok(()) => return Ok(final_name),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Copy a file into `assets/`, returning the stored filename. De-duplicates
    /// against existing assets (never overwrites one already referenced by notes).
    pub fn import_asset(&self, src: &Path, name: Option<&str>) -> io::Result<String> {
        // Desired stored name (a timestamped name from the frontend), else the
        // source basename. `reserve_asset` still dedups same-name collisions.
        let name = match name {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => src
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad source filename"))?
                .to_string(),
        };
        let assets = self.assets_path();
        self.ensure_asset_write_target(&assets)?;
        fs::create_dir_all(&assets)?;
        top_level_asset_name(&name)?;
        let (stem, ext) = split_asset_stem_ext(&name);
        for i in 0usize.. {
            let final_name = if i == 0 {
                name.clone()
            } else {
                format!("{stem}_{i}{ext}")
            };
            match atomic_copy_new(src, &assets.join(&final_name)) {
                Ok(()) => return Ok(final_name),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Stream an already-open native capture into `assets/` without ever
    /// materializing it as a bridge/base64 value. The source handle is the
    /// capability validated by the native caller; collision retries rewind it.
    pub fn import_asset_file(
        &self,
        src: &mut fs::File,
        name: &str,
        max_bytes: u64,
    ) -> io::Result<String> {
        let assets = self.assets_path();
        self.ensure_asset_write_target(&assets)?;
        fs::create_dir_all(&assets)?;
        top_level_asset_name(name)?;
        let (stem, ext) = split_asset_stem_ext(name);
        for i in 0usize.. {
            let final_name = if i == 0 {
                name.to_string()
            } else {
                format!("{stem}_{i}{ext}")
            };
            src.seek(io::SeekFrom::Start(0))?;
            match atomic_copy_file_new(src, &assets.join(&final_name), max_bytes) {
                Ok(()) => return Ok(final_name),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    /// Write a cropped area-highlight image to OG's file-graph layout:
    /// `assets/<key>/<page>_<id>_<stamp>.png` (`<stamp>` = the `js/Date.now()`
    /// epoch-ms integer also stored in the highlight's `:content {:image …}`).
    /// Returns the assets-relative path.
    ///
    /// **Non-dedup on purpose:** the filename IS the stable link from the `.edn`
    /// entry to the file, so a re-save must overwrite in place rather than rename
    /// on collision (which `reserve_asset` would do, breaking the link).
    pub fn write_pdf_area_image(
        &self,
        pdf_filename: &str,
        page: i64,
        id: &str,
        stamp: i64,
        bytes: &[u8],
    ) -> io::Result<String> {
        let key = crate::pdf::asset_key(pdf_filename);
        let dir = self.assets_path().join(&key);
        self.ensure_asset_write_target(&dir)?;
        fs::create_dir_all(&dir)?;
        let name = format!("{page}_{id}_{stamp}.png");
        // The highlight `id` round-trips through the graph `.edn`, so a synced/hand-edited
        // file can control it — reject any path separator so it can't escape the assets
        // dir and write a `.png` anywhere (audit M3, path traversal).
        top_level_asset_name(&name)?;
        let target = dir.join(&name);
        self.ensure_asset_write_target(&target)?;
        atomic_write(&target, bytes)?;
        Ok(format!("{key}/{name}"))
    }

    /// After the highlight sidecar + hls page pair is durably committed, move
    /// deleted area crops to recoverable asset trash. OG removes this exact crop
    /// with its highlight (`extensions/pdf/core.cljs:155-159` and
    /// `extensions/pdf/assets.cljs:137-147` at OG 6e7afa8eb); Tine keeps the same
    /// lifecycle without introducing OG's hard delete.
    ///
    /// Cleanup is deliberately best-effort and compare-guarded: the paired save
    /// is already committed, so a cleanup failure must not make the frontend
    /// restore stale state. Any sidecar change before or immediately after a move
    /// aborts cleanup, rolling that move back when possible.
    fn trash_deleted_pdf_area_images(
        &self,
        source_key: &str,
        edn_path: &Path,
        committed_edn: &str,
        source_sidecar_guard: Option<(&Path, &str)>,
        deleted: &[crate::pdf::Highlight],
    ) {
        if deleted.is_empty() {
            return;
        }
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Asset);
        for highlight in deleted {
            let Some(stamp) = highlight.image else {
                continue;
            };
            let Ok(Some(current_edn)) = read_optional_text(edn_path) else {
                return;
            };
            if current_edn != committed_edn {
                return;
            }
            if source_sidecar_guard.is_some_and(|(path, baseline)| {
                read_optional_text(path)
                    .map(|raw| raw.as_deref() != Some(baseline))
                    .unwrap_or(true)
            }) {
                return;
            }
            if crate::pdf::parse_highlights(&current_edn)
                .iter()
                .any(|remaining| remaining.image == Some(stamp))
            {
                continue;
            }

            let name = format!("{}_{}_{}.png", highlight.page, highlight.id, stamp);
            if top_level_asset_name(&name).is_err() {
                continue;
            }
            let source = self.assets_path().join(source_key).join(&name);
            if !source.is_file()
                || self.ensure_asset_write_target(&source).is_err()
                || self.ensure_write_target(&trash).is_err()
                || fs::create_dir_all(&trash).is_err()
            {
                continue;
            }
            let trash_name = format!("{}__pdf-area__{}__{name}", trash_stamp(), source_key);
            if top_level_asset_name(&trash_name).is_err() {
                continue;
            }
            let destination = trash.join(trash_name);
            if move_to_trash(&source, &destination, &trash).is_err() {
                continue;
            }

            // A non-cooperating writer can change the sidecar between the
            // last-moment read and rename. Put the crop back if that happened.
            let primary_unchanged = read_optional_text(edn_path)
                .map(|raw| raw.as_deref() == Some(committed_edn))
                .unwrap_or(false);
            let source_unchanged = source_sidecar_guard.is_none_or(|(path, baseline)| {
                read_optional_text(path)
                    .map(|raw| raw.as_deref() == Some(baseline))
                    .unwrap_or(false)
            });
            if primary_unchanged && source_unchanged {
                continue;
            }
            let _ = move_file_noreplace(&destination, &source);
            return;
        }
    }

    /// Read highlights for a PDF from `assets/<key>.edn`.
    ///
    /// If the OG-compatible key's file is absent but a file under Tine's old
    /// `legacy_asset_key` exists, read that instead (it is migrated forward to
    /// the new key on the next `write_highlights`). This keeps highlights made
    /// by pre-launch Tine builds from disappearing after the key change.
    pub fn read_highlights(&self, pdf_filename: &str) -> Vec<crate::pdf::Highlight> {
        self.read_pdf_state(pdf_filename).highlights
    }

    fn read_pdf_state(&self, pdf_filename: &str) -> crate::pdf::PdfState {
        let key = crate::pdf::asset_key(pdf_filename);
        let s = self
            .asset_file_for_read(&format!("{key}.edn"))
            .and_then(fs::read_to_string)
            .ok()
            .or_else(|| {
                let legacy = crate::pdf::legacy_asset_key(pdf_filename);
                (legacy != key)
                    .then(|| {
                        self.asset_file_for_read(&format!("{legacy}.edn"))
                            .and_then(fs::read_to_string)
                            .ok()
                    })
                    .flatten()
            });
        s.map(|s| crate::pdf::parse_pdf_state(&s))
            .unwrap_or_default()
    }

    fn existing_hls_page_path(&self, key: &str) -> io::Result<Option<PathBuf>> {
        let name = crate::pdf::hls_page_name(key);
        let md = self.pages_path().join(format!("{name}.md"));
        let org = self.pages_path().join(format!("{name}.org"));
        match (md.exists(), org.exists()) {
            (true, true) => Err(twin_error(&name)),
            (true, false) => Ok(Some(md)),
            (false, true) => Ok(Some(org)),
            (false, false) => Ok(self
                .find_entry(&name, PageKind::Page)
                .map(|entry| entry.path)),
        }
    }

    fn hls_page_path(&self, pdf_filename: &str, key: &str) -> io::Result<PathBuf> {
        if let Some(existing) = self.existing_hls_page_path(key)? {
            return Ok(existing);
        }
        // A key migration renames the annotation page but must not implicitly
        // convert its syntax because the graph's preference changed meanwhile.
        let legacy_key = crate::pdf::legacy_asset_key(pdf_filename);
        if legacy_key != key && !self.asset_key_in_use_by_pdf(&legacy_key) {
            if let Some(legacy) = self.existing_hls_page_path(&legacy_key)? {
                let ext = legacy
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("md");
                return Ok(legacy.with_file_name(format!(
                    "{}.{}",
                    crate::pdf::hls_page_name(key),
                    ext
                )));
            }
        }
        Ok(self.pages_path().join(format!(
            "{}.{}",
            crate::pdf::hls_page_name(key),
            self.preferred_format().ext()
        )))
    }

    fn pdf_sidecar_for_update(&self, pdf_filename: &str) -> io::Result<PathBuf> {
        let key = crate::pdf::asset_key(pdf_filename);
        let primary = self.assets_path().join(format!("{key}.edn"));
        if primary.exists() {
            return Ok(primary);
        }
        let legacy_key = crate::pdf::legacy_asset_key(pdf_filename);
        if legacy_key != key && !self.asset_key_in_use_by_pdf(&legacy_key) {
            let legacy = self.assets_path().join(format!("{legacy_key}.edn"));
            if legacy.exists() {
                return Ok(legacy);
            }
        }
        Ok(primary)
    }

    /// Open-time OG artifact initialization plus the persisted PDF state. Existing
    /// sidecars/pages are read without being rewritten; only missing artifacts are
    /// created. Old Tine-key artifacts remain in place until the established
    /// edit-time migration path can carry their notes forward safely.
    pub fn open_pdf(&self, pdf_filename: &str, label: &str) -> io::Result<crate::pdf::PdfState> {
        let key = crate::pdf::asset_key(pdf_filename);
        let page_path = self.hls_page_path(pdf_filename, &key)?;
        self.ensure_write_target(&page_path)?;
        let page_lock = self.page_lock(&page_path);
        let _guard = page_lock.lock().unwrap();

        fs::create_dir_all(self.assets_path())?;
        let sidecar_path = self.pdf_sidecar_for_update(pdf_filename)?;
        self.ensure_asset_write_target(&sidecar_path)?;
        let mut sidecar = read_optional_text(&sidecar_path)?;
        if let Some(raw) = &sidecar {
            validate_highlight_edn(raw)?;
        } else {
            let skeleton = crate::pdf::write_highlights(&[], "");
            // Recheck immediately before publish so an external creator wins.
            if let Some(external) = read_optional_text(&sidecar_path)? {
                validate_highlight_edn(&external)?;
                sidecar = Some(external);
            } else {
                match atomic_write_new(&sidecar_path, skeleton.as_bytes()) {
                    Ok(()) => sidecar = Some(skeleton),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let external = read_optional_text(&sidecar_path)?.ok_or(error)?;
                        validate_highlight_edn(&external)?;
                        sidecar = Some(external);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let state = crate::pdf::parse_pdf_state(sidecar.as_deref().unwrap_or(""));

        // Do not create a new-key page on top of an unmigrated legacy page: the
        // normal highlight write carries its notes forward under one guarded merge.
        let legacy_key = crate::pdf::legacy_asset_key(pdf_filename);
        let legacy_page_exists = legacy_key != key
            && !self.asset_key_in_use_by_pdf(&legacy_key)
            && self.existing_hls_page_path(&legacy_key)?.is_some();
        let page_baseline = read_optional_text(&page_path)?;
        if page_baseline.is_none() && !legacy_page_exists {
            let format = Format::from_path(&page_path);
            let page_doc = crate::pdf::hls_page_document_for_format(
                pdf_filename,
                label,
                &state.highlights,
                format,
            );
            let content = serialize_pdf_hls_page(&page_path, &page_doc, None)?;
            let page_rev = self.commit_write(&page_path, &content, None, true)?;
            let name = crate::pdf::hls_page_name(&key);
            let entry = PageEntry {
                name,
                kind: PageKind::Page,
                date_key: None,
                rel_path: self.rel_path(&page_path),
                path: page_path.clone(),
            };
            self.cache_upsert(entry, page_doc, page_rev.clone());
            self.drop_self_write_marker(&page_path, &page_rev);
        }
        Ok(state)
    }

    /// Persist only OG's last-view page/scale fields. The hls-page lock is shared
    /// with highlight writes so an in-app highlight update cannot race this
    /// read-modify-write; external writers are handled by the same bounded
    /// compare/retry discipline.
    pub fn write_pdf_view_state(
        &self,
        pdf_filename: &str,
        page: i64,
        scale: f64,
    ) -> io::Result<()> {
        let key = crate::pdf::asset_key(pdf_filename);
        let page_path = self.hls_page_path(pdf_filename, &key)?;
        let lock = self.page_lock(&page_path);
        let _guard = lock.lock().unwrap();
        fs::create_dir_all(self.assets_path())?;
        let sidecar_path = self.pdf_sidecar_for_update(pdf_filename)?;
        self.ensure_asset_write_target(&sidecar_path)?;
        for _attempt in 0..4 {
            let baseline = read_optional_text(&sidecar_path)?;
            if let Some(raw) = &baseline {
                validate_highlight_edn(raw)?;
            }
            let next =
                crate::pdf::write_pdf_view_state(baseline.as_deref().unwrap_or(""), page, scale)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid PDF view state")
                    })?;
            if read_optional_text(&sidecar_path)? != baseline {
                continue;
            }
            let publish = if baseline.is_none() {
                atomic_write_new(&sidecar_path, next.as_bytes())
            } else {
                atomic_write(&sidecar_path, next.as_bytes())
            };
            match publish {
                Ok(()) => return Ok(()),
                Err(error)
                    if baseline.is_none() && error.kind() == io::ErrorKind::AlreadyExists =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "highlight sidecar changed repeatedly during view-state update",
        ))
    }

    fn asset_key_in_use_by_pdf(&self, candidate_key: &str) -> bool {
        let Ok(entries) = fs::read_dir(self.assets_path()) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let filename = entry.file_name();
            let Some(filename) = filename.to_str() else {
                return false;
            };
            if !filename.ends_with(".pdf") && !filename.ends_with(".PDF") {
                return false;
            }
            crate::pdf::asset_key(filename) == candidate_key
        })
    }

    /// Persist highlights: write `assets/<key>.edn` and the `hls__<key>` page.
    /// `base_ids` are the highlight ids the editor LOADED (its baseline) — used for
    /// a 3-way merge so a highlight the user deleted is honored while one added
    /// externally (e.g. by OG between load and write) is still preserved.
    pub fn write_highlights(
        &self,
        pdf_filename: &str,
        label: &str,
        highlights: &[crate::pdf::Highlight],
        base_ids: &[String],
    ) -> io::Result<()> {
        let key = crate::pdf::asset_key(pdf_filename);
        // Legacy (pre-launch) key. When it differs and only the legacy files
        // exist, we read those as the baseline and migrate them to the new key
        // below — so the key change never strands existing highlights.
        let legacy_key = crate::pdf::legacy_asset_key(pdf_filename);
        let legacy_active = legacy_key != key && !self.asset_key_in_use_by_pdf(&legacy_key);
        let legacy_edn =
            legacy_active.then(|| self.assets_path().join(format!("{legacy_key}.edn")));
        let legacy_page = if legacy_active {
            self.existing_hls_page_path(&legacy_key)?
        } else {
            None
        };
        // Serialize against an editor save of the SAME `hls__` page (see
        // `page_locks`): hold the page lock across the .edn merge AND the page
        // read→merge→write→cache_upsert, so the two writers can't clobber each
        // other or trip a false self-write conflict.
        let page_path = self.hls_page_path(pdf_filename, &key)?;
        self.ensure_write_target(&page_path)?;
        let lock = self.page_lock(&page_path);
        let _guard = lock.lock().unwrap();
        fs::create_dir_all(self.assets_path())?;
        let edn_path = self.assets_path().join(format!("{key}.edn"));
        self.ensure_asset_write_target(&edn_path)?;
        // 3-way merge against the on-disk set: keep our current highlights, plus
        // any disk highlight that is an EXTERNAL addition (id not in our baseline
        // and not already present). A highlight we deliberately deleted (in the
        // baseline, absent from current) is NOT resurrected. Prefer the new-key
        // file; fall back to the legacy-key file (migrating it forward).
        let base: std::collections::HashSet<&str> = base_ids.iter().map(|s| s.as_str()).collect();
        // Read every artifact that will participate before committing either one.
        // If the notes page (or its legacy source) is unreadable, abort while the
        // sidecar is still untouched rather than leaving a half-updated pair.
        let page_baseline = read_optional_text(&page_path)?;
        let legacy_page_baseline = if page_baseline.is_none() {
            match &legacy_page {
                Some(path) => read_optional_text(path)?,
                None => None,
            }
        } else {
            None
        };
        let existing_raw = page_baseline
            .clone()
            .or_else(|| legacy_page_baseline.clone());
        // The sidecar and annotation page are one logical update. Reject a
        // non-round-trippable Org page before publishing the sidecar so a failed
        // page serialization cannot leave the pair half-updated.
        if Format::from_path(&page_path) == Format::Org
            && existing_raw
                .as_deref()
                .is_some_and(|raw| !crate::org::org_editable(raw))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "org highlight page is read-only (does not round-trip)",
            ));
        }
        // Merge and publish the sidecar with the same external-writer guard as
        // config updates. If Logseq/Syncthing changes either the primary or the
        // legacy fallback after our read, retry against those new bytes instead
        // of replacing them with a stale full-file serialization.
        let mut committed_sidecar = None;
        for _attempt in 0..4 {
            let primary_baseline = read_optional_text(&edn_path)?;
            let legacy_baseline = if primary_baseline.is_none() {
                match &legacy_edn {
                    Some(path) => read_optional_text(path)?,
                    None => None,
                }
            } else {
                None
            };
            let existing_edn = primary_baseline.as_ref().or(legacy_baseline.as_ref());
            if let Some(raw) = existing_edn {
                validate_highlight_edn(raw)?;
            }
            let disk_highlights = existing_edn
                .map(|raw| crate::pdf::parse_highlights(raw))
                .unwrap_or_default();
            let have: std::collections::HashSet<&str> =
                highlights.iter().map(|h| h.id.as_str()).collect();
            let mut merged = highlights.to_vec();
            for h in &disk_highlights {
                if !have.contains(h.id.as_str()) && !base.contains(h.id.as_str()) {
                    merged.push(h.clone());
                }
            }
            let merged_ids: std::collections::HashSet<&str> =
                merged.iter().map(|h| h.id.as_str()).collect();
            let deleted_areas: Vec<crate::pdf::Highlight> = disk_highlights
                .into_iter()
                .filter(|h| h.image.is_some() && !merged_ids.contains(h.id.as_str()))
                .collect();
            let area_source_key = if primary_baseline.is_none() && legacy_baseline.is_some() {
                legacy_key.as_str()
            } else {
                key.as_str()
            };
            let next =
                crate::pdf::write_highlights(&merged, existing_edn.map_or("", String::as_str));
            let primary_now = read_optional_text(&edn_path)?;
            let legacy_now = if primary_now.is_none() && primary_baseline.is_none() {
                match &legacy_edn {
                    Some(path) => read_optional_text(path)?,
                    None => None,
                }
            } else {
                None
            };
            if primary_now != primary_baseline
                || (primary_baseline.is_none() && legacy_now != legacy_baseline)
            {
                continue;
            }
            let publish = if primary_baseline.is_none() {
                atomic_write_new(&edn_path, next.as_bytes())
            } else {
                atomic_write(&edn_path, next.as_bytes())
            };
            match publish {
                Ok(()) => {}
                Err(error)
                    if primary_baseline.is_none()
                        && error.kind() == io::ErrorKind::AlreadyExists =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
            committed_sidecar = Some((
                merged,
                primary_baseline,
                legacy_baseline,
                next,
                area_source_key.to_string(),
                deleted_areas,
            ));
            break;
        }
        let (
            merged,
            committed_primary_edn_baseline,
            committed_legacy_edn_baseline,
            committed_edn,
            area_source_key,
            deleted_areas,
        ) = committed_sidecar.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "highlight sidecar changed repeatedly during update",
            )
        })?;

        // Upsert into the existing hls page, preserving note children by id.
        // (`page_path` + its lock were taken at the top of this fn.) Prefer the
        // new-key page; fall back to the legacy-key page so its user notes are
        // carried over during migration.
        // The hls page's OWN bytes are its write baseline; the legacy fallback is
        // used only as a migration merge source. The A3-style recheck below
        // compares the page against this baseline.
        let existing = existing_raw
            .as_deref()
            .map(|raw| parse_doc(&page_path, raw));
        let page_doc = crate::pdf::merge_hls_page_for_format(
            existing.as_ref(),
            pdf_filename,
            label,
            &merged,
            Format::from_path(&page_path),
        );
        // Preserve the notes page's CRLF (shared with write_page), then go through
        // the shared write commit (self-write marker → A3 recheck vs `page_baseline`
        // → atomic_write). The recheck is mandatory here precisely because this path
        // lacked save_page's guard: a non-cooperating external writer (OG / Syncthing)
        // could have added a note between `page_baseline` and now, and merge_hls_page
        // only carried notes from the bytes we read — so an overwrite would clobber it.
        // On mismatch → conflict; PdfViewer.persist toasts + reverts and a retry merges
        // cleanly (the .edn was already 3-way-merged, so no highlight is lost).
        let page_md = serialize_pdf_hls_page(&page_path, &page_doc, existing_raw.as_deref())?;
        let page_rev = match self.commit_write(&page_path, &page_md, page_baseline.as_deref(), true)
        {
            Ok(rev) => rev,
            Err(page_error) => {
                if let Err(rollback_error) = self.rollback_highlight_sidecar(
                    &edn_path,
                    committed_primary_edn_baseline.as_deref(),
                    &committed_edn,
                ) {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "highlight notes page was not saved ({page_error}); the sidecar rollback also failed ({rollback_error})"
                        ),
                    ));
                }
                return Err(page_error);
            }
        };
        // The hls page is a real page; reflect it in the search cache.
        let name = crate::pdf::hls_page_name(&key);
        let entry = self.find_entry(&name, PageKind::Page).unwrap_or(PageEntry {
            name,
            kind: PageKind::Page,
            date_key: None,
            rel_path: self.rel_path(&page_path),
            path: page_path.clone(),
        });
        self.cache_upsert(entry, page_doc, page_rev.clone());
        // Drop the self-write marker now the write is published + cached (see
        // write_page / drop_self_write_marker).
        self.drop_self_write_marker(&page_path, &page_rev);
        let source_sidecar_guard = (area_source_key == legacy_key)
            .then(|| legacy_edn.as_deref())
            .flatten()
            .zip(committed_legacy_edn_baseline.as_deref());
        self.trash_deleted_pdf_area_images(
            &area_source_key,
            &edn_path,
            &committed_edn,
            source_sidecar_guard,
            &deleted_areas,
        );
        // Migrate-on-write cleanup is compare-and-recover: only retire a legacy
        // artifact if it still equals the exact bytes we merged. A concurrent
        // legacy update stays at its original path. Unchanged files are moved to
        // recoverable trash rather than hard-deleted.
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Conflict);
        self.ensure_write_target(&trash)?;
        fs::create_dir_all(&trash)?;
        if let (Some(path), Some(baseline)) = (&legacy_edn, &committed_legacy_edn_baseline) {
            if read_optional_text(path)?.as_ref() == Some(baseline) {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("legacy.edn");
                let dest = trash.join(format!("{}__legacy__{name}", trash_stamp()));
                if move_file_noreplace(path, &dest).is_ok()
                    && read_optional_text(&dest)?.as_ref() != Some(baseline)
                {
                    let _ = move_file_noreplace(&dest, path);
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "legacy highlight sidecar changed during migration cleanup",
                    ));
                }
            }
        }
        if let (Some(path), Some(baseline)) = (&legacy_page, &legacy_page_baseline) {
            if read_optional_text(path)?.as_ref() == Some(baseline) {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("legacy.md");
                let dest = trash.join(format!("{}__legacy__{name}", trash_stamp()));
                if move_file_noreplace(path, &dest).is_ok() {
                    if read_optional_text(&dest)?.as_ref() != Some(baseline) {
                        let _ = move_file_noreplace(&dest, path);
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "legacy highlight page changed during migration cleanup",
                        ));
                    }
                    self.cache_remove(&crate::pdf::hls_page_name(&legacy_key), PageKind::Page);
                }
            }
        }
        Ok(())
    }

    /// Map an on-disk `.md` path to its page entry (journal or page), or None if
    /// it isn't in the graph's journals/pages dirs.
    pub fn entry_for_path(&self, path: &Path) -> Option<PageEntry> {
        if !is_page_file(path) {
            return None;
        }
        let stem = path.file_stem().and_then(|s| s.to_str())?;
        // Accept a file anywhere UNDER journals/ or pages/, not just a direct child
        // (#21 recursive subdirs). The page name is the basename (the sub-path is
        // discarded, matching OG); the file's own `path` remains its load/save
        // identity. `starts_with` is a lexical prefix over path components, so a
        // file at `pages/x/foo.md` matches `pages/` but nothing outside it.
        if path.starts_with(self.journals_path()) {
            let (name, date_key) = match self.journal_format.parse(stem) {
                Some(d) => (self.journal_format.title(d), Some(d.ordinal_key())),
                None => (stem.to_string(), None),
            };
            Some(PageEntry {
                name,
                kind: PageKind::Journal,
                date_key,
                rel_path: self.rel_path(path),
                path: path.to_path_buf(),
            })
        } else if path.starts_with(self.pages_path()) {
            Some(PageEntry {
                name: decode_page_name(stem, self.config.file_name_format),
                kind: PageKind::Page,
                date_key: None,
                rel_path: self.rel_path(path),
                path: path.to_path_buf(),
            })
        } else {
            None
        }
    }

    /// Record that Tine just wrote content with rev `rev` to `path`, so the file
    /// watcher recognizes the write as ours (see `sync_file_content`). The map is
    /// consumed on first match; this hard cap is a backstop so a write that the
    /// watcher never observes (file deleted before the next poll, watcher idle)
    /// can't leak across a long session. Clearing only reopens the tiny
    /// rename→cache_upsert race for genuinely in-flight writes — harmless.
    fn note_self_write(&self, path: &Path, rev: String) {
        let mut recent = self.recent_writes.lock().unwrap();
        if recent.len() >= 1024 {
            recent.clear();
        }
        recent.insert(path.to_path_buf(), rev);
    }

    /// Drop the self-write marker for `path` once a write is fully published (after
    /// the cache_upsert), bounding it to its write window so it can never outlive
    /// this save and later suppress a real external change. Removes it only if it's
    /// still OURS (a concurrent same-path writer may have replaced it).
    fn drop_self_write_marker(&self, path: &Path, rev: &str) {
        let mut recent = self.recent_writes.lock().unwrap();
        if recent.get(path).is_some_and(|r| r == rev) {
            recent.remove(path);
        }
    }

    /// Restore a PDF highlight sidecar after the paired `hls__` page failed to
    /// commit. The sidecar is restored only while it still contains the exact
    /// bytes this call published; a later external edit is never knowingly
    /// replaced. A newly-created sidecar is moved to recoverable conflict trash
    /// rather than hard-deleted.
    fn rollback_highlight_sidecar(
        &self,
        path: &Path,
        baseline: Option<&str>,
        committed: &str,
    ) -> io::Result<()> {
        if read_optional_text(path)?.as_deref() != Some(committed) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "highlight sidecar changed after its commit",
            ));
        }
        if let Some(previous) = baseline {
            return atomic_write(path, previous.as_bytes());
        }

        let trash = typed_trash_dir(&self.root, TrashEntryKind::Conflict);
        self.ensure_write_target(&trash)?;
        fs::create_dir_all(&trash)?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("highlights.edn");
        let destination = trash.join(format!("{}__failed-highlight-pair__{name}", trash_stamp()));
        move_file_noreplace(path, &destination)?;
        if read_optional_text(&destination)?.as_deref() == Some(committed) {
            return Ok(());
        }

        let _ = move_file_noreplace(&destination, path);
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "highlight sidecar changed during rollback",
        ))
    }

    /// Remove a transaction-owned live file without ever unlinking a race winner.
    /// The currently named inode is first moved atomically into recoverable
    /// conflict trash. Exact expected bytes stay there as the withdrawn copy; a
    /// different inode is restored if the live name is free, or retained in
    /// recovery if another writer has already recreated the name.
    fn withdraw_file_to_conflict_if_exact(
        &self,
        path: &Path,
        expected: &[u8],
        reason: &str,
    ) -> io::Result<bool> {
        withdrawal_race_hook(path)?;
        if fs::symlink_metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound) {
            return Ok(false);
        }
        self.ensure_write_target(path)?;
        let trash = typed_trash_dir(&self.root, TrashEntryKind::Conflict);
        self.ensure_write_target(&trash)?;
        fs::create_dir_all(&trash)?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("file");
        let staged = trash.join(format!("{}__{reason}__{name}", trash_stamp()));
        match move_file_noreplace(path, &staged) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
        let staged_bytes = match fs::read(&staged) {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = move_file_noreplace(&staged, path);
                return Err(error);
            }
        };
        if staged_bytes == expected {
            return Ok(true);
        }
        match move_file_noreplace(&staged, path) {
            Ok(()) => Ok(false),
            // A new live winner appeared after staging. Keeping the displaced
            // inode in conflict trash preserves both versions.
            Err(_) if path.exists() => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The shared page-write commit protocol, written ONCE so `write_page` and
    /// `write_highlights` can't drift apart on it (a missed step = a stale marker
    /// suppressing a real external edit, or a phantom conflict):
    ///   record self-write marker → ensure parent dir → (A3) optional last-moment
    ///   recheck that disk still == `baseline` → atomic_write.
    /// We hold the page lock, so no other Tine writer raced us; the recheck guards
    /// a non-cooperating external writer (OG/Syncthing) that touched the file since
    /// our baseline read — on mismatch we abort WITHOUT writing and drop our marker
    /// so the watcher still sees the external change. Returns the new content rev.
    /// The post-publish marker drop is `drop_self_write_marker` (it must run AFTER
    /// the caller's cache_upsert, so it stays the caller's responsibility).
    fn commit_write(
        &self,
        path: &Path,
        content: &str,
        baseline: Option<&str>,
        recheck: bool,
    ) -> io::Result<String> {
        let rev = content_rev(content);
        self.note_self_write(path, rev.clone());
        let result = (|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if recheck {
                // Only NotFound means "no baseline file". Permission errors, invalid
                // UTF-8, and transient I/O failures must abort; collapsing them to
                // None would authorize an overwrite of unreadable on-disk data.
                let now = read_optional_text(path)?;
                let still_matches = match (now.as_deref(), baseline) {
                    (Some(n), Some(e)) => n == e,
                    (None, None) => true,
                    _ => false,
                };
                if !still_matches {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
            }
            if baseline.is_none() {
                atomic_write_new(path, content.as_bytes())
            } else {
                atomic_write(path, content.as_bytes())
            }
        })();
        if let Err(error) = result {
            self.drop_self_write_marker(path, &rev);
            return Err(error);
        }
        Ok(rev)
    }

    /// Reconcile a (possibly externally-changed) file with the in-memory cache.
    /// Returns the entry only if its parsed content actually differs from the
    /// cache (i.e. a real external change) — Tine's own writes keep the cache in
    /// sync, so they return None. No-op if the cache hasn't been built yet.
    pub fn sync_file(&self, path: &Path) -> Option<PageEntry> {
        match self.sync_file_checked(path) {
            Ok(entry) => entry,
            Err(_error) => {
                #[cfg(debug_assertions)]
                eprintln!("file reconcile deferred for {}: {_error}", path.display());
                None
            }
        }
    }

    /// Checked watcher entrypoint. When managed sync is active, an unexplained
    /// file must be durably imported before its bytes enter the live cache.
    pub fn sync_file_checked(&self, path: &Path) -> io::Result<Option<PageEntry>> {
        let lock = self.page_lock(path);
        let _guard = lock.lock().unwrap();
        // Watch events are untrusted path inputs. Never follow a page symlink
        // (which could expose an arbitrary file outside the graph), and recheck
        // canonical containment immediately before the read to close rename /
        // symlink-swap races between directory scanning and reconciliation.
        let md = match fs::symlink_metadata(path) {
            Ok(md) => md,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if md.file_type().is_symlink() || !md.is_file() || !path_stays_within_root(&self.root, path)
        {
            return Ok(None);
        }
        let mut content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let imported_external = self.reconcile_managed_external_locked(path, &content, None)?;
        if imported_external {
            content = fs::read_to_string(path)?;
        }
        // The watcher consumes the self-write marker (one-shot) so the map stays
        // bounded to in-flight writes.
        let reconciled = self.sync_file_content(path, &content, true);
        if imported_external {
            Ok(reconciled.or_else(|| self.entry_for_path(path)))
        } else {
            Ok(reconciled)
        }
    }

    /// Import an unexplained page-file snapshot and immediately publish the
    /// joined CRDT projection. Caller holds this path's page lock. Returns true
    /// when the input was external (as opposed to a receipt-backed projection).
    fn reconcile_managed_external_locked(
        &self,
        path: &Path,
        content: &str,
        page_id_hint: Option<CrdtPageId>,
    ) -> io::Result<bool> {
        if path_is_sync_conflict(path) {
            return Ok(false);
        }
        let rel = self.rel_path(path);
        let mut sync_guard = self.managed_sync.lock().unwrap();
        let Some(sync) = sync_guard.as_mut() else {
            return Ok(false);
        };
        if sync
            .is_known_projection(&rel, content)
            .map_err(crdt_io_error)?
        {
            return Ok(false);
        }
        let entry = self.entry_for_path(path).ok_or_else(bad_path)?;
        let mut doc = parse_doc(path, content);
        assign_doc_runtime_ids(&mut doc.roots, &rel);
        let mut page = page_dto(&entry, &doc);
        page.path = rel.clone();
        page.rev = Some(content_rev(content));
        page.format = Format::from_path(path);
        page.read_only = read_only_org(path, content);
        if page.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read-only Org projection cannot be imported into managed sync",
            ));
        }
        page = page_with_persisted_sync_ids(&page)?;
        let current_pages = sync.materialize_pages().map_err(crdt_io_error)?;
        let current_at_path = current_pages.iter().find(|snapshot| snapshot.path == rel);
        let mut incoming_ids = std::collections::HashSet::new();
        collect_persisted_sync_ids(&page.blocks, page.format, &mut incoming_ids);
        let mut owners = std::collections::HashMap::new();
        for snapshot in &current_pages {
            if snapshot.path == rel {
                continue;
            }
            for block in &snapshot.blocks {
                owners.insert(block.id.to_string(), (snapshot.id, snapshot.path.clone()));
            }
        }
        let conflicting_ids: std::collections::HashSet<String> = incoming_ids
            .iter()
            .filter(|id| owners.contains_key(*id))
            .cloned()
            .collect();
        let conflicting_owners: std::collections::HashSet<(CrdtPageId, String)> = conflicting_ids
            .iter()
            .filter_map(|id| owners.get(id).cloned())
            .collect();

        // A destination-first file copy and a rename initially look alike because
        // both retain `id::`. The old projection still existing makes the case
        // ambiguous, so preserve both pages by assigning the copy fresh durable
        // identities. Only one now-missing source is sufficient rename evidence.
        let rename_owner = (current_at_path.is_none() && conflicting_owners.len() == 1)
            .then(|| conflicting_owners.iter().next().cloned())
            .flatten()
            .filter(|(_, source_path)| {
                self.resolve_rel(source_path)
                    .is_some_and(|source| !source.exists())
            });
        if rename_owner.is_none() && !conflicting_ids.is_empty() {
            rekey_conflicting_sync_ids(&mut page.blocks, page.format, &conflicting_ids);
        }
        let page_id = page_id_hint
            .or_else(|| current_at_path.map(|snapshot| snapshot.id))
            .or_else(|| rename_owner.map(|(id, _)| id))
            .unwrap_or_else(CrdtPageId::new);
        sync.commit_page(crdt_snapshot_for_page(&page, page_id)?)
            .map_err(crdt_io_error)?;
        let joined = sync
            .materialize_page(page_id)
            .map_err(crdt_io_error)?
            .ok_or_else(|| io::Error::other("imported page vanished from managed sync"))?;
        let joined = page_dto_from_crdt(&joined)?;
        drop(sync_guard);

        let cache = self.path_is_cacheable(path);
        self.write_page(&joined, path, Some(content), true, cache)?;
        self.record_managed_projection(path);
        Ok(true)
    }

    /// Reconcile the cache for `path` given its already-read `content` — so a
    /// caller that has just read the file (e.g. load_page) doesn't read it twice.
    /// `consume_self_write`: whether a match on the self-write marker REMOVES it.
    /// The watcher passes true (bounding); load_page passes false — load_page can
    /// run in the rename→cache_upsert window and must not steal the marker out
    /// from under the watcher, which would turn the watcher's later poll into a
    /// false "changed on disk".
    fn sync_file_content(
        &self,
        path: &Path,
        content: &str,
        consume_self_write: bool,
    ) -> Option<PageEntry> {
        let entry = self.entry_for_path(path)?;
        // A sync-tool conflict copy (`*.sync-conflict-*`) is never a real page: keep
        // it out of the `(kind,name)` cache (it would show as a garbage page and its
        // shared `id::` values would churn the id space). It's surfaced separately via
        // `list_sync_conflicts` and loaded on demand by path for the merge UI.
        if path_is_sync_conflict(path) {
            return None;
        }
        // A shadow journal file (a title-named leftover coexisting with a canonical
        // date-stem file for the same day, #21) must never be reconciled into the
        // `(kind,name)` cache — that slot belongs to the canonical file, and caching
        // the shadow there would make name-resolution serve the wrong file. A
        // shadow's own external edits are picked up by a fresh path-addressed load
        // (`load_by_path`), so there's nothing to reconcile here.
        if entry.kind == PageKind::Journal {
            if let Some(date) = entry.date_key.map(crate::date::JournalDate::from_ordinal) {
                if self.is_shadow_journal(path, date) {
                    return None;
                }
            }
        }
        // Our own write: if the bytes on disk are exactly what Tine last wrote
        // here, this is not an external change — suppress it even if the parse
        // cache hasn't folded in the write yet (the rename→cache_upsert gap the
        // watcher can read into). The cache comparison below alone races that gap.
        let disk_rev = content_rev(content);
        // The self-write marker exists ONLY to stop the WATCHER raising a false
        // "changed on disk" during the rename→cache_upsert window, so only the
        // watcher (consume_self_write) consults it. load_page must NOT short-
        // circuit here: in that window the cache is still pre-write, so returning
        // early would serve a STALE cached doc with the fresh disk rev (a later
        // save could then clobber disk). load_page instead falls through to the
        // disk_revs fast path / parse-reconcile below and serves content matching
        // the exact bytes it just read.
        if consume_self_write {
            let mut recent = self.recent_writes.lock().unwrap();
            if recent.get(path).is_some_and(|r| *r == disk_rev) {
                recent.remove(path);
                return None;
            }
        }
        // Fast freshness check (B1): if the cache for this page already reflects
        // these exact disk bytes, there's nothing to reconcile — skip the parse +
        // serialize→parse normalization comparison below. Read disk_revs WHILE
        // HOLDING cache.read(): cache_upsert publishes the cache slot and its rev
        // together under cache.write(), so taking the cache read lock here makes
        // the reader mutually exclusive with that writer and guarantees a
        // consistent (cache, rev) pair — without it, the reader could observe a
        // slot already updated to a new doc while its rev hadn't been inserted yet
        // (separate lock), match the stale rev against disk, and serve the wrong
        // doc. Lock order cache → disk_revs matches every writer, so no deadlock;
        // the guard is dropped before the reconcile path below re-locks the cache.
        // A missing/mismatched entry falls through to the exact comparison, so this
        // can only ever save work, never serve stale content.
        {
            let _cache_guard = self.cache.read().unwrap();
            if self
                .disk_revs
                .read()
                .unwrap()
                .get(path)
                .is_some_and(|r| *r == disk_rev)
            {
                return None;
            }
        }
        let newdoc = parse_doc(path, content);
        {
            let guard = self.cache.read().unwrap();
            let Some(cache) = guard.as_ref() else {
                // Cache not built yet: nothing to reconcile in the parse cache, but
                // the page SET may have changed (this could be a newly-created
                // file). Drop the page-list memo so list_pages — and the eventual
                // warm build that reads it — re-scan the dir and include it.
                // This path does NOT bump cache_gen, so the gen-keyed find_entry
                // index would otherwise stay stale here (miss the new/removed file)
                // until some other op bumps the gen — drop it alongside the list memo.
                drop(guard);
                *self.page_list_cache.write().unwrap() = None;
                *self.find_entry_cache.write().unwrap() = None;
                *self.cache_index.write().unwrap() = None;
                return None;
            };
            if let Some(i) = self.cached_page_index_for_path(cache, path) {
                let cached = &cache[i].1;
                // Compare CONTENT, not the in-memory uuids: cached blocks carry
                // generated uuids (assigned at cache build / upsert), while a
                // fresh `parse` leaves them empty for non-ref-target blocks, so a
                // direct `cached == newdoc` would never match and would flag every
                // one of Tine's own writes as an external change. Normalize the
                // cached doc through the same serialize→parse round-trip the file
                // went through (both sides then have empty uuids) and compare.
                let cached_norm = match Format::from_path(path) {
                    Format::Md => {
                        let opts = doc::SerializeOpts::detect(Some(content));
                        doc::parse(&doc::serialize_with(cached, &opts))
                    }
                    Format::Org => crate::org::parse_org(&crate::org::serialize_org_detect(
                        cached,
                        Some(content),
                    )),
                };
                if cached_norm == newdoc {
                    return None; // unchanged / our own write
                }
            }
        }
        self.cache_upsert(entry.clone(), newdoc, disk_rev);
        Some(entry)
    }

    /// Drop a file deleted on disk from the cache; returns the entry if it was
    /// cached (so the UI can react).
    pub fn forget_file(&self, path: &Path) -> Option<PageEntry> {
        let entry = self.entry_for_path(path)?;
        let was_cached = {
            let guard = self.cache.read().unwrap();
            guard
                .as_ref()
                .is_some_and(|c| self.cached_page_index_for_path(c, &entry.path).is_some())
        };
        self.cache_remove_path(&entry);
        was_cached.then_some(entry)
    }

    /// Checked watcher path for an externally removed projection. A Tine delete
    /// or rename already removed the CRDT page/path, so this is idempotent; an OG
    /// Logseq/provider deletion becomes operation truth before cache eviction.
    pub fn sync_deleted_file(&self, path: &Path) -> io::Result<Option<PageEntry>> {
        let lock = self.page_lock(path);
        let guard = lock.lock().unwrap();
        if path.exists() {
            return Ok(None);
        }
        let promoted = self.commit_managed_delete(path)?;
        let forgotten = self.forget_file(path);
        drop(guard);
        if let Some((page_id, paths)) = promoted {
            self.project_managed_page(page_id, &paths)?;
        }
        Ok(forgotten)
    }

    /// Count blocks that still need a durable on-disk identity before managed
    /// sync can be enabled. This is read-only and suitable for a confirmation UI.
    pub fn sync_identity_plan(&self) -> io::Result<SyncIdentityPlan> {
        let mut plan = SyncIdentityPlan {
            pages: 0,
            blocks: 0,
        };
        for entry in self.list_pages() {
            let page = self.load_page(&entry)?;
            let missing = count_missing_sync_ids(&page.blocks, page.format);
            if missing != 0 {
                plan.pages += 1;
                plan.blocks += missing;
            }
        }
        Ok(plan)
    }

    /// Persist exact block identities through the ordinary guarded page-save
    /// path. All pages are parsed and validated before the first write. A race
    /// with an external writer can still stop the write phase part-way through;
    /// that is safe and resumable because adding an id is semantically inert and
    /// the next run skips every block already migrated.
    pub fn migrate_sync_identities(&self) -> io::Result<SyncIdentityMigration> {
        let mut prepared = Vec::new();
        let mut graph_ids = std::collections::HashSet::new();
        for entry in self.list_pages() {
            let page = self.load_page(&entry)?;
            let missing = count_missing_sync_ids(&page.blocks, page.format);
            if page.read_only {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is read-only and cannot join managed sync", page.path),
                ));
            }
            let migrated = page_with_persisted_sync_ids(&page)?;
            validate_graph_sync_ids(&migrated.blocks, migrated.format, &mut graph_ids)?;
            if missing != 0 {
                prepared.push((migrated, missing));
            }
        }

        let mut result = SyncIdentityMigration {
            pages_changed: 0,
            blocks_changed: 0,
        };
        for (page, missing) in prepared {
            self.save_page(&page, page.rev.as_deref())?;
            result.pages_changed += 1;
            result.blocks_changed += missing;
        }
        Ok(result)
    }

    /// Return a save-ready clone whose every block id is represented in the
    /// page's native Logseq syntax. Managed-sync saves use this for new blocks
    /// created after the one-time graph migration.
    pub fn page_with_sync_ids(&self, page: &PageDto) -> io::Result<PageDto> {
        page_with_persisted_sync_ids(page)
    }

    /// If managed sync is active, stamp any newly-created block ids and commit
    /// the intended page state to the immutable operation stream. The returned
    /// DTO is the exact projection that must be written afterwards.
    fn commit_managed_page(&self, page: &PageDto, path: &Path) -> io::Result<Option<PageDto>> {
        let mut sync_guard = self.managed_sync.lock().unwrap();
        let Some(sync) = sync_guard.as_mut() else {
            return Ok(None);
        };
        let mut prepared = page_with_persisted_sync_ids(page)?;
        prepared.path = self.rel_path(path);
        let page_id = sync
            .materialize_page(prepared.path.as_str())
            .map_err(crdt_io_error)?
            .map(|snapshot| snapshot.id)
            .unwrap_or_else(CrdtPageId::new);
        let snapshot = crdt_snapshot_for_page(&prepared, page_id)?;
        sync.commit_page(snapshot).map_err(crdt_io_error)?;
        Ok(Some(prepared))
    }

    fn record_managed_projection(&self, path: &Path) {
        let Ok(content) = fs::read_to_string(path) else {
            return;
        };
        let rel = self.rel_path(path);
        let sync_guard = self.managed_sync.lock().unwrap();
        let Some(sync) = sync_guard.as_ref() else {
            return;
        };
        // A missing receipt is conservative (the file is treated as external),
        // not data loss. Do not turn a successful page save into a false conflict
        // if this secondary provenance write fails.
        if let Err(_error) = sync.record_projection(&rel, &content) {
            #[cfg(debug_assertions)]
            eprintln!("managed-sync projection receipt failed for {rel}: {_error}");
        }
    }

    fn commit_managed_delete(&self, path: &Path) -> io::Result<Option<(CrdtPageId, Vec<String>)>> {
        let rel = self.rel_path(path);
        let mut guard = self.managed_sync.lock().unwrap();
        let Some(sync) = guard.as_mut() else {
            return Ok(None);
        };
        if let Some(source) = sync.materialize_page(rel.as_str()).map_err(crdt_io_error)? {
            let mut candidates = Vec::new();
            for destination in sync.materialize_pages().map_err(crdt_io_error)? {
                if destination.id == source.id
                    || !self
                        .resolve_rel(&destination.path)
                        .is_some_and(|candidate| candidate.is_file())
                {
                    continue;
                }
                if let Some(promoted) = copy_promotion_snapshot(&source, &destination) {
                    candidates.push((destination.id, promoted));
                }
            }
            if candidates.len() == 1 {
                let (copy_id, promoted) = candidates.pop().unwrap();
                let report = sync
                    .promote_copy(source.id, copy_id, promoted)
                    .map_err(crdt_io_error)?;
                return Ok(Some((source.id, report.affected_paths)));
            }
        }
        match sync.delete_page(rel.as_str()) {
            Ok(_) | Err(crate::crdt::CrdtError::PageNotFound) => Ok(None),
            Err(error) => Err(crdt_io_error(error)),
        }
    }

    /// Resolve the file a save writes to, and whether it participates in the
    /// `(kind,name)` page cache. A page pinned to a specific file (`page.path` set
    /// — a duplicate-day stray, #21) writes to THAT exact file and stays OUT of the
    /// cache (the `(kind,name)` slot belongs to the canonical file; caching the
    /// stray there would make name-resolution serve it). A normal page resolves its
    /// path by name and caches as before. Errors on an invalid pinned path (escapes
    /// the graph) or a `.md`+`.org` twin (ambiguous identity, M1).
    fn save_target(&self, page: &PageDto) -> io::Result<(PathBuf, bool)> {
        if !page.path.is_empty() {
            // The page knows its own file (every loaded page carries its path).
            // Write THERE — that's how a duplicate-day stray saves to its own file
            // instead of being re-resolved by name to the canonical one. It still
            // participates in the `(kind,name)` cache UNLESS it's a shadow (a
            // title-named journal coexisting with a canonical date-stem file): a
            // shadow's cache slot belongs to the canonical, so it stays out.
            let path = self
                .resolve_rel(&page.path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid page path"))?;
            let cache = self.path_is_cacheable(&path);
            return Ok((path, cache));
        }
        // M1: refuse to write an ambiguous page (both .md and .org on disk) — we
        // can't tell which file the editor's content belongs to.
        if self.has_twin(&page.name, page.kind) {
            return Err(twin_error(&page.name));
        }
        let path = self.path_for(&page.name, page.kind);
        if !path_stays_within_root(&self.root, &path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "page path escapes graph root",
            ));
        }
        Ok((path, true))
    }

    /// Save a page, refusing to clobber an external change. If the file on disk
    /// no longer matches what Tine last knew (another app or a Syncthing pull
    /// wrote it), returns an `AlreadyExists` "conflict" error WITHOUT writing,
    /// so the caller can surface it and keep the in-memory edits.
    pub fn save_page(&self, page: &PageDto, base_rev: Option<&str>) -> io::Result<String> {
        if page.guide {
            #[cfg(debug_assertions)]
            eprintln!("attempted to persist an ephemeral bundled Guide page");
            return Ok("guide-ephemeral".into());
        }
        let (path, cache) = self.save_target(page)?;
        // Serialize against any other writer of THIS page (a PDF highlight write
        // of the same `hls__` page, or another save) for the whole
        // read→conflict-check→write→cache_upsert, so neither can clobber the other
        // or steal its self-write marker (see `page_locks`).
        let lock = self.page_lock(&path);
        let _guard = lock.lock().unwrap();
        // Single read of the current file (the conflict baseline AND the
        // formatting source AND, with the written content, the returned rev) —
        // avoids re-reading the file 2-3× per save, which is felt on NFS.
        let existing: Option<String> = match fs::read_to_string(&path) {
            Ok(disk_s) => {
                // The file must still match the exact bytes the editor loaded
                // (`base_rev`); if it changed underneath us (external edit /
                // Syncthing pull), refuse to clobber. `base_rev == None` means the
                // editor believed the page was new, so any existing file is an
                // external creation → conflict.
                if !base_rev.is_some_and(|rev| content_rev(&disk_s) == rev) {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
                Some(disk_s)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // The file is gone. If the editor had a baseline (page existed at
                // load), it was deleted externally — DON'T silently resurrect it.
                if base_rev.is_some() {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
                None
            }
            Err(e) => return Err(e), // hard error (permission, I/O) — don't write blind
        };
        // recheck = true: re-verify the file hasn't changed on disk in the instant
        // before the write, to narrow the inherent race against a NON-cooperating
        // external writer (OG/Syncthing) that doesn't take our page lock.
        // M2: write to the SAME path we locked + read the baseline from — never
        // re-resolve `path_for` under the lock (an `exists()`-probe could otherwise
        // pick a different extension if a twin appears mid-save).
        let prepared = self.commit_managed_page(page, &path)?;
        let result = self.write_page(
            prepared.as_ref().unwrap_or(page),
            &path,
            existing.as_deref(),
            true,
            cache,
        );
        if result.is_ok() && prepared.is_some() {
            self.record_managed_projection(&path);
        }
        result
    }

    /// Save a page unconditionally (the user chose "keep mine" over a conflict).
    pub fn force_save_page(&self, page: &PageDto) -> io::Result<String> {
        if page.guide {
            #[cfg(debug_assertions)]
            eprintln!("attempted to force-persist an ephemeral bundled Guide page");
            return Ok("guide-ephemeral".into());
        }
        let (path, cache) = self.save_target(page)?;
        let lock = self.page_lock(&path);
        let _guard = lock.lock().unwrap();
        // "Keep mine" resolves a content conflict, but it must not turn an I/O or
        // decoding failure into permission to overwrite unknown bytes.
        let existing = read_optional_text(&path)?;
        // recheck = false: "keep mine" overwrites unconditionally. Same locked path
        // is threaded into write_page (M2) so a forced save can't land on a twin.
        let prepared = self.commit_managed_page(page, &path)?;
        let result = self.write_page(
            prepared.as_ref().unwrap_or(page),
            &path,
            existing.as_deref(),
            false,
            cache,
        );
        if result.is_ok() && prepared.is_some() {
            self.record_managed_projection(&path);
        }
        result
    }

    /// Write a page to `path` (already resolved + locked by the caller), reproducing
    /// `existing`'s formatting, and return the new on-disk content rev (computed from
    /// what was written — no extra read).
    fn write_page(
        &self,
        page: &PageDto,
        path: &Path,
        existing: Option<&str>,
        recheck: bool,
        cache: bool,
    ) -> io::Result<String> {
        // (A new journal's `path` was named by `path_for` using the graph's
        // `:journal/file-name-format` — so custom-format graphs create the correct
        // file for the day instead of a misplaced default-named duplicate.)
        let dto_is_org = matches!(Format::from_path(path), Format::Org);
        let mut doc = Document {
            pre_block: page.pre_block.clone(),
            roots: page
                .blocks
                .iter()
                .map(|b| dto_to_doc(b, dto_is_org))
                .collect(),
        };
        // Data-preservation firewall for page-header properties (GH #163).
        // A frontend/store bug once reclassified a suffix of the page pre-block
        // as the first outline block (`A::` stayed in the header while `B::` and
        // `C::` were serialized as `- B::` / indented continuation text).  The
        // string helper used by the gear panel was correct, so helper tests could
        // not protect the actual DTO -> disk boundary.  No Tine editing command
        // intentionally moves an existing page-header property line into the
        // outline; promotion keeps property lines in the pre-block.  Refuse both
        // normal and force writes that do so, leaving the original bytes intact.
        if let Some(existing) = existing {
            // A nonempty disk preamble is authoritative. If a contradictory DTO
            // drops it while presenting a first-root header candidate, refusing
            // the save is safer than either overwriting the preamble or silently
            // keeping the candidate as a bullet. This also protects force-save.
            let existing_doc = doc::parse(existing);
            if existing_doc
                .pre_block
                .as_deref()
                .is_some_and(|pre| !pre.is_empty())
                && doc.pre_block.as_deref().unwrap_or("").is_empty()
                && first_root_is_promotable_page_header(&doc)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "refusing to drop an existing page preamble while authoring page-header properties",
                ));
            }
            if let Some(line) = newly_reclassified_page_property_line(existing, &doc) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("refusing to move page-header property into outline content: {line}"),
                ));
            }
        }
        // Match OG's pre-block serialization decision at one native boundary:
        // a genuinely headerless Markdown page may author a qualifying first
        // root through the ordinary editor, but the persisted/cache shape is an
        // unbulleted page header. Existing nonempty preambles were rejected above.
        if !dto_is_org
            && page.pre_block.as_deref().unwrap_or("").is_empty()
            && existing
                .map(doc::parse)
                .and_then(|parsed| parsed.pre_block)
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            promote_first_root_page_header(&mut doc);
        }
        // Own the caller's resolved+locked path (M2: never re-resolve path_for here).
        let path = path.to_path_buf();
        let content = match Format::from_path(&path) {
            Format::Md => {
                // Reproduce the existing file's formatting (trailing newline,
                // post-property blank line, indent) so an unchanged save is
                // byte-identical and edits produce a minimal diff — critical to
                // avoid Syncthing churn against Logseq.
                let opts = doc::SerializeOpts::detect(existing);
                let mut content = doc::serialize_with(&doc, &opts);
                // A5: if the ONLY difference from disk is whitespace trivia the
                // serializer doesn't round-trip byte-exactly (post-property
                // blank-line count, empty-bullet spelling `- ` vs `-`, indented
                // blank continuation lines), keep the existing bytes verbatim.
                // `doc::parse` collapses exactly this trivia (and ignores uuids),
                // so equal parses ⟹ the user changed nothing of substance → don't
                // rewrite (avoids needless Syncthing churn). Adopting the disk
                // bytes makes `changed` below false and the returned rev the
                // on-disk rev — so every downstream path stays correct.
                if let Some(e) = existing {
                    if e != content && doc::parse(e) == doc::parse(&content) {
                        content = e.to_string();
                    }
                }
                // CRLF preservation (shared with write_highlights). No-op saves
                // already kept the existing bytes verbatim (A5 above), so this can't
                // double-convert.
                preserve_crlf(content, existing)
            }
            Format::Org => {
                // Corruption firewall: never write a .org file Tine cannot
                // reproduce byte-for-byte. Such a page is served read-only (the
                // editor blocks edits), but defend the write path too — a stale
                // editor or a direct save must not rewrite it. The org serializer
                // is itself byte-exact (no trivia dance / CRLF rewrite needed):
                // the block bodies carry their verbatim text, including any `\r`.
                if let Some(e) = existing {
                    if !crate::org::org_editable(e) {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "org file is read-only (does not round-trip)",
                        ));
                    }
                }
                crate::org::serialize_org_detect(&doc, existing)
            }
        };
        // No-op save: identical bytes already on disk (e.g. focus/blur with no real
        // edit, or a forced flush of an unchanged page). Skip the write, the
        // watcher record, AND — crucially — the cache update below.
        let changed = existing != Some(content.as_str());
        // The shared commit protocol (marker → A3 recheck vs `existing` →
        // atomic_write); `force_save_page` passes recheck=false so "keep mine"
        // overwrites unconditionally. On a no-op, just hash the unchanged bytes for
        // the returned/cached rev — no write, no marker.
        let rev = if changed {
            self.commit_write(&path, &content, existing, recheck)?
        } else {
            content_rev(&content)
        };
        // Touch the cache only when the bytes changed, or the page isn't in an
        // already-built cache yet (fold a cold page in). A no-op save of an
        // already-cached page MUST NOT call cache_upsert: it bumps `cache_gen`,
        // which keys every memoized query/backlink/derived result — so an unchanged
        // re-save would force a whole-graph requery on every open dashboard.
        // A path-pinned save (`cache == false`, a duplicate-day stray, #21) NEVER
        // touches the `(kind,name)` cache: that slot belongs to the canonical file,
        // and folding the stray's content in would make name-resolution serve it.
        // The stray is re-parsed from disk on its next path-addressed load.
        let need_cache_update = cache
            && (changed || {
                let guard = self.cache.read().unwrap();
                guard
                    .as_ref()
                    .is_some_and(|pages| self.cached_page_index_for_path(pages, &path).is_none())
            });
        if need_cache_update {
            // For a brand-new journal, derive its date_key from the name so it's
            // recognized as a dated journal by `journals_desc` (which reads this
            // cache) — otherwise today's freshly-created page would be missing.
            let entry = self.entry_for_path(&path).unwrap_or_else(|| {
                let date_key = if page.kind == PageKind::Journal {
                    crate::date::JournalDate::from_title(&page.name).map(|d| d.ordinal_key())
                } else {
                    None
                };
                PageEntry {
                    name: page.name.clone(),
                    kind: page.kind,
                    date_key,
                    rel_path: self.rel_path(&path),
                    path: path.clone(),
                }
            });
            // H4: for org, the on-disk bytes are authoritative. If the user typed a
            // structural marker (a column-0 `* ` line, or an unbalanced #+BEGIN_)
            // into a block body, `content` re-parses to a DIFFERENT tree than the
            // frontend `doc` — cache what's actually on disk so the next load shows
            // the real structure instead of a cache that silently disagrees. Common
            // case: structures match → keep `doc` (block uuids stay stable). Markdown
            // continuation lines are indented, so they can't re-read differently.
            let cache_doc = if Format::from_path(&path) == Format::Org {
                let reparsed = crate::org::parse_org(&content);
                if reparsed == doc {
                    doc
                } else {
                    reparsed
                }
            } else {
                doc
            };
            self.cache_upsert(entry, cache_doc, rev.clone());
        }
        // Drop the self-write marker now the write is published + cached (it only
        // had to cover the atomic-write → cache_upsert window; disk_revs now
        // suppresses the watcher). See drop_self_write_marker.
        if changed {
            self.drop_self_write_marker(&path, &rev);
        }
        // The new baseline rev = hash of exactly what's now on disk (the content we
        // serialized, or the identical existing bytes on a no-op) — no re-read.
        Ok(rev)
    }
}

/// Canonical Markdown page-header property grammar mirrored from
/// `src/editor/properties.ts`. It is deliberately narrower than OG's historical
/// "first line contains `:: `" serializer heuristic, so ordinary prose/fences
/// can never be promoted accidentally.
fn page_header_property_line(line: &str) -> Option<(&str, &str)> {
    static KEY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let (key, value) = line.split_once("::")?;
    if key.is_empty() || key.starts_with('#') {
        return None;
    }
    let valid = KEY
        .get_or_init(|| regex::Regex::new(r"^[\p{L}\p{M}\p{N}_./-]+$").unwrap())
        .is_match(key);
    valid.then_some((key, value))
}

fn page_header_properties_only(raw: &str) -> bool {
    if raw.is_empty() || raw.starts_with('\n') || raw.ends_with('\n') {
        return false;
    }
    let mut saw_property = false;
    for line in raw.split('\n') {
        if line.is_empty() {
            if !saw_property {
                return false;
            }
            continue;
        }
        if page_header_property_line(line).is_none() {
            return false;
        }
        saw_property = true;
    }
    saw_property
}

fn first_root_is_promotable_page_header(doc: &Document) -> bool {
    let Some(first) = doc.roots.first() else {
        return false;
    };
    first.children.is_empty()
        && page_header_properties_only(&first.raw)
        && !first.raw.split('\n').any(|line| {
            page_header_property_line(line).is_some_and(|(key, _)| key.eq_ignore_ascii_case("id"))
        })
}

fn promote_first_root_page_header(doc: &mut Document) {
    if !first_root_is_promotable_page_header(doc) {
        return;
    }
    let first = doc.roots.remove(0);
    doc.pre_block = Some(first.raw);
}

/// Return the first property-shaped outline line that has no outline provenance
/// on disk while the proposal also loses page-header property slots.
///
/// The firewall is deliberately structural rather than an exact-string test:
/// a broken DTO must not evade it by editing the moved line's key/value. At the
/// same time, a property-shaped outline block that genuinely existed on disk is
/// allowed to stay, move, or be edited. We therefore treat existing outline
/// property lines as provenance slots: exact multiset matches consume their
/// original slots first, and remaining slots cover ordinary edits. Only an
/// excess proposed outline line is newly unproven. There is no implicit repair;
/// contradictory structure is rejected before bytes or cache can change.
fn newly_reclassified_page_property_line(existing: &str, proposed: &Document) -> Option<String> {
    // The general data-preservation guard is intentionally a little broader
    // than Tine's editable property grammar: Logseq graphs can contain Unicode
    // or plugin-defined keys that Tine does not expose in its settings panel,
    // but they still must never be reclassified into outline content.
    fn page_header_property(line: &str) -> bool {
        let Some((key, _)) = line.split_once("::") else {
            return false;
        };
        let key = key.trim();
        !key.is_empty() && key.chars().all(|ch| !ch.is_whitespace() && ch != ':')
    }

    fn pre_property_lines(raw: Option<&str>) -> Vec<&str> {
        raw.unwrap_or("")
            .split('\n')
            .filter(|line| page_header_property(line))
            .collect()
    }

    fn outline_property_lines<'a>(blocks: &'a [DocBlock], out: &mut Vec<&'a str>) {
        for block in blocks {
            out.extend(
                block
                    .raw
                    .split('\n')
                    .filter(|line| page_header_property(line)),
            );
            outline_property_lines(&block.children, out);
        }
    }

    let existing_doc = doc::parse(existing);
    let existing_pre = pre_property_lines(existing_doc.pre_block.as_deref());
    let proposed_pre = pre_property_lines(proposed.pre_block.as_deref());
    if proposed_pre.len() >= existing_pre.len() {
        return None;
    }

    let mut existing_outline = Vec::new();
    outline_property_lines(&existing_doc.roots, &mut existing_outline);
    let mut proposed_outline = Vec::new();
    outline_property_lines(&proposed.roots, &mut proposed_outline);
    if proposed_outline.len() <= existing_outline.len() {
        return None;
    }

    // Cancel exact matches as a multiset so the diagnostic identifies a truly
    // excess proposal line even in the presence of duplicates. Any remaining
    // existing slots then cover changed/reordered pre-existing outline lines.
    let mut exact_slots: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in &existing_outline {
        *exact_slots.entry(*line).or_default() += 1;
    }
    let mut unmatched = Vec::new();
    let mut exact_matches = 0usize;
    for line in proposed_outline {
        match exact_slots.get_mut(line) {
            Some(count) if *count > 0 => {
                *count -= 1;
                exact_matches += 1;
            }
            _ => unmatched.push(line),
        }
    }
    let edited_provenance_slots = existing_outline.len() - exact_matches;
    unmatched
        .get(edited_provenance_slots)
        .map(|line| (*line).to_string())
}

/// Atomically reserve a unique filename in `assets/` for `name`, de-duplicating
/// against existing files by appending `_1`, `_2`, … to the stem. Unlike a plain
/// `exists()` check followed by a write, this CREATES the file exclusively
/// (`create_new`), so a concurrent writer (OG Logseq, or another asset op) that
/// races between the name check and our write can't claim the same name and get
/// silently overwritten — whoever loses the create retries the next candidate.
/// Returns the chosen name and the open (empty) file handle.
/// Reject an asset name that isn't a plain top-level filename — a path separator
/// or a `.`/`..` component — so a frontend-supplied name can't reach outside
/// `assets/` (defense-in-depth; mirrors `trash_asset`). `create_new` already
/// blocks overwriting an existing file, so the realistic pre-guard outcome was a
/// stray file, not corruption — but reject it outright anyway.
fn top_level_asset_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad asset name",
        ));
    }
    Ok(())
}

/// Preserve a file's CRLF line endings on re-write: if `existing` used Windows
/// endings and the freshly-serialized `content` is all-LF, convert it back, so a
/// real edit produces a minimal diff instead of flipping every line (Syncthing
/// churn vs a Windows editor). New files stay LF. Shared by write_page +
/// write_highlights so the two can't drift on it.
fn serialize_pdf_hls_page(
    path: &Path,
    document: &Document,
    existing: Option<&str>,
) -> io::Result<String> {
    match Format::from_path(path) {
        Format::Md => Ok(preserve_crlf(doc::serialize(document), existing)),
        Format::Org => {
            if existing.is_some_and(|raw| !crate::org::org_editable(raw)) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "org highlight page is read-only (does not round-trip)",
                ));
            }
            Ok(crate::org::serialize_org_detect(document, existing))
        }
    }
}

fn preserve_crlf(content: String, existing: Option<&str>) -> String {
    if existing.is_some_and(|e| e.contains("\r\n")) && !content.contains('\r') {
        content.replace('\n', "\r\n")
    } else {
        content
    }
}

/// Read an optional UTF-8 text file without conflating "missing" with "could not
/// safely read". Mutation paths use this for their baselines: only NotFound may
/// become `None`; every other error must stop the write.
fn read_optional_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn validate_highlight_edn(raw: &str) -> io::Result<()> {
    if raw.trim().is_empty() {
        return Ok(());
    }
    if matches!(crate::edn::parse_strict(raw), Some(crate::edn::Edn::Map(_))) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "highlight sidecar is malformed; refusing to replace it",
        ))
    }
}

/// Parse one page under a page-sized unwind boundary. lsdoc deliberately panics
/// when its v2 parser does not own an input shape; isolating here preserves that
/// loud guard while limiting the search-cache blast radius to this page.
fn parse_page_entry_isolated(e: PageEntry) -> PageParseResult {
    isolate_page_parse(e, |entry| {
        let content = fs::read_to_string(&entry.path).ok()?;
        Some(parse_page_content(entry, &content))
    })
}

fn parse_page_content(e: &PageEntry, content: &str) -> (Document, String) {
    let rev = content_rev(&content);
    let mut d = parse_doc(&e.path, &content);
    #[cfg(test)]
    if content.contains(TEST_PAGE_PARSE_PANIC_SENTINEL) {
        panic!("deterministic test sentinel for a page projection panic");
    }
    assign_doc_runtime_ids(&mut d.roots, &e.rel_path);
    (d, rev)
}

fn isolate_page_parse(
    e: PageEntry,
    parse: impl FnOnce(&PageEntry) -> Option<(Document, String)>,
) -> PageParseResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(&e))) {
        Ok(Some((doc, rev))) => Ok(Some((e, doc, rev))),
        Ok(None) => Ok(None),
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic payload");
            eprintln!(
                "Tine search index skipped page {:?}: page parse/projection panicked: {detail}",
                e.rel_path
            );
            Err(e.rel_path)
        }
    }
}

fn page_cache_worker_count() -> usize {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8);
    #[cfg(test)]
    let workers = workers.max(2);
    workers
}

#[cfg(test)]
const TEST_PAGE_PARSE_PANIC_SENTINEL: &str = "__TINE_TEST_PAGE_PARSE_PANIC__";

/// Compound asset extensions that a downstream matcher keys on AS A WHOLE (e.g.
/// drawio's editable SVG, whose `.drawio.svg` suffix is what surfaces the
/// "Edit in draw.io" affordance). De-dup must insert its `_N` counter BEFORE the
/// whole suffix — `flow.drawio.svg` must collide to `flow_1.drawio.svg`, NOT
/// `flow.drawio_1.svg` (a naive last-dot split), which would still end in `.svg`
/// but no longer match `\.drawio\.svg$` and silently lose the editor button
/// (GH #38). Longest match wins; case-insensitive.
const COMPOUND_ASSET_EXTS: &[&str] = &[".drawio.svg", ".excalidraw.svg", ".excalidraw.png"];

/// Split an asset filename into (stem, extension) for de-dup counter insertion,
/// preserving known compound extensions (see `COMPOUND_ASSET_EXTS`). Falls back
/// to a last-dot split for ordinary single extensions.
fn split_asset_stem_ext(name: &str) -> (String, String) {
    let lower = name.to_ascii_lowercase();
    for ext in COMPOUND_ASSET_EXTS {
        if lower.ends_with(ext) {
            let cut = name.len() - ext.len();
            return (name[..cut].to_string(), name[cut..].to_string());
        }
    }
    match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    }
}

#[cfg(test)]
fn reserve_asset(assets: &Path, name: &str) -> io::Result<(String, fs::File)> {
    top_level_asset_name(name)?;
    let create_new = |n: &str| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(assets.join(n))
    };
    match create_new(name) {
        Ok(f) => return Ok((name.to_string(), f)),
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    let (stem, ext) = split_asset_stem_ext(name);
    let mut i = 1;
    loop {
        let candidate = format!("{stem}_{i}{ext}");
        match create_new(&candidate) {
            Ok(f) => return Ok((candidate, f)),
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
            _ => i += 1,
        }
    }
}

/// Collapse journal entries that resolve to the SAME date down to one (the
/// canonical `yyyy_MM_dd` file) — a leftover title-named duplicate must not show
/// the day twice in the feed, quick-switch, or All-Pages. Non-journal entries and
/// the input order are preserved.
fn dedup_journal_days(entries: Vec<PageEntry>) -> Vec<PageEntry> {
    let is_canonical = |e: &PageEntry| {
        e.path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| JournalDate::from_file_stem(s).is_some())
    };
    let mut idx_of: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut out: Vec<PageEntry> = Vec::new();
    for e in entries {
        match e.date_key {
            Some(k) if e.kind == PageKind::Journal => {
                if let Some(&i) = idx_of.get(&k) {
                    if is_canonical(&e) && !is_canonical(&out[i]) {
                        out[i] = e;
                    }
                } else {
                    idx_of.insert(k, out.len());
                    out.push(e);
                }
            }
            _ => out.push(e),
        }
    }
    out
}

#[cfg(test)]
thread_local! {
    static LIST_MD_CALLS: std::cell::Cell<usize> = std::cell::Cell::new(0);
    static CACHE_LINEAR_SCAN_STEPS: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

#[cfg(test)]
fn count_cache_linear_scan(n: usize) {
    CACHE_LINEAR_SCAN_STEPS.with(|steps| steps.set(steps.get() + n));
}

fn list_md(
    dir: &Path,
    kind: PageKind,
    fmt: &JournalFormat,
    name_fmt: FileNameFormat,
    rel_dir: &str,
) -> Vec<PageEntry> {
    #[cfg(test)]
    LIST_MD_CALLS.with(|calls| calls.set(calls.get() + 1));

    let mut out = Vec::new();
    walk_page_files(dir, |path| {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            return;
        };
        if is_sync_conflict(stem) {
            return; // sync-tool conflict copy — not a page (see list_sync_conflicts)
        }
        let (name, date_key) = match kind {
            PageKind::Journal => match fmt.parse(stem) {
                Some(d) => (fmt.title(d), Some(d.ordinal_key())),
                None => (stem.to_string(), None),
            },
            PageKind::Page => (decode_page_name(stem, name_fmt), None),
        };
        out.push(PageEntry {
            name,
            kind,
            date_key,
            rel_path: rel_under_dir(rel_dir, dir, &path),
            path,
        });
    });
    out
}

fn walk_page_files(dir: &Path, mut visit: impl FnMut(PathBuf)) {
    // Descend into sub-directories (#21). Logseq scans the whole graph root
    // recursively, so a page archived under `pages/client-a/foo.md` is a real
    // page — keyed by its BASENAME (`foo`); the sub-path is discarded, matching
    // OG's `path->file-name` (the file's own `path` stays its load/save identity).
    // One stack-based walk, O(files), no re-scan.
    //
    // `file_type()` does not follow symlinks. Check it for page-looking entries
    // too: otherwise `pages/secret.md -> /outside/secret.md` would be indexed and
    // exposed. Hidden dirs (`.git` &c.) are skipped — never a page store.
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if is_page_file(&path) && file_type.is_file() {
                visit(path);
                continue;
            }
            // Non-page entry: recurse if it's a real (non-symlink, non-hidden)
            // sub-directory. This is the only stat we pay, and never on the hot
            // page-file path above.
            let hidden = path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with('.'))
                .unwrap_or(true);
            if !hidden && file_type.is_dir() {
                stack.push(path);
            }
        }
    }
}

/// Union of two markdown page-property pre-blocks: keep `mine` and append any
/// `key:: value` line `theirs` defines that `mine` doesn't (mine wins on a clash),
/// so a sync-conflict resolve doesn't silently drop the other device's
/// `alias::`/`tags::`/`icon::`. Free text in `theirs`' pre-block is dropped (rare;
/// the conflict copy is trashed-recoverable). Mirrors the property-carry in
/// [`Graph::merge_pages`].
fn union_pre(mine: Option<&str>, theirs: Option<&str>) -> Option<String> {
    let mine = mine.unwrap_or("");
    let Some(theirs) = theirs else {
        return (!mine.is_empty()).then(|| mine.to_string());
    };
    let mine_keys: std::collections::HashSet<String> = mine
        .lines()
        .filter_map(|l| doc::parse_property_line(l).map(|(k, _)| k.to_ascii_lowercase()))
        .collect();
    let extra: Vec<&str> = theirs
        .lines()
        .filter(|l| {
            doc::parse_property_line(l)
                .is_some_and(|(k, _)| !mine_keys.contains(&k.to_ascii_lowercase()))
        })
        .collect();
    if extra.is_empty() {
        return (!mine.is_empty()).then(|| mine.to_string());
    }
    let mut pre = mine.to_string();
    if !pre.is_empty() && !pre.ends_with('\n') {
        pre.push('\n');
    }
    pre.push_str(&extra.join("\n"));
    Some(pre)
}

/// True if any block in the subtree has a non-empty line that isn't a `key::`
/// property line — i.e. the page is more than an empty/placeholder bullet.
fn doc_has_content(blocks: &[DocBlock]) -> bool {
    blocks.iter().any(|b| {
        b.raw
            .lines()
            .any(|l| !l.trim().is_empty() && crate::doc::parse_property_line(l).is_none())
            || doc_has_content(&b.children)
    })
}

/// Versioned namespace for file-mode runtime block locators. These UUIDs are
/// store/UI keys only: persisted `id::` remains the external `((id))` identity.
const FILE_BLOCK_RUNTIME_NAMESPACE_V1: Uuid =
    Uuid::from_u128(0x1e0c_5a13_9b42_5da4_a73c_0be5_8f6a_2320);

fn normalized_runtime_owner(owner: &str) -> String {
    let owner = owner.replace('\\', "/");
    let mut parts = Vec::new();
    for part in owner.split('/') {
        match part {
            "" | "." => {}
            ".." => panic!("runtime identity owner must be graph-relative"),
            _ => parts.push(part),
        }
    }
    assert!(
        !parts.is_empty(),
        "runtime identity owner must not be empty"
    );
    parts.join("/")
}

fn deterministic_runtime_uuid(namespace: Uuid, name: &[u8]) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name);
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 variant + version 8 (application-defined deterministic UUID).
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn runtime_owner_namespace(domain: &str, owner: &str) -> Uuid {
    let owner = normalized_runtime_owner(owner);
    let mut name = Vec::with_capacity(domain.len() + owner.len() + 16);
    name.extend_from_slice(&(domain.len() as u64).to_be_bytes());
    name.extend_from_slice(domain.as_bytes());
    name.extend_from_slice(&(owner.len() as u64).to_be_bytes());
    name.extend_from_slice(owner.as_bytes());
    deterministic_runtime_uuid(FILE_BLOCK_RUNTIME_NAMESPACE_V1, &name)
}

fn assign_runtime_ids_rec(blocks: &mut [DocBlock], parent: Uuid) {
    for (sibling_index, block) in blocks.iter_mut().enumerate() {
        // Hierarchical derivation is equivalent to hashing the full sibling-index
        // path, while doing constant work per node (O(blocks)).
        let structural = deterministic_runtime_uuid(parent, &(sibling_index as u64).to_be_bytes());
        if block.uuid.is_empty() {
            block.uuid = structural.to_string();
        }
        assign_runtime_ids_rec(&mut block.children, structural);
    }
}

/// Seed missing runtime keys for a graph-backed document from its normalized,
/// graph-relative physical owner. Existing live keys survive ordinary saves.
pub fn assign_doc_runtime_ids(roots: &mut [DocBlock], owner_rel_path: &str) {
    let owner = runtime_owner_namespace("file-block-runtime-v1", owner_rel_path);
    assign_runtime_ids_rec(roots, owner);
}

fn assign_virtual_doc_runtime_ids(roots: &mut [DocBlock], domain: &str, owner: &str) {
    let owner = runtime_owner_namespace(domain, owner);
    assign_runtime_ids_rec(roots, owner);
}

fn block_runtime_id(b: &DocBlock) -> String {
    assert!(
        !b.uuid.is_empty(),
        "DocBlock must have an explicit runtime owner before DTO projection"
    );
    b.uuid.clone()
}

/// Convert a parsed (cached) block to a DTO, carrying its stable uuid as the id.
pub fn block_to_dto(b: &DocBlock) -> BlockDto {
    BlockDto {
        id: block_runtime_id(b),
        raw: b.raw.clone(),
        collapsed: b.collapsed(),
        children: b.children.iter().map(block_to_dto).collect(),
        breadcrumb: Vec::new(),
        page_property: false,
        // All header facets off the one lsdoc projection (marker/priority/heading/
        // properties/scheduled/deadline) — priority is header-position only, matching
        // the chip, so a loaded block never shows a priority the edit path wouldn't.
        marker: b.marker().map(str::to_string),
        priority: b.priority().map(str::to_string),
        heading_level: b.heading_level(),
        scheduled: b.scheduled().map(str::to_string),
        deadline: b.deadline().map(str::to_string),
        tags: b.tags(),
        properties: b.properties(),
    }
}

/// Convert one block to the result-row wire shape. Result membership is about
/// block identity, raw text, and facets; descendants belong to the source page
/// and are hydrated once per page by live consumers. Keeping this constructor
/// separate makes it difficult to accidentally reintroduce overlapping subtree
/// amplification in queries, references, search, or batched resolution.
pub fn block_to_shallow_dto(b: &DocBlock) -> BlockDto {
    BlockDto {
        id: block_runtime_id(b),
        raw: b.raw.clone(),
        collapsed: b.collapsed(),
        children: Vec::new(),
        breadcrumb: Vec::new(),
        page_property: false,
        marker: b.marker().map(str::to_string),
        priority: b.priority().map(str::to_string),
        heading_level: b.heading_level(),
        scheduled: b.scheduled().map(str::to_string),
        deadline: b.deadline().map(str::to_string),
        tags: b.tags(),
        properties: b.properties(),
    }
}

/// Convert a frontend DTO subtree back to a doc block, preserving the frontend's
/// block id as the node uuid so the cache and the frontend agree on identity.
fn dto_to_doc(b: &BlockDto, is_org: bool) -> DocBlock {
    DocBlock {
        raw: b.raw.clone(),
        children: b.children.iter().map(|c| dto_to_doc(c, is_org)).collect(),
        uuid: b.id.clone(),
        is_org,
        proj: std::sync::OnceLock::new(),
    }
}

fn persisted_sync_id(raw: &str, format: Format) -> Option<String> {
    let mut block = DocBlock::new(raw);
    block.is_org = format == Format::Org;
    block.property("id").filter(|id| !id.is_empty())
}

fn count_missing_sync_ids(blocks: &[BlockDto], format: Format) -> usize {
    blocks
        .iter()
        .map(|block| {
            usize::from(persisted_sync_id(&block.raw, format).is_none())
                + count_missing_sync_ids(&block.children, format)
        })
        .sum()
}

fn validate_graph_sync_ids(
    blocks: &[BlockDto],
    format: Format,
    seen: &mut std::collections::HashSet<String>,
) -> io::Result<()> {
    for block in blocks {
        let id = persisted_sync_id(&block.raw, format).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "managed-sync block has no persisted id",
            )
        })?;
        if Uuid::parse_str(&id).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("persisted block id {id} is not a UUID"),
            ));
        }
        if !seen.insert(id.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("duplicate persisted block id {id} across graph"),
            ));
        }
        validate_graph_sync_ids(&block.children, format, seen)?;
    }
    Ok(())
}

fn org_raw_with_sync_id(raw: &str, id: &str) -> String {
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let mut offset = 0usize;
    let mut in_drawer = false;
    for segment in raw.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case(":PROPERTIES:") {
            in_drawer = true;
        } else if in_drawer && trimmed.eq_ignore_ascii_case(":END:") {
            let indent_len = line.len() - line.trim_start().len();
            let indent = &line[..indent_len];
            let mut out = String::with_capacity(raw.len() + id.len() + 7);
            out.push_str(&raw[..offset]);
            out.push_str(indent);
            out.push_str(":ID: ");
            out.push_str(id);
            out.push_str(newline);
            out.push_str(&raw[offset..]);
            return out;
        }
        offset += segment.len();
    }

    let separator = if raw.is_empty() || raw.ends_with('\n') {
        ""
    } else {
        newline
    };
    format!("{raw}{separator}:PROPERTIES:{newline}:ID: {id}{newline}:END:")
}

fn raw_with_sync_id(raw: &str, id: &str, format: Format) -> String {
    if format == Format::Org {
        return org_raw_with_sync_id(raw, id);
    }
    let separator = if raw.is_empty() || raw.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("{raw}{separator}id:: {id}")
}

fn persist_block_sync_ids(
    block: &mut BlockDto,
    format: Format,
    seen: &mut std::collections::HashSet<String>,
) -> io::Result<()> {
    let persisted = persisted_sync_id(&block.raw, format);
    let id = match persisted {
        Some(id) => id,
        None => {
            let id = Uuid::new_v4().to_string();
            block.raw = raw_with_sync_id(&block.raw, &id, format);
            id
        }
    };
    if !seen.insert(id.clone()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("duplicate persisted block id {id}"),
        ));
    }
    // `BlockDto.id` is a structural runtime locator owned by the page/path. The
    // persisted id is the cross-device identity and stays exclusively in `raw`.
    for child in &mut block.children {
        persist_block_sync_ids(child, format, seen)?;
    }
    Ok(())
}

fn page_with_persisted_sync_ids(page: &PageDto) -> io::Result<PageDto> {
    let mut page = page.clone();
    let mut seen = std::collections::HashSet::new();
    for block in &mut page.blocks {
        persist_block_sync_ids(block, page.format, &mut seen)?;
    }
    Ok(page)
}

fn collect_persisted_sync_ids(
    blocks: &[BlockDto],
    format: Format,
    output: &mut std::collections::HashSet<String>,
) {
    for block in blocks {
        if let Some(id) = persisted_sync_id(&block.raw, format) {
            output.insert(id);
        }
        collect_persisted_sync_ids(&block.children, format, output);
    }
}

fn replace_persisted_sync_id(raw: &str, old: &str, new: &str, format: Format) -> String {
    let mut output = String::with_capacity(raw.len() + new.len().saturating_sub(old.len()));
    let mut replaced = false;
    for segment in raw.split_inclusive('\n') {
        let (line, ending) = segment
            .strip_suffix("\r\n")
            .map(|line| (line, "\r\n"))
            .or_else(|| segment.strip_suffix('\n').map(|line| (line, "\n")))
            .unwrap_or((segment, ""));
        let delimiter = if format == Format::Org {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix(':')
                .and_then(|rest| {
                    rest.find(':')
                        .map(|index| line.len() - rest.len() + index + 1)
                })
                .filter(|index| line[..*index].trim_matches(':').eq_ignore_ascii_case("id"))
        } else {
            line.find("::")
                .filter(|index| line[..*index].trim().eq_ignore_ascii_case("id"))
                .map(|index| index + 2)
        };
        if !replaced && delimiter.is_some_and(|index| line[index..].trim() == old) {
            let index = delimiter.unwrap();
            output.push_str(&line[..index]);
            output.push(' ');
            output.push_str(new);
            output.push_str(ending);
            replaced = true;
        } else {
            output.push_str(segment);
        }
    }
    debug_assert!(replaced, "persisted id selected for replacement must exist");
    output
}

fn rekey_conflicting_sync_ids(
    blocks: &mut [BlockDto],
    format: Format,
    conflicts: &std::collections::HashSet<String>,
) {
    for block in blocks {
        if let Some(id) = persisted_sync_id(&block.raw, format) {
            if conflicts.contains(&id) {
                block.raw =
                    replace_persisted_sync_id(&block.raw, &id, &Uuid::new_v4().to_string(), format);
            }
        }
        rekey_conflicting_sync_ids(&mut block.children, format, conflicts);
    }
}

fn crdt_io_error(error: crate::crdt::CrdtError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error)
}

fn flatten_crdt_blocks(
    blocks: &[BlockDto],
    format: Format,
    parent: Option<CrdtBlockId>,
    output: &mut Vec<CrdtBlockSnapshot>,
) -> io::Result<()> {
    for (order, block) in blocks.iter().enumerate() {
        let persisted = persisted_sync_id(&block.raw, format).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "managed-sync block has no persisted id",
            )
        })?;
        let uuid = Uuid::parse_str(&persisted).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("persisted block id {persisted} is not a UUID"),
            )
        })?;
        let id = CrdtBlockId::from_uuid(uuid);
        output.push(CrdtBlockSnapshot {
            id,
            parent,
            order: u32::try_from(order)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "too many siblings"))?,
            raw: block.raw.clone(),
        });
        flatten_crdt_blocks(&block.children, format, Some(id), output)?;
    }
    Ok(())
}

fn crdt_snapshot_for_page(page: &PageDto, id: CrdtPageId) -> io::Result<CrdtPageSnapshot> {
    if page.path.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "managed-sync page snapshot has no storage path",
        ));
    }
    let mut blocks = Vec::new();
    flatten_crdt_blocks(&page.blocks, page.format, None, &mut blocks)?;
    Ok(CrdtPageSnapshot {
        id,
        path: page.path.clone(),
        name: page.name.clone(),
        kind: match page.kind {
            PageKind::Page => "page",
            PageKind::Journal => "journal",
        }
        .into(),
        format: page.format.ext().into(),
        pre_block: page.pre_block.clone(),
        blocks,
    })
}

fn copy_promotion_snapshot(
    source: &CrdtPageSnapshot,
    destination: &CrdtPageSnapshot,
) -> Option<CrdtPageSnapshot> {
    if source.kind != destination.kind
        || source.format != destination.format
        || source.pre_block != destination.pre_block
    {
        return None;
    }
    let format = match source.format.as_str() {
        "md" => Format::Md,
        "org" => Format::Org,
        _ => return None,
    };
    let source_blocks: std::collections::HashMap<_, _> = source
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect();
    let destination_blocks: std::collections::HashMap<_, _> = destination
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect();
    let mut id_map = std::collections::HashMap::new();

    fn pair_children(
        source_parent: Option<CrdtBlockId>,
        destination_parent: Option<CrdtBlockId>,
        source: &std::collections::HashMap<CrdtBlockId, &CrdtBlockSnapshot>,
        destination: &std::collections::HashMap<CrdtBlockId, &CrdtBlockSnapshot>,
        format: Format,
        id_map: &mut std::collections::HashMap<CrdtBlockId, CrdtBlockId>,
    ) -> bool {
        let mut source_children: Vec<_> = source
            .values()
            .copied()
            .filter(|block| block.parent == source_parent)
            .collect();
        let mut destination_children: Vec<_> = destination
            .values()
            .copied()
            .filter(|block| block.parent == destination_parent)
            .collect();
        source_children.sort_by_key(|block| (block.order, block.id));
        destination_children.sort_by_key(|block| (block.order, block.id));
        if source_children.len() != destination_children.len() {
            return false;
        }
        for (source_block, destination_block) in
            source_children.into_iter().zip(destination_children)
        {
            let marker = Uuid::nil().to_string();
            let source_raw = replace_persisted_sync_id(
                &source_block.raw,
                &source_block.id.to_string(),
                &marker,
                format,
            );
            let destination_raw = replace_persisted_sync_id(
                &destination_block.raw,
                &destination_block.id.to_string(),
                &marker,
                format,
            );
            if source_block.order != destination_block.order || source_raw != destination_raw {
                return false;
            }
            id_map.insert(destination_block.id, source_block.id);
            if !pair_children(
                Some(source_block.id),
                Some(destination_block.id),
                source,
                destination,
                format,
                id_map,
            ) {
                return false;
            }
        }
        true
    }

    if !pair_children(
        None,
        None,
        &source_blocks,
        &destination_blocks,
        format,
        &mut id_map,
    ) || id_map.len() != source.blocks.len()
        || id_map.len() != destination.blocks.len()
    {
        return None;
    }

    let mut promoted = destination.clone();
    promoted.id = source.id;
    for block in &mut promoted.blocks {
        let old = block.id;
        let new = *id_map.get(&old)?;
        block.id = new;
        block.parent = block.parent.and_then(|parent| id_map.get(&parent).copied());
        block.raw =
            replace_persisted_sync_id(&block.raw, &old.to_string(), &new.to_string(), format);
    }
    Some(promoted)
}

fn dto_blocks_from_crdt(
    parent: Option<CrdtBlockId>,
    blocks: &std::collections::HashMap<CrdtBlockId, &CrdtBlockSnapshot>,
    is_org: bool,
) -> io::Result<Vec<DocBlock>> {
    let mut children: Vec<&CrdtBlockSnapshot> = blocks
        .values()
        .copied()
        .filter(|block| block.parent == parent)
        .collect();
    children.sort_by_key(|block| (block.order, block.id));
    children
        .into_iter()
        .map(|block| {
            let doc = DocBlock {
                raw: block.raw.clone(),
                children: dto_blocks_from_crdt(Some(block.id), blocks, is_org)?,
                uuid: String::new(),
                is_org,
                proj: std::sync::OnceLock::new(),
            };
            Ok(doc)
        })
        .collect()
}

fn page_dto_from_crdt(snapshot: &CrdtPageSnapshot) -> io::Result<PageDto> {
    let kind = match snapshot.kind.as_str() {
        "page" => PageKind::Page,
        "journal" => PageKind::Journal,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown managed-sync page kind {other}"),
            ))
        }
    };
    let format = match snapshot.format.as_str() {
        "md" => Format::Md,
        "org" => Format::Org,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown managed-sync page format {other}"),
            ))
        }
    };
    let blocks: std::collections::HashMap<_, _> = snapshot
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect();
    let mut doc_blocks = dto_blocks_from_crdt(None, &blocks, format == Format::Org)?;
    assign_doc_runtime_ids(&mut doc_blocks, &snapshot.path);
    Ok(PageDto {
        name: snapshot.name.clone(),
        kind,
        title: snapshot.name.clone(),
        pre_block: snapshot.pre_block.clone(),
        blocks: doc_blocks.iter().map(block_to_dto).collect(),
        rev: None,
        format,
        read_only: false,
        path: snapshot.path.clone(),
        guide: false,
    })
}

/// Build a page DTO from a cached document. `read_only` is left false here (the
/// on-disk bytes aren't known at this point); `load_page` sets it from the file
/// it reads.
fn page_dto(entry: &PageEntry, doc: &Document) -> PageDto {
    PageDto {
        name: entry.name.clone(),
        kind: entry.kind,
        title: entry.name.clone(),
        pre_block: doc.pre_block.clone(),
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: Format::from_path(&entry.path),
        read_only: false,
        path: String::new(),
        guide: false,
    }
}

/// Build a Markdown page DTO from raw Logseq Markdown without touching disk.
/// Used by the bundled in-app Guide so it reuses the same document parser and
/// DTO projection as normal graph pages.
pub fn markdown_page_dto(name: &str, title: &str, markdown: &str) -> PageDto {
    let mut doc = doc::parse(markdown);
    assign_virtual_doc_runtime_ids(&mut doc.roots, "bundled-markdown-v1", name);
    PageDto {
        name: name.to_string(),
        kind: PageKind::Page,
        title: title.to_string(),
        pre_block: doc.pre_block.clone(),
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    }
}

/// Whether a page should load read-only: an org file whose on-disk bytes don't
/// round-trip through Tine's org parser/serializer, so Tine must never rewrite
/// it (lest it corrupt the user's graph). Markdown pages are always editable.
fn read_only_org(path: &Path, content: &str) -> bool {
    Format::from_path(path) == Format::Org && !crate::org::org_editable(content)
}

/// A page's `icon::` property value from its pre-block, handling markdown
/// (`icon:: 🏁`), org property drawers (`:icon: 🏁`) and org `#+ICON:` directives.
/// None if absent or blank.
fn pre_block_icon(pre: &str) -> Option<String> {
    for line in pre.lines() {
        // Markdown `icon:: value` (single shared parser; needs the `::`).
        if let Some((k, v)) = crate::doc::parse_property_line(line) {
            let v = v.trim();
            if k.eq_ignore_ascii_case("icon") && !v.is_empty() {
                return Some(v.to_string());
            }
        }
        let t = line.trim();
        // Org property drawer `:icon: value` or directive `#+ICON: value`.
        for stripped in [t.strip_prefix(':'), t.strip_prefix("#+")]
            .into_iter()
            .flatten()
        {
            if let Some(idx) = stripped.find(':') {
                let (k, v) = (&stripped[..idx], stripped[idx + 1..].trim());
                if k.eq_ignore_ascii_case("icon") && !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Stable (deterministic, seed-free) content hash — FNV-1a/64 as hex. Used as a
/// per-load baseline so a save can detect that the file changed underneath the
/// editor. Deterministic so a rev returned from one save matches the next read.
pub fn content_rev(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Encode a page name to its on-disk filename stem, honoring the graph's
/// `:file/name-format` (so Tine round-trips namespaces with OG on BOTH legacy
/// `%2F` graphs and modern `___` graphs). Mirrors OG's `legacy-url-file-name-sanity`
/// (legacy) and `tri-lb-file-name-sanity`/`escape-namespace-slashes-and-multilowbars`
/// (triple-lowbar) for the high-frequency case: the namespace `/` separator and
/// the `_`-adjacency / literal-`___` disambiguation. Exotic reserved-char and
/// Windows-reserved-name rules are not yet mirrored (rare).
fn encode_page_name(name: &str, fmt: FileNameFormat) -> String {
    match fmt {
        FileNameFormat::Legacy => name.replace('/', "%2F"),
        FileNameFormat::TripleLowbar => name
            // Disambiguate underscores that would otherwise be ambiguous after
            // `/`→`___` (OG `fs.cljs:99-103`), THEN map the separator.
            .replace("___", "%5F%5F%5F")
            .replace("_/", "%5F/")
            .replace("/_", "/%5F")
            .replace('/', "___"),
    }
}

/// Inverse of [`encode_page_name`]. Legacy: percent-decode (`%2F`→`/`).
/// Triple-lowbar: `___`→`/` FIRST, then percent-decode — the OG order
/// (`util.cljs:153-160`), so an encoded literal `___` (stored `%5F%5F%5F`)
/// survives instead of being turned into a separator.
fn decode_page_name(stem: &str, fmt: FileNameFormat) -> String {
    match fmt {
        FileNameFormat::Legacy => percent_decode(stem),
        FileNameFormat::TripleLowbar => percent_decode(&stem.replace("___", "/")),
    }
}

/// Decode `%XX` percent-escapes (UTF-8 aware, like JS `decodeURIComponent`). An
/// invalid or truncated escape is left literal rather than dropped.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex_nibble(b[i + 1]), hex_nibble(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// A unique-ish label (epoch millis + process-local sequence) for trashed files,
/// so deleting two pages with the same name doesn't collide in the trash.
/// Collect every `assets/<name>` reference in `text` into `into`. Captures both
/// markdown (`![](../assets/x.png)`, `[f](../assets/x.pdf)`) and org
/// (`[[file:../assets/x.png]]`) forms — the name runs from after `assets/` to the
/// next markup closer (`)`/`]`/quote/etc.) or line break. Crucially it does NOT
/// stop at a space, so a referenced filename containing spaces is matched in full
/// (mis-truncating it would make `orphan_assets` flag a file that IS in use). The
/// first path segment is added too, so a PDF area-image ref (`assets/<key>/p.png`)
/// marks `<key>` as in use.
fn collect_asset_refs(text: &str, into: &mut std::collections::HashSet<String>) {
    let mut rest = text;
    while let Some(i) = rest.find("assets/") {
        let after = &rest[i + "assets/".len()..];
        let end = after
            .find(|c: char| {
                matches!(
                    c,
                    ')' | ']' | '"' | '\'' | '<' | '>' | '|' | '\n' | '\r' | '\t'
                )
            })
            .unwrap_or(after.len());
        let name = &after[..end];
        if !name.is_empty() {
            insert_asset_ref(into, name);
            if let Some(seg) = name.split('/').next() {
                if seg != name {
                    insert_asset_ref(into, seg);
                }
            }
        }
        rest = &after[end..];
    }
}

/// Record an asset reference under BOTH its raw form AND its percent-decoded form.
/// A link like `../assets/my%20file.png` names the on-disk file `my file.png`, so
/// comparing the raw URL substring against directory entries would miss the real
/// file and let `orphan_assets` offer an IN-USE asset for trashing (DS Codex#7).
/// Keeping the raw form too covers a file literally named with a `%` escape.
fn insert_asset_ref(into: &mut std::collections::HashSet<String>, raw: &str) {
    let decoded = percent_decode(raw);
    if decoded != raw {
        into.insert(decoded);
    }
    into.insert(raw.to_string());
}

fn collect_block_asset_refs(b: &DocBlock, into: &mut std::collections::HashSet<String>) {
    collect_asset_refs(&b.raw, into);
    for c in &b.children {
        collect_block_asset_refs(c, into);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrashEntryKind {
    Asset,
    Page,
    Journal,
    Conflict,
    Other,
}

impl TrashEntryKind {
    fn dir_name(self) -> Option<&'static str> {
        match self {
            TrashEntryKind::Asset => Some("assets"),
            TrashEntryKind::Page => Some("pages"),
            TrashEntryKind::Journal => Some("journals"),
            TrashEntryKind::Conflict => Some("conflicts"),
            TrashEntryKind::Other => None,
        }
    }
}

fn trash_root(root: &Path) -> PathBuf {
    root.join("logseq").join(".tine-trash")
}

fn typed_trash_dir(root: &Path, kind: TrashEntryKind) -> PathBuf {
    trash_root(root).join(kind.dir_name().unwrap_or("other"))
}

fn trash_dir_kind(path: &Path) -> Option<TrashEntryKind> {
    match path.file_name().and_then(|s| s.to_str()) {
        Some("assets") => Some(TrashEntryKind::Asset),
        Some("pages") => Some(TrashEntryKind::Page),
        Some("journals") => Some(TrashEntryKind::Journal),
        Some("conflicts") => Some(TrashEntryKind::Conflict),
        _ => None,
    }
}

fn add_trash_stat(stats: &mut TrashStats, kind: TrashEntryKind, bytes: u64) {
    match kind {
        TrashEntryKind::Asset => {
            stats.count += 1;
            stats.bytes += bytes;
        }
        TrashEntryKind::Page => stats.pages += 1,
        TrashEntryKind::Journal => stats.journals += 1,
        TrashEntryKind::Conflict => stats.conflicts += 1,
        TrashEntryKind::Other => stats.other += 1,
    }
}

fn trash_stats(trash: &Path) -> TrashStats {
    let mut stats = TrashStats::default();
    let Ok(rd) = fs::read_dir(trash) else {
        return stats;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(kind) = trash_dir_kind(&path) {
                add_typed_trash_dir_stats(&path, kind, &mut stats);
            } else {
                stats.other += 1;
            }
            continue;
        }
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        add_trash_stat(&mut stats, classify_legacy_trash_entry(&path, ft), bytes);
    }
    stats
}

fn add_typed_trash_dir_stats(path: &Path, kind: TrashEntryKind, stats: &mut TrashStats) {
    let Ok(rd) = fs::read_dir(path) else { return };
    for entry in rd.flatten() {
        let bytes = entry
            .file_type()
            .ok()
            .filter(|ft| ft.is_file())
            .and_then(|_| entry.metadata().ok())
            .map(|m| m.len())
            .unwrap_or(0);
        add_trash_stat(stats, kind, bytes);
    }
}

fn classify_legacy_trash_entry(path: &Path, ft: fs::FileType) -> TrashEntryKind {
    if !ft.is_file() {
        return TrashEntryKind::Other;
    }
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return TrashEntryKind::Other;
    };
    let original = legacy_trash_original_name(name);
    let original_path = Path::new(original);
    if path_is_sync_conflict(original_path) {
        return TrashEntryKind::Conflict;
    }
    let ext = original_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("md" | "org")) {
        return original_path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|stem| crate::date::JournalDate::from_file_stem(stem).is_some())
            .map(|_| TrashEntryKind::Journal)
            .unwrap_or(TrashEntryKind::Page);
    }
    if legacy_name_is_asset(original) {
        TrashEntryKind::Asset
    } else {
        TrashEntryKind::Other
    }
}

fn legacy_trash_original_name(name: &str) -> &str {
    name.split_once("__")
        .map(|(_, original)| original)
        .unwrap_or(name)
}

fn legacy_name_is_asset(name: &str) -> bool {
    if name.starts_with('.') || name.contains('/') || name.contains('\\') || name.ends_with(".edn")
    {
        return false;
    }
    let Some(ext) = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
    else {
        return false;
    };
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "avif"
            | "svg"
            | "bmp"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "pdf"
            | "mp4"
            | "mov"
            | "m4v"
            | "webm"
            | "mkv"
            | "avi"
            | "mp3"
            | "wav"
            | "m4a"
            | "ogg"
            | "flac"
            | "aac"
            | "opus"
            | "txt"
            | "csv"
            | "tsv"
            | "json"
            | "yaml"
            | "yml"
            | "zip"
            | "tar"
            | "gz"
            | "tgz"
            | "7z"
            | "rar"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            | "rtf"
    )
}

fn trash_stamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

fn move_to_trash(src: &Path, dest: &Path, trash: &Path) -> io::Result<()> {
    fs::create_dir_all(trash).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("could not create trash directory {}: {e}", trash.display()),
        )
    })?;
    move_file_noreplace(src, dest).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("could not move file to trash {}: {e}", trash.display()),
        )
    })
}

/// Atomically move one file without ever replacing an existing destination.
/// Platform-native no-replace rename semantics ensure the source name and inode
/// cannot be swapped between a check and an unlink.
pub(crate) fn move_file_noreplace(src: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let src = std::ffi::CString::new(src.as_os_str().as_bytes())?;
        let dest = std::ffi::CString::new(dest.as_os_str().as_bytes())?;
        // Atomic move + create-if-absent. Call the syscall directly: Android's
        // bionic `renameat2` wrapper is only exported from API 30, whereas
        // `syscall` is available from API 1. A wrapper reference here survived
        // the first GH #192 fix in backup.rs and still prevented the complete
        // native library from loading on Android 9. Whichever inode currently
        // owns `src` at the syscall boundary is moved intact, so the safety and
        // errno contracts remain unchanged.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                src.as_ptr(),
                libc::AT_FDCWD,
                dest.as_ptr(),
                libc::RENAME_NOREPLACE as libc::c_uint,
            )
        };
        return (result == 0)
            .then_some(())
            .ok_or_else(io::Error::last_os_error);
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let src = std::ffi::CString::new(src.as_os_str().as_bytes())?;
        let dest = std::ffi::CString::new(dest.as_os_str().as_bytes())?;
        let result = unsafe { libc::renamex_np(src.as_ptr(), dest.as_ptr(), libc::RENAME_EXCL) };
        return (result == 0)
            .then_some(())
            .ok_or_else(io::Error::last_os_error);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut src: Vec<u16> = src.as_os_str().encode_wide().collect();
        let mut dest: Vec<u16> = dest.as_os_str().encode_wide().collect();
        src.push(0);
        dest.push(0);
        // MoveFileW fails when the destination already exists (unlike Rust's
        // cross-platform `rename` contract, which permits replacement).
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileW(src.as_ptr(), dest.as_ptr())
        };
        return (result != 0)
            .then_some(())
            .ok_or_else(io::Error::last_os_error);
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        let _ = (src, dest);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace move is unavailable on this platform",
        ))
    }
}

/// Atomically publish a newly-created file without clobbering a destination that
/// appeared after the caller's collision check. The payload is fsynced in a
/// same-directory temp, then atomically renamed into the final name only if absent.
pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{}.new.tmp", std::process::id(), seq));
    let res = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        move_file_noreplace(&tmp, path)?;
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Atomic write: write to a temp file in the same directory, then rename. The
/// temp name is unique per write (pid + sequence) so two concurrent writers to
/// the same path (e.g. an autosave and a highlight/rename rewrite) can't truncate
/// each other's temp; the rename is still atomic. The temp is removed if the
/// write fails, so a unique name never leaks an orphan behind.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{seq}.tmp", std::process::id()));
    let res = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp); // never leave a temp behind on failure
    } else {
        // Persist the rename itself: fsync the directory so a crash right after the
        // write can't lose the new directory entry (the rename) on some
        // filesystems. Best-effort — not all platforms allow fsync on a dir.
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    res
}

/// Like [`atomic_write`] but the payload is COPIED from `src` (so a large import —
/// a PDF, a big image — isn't slurped fully into memory): copy into a unique temp
/// in the destination dir, fsync it, then atomically rename into place. The temp
/// is removed on any failure, and the directory entry is fsynced on success. The
/// temp name is hidden (`.`-prefixed) so the orphan-asset scanner never lists it.
pub fn atomic_copy(src: &Path, dst: &Path) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = dst.parent().unwrap_or_else(|| Path::new("."));
    let fname = dst.file_name().and_then(|s| s.to_str()).unwrap_or("asset");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{seq}.import.tmp", std::process::id()));
    let res = (|| {
        let mut input = fs::File::open(src)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&tmp, dst)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    } else {
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    res
}

/// Copy into a newly-created destination without replacing a path that appeared
/// concurrently. Used by restore after the previous live inode has been moved to
/// recovery: a sync writer that recreates the live name wins and the restore
/// aborts instead of clobbering it.
pub fn atomic_copy_new(src: &Path, dst: &Path) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = dst.parent().unwrap_or_else(|| Path::new("."));
    let fname = dst.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{fname}.{}.{}.restore.tmp",
        std::process::id(),
        seq
    ));
    let res = (|| {
        let mut input = fs::File::open(src)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        move_file_noreplace(&tmp, dst)?;
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Copy from an already-open source capability into a new destination while
/// enforcing a byte ceiling during the stream. This is the native-capture path:
/// it avoids reopening an attacker-replaceable pathname and avoids whole-value
/// Android/IPC/base64 amplification.
pub fn atomic_copy_file_new(input: &mut fs::File, dst: &Path, max_bytes: u64) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = dst.parent().unwrap_or_else(|| Path::new("."));
    let fname = dst.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{fname}.{}.{}.capture.tmp",
        std::process::id(),
        seq
    ));
    let res = (|| {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let mut limited = input.take(max_bytes.saturating_add(1));
        let copied = io::copy(&mut limited, &mut output)?;
        if copied > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("capture exceeds {max_bytes} byte limit"),
            ));
        }
        output.sync_all()?;
        drop(output);
        move_file_noreplace(&tmp, dst)?;
        let _ = fs::File::open(dir).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Read–modify–write a small text file (config.edn, device settings) under a lock,
/// committed via [`atomic_write`]. The ONE guarded path every settings writer goes
/// through, so the discipline is uniform rather than re-derived per call site:
///   - a MISSING file is the empty document `{}`, but any OTHER read error
///     (permission, NFS stale handle, transient I/O) ABORTS — otherwise `edit` would
///     rebuild the whole file from `{}` and destroy every other key (audit H2);
///   - the `lock` serializes concurrent writers to the same logical file so a
///     read-modify-write can't clobber a concurrent one (audit M1/M2);
///   - `edit` returns the new full contents, or an `Err` to abort without writing;
///   - the commit is atomic (temp + fsync + rename), so a crash can't truncate it.
pub fn atomic_update(
    path: &Path,
    lock: &std::sync::Mutex<()>,
    edit: impl Fn(&str) -> io::Result<String>,
) -> io::Result<()> {
    atomic_update_with_hooks(path, lock, edit, |_| {}, |_| {})
}

fn atomic_update_with_hooks(
    path: &Path,
    lock: &std::sync::Mutex<()>,
    edit: impl Fn(&str) -> io::Result<String>,
    before_recheck: impl Fn(usize),
    before_publish: impl Fn(usize),
) -> io::Result<()> {
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for attempt in 0..4 {
        let baseline = match fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let next = edit(baseline.as_deref().unwrap_or("{}\n"))?;
        // CONFIG_LOCK serializes Tine writers, but Logseq/Syncthing do not take
        // it. Re-read immediately before publish and retry the key-local edit on
        // their new bytes instead of overwriting an external update with our stale
        // full-file copy.
        before_recheck(attempt);
        let current = match fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if current != baseline {
            continue;
        }
        before_publish(attempt);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let published = if baseline.is_none() {
            atomic_write_new(path, next.as_bytes())
        } else {
            atomic_write(path, next.as_bytes())
        };
        match published {
            Ok(()) => return Ok(()),
            Err(error) if baseline.is_none() && error.kind() == io::ErrorKind::AlreadyExists => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "config changed repeatedly during update",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_name_encoding_round_trips_both_formats() {
        // Legacy: `/` ↔ `%2F`; a literal `___` is NOT a separator (stays put).
        let leg = FileNameFormat::Legacy;
        assert_eq!(encode_page_name("a/b/c", leg), "a%2Fb%2Fc");
        assert_eq!(decode_page_name("a%2Fb%2Fc", leg), "a/b/c");
        assert_eq!(encode_page_name("a___b", leg), "a___b");
        assert_eq!(decode_page_name("a___b", leg), "a___b");

        // Triple-lowbar: `/` ↔ `___`; a literal `___` is disambiguated via `%5F`
        // so it survives the round-trip (and isn't read back as a separator).
        let tlb = FileNameFormat::TripleLowbar;
        assert_eq!(encode_page_name("a/b/c", tlb), "a___b___c");
        assert_eq!(decode_page_name("a___b___c", tlb), "a/b/c");
        assert_eq!(encode_page_name("a___b", tlb), "a%5F%5F%5Fb");
        assert_eq!(decode_page_name("a%5F%5F%5Fb", tlb), "a___b");
        // `_` adjacent to the separator round-trips too.
        assert_eq!(
            decode_page_name(&encode_page_name("a_/b", tlb), tlb),
            "a_/b"
        );
        assert_eq!(
            decode_page_name(&encode_page_name("x/_y", tlb), tlb),
            "x/_y"
        );

        // The cross-format hazard the fix addresses: a legacy `%2F` file is read
        // as a namespace ONLY under legacy; a triple-lowbar `___` file ONLY under
        // triple-lowbar — each matching its OG counterpart.
        assert_eq!(decode_page_name("math%2Falgebra", leg), "math/algebra");
        assert_eq!(decode_page_name("math___algebra", tlb), "math/algebra");
        // A unicode percent-escape decodes (UTF-8 aware), like OG.
        assert_eq!(decode_page_name("caf%C3%A9", leg), "café");
    }

    #[test]
    fn runtime_ids_are_owner_structural_and_separate_from_explicit_ids() {
        // Equal-text siblings, including duplicate persisted ids, are distinct
        // runtime nodes. Persisted ids remain content used by external resolution.
        let mut roots = vec![
            DocBlock::new("first\nid:: dup-1234"),
            DocBlock::new("first\nid:: dup-1234"),
        ];
        assign_doc_runtime_ids(&mut roots, "pages/client-a/Foo.md");
        assert_ne!(roots[0].uuid, roots[1].uuid);
        assert_ne!(roots[0].uuid, "dup-1234");
        assert_ne!(roots[1].uuid, "dup-1234");

        let first_ids = roots.iter().map(|b| b.uuid.clone()).collect::<Vec<_>>();
        let mut same = vec![
            DocBlock::new("first\nid:: dup-1234"),
            DocBlock::new("first\nid:: dup-1234"),
        ];
        assign_doc_runtime_ids(&mut same, "pages/client-a/Foo.md");
        assert_eq!(
            first_ids,
            same.iter().map(|b| b.uuid.clone()).collect::<Vec<_>>()
        );

        let mut other_owner = vec![DocBlock::new("first\nid:: dup-1234")];
        assign_doc_runtime_ids(&mut other_owner, "pages/client-b/Foo.md");
        assert_ne!(roots[0].uuid, other_owner[0].uuid);

        // A nested duplicate derives from its structural child path.
        let mut parent = DocBlock::new("p\nid:: x");
        parent.children.push(DocBlock::new("c\nid:: x"));
        assign_doc_runtime_ids(std::slice::from_mut(&mut parent), "pages/tree.md");
        assert_ne!(parent.uuid, parent.children[0].uuid);
        assert_ne!(parent.uuid, "x");
        assert_ne!(parent.children[0].uuid, "x");
    }

    #[test]
    fn sync_identity_projection_persists_every_markdown_block() {
        let page = markdown_page_dto("Sync", "Sync", "- parent\n\t- child\n");
        let runtime_ids = [
            page.blocks[0].id.clone(),
            page.blocks[0].children[0].id.clone(),
        ];
        let migrated = page_with_persisted_sync_ids(&page).unwrap();
        let parent_id = persisted_sync_id(&migrated.blocks[0].raw, Format::Md).unwrap();
        let child_id = persisted_sync_id(&migrated.blocks[0].children[0].raw, Format::Md).unwrap();
        assert!(Uuid::parse_str(&parent_id).is_ok());
        assert!(Uuid::parse_str(&child_id).is_ok());
        assert_ne!(parent_id, runtime_ids[0]);
        assert_ne!(child_id, runtime_ids[1]);
        assert_eq!(migrated.blocks[0].id, runtime_ids[0]);
        assert_eq!(migrated.blocks[0].children[0].id, runtime_ids[1]);
        assert_eq!(count_missing_sync_ids(&migrated.blocks, Format::Md), 0);
        // Resuming the migration is byte-stable and does not add duplicate ids.
        let resumed = page_with_persisted_sync_ids(&migrated).unwrap();
        assert_eq!(resumed.blocks[0].raw, migrated.blocks[0].raw);
        assert_eq!(
            resumed.blocks[0].children[0].raw,
            migrated.blocks[0].children[0].raw
        );
    }

    #[test]
    fn sync_identity_projection_extends_an_org_drawer() {
        let mut page = markdown_page_dto("Org", "Org", "- placeholder\n");
        page.format = Format::Org;
        page.blocks[0].raw = "Task\n:PROPERTIES:\n:foo: bar\n:END:".into();
        let runtime_id = page.blocks[0].id.clone();
        let migrated = page_with_persisted_sync_ids(&page).unwrap();
        let persisted = persisted_sync_id(&migrated.blocks[0].raw, Format::Org).unwrap();
        assert!(Uuid::parse_str(&persisted).is_ok());
        assert_ne!(persisted, runtime_id);
        assert_eq!(migrated.blocks[0].id, runtime_id);
        assert_eq!(
            migrated.blocks[0].raw,
            format!("Task\n:PROPERTIES:\n:foo: bar\n:ID: {persisted}\n:END:")
        );
    }

    #[test]
    fn sync_identity_projection_rejects_duplicate_persisted_ids() {
        let page = markdown_page_dto("Dup", "Dup", "- a\n  id:: same\n- b\n  id:: same\n");
        let err = page_with_persisted_sync_ids(&page).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err
            .to_string()
            .contains("duplicate persisted block id same"));
    }

    #[test]
    fn sync_identity_graph_validation_requires_unique_uuids() {
        let mut a = markdown_page_dto(
            "A",
            "A",
            "- a\n  id:: aaaaaaaa-0000-4000-8000-000000000001\n",
        );
        let mut b = markdown_page_dto(
            "B",
            "B",
            "- b\n  id:: aaaaaaaa-0000-4000-8000-000000000001\n",
        );
        a = page_with_persisted_sync_ids(&a).unwrap();
        b = page_with_persisted_sync_ids(&b).unwrap();
        let mut seen = std::collections::HashSet::new();
        validate_graph_sync_ids(&a.blocks, a.format, &mut seen).unwrap();
        let duplicate = validate_graph_sync_ids(&b.blocks, b.format, &mut seen).unwrap_err();
        assert!(duplicate
            .to_string()
            .contains("duplicate persisted block id"));

        let invalid = markdown_page_dto("Bad", "Bad", "- bad\n  id:: legacy-id\n");
        let invalid = page_with_persisted_sync_ids(&invalid).unwrap();
        let err = validate_graph_sync_ids(
            &invalid.blocks,
            invalid.format,
            &mut std::collections::HashSet::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("is not a UUID"));
    }

    #[test]
    fn reserve_asset_avoids_overwrite() {
        let dir = std::env::temp_dir().join(format!("tine-asset-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Each reserve CREATES the file (exclusively), so the next reserve of the
        // same name is forced onto a fresh suffix — no manual writes needed, and a
        // racing writer can never be handed an already-taken name.
        assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper.pdf");
        assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper_1.pdf");
        assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper_2.pdf");
        // Extensionless names work too.
        assert_eq!(reserve_asset(&dir, "NOTES").unwrap().0, "NOTES");
        assert_eq!(reserve_asset(&dir, "NOTES").unwrap().0, "NOTES_1");
        // Compound extensions (drawio/excalidraw editable assets) survive de-dup:
        // the counter goes BEFORE the whole `.drawio.svg` suffix so the collided
        // name still matches the editor affordance (GH #38). A naive last-dot
        // split would have produced `flow.drawio_1.svg`.
        assert_eq!(
            reserve_asset(&dir, "flow.drawio.svg").unwrap().0,
            "flow.drawio.svg"
        );
        assert_eq!(
            reserve_asset(&dir, "flow.drawio.svg").unwrap().0,
            "flow_1.drawio.svg"
        );
        assert_eq!(
            reserve_asset(&dir, "flow.drawio.svg").unwrap().0,
            "flow_2.drawio.svg"
        );
        // Case-insensitive suffix match, and .excalidraw.png too.
        assert_eq!(
            reserve_asset(&dir, "S.DRAWIO.SVG").unwrap().0,
            "S.DRAWIO.SVG"
        );
        assert_eq!(
            reserve_asset(&dir, "S.DRAWIO.SVG").unwrap().0,
            "S_1.DRAWIO.SVG"
        );
        assert_eq!(
            reserve_asset(&dir, "art.excalidraw.png").unwrap().0,
            "art.excalidraw.png"
        );
        assert_eq!(
            reserve_asset(&dir, "art.excalidraw.png").unwrap().0,
            "art_1.excalidraw.png"
        );
        // An ordinary double-dotted name (not a known compound) still splits on
        // the last dot — `my.file.txt` → `my.file_1.txt`.
        assert_eq!(reserve_asset(&dir, "my.file.txt").unwrap().0, "my.file.txt");
        assert_eq!(
            reserve_asset(&dir, "my.file.txt").unwrap().0,
            "my.file_1.txt"
        );
        // Every reserved name is a real, distinct file on disk.
        for n in [
            "paper.pdf",
            "paper_1.pdf",
            "paper_2.pdf",
            "NOTES",
            "NOTES_1",
            "flow.drawio.svg",
            "flow_1.drawio.svg",
            "flow_2.drawio.svg",
            "art.excalidraw.png",
            "art_1.excalidraw.png",
            "my.file.txt",
            "my.file_1.txt",
        ] {
            assert!(dir.join(n).exists(), "{n} reserved");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reserve_asset_rejects_path_traversal() {
        // F5: a frontend-supplied asset name with a separator or `..`/`.` component
        // must not reach outside assets/. (read_asset shares the same guard.)
        let dir = std::env::temp_dir().join(format!("tine-asset-trav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for bad in ["../evil.md", "..", ".", "a/b.png", "a\\b.png", ""] {
            assert!(reserve_asset(&dir, bad).is_err(), "must reject {bad:?}");
        }
        // A plain top-level name still works.
        assert_eq!(reserve_asset(&dir, "ok.png").unwrap().0, "ok.png");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_capture_import_streams_with_limit_and_collision_rewind() {
        let dir = scratch("native-capture-import");
        let graph = Graph::open(&dir);
        let source_path = dir.join("tine_memo_source.m4a");
        fs::write(&source_path, b"bounded voice memo").unwrap();
        let mut source = fs::File::open(&source_path).unwrap();

        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("assets/voice.m4a"), b"existing memo").unwrap();
        let stored = graph
            .import_asset_file(&mut source, "voice.m4a", 32 * 1024 * 1024)
            .unwrap();
        assert_eq!(stored, "voice_1.m4a");
        assert_eq!(
            fs::read(dir.join("assets/voice_1.m4a")).unwrap(),
            b"bounded voice memo"
        );
        assert_eq!(
            fs::read(dir.join("assets/voice.m4a")).unwrap(),
            b"existing memo",
            "collision retry must not overwrite an existing graph asset"
        );

        source.seek(io::SeekFrom::Start(0)).unwrap();
        assert!(graph
            .import_asset_file(&mut source, "too-large.m4a", 4)
            .is_err());
        assert!(
            !dir.join("assets/too-large.m4a").exists(),
            "an over-limit stream must not leave a visible partial asset"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_pages_preserves_src_page_properties() {
        // F2: reconciling a duplicate page must not silently drop src's page
        // properties (alias/tags/icon). dst wins on a key clash (no duplicate line).
        let dir = std::env::temp_dir().join(format!("tine-merge-props-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages").join("dst.md"),
            "tags:: Keep\n- dst body\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("src.md"),
            "alias:: Foo\ntags:: Other\n- src body\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        g.merge_pages("pages/src.md", "pages/dst.md").unwrap();
        let merged = fs::read_to_string(dir.join("pages").join("dst.md")).unwrap();
        assert!(
            merged.contains("alias:: Foo"),
            "src alias:: preserved: {merged:?}"
        );
        assert!(
            merged.contains("tags:: Keep"),
            "dst tags:: kept: {merged:?}"
        );
        assert!(
            !merged.contains("tags:: Other"),
            "src tags:: must not duplicate dst's key: {merged:?}"
        );
        assert!(
            merged.contains("dst body") && merged.contains("src body"),
            "both bodies merged: {merged:?}"
        );
        assert!(
            !dir.join("pages").join("src.md").exists(),
            "src moved to trash"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gh62_alias_from_first_bullet_merges_backlinks() {
        // GH #62: a user types `alias:: book` as the FIRST bullet on the "books"
        // page (the natural outliner action). OG treats a properties-only first
        // block as page properties, so `#book` references must resolve to "books"
        // and appear in its backlinks. Before the fix this only worked when the
        // alias lived in the page pre-block (dedicated properties panel / Logseq
        // file convention); the bulleted form silently did nothing.
        let build = |books_body: &str| {
            let dir = std::env::temp_dir().join(format!(
                "tine-gh62-{}-{}",
                books_body.len(),
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("journals")).unwrap();
            fs::create_dir_all(dir.join("pages")).unwrap();
            fs::write(dir.join("pages").join("books.md"), books_body).unwrap();
            fs::write(
                dir.join("pages").join("note.md"),
                "- I read a #book today\n",
            )
            .unwrap();
            let g = Graph::open(&dir);
            g.warm_cache();
            let aliases = g.page_aliases();
            let n: usize = g
                .backlinks("books")
                .iter()
                .map(|grp| grp.blocks.len())
                .sum();
            let _ = fs::remove_dir_all(&dir);
            (aliases, n)
        };

        // Alias as the first bullet — now recognized.
        let (a, n) = build("- alias:: book\n- I like reading\n");
        assert_eq!(
            a,
            vec![("book".to_string(), "books".to_string())],
            "first-bullet alias registered"
        );
        assert_eq!(n, 1, "#book backlink merges onto the books page");

        // Pre-block alias keeps working (Logseq file convention / properties panel).
        let (a, n) = build("alias:: book\n\n- I like reading\n");
        assert_eq!(
            a,
            vec![("book".to_string(), "books".to_string())],
            "pre-block alias still registered"
        );
        assert_eq!(n, 1, "pre-block alias backlink still merges");

        // Both Logseq spellings and both common comma glyphs are accepted.
        let (a, n) = build("- aliases:: book，volume\n- I like reading\n");
        assert_eq!(
            a,
            vec![
                ("book".to_string(), "books".to_string()),
                ("volume".to_string(), "books".to_string()),
            ],
            "plural aliases and full-width comma registered"
        );
        assert_eq!(n, 1, "plural alias backlink merges");

        // A whole quoted value is literal text, not a list of page aliases.
        let (a, n) = build("- alias:: \"book\"\n- I like reading\n");
        assert!(a.is_empty(), "quoted alias stays literal: {a:?}");
        assert_eq!(n, 0, "quoted alias does not merge backlinks");

        // A NON-first bullet with `alias::` is a block property, NOT a page alias
        // (OG parity — only the first properties block counts).
        let (a, n) = build("- I like reading\n- alias:: book\n");
        assert!(
            a.is_empty(),
            "alias in a non-first block is not a page alias: {a:?}"
        );
        assert_eq!(n, 0, "no backlink merge for a mid-page block alias");

        // A first block that mixes content with the property is a regular block,
        // not a page-properties block.
        let (a, _) = build("- reading list\nalias:: book\n");
        assert!(
            a.is_empty(),
            "content+property first block is not page properties: {a:?}"
        );
    }

    #[test]
    fn gh62_alias_typed_into_first_block_survives_save_and_reload() {
        let dir = scratch("gh62-save-reload");
        fs::write(
            dir.join("pages").join("books.md"),
            "- placeholder\n- I like reading\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("note.md"),
            "- I read a #book today\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let mut books = g.load_named("books", PageKind::Page).unwrap().unwrap();
        books.blocks[0].raw = "alias:: book".into();
        g.save_page(&books, books.rev.as_deref()).unwrap();

        let disk = fs::read_to_string(dir.join("pages").join("books.md")).unwrap();
        assert_eq!(disk, "alias:: book\n\n- I like reading\n");
        assert_eq!(
            g.load_named("book", PageKind::Page).unwrap().unwrap().name,
            "books"
        );
        assert_eq!(
            g.backlinks("books")
                .iter()
                .map(|group| group.blocks.len())
                .sum::<usize>(),
            1
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_switch_includes_referenced_pages() {
        // A page referenced by `#tag` / `[[link]]` but with no file of its own
        // still "exists" (OG semantics) and must show up in quick-switch — that's
        // what lets `#`/`[[ ]]` autocomplete say "#thistag" rather than a
        // misleading "Create #thistag" when the tag is already used elsewhere.
        let dir = std::env::temp_dir().join(format!("tine-refpages-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages").join("notes.md"),
            "- uses #thistag and [[Some Page]]\n",
        )
        .unwrap();
        // A page whose page-properties carry tags::/alias:: (OG autolinks these as
        // page references, bare or bracketed).
        fs::write(
            dir.join("pages").join("paper.md"),
            "tags:: ProjectX， [[Linear IP]]\naliases:: LP Survey，Paper Notes\nstatus:: \"Private, Draft\"\n- body\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache(); // referenced names come from the whole-graph cache

        let has = |q: &str, name: &str| {
            g.quick_switch(q, 8)
                .iter()
                .any(|e| crate::refs::same_page(&e.name, name))
        };
        assert!(
            has("thistag", "thistag"),
            "referenced #thistag should appear"
        );
        assert!(
            has("some page", "Some Page"),
            "referenced [[Some Page]] should appear"
        );
        // tags:: values (bare and bracketed) and alias:: values count too.
        assert!(
            has("projectx", "ProjectX"),
            "bare tags:: value should appear"
        );
        assert!(
            has("linear ip", "Linear IP"),
            "bracketed tags:: value should appear"
        );
        assert!(has("lp survey", "LP Survey"), "alias:: value should appear");
        assert!(
            has("paper notes", "Paper Notes"),
            "aliases:: value should appear"
        );
        assert!(
            !has("private", "Private"),
            "quoted custom value stays literal"
        );
        // Neither filed nor referenced → not offered (so autocomplete still says
        // "Create" for a genuinely new name).
        assert!(!has("nonexistent", "nonexistent"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tine-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        dir
    }

    fn candidate_paths(candidates: &ReferenceCandidatePages) -> Vec<String> {
        let mut paths = candidates
            .pages
            .iter()
            .map(|(entry, _)| entry.rel_path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    fn assert_reference_candidates_equal_full_scan(
        graph: &Graph,
        target: &str,
        names: &[String],
        kind: ReferenceKind,
    ) {
        let aliases = graph.page_aliases();
        let real_pages = crate::query::real_page_names(graph);
        let exact_paths = |pages: &[(PageEntry, Arc<Document>)]| {
            let mut paths = pages
                .iter()
                .filter(|(entry, doc)| match kind {
                    ReferenceKind::Explicit => crate::query::page_affects_backlinks(
                        &real_pages,
                        &aliases,
                        target,
                        entry,
                        doc,
                    ),
                    ReferenceKind::Plain => crate::query::page_affects_unlinked(
                        &real_pages,
                        &aliases,
                        target,
                        entry,
                        doc,
                    ),
                })
                .map(|(entry, _)| entry.rel_path.clone())
                .collect::<Vec<_>>();
            paths.sort();
            paths
        };
        let full = graph.with_pages(exact_paths);
        let candidates = graph.reference_candidate_pages(names, kind);
        assert_eq!(exact_paths(&candidates.pages), full);
    }

    fn assert_indexed_reference_results_equal_full_scan(graph: &Graph, target: &str) {
        {
            let mut guard = graph.reference_candidate_index.write().unwrap();
            let index = guard.as_mut().unwrap();
            index.complete = true;
            index.generation = graph.cache_generation();
        }
        let indexed_backlinks = crate::query::backlinks(graph, target);
        let indexed_unlinked = crate::query::unlinked_refs(graph, target);
        graph
            .reference_candidate_index
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .complete = false;
        let full_backlinks = crate::query::backlinks(graph, target);
        let full_unlinked = crate::query::unlinked_refs(graph, target);
        assert_eq!(
            serde_json::to_value(indexed_backlinks).unwrap(),
            serde_json::to_value(full_backlinks).unwrap()
        );
        assert_eq!(
            serde_json::to_value(indexed_unlinked).unwrap(),
            serde_json::to_value(full_unlinked).unwrap()
        );
        graph
            .reference_candidate_index
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .complete = true;
    }

    #[test]
    fn reference_candidate_index_tracks_every_cache_seam_and_falls_back_safely() {
        let dir = scratch("reference-candidate-index");
        fs::write(
            dir.join("pages/Target.md"),
            "alias:: Alias\n\n- target body\n",
        )
        .unwrap();
        let source_path = dir.join("pages/Source.md");
        fs::write(&source_path, "- [[Alias]] and plain Target\n").unwrap();
        fs::write(dir.join("pages/Irrelevant.md"), "- unrelated\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let names = vec![
            crate::refs::page_key("Target"),
            crate::refs::page_key("Alias"),
        ];
        let explicit = g.reference_candidate_pages(&names, ReferenceKind::Explicit);
        assert!(explicit.indexed);
        assert!(candidate_paths(&explicit).contains(&"pages/Source.md".to_string()));
        assert!(explicit.pages.len() < explicit.full_page_count);
        let plain = g.reference_candidate_pages(&names, ReferenceKind::Plain);
        assert!(plain.indexed);
        assert!(candidate_paths(&plain).contains(&"pages/Source.md".to_string()));
        let unicode_fallback =
            g.reference_candidate_pages(&[crate::refs::page_key("Café")], ReferenceKind::Plain);
        assert!(!unicode_fallback.indexed);
        assert_eq!(
            unicode_fallback.pages.len(),
            unicode_fallback.full_page_count
        );
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Explicit);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Plain);
        assert_indexed_reference_results_equal_full_scan(&g, "Target");

        // Normal save/cache-upsert removes both projections without rebuilding
        // the graph cache.
        let mut source = g.load_named("Source", PageKind::Page).unwrap().unwrap();
        source.blocks[0].raw = "nothing here".into();
        g.save_page(&source, source.rev.as_deref()).unwrap();
        assert!(
            !candidate_paths(&g.reference_candidate_pages(&names, ReferenceKind::Explicit))
                .contains(&"pages/Source.md".to_string())
        );
        assert!(
            !candidate_paths(&g.reference_candidate_pages(&names, ReferenceKind::Plain))
                .contains(&"pages/Source.md".to_string())
        );
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Explicit);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Plain);
        assert_indexed_reference_results_equal_full_scan(&g, "Target");

        // Watcher-equivalent physical replace is an upsert at the same seam.
        fs::write(&source_path, "- [[Target]] plus Target\n").unwrap();
        assert!(g.sync_file(&source_path).is_some());
        assert!(
            candidate_paths(&g.reference_candidate_pages(&names, ReferenceKind::Explicit))
                .contains(&"pages/Source.md".to_string())
        );
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Explicit);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Plain);
        assert_indexed_reference_results_equal_full_scan(&g, "Target");

        g.delete_page("Source", PageKind::Page).unwrap();
        let after_delete = g.reference_candidate_pages(&names, ReferenceKind::Explicit);
        assert!(after_delete.indexed);
        assert!(!candidate_paths(&after_delete).contains(&"pages/Source.md".to_string()));
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Explicit);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Plain);
        assert_indexed_reference_results_equal_full_scan(&g, "Target");

        // A broad invalidation reconstructs from the new physical page set.
        fs::write(&source_path, "- [[Alias]] and Target again\n").unwrap();
        g.invalidate_cache();
        g.warm_cache();
        assert!(
            candidate_paths(&g.reference_candidate_pages(&names, ReferenceKind::Explicit))
                .contains(&"pages/Source.md".to_string())
        );
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Explicit);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Plain);
        assert_indexed_reference_results_equal_full_scan(&g, "Target");

        assert!(g
            .create_markdown_page_if_absent("Created", "- [[Target]] and Target\n")
            .unwrap());
        let after_create = g.reference_candidate_pages(&names, ReferenceKind::Explicit);
        assert!(after_create.indexed);
        assert!(candidate_paths(&after_create).contains(&"pages/Created.md".to_string()));
        assert_indexed_reference_results_equal_full_scan(&g, "Target");

        // Deliberate incompleteness can never narrow the authority set.
        g.reference_candidate_index
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .complete = false;
        let fallback = g.reference_candidate_pages(&names, ReferenceKind::Explicit);
        assert!(!fallback.indexed);
        assert_eq!(fallback.pages.len(), fallback.full_page_count);
        assert_eq!(fallback.full_page_count, 4);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Explicit);
        assert_reference_candidates_equal_full_scan(&g, "Target", &names, ReferenceKind::Plain);

        let current_generation = g.cache_generation();
        {
            let mut guard = g.reference_candidate_index.write().unwrap();
            let index = guard.as_mut().unwrap();
            index.complete = true;
            index.generation = current_generation.saturating_sub(1);
        }
        let stale_fallback = g.reference_candidate_pages(&names, ReferenceKind::Explicit);
        assert!(!stale_fallback.indexed);
        assert_eq!(stale_fallback.pages.len(), stale_fallback.full_page_count);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_reference_signature_folds_unicode_before_ascii_tokenizing() {
        let dir = scratch("reference-candidate-unicode-fold");
        fs::write(dir.join("pages/K.md"), "- target body\n").unwrap();
        fs::write(dir.join("pages/Source.md"), "- plain K mention\n").unwrap();
        fs::write(dir.join("pages/Irrelevant.md"), "- unrelated\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let names = vec![crate::refs::page_key("K")];
        let candidates = g.reference_candidate_pages(&names, ReferenceKind::Plain);
        assert!(candidates.indexed);
        assert!(candidate_paths(&candidates).contains(&"pages/Source.md".to_string()));
        assert!(candidates.pages.len() < candidates.full_page_count);
        assert_reference_candidates_equal_full_scan(&g, "K", &names, ReferenceKind::Plain);
        assert_indexed_reference_results_equal_full_scan(&g, "K");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "10k synthetic performance receipt"]
    fn reference_candidate_index_10k_receipt() {
        let dir = scratch("reference-candidate-10k");
        for index in 0..10_000 {
            let body = if index % 1_000 == 0 {
                format!("- [[Needle]] explicit Needle {index}\n")
            } else if index % 500 == 0 {
                format!("- plain Needle {index}\n")
            } else {
                format!("- ordinary synthetic page {index}\n")
            };
            fs::write(dir.join("pages").join(format!("Page {index:05}.md")), body).unwrap();
        }
        let g = Graph::open(&dir);
        let started = std::time::Instant::now();
        g.with_pages(|_| ());
        let build_ms = started.elapsed().as_millis();
        let names = vec![crate::refs::page_key("Needle")];
        let explicit = g.reference_candidate_pages(&names, ReferenceKind::Explicit);
        let plain = g.reference_candidate_pages(&names, ReferenceKind::Plain);
        let estimated_bytes = g
            .reference_candidate_index
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .estimated_bytes();
        let indexed_backlinks = crate::query::backlinks(&g, "Needle");
        let indexed_unlinked = crate::query::unlinked_refs(&g, "Needle");
        g.reference_candidate_index
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .complete = false;
        let full_backlinks = crate::query::backlinks(&g, "Needle");
        let full_unlinked = crate::query::unlinked_refs(&g, "Needle");
        assert!(explicit.indexed && plain.indexed);
        assert_eq!(explicit.pages.len(), 10);
        assert!(plain.pages.len() >= 20);
        assert_eq!(
            serde_json::to_value(&indexed_backlinks).unwrap(),
            serde_json::to_value(&full_backlinks).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&indexed_unlinked).unwrap(),
            serde_json::to_value(&full_unlinked).unwrap()
        );
        eprintln!(
            "reference-index-10k build_ms={build_ms} estimated_bytes={estimated_bytes} explicit_candidates={} plain_candidates={} full_pages={} linked_exact_equal=true unlinked_exact_equal=true",
            explicit.pages.len(),
            plain.pages.len(),
            explicit.full_page_count,
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_cache_isolates_one_page_projection_panic() {
        let dir = scratch("search-page-panic-isolation");
        for i in 0..64 {
            fs::write(
                dir.join("pages").join(format!("Page {i:02}.md")),
                format!("- ordinary page {i}\n"),
            )
            .unwrap();
        }

        let g = Graph::open(&dir);
        let entries = g.list_pages();
        let workers = page_cache_worker_count();
        assert!(workers > 1, "test must exercise the parallel cache build");
        assert!(
            entries.len() >= 64,
            "test must cross the parallel threshold"
        );
        let per = (entries.len() + workers - 1) / workers;
        assert!(per >= 2, "a worker shard must contain a sibling page");

        // Pick adjacent entries after observing the actual directory-walk order,
        // guaranteeing both are in the first worker shard on every filesystem.
        let bad = &entries[0];
        let sibling = &entries[1];
        fs::write(&bad.path, format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n")).unwrap();
        let needle = "uniquesameshardsibling";
        fs::write(&sibling.path, format!("- {needle}\n")).unwrap();
        let sibling_path = sibling.rel_path.clone();
        let bad_path = bad.rel_path.clone();

        let execution = g.run_graph_search(needle, 0, 8, false);
        assert!(
            execution.hits.iter().any(|hit| matches!(
                hit,
                crate::query_plan::QueryHit::Block { path, .. } if path == &sibling_path
            )),
            "a normal same-shard sibling must remain searchable"
        );
        assert_eq!(g.page_index_failures(), vec![bad_path]);

        // Invalidation clears the old diagnostic, and the paced warm-cache path
        // applies the same page-sized isolation when it rebuilds.
        g.invalidate_cache();
        assert!(g.page_index_failures().is_empty());
        g.warm_cache();
        assert!(g
            .run_graph_search(needle, 0, 8, false)
            .hits
            .iter()
            .any(|hit| matches!(
                hit,
                crate::query_plan::QueryHit::Block { path, .. } if path == &sibling_path
            )));
        assert_eq!(g.page_index_failures(), vec![bad.rel_path.clone()]);
        let _ = fs::remove_dir_all(&dir);
    }

    fn reset_list_md_calls() {
        LIST_MD_CALLS.with(|calls| calls.set(0));
    }

    fn list_md_calls() -> usize {
        LIST_MD_CALLS.with(|calls| calls.get())
    }

    fn reset_cache_linear_scan_steps() {
        CACHE_LINEAR_SCAN_STEPS.with(|steps| steps.set(0));
    }

    fn cache_linear_scan_steps() -> usize {
        CACHE_LINEAR_SCAN_STEPS.with(|steps| steps.get())
    }

    fn reference_find_entry(g: &Graph, name: &str, kind: PageKind) -> Option<PageEntry> {
        let dir = match kind {
            PageKind::Journal => g.journals_path(),
            PageKind::Page => g.pages_path(),
        };
        let rel_dir = match kind {
            PageKind::Journal => &g.config.journals_dir,
            PageKind::Page => &g.config.pages_dir,
        };
        let matches: Vec<PageEntry> = list_md(
            &dir,
            kind,
            &g.journal_format,
            g.config.file_name_format,
            rel_dir,
        )
        .into_iter()
        .filter(|e| crate::refs::same_page(&e.name, name))
        .collect();
        matches
            .iter()
            .find(|e| is_date_stem_entry(e))
            .or_else(|| matches.first())
            .cloned()
    }

    #[test]
    fn find_entry_cache_avoids_per_lookup_list_md_fanout() {
        let dir = scratch("find-entry-cache-fanout");
        for i in 0..16 {
            fs::write(dir.join("pages").join(format!("Page {i}.md")), "- body\n").unwrap();
        }
        let g = Graph::open(&dir);
        g.warm_cache();

        reset_list_md_calls();
        for i in 0..16 {
            let entry = g
                .find_entry(&format!("Page {i}"), PageKind::Page)
                .expect("page exists");
            assert_eq!(entry.name, format!("Page {i}"));
        }
        assert_eq!(
            list_md_calls(),
            1,
            "all page lookups in one generation should share one raw page scan"
        );

        for i in 0..16 {
            assert!(g.find_entry(&format!("Page {i}"), PageKind::Page).is_some());
        }
        assert_eq!(
            list_md_calls(),
            1,
            "warm find_entry index should serve repeated lookups without rescanning"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_entry_cache_matches_old_list_md_selection() {
        let dir = scratch("find-entry-cache-equivalence");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Foo.md"), "- normal\n").unwrap();
        fs::create_dir_all(dir.join("pages").join("sub")).unwrap();
        fs::write(
            dir.join("pages").join("sub").join("Nested.md"),
            "- nested\n",
        )
        .unwrap();
        fs::write(dir.join("journals").join("2026_06_26.org"), "* canonical\n").unwrap();
        fs::write(
            dir.join("journals").join("Friday, 26-06-2026.org"),
            "* stray\n",
        )
        .unwrap();
        let g = Graph::open(&dir);

        for (name, kind) in [
            ("foo", PageKind::Page),
            ("Nested", PageKind::Page),
            ("Friday, 26-06-2026", PageKind::Journal),
        ] {
            let expected = reference_find_entry(&g, name, kind)
                .unwrap_or_else(|| panic!("reference missing {kind:?} {name:?}"));
            let actual = g
                .find_entry(name, kind)
                .unwrap_or_else(|| panic!("cached lookup missing {kind:?} {name:?}"));
            assert_eq!(
                actual.path, expected.path,
                "cached lookup must match old selection for {kind:?} {name:?}"
            );
        }

        let journal = g
            .find_entry("Friday, 26-06-2026", PageKind::Journal)
            .unwrap();
        assert_eq!(journal.rel_path, "journals/2026_06_26.org");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parsed_doc_cache_index_avoids_warm_open_linear_scans() {
        let dir = scratch("doc-cache-index-fanout");
        for i in 0..24 {
            fs::write(dir.join("pages").join(format!("Page {i}.md")), "- body\n").unwrap();
        }
        let g = Graph::open(&dir);
        g.warm_cache();
        assert!(
            g.cache_index.read().unwrap().is_some(),
            "warm cache should install the by-name parsed-doc index"
        );

        reset_cache_linear_scan_steps();
        for i in 0..24 {
            let page = g
                .load_named(&format!("Page {i}"), PageKind::Page)
                .unwrap()
                .expect("page exists");
            assert_eq!(page.name, format!("Page {i}"));
        }
        assert_eq!(
            cache_linear_scan_steps(),
            0,
            "warm page opens must not fall back to Vec scans"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parsed_doc_cache_index_does_not_serve_deleted_page() {
        let dir = scratch("doc-cache-index-delete");
        fs::write(dir.join("pages").join("Gone.md"), "- old\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let entry = g.find_entry("Gone", PageKind::Page).unwrap();
        assert!(g.load_page(&entry).is_ok());

        g.delete_page("Gone", PageKind::Page).unwrap();
        assert!(
            g.load_page(&entry).is_err(),
            "stale cache/index must not serve the deleted entry"
        );
        assert!(g.load_named("Gone", PageKind::Page).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parsed_doc_cache_index_rebuilds_after_rename() {
        let dir = scratch("doc-cache-index-rename");
        fs::write(
            dir.join("pages").join("Old.md"),
            "- links [[Old]] and #Old\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let old_entry = g.find_entry("Old", PageKind::Page).unwrap();
        assert!(g.load_page(&old_entry).is_ok());

        g.rename_page("Old", "New").unwrap();
        assert!(
            g.load_page(&old_entry).is_err(),
            "old entry must not be served after rename"
        );
        assert!(g.load_named("Old", PageKind::Page).unwrap().is_none());
        let new_page = g
            .load_named("New", PageKind::Page)
            .unwrap()
            .expect("new name resolves");
        assert_eq!(new_page.name, "New");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_entry_cache_rebuilds_after_file_rescue_generation_bump() {
        let dir = scratch("find-entry-cache-rescue");
        fs::write(dir.join("journals").join("Loose.md"), "- loose\n").unwrap();
        let g = Graph::open(&dir);

        assert!(g.find_entry("Rescued", PageKind::Page).is_none());
        assert!(g.find_entry("Loose", PageKind::Journal).is_some());

        g.rename_file_to_page("journals/Loose.md", "Rescued")
            .unwrap();
        assert!(g.find_entry("Loose", PageKind::Journal).is_none());
        assert!(g.find_entry("Rescued", PageKind::Page).is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_entry_cache_invalidated_by_cold_sync_file() {
        // Regression: the gen-keyed find_entry index must not go stale on
        // sync_file's cold-cache branch (the parsed-doc cache not yet built), which
        // drops the page-list memo WITHOUT bumping cache_gen. Before the fix,
        // find_entry kept serving the pre-create index here (missing the new file)
        // until some other op happened to bump the generation.
        let dir = scratch("find-entry-cache-cold-sync");
        fs::write(dir.join("pages").join("Existing.md"), "- body\n").unwrap();
        let g = Graph::open(&dir);

        // Do NOT warm the doc cache: find_entry builds only its own index, so
        // self.cache stays cold and sync_file below takes the else-branch.
        assert!(g.find_entry("New", PageKind::Page).is_none());

        // A brand-new external file appears (as Logseq/Syncthing would create it),
        // reconciled while the doc cache is still cold.
        fs::write(dir.join("pages").join("New.md"), "- new body\n").unwrap();
        g.sync_file(&dir.join("pages").join("New.md"));

        assert!(
            g.find_entry("New", PageKind::Page).is_some(),
            "find_entry index must reflect a file added via the cold sync_file branch"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_pages_snapshot_does_not_block_cache_upsert() {
        let dir = scratch("with-pages-snapshot-nonblocking");
        fs::write(dir.join("pages").join("A.md"), "- old\n").unwrap();
        let g = Arc::new(Graph::open(&dir));
        g.warm_cache();

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let scan_graph = Arc::clone(&g);
        let scan = std::thread::spawn(move || {
            scan_graph.with_pages(|pages| {
                assert!(!pages.is_empty());
                entered_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .expect("test should release the blocked snapshot scan");
            });
        });

        entered_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("snapshot scan should enter its closure");

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let write_graph = Arc::clone(&g);
        let path = dir.join("pages").join("B.md");
        fs::write(&path, "- new\n").unwrap();
        let writer = std::thread::spawn(move || {
            let content = "- new\n";
            let entry = PageEntry {
                name: "B".to_string(),
                kind: PageKind::Page,
                date_key: None,
                rel_path: "pages/B.md".to_string(),
                path: path.clone(),
            };
            write_graph.cache_upsert(entry, parse_doc(&path, content), content_rev(content));
            done_tx.send(()).unwrap();
        });

        let writer_finished_while_scan_blocked = done_rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_ok();
        release_tx.send(()).unwrap();
        scan.join().unwrap();
        writer.join().unwrap();

        assert!(
            writer_finished_while_scan_blocked,
            "cache_upsert must not wait for a with_pages closure to finish"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_pages_snapshot_survives_concurrent_upsert() {
        let dir = scratch("with-pages-snapshot-consistent");
        let path = dir.join("pages").join("A.md");
        fs::write(&path, "- old body\n").unwrap();
        let g = Arc::new(Graph::open(&dir));
        g.warm_cache();

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (observed_tx, observed_rx) = std::sync::mpsc::channel();
        let scan_graph = Arc::clone(&g);
        let scan = std::thread::spawn(move || {
            scan_graph.with_pages(|pages| {
                let (_, doc) = pages
                    .iter()
                    .find(|(entry, _)| entry.kind == PageKind::Page && entry.name == "A")
                    .expect("cached page exists");
                let before = doc.roots[0].raw.clone();
                entered_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .expect("test should release the blocked snapshot scan");
                let after = doc.roots[0].raw.clone();
                observed_tx.send((before, after)).unwrap();
            });
        });

        entered_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("snapshot scan should enter its closure");

        let new_content = "- new body\n";
        fs::write(&path, new_content).unwrap();
        let entry = PageEntry {
            name: "A".to_string(),
            kind: PageKind::Page,
            date_key: None,
            rel_path: "pages/A.md".to_string(),
            path: path.clone(),
        };
        g.cache_upsert(
            entry,
            parse_doc(&path, new_content),
            content_rev(new_content),
        );

        release_tx.send(()).unwrap();
        let (before, after) = observed_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("snapshot scan should report observed values");
        scan.join().unwrap();

        assert_eq!(before, "old body");
        assert_eq!(
            after, "old body",
            "a with_pages scan must keep iterating its original snapshot"
        );
        let loaded = g
            .load_named("A", PageKind::Page)
            .unwrap()
            .expect("page remains loadable");
        assert_eq!(loaded.blocks[0].raw, "new body");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn warm_cache_primes_alias_and_block_ref_count_caches() {
        let dir = scratch("warm-derived");
        fs::write(
            dir.join("pages").join("Target.md"),
            "alias:: Alias One\n\n- target\n  id:: aaaaaaaa-0000-0000-0000-000000000001\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Refs.md"),
            "- see ((aaaaaaaa-0000-0000-0000-000000000001))\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        assert!(
            g.alias_cache.read().unwrap().is_none(),
            "alias cache starts cold"
        );
        assert!(
            g.block_ref_count_cache.read().unwrap().is_none(),
            "block-ref count cache starts cold"
        );

        g.warm_cache();

        let aliases = g.alias_cache.read().unwrap().as_ref().cloned().unwrap();
        assert!(
            aliases
                .iter()
                .any(|(alias, canon, _)| alias == "alias one" && canon == "Target"),
            "alias cache warmed: {aliases:?}"
        );
        let gen = g.cache_generation();
        let counts = g.block_ref_count_cache.read().unwrap();
        let (count_gen, count_map) = counts.as_ref().expect("block-ref count cache warmed");
        assert_eq!(
            *count_gen, gen,
            "count cache is keyed to the current cache generation"
        );
        assert_eq!(
            count_map
                .get("aaaaaaaa-0000-0000-0000-000000000001")
                .copied(),
            Some(1)
        );

        let first = g.block_ref_counts();
        let second = g.block_ref_counts();
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "re-entering block_ref_counts should reuse the warmed Arc"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_reference_edit_reuses_block_ref_count_index() {
        let dir = scratch("block-ref-count-scoped");
        fs::write(
            dir.join("pages").join("Target.md"),
            "- target\n  id:: aaaaaaaa-0000-0000-0000-000000000001\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Refs.md"),
            "- see ((aaaaaaaa-0000-0000-0000-000000000001))\n",
        )
        .unwrap();

        let g = Graph::open(&dir);
        g.warm_cache();
        let before = g.block_ref_counts();
        let mut target = g.load_named("Target", PageKind::Page).unwrap().unwrap();
        target.blocks[0].raw = "target edited without changing references".into();
        g.save_page(&target, target.rev.as_deref()).unwrap();
        let after = g.block_ref_counts();

        assert!(
            Arc::ptr_eq(&before, &after),
            "a non-reference edit must retain the already-built whole-graph count map"
        );
        assert_eq!(
            after.get("aaaaaaaa-0000-0000-0000-000000000001").copied(),
            Some(1)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_recovers_title_named_org_journals() {
        // Regression: changing :journal/page-title-format while a stale in-memory
        // format was still active saved new journals under their title
        // ("Thursday, 25-06-2026.org") instead of the date stem, so they dropped
        // out of the feed. A reopen + migrate (now .org-aware) must recover them.
        let dir = scratch("journal-migrate-org");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals").join("Thursday, 25-06-2026.org"),
            "* bla\n",
        )
        .unwrap();
        // A canonical file for another day must be left untouched.
        fs::write(dir.join("journals").join("2026_06_24.org"), "* prior\n").unwrap();

        let g = Graph::open(&dir);
        assert_eq!(
            g.migrate_journal_filenames(),
            1,
            "exactly the title-named file renamed"
        );
        assert!(
            dir.join("journals").join("2026_06_25.org").exists(),
            "renamed to date stem"
        );
        assert!(
            !dir.join("journals")
                .join("Thursday, 25-06-2026.org")
                .exists(),
            "old name gone"
        );
        assert!(
            dir.join("journals").join("2026_06_24.org").exists(),
            "canonical file untouched"
        );

        // It's now recognized in the feed listing (name via the title format).
        let names: Vec<String> = Graph::open(&dir)
            .journals_desc()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(
            names.iter().any(|n| n == "Thursday, 25-06-2026"),
            "listed: {names:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_conflicts_reports_duplicate_days() {
        let dir = scratch("journal-conflicts");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        // Same day, two files (canonical stem + title-named) — a conflict.
        fs::write(
            dir.join("journals").join("2026_06_26.org"),
            "* canonical content\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals").join("Friday, 26-06-2026.org"),
            "* stray content\n",
        )
        .unwrap();
        // A clean day with one file — not a conflict.
        fs::write(dir.join("journals").join("2026_06_24.org"), "* fine\n").unwrap();

        let conflicts = Graph::open(&dir).journal_conflicts();
        assert_eq!(
            conflicts.len(),
            1,
            "exactly one conflicted day: {conflicts:?}"
        );
        let c = &conflicts[0];
        assert_eq!(c.title, "Friday, 26-06-2026");
        assert_eq!(c.files.len(), 2);
        // Canonical (date-stem) file sorts first and is flagged; preview is the body line.
        assert_eq!(c.files[0].name, "2026_06_26.org");
        assert!(c.files[0].canonical);
        assert_eq!(c.files[0].preview, "canonical content");
        assert!(!c.files[1].canonical);
        assert_eq!(c.files[1].name, "Friday, 26-06-2026.org");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_conflicts_reports_nested_duplicate_days() {
        let dir = scratch("journal-conflicts-nested");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("journals").join("archive")).unwrap();
        fs::write(
            dir.join("journals").join("archive").join("2026_06_26.org"),
            "* canonical nested\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals")
                .join("archive")
                .join("Friday, 26-06-2026.org"),
            "* stray nested\n",
        )
        .unwrap();

        let conflicts = Graph::open(&dir).journal_conflicts();
        assert_eq!(
            conflicts.len(),
            1,
            "nested duplicate day is surfaced: {conflicts:?}"
        );
        let files = &conflicts[0].files;
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "journals/archive/2026_06_26.org");
        assert_eq!(files[1].path, "journals/archive/Friday, 26-06-2026.org");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_sync_conflicts_reports_nested_conflict_copy() {
        let dir = scratch("sync-conflicts-nested");
        fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
        fs::write(
            dir.join("pages").join("client-a").join("Foo.md"),
            "- base\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages")
                .join("client-a")
                .join("Foo.sync-conflict-20260705-141233-A1B2C3D.md"),
            "- conflict copy\n",
        )
        .unwrap();

        let conflicts = Graph::open(&dir).list_sync_conflicts();
        assert_eq!(
            conflicts.len(),
            1,
            "nested sync-conflict copy is surfaced: {conflicts:?}"
        );
        let c = &conflicts[0];
        assert_eq!(
            c.path,
            "pages/client-a/Foo.sync-conflict-20260705-141233-A1B2C3D.md"
        );
        assert_eq!(c.base_path.as_deref(), Some("pages/client-a/Foo.md"));
        assert_eq!(c.base_name, "Foo");
        assert_eq!(c.kind, PageKind::Page);
        assert_eq!(c.preview, "conflict copy");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journals_desc_dedups_duplicate_day_to_canonical() {
        // The feed must show a day ONCE even when two files resolve to it — else
        // the same day renders twice (loaded from whichever file path_for picks).
        let dir = scratch("journal-dedup");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::write(dir.join("journals").join("2026_06_26.org"), "* real day\n").unwrap();
        fs::write(
            dir.join("journals").join("Friday, 26-06-2026.org"),
            "* stray\n",
        )
        .unwrap();
        fs::write(dir.join("journals").join("2026_06_24.org"), "* other day\n").unwrap();

        let js = Graph::open(&dir).journals_desc();
        assert_eq!(
            js.len(),
            2,
            "one entry per day: {:?}",
            js.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        // The deduped 26th keeps the canonical date-stem file (what saves resolve to).
        let day26 = js
            .iter()
            .find(|e| e.name == "Friday, 26-06-2026")
            .expect("26th present");
        assert_eq!(
            day26.path.file_name().unwrap().to_str().unwrap(),
            "2026_06_26.org"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn future_journals_are_feed_only_excluded_but_keep_raw_identity() {
        let dir = scratch("future-feed-raw-identity");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:journal/file-name-format \"dd-MM-yyyy\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        let future = dir.join("journals/17-07-2030.md");
        let future_bytes = b"- future-search-sentinel\n";
        fs::write(&future, future_bytes).unwrap();
        fs::write(dir.join("journals/15-07-2030.md"), "- today sentinel\n").unwrap();
        fs::write(dir.join("journals/14-07-2030.md"), "- past sentinel\n").unwrap();
        let g = Graph::open(&dir);
        let future_title = "Wednesday, 17-07-2030";
        assert_eq!(
            g.journals_desc().len(),
            3,
            "raw inventory retains future journals"
        );
        let feed = g.feed_journals_desc_through(JournalDate {
            year: 2030,
            month: 7,
            day: 15,
        });
        assert_eq!(
            feed.iter().map(|e| e.date_key).collect::<Vec<_>>(),
            vec![Some(20300715), Some(20300714)]
        );
        let future_entry = g
            .journals_desc()
            .into_iter()
            .find(|e| e.date_key == Some(20300717))
            .unwrap();
        assert_eq!(future_entry.path, future);
        assert_eq!(
            g.load_page(&future_entry).unwrap().blocks[0].raw,
            "future-search-sentinel"
        );
        assert!(g.list_pages().iter().any(|e| e.path == future));
        assert_eq!(
            g.find_entry(future_title, PageKind::Journal).unwrap().path,
            future
        );
        assert_eq!(
            g.load_named(future_title, PageKind::Journal)
                .unwrap()
                .unwrap()
                .blocks[0]
                .raw,
            "future-search-sentinel"
        );
        // Ctrl-K uses the current combined latest-wins graph-search path, not
        // the legacy quick_switch adapter. Its whole-graph inventory remains
        // deliberately separate from the filtered Journals feed.
        assert!(g
            .run_graph_search_latest("future-feed-test", future_title, 8, 8, false)
            .hits
            .iter()
            .any(|hit| matches!(hit,
                crate::query_plan::QueryHit::Page { page, .. } if page.path == future
            )));
        assert!(!g.search("future-search-sentinel", 8).is_empty());
        assert_eq!(g.path_for(future_title, PageKind::Journal), future);
        assert_eq!(
            g.page_source_file(future_title, PageKind::Journal, None)
                .unwrap(),
            future.canonicalize().unwrap()
        );
        assert_eq!(
            fs::read(&future).unwrap(),
            future_bytes,
            "feed/list/search performed no write"
        );

        // The warmed cache retains exactly the cold membership/order and later
        // whole-graph lookups still see the excluded future page.
        g.warm_cache();
        assert_eq!(
            g.feed_journals_desc_through(JournalDate {
                year: 2030,
                month: 7,
                day: 15
            })
            .iter()
            .map(|e| e.date_key)
            .collect::<Vec<_>>(),
            vec![Some(20300715), Some(20300714)]
        );
        assert!(g
            .run_graph_search_latest("future-feed-test", future_title, 8, 8, false)
            .hits
            .iter()
            .any(|hit| matches!(hit,
                crate::query_plan::QueryHit::Page { page, .. } if page.path == future
            )));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn warmed_save_cache_upsert_keeps_future_and_duplicate_days_out_of_feed() {
        let dir = scratch("future-feed-warm-save");
        let g = Graph::open(&dir);
        g.warm_cache();

        let mut past = jdto("Jul 14th, 2030");
        past.blocks[0].raw = "past after warm cache".into();
        g.save_page(&past, None).unwrap();
        let mut today = jdto("Jul 15th, 2030");
        today.blocks[0].raw = "today after warm cache".into();
        g.save_page(&today, None).unwrap();
        let mut future = jdto("Jul 17th, 2030");
        future.blocks[0].raw = "future after warm cache".into();
        g.save_page(&future, None).unwrap();

        let cutoff = JournalDate {
            year: 2030,
            month: 7,
            day: 15,
        };
        assert_eq!(
            g.feed_journals_desc_through(cutoff)
                .iter()
                .map(|e| e.date_key)
                .collect::<Vec<_>>(),
            vec![Some(20300715), Some(20300714)],
            "guarded save/cache-upsert must not leak a future day into warm feed membership"
        );
        assert!(g
            .load_named("Jul 17th, 2030", PageKind::Journal)
            .unwrap()
            .is_some());
        assert!(g.list_pages().iter().any(|e| e.name == "Jul 17th, 2030"));

        // The raw inventory retains duplicate future files for conflict discovery,
        // while date deduplication still leaves no future feed row at all.
        fs::write(dir.join("journals/2030_07_17.org"), "* future twin\n").unwrap();
        let duplicate = Graph::open(&dir);
        assert_eq!(
            duplicate
                .journals_desc()
                .iter()
                .filter(|e| e.date_key == Some(20300717))
                .count(),
            1
        );
        assert!(duplicate
            .feed_journals_desc_through(cutoff)
            .iter()
            .all(|e| e.date_key != Some(20300717)));
        assert!(
            !duplicate.journal_conflicts().is_empty(),
            "future duplicate remains discoverable outside feed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_transaction_moves_file_and_rewrites_refs() {
        let dir = scratch("rename");
        fs::write(dir.join("pages").join("Alpha.md"), "- alpha body\n").unwrap();
        fs::write(dir.join("pages").join("Other.md"), "- see [[Alpha]] here\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        g.rename_page("Alpha", "Beta").unwrap();
        // The page file moved (content preserved) and the old file is gone.
        assert!(!dir.join("pages").join("Alpha.md").exists());
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Beta.md")).unwrap(),
            "- alpha body\n"
        );
        // Every reference was rewritten across the graph.
        let other = fs::read_to_string(dir.join("pages").join("Other.md")).unwrap();
        assert!(other.contains("[[Beta]]"), "ref rewritten to [[Beta]]");
        assert!(!other.contains("[[Alpha]]"), "no stale [[Alpha]] left");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_falls_back_when_a_non_candidate_changed_on_disk() {
        let dir = scratch("rename-stale-non-candidate");
        fs::write(dir.join("pages/Old.md"), "- old body\n").unwrap();
        let referrer = dir.join("pages/Referrer.md");
        fs::write(&referrer, "- unrelated\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let entries = g.list_pages();
        let names = vec![crate::refs::page_key("Old")];
        let initial_candidates = g
            .reference_candidate_paths_for_entries(&names, &entries)
            .expect("warm coherent index should narrow");
        assert!(!initial_candidates.contains(&referrer));

        fs::write(&referrer, "- newly landed [[Old]] reference\n").unwrap();
        assert!(
            g.reference_candidate_paths_for_entries(&names, &entries)
                .is_none(),
            "disk-ahead-of-cache content must force the full rename scan"
        );

        g.rename_page("Old", "New").unwrap();
        let rewritten = fs::read_to_string(&referrer).unwrap();
        assert!(rewritten.contains("[[New]]"));
        assert!(!rewritten.contains("[[Old]]"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_rolls_back_destination_when_source_remove_fails() {
        let dir = scratch("rename-remove-failure");
        let original = "- alpha body\n";
        let ref_original = "- see [[Alpha]] here\n";
        fs::write(dir.join("pages/Alpha.md"), original).unwrap();
        fs::write(dir.join("pages/Other.md"), ref_original).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        FAIL_NEXT_RENAME_SOURCE_REMOVE.with(|flag| flag.set(true));
        WITHDRAW_RACE_REPLACEMENT.with(|replacement| {
            *replacement.borrow_mut() = Some(b"- external replacement\n".to_vec());
        });
        assert!(g.rename_page("Alpha", "Beta").is_err());
        assert_eq!(
            fs::read_to_string(dir.join("pages/Alpha.md")).unwrap(),
            original
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages/Beta.md")).unwrap(),
            "- external replacement\n",
            "rollback must not unlink a destination replaced after its check"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages/Other.md")).unwrap(),
            ref_original
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_namespace_rewrites_all_descendant_refs_in_one_pass() {
        // A namespace rename (`Project` -> `Archive`) moves the primary page AND
        // every file-backed descendant, and rewrites every reference to ANY of
        // them across the graph in a SINGLE multi-target pass per file (perf
        // Codex#2). Default file-name format is Legacy, so `Project/Alpha` lives
        // on disk as `Project%2FAlpha.md`.
        let dir = scratch("rename-ns");
        fs::write(dir.join("pages").join("Project.md"), "- project body\n").unwrap();
        fs::write(
            dir.join("pages").join("Project%2FAlpha.md"),
            "- alpha body\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Project%2FBeta.md"), "- beta body\n").unwrap();
        // One file references the primary AND both descendants (inline) plus two
        // bare `tags::` values — all rewritten in the single multi-target pass.
        fs::write(
            dir.join("pages").join("Refs.md"),
            "tags:: Project, Project/Beta\n- see [[Project]], [[Project/Alpha]] and #[[Project/Beta]]\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        g.rename_page("Project", "Archive").unwrap();

        // Primary + every descendant file moved (content preserved), old names gone.
        assert!(!dir.join("pages").join("Project.md").exists());
        assert!(!dir.join("pages").join("Project%2FAlpha.md").exists());
        assert!(!dir.join("pages").join("Project%2FBeta.md").exists());
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Archive.md")).unwrap(),
            "- project body\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Archive%2FAlpha.md")).unwrap(),
            "- alpha body\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Archive%2FBeta.md")).unwrap(),
            "- beta body\n"
        );

        // Every inline ref AND both bare tag values rewritten; no stale `Project`.
        let refs = fs::read_to_string(dir.join("pages").join("Refs.md")).unwrap();
        assert!(refs.contains("[[Archive]]"), "primary inline ref: {refs:?}");
        assert!(
            refs.contains("[[Archive/Alpha]]"),
            "descendant inline ref: {refs:?}"
        );
        // `Archive/Beta` is bare-tag-safe (`/` is a tag char), so `#[[..]]`
        // collapses to the bare `#Archive/Beta` form, matching Logseq.
        assert!(
            refs.contains("#Archive/Beta"),
            "descendant tag ref: {refs:?}"
        );
        assert!(
            refs.contains("tags:: Archive, Archive/Beta"),
            "bare tags rewritten: {refs:?}"
        );
        assert!(
            !refs.contains("Project"),
            "no stale Project anywhere: {refs:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn org_page_lists_loads_edits_and_round_trips() {
        let dir = scratch("org-page");
        let src = "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block\n";
        fs::write(dir.join("pages").join("Org Notes.org"), src).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        // Listed, recognized as an org page.
        let entry = g
            .list_pages()
            .into_iter()
            .find(|e| e.name == "Org Notes")
            .expect("org page listed");
        assert_eq!(Format::from_path(&entry.path), Format::Org);

        // Loaded: format=org, editable, headlines decomposed into blocks.
        let dto = g.load_named("Org Notes", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.format, Format::Org);
        assert!(!dto.read_only);
        assert_eq!(dto.blocks.len(), 2);
        assert_eq!(
            dto.blocks[0].raw,
            "TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>"
        );
        assert_eq!(dto.blocks[1].raw, "second block");

        // No-op save leaves the file byte-identical (no churn).
        let rev = g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Org Notes.org")).unwrap(),
            src
        );

        // Edit a block and save → file updated, still org, byte-faithful.
        let mut edited = dto.clone();
        edited.blocks[1].raw = "second block edited".into();
        g.save_page(&edited, Some(&rev)).unwrap();
        let on_disk = fs::read_to_string(dir.join("pages").join("Org Notes.org")).unwrap();
        assert_eq!(
            on_disk,
            "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block edited\n"
        );
        // No stray .md twin was created.
        assert!(!dir.join("pages").join("Org Notes.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn guide_flagged_pages_are_never_written_to_graph_files() {
        let dir = scratch("guide-no-save");
        let g = Graph::open(&dir);
        let page = PageDto {
            name: "Tine-guide/Features/Sheets".into(),
            kind: PageKind::Page,
            title: "Features/Sheets".into(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: "guide-block".into(),
                raw: "This is an ephemeral guide block".into(),
                collapsed: false,
                ..Default::default()
            }],
            rev: None,
            format: Format::Md,
            read_only: true,
            path: String::new(),
            guide: true,
        };

        assert_eq!(g.save_page(&page, None).unwrap(), "guide-ephemeral");
        assert_eq!(g.force_save_page(&page).unwrap(), "guide-ephemeral");
        let files: Vec<_> = fs::read_dir(dir.join("pages"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            files.is_empty(),
            "guide save guard must be load-bearing; wrote files: {files:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn org_journal_recognized_and_listed() {
        let dir = scratch("org-journal");
        fs::write(
            dir.join("journals").join("2026_06_24.org"),
            "* woke up\n* TODO ship\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        let j = g
            .journals_desc()
            .into_iter()
            .find(|e| e.kind == PageKind::Journal)
            .expect("org journal listed");
        assert_eq!(Format::from_path(&j.path), Format::Org);
        assert!(j.date_key.is_some(), "journal date parsed from .org stem");
        let dto = g.load_page(&j).unwrap();
        assert_eq!(dto.blocks.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_round_trip_org_is_read_only_and_save_refused() {
        let dir = scratch("org-ro");
        // Skipped heading level (`*` then `***`) cannot be reproduced from tree
        // depth → not round-trip safe → must load read-only and refuse writes.
        let src = "* a\n*** c\n";
        fs::write(dir.join("pages").join("Weird.org"), src).unwrap();
        let g = Graph::open(&dir);
        let dto = g.load_named("Weird", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.format, Format::Org);
        assert!(dto.read_only, "non-round-tripping org loads read-only");
        // Even a forced save must refuse (defense in depth) and leave bytes intact.
        let err = g.force_save_page(&dto).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Weird.org")).unwrap(),
            src
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn force_save_refuses_unreadable_existing_bytes() {
        let dir = scratch("force-save-invalid-utf8");
        let path = dir.join("pages").join("A.md");
        fs::write(&path, "- original\n").unwrap();
        let g = Graph::open(&dir);
        let mut dto = g.load_named("A", PageKind::Page).unwrap().unwrap();
        dto.blocks[0].raw = "replacement".into();
        let unknown = b"\xff\xfeunknown on-disk bytes";
        fs::write(&path, unknown).unwrap();

        let err = g.force_save_page(&dto).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), unknown);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn twin_md_org_refuses_writes() {
        // M1: a page that exists as BOTH Foo.md and Foo.org is ambiguous — save,
        // force-save, rename, and delete must all refuse (no clobber of either).
        let dir = scratch("org-twin");
        fs::write(dir.join("pages").join("Foo.md"), "- md body\n").unwrap();
        fs::write(dir.join("pages").join("Foo.org"), "* org body\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let page = PageDto {
            name: "Foo".into(),
            kind: PageKind::Page,
            title: "Foo".into(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: "x".into(),
                raw: "edited".into(),
                ..Default::default()
            }],
            rev: None,
            format: Format::Md,
            read_only: false,
            path: String::new(),
            guide: false,
        };
        assert!(g.save_page(&page, None).is_err(), "save refused on twin");
        assert!(
            g.force_save_page(&page).is_err(),
            "force_save refused on twin"
        );
        assert!(
            g.rename_page("Foo", "Bar").is_err(),
            "rename refused on twin"
        );
        assert!(
            g.delete_page("Foo", PageKind::Page).is_err(),
            "delete refused on twin"
        );
        // Both files are byte-intact (nothing was written/moved/trashed).
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Foo.md")).unwrap(),
            "- md body\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Foo.org")).unwrap(),
            "* org body\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn readonly_org_unchanged_does_not_reconcile() {
        // L2 check: an UNCHANGED read-only (non-round-tripping) .org file must not
        // spuriously reconcile (bump cache_gen) on a watcher tick — the disk_revs
        // fast path + structural normalize-compare should both treat it as "ours".
        let dir = scratch("org-ro-l2");
        let src = "* a\n*** c\n"; // skipped heading level → read-only
        let path = dir.join("pages").join("RO.org");
        fs::write(&path, src).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        // Confirm it loaded read-only.
        let dto = g.load_named("RO", PageKind::Page).unwrap().unwrap();
        assert!(dto.read_only);
        let gen0 = g.cache_generation();
        // Two watcher reconciles of the unchanged file must be no-ops.
        g.sync_file(&path);
        g.sync_file(&path);
        assert_eq!(
            g.cache_generation(),
            gen0,
            "unchanged read-only org reconciled spuriously"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn orphan_assets_lists_only_unreferenced_media() {
        let dir = scratch("orphans");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        // Referenced by blocks (kept): an image, a pdf, a spaced-name clip.
        fs::write(assets.join("used.png"), b"x").unwrap();
        fs::write(assets.join("paper.pdf"), b"x").unwrap();
        fs::write(assets.join("my clip.mp4"), b"x").unwrap();
        // Not referenced (orphans).
        fs::write(assets.join("stray.png"), b"x").unwrap();
        fs::write(assets.join("old_video.webm"), b"x").unwrap();
        // Sidecars / non-media — never flagged.
        fs::write(assets.join("paper.edn"), b"{}").unwrap();
        fs::create_dir_all(assets.join("paper")).unwrap(); // PDF area-image dir
        fs::write(assets.join("paper").join("1_a_2.png"), b"x").unwrap();
        fs::write(
            dir.join("pages").join("P.md"),
            "- ![](../assets/used.png)\n- [paper](../assets/paper.pdf)\n- ![](../assets/my clip.mp4)\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        let orphans: Vec<String> = g.orphan_assets().into_iter().map(|a| a.name).collect();
        assert_eq!(
            orphans,
            vec!["old_video.webm".to_string(), "stray.png".to_string()]
        );
        // Trash one → it moves out of assets/ into the recoverable trash.
        g.trash_asset("stray.png").unwrap();
        assert!(!assets.join("stray.png").exists());
        assert!(dir.join("logseq").join(".tine-trash").exists());
        // A name with a separator is refused (can't escape assets/).
        assert!(g.trash_asset("../pages/P.md").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn orphan_assets_does_not_flag_percent_encoded_in_use_asset() {
        // A block links `../assets/my%20file.png` but the file on disk is named
        // `my file.png` (the space percent-encoded in the URL, valid Markdown).
        // The scanner must percent-decode the reference before comparing, so the
        // in-use file is NOT offered for trashing (DS Codex#7).
        let dir = scratch("orphan-pct");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("my file.png"), b"x").unwrap(); // referenced via %20
        fs::write(assets.join("real orphan.png"), b"x").unwrap(); // genuinely unused
        fs::write(
            dir.join("pages").join("P.md"),
            "- ![pic](../assets/my%20file.png)\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        let orphans: Vec<String> = g.orphan_assets().into_iter().map(|a| a.name).collect();
        assert_eq!(
            orphans,
            vec!["real orphan.png".to_string()],
            "the percent-encoded in-use asset must not be flagged orphan"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_asset_trash_clears_trashed_files() {
        let dir = scratch("empty-trash");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("junk1.png"), b"xx").unwrap(); // 2 bytes
        fs::write(assets.join("junk2.png"), b"yyy").unwrap(); // 3 bytes
        let g = Graph::open(&dir);
        g.trash_asset("junk1.png").unwrap();
        g.trash_asset("junk2.png").unwrap();
        let s = g.asset_trash_stats();
        assert_eq!(s.count, 2, "two files in trash");
        assert_eq!(s.bytes, 5, "2 + 3 bytes preserved through the move");
        assert_eq!(g.empty_asset_trash().unwrap(), 2, "both removed");
        assert_eq!(g.asset_trash_stats().count, 0, "trash empty afterwards");
        // Emptying a never-created trash is a no-op, not an error.
        let dir2 = scratch("empty-trash-missing");
        assert_eq!(Graph::open(&dir2).empty_asset_trash().unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn empty_asset_trash_keeps_legacy_trashed_pages() {
        let dir = scratch("empty-trash-keeps-pages");
        let trash = dir.join("logseq").join(".tine-trash");
        fs::create_dir_all(&trash).unwrap();
        let asset = trash.join("123-0__unused.png");
        let page = trash.join("123-1__Recovered Page.md");
        fs::write(&asset, b"img").unwrap();
        fs::write(&page, b"- recovered page\n").unwrap();

        let g = Graph::open(&dir);
        let stats = g.asset_trash_stats();
        assert_eq!(stats.count, 1, "legacy asset trash is asset-counted");
        assert_eq!(stats.pages, 1, "legacy page trash is protected-counted");
        assert_eq!(g.empty_asset_trash().unwrap(), 1);
        assert!(
            !asset.exists(),
            "legacy asset trash entry should be deleted"
        );
        assert!(page.exists(), "legacy page trash entry must survive");
        let stats = g.asset_trash_stats();
        assert_eq!(stats.count, 0, "asset trash should be empty");
        assert_eq!(stats.pages, 1, "page trash should still be counted");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_asset_uses_given_name() {
        let dir = scratch("import-name");
        let src = dir.join("source.png");
        fs::write(&src, b"img").unwrap();
        let g = Graph::open(&dir);
        let saved = g
            .import_asset(&src, Some("source_20260626_120000.png"))
            .unwrap();
        assert_eq!(saved, "source_20260626_120000.png");
        assert!(dir.join("assets").join(&saved).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_asset_limited_rejects_before_returning_oversized_bytes() {
        let dir = scratch("read-asset-limited");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("large.pdf"), b"12345").unwrap();
        let g = Graph::open(&dir);
        assert_eq!(g.read_asset_limited("large.pdf", 5).unwrap(), b"12345");
        let err = g.read_asset_limited("large.pdf", 4).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("asset exceeds 4 byte limit"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_aborts_on_readonly_org_referrer() {
        // H1: a rename must NOT rewrite a read-only (non-round-tripping) .org file.
        let dir = scratch("org-rename-ro");
        fs::write(dir.join("pages").join("Alpha.md"), "- alpha\n").unwrap();
        // `* a\n*** c` skips a heading level → not round-trip-safe → read-only.
        let ro = "* a\n*** c referencing [[Alpha]]\n";
        fs::write(dir.join("pages").join("Weird.org"), ro).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let err = g.rename_page("Alpha", "Beta").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        // All-or-nothing: neither file moved/changed.
        assert!(
            dir.join("pages").join("Alpha.md").exists(),
            "rename rolled back"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Weird.org")).unwrap(),
            ro
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_org_skips_refs_in_src_block() {
        // H2 end-to-end: renaming a page leaves a `[[Old]]` literal inside an org
        // src block untouched while rewriting a real ref outside it.
        let dir = scratch("org-rename-src");
        fs::write(dir.join("pages").join("Old.md"), "- old body\n").unwrap();
        let org = "* note\nsee [[Old]]\n#+BEGIN_SRC clojure\n\"[[Old]]\"\n#+END_SRC\n";
        fs::write(dir.join("pages").join("Ref.org"), org).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        g.rename_page("Old", "New").unwrap();
        let got = fs::read_to_string(dir.join("pages").join("Ref.org")).unwrap();
        assert_eq!(
            got,
            "* note\nsee [[New]]\n#+BEGIN_SRC clojure\n\"[[Old]]\"\n#+END_SRC\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn org_save_with_typed_headline_caches_disk_tree() {
        // H4: typing a column-0 `* ` line into a block body makes the saved bytes
        // re-parse to a DIFFERENT tree; the cache must reflect what's on disk, not
        // the (now-stale) frontend doc — so reads after the save see the real shape.
        let dir = scratch("org-h4");
        fs::write(dir.join("pages").join("P.org"), "* one\n* two\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let dto = g.load_named("P", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.blocks.len(), 2);
        // Edit block 0's body to contain a column-0 headline marker.
        let mut edited = dto.clone();
        edited.blocks[0].raw = "one\n* injected".into();
        let rev = g.save_page(&edited, dto.rev.as_deref()).unwrap();
        // Disk now has THREE headlines.
        let disk = fs::read_to_string(dir.join("pages").join("P.org")).unwrap();
        assert_eq!(disk, "* one\n* injected\n* two\n");
        // A fresh load (served from cache) must reflect the 3-block disk structure,
        // not the 2-block frontend doc that produced it.
        let again = g.load_named("P", PageKind::Page).unwrap().unwrap();
        assert_eq!(
            again.blocks.len(),
            3,
            "cache reflects disk structure after H4 reparse"
        );
        assert_eq!(again.rev.as_deref(), Some(rev.as_str()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_page_uses_preferred_format_org() {
        let dir = scratch("org-pref");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        assert_eq!(g.preferred_format(), Format::Org);
        // Create a brand-new page via save (no baseline) — it must land as .org.
        let page = PageDto {
            name: "Fresh".into(),
            kind: PageKind::Page,
            title: "Fresh".into(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: "x".into(),
                raw: "hello org".into(),
                ..Default::default()
            }],
            rev: None,
            format: Format::Org,
            read_only: false,
            path: String::new(),
            guide: false,
        };
        g.save_page(&page, None).unwrap();
        assert!(
            dir.join("pages").join("Fresh.org").exists(),
            "new page created as .org"
        );
        assert!(!dir.join("pages").join("Fresh.md").exists());
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("Fresh.org")).unwrap(),
            "* hello org\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_skips_rewrite_when_only_whitespace_trivia_differs() {
        // The file has an empty bullet written `- ` (trailing space); the
        // serializer would re-emit it as `-`. A5: a load→save with no real edit
        // must NOT rewrite the file (no Syncthing churn) and must not bump the
        // cache generation — the parsed structure is identical.
        let dir = scratch("noop");
        let path = dir.join("pages").join("A.md");
        let original = "- a\n- \n"; // second bullet: dash + trailing space
        fs::write(&path, original).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let entry = g.find_entry("A", PageKind::Page).unwrap();
        let dto = g.load_page(&entry).unwrap();
        let gen_before = g.cache_generation();
        let rev = g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original,
            "bytes left untouched"
        );
        assert_eq!(
            rev,
            content_rev(original),
            "returned rev is the on-disk rev"
        );
        assert_eq!(
            g.cache_generation(),
            gen_before,
            "no cache_gen bump on a trivia-only no-op"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refuses_page_header_properties_reclassified_as_outline() {
        // GH #163's v0.5.9 Windows follow-up.  The pure property-line helper was
        // innocent; the damaging shape arrived at the native save boundary.
        // Prove that even a contradictory frontend DTO cannot turn B/C into a
        // bullet and continuation line, for either common line-ending family.
        for (label, original) in [
            ("lf", "A:: XX\nB:: XX\nC:: XX\n"),
            ("crlf", "A:: XX\r\nB:: XX\r\nC:: XX\r\n"),
            ("unicode", "A:: XX\nklíč:: hodnota\nC:: XX\n"),
        ] {
            let dir = scratch(&format!("page-property-firewall-{label}"));
            let path = dir.join("pages").join("Property.md");
            fs::write(&path, original).unwrap();
            let g = Graph::open(&dir);
            let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
            let normalized = original.replace("\r\n", "\n");
            let normalized = normalized.trim_end_matches('\n');
            assert_eq!(dto.pre_block.as_deref(), Some(normalized));
            assert!(dto.blocks.is_empty());

            let (kept, moved) = normalized.split_once('\n').unwrap();
            dto.pre_block = Some(kept.into());
            dto.blocks = vec![BlockDto {
                id: "corrupt-shape".into(),
                raw: moved.into(),
                ..Default::default()
            }];

            let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("page-header property"));
            assert_eq!(fs::read_to_string(&path).unwrap(), original);

            let err = g.force_save_page(&dto).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn save_refuses_changed_page_header_properties_reclassified_as_outline() {
        // H7: the preservation firewall is structural, not an exact-text check.
        // A stale/buggy DTO must not evade it by changing the moved property's
        // value or key while reclassifying it as outline content. Exercise both
        // ordinary and force-save paths from a warm cache and prove neither the
        // bytes nor cached document move on validation failure.
        for (shape, original, kept, moved, childful) in [
            (
                "partial-value",
                "A:: old\nB:: old\n",
                Some("A:: old"),
                "B:: changed",
                false,
            ),
            (
                "partial-key",
                "A:: old\nB:: old\n",
                Some("A:: old"),
                "Renamed:: old",
                false,
            ),
            (
                "whole-key-value",
                "A:: old\nB:: old\n",
                None,
                "Renamed:: changed\nC:: newer",
                true,
            ),
            (
                "crlf",
                "A:: old\r\nB:: old\r\n",
                Some("A:: old"),
                "B:: changed",
                false,
            ),
            (
                "unicode-plugin",
                "A:: old\n插件/键:: old\n",
                Some("A:: old"),
                "插件/新:: changed",
                false,
            ),
        ] {
            for forced in [false, true] {
                let dir = scratch(&format!("page-property-firewall-changed-{shape}-{forced}"));
                let path = dir.join("pages").join("Property.md");
                fs::write(&path, original).unwrap();
                let g = Graph::open(&dir);
                g.warm_cache();
                let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
                let cached_before = dto.clone();
                let generation_before = g.cache_generation();
                dto.pre_block = kept.map(str::to_string);
                dto.blocks = vec![BlockDto {
                    id: "reclassified-header".into(),
                    raw: moved.into(),
                    children: childful
                        .then(|| BlockDto {
                            id: "body".into(),
                            raw: "Body".into(),
                            ..Default::default()
                        })
                        .into_iter()
                        .collect(),
                    ..Default::default()
                }];

                let err = if forced {
                    g.force_save_page(&dto).unwrap_err()
                } else {
                    g.save_page(&dto, dto.rev.as_deref()).unwrap_err()
                };
                assert_eq!(err.kind(), io::ErrorKind::InvalidData);
                assert_eq!(fs::read_to_string(&path).unwrap(), original);
                assert_eq!(g.cache_generation(), generation_before);
                let cached_after = g.load_named("Property", PageKind::Page).unwrap().unwrap();
                assert_eq!(cached_after.pre_block, cached_before.pre_block);
                assert_eq!(cached_after.blocks.len(), cached_before.blocks.len());
                assert_eq!(cached_after.rev, cached_before.rev);
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }

    #[test]
    fn existing_outline_property_root_remains_editable_beside_page_header() {
        // An outline block that already had page-property-shaped syntax is not a
        // reclassified header. Its structural provenance permits a duplicate
        // header line to be deleted without blaming the already-existing root,
        // and the root remains ordinarily editable afterwards.
        let dir = scratch("page-property-existing-outline-provenance");
        let path = dir.join("pages").join("Property.md");
        fs::write(&path, "A:: header\nB:: shared\n\n- B:: shared\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
        dto.pre_block = Some("A:: edited header".into());
        g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "A:: edited header\n\n- B:: shared\n"
        );
        let mut warm = g.load_named("Property", PageKind::Page).unwrap().unwrap();
        assert_eq!(warm.pre_block.as_deref(), Some("A:: edited header"));
        assert_eq!(warm.blocks[0].raw, "B:: shared");
        warm.blocks[0].raw = "Renamed:: edited outline".into();
        g.save_page(&warm, warm.rev.as_deref()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "A:: edited header\n\n- Renamed:: edited outline\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_header_property_save_reopens_as_metadata_with_original_line_endings() {
        // Complements the real gear-panel E2E: drive the native save and a fresh
        // Graph/parser instance so success cannot come from the just-written
        // frontend store or Graph cache.
        for (label, original, expected) in [
            (
                "lf",
                "A:: XX\nB:: XX\nC:: XX\n",
                "icon:: ★\nA:: XX\nB:: XX\nC:: XX\n",
            ),
            (
                "crlf",
                "A:: XX\r\nB:: XX\r\nC:: XX\r\n",
                "icon:: ★\r\nA:: XX\r\nB:: XX\r\nC:: XX\r\n",
            ),
        ] {
            let dir = scratch(&format!("page-property-positive-{label}"));
            let path = dir.join("pages").join("Property.md");
            fs::write(&path, original).unwrap();
            let g = Graph::open(&dir);
            let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
            dto.pre_block = Some("icon:: ★\nA:: XX\nB:: XX\nC:: XX".into());
            g.save_page(&dto, dto.rev.as_deref()).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
            drop(g);

            let reopened = Graph::open(&dir)
                .load_named("Property", PageKind::Page)
                .unwrap()
                .unwrap();
            assert_eq!(
                reopened.pre_block.as_deref(),
                Some("icon:: ★\nA:: XX\nB:: XX\nC:: XX")
            );
            assert!(reopened.blocks.is_empty());
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn new_property_only_first_root_becomes_canonical_page_header() {
        let dir = scratch("page-property-authoring");
        let g = Graph::open(&dir);
        g.warm_cache();
        let page = PageDto {
            name: "Property Authoring".into(),
            kind: PageKind::Page,
            title: "Property Authoring".into(),
            pre_block: None,
            blocks: vec![
                BlockDto {
                    id: "transient-header".into(),
                    raw: "alias:: book\n\nklíč:: hodnota".into(),
                    ..Default::default()
                },
                BlockDto {
                    id: "body".into(),
                    raw: "Reading list".into(),
                    ..Default::default()
                },
            ],
            rev: None,
            format: Format::Md,
            read_only: false,
            path: String::new(),
            guide: false,
        };
        g.save_page(&page, None).unwrap();
        let path = dir.join("pages").join("Property Authoring.md");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "alias:: book\n\nklíč:: hodnota\n\n- Reading list\n"
        );

        let warm = g
            .load_named("Property Authoring", PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(
            warm.pre_block.as_deref(),
            Some("alias:: book\n\nklíč:: hodnota")
        );
        assert_eq!(warm.blocks.len(), 1);
        assert_eq!(warm.blocks[0].raw, "Reading list");
        assert_eq!(
            warm.blocks[0].id, "body",
            "normalization changed the body root identity"
        );
        drop(g);
        let cold = Graph::open(&dir)
            .load_named("Property Authoring", PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(cold.pre_block, warm.pre_block);
        assert_eq!(cold.blocks.len(), warm.blocks.len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gh198_canonical_preamble_dto_resaves_cleanly_over_existing_preamble() {
        // GH #198 persistence-boundary complement. The store fix (pageToDto folds
        // a flagless properties-only first bullet into pre_block) makes the frontend
        // emit pre_block=properties + no bullet. Prove that this corrected DTO
        // shape resaves without tripping the GH #163 preservation firewall even
        // when disk already carries the identical unbulleted preamble — the exact
        // second-save that previously jammed the queue with "will retry".
        let dir = scratch("gh198-canonical-resave");
        let path = dir.join("pages").join("The Nazi Mind.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "title:: The Nazi Mind\ntags:: books\n").unwrap();
        let g = Graph::open(&dir);
        let loaded = g
            .load_named("The Nazi Mind", PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.pre_block.as_deref(),
            Some("title:: The Nazi Mind\ntags:: books")
        );
        assert!(loaded.blocks.is_empty());

        let dto = PageDto {
            name: "The Nazi Mind".into(),
            kind: PageKind::Page,
            title: "The Nazi Mind".into(),
            pre_block: Some("title:: The Nazi Mind\ntags:: books".into()),
            blocks: vec![],
            rev: None,
            format: Format::Md,
            read_only: false,
            path: String::new(),
            guide: false,
        };
        g.save_page(&dto, loaded.rev.as_deref())
            .expect("corrected canonical-preamble DTO must save over an existing preamble");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "title:: The Nazi Mind\ntags:: books\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_header_authoring_is_bounded_and_preserves_existing_preambles() {
        assert!(page_header_properties_only(
            "alias:: book\n\ne\u{301}/plugin.key::value"
        ));
        for invalid in [
            " alias:: x",
            "#alias:: x",
            "alias key:: x",
            "alias:: x\nprose",
            "```\nalias:: x\n```",
            "alias:: x\n",
        ] {
            assert!(
                !page_header_properties_only(invalid),
                "accepted {invalid:?}"
            );
        }

        // A headerless CRLF page can add a canonical header; both the warm cache
        // and a fresh parser expose exactly the normalized document shape.
        let dir = scratch("page-property-existing-headerless");
        let path = dir.join("pages").join("Existing.md");
        fs::write(&path, "- Body\r\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let mut dto = g.load_named("Existing", PageKind::Page).unwrap().unwrap();
        dto.blocks.insert(
            0,
            BlockDto {
                id: "transient-header".into(),
                raw: "custom/key:: exact value".into(),
                ..Default::default()
            },
        );
        g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "custom/key:: exact value\r\n\r\n- Body\r\n"
        );
        let warm = g.load_named("Existing", PageKind::Page).unwrap().unwrap();
        assert_eq!(warm.pre_block.as_deref(), Some("custom/key:: exact value"));
        assert_eq!(warm.blocks.len(), 1);
        drop(g);
        let cold = Graph::open(&dir)
            .load_named("Existing", PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(cold.pre_block, warm.pre_block);
        assert_eq!(cold.blocks.len(), warm.blocks.len());
        let _ = fs::remove_dir_all(&dir);

        // A non-property preamble may only move through GH #85's explicit prose
        // promotion. A property candidate cannot make that preamble disappear,
        // even through force-save, and the warm cache stays on the disk version.
        let dir = scratch("page-property-preamble-loss");
        let path = dir.join("pages").join("Imported.md");
        let original = "Intro before outline\n\n- Body\n";
        fs::write(&path, original).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let mut dto = g.load_named("Imported", PageKind::Page).unwrap().unwrap();
        dto.pre_block = None;
        dto.blocks.insert(
            0,
            BlockDto {
                id: "candidate".into(),
                raw: "alias:: book".into(),
                ..Default::default()
            },
        );
        for forced in [false, true] {
            let err = if forced {
                g.force_save_page(&dto).unwrap_err()
            } else {
                g.save_page(&dto, dto.rev.as_deref()).unwrap_err()
            };
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("existing page preamble"));
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            let cached = g.load_named("Imported", PageKind::Page).unwrap().unwrap();
            assert_eq!(cached.pre_block.as_deref(), Some("Intro before outline"));
            assert_eq!(cached.blocks.len(), 1);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_header_authoring_never_promotes_unsafe_or_nonfirst_roots() {
        let cases: Vec<(&str, Format, Vec<BlockDto>)> = vec![
            (
                "later",
                Format::Md,
                vec![
                    BlockDto {
                        id: "body".into(),
                        raw: "Body".into(),
                        ..Default::default()
                    },
                    BlockDto {
                        id: "prop".into(),
                        raw: "alias:: book".into(),
                        ..Default::default()
                    },
                ],
            ),
            (
                "mixed",
                Format::Md,
                vec![BlockDto {
                    id: "mixed".into(),
                    raw: "alias:: book\nprose".into(),
                    ..Default::default()
                }],
            ),
            (
                "fenced",
                Format::Md,
                vec![BlockDto {
                    id: "fenced".into(),
                    raw: "```\nalias:: book\n```".into(),
                    ..Default::default()
                }],
            ),
            (
                "empty",
                Format::Md,
                vec![BlockDto {
                    id: "empty".into(),
                    raw: "".into(),
                    ..Default::default()
                }],
            ),
            (
                "childful",
                Format::Md,
                vec![BlockDto {
                    id: "parent".into(),
                    raw: "alias:: book".into(),
                    children: vec![BlockDto {
                        id: "child".into(),
                        raw: "Child".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
            ),
            (
                "id-bearing",
                Format::Md,
                vec![BlockDto {
                    id: "durable".into(),
                    raw: "id:: 11111111-1111-4111-8111-111111111111".into(),
                    ..Default::default()
                }],
            ),
            (
                "org",
                Format::Org,
                vec![BlockDto {
                    id: "org".into(),
                    raw: "alias:: book".into(),
                    ..Default::default()
                }],
            ),
        ];
        for (label, format, blocks) in cases {
            let dir = scratch(&format!("page-property-negative-{label}"));
            if format == Format::Org {
                fs::create_dir_all(dir.join("logseq")).unwrap();
                fs::write(
                    dir.join("logseq").join("config.edn"),
                    "{:preferred-format \"Org\"}\n",
                )
                .unwrap();
            }
            let g = Graph::open(&dir);
            let page = PageDto {
                name: format!("Negative {label}"),
                kind: PageKind::Page,
                title: format!("Negative {label}"),
                pre_block: None,
                blocks: blocks.clone(),
                rev: None,
                format,
                read_only: false,
                path: String::new(),
                guide: false,
            };
            g.save_page(&page, None).unwrap();
            let reopened = g.load_named(&page.name, PageKind::Page).unwrap().unwrap();
            assert!(reopened.pre_block.is_none(), "promoted unsafe case {label}");
            assert_eq!(
                reopened.blocks.len(),
                blocks.len(),
                "changed root count for {label}"
            );
            if label == "id-bearing" {
                assert!(
                    g.resolve_block("11111111-1111-4111-8111-111111111111")
                        .is_some(),
                    "ID-bearing root lost addressability"
                );
            }
            let _ = fs::remove_dir_all(&dir);
        }
    }

    fn mkhl(id: &str, page: i64, text: Option<&str>) -> crate::pdf::Highlight {
        let r = crate::pdf::Rect {
            top: 1.0,
            left: 2.0,
            width: 3.0,
            height: 4.0,
            source_width: None,
            source_height: None,
        };
        crate::pdf::Highlight {
            id: id.into(),
            page,
            position: crate::pdf::Position {
                page,
                bounding: r.clone(),
                rects: vec![r],
            },
            color: "yellow".into(),
            text: text.map(String::from),
            image: None,
        }
    }

    #[test]
    fn write_highlights_refuses_unreadable_artifacts_without_partial_commit() {
        let dir = scratch("highlights-invalid-utf8");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let edn_path = dir.join("assets").join(format!("{key}.edn"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        let unknown = b"\xff\xfeunknown sidecar bytes";
        fs::write(&edn_path, unknown).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let err = g
            .write_highlights("paper.pdf", "Paper", &[h], &[])
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&edn_path).unwrap(), unknown);
        assert!(!dir.join("pages").join(format!("hls__{key}.md")).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_pdf_creates_og_artifacts_in_preferred_org_format() {
        let dir = scratch("pdf-open-org");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        let state = g.open_pdf("paper.pdf", "Paper").unwrap();
        assert!(state.highlights.is_empty());
        assert_eq!(state.page, None);
        assert_eq!(state.scale, None);

        let sidecar = fs::read_to_string(dir.join("assets").join("paper.edn")).unwrap();
        assert_eq!(crate::pdf::parse_pdf_state(&sidecar), state);
        let org_path = dir.join("pages").join("hls__paper.org");
        assert!(org_path.exists());
        assert!(!dir.join("pages").join("hls__paper.md").exists());
        let org = fs::read_to_string(org_path).unwrap();
        assert!(
            org.contains("#+FILE: [[../assets/paper.pdf][Paper]]"),
            "{org}"
        );
        assert!(org.contains("#+FILE-PATH: ../assets/paper.pdf"), "{org}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_view_state_update_preserves_highlights_and_foreign_edn() {
        let dir = scratch("pdf-view-state");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("text"));
        let original = crate::pdf::write_highlights(&[h.clone()], "{:extra {:plugin \"keep\"}}");
        fs::write(&sidecar_path, original).unwrap();

        g.write_pdf_view_state("paper.pdf", 8, 1.9).unwrap();

        let written = fs::read_to_string(&sidecar_path).unwrap();
        let state = crate::pdf::parse_pdf_state(&written);
        assert_eq!(state.highlights, vec![h]);
        assert_eq!(state.page, Some(8));
        assert_eq!(state.scale, Some(1.9));
        let root = crate::edn::parse_strict(&written).unwrap();
        assert_eq!(
            root.get("extra")
                .unwrap()
                .get("plugin")
                .and_then(crate::edn::Edn::as_str),
            Some("keep")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn highlight_write_keeps_existing_hls_format_and_uses_org_drawers() {
        let dir = scratch("pdf-highlight-org");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        let h = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("text"));
        g.write_highlights("paper.pdf", "Paper", &[h], &[]).unwrap();
        let org_path = dir.join("pages").join("hls__paper.org");
        let org = fs::read_to_string(&org_path).unwrap();
        assert!(org.contains("* text"), "{org}");
        assert!(org.contains(":PROPERTIES:"), "{org}");
        assert!(org.contains(":hl-page: 3"), "{org}");
        assert!(crate::org::org_round_trips(&org));

        // Preferred format changes later must not fork the existing annotation
        // page into a second extension.
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Markdown\"}\n",
        )
        .unwrap();
        let reopened = Graph::open(&dir);
        let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("more"));
        reopened
            .write_highlights("paper.pdf", "Paper", &[h2], &[])
            .unwrap();
        assert!(org_path.exists());
        assert!(!dir.join("pages").join("hls__paper.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_highlights_checks_notes_page_before_sidecar_commit() {
        let dir = scratch("highlights-invalid-page");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let page_path = dir.join("pages").join(format!("hls__{key}.md"));
        let unknown = b"\xff\xfeunknown notes bytes";
        fs::write(&page_path, unknown).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let err = g
            .write_highlights("paper.pdf", "Paper", &[h], &[])
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&page_path).unwrap(), unknown);
        assert!(!dir.join("assets").join(format!("{key}.edn")).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_highlights_checks_read_only_org_page_before_sidecar_commit() {
        let dir = scratch("highlights-readonly-org-page");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"}\n",
        )
        .unwrap();
        let key = crate::pdf::asset_key("paper.pdf");
        let page_path = dir.join("pages").join(format!("hls__{key}.org"));
        fs::write(&page_path, "* a\n*** c\n").unwrap();
        let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        let original = "{:highlights [] :extra {:plugin \"keep\"}}\n";
        fs::write(&sidecar_path, original).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let err = Graph::open(&dir)
            .write_highlights("paper.pdf", "Paper", &[h], &[])
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&sidecar_path).unwrap(), original);
        assert_eq!(fs::read_to_string(&page_path).unwrap(), "* a\n*** c\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_highlights_rolls_back_sidecar_when_notes_page_commit_fails() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("highlights-page-commit-rollback");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let page_path = dir.join("pages").join(format!("hls__{key}.md"));
        let page_before = "- Existing annotation note\n";
        fs::write(&page_path, page_before).unwrap();
        let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        let sidecar_before = "{:highlights [] :extra {:plugin \"keep\"}}\n";
        fs::write(&sidecar_path, sidecar_before).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let pages = dir.join("pages");
        let original_permissions = fs::metadata(&pages).unwrap().permissions();
        let mut read_only = original_permissions.clone();
        read_only.set_mode(0o555);
        fs::set_permissions(&pages, read_only).unwrap();
        let result = g.write_highlights("paper.pdf", "Paper", &[h], &[]);
        fs::set_permissions(&pages, original_permissions).unwrap();

        assert!(
            result.is_err(),
            "the notes-page commit must fail in a read-only directory"
        );
        assert_eq!(fs::read_to_string(&sidecar_path).unwrap(), sidecar_before);
        assert_eq!(fs::read_to_string(&page_path).unwrap(), page_before);
        assert!(
            !g.recent_writes.lock().unwrap().contains_key(&page_path),
            "a failed page commit must not leave a stale watcher suppression marker"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_highlights_quarantines_new_sidecar_when_notes_page_commit_fails() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("highlights-new-sidecar-page-failure");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let page_path = dir.join("pages").join(format!("hls__{key}.md"));
        let page_before = "- Existing annotation note\n";
        fs::write(&page_path, page_before).unwrap();
        fs::create_dir_all(dir.join("assets")).unwrap();
        let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let pages = dir.join("pages");
        let original_permissions = fs::metadata(&pages).unwrap().permissions();
        let mut read_only = original_permissions.clone();
        read_only.set_mode(0o555);
        fs::set_permissions(&pages, read_only).unwrap();
        let result = g.write_highlights("paper.pdf", "Paper", &[h], &[]);
        fs::set_permissions(&pages, original_permissions).unwrap();

        assert!(result.is_err());
        assert!(
            !sidecar_path.exists(),
            "the failed pair must leave the primary target absent"
        );
        assert_eq!(fs::read_to_string(&page_path).unwrap(), page_before);
        let trash = typed_trash_dir(&dir, TrashEntryKind::Conflict);
        assert!(
            fs::read_dir(trash).unwrap().flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("failed-highlight-pair")
            }),
            "the exact new sidecar remains recoverable in conflict trash"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_highlights_preserves_malformed_utf8_sidecar() {
        let dir = scratch("highlights-malformed-edn");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let edn_path = dir.join("assets").join(format!("{key}.edn"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        let malformed = "{:highlights [BROKEN :sentinel \"keep me\"";
        fs::write(&edn_path, malformed).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let err = g
            .write_highlights("paper.pdf", "Paper", &[h], &[])
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&edn_path).unwrap(), malformed);
        assert!(!dir.join("pages").join(format!("hls__{key}.md")).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_highlights_rejects_valid_map_with_trailing_sync_data() {
        let dir = scratch("highlights-trailing-edn");
        let g = Graph::open(&dir);
        let key = crate::pdf::asset_key("paper.pdf");
        let edn_path = dir.join("assets").join(format!("{key}.edn"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        let malformed = "{:highlights [] :extra {}} TRAILING-SYNC-DATA";
        fs::write(&edn_path, malformed).unwrap();
        let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

        let err = g
            .write_highlights("paper.pdf", "Paper", &[h], &[])
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&edn_path).unwrap(), malformed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_highlights_migrates_legacy_key_forward() {
        // Old Tine wrote highlight files under a lowercase+underscore key
        // (`my_paper`); the OG-compatible key for "My Paper.pdf" is "My Paper". A
        // read must find the legacy file, and the next write must migrate the
        // artifacts to the new key (removing the stale legacy ones).
        let dir = scratch("hlmig");
        let pdf = "My Paper.pdf";
        let legacy_key = crate::pdf::legacy_asset_key(pdf); // "my_paper"
        let new_key = crate::pdf::asset_key(pdf); // "My Paper"
        assert_ne!(legacy_key, new_key);
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        let h1 = mkhl(
            "11111111-1111-1111-1111-111111111111",
            3,
            Some("legacy text"),
        );
        fs::write(
            assets.join(format!("{legacy_key}.edn")),
            crate::pdf::write_highlights(&[h1.clone()], ""),
        )
        .unwrap();
        let legacy_page = crate::pdf::hls_page_document(pdf, "My Paper", &[h1.clone()]);
        fs::write(
            dir.join("pages").join(format!("hls__{legacy_key}.md")),
            doc::serialize(&legacy_page),
        )
        .unwrap();

        let g = Graph::open(&dir);
        g.warm_cache();
        // Read-fallback: the legacy file is found under the new-key lookup.
        let read = g.read_highlights(pdf);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, h1.id);

        // Write H1 + a newly-added H2 (editor baseline = [H1]).
        let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("new text"));
        g.write_highlights(pdf, "My Paper", &[h1.clone(), h2.clone()], &[h1.id.clone()])
            .unwrap();

        // New-key artifacts exist with both highlights; the legacy ones are gone.
        let new_edn = assets.join(format!("{new_key}.edn"));
        assert!(new_edn.exists(), "new-key edn written");
        let migrated = crate::pdf::parse_highlights(&fs::read_to_string(&new_edn).unwrap());
        assert_eq!(migrated.len(), 2, "both highlights carried forward");
        assert!(
            dir.join("pages")
                .join(format!("hls__{new_key}.md"))
                .exists(),
            "new hls page"
        );
        assert!(
            !assets.join(format!("{legacy_key}.edn")).exists(),
            "legacy edn removed"
        );
        assert!(
            !dir.join("pages")
                .join(format!("hls__{legacy_key}.md"))
                .exists(),
            "legacy hls page removed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_hls_migration_preserves_page_format_when_preference_changed() {
        let dir = scratch("hlmig-format");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"}\n",
        )
        .unwrap();
        let pdf = "My Paper.pdf";
        let legacy_key = crate::pdf::legacy_asset_key(pdf);
        let new_key = crate::pdf::asset_key(pdf);
        let h = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("legacy"));
        fs::write(
            dir.join("assets").join(format!("{legacy_key}.edn")),
            crate::pdf::write_highlights(&[h.clone()], ""),
        )
        .unwrap();
        let mut legacy_page = crate::pdf::hls_page_document(pdf, "Paper", &[h.clone()]);
        legacy_page.roots[0]
            .children
            .push(DocBlock::new("private note"));
        fs::write(
            dir.join("pages").join(format!("hls__{legacy_key}.md")),
            doc::serialize(&legacy_page),
        )
        .unwrap();

        let g = Graph::open(&dir);
        g.write_highlights(pdf, "Paper", &[h.clone()], &[h.id.clone()])
            .unwrap();

        let migrated = dir.join("pages").join(format!("hls__{new_key}.md"));
        assert!(migrated.exists(), "legacy .md format should be retained");
        assert!(!dir
            .join("pages")
            .join(format!("hls__{new_key}.org"))
            .exists());
        assert!(fs::read_to_string(migrated)
            .unwrap()
            .contains("private note"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_highlights_does_not_migrate_legacy_key_used_by_another_pdf() {
        let dir = scratch("hl-legacy-collision");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();

        let lower_pdf = "my_paper.pdf";
        let spaced_pdf = "My Paper.pdf";
        fs::write(assets.join(lower_pdf), b"lower pdf").unwrap();
        fs::write(assets.join(spaced_pdf), b"spaced pdf").unwrap();

        let lower_key = crate::pdf::asset_key(lower_pdf);
        let spaced_key = crate::pdf::asset_key(spaced_pdf);
        let spaced_legacy_key = crate::pdf::legacy_asset_key(spaced_pdf);
        assert_eq!(lower_key, spaced_legacy_key);
        assert_ne!(spaced_key, spaced_legacy_key);

        let lower_highlight = mkhl(
            "33333333-3333-3333-3333-333333333333",
            3,
            Some("lower pdf highlight"),
        );
        let lower_edn = crate::pdf::write_highlights(&[lower_highlight.clone()], "");
        let lower_edn_path = assets.join(format!("{lower_key}.edn"));
        fs::write(&lower_edn_path, &lower_edn).unwrap();

        let mut lower_page =
            crate::pdf::hls_page_document(lower_pdf, "Lower Paper", &[lower_highlight.clone()]);
        lower_page.roots[0]
            .children
            .push(DocBlock::new("lower pdf private note"));
        let lower_page_bytes = doc::serialize(&lower_page);
        let lower_page_path = dir
            .join("pages")
            .join(format!("{}.md", crate::pdf::hls_page_name(&lower_key)));
        fs::write(&lower_page_path, &lower_page_bytes).unwrap();

        let g = Graph::open(&dir);
        g.warm_cache();
        let spaced_highlight = mkhl(
            "44444444-4444-4444-4444-444444444444",
            4,
            Some("spaced pdf highlight"),
        );
        g.write_highlights(spaced_pdf, "My Paper", &[spaced_highlight], &[])
            .unwrap();

        assert!(
            lower_edn_path.exists(),
            "live colliding pdf edn must not be deleted"
        );
        assert_eq!(
            fs::read_to_string(&lower_edn_path).unwrap(),
            lower_edn,
            "live colliding pdf edn must remain byte-for-byte intact"
        );
        assert!(
            lower_page_path.exists(),
            "live colliding pdf hls page must not be deleted"
        );
        assert_eq!(
            fs::read_to_string(&lower_page_path).unwrap(),
            lower_page_bytes,
            "live colliding pdf hls page must remain byte-for-byte intact"
        );

        let spaced_edn_path = assets.join(format!("{spaced_key}.edn"));
        let spaced_edn = fs::read_to_string(&spaced_edn_path).unwrap();
        let spaced_highlights = crate::pdf::parse_highlights(&spaced_edn);
        assert_eq!(spaced_highlights.len(), 1);
        assert_eq!(
            spaced_highlights[0].id,
            "44444444-4444-4444-4444-444444444444"
        );

        let spaced_page_path = dir
            .join("pages")
            .join(format!("{}.md", crate::pdf::hls_page_name(&spaced_key)));
        let spaced_page = fs::read_to_string(&spaced_page_path).unwrap();
        assert!(
            !spaced_page.contains("lower pdf private note"),
            "colliding pdf note must not be merged into the spaced pdf hls page"
        );
        assert!(
            !spaced_page.contains(&lower_highlight.id),
            "colliding pdf highlight must not be merged into the spaced pdf hls page"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_highlight_write_is_not_seen_as_external() {
        // Repro for the "someone else edited the note" warning when deleting a
        // highlight while its hls__ page is open: the hls page write (and the
        // delete-rewrite) must be recognized as Tine's OWN write by the watcher,
        // not flagged as an external change.
        let dir = scratch("hldel");
        let g = Graph::open(&dir);
        g.warm_cache();
        let h1 = mkhl("aaaaaaaa-0000-0000-0000-000000000001", 1, Some("one"));
        let h2 = mkhl("bbbbbbbb-0000-0000-0000-000000000002", 2, Some("two"));
        let page_path = dir.join("pages").join("hls__paper.md");
        g.write_highlights("paper.pdf", "Paper", &[h1.clone(), h2.clone()], &[])
            .unwrap();
        assert!(
            g.sync_file(&page_path).is_none(),
            "initial highlight write looked external"
        );
        // Delete h2 (write just h1; baseline = both) — the rewrite must also be ours.
        g.write_highlights(
            "paper.pdf",
            "Paper",
            &[h1.clone()],
            &[h1.id.clone(), h2.id.clone()],
        )
        .unwrap();
        assert!(
            g.sync_file(&page_path).is_none(),
            "delete-rewrite looked external (false conflict)"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_pdf_area_image_uses_og_layout() {
        let dir = scratch("areaimg");
        let g = Graph::open(&dir);
        let rel = g
            .write_pdf_area_image("My Paper.pdf", 7, "abc-id", 1659920114630, &[1, 2, 3, 4])
            .unwrap();
        // OG layout: assets/<key>/<page>_<id>_<stamp>.png with the OG-compatible key.
        assert_eq!(rel, "My Paper/7_abc-id_1659920114630.png");
        let p = dir
            .join("assets")
            .join("My Paper")
            .join("7_abc-id_1659920114630.png");
        assert_eq!(fs::read(&p).unwrap(), vec![1, 2, 3, 4]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_pdf_area_image_rejects_nested_asset_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = scratch("areaimg-nested-symlink");
        let outside =
            std::env::temp_dir().join(format!("tine-areaimg-outside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(dir.join("assets")).unwrap();
        symlink(&outside, dir.join("assets").join("My Paper")).unwrap();
        let g = Graph::open(&dir);

        assert!(g
            .write_pdf_area_image("My Paper.pdf", 7, "abc-id", 1659920114630, &[1, 2, 3])
            .is_err());
        assert!(!outside.join("7_abc-id_1659920114630.png").exists());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    fn jdto(name: &str) -> PageDto {
        PageDto {
            name: name.into(),
            kind: PageKind::Journal,
            title: name.into(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: String::new(),
                raw: "hi".into(),
                collapsed: false,
                children: vec![],
                breadcrumb: vec![],
                ..Default::default()
            }],
            rev: None,
            format: Format::Md,
            read_only: false,
            path: String::new(),
            guide: false,
        }
    }

    #[test]
    fn custom_journal_format_creates_in_user_format() {
        let dir = scratch("jfmt");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:journal/file-name-format \"yyyy-MM-dd\"}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        // A custom filename format now creates today's journal at the CORRECT path
        // (the user's format) — not a misplaced default `yyyy_MM_dd` duplicate.
        g.save_page(&jdto("Jun 24th, 2026"), None).unwrap();
        assert!(dir.join("journals").join("2026-06-24.md").exists());
        assert!(!dir.join("journals").join("2026_06_24.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_format_journal_files_load_and_display() {
        // THE reported bug: a graph whose journal files use a non-default format
        // must still load — the files are recognized and titled in the user's
        // page-title-format.
        let dir = scratch("jfmt-load");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:journal/file-name-format \"dd-MM-yyyy\" :journal/page-title-format \"yyyy-MM-dd\"}\n",
        )
        .unwrap();
        // A real journal file in the user's dd-MM-yyyy filename format.
        fs::write(dir.join("journals").join("24-06-2026.md"), "- hi\n").unwrap();
        let g = Graph::open(&dir);
        let js = g.journals_desc();
        assert_eq!(
            js.len(),
            1,
            "custom-format journal must be recognized (was dropped before)"
        );
        assert_eq!(js[0].date_key, Some(20260624));
        assert_eq!(
            js[0].name, "2026-06-24",
            "title rendered in :journal/page-title-format"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_journal_format_creates_journal() {
        let dir = scratch("jfmt-default");
        // No config.edn → defaults → creation proceeds as before.
        let g = Graph::open(&dir);
        g.save_page(&jdto("Jun 24th, 2026"), None).unwrap();
        assert!(dir.join("journals").join("2026_06_24.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advanced_query_runs_supported_subset_flags_rest() {
        let dir = scratch("adv");
        fs::write(
            dir.join("journals").join("2026_06_20.md"),
            "- TODO ship it\n- DONE done\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Note.md"), "- TODO not a journal\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        // (task ?b #{"TODO"}) maps to the existing Task predicate.
        let r = g.run_advanced_query(r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#, None);
        assert!(r.supported);
        assert!(r.ran.contains(&"task".to_string()));
        let total: usize = r.groups.iter().map(|grp| grp.blocks.len()).sum();
        assert_eq!(total, 2, "both TODO blocks match");
        // A clause outside the subset (a raw [?e :a ?v] join) → nothing supported.
        let u = g.run_advanced_query("[:find ?b :where [?b :block/foo ?v]]", None);
        assert!(!u.supported);
        assert!(u.groups.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advanced_query_covers_widened_clause_subset() {
        // 1c: the advanced (datalog) parser maps the same heads the simple DSL
        // supports — page / namespace / page-tags / scheduled / deadline / journal
        // — not just the original task/priority/page-ref/property/between set.
        let dir = scratch("adv-wide");
        fs::write(
            dir.join("journals").join("2026_06_20.md"),
            "- TODO ship it\n  SCHEDULED: <2026-06-25 Thu>\n- pay rent\n  DEADLINE: <2026-06-30 Tue>\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Proj.md"),
            "tags:: work, urgent\n\n- a task on a named page\n",
        )
        .unwrap();
        // Default file-name format is Legacy (`%2F`), so encode the namespace slash.
        fs::write(dir.join("pages").join("Proj%2FSub.md"), "- nested note\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let count = |src: &str| -> usize {
            let r = g.run_advanced_query(src, None);
            assert!(r.supported, "expected supported: {src} (ran={:?})", r.ran);
            r.groups.iter().map(|grp| grp.blocks.len()).sum()
        };

        // (scheduled) / (deadline) map to the planning predicates.
        assert_eq!(count("[:find (pull ?b [*]) :where (scheduled ?b)]"), 1);
        assert_eq!(count("[:find (pull ?b [*]) :where (deadline ?b)]"), 1);
        // (journal) restricts to blocks on journal pages.
        assert_eq!(count("[:find (pull ?b [*]) :where (journal ?b)]"), 2);
        // (page "Name") pins to one page.
        assert_eq!(count(r#"[:find (pull ?b [*]) :where (page ?b "Proj")]"#), 1);
        // (namespace "Proj") matches pages under the namespace.
        assert_eq!(
            count(r#"[:find (pull ?b [*]) :where (namespace ?b "Proj")]"#),
            1
        );
        // (page-tags "work") matches the tags:: page-property.
        assert_eq!(
            count(r#"[:find (pull ?b [*]) :where (page-tags ?b "work")]"#),
            1
        );
        // (between scheduled …) is now field-aware, not hardwired to journal-day.
        assert_eq!(
            count(
                r#"[:find (pull ?b [*]) :where (between scheduled ?b "2026-06-24" "2026-06-26")]"#
            ),
            1
        );

        // Unknown heads still land in `ignored`, never guessed.
        let r = g.run_advanced_query("[:find ?b :where (bogus ?b)]", None);
        assert!(r.ignored.contains(&"bogus".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advanced_query_skeleton_ignores_comment_hints() {
        // 1b: the "switch to advanced" skeleton lists supported heads as `;;` EDN
        // comments. Those example clauses must NOT be parsed as real filters — only
        // the single active clause runs. (Regression: scan_groups now skips `; …`.)
        let dir = scratch("adv-skel");
        fs::write(
            dir.join("journals").join("2026_06_20.md"),
            "- TODO ship it\n- DOING wire it\n- DONE done\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let skeleton = "[:find (pull ?b [*])\n \
             :where\n \
             ;; supported: (priority ?b \"A\") (page-ref ?b \"Nope\") (property ?b :k \"v\")\n \
             ;; (scheduled ?b) (deadline ?b) (page ?b \"Nowhere\")\n \
             (task ?b #{\"TODO\" \"DOING\"})]";
        let r = g.run_advanced_query(skeleton, None);
        assert!(r.supported, "ran: {:?} ignored: {:?}", r.ran, r.ignored);
        // Only the task clause ran — the commented priority/page-ref/etc. did not.
        assert_eq!(r.ran, vec!["task".to_string()]);
        assert!(
            r.ignored.is_empty(),
            "no clause should be ignored: {:?}",
            r.ignored
        );
        let total: usize = r.groups.iter().map(|grp| grp.blocks.len()).sum();
        assert_eq!(
            total, 2,
            "TODO + DOING match; the commented (page-ref \"Nope\") is inert"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_query_sources_cannot_reach_unbounded_cache_keys_or_parser_recursion() {
        let dir = scratch("query-source-recursion-bound");
        fs::write(dir.join("pages").join("P.md"), "- TODO ship\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        // This is the graph-authored shape that previously overflowed the Rust
        // stack when a persisted query macro rendered. Keep it below the byte
        // ceiling so the independent nesting guard is the reason it fails shut.
        let nested = format!("{}(task TODO){}", "(and ".repeat(1_000), ")".repeat(1_000));
        assert!(crate::query::query_source_within_limit(&nested));
        assert!(!crate::query::query_nesting_within_limit(&nested));
        let simple = g.run_query_bounded(&nested, 20_000, 32 * 1024 * 1024);
        assert!(simple.groups.is_empty());
        assert!(g.derived_cache.read().unwrap().is_none());

        let advanced = format!("[:find (pull ?b [*]) :where {nested}]");
        let result = g.run_advanced_query(&advanced, None);
        assert!(!result.supported);
        assert_eq!(result.ignored, vec!["query-nesting-too-deep"]);
        assert!(g.advanced_cache.read().unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advanced_query_reuses_cached_result_until_graph_changes() {
        let dir = scratch("adv-memo");
        fs::write(dir.join("pages").join("P.md"), "- TODO ship\n").unwrap();
        fs::write(
            dir.join("pages").join("Notes.md"),
            "alias:: Scratch\n- ordinary note\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let q = r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#;

        let first = g.run_advanced_query_cached(q, None);
        let second = g.run_advanced_query_cached(q, None);
        assert!(
            Arc::ptr_eq(&first, &second),
            "identical advanced query should be served from the memo cache"
        );
        assert_eq!(first.groups.len(), 1);
        let bounded_key = format!("AQ\0{}\0{}\0n:\0{q}", 20_000, 32 * 1024 * 1024);
        let _ = g.run_advanced_query_bounded_cached(q, None, 20_000, 32 * 1024 * 1024);
        let bounded_first = g
            .advanced_cache
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .get(&bounded_key)
            .unwrap()
            .0
            .result
            .clone();

        let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
        notes.blocks[0].raw = "still unrelated".into();
        g.save_page(&notes, notes.rev.as_deref()).unwrap();
        let after_unrelated = g.run_advanced_query_cached(q, None);
        let _ = g.run_advanced_query_bounded_cached(q, None, 20_000, 32 * 1024 * 1024);
        let bounded_after_unrelated = g
            .advanced_cache
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .get(&bounded_key)
            .unwrap()
            .0
            .result
            .clone();
        assert!(
            Arc::ptr_eq(&first, &after_unrelated),
            "an unrelated edit must retain the advanced-query memo"
        );
        assert!(Arc::ptr_eq(&bounded_first, &bounded_after_unrelated));

        notes.pre_block = Some("alias:: Renamed Scratch\n".into());
        let rev = g.load_named("Notes", PageKind::Page).unwrap().unwrap().rev;
        g.save_page(&notes, rev.as_deref()).unwrap();
        let after_alias_change = g.run_advanced_query_cached(q, None);
        assert!(
            !Arc::ptr_eq(&first, &after_alias_change),
            "a semantic alias change must invalidate graph-wide derived results"
        );

        let mut dto = g.load_named("P", PageKind::Page).unwrap().unwrap();
        dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DONE");
        g.save_page(&dto, dto.rev.as_deref()).unwrap();

        let third = g.run_advanced_query_cached(q, None);
        let _ = g.run_advanced_query_bounded_cached(q, None, 20_000, 32 * 1024 * 1024);
        let bounded_after_affected = g
            .advanced_cache
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .results
            .get(&bounded_key)
            .unwrap()
            .0
            .result
            .clone();
        assert!(
            !Arc::ptr_eq(&first, &third),
            "graph mutation must invalidate the advanced-query memo"
        );
        assert!(!Arc::ptr_eq(&bounded_first, &bounded_after_affected));
        assert!(third.groups.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bounded_query_memo_survives_unrelated_edits_and_recomputes_affected_pages() {
        let dir = scratch("bounded-query-scoped-memo");
        fs::write(dir.join("pages").join("Tasks.md"), "- TODO ship\n").unwrap();
        fs::write(
            dir.join("pages").join("Notes.md"),
            "alias:: Scratch\n- ordinary note\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let first = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
        let second = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
        assert!(Arc::ptr_eq(&first.groups, &second.groups));

        let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
        notes.blocks[0].raw = "still an ordinary note".into();
        g.save_page(&notes, notes.rev.as_deref()).unwrap();
        let after_unrelated = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
        assert!(
            Arc::ptr_eq(&first.groups, &after_unrelated.groups),
            "an unrelated edit must retain the scoped bounded-query memo"
        );

        let mut tasks = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
        tasks.blocks[0].raw = "DONE ship".into();
        g.save_page(&tasks, tasks.rev.as_deref()).unwrap();
        let after_affected = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
        assert!(!Arc::ptr_eq(&first.groups, &after_affected.groups));
        assert!(after_affected.groups.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bounded_reference_memos_survive_unrelated_edits_and_recompute_all_families() {
        const TARGET: &str = "12345678-1234-1234-1234-123456789abc";
        let dir = scratch("bounded-reference-scoped-memos");
        fs::write(
            dir.join("pages").join("Referrer.md"),
            format!("- See [[Target]], plain Target, and (({TARGET}))\n"),
        )
        .unwrap();
        fs::write(dir.join("pages").join("Target.md"), "- target page\n").unwrap();
        fs::write(
            dir.join("pages").join("Notes.md"),
            "alias:: Scratch\n- ordinary note\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let first_block = g.block_referrers_bounded(TARGET, 20_000, 32 * 1024 * 1024);
        let first_backlink = g.backlinks_bounded("Target", 20_000, 32 * 1024 * 1024);
        let first_unlinked = g.unlinked_refs_bounded("Target", 20_000, 32 * 1024 * 1024);
        assert_eq!(first_block.total, 1);
        assert_eq!(first_backlink.total, 1);
        assert_eq!(first_unlinked.total, 1);

        let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
        notes.blocks[0].raw = "still unrelated".into();
        g.save_page(&notes, notes.rev.as_deref()).unwrap();
        let after_block = g.block_referrers_bounded(TARGET, 20_000, 32 * 1024 * 1024);
        let after_backlink = g.backlinks_bounded("Target", 20_000, 32 * 1024 * 1024);
        let after_unlinked = g.unlinked_refs_bounded("Target", 20_000, 32 * 1024 * 1024);
        assert!(Arc::ptr_eq(&first_block.groups, &after_block.groups));
        assert!(Arc::ptr_eq(&first_backlink.groups, &after_backlink.groups));
        assert!(Arc::ptr_eq(&first_unlinked.groups, &after_unlinked.groups));

        let mut referrer = g.load_named("Referrer", PageKind::Page).unwrap().unwrap();
        referrer.blocks[0].raw = "No longer a referrer".into();
        g.save_page(&referrer, referrer.rev.as_deref()).unwrap();
        let affected_block = g.block_referrers_bounded(TARGET, 20_000, 32 * 1024 * 1024);
        let affected_backlink = g.backlinks_bounded("Target", 20_000, 32 * 1024 * 1024);
        let affected_unlinked = g.unlinked_refs_bounded("Target", 20_000, 32 * 1024 * 1024);
        assert!(!Arc::ptr_eq(&first_block.groups, &affected_block.groups));
        assert!(!Arc::ptr_eq(
            &first_backlink.groups,
            &affected_backlink.groups
        ));
        assert!(!Arc::ptr_eq(
            &first_unlinked.groups,
            &affected_unlinked.groups
        ));
        assert_eq!(affected_block.total, 0);
        assert_eq!(affected_backlink.total, 0);
        assert_eq!(affected_unlinked.total, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scoped_reference_invalidation_uses_real_page_before_colliding_alias() {
        let dir = scratch("reference-invalidation-real-page-first");
        fs::write(dir.join("pages").join("X.md"), "alias:: Q\n\n- real X\n").unwrap();
        fs::write(
            dir.join("pages").join("Y.md"),
            "alias:: X\n\n- alias owner\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Source.md"), "- unrelated\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let first_linked = g.backlinks("X");
        let first_unlinked = g.unlinked_refs("X");
        assert!(!first_linked.iter().any(|group| group.page == "Source"));
        assert!(!first_unlinked.iter().any(|group| group.page == "Source"));

        let mut source = g.load_named("Source", PageKind::Page).unwrap().unwrap();
        source.blocks[0].raw = "Q and [[Q]]".into();
        g.save_page(&source, source.rev.as_deref()).unwrap();

        let linked = g.backlinks("X");
        let unlinked = g.unlinked_refs("X");
        assert!(!Arc::ptr_eq(&first_linked, &linked));
        assert!(!Arc::ptr_eq(&first_unlinked, &unlinked));
        assert!(linked.iter().any(|group| group.page == "Source"));
        assert!(unlinked.iter().any(|group| group.page == "Source"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nfd_alias_resolves_and_canonical_equivalent_alias_cannot_shadow_real_page() {
        let dir = scratch("nfd-alias-resolution");
        fs::write(
            dir.join("pages").join("Owner.md"),
            "alias:: Re\u{301}sume\u{301}\n\n- owner\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Shadow.md"),
            "alias:: Cafe\u{301}\n\n- shadow\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Café.md"), "- real page\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        assert_eq!(
            g.load_named("Re\u{301}sume\u{301}", PageKind::Page)
                .unwrap()
                .unwrap()
                .name,
            "Owner"
        );
        assert_eq!(
            g.load_named("Cafe\u{301}", PageKind::Page)
                .unwrap()
                .unwrap()
                .name,
            "Café",
            "the canonically equivalent real title must win before alias fallback"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overflowed_bounded_memo_recomputes_when_an_omitted_match_stops_matching() {
        let dir = scratch("bounded-overflow-negative-transition");
        fs::write(dir.join("pages").join("A.md"), "- TODO first\n").unwrap();
        fs::write(dir.join("pages").join("B.md"), "- TODO second\n").unwrap();
        fs::write(dir.join("pages").join("Notes.md"), "- unrelated\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let first = g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024);
        assert!(first.exceeded);
        assert_eq!(first.total, 2);
        let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
        notes.blocks[0].raw = "still unrelated".into();
        g.save_page(&notes, notes.rev.as_deref()).unwrap();
        let after_unrelated = g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024);
        assert!(Arc::ptr_eq(&first.groups, &after_unrelated.groups));
        assert!(after_unrelated.exceeded);
        assert_eq!(after_unrelated.total, 2);

        let admitted = first.groups[0].page.clone();
        let omitted = if admitted == "A" { "B" } else { "A" };
        let mut page = g.load_named(omitted, PageKind::Page).unwrap().unwrap();
        page.blocks[0].raw = "DONE no longer matches".into();
        g.save_page(&page, page.rev.as_deref()).unwrap();

        let after = g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024);
        assert!(!Arc::ptr_eq(&first.groups, &after.groups));
        assert!(!after.exceeded);
        assert_eq!(after.total, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advanced_cache_invalidation_preserves_nul_inside_opaque_query_source() {
        let dir = scratch("advanced-cache-nul-query");
        fs::write(dir.join("pages").join("P.md"), "- DONE ship\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let query = "[:find (pull ?b [*]) :where \0 (task ?b #{\"TODO\"})]";
        let first = g.run_advanced_query_cached(query, None);
        assert!(first.groups.is_empty());

        let mut page = g.load_named("P", PageKind::Page).unwrap().unwrap();
        page.blocks[0].raw = "TODO ship".into();
        g.save_page(&page, page.rev.as_deref()).unwrap();
        let warm = g.run_advanced_query_cached(query, None);
        let fresh = Graph::open(&dir).run_advanced_query(query, None);
        assert_eq!(warm.groups.len(), 1);
        assert_eq!(warm.groups.len(), fresh.groups.len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn derived_and_advanced_memos_are_lru_bounded() {
        let dir = scratch("memo-lru-bound");
        let g = Graph::open(&dir);
        for i in 0..(DERIVED_CACHE_MAX_ENTRIES + 20) {
            let _ = g.derived_memo(format!("test\0{i}"), Vec::new);
            let _ = g.advanced_memo(format!("test\0{i}"), || crate::query::AdvancedResult {
                groups: Vec::new(),
                ran: Vec::new(),
                ignored: Vec::new(),
                supported: true,
            });
        }
        let oversized_key = "x".repeat(DERIVED_CACHE_MAX_ENTRY_BYTES / 2 + 1);
        let _ = g.derived_memo(oversized_key.clone(), Vec::new);
        let _ = g.advanced_memo(oversized_key.clone(), || crate::query::AdvancedResult {
            groups: Vec::new(),
            ran: Vec::new(),
            ignored: Vec::new(),
            supported: true,
        });
        let derived = g.derived_cache.read().unwrap();
        let advanced = g.advanced_cache.read().unwrap();
        assert_eq!(
            derived.as_ref().unwrap().results.len(),
            DERIVED_CACHE_MAX_ENTRIES
        );
        assert!(!derived
            .as_ref()
            .unwrap()
            .results
            .contains_key(&oversized_key));
        assert!(!advanced
            .as_ref()
            .unwrap()
            .results
            .contains_key(&oversized_key));
        assert_eq!(
            advanced.as_ref().unwrap().results.len(),
            DERIVED_CACHE_MAX_ENTRIES
        );
        let oldest = format!("test\0{}", 0);
        assert!(!derived.as_ref().unwrap().results.contains_key(&oldest));
        assert!(!advanced.as_ref().unwrap().results.contains_key(&oldest));
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- #21: path-pinned pages + duplicate-day reconcile ----

    /// A graph with a canonical day file AND a title-named stray for the same day,
    /// in the user's `EEEE, dd-MM-yyyy` title format. Both resolve to the journal
    /// name "Friday, 26-06-2026" — the collision #21 makes addressable by path.
    fn dup_day_graph(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals").join("2026_06_26.org"),
            "* canonical body\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals").join("Friday, 26-06-2026.org"),
            "* stray body\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn resolve_rel_accepts_graph_files_and_rejects_escapes() {
        let dir = scratch("resolve-rel");
        let g = Graph::open(&dir);
        // Valid: one segment under journals/ or pages/, md/org extension.
        assert_eq!(
            g.resolve_rel("journals/2026_06_26.org"),
            Some(dir.join("journals").join("2026_06_26.org"))
        );
        assert_eq!(
            g.resolve_rel("pages/Note.md"),
            Some(dir.join("pages").join("Note.md"))
        );
        // Valid: nested sub-directories under pages/ (#21) — any depth.
        assert_eq!(
            g.resolve_rel("pages/client-a/foo.md"),
            Some(dir.join("pages").join("client-a").join("foo.md"))
        );
        assert_eq!(
            g.resolve_rel("pages/a/b/c/deep.org"),
            Some(
                dir.join("pages")
                    .join("a")
                    .join("b")
                    .join("c")
                    .join("deep.org")
            )
        );
        // Rejections: traversal (incl. FROM a subdir), absolute, empty/`.` segment,
        // wrong dir, wrong/no extension, a bare dir. Nesting itself is NOT rejected.
        for bad in [
            "../secrets.md",
            "journals/../../etc/passwd.md",
            "pages/../../etc/passwd.md",
            "pages/sub/../../../etc/passwd.md",
            "pages/client-a/../../escape.md",
            "pages/./foo.md",
            "pages/a//b.md",
            "pages/sub/.md",
            "/etc/passwd.md",
            "assets/pic.png",
            "journals/note.txt",
            "journals/",
            "pages/sub/",
            "Note.md",
            "",
        ] {
            assert_eq!(g.resolve_rel(bad), None, "should reject {bad:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_source_file_prefers_the_recorded_nested_identity() {
        let dir = scratch("page-source-file");
        fs::create_dir_all(dir.join("pages/client-a")).unwrap();
        let canonical = dir.join("pages/Note.md");
        let nested = dir.join("pages/client-a/Note.md");
        fs::write(&canonical, "- canonical\n").unwrap();
        fs::write(&nested, "- nested\n").unwrap();
        let g = Graph::open(&dir);

        assert_eq!(
            g.page_source_file("Note", PageKind::Page, Some("pages/client-a/Note.md"))
                .unwrap(),
            nested.canonicalize().unwrap()
        );
        assert_eq!(
            g.page_source_file("Note", PageKind::Page, None).unwrap(),
            canonical.canonicalize().unwrap()
        );
        assert!(g
            .page_source_file("Note", PageKind::Page, Some("assets/Note.md"))
            .is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn checked_open_rejects_configured_directories_outside_graph() {
        let dir = scratch("checked-open-layout");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        for config in [
            "{:pages-directory \"../outside\"}\n",
            "{:journals-directory \"/tmp/tine-outside\"}\n",
            "{:pages-directory \"pages\\\\escape\"}\n",
        ] {
            fs::write(dir.join("logseq/config.edn"), config).unwrap();
            assert!(Graph::open_checked(&dir).is_err(), "accepted {config:?}");
        }
        fs::write(
            dir.join("logseq/config.edn"),
            "{:pages-directory \"archive/pages\" :journals-directory \"diary\"}\n",
        )
        .unwrap();
        assert!(Graph::open_checked(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_update_retries_on_external_change_without_losing_it() {
        let dir = scratch("atomic-update-external");
        let path = dir.join("config.edn");
        fs::write(&path, "{:base 1}\n").unwrap();
        let lock = std::sync::Mutex::new(());
        let injected = std::sync::atomic::AtomicBool::new(false);
        atomic_update_with_hooks(
            &path,
            &lock,
            |content| Ok(content.replace('}', " :mine 3}")),
            |_| {
                if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    fs::write(&path, "{:base 1 :external 2}\n").unwrap();
                }
            },
            |_| {},
        )
        .unwrap();
        let final_content = fs::read_to_string(&path).unwrap();
        assert!(final_content.contains(":external 2"));
        assert!(final_content.contains(":mine 3"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn atomic_update_absent_publish_preserves_a_concurrent_creator() {
        let dir = scratch("atomic-update-absent-race");
        let path = dir.join("config.edn");
        let lock = std::sync::Mutex::new(());
        let injected = std::sync::atomic::AtomicBool::new(false);
        atomic_update_with_hooks(
            &path,
            &lock,
            |content| Ok(content.replace('}', " :mine 3}")),
            |_| {},
            |_| {
                if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    fs::write(&path, "{:external 2}\n").unwrap();
                }
            },
        )
        .unwrap();
        let final_content = fs::read_to_string(&path).unwrap();
        assert!(final_content.contains(":external 2"));
        assert!(final_content.contains(":mine 3"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn guide_twin_withdrawal_preserves_a_concurrent_markdown_replacement() {
        let dir = scratch("guide-twin-withdrawal-race");
        let graph = Graph::open(&dir);
        GUIDE_TWIN_RACE_CONTENT.with(|content| {
            *content.borrow_mut() = Some(b"* external org twin\n".to_vec());
        });
        WITHDRAW_RACE_REPLACEMENT.with(|replacement| {
            *replacement.borrow_mut() = Some(b"- external markdown replacement\n".to_vec());
        });

        assert!(!graph
            .create_markdown_page_if_absent("Guide", "- bundled guide\n")
            .unwrap());
        assert_eq!(
            fs::read_to_string(dir.join("pages/Guide.md")).unwrap(),
            "- external markdown replacement\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages/Guide.org")).unwrap(),
            "* external org twin\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn checked_open_and_resolve_reject_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = scratch("checked-open-symlink");
        let outside =
            std::env::temp_dir().join(format!("tine-checked-open-outside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        symlink(&outside, dir.join("pages-link")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:pages-directory \"pages-link\"}\n",
        )
        .unwrap();
        assert!(Graph::open_checked(&dir).is_err());

        fs::create_dir_all(dir.join("pages")).unwrap();
        symlink(&outside, dir.join("pages/escape")).unwrap();
        let g = Graph::open(&dir);
        assert!(g.resolve_rel("pages/escape/foreign.md").is_none());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn checked_open_rejects_managed_output_symlink_escapes() {
        use std::os::unix::fs::symlink;
        for managed in ["assets", "logseq", "publish"] {
            let dir = scratch(&format!("checked-open-{managed}-symlink"));
            let outside = std::env::temp_dir().join(format!(
                "tine-checked-open-{managed}-outside-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&outside);
            fs::create_dir_all(&outside).unwrap();
            let managed_path = dir.join(managed);
            let _ = fs::remove_dir_all(&managed_path);
            symlink(&outside, &managed_path).unwrap();

            assert!(
                Graph::open_checked(&dir).is_err(),
                "accepted escaped {managed} directory"
            );

            let _ = fs::remove_dir_all(&dir);
            let _ = fs::remove_dir_all(&outside);
        }
    }

    #[cfg(unix)]
    #[test]
    fn checked_open_accepts_only_the_approved_external_assets_target() {
        use std::os::unix::fs::symlink;
        let dir = scratch("checked-open-approved-assets");
        let outside = std::env::temp_dir().join(format!(
            "tine-checked-open-approved-assets-outside-{}",
            std::process::id()
        ));
        let other = std::env::temp_dir().join(format!(
            "tine-checked-open-approved-assets-other-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&outside);
        let _ = fs::remove_dir_all(&other);
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&other).unwrap();
        let _ = fs::remove_dir_all(dir.join("assets"));
        symlink(&outside, dir.join("assets")).unwrap();

        assert!(Graph::open_checked(&dir).is_err());
        assert!(Graph::open_checked_with_assets(&dir, Some(&other)).is_err());
        let graph = Graph::open_checked_with_assets(&dir, Some(&outside)).unwrap();
        assert_eq!(graph.assets_path(), outside.canonicalize().unwrap());
        assert_eq!(
            graph.save_asset("approved.txt", b"safe").unwrap(),
            "approved.txt"
        );
        assert_eq!(fs::read(outside.join("approved.txt")).unwrap(), b"safe");

        // Retargeting the graph link cannot redirect an already-open graph: the
        // Graph holds the originally approved canonical capability. A fresh open
        // also fails because the stored approval no longer matches.
        fs::remove_file(dir.join("assets")).unwrap();
        symlink(&other, dir.join("assets")).unwrap();
        assert_eq!(
            graph
                .save_asset("after-retarget.txt", b"still safe")
                .unwrap(),
            "after-retarget.txt"
        );
        assert!(outside.join("after-retarget.txt").exists());
        assert!(!other.join("after-retarget.txt").exists());
        assert!(Graph::open_checked_with_assets(&dir, Some(&outside)).is_err());

        // A nested link inside the approved root remains confined: neither read
        // nor write may follow it into another directory.
        symlink(other.join("secret.txt"), outside.join("escape.txt")).unwrap();
        fs::write(other.join("secret.txt"), b"private").unwrap();
        assert!(graph.read_asset("escape.txt").is_err());
        let area_key = crate::pdf::asset_key("Escaping area.pdf");
        symlink(&other, outside.join(&area_key)).unwrap();
        assert!(graph
            .write_pdf_area_image("Escaping area.pdf", 1, "id", 1, b"png")
            .is_err());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
        let _ = fs::remove_dir_all(&other);
    }

    #[cfg(windows)]
    #[test]
    fn checked_open_accepts_an_approved_windows_assets_junction() {
        let dir = scratch("checked-open-approved-assets-junction");
        let outside = std::env::temp_dir().join(format!(
            "tine-approved-assets-junction-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        let _ = fs::remove_dir_all(dir.join("assets"));
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &dir.join("assets").display().to_string(),
                &outside.display().to_string(),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J must create the test junction");

        assert!(Graph::open_checked(&dir).is_err());
        let graph = Graph::open_checked_with_assets(&dir, Some(&outside)).unwrap();
        assert_eq!(graph.assets_path(), outside.canonicalize().unwrap());

        let _ = fs::remove_dir(dir.join("assets"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn checked_open_rejects_managed_directories_aliased_inside_graph() {
        use std::os::unix::fs::symlink;
        let dir = scratch("checked-open-managed-alias");
        symlink(dir.join("assets"), dir.join("publish")).unwrap();
        assert!(Graph::open_checked(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_filename_format_cannot_escape_graph_on_save() {
        let dir = scratch("journal-format-escape");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:journal/file-name-format \"../../yyyy_MM_dd\"}\n",
        )
        .unwrap();
        let g = Graph::open_checked(&dir).unwrap();
        let page = PageDto {
            name: "Jul 10th, 2026".into(),
            kind: PageKind::Journal,
            title: "Jul 10th, 2026".into(),
            pre_block: None,
            blocks: vec![],
            format: Format::Md,
            rev: None,
            read_only: false,
            path: String::new(),
            guide: false,
        };
        assert!(g.save_page(&page, None).is_err());
        assert!(!dir.parent().unwrap().join("2026_07_10.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_by_path_serves_the_stray_not_the_canonical() {
        let dir = dup_day_graph("loadbypath");
        let g = Graph::open(&dir);
        g.warm_cache(); // canonical is what name-resolution caches

        // By name → canonical.
        let by_name = g
            .load_named("Friday, 26-06-2026", PageKind::Journal)
            .unwrap()
            .unwrap();
        assert_eq!(by_name.blocks[0].raw, "canonical body");
        assert_eq!(by_name.path, "journals/2026_06_26.org");

        // By path → the STRAY's own content, even though it shares the (kind,name).
        let stray = g
            .load_by_path("journals/Friday, 26-06-2026.org")
            .unwrap()
            .unwrap();
        assert_eq!(stray.blocks[0].raw, "stray body");
        assert_eq!(stray.path, "journals/Friday, 26-06-2026.org");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_with_path_writes_the_pinned_file_and_leaves_canonical_intact() {
        // The core regression for #21: editing a path-pinned stray must save to the
        // stray file, NOT be re-resolved by name onto the canonical one.
        let dir = dup_day_graph("savepinned");
        let g = Graph::open(&dir);
        g.warm_cache();

        let mut stray = g
            .load_by_path("journals/Friday, 26-06-2026.org")
            .unwrap()
            .unwrap();
        stray.blocks[0].raw = "stray body edited".into();
        let rev = g.save_page(&stray, stray.rev.as_deref()).unwrap();
        assert_eq!(
            rev,
            content_rev(
                &fs::read_to_string(dir.join("journals").join("Friday, 26-06-2026.org")).unwrap()
            )
        );

        // The stray file got the edit; the canonical file is byte-for-byte untouched.
        assert_eq!(
            fs::read_to_string(dir.join("journals").join("Friday, 26-06-2026.org")).unwrap(),
            "* stray body edited\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("journals").join("2026_06_26.org")).unwrap(),
            "* canonical body\n"
        );
        // And name-resolution still serves the canonical (the stray didn't poison
        // the (kind,name) cache slot).
        let by_name = g
            .load_named("Friday, 26-06-2026", PageKind::Journal)
            .unwrap()
            .unwrap();
        assert_eq!(by_name.blocks[0].raw, "canonical body");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_pinned_path_that_escapes_the_graph() {
        let dir = dup_day_graph("savebadpath");
        let g = Graph::open(&dir);
        let mut p = g
            .load_by_path("journals/Friday, 26-06-2026.org")
            .unwrap()
            .unwrap();
        p.path = "../escape.md".into();
        assert!(
            g.save_page(&p, p.rev.as_deref()).is_err(),
            "save must refuse an out-of-graph path"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- #21: recursive sub-directory scanning under pages/ ----

    #[test]
    fn nested_page_is_listed_openable_by_name_and_searchable() {
        // A page archived in a real sub-folder (`pages/client-a/foo.md`) must show
        // up as a page — by its BASENAME `foo` (the directory is discarded, OG
        // parity) — and be openable by name and findable by search.
        let dir = scratch("nested-visible");
        fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
        fs::write(
            dir.join("pages").join("client-a").join("foo.md"),
            "- nestedsentinel body\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        // Listed by basename, carrying its nested path.
        let entry = g
            .list_pages()
            .into_iter()
            .find(|e| e.kind == PageKind::Page && e.name == "foo")
            .expect("nested page listed by basename");
        assert_eq!(g.rel_path(&entry.path), "pages/client-a/foo.md");
        assert_eq!(entry.rel_path, "pages/client-a/foo.md");

        // Openable by name (find_entry resolves via the recursive scan), and the
        // DTO carries the nested path so a later save round-trips in place.
        let dto = g
            .load_named("foo", PageKind::Page)
            .unwrap()
            .expect("open nested page by name");
        assert_eq!(dto.blocks[0].raw, "nestedsentinel body");
        assert_eq!(dto.path, "pages/client-a/foo.md");

        // Indexed for full-text search (the cache folded it in via list_pages).
        assert!(
            !g.search("nestedsentinel", 10).is_empty(),
            "nested page is searchable"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_page_edit_saves_in_place_with_no_flat_twin() {
        // The data-safety invariant: editing a nested page must write back to its
        // own file — never re-resolve by name and create a flat `pages/foo.md` twin.
        let dir = scratch("nested-roundtrip");
        fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
        fs::write(
            dir.join("pages").join("client-a").join("foo.md"),
            "- before\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let mut dto = g.load_named("foo", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.path, "pages/client-a/foo.md");
        dto.blocks[0].raw = "after".into();
        g.save_page(&dto, dto.rev.as_deref()).unwrap();

        // The nested file got the edit…
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("client-a").join("foo.md")).unwrap(),
            "- after\n"
        );
        // …and NO flat twin was created.
        assert!(
            !dir.join("pages").join("foo.md").exists(),
            "save must not create a flat pages/foo.md twin"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn colliding_nested_pages_round_trip_by_path_without_flat_twin() {
        let dir = scratch("nested-collision-roundtrip");
        fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
        fs::create_dir_all(dir.join("pages").join("client-b")).unwrap();
        fs::write(
            dir.join("pages").join("client-a").join("foo.md"),
            "- before a\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("client-b").join("foo.md"),
            "- before b\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        let mut a = g.load_by_path("pages/client-a/foo.md").unwrap().unwrap();
        let mut b = g.load_by_path("pages/client-b/foo.md").unwrap().unwrap();
        assert_eq!(a.name, "foo");
        assert_eq!(b.name, "foo");
        assert_eq!(a.path, "pages/client-a/foo.md");
        assert_eq!(b.path, "pages/client-b/foo.md");

        a.blocks[0].raw = "after a".into();
        b.blocks[0].raw = "after b".into();
        g.save_page(&a, a.rev.as_deref()).unwrap();
        g.save_page(&b, b.rev.as_deref()).unwrap();

        assert_eq!(
            fs::read_to_string(dir.join("pages").join("client-a").join("foo.md")).unwrap(),
            "- after a\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("pages").join("client-b").join("foo.md")).unwrap(),
            "- after b\n"
        );
        assert!(
            !dir.join("pages").join("foo.md").exists(),
            "path-pinned saves must not create a flat pages/foo.md twin"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn warmed_duplicate_name_cache_keeps_physical_owners_distinct() {
        let dir = scratch("warmed-duplicate-name-owners");
        fs::create_dir_all(dir.join("pages/duplicates")).unwrap();
        let flat = dir.join("pages/Exact Storage Twin.md");
        let nested = dir.join("pages/duplicates/Exact Storage Twin.md");
        fs::write(&flat, "- flat original sentinel\n").unwrap();
        fs::write(&nested, "- nested original sentinel\n").unwrap();

        let g = Graph::open(&dir);
        g.warm_cache();
        let logical_winner = g
            .find_entry("Exact Storage Twin", PageKind::Page)
            .expect("one duplicate is the stable name winner");
        let non_winner_path = if logical_winner.path == flat {
            &nested
        } else {
            &flat
        };
        let non_winner_entry = g
            .entry_for_path(non_winner_path)
            .expect("non-winning duplicate is addressable by path");
        let winner_original = fs::read_to_string(&logical_winner.path).unwrap();

        // Save through the duplicate's captured physical path after both entries
        // have been warmed. The name winner must remain stable while the other
        // physical owner receives its own cached document and revision.
        let mut non_winner = g
            .load_by_path(&non_winner_entry.rel_path)
            .unwrap()
            .expect("non-winning duplicate loads by path");
        non_winner.blocks[0].raw = "nested saved sentinel".into();
        g.save_page(&non_winner, non_winner.rev.as_deref()).unwrap();

        assert_eq!(
            g.find_entry("Exact Storage Twin", PageKind::Page)
                .expect("name winner remains present")
                .path,
            logical_winner.path,
            "path-addressed save must not repoint the logical first winner"
        );

        let winner_loaded = g.load_page(&logical_winner).unwrap();
        assert_eq!(
            winner_loaded.blocks[0].raw,
            winner_original.trim_start_matches("- ").trim_end(),
            "the name winner retains its own warmed bytes"
        );
        let non_winner_loaded = g.load_page(&non_winner_entry).unwrap();
        assert_eq!(non_winner_loaded.blocks[0].raw, "nested saved sentinel");
        assert_eq!(non_winner_loaded.path, non_winner_entry.rel_path);

        let cached = g.with_pages(|pages| {
            pages
                .iter()
                .filter(|(entry, _)| entry.name == "Exact Storage Twin")
                .map(|(entry, doc)| (entry.path.clone(), doc.roots[0].raw.clone()))
                .collect::<Vec<_>>()
        });
        assert!(cached.iter().any(|(path, raw)| {
            *path == logical_winner.path
                && raw == winner_original.trim_start_matches("- ").trim_end()
        }));
        assert!(cached
            .iter()
            .any(|(path, raw)| *path == *non_winner_path && raw == "nested saved sentinel"));

        for (needle, path) in [
            (
                winner_original.trim_start_matches("- ").trim_end(),
                logical_winner.rel_path.as_str(),
            ),
            ("nested saved sentinel", non_winner_entry.rel_path.as_str()),
        ] {
            assert!(
                g.run_graph_search(needle, 0, 8, false).hits.iter().any(|hit| matches!(
                    hit,
                    crate::query_plan::QueryHit::Block { path: hit_path, .. } if hit_path == path
                )),
                "search hit for {needle:?} must retain its physical owner {path:?}"
            );
        }

        // Give the winner the non-winner's current bytes. A name-keyed revision
        // map incorrectly treats that as already fresh and suppresses its reload.
        fs::write(&logical_winner.path, "- nested saved sentinel\n").unwrap();
        assert!(
            g.sync_file(&logical_winner.path)
                .is_some_and(|entry| entry.path == logical_winner.path),
            "one duplicate's revision must not mark the other duplicate fresh"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn forget_file_evicts_only_the_deleted_duplicate_path() {
        let dir = scratch("forget-duplicate-path");
        fs::create_dir_all(dir.join("pages/duplicates")).unwrap();
        let flat = dir.join("pages/Exact Storage Twin.md");
        let nested = dir.join("pages/duplicates/Exact Storage Twin.md");
        fs::write(&flat, "- flat survives if not removed\n").unwrap();
        fs::write(&nested, "- nested survives if not removed\n").unwrap();

        let g = Graph::open(&dir);
        g.warm_cache();
        let removed = g
            .find_entry("Exact Storage Twin", PageKind::Page)
            .expect("one duplicate is the initial logical winner");
        let survivor_path = if removed.path == flat { &nested } else { &flat };
        let survivor = g
            .entry_for_path(survivor_path)
            .expect("the other duplicate is a physical cache owner");

        fs::remove_file(&removed.path).unwrap();
        assert_eq!(
            g.forget_file(&removed.path)
                .expect("the deleted path had a cache entry")
                .path,
            removed.path
        );
        assert_eq!(
            g.find_entry("Exact Storage Twin", PageKind::Page)
                .expect("surviving duplicate is the new name winner")
                .path,
            survivor.path
        );
        assert_eq!(
            g.with_pages(|pages| {
                pages
                    .iter()
                    .filter(|(entry, _)| entry.name == "Exact Storage Twin")
                    .map(|(entry, _)| entry.path.clone())
                    .collect::<Vec<_>>()
            }),
            vec![survivor.path.clone()],
            "forgetting one physical duplicate leaves the other cached"
        );
        assert_eq!(
            g.load_page(&survivor).unwrap().blocks[0].raw,
            fs::read_to_string(&survivor.path)
                .unwrap()
                .trim_start_matches("- ")
                .trim_end()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_refuses_colliding_nested_page_identities_without_losing_either() {
        let dir = scratch("nested-collision-rename");
        fs::create_dir_all(dir.join("pages/client-a")).unwrap();
        fs::create_dir_all(dir.join("pages/client-b")).unwrap();
        let a = dir.join("pages/client-a/foo.md");
        let b = dir.join("pages/client-b/foo.md");
        fs::write(&a, "- body a\n").unwrap();
        fs::write(&b, "- body b\n").unwrap();
        let g = Graph::open(&dir);

        let err = g.rename_page("foo", "bar").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&a).unwrap(), "- body a\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "- body b\n");
        assert!(!dir.join("pages/bar.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_refuses_ambiguous_nested_page_identity() {
        let dir = scratch("nested-collision-delete");
        fs::create_dir_all(dir.join("pages/client-a")).unwrap();
        fs::create_dir_all(dir.join("pages/client-b")).unwrap();
        let a = dir.join("pages/client-a/foo.md");
        let b = dir.join("pages/client-b/foo.md");
        fs::write(&a, "- body a\n").unwrap();
        fs::write(&b, "- body b\n").unwrap();
        let g = Graph::open(&dir);

        let err = g.delete_page("foo", PageKind::Page).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&a).unwrap(), "- body a\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "- body b\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_mutations_require_the_captured_exact_owner_and_still_refuse_duplicates() {
        let dir = scratch("expected-page-owner");
        fs::create_dir_all(dir.join("pages/client-a")).unwrap();
        fs::create_dir_all(dir.join("pages/client-b")).unwrap();
        let a = dir.join("pages/client-a/Twin.md");
        let b = dir.join("pages/client-b/Twin.md");
        fs::write(&a, "- client a\n").unwrap();
        let g = Graph::open(&dir);

        let stale = g
            .delete_page_expected("Twin", PageKind::Page, Some("pages/client-b/Twin.md"))
            .unwrap_err();
        assert_eq!(stale.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read_to_string(&a).unwrap(), "- client a\n");

        fs::write(&b, "- client b\n").unwrap();
        let g = Graph::open(&dir);
        let ambiguous = g
            .rename_page_expected("Twin", "Renamed", Some("pages/client-b/Twin.md"))
            .unwrap_err();
        assert_eq!(ambiguous.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&a).unwrap(), "- client a\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "- client b\n");
        assert!(!dir.join("pages/Renamed.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_refuses_target_that_exists_in_other_format() {
        let dir = scratch("rename-cross-format-target");
        let old = dir.join("pages/Old.org");
        let target = dir.join("pages/New.md");
        fs::write(&old, "* old body\n").unwrap();
        fs::write(&target, "- existing target\n").unwrap();
        let g = Graph::open(&dir);

        let err = g.rename_page("Old", "New").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&old).unwrap(), "* old body\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "- existing target\n");
        assert!(!dir.join("pages/New.org").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_refuses_logical_target_in_nested_directory() {
        let dir = scratch("rename-nested-target");
        fs::create_dir_all(dir.join("pages/client")).unwrap();
        let old = dir.join("pages/Old.org");
        let target = dir.join("pages/client/New.md");
        fs::write(&old, "* old body\n").unwrap();
        fs::write(&target, "- nested target\n").unwrap();
        let g = Graph::open(&dir);

        let err = g.rename_page("Old", "New").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&old).unwrap(), "* old body\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "- nested target\n");
        assert!(!dir.join("pages/New.org").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_pages_appends_stray_into_canonical_and_trashes_stray() {
        let dir = dup_day_graph("merge");
        let g = Graph::open(&dir);
        g.warm_cache();
        g.merge_pages("journals/Friday, 26-06-2026.org", "journals/2026_06_26.org")
            .unwrap();

        // Canonical now holds both bodies; the stray is gone (moved to trash).
        let merged = fs::read_to_string(dir.join("journals").join("2026_06_26.org")).unwrap();
        assert!(
            merged.contains("canonical body"),
            "canonical kept: {merged:?}"
        );
        assert!(merged.contains("stray body"), "stray appended: {merged:?}");
        assert!(
            !dir.join("journals").join("Friday, 26-06-2026.org").exists(),
            "stray trashed"
        );
        // Recoverable, not hard-deleted.
        let trash = dir.join("logseq").join(".tine-trash");
        let kept = fs::read_dir(&trash).unwrap().flatten().count();
        assert_eq!(kept, 1, "stray sits in the recoverable trash");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_file_to_page_rescues_stray_and_refuses_collision() {
        let dir = dup_day_graph("renamefile");
        let g = Graph::open(&dir);
        g.warm_cache();
        g.rename_file_to_page("journals/Friday, 26-06-2026.org", "Old Friday")
            .unwrap();

        // The stray became a normal page, reachable by its new unique name.
        assert!(!dir.join("journals").join("Friday, 26-06-2026.org").exists());
        let page = g.load_named("Old Friday", PageKind::Page).unwrap().unwrap();
        assert_eq!(page.blocks[0].raw, "stray body");
        assert_eq!(page.kind, PageKind::Page);

        // A second rescue onto an existing page name is refused (never clobbers).
        fs::write(
            dir.join("journals").join("Saturday, 27-06-2026.org"),
            "* s\n",
        )
        .unwrap();
        assert!(
            g.rename_file_to_page("journals/Saturday, 27-06-2026.org", "Old Friday")
                .is_err(),
            "collision refused"
        );
        assert!(
            dir.join("journals")
                .join("Saturday, 27-06-2026.org")
                .exists(),
            "source left intact on refusal"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_conflicts_expose_a_routable_path_per_file() {
        let dir = dup_day_graph("conflictpath");
        let g = Graph::open(&dir);
        let conflicts = g.journal_conflicts();
        assert_eq!(conflicts.len(), 1, "one duplicated day");
        let files = &conflicts[0].files;
        assert_eq!(files.len(), 2);
        // Canonical first; both carry a graph-root-relative, resolvable path.
        assert!(files[0].canonical);
        assert_eq!(files[0].path, "journals/2026_06_26.org");
        assert_eq!(files[1].path, "journals/Friday, 26-06-2026.org");
        for f in files {
            assert!(
                g.resolve_rel(&f.path).is_some(),
                "conflict path resolves: {}",
                f.path
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
