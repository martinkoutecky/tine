//! Graph model: opening a graph directory, listing/loading/saving pages, and
//! the DTOs that cross the Tauri IPC boundary.
//!
//! For M0/M1 the canonical state is the on-disk files; Rust loads a page into a
//! [`PageDto`] tree and writes it back from one. The frontend owns the live
//! editing tree (see plan). File-backed runtime UUIDs are deterministic structural
//! locators; persisted `id::` values remain a separate external reference identity.

use crate::config::Config;
#[cfg(test)]
use crate::config::FileNameFormat;
use crate::date::{JournalDate, JournalFormat};
use crate::doc::{self, DocBlock, Document, StructuralLayoutIdentity};
use crate::graph_text_path::{
    graph_text_component_is_portable, BlobDescription, CanonicalGraphResourceId, GraphTextKind,
    GraphTextPath, PortablePathKey, UnsafeGraphTextPath,
};
use crate::graph_text_scope::{GraphTextScope, GraphTextScopeBinding};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions as CapOpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::RwLock;
use tine_storage::ContentDigest;
use tine_storage::{DurableDirectoryPublication, FilesystemError};
use uuid::Uuid;

mod asset_files;
mod asset_refs;
mod asset_reserve;
mod assets;
mod atomic_copy;
mod bounded_walks;
mod budgets;
mod config_writes;
mod conflicts;
mod derived_cache;
mod derived_reads;
mod direct_query;
mod dto;
mod editor_activation;
mod editor_types;
mod graph_dir;
mod graph_text_admission;
mod graph_text_capture;
mod graph_text_errors;
mod graph_text_identity;
mod graph_text_inventory;
mod graph_text_scope;
mod graph_text_sources;
mod graph_text_state;
mod graph_text_targets;
mod graph_text_writes;
mod journals;
mod lookup;
mod open_graph;
mod page_cache;
mod page_cache_index;
mod page_header;
mod page_inventory;
mod page_parse;
mod page_rename;
mod pages_merge;
mod paths;
mod pdf;
mod persistent_map;
pub use atomic_copy::*;
mod projection_rename;
use bounded_walks::*;
use budgets::*;
use graph_text_capture::*;
pub use graph_text_errors::*;
pub(crate) use projection_rename::*;
mod projection_fs;
pub(crate) use crate::filesystem_durability::*;
pub use crate::filesystem_durability::{
    atomic_update, atomic_write, dir_fsync_error_is_unsupported, sync_dir_for_rename,
};
use projection_fs::*;
mod trash;
use asset_files::*;
use asset_refs::*;
use asset_reserve::*;
pub use derived_cache::*;
pub use editor_types::*;
use graph_dir::*;
pub use graph_text_state::*;
use page_cache_index::*;
use page_header::*;
pub(crate) use page_parse::*;
use trash::*;
mod write_gate;
pub use dto::*;
use write_gate::*;
mod projection_lifetime;
pub use projection_lifetime::LaunchWarmAnnouncement;
mod retired_files;
use retired_files::*;
mod queries;
mod query_graph;
mod save_path;
mod search;
mod sync_file;
mod write_receipts;
pub use crate::vocab::*;
use persistent_map::{PersistentMap, PersistentMapNode};

const LOGSEQ_TEXT_EXTENSIONS: [&str; 3] = ["md", "markdown", "org"];

#[cfg(test)]
thread_local! {
    /// §5.3's hydration census: the pages a DISPATCHED query loaded a `Document`
    /// for. The claim it makes observable is I-13/I-15's — "pages loaded equals
    /// result pages" — which production also enforces by refusing a mismatched
    /// hydration, but a counter a test can read is what keeps the claim from
    /// quietly becoming "pages loaded is at most the whole graph".
    static DIRECT_HYDRATED_PAGES: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn is_logseq_text_extension(extension: &str) -> bool {
    LOGSEQ_TEXT_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

fn text_extension_from_path(path: &Path) -> Option<&str> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| is_logseq_text_extension(extension))
}

fn split_logseq_text_filename(filename: &str) -> Option<(&str, &str)> {
    filename
        .rsplit_once('.')
        .filter(|(stem, extension)| !stem.is_empty() && is_logseq_text_extension(extension))
}

fn configured_text_variant_paths(dir: &Path, stem: &str) -> [PathBuf; 3] {
    LOGSEQ_TEXT_EXTENSIONS.map(|extension| dir.join(format!("{stem}.{extension}")))
}

