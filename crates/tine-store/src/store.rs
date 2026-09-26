//! Store boundary. `Store` owns the graph, its watcher, publication queue and
//! guarded writer. `open` validates the layout, lists pages and journals, then
//! starts background parsing and the per-store watcher. Cost O(P metadata) on
//! the caller; the load and watch run in the background. `subscribe` delivers
//! every published generation in order with no replay or queue bound; a second
//! subscription ends the first. Own commits and restore publish `Origin::Own`;
//! watcher reconciliation and `scan_refresh` publish `Origin::External`.
//! `set_watch_mode` changes between notify with a 200 ms debounce and a 3 s
//! poll, with notify failure falling back to polling. `scan_refresh` reads
//! metadata for all page files and config, then changed bytes, at cost
//! O(P metadata + changed bytes). It reports `LoadError::Closed` after close or
//! `Failed` for a lost root or unsafe config layout. `close` stops observation,
//! waits for a writer, releases load waiters and ends the subscription. Callers
//! need no watcher, cache, lock or file layout state.
//!
//! `page` reads and
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
//! `WholeGraph` still uses the interim live cache: two calls on one view may
//! observe different states. The `GraphRev` on a view records the publication
//! generation observed when it was acquired.
//! `trash_stats` scans recoverable entries in O(trash entries), returning typed
//! counts and bytes or an I/O error. `purge_asset_trash` irreversibly removes
//! asset and legacy-asset entries in O(asset trash entries + metadata), leaving
//! other kinds intact. On an error it returns completed removal counts and
//! bytes. Callers need no trash layout or legacy name classifier.
//! `scan_area` lists regular files in O(entries), sorted by area-relative name,
//! and reports stat/list failures. `WholeGraph::referenced_assets` walks page
//! text in O(B) on the interim live view. Clients compare these answers without
//! opening graph paths.
//! `create_graph` selects an empty parent or first unused demo child and writes
//! the scaffold and seed by no-replace create. Cost O(siblings probed + seed
//! bytes). Invalid folders and partial creation failures are typed `OpenError`s;
//! callers need no folder naming or scaffold protocol.
//! `restore` takes verified open backup files and replaces graph text by
//! no-replace publication, retiring replaced and extra files into recovery on
//! each live filesystem. Cost O(input bytes + live text entries). It reports
//! completed work and recovery locations on a partial failure; callers need no
//! graph layout, recovery path, or file move protocol.
//!
//! `WholeGraph::resolve` answers a name with an existing or proposed `PageId`.
//! It may build the graph cache on first use (O(P + B + disk)); a warm absent or
//! alias lookup still scans O(aliases).
//! `transaction` collects named file steps. Commit preflights all steps before
//! writing, applies them under sorted path locks, and undoes a failed apply into
//! recoverable trash. Its cost is O(bytes of named files + affected page blocks).
//! `save` is one `save_page` step: it writes exactly the supplied `PageId`,
//! preserving legacy formatting, no-op, Org, preamble, and cache rules. Its
//! cost is O(page bytes + its blocks). Conflicts, deletion, read-only pages,
//! twins, invalid targets, and I/O are typed outcomes; callers keep unsaved
//! edits on refusal. No caller manages page locks, cache state, or paths.
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

use std::collections::{BTreeMap, VecDeque};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::SystemTime;

use crate::model::{classify_legacy_trash_entry, trash_dir_kind, trash_root, TrashEntryKind};

use serde::{Deserialize, Serialize};
use tine_core::date::JournalDate;
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockPreview, BoundedRefGroups, PageDto,
    PageEntry, PageKind, RefGroup, TemplateDto,
};
pub use tine_core::model::{FileId, PageId};
use tine_core::query::{AdvancedResult, QueryExportBatch, QueryExportSpec};
use tine_core::query_plan::QueryExecution;

use crate::model::{CheckedOpenError, Graph};

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
    pub(crate) graph: Arc<Graph>,
    pub(crate) writer: Arc<Mutex<()>>,
    load: Arc<LoadState>,
    config_state: Arc<RwLock<ConfigState>>,
    journal_ids: Arc<Mutex<HashMap<Day, PageId>>>,
    pub(crate) changes: Arc<ChangeFeed>,
    pub(crate) watch: crate::watch::WatchHandle,
    #[cfg(any(test, feature = "test-faults"))]
    pub(crate) faults: std::sync::Mutex<std::collections::HashSet<crate::transaction::FaultPoint>>,
}

impl Drop for Store {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) struct LoadState {
    cancelled: AtomicBool,
    closed: AtomicBool,
    pub(crate) status: Mutex<LoadStatus>,
    pub(crate) ready: Condvar,
}

#[derive(Clone)]
pub(crate) enum LoadStatus {
    Loading,
    Ready,
    Failed(String),
    Closed,
}

impl LoadState {
    fn new(status: LoadStatus) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            status: Mutex::new(status),
            ready: Condvar::new(),
        }
    }
}