/// Whether `path` is a page file Tine reads (markdown or org).
fn is_page_file(path: &Path) -> bool {
    text_extension_from_path(path).is_some()
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Error for an ambiguous page that exists in multiple supported text extensions.
/// Deliberately NOT the `AlreadyExists`/"conflict" signal, so the UI surfaces it
/// as a plain error (a toast) instead of a keep-mine/use-disk conflict prompt.
fn twin_error(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Other,
        format!(
            "\"{name}\" exists in multiple .md/.markdown/.org files — remove all but one (e.g. in Logseq) to edit it in Tine"
        ),
    )
}

/// The error for a path-addressed op (#21) whose graph-root-relative path is
/// invalid — outside `journals/`/`pages/`, a traversal, or the wrong extension.
fn bad_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid file path")
}

/// A confirmed `"merged"` row decision the resolve could not re-derive from the
/// same base (see [`crate::sync_diff::MergeRefused`]). Refusing the whole
/// resolve is the point: no side is silently substituted for the merged body
/// the user approved, and nothing has been written when this is returned.
fn merge_refused(refusal: crate::sync_diff::MergeRefused) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, refusal.to_string())
}

thread_local! {
    /// Physical identities captured after this process displaced a live page.
    /// They are intentionally process-local: after a crash recovery must
    /// quarantine rather than treating a persisted identity as unlink authority.
    static IN_TURN_RECOVERY_IDENTITIES:
        std::cell::RefCell<std::collections::BTreeMap<Uuid, ContentDigest>> = const {
            std::cell::RefCell::new(std::collections::BTreeMap::new())
        };
}

struct ProjectionTarget {
    absolute_path: PathBuf,
    parent_components: Vec<String>,
    filename: String,
}

#[derive(Debug)]
struct ProjectionSemanticRefusal(String);

impl std::fmt::Display for ProjectionSemanticRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectionSemanticRefusal {}

fn projection_semantic_refusal(kind: io::ErrorKind, message: impl Into<String>) -> io::Error {
    io::Error::new(kind, ProjectionSemanticRefusal(message.into()))
}

pub(crate) fn is_projection_semantic_refusal(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|source| source.is::<ProjectionSemanticRefusal>())
}

/// Name the filesystem primitive and the graph location behind a raw platform
/// errno on the projection leg.
///
/// The device is the only oracle for Android's shared-storage semantics and one
/// CI round trip costs ~20 minutes, so a receipt that says only
/// `Invalid argument (os error 22)` cannot be acted on. `ErrorKind` is
/// preserved, because callers above classify on it (`NotFound`/`AlreadyExists`
/// are guarded-conflict signals) and the platform durability policy matches on
/// it too. A semantic refusal is returned untouched so its marker type survives.
fn projection_platform_error(operation: &str, location: &str, error: io::Error) -> io::Error {
    if is_projection_semantic_refusal(&error) {
        return error;
    }
    io::Error::new(
        error.kind(),
        format!("{operation} failed at {location}: {error}"),
    )
}

/// One lexical/scope validation result shared by exact points and feed events.
///
/// This deliberately has no twin path. `.markdown` is one exact physical
/// spelling, not an instruction to synthesize an `.md` or `.org` neighbor.
#[derive(Clone, Debug)]
struct GraphTextExactPath {
    graph_text_path: Option<GraphTextPath>,
    parent_components: Vec<String>,
    filename: String,
}

struct ProjectionParent {
    chain: Vec<Dir>,
}

impl ProjectionParent {
    fn final_dir(&self) -> &Dir {
        self.chain
            .last()
            .expect("projection parent chain always contains the graph root")
    }
}

enum ProjectionParentCapture {
    Missing,
    Present(ProjectionParent),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GraphTextPublicationValidation {
    /// Standalone callers have not established graph-wide collision evidence.
    CompleteIndex,
    /// One exact source/destination transition proves portable aliases,
    /// retained parent ownership, the source's single-link identity, and the
    /// destination's absence directly. No document contents are relevant.
    PathLocal,
    /// A surrounding transaction owns graph-text identity authority and has
    /// already completed a bounded no-follow inventory. Publication still
    /// repeats exact target, single-link, portable-path, and no-clobber checks.
    TransactionInventory,
}

/// Parse a page file's bytes into a [`Document`] using the parser for its
/// format (org headlines vs markdown bullets), chosen by the path's extension.
fn parse_doc(path: &Path, content: &str) -> Document {
    match Format::from_path(path) {
        Format::Md => doc::parse(content),
        Format::Org => crate::org::parse_org(content),
    }
}

pub struct Graph {
    pub root: PathBuf,
    /// Retained no-follow identity of the graph root. Projection writes fail
    /// closed when this capability could not be established at graph open.
    projection_root: Option<Dir>,
    /// Graph-relative live names whose editor-publication claimants could not
    /// be reconciled during the checked-open walk. Journal replay must never
    /// interpret one of these absences as an external deletion (I2c).
    interrupted_publication_claimants: RwLock<std::collections::BTreeSet<GraphTextPath>>,
    /// The canonical filesystem capability used for every asset operation. For
    /// ordinary graphs this is `<root>/assets`; when the runtime has explicitly
    /// approved an external assets symlink/junction it is that exact resolved
    /// directory. No other graph path may use this capability.
    assets_root: PathBuf,
    pub config: Config,
    /// Sole versioned eligibility policy for normal graph text discovery and
    /// exact existing-file access. It grants no creation/projection authority.
    graph_text_scope: GraphTextScope,
    /// Exact bytes from which this scan-capable instance derived its scope and
    /// configured text roots. A scan must require a fresh Graph when the case-insensitive
    /// on-disk config path no longer has this description.
    reconciliation_scan_open_config_description: Option<BlobDescription>,
    /// Digest of the configuration bytes THIS instance last published.
    ///
    /// The watcher cannot otherwise tell Tine's own settings write from an
    /// outside one, and would reopen the whole graph — discarding every cache
    /// it has built — every time the user toggles a star.
    recent_config_write: RwLock<Option<BlobDescription>>,
    /// Unforgeable identity of this exact Graph instance. Reopening the same
    /// resource intentionally produces a different token.
    graph_text_admission_instance: Arc<GraphTextAdmissionInstance>,
    /// Complete graph-text identity evidence retained specifically for ordinary
    /// guarded writes. The legacy watcher records exact paths or uncertainty
    /// here before its deferred cache reconciliation.
    guarded_graph_text_identity: RwLock<GuardedGraphTextIdentityState>,
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
    /// Compact runtime IDs for exact revisions published during this session.
    /// Page loading and disposable projection recovery share this owner; neither
    /// needs to retain a Document or trust a damaged database to recover IDs.
    session_page_ids: RwLock<std::collections::HashMap<PathBuf, SessionPageIds>>,
    /// One source-inventory repair at a time. Joiners return to readiness
    /// admission without retaining a snapshot or waiting under a graph lock.
    projection_recovery: std::sync::Mutex<()>,
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
    /// Generation-bound effective ownership and parse-failure evidence derived
    /// from the warm physical-owner cache. Name-only creation uses this exact
    /// generation plus target-local no-replace validation; raw watcher events
    /// block creation until their debounced reconciliation has advanced it.
    effective_identity_index: RwLock<Option<Arc<EffectiveIdentityIndex>>>,
    /// Bumped on every cache mutation (upsert/remove). The lock-free cache build
    /// captures this before reading disk and rebuilds if a mutation raced it
    /// (which would otherwise install stale content over a concurrent save).
    cache_gen: std::sync::atomic::AtomicU64,
    /// Moved, under the cache write lock and before `cache_gen`, by every
    /// generation move that is not a one-page upsert: a removal or a whole
    /// cache invalidation. A warm validation that sees `cache_gen` move while
    /// this stays put knows each move published one page and its session
    /// revision, so it can check those pages against what it read instead of
    /// rereading the whole graph (GH #543).
    cache_structural_gen: std::sync::atomic::AtomicU64,
    /// Pages counted by the running whole-graph check or read, for the
    /// indexing progress bar only (GH #543).
    indexing_progress: crate::indexing_progress::ProgressCounter,
    /// Raw watcher callbacks publish an O(1) admission barrier before their
    /// debounced reconciliation. The app registry admits only one Graph slot per
    /// canonical root, so this frontier is instance-local and cannot be cleared
    /// by a different cache. Name-only creation refuses while the two epochs
    /// differ; existing exact-owner saves keep their path-local validation.
    external_observation_epoch: std::sync::atomic::AtomicU64,
    external_reconciled_epoch: std::sync::atomic::AtomicU64,
    external_observation_instance: u64,
    /// One explicit whole-graph cache-build flight. Owners parse without holding
    /// this mutex; joiners wait on the flight's own notification and therefore
    /// never wait while holding cache or index locks.
    page_build_flight: std::sync::Mutex<Option<Arc<PageBuildFlight>>>,
    /// The app has replaced this graph (a switch or a refresh): a display read
    /// still running on it must not start graph-sized work (GH #543).
    retired: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    page_build_test: PageBuildTestState,
    /// Memoized reference results (backlinks and unlinked references), keyed by `(cache_gen, today)` so it self-invalidates on ANY
    /// cache mutation and on a date rollover (relative-date queries depend on
    /// today). Lets a re-render, a second component showing the same query, or
    /// navigating back to a page recompute nothing; never serves a stale result.
    derived_cache: RwLock<Option<DerivedCache>>,
    /// Disposable SQLite facts for Direct Files. Markdown/Org and the parsed
    /// page cache remain authoritative; indexed reads are admitted only when
    /// this worker has published the exact current `cache_gen`.
    direct_projection: std::sync::Mutex<Option<Arc<crate::direct_projection::DirectProjection>>>,
    /// Memoized `list_pages()` (the journals//pages/ directory scan), keyed by
    /// cache_gen — which bumps on every page create/delete/rename (Tine or watcher)
    /// — so quick-switch / [[ ]] autocomplete don't re-read both dirs on every
    /// keystroke. An externally-created page not yet seen by the watcher is at most
    /// one watcher tick (≤3s) stale here.
    page_list_cache: RwLock<Option<(u64, Vec<PageEntry>)>>,
    /// Memoized `referenced_page_names()`, keyed by `cache_gen`, with the set's
    /// digest stored beside it so a hit does not re-hash every name.
    ///
    /// The projection answers this question by draining one row per (source
    /// page, referenced name) pair and folding it down to distinct names: on a
    /// 10,000-page graph that is 110,000 rows for 10,010 names, measured at
    /// 1.29 s — essentially the whole 1.41 s a `[[ ]]` autocomplete keystroke
    /// used to cost before page-name autocomplete moved to the dictionary-backed
    /// executor. Within one generation every later
    /// keystroke, and every other caller of this set, then answers from here.
    /// The first lookup after a save still pays the drain, because a save bumps
    /// `cache_gen`; priming the memo at generation publish would only move that
    /// 1.4 s behind every save instead.
    ///
    /// Batching does not help (512 → 16384 rows per statement leaves the cost
    /// unchanged; the work is the scan, not the round trips) and neither does a
    /// distinct-names query (`raw_name` is not indexed, so it is 2–13× SLOWER).
    /// Keyed on `cache_gen`, this is exactly as fresh as the projection read it
    /// replaces, which already refuses to answer at any other generation.
    referenced_names_cache: RwLock<Option<(u64, u64, Vec<String>)>>,
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
    /// Recent exact Direct Files states which the native watcher may still echo.
    /// Unlike `recent_writes`, the first receipt is minted only after Tine's
    /// final no-follow reread proved both the published bytes and physical file
    /// identity. Successful debounced reconciliation replaces it with the exact
    /// accepted final state, so delayed duplicate callbacks remain no-ops while
    /// an old state can never regain authority after a newer state was admitted.
    /// The raw callback reopens only candidate paths under the same page lock and
    /// may omit the external-change frontier only when both identity and revision
    /// still match.
    recent_graph_text_states:
        std::sync::Mutex<std::collections::HashMap<PathBuf, ExactGraphTextStateReceipt>>,
    /// Concord base ledger (ADR 0056): the per-page last text Tine agreed on
    /// with the disk, updated best-effort after successful saves and external-
    /// change admissions. A disposable cache stored OUTSIDE the sync tree;
    /// unset (most tests) makes every hook a no-op. Never
    /// consulted on the save critical path — only by conflict diffs.
    concord_ledger: std::sync::OnceLock<Arc<crate::concord_ledger::ConcordLedger>>,
    /// The exact page files currently being rewritten as the DIRECT result of a
    /// user's VCS-marker resolution (Concord L5, `resolve_vcs_marker_conflict`).
    /// Concord invariant 3 says Tine never rewrites a marker-bearing file — the
    /// one exception is the resolution the user just confirmed, which REMOVES
    /// the markers. Scoping the exception to an exact path (held only across the
    /// one guarded write, under that page's lock) means a concurrent editor save
    /// to any OTHER marker-bearing page is still refused. See
    /// `serialize_page_document`.
    marker_resolutions: std::sync::Mutex<std::collections::HashSet<PathBuf>>,
    /// `path → content_rev` of the on-disk bytes the cached page's
    /// `Document` was parsed from. Invariant: an entry exists IFF the page is in
    /// the cache, and `disk_revs[path] == content_rev(current disk bytes)` ⟹ the
    /// cached doc reflects disk (is fresh). Lets `sync_file_content` skip the
    /// parse→serialize→parse freshness comparison when a file is unchanged — the
    /// common case on every page navigation and most watcher polls. A missing or
    /// mismatched entry always falls through to the correct parse-compare path, so
    /// the worst a desync can cause is redundant work, never a stale serve.
    disk_revs: RwLock<std::collections::HashMap<PathBuf, String>>,
    /// Exact no-follow file identity observed for a successfully parsed load,
    /// bound to its content revision. Existing-file saves require the same
    /// identity and bytes; this is discovery/read evidence, never creation
    /// authority.
    loaded_file_identities: RwLock<std::collections::HashMap<PathBuf, (String, ContentDigest)>>,
    /// One-shot authority minted only by a coherent editor-conflict observation.
    /// This is deliberately separate from `loaded_file_identities`: ordinary
    /// loads are evidence for ordinary saves, never permission to overwrite.
    conflict_authority: std::sync::Mutex<ConflictAuthorityState>,
    /// Live editor activations, keyed by the exact path each is live for.
    ///
    /// Deliberately a registry on the `Graph` rather than a field of any page
    /// value: a token stored inside a page object is copied by every clone,
    /// snapshot and DTO round-trip, and a copy would then claim an identity it
    /// does not have (see the frontend's `clonePages`/history snapshots).
    editor_activations: std::sync::Mutex<EditorActivationState>,
    /// Per-resolved-path write locks. The same page file has TWO in-process
    /// writers — the editor (`save_page`/`write_page`) and the PDF highlight path
    /// (`write_highlights`, for an `hls__` page) — and a rename rewrites many
    /// files at once. Holding the per-path lock across the whole
    /// read→conflict-check→write→`cache_upsert` makes same-page writes serialize,
    /// so they can't clobber each other or leave a stale self-write marker.
    /// Lock order is ALWAYS page_lock → cache → disk_revs; never the reverse.
    page_locks:
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
    /// Resource-scoped shared admission boundary for all page/journal
    /// writers. Identity acquisition failure is retained as an error so an open
    /// can never fall back to an unshared gate.
    graph_text_write_binding: io::Result<GraphTextWriteBinding>,
    /// Per-UI-lane cancellation epochs for whole-graph text searches. Starting a
    /// newer search makes its superseded prefix stop promptly.
    search_lanes: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicU64>>,
    >,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GraphTextAdmissionTestCounters {
    builder_enumerations: usize,
    direct_creation_censuses: usize,
    direct_creation_files_hashed: usize,
    point_query_attempts: usize,
    parser_invocations: usize,
    index_map_insertions: usize,
    event_map_key_reads: usize,
    event_map_key_writes: usize,
    event_reverse_members: usize,
    persistent_node_allocations: usize,
    persistent_rotations: usize,
    persistent_payload_members: usize,
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_RENAME_SOURCE_REMOVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WITHDRAW_RACE_REPLACEMENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static GUIDE_TWIN_RACE_CONTENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static PROJECTION_LAST_MOMENT_REPLACEMENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static PROJECTION_PUBLICATION_RACE_REPLACEMENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static PROJECTION_AFTER_RETIRE_REPLACEMENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static PROJECTION_STALE_RECOVERY_WRITE: std::cell::RefCell<Option<(fs::File, Vec<u8>)>> = const { std::cell::RefCell::new(None) };
    static PROJECTION_POST_PUBLISH_REPLACEMENT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static PROJECTION_LATE_COLLISION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static PROJECTION_AFTER_RETIRE_COLLISION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static PROJECTION_POST_PUBLISH_COLLISION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static PROJECTION_BEFORE_RESTORE: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static FAIL_NEXT_PROJECTION_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PROJECTION_EXACT_OPEN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static GRAPH_TEXT_INVENTORY_READ_RACE: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_CAPTURE_REVALIDATION_RACE: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE: std::cell::RefCell<Option<GraphTextInventoryLimits>> = const { std::cell::RefCell::new(None) };
    static GRAPH_TEXT_BUDGET_LAST_PEAK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static BOUNDED_READ_AFTER_METADATA: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_IDENTITY_ACQUISITION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_AFTER_ADMISSION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_AFTER_IDENTITY_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_BEFORE_MUTATION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_AFTER_RETIRE: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static JOURNAL_PROJECTION_BEFORE_PUBLISH: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static JOURNAL_PROJECTION_AFTER_PUBLISH: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static JOURNAL_PROJECTION_AFTER_TARGET_REREAD: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static JOURNAL_PROJECTION_BEFORE_CACHE_PUBLICATION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_BEFORE_RESTORE: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_WRITE_DURING_ROLLBACK: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static EDITOR_RETIRED_CLEANUP: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static EDITOR_COMMIT_BEFORE_RECHECK: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static EDITOR_COMMIT_BEFORE_FINAL_REREAD: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static EXACT_GRAPH_TEXT_EVENT_AFTER_CANDIDATE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static CONFLICT_OBSERVATION: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static GRAPH_TEXT_ADMISSION_TEST_COUNTERS: std::cell::Cell<GraphTextAdmissionTestCounters> = const { std::cell::Cell::new(GraphTextAdmissionTestCounters { builder_enumerations: 0, direct_creation_censuses: 0, direct_creation_files_hashed: 0, point_query_attempts: 0, parser_invocations: 0, index_map_insertions: 0, event_map_key_reads: 0, event_map_key_writes: 0, event_reverse_members: 0, persistent_node_allocations: 0, persistent_rotations: 0, persistent_payload_members: 0 }) };
    static GRAPH_TEXT_PARSE_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static GRAPH_TEXT_PORTABLE_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static GRAPH_TEXT_PORTABLE_DIRECTORY_LISTINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static GRAPH_TEXT_EVENT_REVALIDATION_RACE: std::cell::RefCell<Option<Box<dyn FnOnce() -> io::Result<()>>>> = std::cell::RefCell::new(None);
    static FAIL_NEXT_GUARDED_GRAPH_TEXT_IDENTITY_UPDATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DIRECT_CREATION_CENSUS_BUMP_CACHE_GEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn reset_graph_text_admission_test_counters() {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS
        .with(|counters| counters.set(GraphTextAdmissionTestCounters::default()));
}

#[cfg(test)]
fn graph_text_admission_test_counters() -> GraphTextAdmissionTestCounters {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(Cell::get)
}

#[cfg(test)]
fn count_graph_text_admission_builder_enumeration() {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.builder_enumerations += 1;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_builder_enumeration() {}

#[cfg(test)]
fn count_graph_text_admission_parser_invocation() {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.parser_invocations += 1;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_parser_invocation() {}

#[cfg(test)]
fn count_graph_text_admission_index_map_insertion() {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.index_map_insertions += 1;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_index_map_insertion() {}

#[cfg(test)]
fn count_graph_text_admission_event_work(reads: usize, writes: usize, members: usize) {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.event_map_key_reads += reads;
        value.event_map_key_writes += writes;
        value.event_reverse_members += members;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_event_work(_reads: usize, _writes: usize, _members: usize) {}

#[cfg(test)]
fn count_graph_text_admission_persistent_node_allocation() {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.persistent_node_allocations += 1;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_persistent_node_allocation() {}

#[cfg(test)]
fn count_graph_text_admission_persistent_rotation() {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.persistent_rotations += 1;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_persistent_rotation() {}

#[cfg(test)]
fn count_graph_text_admission_persistent_payload_members(members: usize) {
    GRAPH_TEXT_ADMISSION_TEST_COUNTERS.with(|counters| {
        let mut value = counters.get();
        value.persistent_payload_members += members;
        counters.set(value);
    });
}

#[cfg(not(test))]
fn count_graph_text_admission_persistent_payload_members(_members: usize) {}

#[cfg(test)]
fn graph_text_event_revalidation_race_hook() -> io::Result<()> {
    GRAPH_TEXT_EVENT_REVALIDATION_RACE.with(|hook| {
        let callback = hook.borrow_mut().take();
        callback.map_or(Ok(()), |callback| callback())
    })
}

#[cfg(not(test))]
fn graph_text_event_revalidation_race_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_parse_failure_hook() -> io::Result<()> {
    GRAPH_TEXT_PARSE_FAILURE.with(|failure| {
        if failure.replace(false) {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "injected graph-text parse failure",
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(test))]
fn graph_text_parse_failure_hook() -> io::Result<()> {
    Ok(())
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

// Keep the test fault at the narrow core Result boundary rather than adding a
// test-only control surface to tine-storage.  Keying it by the deterministic
// ambient return path keeps parallel runtime fixtures independent.

/// Arm the one-shot directory-sync fault from a test outside this module.
/// The fault itself stays at the narrow core `Result` boundary above; this only
/// makes it reachable from the projection tests, which need a move that RENAMED
/// and then failed (GH #543, seventh audit A7-N3).
#[cfg(test)]
pub(crate) fn fail_next_projection_directory_sync() {
    FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| fail.set(true));
}

#[cfg(test)]
pub(crate) fn fail_graph_text_directory_sync_after_mutation() {
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            fail_next_projection_directory_sync();
            Ok(())
        }));
    });
}

#[cfg(test)]
fn projection_directory_sync_hook(_dir: &Path) -> io::Result<()> {
    FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| {
        if fail.replace(false) {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "injected projection directory sync failure",
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(test))]
fn projection_directory_sync_hook(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_inventory_read_hook() -> io::Result<()> {
    GRAPH_TEXT_INVENTORY_READ_RACE.with(|hook| {
        let hook = hook.borrow_mut().take();
        match hook {
            Some(hook) => hook(),
            None => Ok(()),
        }
    })
}

#[cfg(not(test))]
fn graph_text_inventory_read_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_capture_revalidation_hook(_root: &Path) -> io::Result<()> {
    GRAPH_TEXT_CAPTURE_REVALIDATION_RACE.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn graph_text_capture_revalidation_hook(_root: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn bounded_read_after_metadata_hook() -> io::Result<()> {
    BOUNDED_READ_AFTER_METADATA.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn bounded_read_after_metadata_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_write_identity_acquisition_hook() -> io::Result<()> {
    GRAPH_TEXT_WRITE_IDENTITY_ACQUISITION.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn graph_text_write_identity_acquisition_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_write_after_admission_hook() -> io::Result<()> {
    GRAPH_TEXT_WRITE_AFTER_ADMISSION.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn graph_text_write_after_admission_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_write_after_identity_check_hook() {
    GRAPH_TEXT_WRITE_AFTER_IDENTITY_CHECK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn graph_text_write_after_identity_check_hook() {}

#[cfg(test)]
fn graph_text_write_before_mutation_hook() -> io::Result<()> {
    // Take the hook and release the borrow before running it, so a hook can
    // re-arm itself to fire on a later mutation.
    let hook = GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| hook.borrow_mut().take());
    hook.map_or(Ok(()), |hook| hook())
}

#[cfg(not(test))]
fn graph_text_write_before_mutation_hook() -> io::Result<()> {
    Ok(())
}

/// The editor writer's displacement fault point (journal-universal durability
/// design §4.6, W1 / §4.3 row F3).
///
/// It fires strictly between the displacement rename `T -> .editor-recovery`
/// and the publication rename `staged -> T`, so arming it produces the state
/// F3 names: `T` absent, the `.editor-recovery` claim holding the precondition.
/// The design lists a hook here as packet-1 work; the hook already existed with
/// exactly that placement and semantics, so packet 1 documents and tests it
/// rather than adding a second one at the same cut.
///
/// PRODUCTION ARMS NOTHING: the non-test definition is a constant `Ok(())`.
#[cfg(test)]
fn graph_text_write_after_retire_hook() -> io::Result<()> {
    GRAPH_TEXT_WRITE_AFTER_RETIRE.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn graph_text_write_after_retire_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn journal_projection_after_publish_hook() -> io::Result<()> {
    JOURNAL_PROJECTION_AFTER_PUBLISH.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn journal_projection_after_publish_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_write_before_restore_hook() -> io::Result<()> {
    GRAPH_TEXT_WRITE_BEFORE_RESTORE.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn graph_text_write_before_restore_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn graph_text_write_during_rollback_hook() -> io::Result<()> {
    GRAPH_TEXT_WRITE_DURING_ROLLBACK.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn graph_text_write_during_rollback_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn editor_retired_cleanup_hook() -> io::Result<()> {
    EDITOR_RETIRED_CLEANUP.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn editor_retired_cleanup_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn editor_commit_before_recheck_hook() -> io::Result<()> {
    EDITOR_COMMIT_BEFORE_RECHECK.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn editor_commit_before_recheck_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn editor_commit_before_final_reread_hook() -> io::Result<()> {
    EDITOR_COMMIT_BEFORE_FINAL_REREAD.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn editor_commit_before_final_reread_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn exact_graph_text_event_after_candidate_hook() {
    EXACT_GRAPH_TEXT_EVENT_AFTER_CANDIDATE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn exact_graph_text_event_after_candidate_hook() {}

#[cfg(test)]
fn conflict_observation_hook() -> io::Result<()> {
    CONFLICT_OBSERVATION.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn conflict_observation_hook() -> io::Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn rename_source_remove_failpoint() -> io::Result<()> {
    Ok(())
}

/// OG's default when `:ref/linked-references-collapsed-threshold` is absent.
fn default_linked_references_collapsed_threshold() -> u32 {
    100
}

// `PartialEq` is load-bearing, not a convenience: the config watcher refreshes
// a graph and then compares the meta it produced against the meta the frontend
// already has, so a rewrite that changes no setting emits nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Backlink count at which a page opens its Linked References collapsed
    /// (`:ref/linked-references-collapsed-threshold`, OG default 100).
    #[serde(default = "default_linked_references_collapsed_threshold")]
    pub linked_references_collapsed_threshold: u32,
    /// Template name applied to a new, empty journal page (if configured).
    pub default_journal_template: Option<String>,
    /// Graph-portable startup page from `:default-home {:page "..."}`.
    #[serde(default)]
    pub default_home: Option<String>,
    /// Favorited page names (read from config.edn `:favorites`).
    pub favorites: Vec<String>,
    /// The page holding Tine's Favorites arrangement (`:tine/favorites-page`),
    /// when this graph has one. `:favorites` above stays the flat, Logseq-
    /// readable membership list; this page owns groups and order.
    #[serde(default)]
    pub favorites_page: Option<String>,
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

/// The graph-relative location of the configuration file, as `Graph::open`
/// reads it and as the exact-feed classifier names it. One constant, so moving
/// it can never land in one of those and miss the other.
pub const CONFIG_RELATIVE_PATH: &str = "logseq/config.edn";
/// Upper bound on bytes read back from one graph text file or private
/// artifact; a file above this is refused rather than retained.
pub(crate) const MAX_PROJECTION_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;
/// Upper bound on parser nodes admitted from one externally edited source
/// file, so a pathological file cannot exhaust memory during a scan.
const MAX_GRAPH_TEXT_PARSER_NODES: u64 = 1_000_000;

fn graph_text_capture_error(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.into())
}

/// Re-exported so a caller outside the crate can name what
/// [`config_file_description`] and [`Graph::open_config_description`] return.
pub use crate::graph_text_path::BlobDescription as ConfigDescription;

/// Digest `logseq/config.edn` as it stands on disk right now, resolving the
/// path exactly as `Graph::open` does.
///
/// `None` means "no readable configuration file", which is precisely what
/// `open` would have parsed as an empty `Config` -- so a `None` here and a
/// `None` from [`Graph::open_config_description`] agree that nothing changed.
pub fn config_file_description(root: &Path) -> Option<BlobDescription> {
    fs::read(reconciliation_scan_config_path_at_open(root))
        .ok()
        .map(|bytes| BlobDescription::of(&bytes))
}

/// Is `path` the configuration file of the graph rooted at `root`?
///
/// Case-insensitive, like the open path and the classifier: a graph delivered
/// by a case-folding filesystem may spell it `Logseq/Config.edn`.
pub fn is_config_file_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let Some(relative) = relative.to_str() else {
        return false;
    };
    relative
        .replace(std::path::MAIN_SEPARATOR, "/")
        .eq_ignore_ascii_case(CONFIG_RELATIVE_PATH)
}

pub(crate) fn reconciliation_scan_config_path_at_open(root: &Path) -> PathBuf {
    let exact = root.join("logseq").join("config.edn");
    let matching = |directory: &Path, expected: &str| -> Option<PathBuf> {
        let mut found = None;
        for entry in fs::read_dir(directory).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name = name.to_str()?;
            if name.eq_ignore_ascii_case(expected) {
                if found.is_some() {
                    return None;
                }
                found = Some(entry.path());
            }
        }
        found
    };
    let Some(logseq) = matching(root, "logseq") else {
        return exact;
    };
    matching(&logseq, "config.edn").unwrap_or(exact)
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