/// Consent supplied by the device for a graph's external assets directory.
#[derive(Default)]
pub struct OpenOptions {
    pub approved_external_assets: Option<PathBuf>,
    pub watch: WatchMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WatchMode {
    #[default]
    Notify,
    Poll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Own,
    External,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
    Touched,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub graph_rev: GraphRev,
    pub origin: Origin,
    pub files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    pub config_changed: bool,
    pages: Vec<(FileId, PageKind, String)>,
}

impl Change {
    /// The graph page this external file change altered, named as the graph
    /// names it (by `title::` if set; the name before removal for `Removed`).
    /// `None` when the parsed document did not change, or the file is not a
    /// graph page (a shadow journal, a conflict copy), as v0.6.5's watcher.
    /// Not in rev 5: `ChangeKind` is byte-level, window events are page-level.
    /// Cost O(files in this publication).
    pub fn page(&self, file: &FileId) -> Option<(PageKind, &str)> {
        self.pages
            .iter()
            .find(|(id, _, _)| id == file)
            .map(|(_, kind, name)| (*kind, name.as_str()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

pub(crate) struct ChangeFeed {
    state: Mutex<FeedState>,
    ready: Condvar,
}

struct FeedState {
    rev: u64,
    subscription: u64,
    queue: VecDeque<Change>,
    closed: bool,
}

impl ChangeFeed {
    fn new() -> Self {
        Self {
            state: Mutex::new(FeedState {
                rev: 0,
                subscription: 0,
                queue: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }

    pub(crate) fn publish(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
    ) -> GraphRev {
        let mut state = self.state.lock().unwrap();
        state.rev += 1;
        let rev = GraphRev(state.rev);
        if !state.closed {
            state.queue.push_back(Change {
                graph_rev: rev,
                origin,
                files,
                config_changed,
                pages,
            });
            self.ready.notify_all();
        }
        rev
    }

    pub(crate) fn rev(&self) -> GraphRev {
        GraphRev(self.state.lock().unwrap().rev)
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.queue.clear();
        self.ready.notify_all();
    }
}

pub struct Subscription {
    feed: Arc<ChangeFeed>,
    number: u64,
}

impl Subscription {
    pub fn recv(&self) -> Result<Change, Closed> {
        let mut state = self.feed.state.lock().unwrap();
        loop {
            if state.closed || state.subscription != self.number {
                return Err(Closed);
            }
            if let Some(change) = state.queue.pop_front() {
                return Ok(change);
            }
            state = self.feed.ready.wait(state).unwrap();
        }
    }

    pub fn try_recv(&self) -> Result<Option<Change>, Closed> {
        let mut state = self.feed.state.lock().unwrap();
        if state.closed || state.subscription != self.number {
            return Err(Closed);
        }
        Ok(state.queue.pop_front())
    }
}

/// Canonical graph root and any external assets target. Inspection writes nothing.
pub struct GraphAccessInspection {
    pub root: PathBuf,
    pub external_assets: Option<PathBuf>,
}

impl GraphAccessInspection {
    /// Compare a user-approved device path with the live external assets target.
    pub fn approves_external_assets(&self, path: &Path) -> std::io::Result<bool> {
        Ok(self.external_assets.as_ref() == Some(&fs::canonicalize(path)?))
    }
}

#[derive(Debug)]
pub enum OpenError {
    NotAFolder(PathBuf),
    Unresolvable {
        path: PathBuf,
        reason: String,
    },
    UnsafeLayout(String),
    ExternalAssetsUnapproved {
        current: PathBuf,
    },
    CreateFailed {
        path: PathBuf,
        cause: crate::IoError,
    },
    Io(crate::IoError),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFolder(path) => write!(f, "graph path is not a folder: {}", path.display()),
            Self::Unresolvable { path, reason } => {
                write!(
                    f,
                    "couldn't resolve graph path {}: {reason}",
                    path.display()
                )
            }
            Self::UnsafeLayout(message) => f.write_str(message),
            Self::ExternalAssetsUnapproved { current } => write!(
                f,
                "external assets directory requires approval: {}",
                current.display()
            ),
            Self::CreateFailed { path, cause } => write!(
                f,
                "couldn't create graph {}: {}",
                path.display(),
                cause.message
            ),
            Self::Io(error) => f.write_str(&error.message),
        }
    }
}

/// Effective config; unreadable config is already defaulted by legacy open.
#[derive(Clone)]
pub struct ConfigState {
    pub config: Arc<tine_core::config::Config>,
    pub problem: Option<crate::IoError>,
}

impl std::ops::Deref for ConfigState {
    type Target = tine_core::config::Config;
    fn deref(&self) -> &Self::Target {
        &self.config
    }
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

pub(crate) fn journal_ids_from_entries(
    graph: &Graph,
    entries: Vec<PageEntry>,
) -> HashMap<Day, PageId> {
    let mut claimants: HashMap<Day, Vec<PageEntry>> = HashMap::new();
    for entry in entries {
        if entry.kind == PageKind::Journal {
            if let Some(day) = entry.date_key {
                claimants.entry(Day(day)).or_default().push(entry);
            }
        }
    }
    claimants
        .into_iter()
        .filter_map(|(day, mut entries)| {
            entries.sort_by(|a, b| {
                crate::model::compare_page_claimants(a, b, &graph.current_journal_format())
            });
            entries.into_iter().next()?.rel_path.map(|id| (day, id))
        })
        .collect()
}

impl Store {
    #[cfg(test)]
    pub(crate) fn from_graph_for_tests(graph: Arc<Graph>) -> Self {
        graph.install_live_config();
        let writer = Arc::new(Mutex::new(()));
        let load = Arc::new(LoadState::new(LoadStatus::Ready));
        let changes = Arc::new(ChangeFeed::new());
        let journal_ids = Arc::new(Mutex::new(journal_ids_from_entries(
            &graph,
            graph.list_pages(),
        )));
        let config_state = Arc::new(RwLock::new(ConfigState {
            config: Arc::new(graph.config.clone()),
            problem: None,
        }));
        let watch = crate::watch::WatchHandle::start(
            Arc::clone(&graph),
            Arc::clone(&writer),
            Arc::clone(&load),
            Arc::clone(&changes),
            Arc::clone(&journal_ids),
            Arc::clone(&config_state),
            WatchMode::Notify,
        );
        Self {
            config_state,
            graph,
            writer,
            load,
            journal_ids,
            changes,
            watch,
            faults: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Scaffold a graph in an empty parent, or in the first unused tine-demo
    /// child. Cost O(siblings probed + seed bytes). Partial failures leave the
    /// created files in place and identify the failing path.
    pub fn create_graph(
        parent: &Path,
        seed: &[(Area, String, Vec<u8>)],
    ) -> Result<PathBuf, OpenError> {
        if parent.as_os_str().is_empty() || !parent.is_dir() {
            return Err(OpenError::NotAFolder(parent.to_path_buf()));
        }
        let failed = |path: &Path, error: std::io::Error| OpenError::CreateFailed {
            path: path.to_path_buf(),
            cause: error.into(),
        };
        let empty = fs::read_dir(parent)
            .map_err(|error| failed(parent, error))?
            .next()
            .is_none();
        let root = if empty {
            parent.to_path_buf()
        } else {
            let mut number = 1usize;
            loop {
                let name = if number == 1 {
                    "tine-demo".to_string()
                } else {
                    format!("tine-demo-{number}")
                };
                let candidate = parent.join(name);
                match fs::create_dir(&candidate) {
                    Ok(()) => break candidate,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        number = number.checked_add(1).ok_or_else(|| {
                            failed(
                                &candidate,
                                std::io::Error::other("no unused demo folder name"),
                            )
                        })?;
                    }
                    Err(error) => return Err(failed(&candidate, error)),
                }
            }
        };
        for area in ["logseq", "pages", "journals", "assets"] {
            let dir = root.join(area);
            fs::create_dir_all(&dir).map_err(|error| failed(&dir, error))?;
        }
        let config = root.join("logseq/config.edn");
        let supplied_config = seed
            .iter()
            .find(|(area, rel, _)| *area == Area::Meta && rel == "config.edn");
        let config_bytes = supplied_config
            .map(|(_, _, bytes)| bytes.as_slice())
            .unwrap_or(tine_core::guide::CONFIG_EDN.as_bytes());
        crate::model::atomic_write_new(&config, config_bytes)
            .map_err(|error| failed(&config, error))?;
        let mut used_config_seed = false;
        for (area, rel, bytes) in seed {
            if *area == Area::Meta && rel == "config.edn" && !used_config_seed {
                used_config_seed = true;
                continue;
            }
            if rel.is_empty()
                || rel.split('/').any(|part| {
                    part.is_empty() || part == "." || part == ".." || part.contains('\\')
                })
            {
                return Err(failed(
                    &root,
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid seed file name"),
                ));
            }
            let base = match area {
                Area::Pages => root.join("pages"),
                Area::Journals => root.join("journals"),
                Area::Assets => root.join("assets"),
                Area::Meta => root.join("logseq"),
                Area::Trash => {
                    return Err(failed(
                        &root,
                        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid seed area"),
                    ))
                }
            };
            let path = base.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| failed(parent, error))?;
            }
            crate::model::atomic_write_new(&path, bytes).map_err(|error| failed(&path, error))?;
        }
        Ok(root)
    }

    /// Resolve a user-chosen graph root and require a folder, without writing.
    pub fn canonical_root(root: &Path) -> Result<PathBuf, OpenError> {
        let canonical = fs::canonicalize(root).map_err(|error| OpenError::Unresolvable {
            path: root.to_path_buf(),
            reason: error.to_string(),
        })?;
        if !canonical.is_dir() {
            return Err(OpenError::NotAFolder(canonical));
        }
        Ok(canonical)
    }

    /// Resolve the graph root and external assets target without writing.
    pub fn inspect(root: &Path) -> Result<GraphAccessInspection, OpenError> {
        let canonical = Self::canonical_root(root)?;
        let external_assets = Graph::external_assets_target(&canonical)
            .map_err(|error| OpenError::Io(error.into()))?;
        Ok(GraphAccessInspection {
            root: canonical,
            external_assets,
        })
    }

    /// Validate the v0.6.5 layout in its original order and start the load.
    pub fn open(
        root: &Path,
        opts: OpenOptions,
    ) -> Result<(Self, tine_core::model::GraphMeta, ConfigState), OpenError> {
        let root = fs::canonicalize(root).map_err(|error| OpenError::Unresolvable {
            path: root.to_path_buf(),
            reason: error.to_string(),
        })?;
        if !root.is_dir() {
            return Err(OpenError::NotAFolder(root));
        }
        let graph =
            Graph::open_checked_with_assets_inner(&root, opts.approved_external_assets.as_deref())
                .map_err(|error| match error {
                    CheckedOpenError::ExternalAssetsUnapproved(current) => {
                        OpenError::ExternalAssetsUnapproved { current }
                    }
                    CheckedOpenError::Io(error)
                        if error.kind() == std::io::ErrorKind::InvalidInput =>
                    {
                        OpenError::UnsafeLayout(error.to_string())
                    }
                    CheckedOpenError::Io(error) => OpenError::Io(error.into()),
                })?;
        graph.install_live_config();
        // Build the legacy filename inventory before returning; parsing remains
        // in the cancellable worker below.
        let journal_ids = journal_ids_from_entries(&graph, graph.list_pages());
        let config_path = root.join("logseq/config.edn");
        let problem = match fs::read_to_string(config_path) {
            Ok(_) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => Some(error.into()),
        };
        let graph = Arc::new(graph);
        let config = ConfigState {
            config: Arc::new(graph.config.clone()),
            problem,
        };
        let meta = tine_core::model::GraphMeta::from_config(
            root.display().to_string(),
            &config.config,
            &graph.journal_format,
        );
        let load = Arc::new(LoadState::new(LoadStatus::Loading));
        let writer = Arc::new(Mutex::new(()));
        let changes = Arc::new(ChangeFeed::new());
        let journal_ids = Arc::new(Mutex::new(journal_ids));
        let config_state = Arc::new(RwLock::new(config.clone()));
        let watch = crate::watch::WatchHandle::start(
            Arc::clone(&graph),
            Arc::clone(&writer),
            Arc::clone(&load),
            Arc::clone(&changes),
            Arc::clone(&journal_ids),
            Arc::clone(&config_state),
            opts.watch,
        );
        let worker_graph = Arc::clone(&graph);
        let worker_load = Arc::clone(&load);
        let worker_writer = Arc::clone(&writer);
        let worker_changes = Arc::clone(&changes);
        let worker_watch = watch.core_for_load();
        let worker_watch_wake = watch.wake_for_load();
        std::thread::spawn(move || {
            #[cfg(any(test, feature = "test-faults"))]
            while worker_graph.root.join(".tine-test-pause-load").exists()
                && !worker_load.cancelled.load(Ordering::Acquire)
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let completed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                let completed = worker_graph
                    .warm_cache_cancellable(|| worker_load.cancelled.load(Ordering::Acquire));
                if completed || worker_load.cancelled.load(Ordering::Acquire) {
                    break completed;
                }
            }));
            let _writer = worker_writer.lock().unwrap();
            if matches!(completed, Ok(true)) {
                worker_watch.fill_revs();
            }
            let mut status = worker_load.status.lock().unwrap();
            if matches!(*status, LoadStatus::Loading) {
                *status = if matches!(completed, Ok(true)) {
                    LoadStatus::Ready
                } else {
                    LoadStatus::Failed("background graph load stopped".into())
                };
                if matches!(*status, LoadStatus::Ready) {
                    worker_changes.publish(Origin::External, Vec::new(), false, Vec::new());
                    let _ = worker_watch_wake.send(());
                }
            }
            worker_load.ready.notify_all();
        });
        Ok((
            Self {
                graph,
                writer,
                load,
                config_state,
                journal_ids,
                changes,
                watch,
                #[cfg(any(test, feature = "test-faults"))]
                faults: Mutex::new(std::collections::HashSet::new()),
            },
            meta,
            config,
        ))
    }

    /// Current graph configuration. Never waits.
    pub fn config(&self) -> ConfigState {
        self.config_state.read().unwrap().clone()
    }

    /// Stop the background load and refuse later I/O. Idempotent.
    pub fn close(&self) {
        self.watch.stop();
        let _writer = self.writer.lock().unwrap();
        self.load.closed.store(true, Ordering::Release);
        self.load.cancelled.store(true, Ordering::Release);
        *self.load.status.lock().unwrap() = LoadStatus::Closed;
        self.load.ready.notify_all();
        self.changes.close();
    }

    pub fn subscribe(&self) -> Subscription {
        let mut state = self.changes.state.lock().unwrap();
        state.subscription += 1;
        state.queue.clear();
        self.changes.ready.notify_all();
        Subscription {
            feed: Arc::clone(&self.changes),
            number: state.subscription,
        }
    }

    pub fn set_watch_mode(&self, mode: WatchMode) {
        if !self.is_closed() {
            self.watch.set_mode(mode);
        }
    }

    pub fn scan_refresh(&self) -> Result<(), LoadError> {
        self.watch.scan_refresh()
    }

    pub(crate) fn publish_own(
        &self,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    ) -> GraphRev {
        let ids: Vec<FileId> = files.iter().map(|(id, _, _)| id.clone()).collect();
        self.watch.note_own(&ids);
        let config_changed = ids.iter().any(|id| id.as_str() == "logseq/config.edn");
        self.refresh_journal_ids();
        self.changes
            .publish(Origin::Own, files, config_changed, Vec::new())
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.load.closed.load(Ordering::Acquire)
    }

    pub(crate) fn refresh_journal_ids(&self) {
        let found = journal_ids_from_entries(&self.graph, self.graph.list_pages());
        *self.journal_ids.lock().unwrap() = found;
    }

    /// Resolve the canonical journal file for a day, or the preferred new file.
    /// Interim implementation uses the legacy claimant index, which can build
    /// from journal names on first use (O(journal entries)); warm lookup O(1).
    pub fn journal_id(&self, day: Day) -> PageId {
        let date = JournalDate::from_ordinal(day.0);
        if let Some(id) = self.journal_ids.lock().unwrap().get(&day) {
            return id.clone();
        }
        PageId::from(format!(
            "{}/{}.{}",
            self.graph.current_config().journals_dir,
            self.graph.current_journal_format().file_stem(date),
            self.graph.current_config().preferred_format.ext()
        ))
    }
    /// Save one page with the editor's baseline; no write occurs on refusal.
    pub fn save(&self, id: &PageId, base: SaveBase, doc: &PageDto) -> SaveOutcome {
        if self.is_closed() {
            return SaveOutcome::Closed;
        }
        if doc.guide {
            return SaveOutcome::GuideEphemeral;
        }
        let mut tx = self.transaction();
        tx.save_page(id, base, doc);
        match tx.commit() {
            crate::TxOutcome::Committed { mut steps, .. } => match steps.remove(0) {
                crate::StepResult::Written { rev, .. } => SaveOutcome::Saved(rev),
                crate::StepResult::Unchanged { rev, .. } => SaveOutcome::Unchanged(rev),
                _ => unreachable!("save_page result"),
            },
            crate::TxOutcome::NotCommitted { why, .. } => match why {
                crate::Why::Conflict {
                    file,
                    disk: Some(disk),
                } if file != id.file() => SaveOutcome::Conflict { disk },
                crate::Why::Conflict {
                    disk: Some(disk), ..
                } => SaveOutcome::Conflict { disk },
                crate::Why::Conflict { disk: None, .. } => SaveOutcome::Deleted,
                crate::Why::Refused(crate::Refusal::ReadOnly(reason)) => {
                    SaveOutcome::ReadOnly(reason)
                }
                crate::Why::Refused(crate::Refusal::Twin { existing }) => {
                    SaveOutcome::Twin { existing }
                }
                crate::Why::Refused(crate::Refusal::InvalidTarget(reason)) => {
                    SaveOutcome::InvalidTarget(reason)
                }
                crate::Why::Refused(crate::Refusal::Closed) => SaveOutcome::Closed,
                crate::Why::Refused(other) => SaveOutcome::InvalidTarget(format!("{other:?}")),
                crate::Why::Failed(error) => {
                    SaveOutcome::Io(std::io::Error::new(error.kind, error.message))
                }
            },
        }
    }

    /// Count entries and bytes by kind in the recoverable trash. Cost: O(trash entries).
    pub fn trash_stats(&self) -> Result<Vec<(TrashKind, u64, u64)>, StoreError> {
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
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
        let _writer = self.writer.lock().unwrap();
        if self.is_closed() {
            return Err((StoreError::Closed, 0, 0));
        }
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
        if area == Area::Meta && rel.starts_with(".tine-") {
            return Err(StoreError::InvalidTarget(rel.into()));
        }
        let directory = match area {
            Area::Pages => &self.graph.current_config().pages_dir,
            Area::Journals => &self.graph.current_config().journals_dir,
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
        if !path.starts_with(&format!("{}/", self.graph.current_config().pages_dir))
            && !path.starts_with(&format!("{}/", self.graph.current_config().journals_dir))
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

    pub(crate) fn validate_file(&self, file: &FileId) -> Result<(), StoreError> {
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
        if !path.starts_with(&format!("{}/", self.graph.current_config().pages_dir))
            && !path.starts_with(&format!("{}/", self.graph.current_config().journals_dir))
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
        let config = self.graph.current_config();
        let area = if path.starts_with(&format!("{}/", config.pages_dir)) {
            config.pages_dir.as_str()
        } else if path.starts_with(&format!("{}/", config.journals_dir)) {
            config.journals_dir.as_str()
        } else {
            path.split('/').next().unwrap_or_default()
        };
        Ok(self.graph.root.join(area))
    }

    /// A validated OS path. A missing final file is allowed; existing ancestors
    /// must still remain inside the selected area. Callers requiring an existing
    /// regular file must check that separately.
    pub fn path_for_os_handoff(&self, file: &FileId) -> Result<PathBuf, StoreError> {
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
        let area = self.area_root(file)?;
        let (area, candidate) = if let Some(rel) = file.as_str().strip_prefix("assets/") {
            let approved = self.graph.assets_path();
            let lexical = self.graph.root.join("assets");
            let live = match fs::canonicalize(&lexical) {
                Ok(path) => path,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && approved == lexical =>
                {
                    lexical
                }
                Err(error) => return Err(StoreError::from_io(error)),
            };
            if live != approved {
                return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
            }
            (approved.clone(), approved.join(rel))
        } else {
            (area, self.graph.root.join(file.as_str()))
        };
        let (area_canonical, area_missing) = match fs::canonicalize(&area) {
            Ok(path) => (path, false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let root = fs::canonicalize(&self.graph.root).map_err(StoreError::from_io)?;
                (
                    root.join(
                        area.strip_prefix(&self.graph.root)
                            .map_err(|_| StoreError::InvalidTarget(file.as_str().to_owned()))?,
                    ),
                    true,
                )
            }
            Err(error) => return Err(StoreError::from_io(error)),
        };
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
        if !candidate.starts_with(&area)
            || (!resolved.starts_with(&area_canonical)
                && !(area_missing && area_canonical.starts_with(&resolved)))
        {
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

    /// Return an existing regular asset for an OS hand-off, after checking its
    /// live assets directory and resolved target. Cost: O(path components).
    pub fn asset_for_os_handoff(&self, file: &FileId) -> Result<PathBuf, StoreError> {
        if !file.as_str().starts_with("assets/") {
            return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
        }
        let target = self.path_for_os_handoff(file)?;
        let target = fs::canonicalize(target).map_err(StoreError::from_io)?;
        let assets = fs::canonicalize(self.graph.assets_path()).map_err(StoreError::from_io)?;
        if !target.starts_with(&assets) || !target.is_file() {
            return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
        }
        Ok(target)
    }

    /// Return an existing page source for an OS hand-off. A page symlink may
    /// lead from pages to journals or conversely, as in the legacy opener.
    /// Cost: O(path components).
    pub fn page_for_os_handoff(&self, id: &PageId) -> Result<PathBuf, StoreError> {
        if self.as_page(&id.file()).is_none() {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        let target =
            fs::canonicalize(self.graph.root.join(id.as_str())).map_err(StoreError::from_io)?;
        if !target.is_file() {
            return Err(StoreError::InvalidTarget(
                "page source is not a file".into(),
            ));
        }
        let config = self.graph.current_config();
        let pages = fs::canonicalize(self.graph.root.join(&config.pages_dir))
            .map_err(StoreError::from_io)?;
        let journals = fs::canonicalize(self.graph.root.join(&config.journals_dir))
            .map_err(StoreError::from_io)?;
        if !target.starts_with(&pages) && !target.starts_with(&journals) {
            return Err(StoreError::InvalidTarget(
                "page source escapes graph directories".into(),
            ));
        }
        Ok(target)
    }

    /// Display the recoverable asset trash location in a user-facing error.
    pub fn asset_trash_location_for_user(&self) -> PathBuf {
        self.graph.root.join("logseq/.tine-trash/assets")
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
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
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

    /// Recursively list regular files in one area, sorted by area-relative name.
    /// Hidden entries and symlinked directories are skipped; stat/list failures
    /// are reported in `unreadable`. An absent `under` is empty. Cost O(entries).
    pub fn scan_area(&self, area: Area, under: Option<&str>) -> Result<Listing, StoreError> {
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
        if let Some(rel) = under {
            self.file_id(area, rel)?;
        }
        let root = match area {
            Area::Pages => self.graph.root.join(&self.graph.current_config().pages_dir),
            Area::Journals => self
                .graph
                .root
                .join(&self.graph.current_config().journals_dir),
            Area::Assets => {
                let live = match fs::canonicalize(self.graph.root.join("assets")) {
                    Ok(path) => path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(Listing::default())
                    }
                    Err(error) => return Err(StoreError::from_io(error)),
                };
                if live != self.graph.assets_path() {
                    return Err(StoreError::InvalidTarget("assets".into()));
                }
                live
            }
            Area::Meta | Area::Trash => self.graph.root.join("logseq"),
        };
        let root = if area == Area::Trash {
            root.join(".tine-trash")
        } else {
            root
        };
        let start = if let Some(rel) = under {
            let dir = self.file_id(area, rel)?;
            if fs::symlink_metadata(root.join(rel)).is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Ok(Listing::default());
            }
            self.path_for_os_handoff(&dir)?
        } else {
            root.clone()
        };
        let mut listing = Listing::default();
        fn walk(store: &Store, area: Area, root: &Path, dir: &Path, out: &mut Listing) {
            #[cfg(test)]
            let forced = SCAN_FAULTS.with(|faults| {
                let rel = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy();
                (faults.borrow().1.as_deref() == Some(rel.as_ref())).then(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected list failure",
                    )
                })
            });
            #[cfg(not(test))]
            let forced: Option<std::io::Error> = None;
            let entries = match forced.map_or_else(|| fs::read_dir(dir), Err) {
                Ok(entries) => entries,
                Err(error) => {
                    out.unreadable.push((
                        dir.strip_prefix(root)
                            .unwrap_or(dir)
                            .to_string_lossy()
                            .replace('\\', "/"),
                        error.into(),
                    ));
                    return;
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        out.unreadable.push((String::new(), error.into()));
                        continue;
                    }
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if area == Area::Meta && rel != "config.edn" && rel != "custom.css" {
                    continue;
                }
                #[cfg(test)]
                let forced = SCAN_FAULTS.with(|faults| {
                    (faults.borrow().0.as_deref() == Some(rel.as_str())).then(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "injected stat failure",
                        )
                    })
                });
                #[cfg(not(test))]
                let forced: Option<std::io::Error> = None;
                let ty = match forced.map_or_else(|| entry.file_type(), Err) {
                    Ok(ty) => ty,
                    Err(error) => {
                        out.unreadable.push((rel, error.into()));
                        continue;
                    }
                };
                if ty.is_dir() {
                    walk(store, area, root, &path, out);
                } else if ty.is_file() {
                    match entry.metadata() {
                        Ok(meta) => {
                            if let Ok(id) = store.file_id(area, &rel) {
                                out.files.push(FileEntry {
                                    page: store.as_page(&id),
                                    day: if area == Area::Journals {
                                        std::path::Path::new(&rel)
                                            .file_stem()
                                            .and_then(|stem| stem.to_str())
                                            .and_then(|stem| {
                                                store.graph.current_journal_format().parse(stem)
                                            })
                                            .map(|date| Day(date.ordinal_key()))
                                    } else {
                                        None
                                    },
                                    date_stem: area == Area::Journals
                                        && std::path::Path::new(&rel)
                                            .file_stem()
                                            .and_then(|stem| stem.to_str())
                                            .is_some_and(|stem| {
                                                store
                                                    .graph
                                                    .current_journal_format()
                                                    .parse(stem)
                                                    .is_some_and(|date| {
                                                        store
                                                            .graph
                                                            .current_journal_format()
                                                            .file_stem(date)
                                                            == stem
                                                    })
                                            }),
                                    id,
                                    area,
                                    rel,
                                    meta: Some(FileMeta {
                                        len: meta.len(),
                                        mtime: meta.modified().ok(),
                                    }),
                                });
                            }
                        }
                        Err(error) => out.unreadable.push((rel, error.into())),
                    }
                }
            }
        }
        match fs::symlink_metadata(&start) {
            Ok(_) => walk(self, area, &root, &start, &mut listing),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => listing
                .unreadable
                .push((under.unwrap_or("").to_owned(), error.into())),
        }
        listing.files.sort_by(|a, b| a.rel.cmp(&b.rel));
        listing.unreadable.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(listing)
    }

    /// Read and parse one page. This can advance the live cache when its bytes
    /// differ from the cached copy (interim behavior, before immutable D3).
    /// Cost: O(page bytes + its blocks).
    pub fn page(&self, id: &PageId) -> Result<PageRead, StoreError> {
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
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
            #[cfg(test)]
            if fs::read_to_string(&path)
                .is_ok_and(|text| text.contains("__TINE_TEST_PAGE_PARSE_PANIC__"))
            {
                panic!("deterministic test page parser panic");
            }
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

    /// Wait for the initial load, then get a live-cache read view.
    pub fn whole_graph(&self) -> Result<WholeGraph, LoadError> {
        let mut status = self.load.status.lock().unwrap();
        while matches!(*status, LoadStatus::Loading) {
            status = self.load.ready.wait(status).unwrap();
        }
        match &*status {
            LoadStatus::Closed => return Err(LoadError::Closed),
            LoadStatus::Failed(reason) => {
                return Err(LoadError::Failed {
                    reason: reason.clone(),
                })
            }
            LoadStatus::Ready => {}
            LoadStatus::Loading => unreachable!(),
        }
        Ok(WholeGraph {
            graph: Arc::clone(&self.graph),
            rev: self.changes.rev(),
            observed_mtimes: self.graph.observed_page_mtimes(),
            unreadable: self.graph.unreadable_pages(),
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

/// One listed file. `rel` is the exact name within its area; metadata is
/// present only when the file could be statted. Cost O(1) to inspect.
pub struct FileEntry {
    pub id: FileId,
    pub area: Area,
    pub rel: String,
    pub page: Option<PageId>,
    /// Parsed journal day under configured and fallback formats; cost O(1).
    pub day: Option<Day>,
    /// Whether the stem is the configured filename form; cost O(1).
    pub date_stem: bool,
    pub meta: Option<FileMeta>,
}

/// Metadata observed during a scan. Modification time may be unavailable.
/// Cost O(1) to inspect.
pub struct FileMeta {
    pub len: u64,
    pub mtime: Option<SystemTime>,
}

/// Files found by `scan_area`; unreadable entries retain their area-relative
/// names and I/O errors. Cost O(files + unreadable entries) to inspect.
#[derive(Default)]
pub struct Listing {
    pub files: Vec<FileEntry>,
    pub unreadable: Vec<(String, crate::IoError)>,
}

#[cfg(test)]
thread_local! {
    static SCAN_FAULTS: std::cell::RefCell<(Option<String>, Option<String>)> =
        const { std::cell::RefCell::new((None, None)) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(format!("{hash:016x}"))
    }

    pub(crate) fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buf)?;
            if count == 0 {
                break;
            }
            for byte in &buf[..count] {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        Ok(Self(format!("{hash:016x}")))
    }
}

impl From<FileRev> for String {
    fn from(rev: FileRev) -> Self {
        rev.0
    }
}

impl From<String> for FileRev {
    fn from(rev: String) -> Self {
        Self(rev)
    }
}

/// The file revision the edit is based on, or a request to create a new file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveBase {
    Existing(FileRev),
    CreateNew,
}

/// Result of one guarded page save. A refusal never authorizes dropping edits.
#[derive(Debug)]
pub enum SaveOutcome {
    Saved(FileRev),
    Unchanged(FileRev),
    Conflict {
        disk: FileRev,
    },
    Deleted,
    ReadOnly(String),
    Twin {
        existing: PageId,
    },
    InvalidTarget(String),
    Io(std::io::Error),
    /// Reserved for the B7 store lifecycle.
    Closed,
    /// Interim bundled-guide result; guide pages have no disk identity.
    GuideEphemeral,
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
/// Physical names, aliases, and names that occur only in references, sorted by
/// the graph's page identity key. Physical entries retain every file claimant.
pub struct InventoryEntry {
    pub name: String,
    pub target: Resolved,
    pub is_journal: bool,
    pub day: Option<Day>,
}

pub struct Inventory(pub Vec<InventoryEntry>);
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
pub struct GraphRev(pub(crate) u64);

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
    observed_mtimes: Arc<HashMap<String, std::time::SystemTime>>,
    unreadable: Arc<Vec<(FileId, String)>>,
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
    /// Page files and subdirectories skipped during the initial or latest cache
    /// build, with a displayable reason for each. Cost O(1).
    pub fn unreadable_files(&self) -> &[(FileId, String)] {
        &self.unreadable
    }

    /// Copy the current parsed-page table into a read-only evaluator input.
    /// Interim cost is O(P) after a possible first cache build of O(P + B + disk).
    /// Parsed documents are shared by `Arc`.
    pub fn corpus(&self) -> tine_core::Corpus {
        let pages = self.graph.with_pages(|pages| {
            pages
                .iter()
                .filter_map(|(entry, document)| {
                    Some(tine_core::CorpusPage {
                        id: entry.rel_path.clone()?,
                        name: entry.name.clone(),
                        kind: entry.kind,
                        document: Arc::clone(document),
                    })
                })
                .collect()
        });
        tine_core::Corpus { pages }
    }

    /// Asset names mentioned in page preambles or blocks, including decoded URL
    /// spellings and the first segment of nested image references. Cost
    /// O(P + B + disk) on the first live-cache build, O(B) thereafter; the
    /// interim view rescans the loaded blocks on every call.
    pub fn referenced_assets(&self) -> Arc<HashSet<String>> {
        let mut names = HashSet::new();
        self.graph.with_pages(|pages| {
            for (_, doc) in pages {
                if let Some(pre) = &doc.pre_block {
                    crate::model::collect_asset_refs(pre, &mut names);
                }
                for block in &doc.roots {
                    crate::model::collect_block_asset_refs(block, &mut names);
                }
            }
        });
        Arc::new(names)
    }

    /// Names and file claimants for the graph. Cost O(P + aliases + referenced
    /// names); the interim graph may refresh its live directory index.
    pub fn inventory(&self) -> Arc<Inventory> {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        let mut claimed_names = HashSet::new();
        let mut page_claimed_names = HashSet::new();
        for page in self.graph.list_pages() {
            let key = tine_core::refs::page_key(&page.name);
            if !visited.insert((page.kind, key.clone())) {
                continue;
            }
            let mut by_name: HashMap<String, Vec<PageId>> = HashMap::new();
            for claimant in self.graph.find_claimants(&page.name, page.kind) {
                if let Some(id) = claimant.rel_path {
                    by_name.entry(claimant.name).or_default().push(id);
                }
            }
            for (name, mut ids) in by_name {
                let key = tine_core::refs::page_key(&name);
                if page.kind == PageKind::Page {
                    page_claimed_names.insert(key.clone());
                }
                claimed_names.insert(key);
                let id = ids.remove(0);
                entries.push(InventoryEntry {
                    name,
                    target: Resolved::Existing { id, others: ids },
                    is_journal: page.kind == PageKind::Journal,
                    day: page.date_key.map(Day),
                });
            }
        }
        let references = self.graph.referenced_page_names();
        let reference_spelling: HashMap<_, _> = references
            .iter()
            .map(|name| (tine_core::refs::page_key(name), name.as_str()))
            .collect();
        // One entry per alias name, owners sorted by path (rev 5
        // `Resolved::Alias`). An alias that is also a page file's name gets no
        // entry: `resolve` prefers the file, and every entry's target must be
        // the answer `resolve` gives for its name.
        let mut alias_owners: BTreeMap<String, (String, Vec<PageId>)> = BTreeMap::new();
        for (alias, _, owner) in self.graph.page_aliases_with_owners() {
            let key = tine_core::refs::page_key(&alias);
            if page_claimed_names.contains(&key) {
                continue;
            }
            let spelling = reference_spelling
                .get(&key)
                .copied()
                .unwrap_or(&alias)
                .to_owned();
            let slot = alias_owners
                .entry(key)
                .or_insert_with(|| (spelling, Vec::new()));
            let owner = PageId::from(owner);
            if !slot.1.contains(&owner) {
                slot.1.push(owner);
            }
        }
        let alias_names: HashSet<String> = alias_owners.keys().cloned().collect();
        for (_, (name, mut owners)) in alias_owners {
            owners.sort();
            entries.push(InventoryEntry {
                name,
                target: Resolved::Alias { owners },
                is_journal: false,
                day: None,
            });
        }
        for name in references {
            let key = tine_core::refs::page_key(&name);
            if claimed_names.contains(&key) || alias_names.contains(&key) {
                continue;
            }
            entries.push(InventoryEntry {
                target: self.resolve(&name, false),
                name,
                is_journal: false,
                day: None,
            });
        }
        entries.sort_by(|a, b| {
            tine_core::refs::page_key(&a.name)
                .cmp(&tine_core::refs::page_key(&b.name))
                .then_with(|| a.name.cmp(&b.name))
        });
        Arc::new(Inventory(entries))
    }

    /// Pages whose explicit references name any of `names` (page keys, compared
    /// after `refs::page_key`). Explicit means OG's `:block/refs`: page refs,
    /// tags, `tags::`-style properties and `{{embed}}`, but not the arguments of
    /// `{{query}}` or other macros, so a page that mentions a name only inside a
    /// query is not a referrer. The answer comes from this snapshot's parse, not
    /// from disk. Cost O(matching postings) with a warm reference index, else
    /// O(P + B) over the parsed snapshot.
    pub fn explicit_referrers(&self, names: &[String]) -> Vec<PageId> {
        let keys: Vec<String> = names
            .iter()
            .map(|name| tine_core::refs::page_key(name))
            .collect();
        let candidates = self
            .graph
            .reference_candidate_pages(&keys, tine_core::model::ReferenceKind::Explicit);
        let mut ids: Vec<PageId> = candidates
            .pages
            .iter()
            .filter(|(entry, doc)| {
                candidates.indexed
                    || crate::query::document_explicit_reference_names(entry, doc)
                        .iter()
                        .any(|name| keys.contains(name))
            })
            .map(|(entry, _)| PageId::from(self.graph.rel_path(&entry.path)))
            .collect();
        ids.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.dedup();
        ids
    }

    /// File modification time as observed when this graph view was acquired.
    /// Cost O(1), without a filesystem read.
    pub fn page_mtime(&self, id: &PageId) -> Option<std::time::SystemTime> {
        self.observed_mtimes.get(id.as_str()).copied()
    }

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
        let path = id.as_str();
        let valid_area = path.starts_with(&format!("{}/", self.graph.current_config().pages_dir))
            || path.starts_with(&format!("{}/", self.graph.current_config().journals_dir));
        let valid_name = !path.contains('\\')
            && !path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            && matches!(
                Path::new(path).extension().and_then(|ext| ext.to_str()),
                Some("md" | "org")
            )
            && Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| !tine_core::model::is_sync_conflict(stem));
        if !valid_area || !valid_name {
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

#[cfg(test)]
mod rev5_tests {
    use super::*;

    #[test]
    fn page_parser_panic_is_unparseable() {
        let root = std::env::temp_dir().join(format!(
            "tine-page-panic-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(
            root.join("pages/Panic.md"),
            "- __TINE_TEST_PAGE_PARSE_PANIC__\n",
        )
        .unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let result = store.page(&PageId::from("pages/Panic.md"));
        assert!(matches!(result, Err(StoreError::Unparseable(_))));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_reports_unstatable_entry_and_unlistable_directory() {
        let root = std::env::temp_dir().join(format!(
            "tine-scan-faults-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages/nested")).unwrap();
        fs::write(root.join("pages/Unreadable.md"), b"- page\n").unwrap();
        fs::write(root.join("pages/nested/Inside.md"), b"- page\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        SCAN_FAULTS.with(|faults| {
            *faults.borrow_mut() = (Some("Unreadable.md".into()), Some("nested".into()));
        });
        let listing = store.scan_area(Area::Pages, None).unwrap();
        SCAN_FAULTS.with(|faults| *faults.borrow_mut() = (None, None));
        assert_eq!(
            listing
                .unreadable
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["Unreadable.md", "nested"]
        );
        assert!(listing.files.is_empty());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
