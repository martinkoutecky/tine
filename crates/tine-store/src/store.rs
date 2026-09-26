//! Synchronous graph I/O and stable published graph views. `Store::open`
//! lists page and journal files before returning; parsing then runs in the
//! background. `Store::whole_graph` waits for that work and returns a view
//! whose answers remain stable across later changes. `GraphRev` identifies
//! the publication captured by that view. A new view is needed to see edits.
//!
//! One `Store::subscribe` consumer receives later publications in order.
//! A new subscription ends the previous one; the queue has no size bound.
//! Subscribe before acquiring a view, then ignore events at or below its
//! revision to avoid a missed update. Own writes and restore use
//! `Origin::Own`; file observation and explicit refresh use
//! `Origin::External`. `Origin::Own` does not identify a window or operation.
//!
//! `Store::save` and `Transaction::commit` guard against bytes already on
//! disk when they check. They serialize writers through the same Store, but
//! cannot exclude another process replacing a file between check and rename.
//! Changed writes publish before returning. A changed page write or read can
//! also scan metadata for all P page and journal files; the first publication
//! can wait for the graph-wide parse. Keep the caller's unsaved edits on a
//! refusal. Multi-file commit is not crash atomic.
//!
//! `Store::scan_refresh` compares file modification time and length, so a
//! same-length edit with unchanged timestamp may remain unseen. It waits for
//! the initial parse. Poll mode scans O(P) file metadata every three seconds;
//! notification mode silently falls back to polling if needed.
//! `Store::close` waits for an in-flight writer and ends observation and the
//! subscription. These calls have no general timeout. Run blocking calls off
//! a UI thread.

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
use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockPreview, BoundedRefGroups, PageDto,
    PageEntry, PageKind, RefGroup, TemplateDto,
};
pub use tine_core::model::{FileId, PageId};
use tine_core::query::{AdvancedResult, QueryExportBatch, QueryExportSpec};
use tine_core::query_plan::QueryExecution;

use crate::model::{CheckedOpenError, Graph, GraphRead, ReadSnapshot};

#[cfg(test)]
pub(crate) type TestPause = Arc<(Mutex<(bool, bool)>, Condvar)>;

#[cfg(test)]
pub(crate) fn pause_at_hook(hook: &Mutex<Option<TestPause>>) {
    let pause = hook.lock().unwrap().clone();
    if let Some(pause) = pause {
        let (state, ready) = &*pause;
        let mut state = state.lock().unwrap();
        state.0 = true;
        ready.notify_all();
        while !state.1 {
            state = ready.wait(state).unwrap();
        }
    }
}

pub(crate) const RESULT_BRIDGE_MAX_ROWS: usize = 20_000;
pub(crate) const RESULT_BRIDGE_MAX_BYTES: usize = 32 * 1024 * 1024;
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

/// Open graph root and guarded write access. Dropping the store calls
/// [`Self::close`], which can wait for a current writer.
/// Opening lists page and journal files and starts background parsing; see
/// [`Self::open`]. `Store` is `Send + Sync`; callers can share it through `Arc`.
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

/// Open-time external-asset approval and initial file observation mode.
#[derive(Default)]
pub struct OpenOptions {
    /// Canonical device path approved for an external `assets/` link, if any.
    pub approved_external_assets: Option<PathBuf>,
    /// Initial file observation mode.
    pub watch: WatchMode,
}

/// How the store observes graph files after opening.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WatchMode {
    /// Default: use filesystem notifications with a 200 ms debounce. A
    /// notified file is hashed even if its length and mtime match the previous
    /// observation. Notification failure silently falls back to three-second
    /// polling.
    #[default]
    Notify,
    /// Poll graph files every three seconds, scanning O(P) page metadata per
    /// tick and hashing files whose metadata changed. Same-length edits with
    /// preserved timestamps may be missed. There is no idle backoff.
    Poll,
}

/// Source of a published change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Bytes saved, written, or restored through this store. This does not
    /// identify the originating window or distinguish those operations.
    Own,
    /// Watcher or explicit scan reconciliation, including the initial
    /// load-completion publication with no file tuple.
    External,
}

/// Byte-level observation of a file across a publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// A file appeared.
    Created,
    /// File bytes changed.
    Modified,
    /// Metadata changed while the observed byte revision stayed equal; refresh
    /// displayed metadata. This does not prove no unobserved write occurred.
    Touched,
    /// A file disappeared.
    Removed,
}

/// One published graph change; subscriptions deliver generations in order.
/// `Change` is `Send + Sync` and can cross worker-thread boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Generation after this change.
    pub graph_rev: GraphRev,
    /// Whether this store or an external actor supplied the final bytes in
    /// this publication. Rollback can emit separate Own and External changes.
    pub origin: Origin,
    /// Affected graph files, including `logseq/config.edn` when observed, with
    /// resulting revisions when present. `config_changed` also marks config.
    /// Trash destinations are not listed. Assets are not watched for external
    /// changes. A committed transaction or restore lists an asset path here
    /// when its final bytes differ from the operation's starting bytes.
    /// Sync-conflict
    /// copies may appear as files while [`Self::page`] returns `None` for them;
    /// they are excluded from parsed search, backlinks, and page inventory.
    pub files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    /// Whether graph config changed in this publication.
    pub config_changed: bool,
    pages: Vec<(FileId, PageKind, String)>,
}

impl Change {
    /// The parsed graph page altered by an external observation or an external
    /// writer whose bytes survived transaction undo, using its graph filename
    /// name (the old name for a removal). A `title::` property does not replace
    /// that identity. Own writes
    /// list changed files but carry no parsed page entries, so this returns
    /// `None` for them. It also returns `None` for a file with no parsed page
    /// change, including a duplicate journal claimant or sync-conflict copy.
    /// A move lists its old file as removed and its destination as created;
    /// `page()` can describe either only when parsed page evidence is present.
    /// An own event's file id and revision
    /// describe disk state but cannot identify the originating window.
    /// Cost O(parsed page entries in this publication) per call; calling it
    /// for every file can be quadratic in a large publication.
    pub fn page(&self, file: &FileId) -> Option<(PageKind, &str)> {
        self.pages
            .iter()
            .find(|(id, _, _)| id == file)
            .map(|(_, kind, name)| (*kind, name.as_str()))
    }
}

/// A subscription ended because the store closed or another subscriber replaced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

pub(crate) struct ChangeFeed {
    state: Mutex<FeedState>,
    ready: Condvar,
    graph: Arc<Graph>,
    config: Arc<RwLock<ConfigState>>,
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    #[cfg(test)]
    pub(crate) snapshot_publish_pause: Mutex<Option<TestPause>>,
}

struct FeedState {
    rev: u64,
    subscription: u64,
    queue: VecDeque<Change>,
    closed: bool,
}

struct Snapshot {
    graph: Arc<ReadSnapshot>,
    rev: GraphRev,
    cache_generation: u64,
    config: ConfigState,
    journal_format: JournalFormat,
    list: Arc<Vec<PageEntry>>,
    claimants: Arc<HashMap<(PageKind, String), Vec<PageEntry>>>,
    observed_mtimes: Arc<HashMap<String, SystemTime>>,
    unreadable: Arc<Vec<(FileId, String)>>,
}

impl Snapshot {
    fn capture(
        graph: &Graph,
        config: &RwLock<ConfigState>,
        old: Option<&Snapshot>,
        files: &[(FileId, ChangeKind, Option<FileRev>)],
        config_changed: bool,
        rev: GraphRev,
    ) -> Self {
        // The publication caller holds the store writer lock. A load worker
        // publishes only after its initial parse has finished.
        graph.with_pages(|_| ());
        let config = config.read().unwrap().clone();
        let journal_format = graph.current_journal_format();
        let cache_generation = graph.cache_generation();
        let changed_names: Vec<_> = files
            .iter()
            .filter(|(id, kind, _)| {
                (id.as_str()
                    .starts_with(&format!("{}/", config.config.pages_dir))
                    || id
                        .as_str()
                        .starts_with(&format!("{}/", config.config.journals_dir)))
                    && matches!(kind, ChangeKind::Created | ChangeKind::Removed)
            })
            .collect();
        let name_set_changed = config_changed || old.is_none() || !changed_names.is_empty();
        let (list, claimants) = if config_changed || old.is_none() {
            let (list, claimants) = graph.snapshot_name_index();
            (list, Arc::new(claimants))
        } else if !changed_names.is_empty() {
            let previous = old.expect("name index from old generation");
            let mut list = Arc::clone(&previous.list);
            let mut claimants = Arc::clone(&previous.claimants);
            for (id, kind, _) in changed_names {
                let path = graph.root.join(id.as_str());
                let entry = graph.entry_for_path(&path);
                let Some(entry) = entry else { continue };
                let key = (entry.kind, tine_core::refs::page_key(&entry.name));
                let bucket = Arc::make_mut(&mut claimants).entry(key).or_default();
                bucket.retain(|candidate| candidate.path != path);
                if *kind == ChangeKind::Created {
                    bucket.push(entry.clone());
                }
                bucket.sort_by(|a, b| crate::model::compare_page_claimants(a, b, &journal_format));
                let list = Arc::make_mut(&mut list);
                list.retain(|candidate| candidate.path != path);
                if entry.kind == PageKind::Journal && entry.date_key.is_some() {
                    list.retain(|candidate| {
                        candidate.kind != PageKind::Journal || candidate.date_key != entry.date_key
                    });
                    if let Some(winner) = bucket.first() {
                        list.push(winner.clone());
                    }
                } else if *kind == ChangeKind::Created {
                    list.push(entry);
                }
            }
            (list, claimants)
        } else {
            let old = old.expect("name index from old generation");
            (Arc::clone(&old.list), Arc::clone(&old.claimants))
        };
        let changed_paths: Vec<String> = files
            .iter()
            .filter(|(id, _, _)| {
                id.as_str()
                    .starts_with(&format!("{}/", config.config.pages_dir))
                    || id
                        .as_str()
                        .starts_with(&format!("{}/", config.config.journals_dir))
            })
            .map(|(id, _, _)| id.as_str().to_owned())
            .collect();
        let evaluator = if let Some(old) =
            old.filter(|old| old.cache_generation == cache_generation && !config_changed)
        {
            Arc::clone(&old.graph)
        } else {
            let evaluator = ReadSnapshot::capture(
                graph,
                (*config.config).clone(),
                Arc::clone(&list),
                old.filter(|_| !config_changed)
                    .map(|old| old.graph.as_ref()),
                &changed_paths,
            );
            if !name_set_changed {
                if let Some(old) = old {
                    evaluator.carry_memos_from(&old.graph, &changed_paths);
                }
            }
            Arc::new(evaluator)
        };
        Self {
            graph: evaluator,
            rev,
            cache_generation,
            config,
            journal_format,
            list,
            claimants,
            observed_mtimes: graph.observed_page_mtimes(),
            unreadable: graph.unreadable_pages(),
        }
    }
}

impl ChangeFeed {
    fn new(graph: Arc<Graph>, config: Arc<RwLock<ConfigState>>) -> Self {
        Self {
            state: Mutex::new(FeedState {
                rev: 0,
                subscription: 0,
                queue: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
            graph,
            config,
            snapshot: RwLock::new(None),
            #[cfg(test)]
            snapshot_publish_pause: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn initialize(&self) {
        let snapshot = Snapshot::capture(&self.graph, &self.config, None, &[], false, GraphRev(0));
        *self.snapshot.write().unwrap() = Some(Arc::new(snapshot));
    }

    pub(crate) fn publish(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
    ) -> GraphRev {
        let old = self.snapshot.read().unwrap().clone();
        let rev = GraphRev(self.rev().0 + 1);
        let snapshot = Arc::new(Snapshot::capture(
            &self.graph,
            &self.config,
            old.as_deref(),
            &files,
            config_changed,
            rev,
        ));
        #[cfg(test)]
        pause_at_hook(&self.snapshot_publish_pause);
        let mut state = self.state.lock().unwrap();
        state.rev += 1;
        let rev = GraphRev(state.rev);
        debug_assert_eq!(snapshot.rev, rev);
        *self.snapshot.write().unwrap() = Some(snapshot);
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

/// Single active change stream for a store, without replay or a queue bound.
/// `Subscription` is `Send + Sync`, though receiving from multiple threads on
/// one subscription should be coordinated by its owner.
pub struct Subscription {
    feed: Arc<ChangeFeed>,
    number: u64,
}

impl Subscription {
    /// Wait without a timeout for the next change; returns [`Closed`] after
    /// close or replacement. A queued result is O(1) to dequeue.
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

    /// Take the next queued change without waiting, or `None` when none is ready.
    /// Returns [`Closed`] after close or replacement.
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
    /// Canonical graph root for display or OS handoff.
    pub root: PathBuf,
    /// Canonical external assets target, when present.
    pub external_assets: Option<PathBuf>,
}

impl GraphAccessInspection {
    /// Canonicalize `path` and compare it with the target captured by this
    /// inspection. Returns an I/O error if canonicalization fails. This does
    /// not reread a link changed since inspection; `Store::open` revalidates it.
    pub fn approves_external_assets(&self, path: &Path) -> std::io::Result<bool> {
        Ok(self.external_assets.as_ref() == Some(&fs::canonicalize(path)?))
    }
}

/// Failure to inspect, create, or open a graph root.
#[derive(Debug)]
pub enum OpenError {
    /// Requested root is not a directory.
    NotAFolder(PathBuf),
    /// Root or a required path could not be resolved.
    Unresolvable {
        /// Path that could not be resolved.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },
    /// Directory layout is unsafe for graph access.
    UnsafeLayout(String),
    /// External assets target requires device approval.
    ExternalAssetsUnapproved {
        /// Canonical external target awaiting approval.
        current: PathBuf,
    },
    /// Creating a new graph root failed.
    CreateFailed {
        /// Requested root.
        path: PathBuf,
        /// Underlying I/O failure.
        cause: crate::IoError,
    },
    /// Other filesystem failure.
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

/// Effective config. File operations remain available when `problem` is set;
/// callers should resolve the read failure before creating files under
/// possibly defaulted directories or journal formats. `scan_refresh()` retries
/// the config read and updates `problem` after the underlying error is fixed.
#[derive(Clone)]
pub struct ConfigState {
    /// Effective graph config, defaulted when loading config failed. A changed
    /// journal title format affects names and date claims in new views after
    /// publication; existing file bytes and reference text are not rewritten.
    pub config: Arc<tine_core::config::Config>,
    /// Config read error, if one occurred. Missing config is not an error.
    /// Unrecognized or malformed config values may default individually and do
    /// not set this field. Journal format strings are not validated here:
    /// unsupported format characters become literal text.
    pub problem: Option<crate::IoError>,
}

impl std::ops::Deref for ConfigState {
    type Target = tine_core::config::Config;
    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

/// Trash categories. Typed directories identify new trash entries; loose old
/// entries are classified from their filename when recognizable. `Legacy`
/// covers the remaining entries with no recognized recoverable type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrashKind {
    /// Trashed asset.
    Asset,
    /// Trashed page.
    Page,
    /// Trashed journal.
    Journal,
    /// Conflict-area entry, including sync-conflict copies, retired publish
    /// sites, and transaction undo staging copies.
    Conflict,
    /// Entry without a recognized typed trash category.
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

fn remove_trash_entry_counted(
    path: &Path,
    removed_bytes: &mut u64,
    remove_file: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for child in fs::read_dir(path)? {
            remove_trash_entry_counted(&child?.path(), removed_bytes, remove_file)?;
        }
        fs::remove_dir(path)
    } else {
        remove_file(path)?;
        if metadata.is_file() {
            *removed_bytes = removed_bytes.saturating_add(metadata.len());
        }
        Ok(())
    }
}

pub(crate) fn journal_ids_from_entries(
    graph: &Graph,
    entries: &[PageEntry],
) -> HashMap<Day, PageId> {
    let mut claimants: HashMap<Day, Vec<PageEntry>> = HashMap::new();
    for entry in entries {
        if entry.kind == PageKind::Journal {
            if let Some(day) = entry.date_key {
                claimants.entry(Day(day)).or_default().push(entry.clone());
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
        let journal_ids = Arc::new(Mutex::new(journal_ids_from_entries(
            &graph,
            graph.list_pages_shared().as_ref(),
        )));
        let config_state = Arc::new(RwLock::new(ConfigState {
            config: Arc::new(graph.config.clone()),
            problem: None,
        }));
        let changes = Arc::new(ChangeFeed::new(
            Arc::clone(&graph),
            Arc::clone(&config_state),
        ));
        changes.initialize();
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

    /// Scaffold a graph in an empty parent, or in the first unused `tine-demo`
    /// child when the parent is nonempty. Use the returned path as the actual
    /// graph root. Cost O(siblings probed + seed bytes). Partial failures leave
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
    /// Cost is a filesystem canonicalization and directory metadata check.
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
    /// Cost is a small fixed number of filesystem path and metadata checks.
    pub fn inspect(root: &Path) -> Result<GraphAccessInspection, OpenError> {
        let canonical = Self::canonical_root(root)?;
        let external_assets = Graph::external_assets_target(&canonical)
            .map_err(|error| OpenError::Io(error.into()))?;
        Ok(GraphAccessInspection {
            root: canonical,
            external_assets,
        })
    }

    /// Open a graph after validating its layout and any external assets target.
    /// Returns the store, graph metadata, and effective config. Lists pages and
    /// journals, including their journal-day identities, before returning
    /// (O(P) file metadata). An unreadable page/journal subtree is omitted
    /// from the file-list index and later reported as unreadable; an exact
    /// destination guard cannot detect every unseen same-name claimant.
    /// A caller that applies the configured journal template must wait for
    /// `WholeGraph::templates()`; saving a new journal does not add it.
    /// Parsing runs in the background and
    /// [`Self::whole_graph`] waits for it. A write during parsing is included
    /// in the first published view. One unreadable page can be skipped and
    /// later reported by `unreadable_files`; a failed entire parse makes
    /// graph-wide queries unavailable until [`Self::scan_refresh`] retries it.
    /// Direct page reads and guarded writes remain available after a parse
    /// failure. Neither publishes a graph generation until a successful
    /// `scan_refresh()`. The returned
    /// `GraphMeta` is a snapshot of open-time settings. After a config change,
    /// callers can derive fresh display metadata with
    /// `GraphMeta::from_config` and `JournalFormat::new` from `Store::config()`;
    /// reopening also refreshes it but restarts this store's revision sequence.
    /// External observations during loading publish after the initial parse.
    /// On failed initial load, the watcher also defers external publication
    /// until `scan_refresh()` successfully retries. Recovery's completion
    /// publication has no file tuples; reconciliation may publish observed
    /// differences separately. Use the recovered view to refresh graph-wide answers.
    /// An unsafe layout, unapproved external target, or I/O failure
    /// returns [`OpenError`].
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
        let journal_ids = journal_ids_from_entries(&graph, graph.list_pages_shared().as_ref());
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
        let journal_ids = Arc::new(Mutex::new(journal_ids));
        let config_state = Arc::new(RwLock::new(config.clone()));
        let changes = Arc::new(ChangeFeed::new(
            Arc::clone(&graph),
            Arc::clone(&config_state),
        ));
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

    /// Current graph configuration. Does not wait for the graph parse, but may
    /// briefly wait for a concurrent configuration update.
    pub fn config(&self) -> ConfigState {
        self.config_state.read().unwrap().clone()
    }

    /// Stop observation, wait for an in-flight writer, end the subscription,
    /// release load waiters, and refuse later I/O. Idempotent; no timeout.
    pub fn close(&self) {
        self.watch.stop();
        let _writer = self.writer.lock().unwrap();
        self.load.closed.store(true, Ordering::Release);
        self.load.cancelled.store(true, Ordering::Release);
        *self.load.status.lock().unwrap() = LoadStatus::Closed;
        self.load.ready.notify_all();
        self.changes.close();
    }

    /// Start the sole app-wide change stream at the next publication. Replaces
    /// the previous subscriber and discards queued changes. The queue is
    /// unbounded; consumers must drain it. To avoid a subscription gap,
    /// subscribe first, then acquire `whole_graph()` and ignore changes with
    /// `graph_rev <= view.rev()`. Initial load completion is an
    /// `Origin::External` publication with no file tuples; a during-load save
    /// has a separate `Origin::Own` publication once a complete snapshot is
    /// available. Writer serialization orders these publications; whichever
    /// comes first has a view containing the save.
    /// Failed load is observed by calling `whole_graph()`, not as a `Change`;
    /// no page read or write publishes while it remains failed.
    /// Multi-window clients must fan this single stream out themselves. A slow
    /// consumer can retain an unbounded number of queued changes in memory;
    /// each change also holds its file tuples and parsed external page names.
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

    /// Change observation mode without waiting for a load; has no effect after
    /// close. The return value does not report whether Notify fell back to Poll.
    pub fn set_watch_mode(&self, mode: WatchMode) {
        if !self.is_closed() {
            self.watch.set_mode(mode);
        }
    }

    /// Wait for the initial graph parse, retrying it if it previously failed,
    /// then reconcile page, journal and
    /// config files. Assets are not scanned. A file is considered unchanged
    /// when its modification time and length both match the previous scan;
    /// same-length edits with preserved timestamps can therefore be missed.
    /// Cost O(P metadata + bytes of files detected as changed + config bytes
    /// hashed), plus the initial load wait. Recovery from a failed initial load
    /// can also parse the whole graph synchronously before reconciliation.
    /// A previously unreadable subtree is retried on this full scan; newly
    /// accessible files can then enter the resulting view.
    /// After a failed initial load, a successful retry
    /// publishes a fresh completion generation with no file tuples even if no file changed during
    /// reconciliation or an older snapshot exists. Returns `LoadError::Closed` after close or
    /// `LoadError::Failed` for a lost root or unsafe config layout.
    pub fn scan_refresh(&self) -> Result<(), LoadError> {
        self.watch.scan_refresh()
    }

    pub(crate) fn publish_own(
        &self,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    ) -> GraphRev {
        self.publish_transaction_change(Origin::Own, files, Vec::new())
    }

    pub(crate) fn publish_transaction_change(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        pages: Vec<(FileId, PageKind, String)>,
    ) -> GraphRev {
        let ids: Vec<FileId> = files.iter().map(|(id, _, _)| id.clone()).collect();
        self.watch.note_own(&ids);
        let config_changed = ids.iter().any(|id| id.as_str() == "logseq/config.edn");
        self.refresh_journal_ids();
        if matches!(*self.load.status.lock().unwrap(), LoadStatus::Failed(_)) {
            return self.changes.rev();
        }
        self.changes.publish(origin, files, config_changed, pages)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.load.closed.load(Ordering::Acquire)
    }

    pub(crate) fn refresh_journal_ids(&self) {
        let found = journal_ids_from_entries(&self.graph, self.graph.list_pages_shared().as_ref());
        *self.journal_ids.lock().unwrap() = found;
    }

    /// Resolve the canonical journal file for a valid day, or a proposed new
    /// file. The day index is built from the accessible file listing before `open`
    /// returns; no parse or disk read is needed here. An unreadable journal
    /// subtree is omitted, so this may propose a second file for an existing
    /// day in that subtree. This does not indicate
    /// existence: call `page(id)` and handle `NotFound`. An invalid `Day`
    /// is not rejected and can yield a nonsensical proposed name. For a custom
    /// journal filename format, the proposal uses that configured format and
    /// the current `preferred_format` extension (`md` or `org`). An
    /// unobserved external creation can change the answer later; callers must
    /// use a guarded `CreateNew` save and handle a conflict or twin.
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
    /// Save one page with a raw-byte [`SaveBase`] guard. Revalidates the
    /// caller-constructible identity, reads current disk bytes, and uses
    /// temporary-file replacement; the temp file is synced before rename and
    /// directory sync is best effort. A power loss after return can therefore
    /// still lose the new directory entry on a filesystem that did not sync
    /// the directory. A stale
    /// base returns `Conflict` even if the proposed bytes equal current disk
    /// bytes. With a matching base, equal bytes return `Unchanged` without a
    /// publication. A changed save publishes
    /// before returning as its own `Origin::Own` change when the initial load
    /// has not failed. A save begun during parsing may wait for the full parse
    /// while capturing its publication view; it can finish before or after
    /// the separate load-completion event. The save's own generation contains
    /// its write, and a later load-completion view contains it too. `Saved`
    /// returns a file revision, not a graph revision; compare the matching
    /// `Origin::Own` change with a newly acquired view when needed. After a failed
    /// initial load it still writes on a matching guard, but publishes no
    /// generation until a successful `scan_refresh()`. Cost includes reading and hashing the page, writing
    /// its new bytes, and O(P) metadata for graph-wide publication; it can
    /// wait for graph snapshot capture and other writers without a timeout.
    /// Missing target parent directories are created during apply. A
    /// separate process can still write between the final guard check and
    /// rename. Serialization and temp-file sync precede that final check.
    /// `doc.rev` does not replace `base`; `doc.format`, `doc.name`, and
    /// `doc.title` do not override the target file identity or extension. Only
    /// `doc.pre_block` and the block tree's `raw` and children become page
    /// text; `doc.name`, `kind`, `title`, `format`, and derived block facets do
    /// not inject page properties or select the serializer. The target
    /// extension selects Markdown or Org.
    /// Serialize saves for one editor page, passing each returned `Saved(rev)`
    /// as the next `SaveBase::Existing`; overlapping saves from the same base
    /// can conflict with each other. A concurrent external write overwritten
    /// in the remaining check-to-rename window may never appear as a separate
    /// `Change`.
    /// `CreateNew` checks the exact destination and alternate extension on
    /// disk; other same-name claims use the file-list index built before
    /// `open` returns and updated by later observations, even before parsing
    /// or after a failed parse. A newly
    /// delivered, unobserved journal twin can still be missed. `CreateNew` on
    /// an existing target returns `Conflict` with its disk
    /// revision, unless another file claims its page name or journal day,
    /// which returns `Twin`. Re-read with `page(id)` for parsed content or
    /// `read(id.file(), None)` for raw bytes; both revisions hash the same
    /// bytes as `Conflict::disk` if the file has not changed again. That revision is a technically valid new
    /// base, but inspect the current bytes before choosing to overwrite. A
    /// guard conflict does not publish an external change. This call does not preserve a separate
    /// conflict copy of bytes it replaces. A caller choosing "keep mine"
    /// must preserve the other bytes separately if they are needed. Its own observed write is not
    /// republished as an external watcher echo. Keep unsaved edits on every
    /// refusal.
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

    /// Count entries and bytes by kind in graph trash. `scan_area(Area::Trash)`
    /// can list files and `move_file` can move one to a live area with a guard,
    /// but there is no dedicated untrash workflow or recovery-root import.
    /// Only asset trash has an in-API purge. Cost O(trash entries and
    /// files inside trashed directories).
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
                        let bytes =
                            trash_entry_bytes(&child.path()).map_err(StoreError::from_io)?;
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

    /// Permanently remove typed asset entries and loose legacy entries whose
    /// filenames classify as assets,
    /// including directories. Legacy pages and other kinds stay recoverable.
    /// On error, returns completed top-level entry count and all bytes already
    /// deleted, including bytes removed from a directory that was only partly
    /// purged. A partly purged directory is not counted as a completed entry.
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
                    remove_trash_entry_counted(&asset.path(), &mut removed.1, &mut |path| {
                        fs::remove_file(path)
                    })
                    .map_err(|error| (StoreError::from_io(error), removed.0, removed.1))?;
                    removed.0 += 1;
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

    /// Type a slash-separated file name within one configured graph area.
    /// Rejects traversal or unsafe identities; validation repeats on use.
    /// Does not require the file to exist or wait for the initial load. Cost is
    /// proportional to the path length; no content read or graph parse.
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

    /// Type a valid `.md` or `.org` file in pages or journals as a page id.
    /// Syncthing `.sync-conflict-` and Dropbox `(conflicted copy)` names,
    /// and invalid ids, return `None`; no disk read or wait.
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

    /// A validated OS path. `existing_regular_file` requires a live file for an
    /// opener; a page source may follow an older graph layout's in-graph link
    /// between its pages and journals directories.
    /// Otherwise a missing final file is allowed, with ancestors inside its area.
    /// Cost O(path components), independent of graph size. Refuses an escaped
    /// target; missing or unreadable existing files return their I/O error.
    pub fn path_for_os_handoff(
        &self,
        file: &FileId,
        existing_regular_file: bool,
    ) -> Result<PathBuf, StoreError> {
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
        if existing_regular_file {
            let target = fs::canonicalize(&candidate).map_err(StoreError::from_io)?;
            if !target.is_file() {
                return Err(StoreError::InvalidTarget(
                    if file.as_str().starts_with("assets/") {
                        file.as_str().to_owned()
                    } else {
                        "page source is not a file".into()
                    },
                ));
            }
            if file.as_str().starts_with("assets/") {
                let assets =
                    fs::canonicalize(self.graph.assets_path()).map_err(StoreError::from_io)?;
                if !target.starts_with(&assets) {
                    return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
                }
            } else {
                if self.as_page(file).is_none() {
                    return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
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
            }
            return Ok(target);
        }
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

    /// Display the recoverable asset trash location in a user-facing error.
    pub fn asset_trash_location_for_user(&self) -> PathBuf {
        self.graph.root.join("logseq/.tine-trash/assets")
    }

    /// Read one file's bytes and its raw-byte revision without updating the
    /// graph or publishing a change, with an optional limit
    /// checked before and after reading. A final symlink can be followed only
    /// when its resolved target remains inside the approved area; `open_read`
    /// refuses final symlinks. If metadata already exceeds the limit, no
    /// content is read; a growing file can exceed it after a full read. A
    /// `TooLarge` result has no revision; use `open_read` for bounded streaming.
    /// Cost O(file bytes).
    pub fn read(
        &self,
        file: &FileId,
        max_bytes: Option<u64>,
    ) -> Result<(Vec<u8>, FileRev), StoreError> {
        let path = self.path_for_os_handoff(file, false)?;
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
        let path = self.path_for_os_handoff(file, false)?;
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
    /// Hidden entries and symlinks encountered while walking are skipped. An
    /// explicit `under` may traverse an in-area symlink in an ancestor. For
    /// `Area::Meta`, only `config.edn` and `custom.css` are included;
    /// other visible metadata is omitted, including from `unreadable`.
    /// Stat/list failures for included entries appear in `unreadable`.
    /// `None` lists the whole area; a supplied path that does not exist gives
    /// an empty listing. This call does not wait for the initial parse and is
    /// available after a parse failure. Cost O(entries).
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
            self.path_for_os_handoff(&dir, false)?
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

    /// Read and parse one page, returning `NotFound` if it is absent. A
    /// newly observed external edit publishes `Origin::External` before
    /// returning; later observation of the same bytes does not duplicate it.
    /// A missing file returns `NotFound` without waiting for the initial parse.
    /// A file under an unreadable directory is read directly when its path is
    /// accessible; the returned error reflects that read or path validation.
    /// An observed edit can wait for that parse while publishing. After a
    /// failed initial parse, a present file still returns its current parsed
    /// content, but publishes no generation until successful `scan_refresh()`.
    /// Cost O(page
    /// bytes + blocks), plus O(P) metadata if publication occurs.
    pub fn page(&self, id: &PageId) -> Result<PageRead, StoreError> {
        let _writer = self.writer.lock().unwrap();
        let before_generation = self.graph.cache_generation();
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
        self.path_for_os_handoff(&id.file(), false)?;
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
        if self.graph.cache_generation() != before_generation
            && !matches!(*self.load.status.lock().unwrap(), LoadStatus::Failed(_))
        {
            self.watch.note_own(&[id.file()]);
            self.changes.publish(
                Origin::External,
                vec![(id.file(), ChangeKind::Modified, Some(rev.clone()))],
                false,
                vec![(id.file(), entry.kind, entry.name)],
            );
        }
        Ok(PageRead {
            id: id.clone(),
            doc,
            rev,
            read_only,
        })
    }

    /// Wait without a timeout for the initial parse (or close), then acquire
    /// the current stable publication. Acquisition and clone are O(1) after
    /// the wait; later writes do not alter this view. After a failed parse,
    /// later calls return `LoadError::Failed` without another parse attempt
    /// until `scan_refresh()` retries it. There is no nonblocking load-state
    /// query; call this from a worker thread to learn completion or failure.
    /// No partial graph generation is available after failure.
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
        let snapshot = self
            .changes
            .snapshot
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .expect("ready store has a graph snapshot");
        Ok(WholeGraph {
            graph: Arc::clone(&snapshot.graph),
            rev: snapshot.rev,
            observed_mtimes: Arc::clone(&snapshot.observed_mtimes),
            unreadable: Arc::clone(&snapshot.unreadable),
            config: snapshot.config.clone(),
            journal_format: snapshot.journal_format.clone(),
            list: Arc::clone(&snapshot.list),
            claimants: Arc::clone(&snapshot.claimants),
            _snapshot: snapshot,
        })
    }
}

/// Graph area used to form area-relative file identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    /// Configured pages directory.
    Pages,
    /// Configured journals directory.
    Journals,
    /// Assets directory, possibly an approved external target. Guarded reads
    /// and writes use that approved target; a cross-filesystem move into graph
    /// trash can fail rather than silently copying and deleting.
    Assets,
    /// `logseq/` metadata directory; names starting with `.tine-` at its root
    /// are refused by `file_id`.
    Meta,
    /// Graph-local `logseq/.tine-trash` area.
    Trash,
}

/// One successfully listed and statted file. `rel` is the exact name within
/// its area. Cost O(1) to inspect.
pub struct FileEntry {
    /// Opaque identity of this file.
    pub id: FileId,
    /// Area containing the file.
    pub area: Area,
    /// Exact slash-separated name within the area.
    pub rel: String,
    /// Page identity for page or journal text, if applicable.
    pub page: Option<PageId>,
    /// Parsed journal day under configured and fallback formats; cost O(1).
    pub day: Option<Day>,
    /// Whether the stem is the configured filename form; cost O(1).
    pub date_stem: bool,
    /// Metadata for a successfully statted entry. Stat failures appear in
    /// `Listing::unreadable` instead.
    pub meta: Option<FileMeta>,
}

/// Metadata observed during a scan. Modification time may be unavailable.
/// Cost O(1) to inspect.
pub struct FileMeta {
    /// Observed byte length.
    pub len: u64,
    /// Observed modification time, when available.
    pub mtime: Option<SystemTime>,
}

/// Files found by `scan_area`; unreadable entries carry I/O errors and an
/// area-relative name when one could be identified. Cost O(files + unreadable
/// entries) to inspect.
#[derive(Default)]
pub struct Listing {
    /// Files successfully listed in this area.
    pub files: Vec<FileEntry>,
    /// Area-relative names that could not be read, with their errors. An empty
    /// name means directory iteration failed before an entry name was available.
    pub unreadable: Vec<(String, crate::IoError)>,
}

#[cfg(test)]
thread_local! {
    static SCAN_FAULTS: std::cell::RefCell<(Option<String>, Option<String>)> =
        const { std::cell::RefCell::new((None, None)) };
}

/// Journal day encoded as a `yyyymmdd` integer. Construct it from
/// `JournalDate::ordinal_key()` and recover the date with
/// `JournalDate::from_ordinal(day.0)`. The public constructor does not validate
/// dates; pass a real calendar day to `Store::journal_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Day(pub i64);

/// Failure to identify or read a graph file.
#[derive(Debug)]
pub enum StoreError {
    /// File is absent.
    NotFound,
    /// File id or path is unsafe or outside its area.
    InvalidTarget(String),
    /// File bytes cannot be decoded as UTF-8.
    Undecodable,
    /// Page text could not be parsed; message describes the failure.
    Unparseable(String),
    /// A bounded read exceeded the caller's byte limit.
    TooLarge {
        /// Requested maximum length.
        limit: u64,
        /// Observed file length.
        len: u64,
    },
    /// Other filesystem failure.
    Io(std::io::Error),
    /// Store was closed before this disk operation.
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

/// Opaque FNV-1a/64 raw-byte revision. Constructing one from a string does not
/// validate it; a value unlike the current disk hash conflicts, while an
/// arbitrary value that happens to equal that hash passes. Guarded writes
/// compare it with a newly computed hash in
/// O(file bytes). This is a conflict marker, not a cryptographic digest.
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
    /// Replace only if the file still has this raw-byte revision.
    Existing(FileRev),
    /// Create only if the file is absent.
    CreateNew,
}

/// Result of one guarded page save. A refusal never authorizes dropping edits.
#[derive(Debug)]
pub enum SaveOutcome {
    /// New page bytes were written; contains their revision.
    Saved(FileRev),
    /// New bytes equal current disk bytes with a matching base; no write or
    /// publication occurred.
    Unchanged(FileRev),
    /// The base no longer authorizes this write; read the current file before
    /// resolving. The proposed bytes can equal those already on disk.
    Conflict {
        /// Revision of the current disk bytes.
        disk: FileRev,
    },
    /// Existing base was requested, but the file disappeared. Retrying as
    /// `CreateNew` would recreate a page that another device may have deleted.
    Deleted,
    /// Existing page cannot be safely rewritten; currently this is the Org
    /// round-trip editability refusal. The reason is for display.
    ReadOnly(String),
    /// Another file claims the page name or journal day. Creation and moves
    /// check this; an ordinary guarded save to either existing claimant is
    /// allowed when its own revision guard and safety checks pass.
    Twin {
        /// Existing claimant.
        existing: PageId,
    },
    /// Invalid or unsafe target; reason is for display.
    InvalidTarget(String),
    /// Filesystem operation failed.
    Io(std::io::Error),
    /// Store was closed before the save.
    Closed,
    /// A `PageDto` with `guide: true` is refused before disk access, regardless
    /// of the supplied page id.
    GuideEphemeral,
}

/// Parsed page and the raw-byte revision used for a guarded save.
pub struct PageRead {
    /// Same page identity passed to [`Store::page`]; no canonicalization occurs.
    pub id: PageId,
    /// Parsed document.
    pub doc: PageDto,
    /// Revision of the bytes that produced `doc`.
    pub rev: FileRev,
    /// Reason a file cannot round-trip safely, if any. Reflects the parsed
    /// `doc.read_only` flag; a twin claim alone does not set it. Save checks
    /// disk safety again rather than trusting either caller-supplied flag.
    pub read_only: Option<String>,
}

/// Result of resolving a page name or journal day in one snapshot.
pub enum Resolved {
    /// One or more files claim the name. The canonical file comes first;
    /// removing it can reveal another claimant with different content. A
    /// subscriber should re-resolve this name after a claimant is removed.
    /// For an `Origin::Own` removal, remember the name from the earlier view:
    /// `Change::page` has no parsed name for that event.
    Existing {
        /// Canonical claimant.
        id: PageId,
        /// Other claimants.
        others: Vec<PageId>,
    },
    /// Name belongs to pages that declare it as an alias. Navigation can choose
    /// an owner; saving requires an actual owner's `PageId`, not the alias.
    Alias {
        /// Alias owners in deterministic file order; no owner is chosen for you.
        owners: Vec<PageId>,
    },
    /// No claimant; this is the id a new page would use.
    Absent {
        /// Proposed file identity.
        id: PageId,
    },
}
/// Physical names, aliases, and names that occur only in references, sorted by
/// the graph's page identity key. A physical page has one entry per decoded
/// spelling, and each entry's target is exactly what [`WholeGraph::resolve`]
/// returns for that name: the canonical claimant, then every other file
/// claiming the same normalized name.
pub struct InventoryEntry {
    /// Decoded file name, alias, or referenced name.
    pub name: String,
    /// Current claimant or proposed identity.
    pub target: Resolved,
    /// Whether this entry represents a journal.
    pub is_journal: bool,
    /// Parsed journal day, if any.
    pub day: Option<Day>,
}

/// Graph inventory entries in page-key order.
pub struct Inventory(pub Vec<InventoryEntry>);
/// Inputs for the query-plan graph search.
pub struct SearchRequest {
    /// Search expression.
    pub text: String,
    /// Restrict search to this physical page id, including a chosen twin; an
    /// alias does not broaden the scope. An absent file produces an empty
    /// result; check existence separately if it matters.
    pub within: Option<PageId>,
    /// Maximum page hits requested.
    pub page_limit: usize,
    /// Maximum block hits requested.
    pub block_limit: usize,
    /// Include an explanation of query planning.
    pub explain: bool,
}
/// Syntax used to evaluate a `{{query}}` expression.
pub enum QueryDialect {
    /// Simple query expression.
    Simple,
    /// Advanced query expression.
    Advanced,
}
/// Answer shape matching the requested query dialect.
pub enum QueryResult {
    /// Simple query reference groups.
    Simple(Arc<Vec<RefGroup>>),
    /// Advanced query result with its diagnostics.
    Advanced(AdvancedResult),
}

/// Sequence number of a published stable view, ordered within one Store.
/// A view with `rev() >= change.graph_rev` includes that publication. This
/// is not a file-write guard. It restarts for each Store opening; do not compare
/// values from separate Store instances. Serialized as a decimal string.
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

/// Failure to acquire or refresh a graph view.
#[derive(Debug)]
pub enum LoadError {
    /// Initial parse or explicit refresh could not continue. Individual
    /// unreadable page files and page/journal subdirectories, including a
    /// top-level page or journal directory, are skipped and reported by
    /// `WholeGraph::unreadable_files`; a lost root or unsafe config layout
    /// can fail the operation. After an initial failure, `page()` can still
    /// read a present file and guarded saves can write, but neither publishes
    /// a graph generation until successful `scan_refresh()` recovery.
    Failed {
        /// Human-readable failure reason.
        reason: String,
    },
    /// Store closed while the caller waited.
    Closed,
}

/// A whole-graph request failed before returning a partial answer.
#[derive(Debug)]
pub enum QueryError {
    /// Page identity is invalid for this graph.
    InvalidTarget(String),
    /// Request exceeds the store's fixed input budget.
    RequestTooLarge {
        /// Input budget exceeded.
        what: Budget,
        /// Requested item count.
        count: usize,
        /// Maximum accepted count.
        limit: usize,
    },
    /// Export request exceeds the macro count or combined key-and-query byte budget.
    ExportRequestTooLarge {
        /// Requested macro count.
        macros: usize,
        /// Sum of caller keys and query sources in bytes.
        bytes: usize,
        /// Maximum macro count.
        macro_limit: usize,
        /// Maximum combined key-and-query bytes.
        byte_limit: usize,
        /// Processing cap applied to this export request.
        processing_cap: usize,
    },
    /// Evaluation reached the store's fixed result budget.
    ResultTooLarge {
        /// Result budget exceeded.
        what: Budget,
        /// Result item count.
        count: usize,
        /// Maximum item count.
        limit: usize,
        /// Estimated serialized bytes, when measured.
        bytes: Option<usize>,
        /// Maximum estimated result bytes in the same accounting unit as
        /// `bytes`; some results include per-block and per-group overhead,
        /// so this need not equal serialized length.
        byte_limit: usize,
    },
    /// Query source exceeds its byte or nesting limit, or has invalid syntax
    /// in an evaluator that reports parse errors.
    Parse(String),
    /// Caller set the cancellation flag; no partial answer is returned.
    Cancelled,
}

/// Named input or result limit used by [`QueryError`].
#[derive(Clone, Copy, Debug)]
pub enum Budget {
    /// Backlink filter roots.
    BacklinkFilterRoots,
    /// Matching block rows.
    MatchingBlocks,
    /// Matching block rows in a serialized response.
    BridgeMatchingBlocks,
    /// Requested block-reference ids.
    RequestedBlockRefs,
    /// Resolved block rows.
    ResolvedBlockRows,
    /// Exported query bytes.
    ExportBytes,
    /// Property facet entries.
    PropertyFacets,
    /// Advanced query matches.
    AdvancedQueryMatches,
    /// Search hits in a serialized response.
    SearchHits,
}

impl QueryError {
    /// Check a final serialized response estimate after assembling groups.
    /// API adapters that add transport fields should call this before sending
    /// the response; ordinary `WholeGraph` callers already receive bounded
    /// results from the query methods.
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

    /// Check a final serialized search response estimate after adapter fields
    /// are known. Call this in a transport adapter, not for a direct
    /// `WholeGraph::search` result.
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

/// Caller-owned cancellation flag, checked by `search` and `find_blocks`
/// before each search page and block. Other graph queries do not take it.
pub struct Cancel(pub Arc<AtomicBool>);

/// Facet answer policy: reject oversized results or return a bounded prefix.
/// Both can cost O(B) over the view's blocks.
pub enum FacetPolicy {
    /// Reject a result that exceeds the fixed facet budget.
    Budgeted,
    /// Return a prefix within that budget.
    Truncated,
}

/// One stable published graph view. Clone is O(1); answers use captured
/// parsed pages and indexes without rereading files or later publications.
/// Holding old views retains graph snapshots, including parsed page bodies and
/// indexes proportional to that publication's graph. There is no view-count
/// cap; release old views when their answers are no longer needed.
/// `WholeGraph` is `Send + Sync` and can be shared among readers.
#[derive(Clone)]
pub struct WholeGraph {
    _snapshot: Arc<Snapshot>,
    pub(crate) graph: Arc<ReadSnapshot>,
    rev: GraphRev,
    observed_mtimes: Arc<HashMap<String, SystemTime>>,
    unreadable: Arc<Vec<(FileId, String)>>,
    pub(crate) config: ConfigState,
    journal_format: JournalFormat,
    pub(crate) list: Arc<Vec<PageEntry>>,
    claimants: Arc<HashMap<(PageKind, String), Vec<PageEntry>>>,
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
    #[cfg(test)]
    pub(crate) fn test_read_snapshot(&self) -> Arc<ReadSnapshot> {
        Arc::clone(&self.graph)
    }
    /// Page files and subdirectories skipped during the parse captured by this
    /// view, with a displayable reason. Their content is absent from graph-wide
    /// answers. Cost O(1).
    pub fn unreadable_files(&self) -> &[(FileId, String)] {
        &self.unreadable
    }

    /// Return an owned `Corpus` of pages in this view for evaluation. Cost
    /// O(P); parsed documents are shared while the result is held.
    // The evaluator receives a copy of the parsed-page table.
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
    /// O(P + B + scanned text bytes) on every call. This does not infer a
    /// sidecar reference merely from its PDF base name.
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

    /// Names and file claimants in this view. Physical twins with the same
    /// decoded spelling produce one name entry whose target contains the
    /// canonical id and the other claimants; `scan_area` lists physical files.
    /// Ordinary page twins can both contribute parsed search and backlink
    /// content; duplicate-day journal strays are absent from view queries but
    /// can be read directly by file id. Such a read does not add them to this
    /// view or publish a change; they do not contribute references or backlinks.
    /// They still appear as `others` of the day's `Resolved::Existing` target.
    // The parsed whole-graph cache omits duplicate-day journal strays.
    /// A first call can traverse all
    /// page blocks and reference text to build indexes; later calls cost
    /// O(P + aliases + referenced names). The result is stable in this view.
    pub fn inventory(&self) -> Arc<Inventory> {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        let mut claimed_names = HashSet::new();
        let mut page_claimed_names = HashSet::new();
        for page in self.list.iter() {
            let key = tine_core::refs::page_key(&page.name);
            if !visited.insert((page.kind, key.clone())) {
                continue;
            }
            // One entry per spelling, each with the same target `resolve`
            // gives: the bucket's first claimant, then every other claimant.
            let mut names: Vec<String> = Vec::new();
            let mut ids: Vec<PageId> = Vec::new();
            if let Some(claimants) = self.claimants.get(&(page.kind, key.clone())) {
                for claimant in claimants {
                    if let Some(id) = &claimant.rel_path {
                        if !names.contains(&claimant.name) {
                            names.push(claimant.name.clone());
                        }
                        ids.push(id.clone());
                    }
                }
            }
            for name in names {
                let key = tine_core::refs::page_key(&name);
                if page.kind == PageKind::Page {
                    page_claimed_names.insert(key.clone());
                }
                claimed_names.insert(key);
                entries.push(InventoryEntry {
                    name,
                    target: Resolved::Existing {
                        id: ids[0].clone(),
                        others: ids[1..].to_vec(),
                    },
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
    /// after `refs::page_key`). Supply display names; this method normalizes
    /// them, so pre-normalized keys are not required. Explicit means OG's
    /// `:block/refs`: page refs, tags, property values such as `tags::`, and
    /// `{{embed}}`. This can include a property reference that `RenameMap`
    /// does not rewrite; pass only supported rewrite names to that map. It
    /// excludes arguments of
    /// `{{query}}` or other macros, so a page that mentions a name only inside a
    /// query is not a referrer. The answer comes from this snapshot's parse, not
    /// from disk. Cost ranges from O(matching references) to O(P + B) over the
    /// parsed snapshot.
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
            .map(|(entry, _)| PageId::from(entry.rel_path_str()))
            .collect();
        ids.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.dedup();
        ids
    }

    /// Last published observation of the file's modification time. A view
    /// acquired later may reuse that observation without another filesystem
    /// read. `None` means no timestamp was recorded for this identity.
    /// Cost O(1), without a filesystem read.
    pub fn page_mtime(&self, id: &PageId) -> Option<std::time::SystemTime> {
        self.observed_mtimes.get(id.as_str()).copied()
    }

    /// Resolve a name in this captured view. The caller supplies whether the
    /// name is a journal title; using the wrong kind searches that other
    /// namespace and may propose a new file. Real files win over aliases.
    /// Journal files with a configured date stem rank first, then Markdown
    /// before Org, then filename and full path lexicographically. Ordinary
    /// page claimants use the latter three rules. Claims come from decoded
    /// filenames and journal dates, not a parsed `title::` property. After a
    /// journal-format change, a new view uses the new title format for date
    /// claims; the parser still tries its documented fallback formats. Old
    /// custom-format links are not rewritten. To classify a clicked journal
    /// title, use the current `JournalFormat::parse`, which tries configured
    /// formats and fallbacks, then pass that result as `is_journal`. Cost
    /// O(1) index lookup when a real file wins; O(alias owners) otherwise.
    pub fn resolve(&self, name: &str, is_journal: bool) -> Resolved {
        let kind = if is_journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let entries = self
            .claimants
            .get(&(kind, tine_core::refs::page_key(name)))
            .cloned()
            .unwrap_or_default();
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
        let config = &self.config.config;
        let (dir, stem) = if is_journal {
            let stem = self
                .journal_format
                .parse(name)
                .map(|date| self.journal_format.file_stem(date))
                .unwrap_or_else(|| name.to_owned());
            (&config.journals_dir, stem)
        } else {
            (
                &config.pages_dir,
                tine_core::model::encode_page_name(name, config.file_name_format),
            )
        };
        Resolved::Absent {
            id: PageId::from(format!("{dir}/{stem}.{}", config.preferred_format.ext())),
        }
    }

    fn validated_page(&self, id: &PageId) -> Result<(), QueryError> {
        let path = id.as_str();
        let valid_area = path.starts_with(&format!("{}/", self.config.config.pages_dir))
            || path.starts_with(&format!("{}/", self.config.config.journals_dir));
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

    /// Execute one simple or advanced query macro over this stable view.
    /// `current_page`, when supplied, must be a syntactically valid page id in
    /// this graph. Existence is not checked. It does not affect evaluation:
    /// `:current-page` inputs are not supported, as in v0.6.5. A simple
    /// source above 64 KiB or 64 parenthesis levels returns
    /// `QueryError::Parse`; advanced unsupported clauses appear in its
    /// diagnostics. Results are bounded to 20,000 rows and 32 MiB. Query cost
    /// depends on the evaluated clauses; a cold full-graph query can visit
    /// O(P + B) pages and blocks before result materialization.
    pub fn query(
        &self,
        source: &str,
        dialect: QueryDialect,
        current_page: Option<&PageId>,
    ) -> Result<QueryResult, QueryError> {
        if let Some(id) = current_page {
            self.validated_page(id)?;
        }
        if matches!(dialect, QueryDialect::Simple) {
            if !tine_core::query::query_source_within_limit(source) {
                return Err(QueryError::Parse(format!(
                    "query exceeds {} bytes",
                    tine_core::query::QUERY_SOURCE_MAX_BYTES
                )));
            }
            if !tine_core::query::query_nesting_within_limit(source) {
                return Err(QueryError::Parse("query nesting exceeds 64 levels".into()));
            }
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

    /// Graph search, with an optional syntactically checked file scope and
    /// caller cancellation. An absent scoped file yields an empty answer;
    /// check existence separately when it matters.
    /// The combined page and block hit allowance is 20,000. The page limit is
    /// clamped first, then the block limit to the remaining allowance; asking
    /// for 20,000 page hits leaves no block-hit allowance. A cancelled call
    /// returns `QueryError::Cancelled`
    /// without a partial result. Inspect `QueryExecution::has_more` for
    /// omitted hits in enabled categories; a zero category limit is not
    /// checked. A successful result has `cancelled == false` through this
    /// API; the lower-level execution type also represents cancelled work.
    /// A cold search can scan O(P + B + text bytes); result construction is
    /// bounded by the requested hit limits.
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

    /// Publication revision captured at acquisition, O(1). Later reads of
    /// this view remain at the same revision.
    pub fn rev(&self) -> GraphRev {
        self.rev
    }

    /// Backlinks to a decoded page or journal title or alias, compared by
    /// normalized name without a separate namespace parameter. Matching is by
    /// name, not journal date: old-title links after a format change count only
    /// when their normalized spelling still matches the requested title. Results are
    /// keyed by display name, so twin files cannot be distinguished by this
    /// result alone. Worst case O(B), with limits of 20,000 rows and 32 MiB.
    pub fn backlinks(&self, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph
                .backlinks_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES),
            Budget::MatchingBlocks,
        )
    }

    /// Unlinked mentions of a decoded page or journal title or alias, compared
    /// by normalized name without a separate namespace parameter. Worst case
    /// O(B + matching text bytes),
    /// with limits of 20,000 rows and 32 MiB.
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

    /// Resolve runtime structural block ids or persisted `id::` values in
    /// request order; unknown ids yield `None`. Runtime ids can change when
    /// the page structure changes, while persisted ids remain external refs.
    /// Cost ranges from the identified page's blocks to O(B) if the id cannot
    /// be routed directly to a page. At most 20,000
    /// requested/result rows and 32 MiB of result data.
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

    /// Bounded subtree preview. `max_nodes` is clamped to 1..=2,000 and
    /// output is capped below 32 MiB. Inspect `BlockPreview::truncated`:
    /// `Some` can omit descendants or even the root under the byte cap.
    /// Cost ranges from one identified page's blocks to O(B) if the id cannot
    /// be routed directly to a page.
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

    /// Referrers of a persisted `id::` value, worst case O(B), with fixed
    /// row/byte limits. Runtime structural block ids are not reference ids.
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

    /// Counts keyed by persisted `id::` values; up to O(B) over this view.
    pub fn block_ref_counts(&self) -> Arc<HashMap<String, usize>> {
        self.graph.block_ref_counts()
    }

    /// `[[` completion over pages, journals, aliases and referenced names.
    /// A first call can traverse all page blocks and reference text to build
    /// indexes; later calls cost O(P + aliases + referenced names), returning
    /// at most `limit` entries.
    pub fn complete_page_names(&self, text: &str, limit: usize) -> Vec<PageEntry> {
        self.graph.quick_switch(text, limit)
    }

    /// Literal `((` block search, O(B) per call, at most `limit` blocks.
    /// Frequent calls on large graphs repeat that scan. Checks `cancel`
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

    /// Selected query subtrees. Accepts at most 1,024 specs and 64 KiB total
    /// across keys and query sources, but evaluates only the first 64. The
    /// batch shares a 50-root, 2,000-node, and 8 MiB output budget; later
    /// results may omit roots or nodes even when small on their own.
    /// Inspect `omitted_queries`, `shown`, `total`, and `omitted_nodes`
    /// before treating `Ok` as complete. Cost up to O(64 × B + selected nodes).
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
            Err(export_bytes_error(bytes))
        } else {
            Ok(batch)
        }
    }

    /// Budgeted facets reject over 20,000 items or 32 MiB; truncated facets
    /// return a prefix of at most 2,000 items and 2 MiB. Cost O(B).
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

    /// Template blocks; up to O(B) over this view.
    pub fn templates(&self) -> Vec<TemplateDto> {
        self.graph.templates()
    }

    /// Icons for requested names, O(P + names + aliases), including a scan of
    /// every parsed page even for one requested name.
    pub fn page_icons(&self, names: &[String]) -> HashMap<String, String> {
        self.graph.page_icons(names)
    }

    /// Journal days with content, O(P + journal blocks) because the scan visits
    /// all parsed pages before filtering for journals.
    pub fn journal_content_days(&self) -> Vec<Day> {
        self.graph
            .journal_content_days()
            .into_iter()
            .map(Day)
            .collect()
    }
}

fn export_bytes_error(bytes: usize) -> QueryError {
    QueryError::ResultTooLarge {
        what: Budget::ExportBytes,
        count: bytes,
        limit: QUERY_EXPORT_MAX_BYTES,
        bytes: Some(bytes),
        byte_limit: QUERY_EXPORT_MAX_BYTES,
    }
}

#[cfg(test)]
mod rev5_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn shared_store_types_are_send_sync() {
        fn require<T: Send + Sync>() {}
        require::<Store>();
        require::<WholeGraph>();
        require::<Subscription>();
        require::<Change>();
    }

    #[test]
    fn save_detects_external_write_after_temp_sync() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-final-guard-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/A.md");
        fs::write(&path, "- before\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "mine".into();
        store.inject_fault(crate::FaultPoint::AfterTempSync);
        assert!(matches!(
            store.save(&id, SaveBase::Existing(read.rev), &doc),
            SaveOutcome::Conflict { .. }
        ));
        assert_eq!(fs::read(&path).unwrap(), b"external after temp sync");
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_load_transaction_write_is_in_first_recovered_view() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-failed-tx-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- before\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let prior = store.whole_graph().unwrap().rev();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "committed while failed".into();
        *store.load.status.lock().unwrap() = LoadStatus::Failed("injected".into());
        let mut tx = store.transaction();
        tx.save_page(&id, SaveBase::Existing(read.rev), &doc);
        assert!(
            matches!(tx.commit(), crate::TxOutcome::Committed { graph_rev, .. } if graph_rev == prior)
        );
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
        store.scan_refresh().unwrap();
        let recovered = store.whole_graph().unwrap();
        assert!(recovered.rev() > prior);
        assert!(recovered.corpus().pages.iter().any(|page| {
            page.name == "A"
                && page.document.roots[0]
                    .raw()
                    .contains("committed while failed")
        }));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_error_reports_effective_byte_limit() {
        match export_bytes_error(QUERY_EXPORT_MAX_BYTES + 1) {
            QueryError::ResultTooLarge {
                limit, byte_limit, ..
            } => {
                assert_eq!(limit, QUERY_EXPORT_MAX_BYTES);
                assert_eq!(byte_limit, QUERY_EXPORT_MAX_BYTES);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn failed_initial_load_defers_page_and_save_publication_until_recovery() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-failed-load-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- before\n").unwrap();
        let pause = root.join(".tine-test-pause-load");
        fs::write(&pause, "").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let changes = store.subscribe();
        *store.load.status.lock().unwrap() = LoadStatus::Failed("injected failure".into());
        store.load.ready.notify_all();
        fs::write(root.join("pages/A.md"), "- changed outside\n").unwrap();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        assert!(read.doc.blocks[0].raw.contains("changed outside"));
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
        assert!(changes.try_recv().unwrap().is_none());

        let mut doc = read.doc;
        doc.blocks[0].raw = "changed here".into();
        assert!(matches!(
            store.save(&id, SaveBase::Existing(read.rev), &doc),
            SaveOutcome::Saved(_)
        ));
        assert!(changes.try_recv().unwrap().is_none());
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));

        store.scan_refresh().unwrap();
        let view = store.whole_graph().unwrap();
        assert!(view.corpus().pages.iter().any(|page| {
            page.name == "A" && page.document.roots[0].raw().contains("changed here")
        }));
        assert!(store.page(&id).unwrap().doc.blocks[0]
            .raw
            .contains("changed here"));
        assert!(changes.try_recv().unwrap().is_some());
        fs::remove_file(pause).unwrap();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_publishes_save_attempted_after_reconcile() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-recovery-save-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- before\n").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let old = store.whole_graph().unwrap();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "saved during recovery".into();
        *store.load.status.lock().unwrap() = LoadStatus::Failed("injected failure".into());
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store
            .watch
            .core_for_load()
            .recovery_reconcile_pause
            .lock()
            .unwrap() = Some(Arc::clone(&pause));
        let recovery_store = Arc::clone(&store);
        let recovery = std::thread::spawn(move || recovery_store.scan_refresh().unwrap());
        wait_hook(&pause);
        let save_store = Arc::clone(&store);
        let (attempting, attempted) = mpsc::channel();
        let save = std::thread::spawn(move || {
            attempting.send(()).unwrap();
            save_store.save(&id, SaveBase::Existing(read.rev), &doc)
        });
        attempted.recv_timeout(Duration::from_secs(5)).unwrap();
        // On the old path the save can complete while recovery is paused;
        // after the fix it waits for recovery's writer critical section.
        std::thread::sleep(Duration::from_millis(100));
        release_hook(&pause);
        recovery.join().unwrap();
        assert!(matches!(save.join().unwrap(), SaveOutcome::Saved(_)));
        let view = store.whole_graph().unwrap();
        assert!(view.rev() > old.rev());
        assert!(view.corpus().pages.iter().any(|page| {
            page.name == "A"
                && page.document.roots[0]
                    .raw()
                    .contains("saved during recovery")
        }));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_external_live_bytes_publish_as_external() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-rollback-origin-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- old A\n").unwrap();
        fs::write(root.join("pages/B.md"), "- old B\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let a = PageId::from("pages/A.md");
        let b = PageId::from("pages/B.md");
        let read_a = store.page(&a).unwrap();
        let read_b = store.page(&b).unwrap();
        let mut doc_a = read_a.doc;
        let mut doc_b = read_b.doc;
        doc_a.blocks[0].raw = "new A".into();
        doc_b.blocks[0].raw = "new B".into();
        let mut tx = store.transaction();
        tx.save_page(&a, SaveBase::Existing(read_a.rev), &doc_a);
        tx.save_page(&b, SaveBase::Existing(read_b.rev), &doc_b);
        store.inject_fault(crate::FaultPoint::MidStepIoAt(1));
        store.inject_fault(crate::FaultPoint::UndoLiveWrite);
        assert!(matches!(tx.commit(), crate::TxOutcome::NotCommitted { .. }));
        let found: Vec<_> = std::iter::from_fn(|| changes.try_recv().unwrap()).collect();
        assert!(
            found.iter().any(|change| {
                change.origin == Origin::External
                    && change.files.iter().any(|(id, _, _)| id == &b.file())
            }),
            "{found:?}"
        );
        let view = store.whole_graph().unwrap();
        assert!(view.corpus().pages.iter().any(|page| {
            page.name == "B"
                && page
                    .document
                    .pre_block
                    .as_ref()
                    .is_some_and(|block| block.contains("external during undo"))
        }));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_directory_purge_reports_deleted_bytes() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tine-partial-purge-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("one"), b"abc").unwrap();
        fs::write(dir.join("two"), b"def").unwrap();
        let mut bytes = 0;
        let mut calls = 0;
        let failure = remove_trash_entry_counted(&dir, &mut bytes, &mut |path| {
            calls += 1;
            if calls == 2 {
                return Err(std::io::Error::other("injected removal failure"));
            }
            fs::remove_file(path)
        });
        assert!(failure.is_err());
        assert_eq!(bytes, 3);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    fn wait_hook(pause: &TestPause) {
        let (state, ready) = &**pause;
        let mut state = state.lock().unwrap();
        while !state.0 {
            state = ready.wait(state).unwrap();
        }
    }

    fn release_hook(pause: &TestPause) {
        let (state, ready) = &**pause;
        state.lock().unwrap().1 = true;
        ready.notify_all();
    }

    #[test]
    fn whole_graph_reader_completes_during_writer_and_snapshot_publication() {
        let root = std::env::temp_dir().join(format!(
            "tine-d3-noblock-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/Source.md"), "- [[Target]] before\n").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let old = store.whole_graph().unwrap();
        let id = PageId::from("pages/Source.md");
        for during_cache_write in [true, false] {
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw.push_str(" edited");
            let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
            let hook = if during_cache_write {
                &store.graph.cache_publish_pause
            } else {
                &store.changes.snapshot_publish_pause
            };
            *hook.lock().unwrap() = Some(Arc::clone(&pause));
            let writer_store = Arc::clone(&store);
            let writer_id = id.clone();
            let writer = std::thread::spawn(move || {
                writer_store.save(&writer_id, SaveBase::Existing(read.rev), &doc)
            });
            wait_hook(&pause);
            let (send, receive) = mpsc::channel();
            let reader_store = Arc::clone(&store);
            let reader_view = old.clone();
            let reader = std::thread::spawn(move || {
                let acquired = reader_store.whole_graph().unwrap();
                let result = (
                    reader_view.backlinks("Target").unwrap().len(),
                    acquired.resolve("Source", false),
                );
                send.send(result).unwrap();
            });
            let result = receive.recv_timeout(Duration::from_secs(2));
            release_hook(&pause);
            *hook.lock().unwrap() = None;
            assert!(matches!(writer.join().unwrap(), SaveOutcome::Saved(_)));
            reader.join().unwrap();
            let (count, resolved) = result.expect("reader blocked behind writer publication");
            assert_eq!(count, 1);
            assert!(matches!(resolved, Resolved::Existing { .. }));
        }
        store.close();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whole_graph_carries_only_unaffected_backlink_memos() {
        let root = std::env::temp_dir().join(format!(
            "tine-d3-memos-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- [[Alpha]] original\n").unwrap();
        fs::write(root.join("pages/B.md"), "- [[Beta]] original\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let old = store.whole_graph().unwrap();
        assert_eq!(old.backlinks("Alpha").unwrap().len(), 1);
        assert_eq!(old.backlinks("Beta").unwrap().len(), 1);
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "[[Alpha]] edited".into();
        assert!(matches!(
            store.save(&id, SaveBase::Existing(read.rev), &doc),
            SaveOutcome::Saved(_)
        ));
        let fresh = store.whole_graph().unwrap();
        let before = crate::query::result_dto_constructions();
        assert_eq!(fresh.backlinks("Beta").unwrap().len(), 1);
        assert_eq!(crate::query::result_dto_constructions(), before);
        let alpha = fresh.backlinks("Alpha").unwrap();
        assert!(crate::query::result_dto_constructions() > before);
        assert_eq!(alpha[0].blocks[0].raw, "[[Alpha]] edited");
        assert_eq!(
            old.backlinks("Alpha").unwrap()[0].blocks[0].raw,
            "[[Alpha]] original"
        );
        store.close();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whole_graph_concurrent_reader_writer_watcher_loop() {
        use std::sync::atomic::AtomicUsize;
        use std::time::Instant;
        let root = std::env::temp_dir().join(format!(
            "tine-d3-loop-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/Writer.md"), "- [[Target]] initial\n").unwrap();
        fs::write(root.join("pages/External.md"), "- outside initial\n").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..3 {
            let store = Arc::clone(&store);
            let stop = Arc::clone(&stop);
            let reads = Arc::clone(&reads);
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let view = store.whole_graph().unwrap();
                    let rev = view.rev();
                    let page_count = view.corpus().pages.len();
                    let _ = view.backlinks("Target").unwrap();
                    let _ = view
                        .query("[[Target]]", QueryDialect::Simple, None)
                        .unwrap();
                    let _ = view.resolve("External", false);
                    let _ = view.inventory();
                    assert_eq!(view.rev(), rev);
                    assert_eq!(view.corpus().pages.len(), page_count);
                    reads.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        let writer_store = Arc::clone(&store);
        let writer_stop = Arc::clone(&stop);
        threads.push(std::thread::spawn(move || {
            let id = PageId::from("pages/Writer.md");
            let mut n = 0;
            while !writer_stop.load(Ordering::Acquire) {
                let read = writer_store.page(&id).unwrap();
                let mut doc = read.doc;
                doc.blocks[0].raw = format!("[[Target]] edit {n}");
                assert!(matches!(
                    writer_store.save(&id, SaveBase::Existing(read.rev), &doc),
                    SaveOutcome::Saved(_)
                ));
                n += 1;
            }
        }));
        let watch_store = Arc::clone(&store);
        let watch_stop = Arc::clone(&stop);
        let external = root.join("pages/External.md");
        threads.push(std::thread::spawn(move || {
            let mut n = 0;
            while !watch_stop.load(Ordering::Acquire) {
                fs::write(&external, format!("- outside {n}\n")).unwrap();
                watch_store.scan_refresh().unwrap();
                n += 1;
            }
        }));
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        stop.store(true, Ordering::Release);
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(reads.load(Ordering::Relaxed) > 0);
        store.close();
        fs::remove_dir_all(root).unwrap();
    }

    fn view_answers(view: &WholeGraph) -> Vec<(&'static str, String)> {
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let id = PageId::from("pages/Source.md");
        let query_rows = |dialect| match view.query("[[Target]]", dialect, Some(&id)).unwrap() {
            QueryResult::Simple(rows) => format!("{rows:?}"),
            QueryResult::Advanced(rows) => format!("{rows:?}"),
        };
        let resolved = |name| match view.resolve(name, false) {
            Resolved::Existing { id, others } => format!("existing:{id:?}:{others:?}"),
            Resolved::Alias { owners } => format!("alias:{owners:?}"),
            Resolved::Absent { id } => format!("absent:{id:?}"),
        };
        let mut assets: Vec<_> = view.referenced_assets().iter().cloned().collect();
        assets.sort();
        vec![
            ("rev", format!("{:?}", view.rev())),
            ("unreadable_files", format!("{:?}", view.unreadable_files())),
            ("page_mtime", format!("{:?}", view.page_mtime(&id))),
            (
                "corpus",
                format!(
                    "{:?}",
                    view.corpus()
                        .pages
                        .iter()
                        .map(|p| &p.name)
                        .collect::<Vec<_>>()
                ),
            ),
            ("referenced_assets", format!("{assets:?}")),
            (
                "inventory",
                format!(
                    "{:?}",
                    view.inventory()
                        .0
                        .iter()
                        .map(|entry| &entry.name)
                        .collect::<Vec<_>>()
                ),
            ),
            (
                "explicit_referrers",
                format!("{:?}", view.explicit_referrers(&["Target".into()])),
            ),
            ("resolve", resolved("New Alias")),
            ("resolve_absent", resolved("A/B")),
            ("query_simple", query_rows(QueryDialect::Simple)),
            ("query_advanced", query_rows(QueryDialect::Advanced)),
            (
                "search",
                format!(
                    "{:?}",
                    view.search(
                        &SearchRequest {
                            text: "Target".into(),
                            within: None,
                            page_limit: 10,
                            block_limit: 10,
                            explain: false
                        },
                        &cancel
                    )
                    .unwrap()
                    .hits
                ),
            ),
            (
                "backlinks",
                format!("{:?}", view.backlinks("Target").unwrap()),
            ),
            (
                "unlinked_references",
                format!("{:?}", view.unlinked_references("Target").unwrap()),
            ),
            (
                "backlink_filter_context",
                format!("{:?}", view.backlink_filter_context("Target", &[]).unwrap()),
            ),
            (
                "blocks",
                format!("{:?}", view.blocks(&["d3-block".into()]).unwrap()),
            ),
            (
                "preview_block",
                format!("{:?}", view.preview_block("d3-block", 10).unwrap()),
            ),
            (
                "block_referrers",
                format!("{:?}", view.block_referrers("d3-block").unwrap()),
            ),
            ("block_ref_counts", format!("{:?}", view.block_ref_counts())),
            (
                "complete_page_names",
                format!(
                    "{:?}",
                    view.complete_page_names("", 20)
                        .iter()
                        .map(|entry| &entry.name)
                        .collect::<Vec<_>>()
                ),
            ),
            (
                "find_blocks",
                format!("{:?}", view.find_blocks("Target", 10, &cancel).unwrap()),
            ),
            (
                "export_query_subtrees",
                format!(
                    "{:?}",
                    view.export_query_subtrees(&[QueryExportSpec {
                        key: "d3".into(),
                        query: "[[Target]]".into(),
                        advanced: false
                    }])
                    .unwrap()
                ),
            ),
            (
                "property_facets",
                format!("{:?}", view.property_facets(FacetPolicy::Budgeted).unwrap()),
            ),
            ("templates", format!("{:?}", view.templates())),
            (
                "page_icons",
                format!("{:?}", view.page_icons(&["Source".into()])),
            ),
            (
                "journal_content_days",
                format!("{:?}", view.journal_content_days()),
            ),
        ]
    }

    #[test]
    fn whole_graph_view_is_stable_across_external_publication() {
        let root = std::env::temp_dir().join(format!(
            "tine-d3-stability-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/Source.md"), "- [[Target]] before\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let old = store.whole_graph().unwrap();
        let original_answers = view_answers(&old);
        let old_corpus = old.corpus().pages.len();
        let old_backlinks = old.backlinks("Target").unwrap().len();
        let old_inventory = old.inventory().0.len();
        assert!(matches!(
            old.resolve("Added", false),
            Resolved::Absent { .. }
        ));

        let source = PageId::from("pages/Source.md");
        let read = store.page(&source).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "[[Target]] after save".into();
        assert!(matches!(
            store.save(&source, SaveBase::Existing(read.rev), &doc),
            SaveOutcome::Saved(_)
        ));
        assert_eq!(view_answers(&old), original_answers);
        let fresh_after_save = store.whole_graph().unwrap();
        assert_ne!(view_answers(&fresh_after_save), original_answers);

        let mut tx = store.transaction();
        tx.create(
            &FileId::from("logseq/config.edn".to_owned()),
            crate::transaction::Content::Bytes(b"{:file/name-format :triple-lowbar}\n".to_vec()),
        );
        assert!(matches!(
            tx.commit(),
            crate::transaction::TxOutcome::Committed { .. }
        ));
        assert_eq!(view_answers(&old), original_answers);

        let alias_page = PageDto {
            name: "AliasOwner".into(),
            kind: PageKind::Page,
            title: "AliasOwner".into(),
            pre_block: Some("alias:: New Alias".into()),
            blocks: vec![tine_core::model::BlockDto {
                id: "alias-block".into(),
                raw: "owner".into(),
                ..Default::default()
            }],
            rev: None,
            format: Default::default(),
            read_only: false,
            guide: false,
        };
        assert!(matches!(
            store.save(
                &PageId::from("pages/AliasOwner.md"),
                SaveBase::CreateNew,
                &alias_page
            ),
            SaveOutcome::Saved(_)
        ));
        assert_eq!(view_answers(&old), original_answers);

        fs::write(root.join("pages/Added.md"), "- [[Target]] after\n").unwrap();
        store.scan_refresh().unwrap();
        let fresh = store.whole_graph().unwrap();
        assert!(fresh.rev() != old.rev());
        assert_eq!(old.corpus().pages.len(), old_corpus);
        assert_eq!(old.backlinks("Target").unwrap().len(), old_backlinks);
        assert_eq!(old.inventory().0.len(), old_inventory);
        assert!(matches!(
            old.resolve("Added", false),
            Resolved::Absent { .. }
        ));
        assert!(matches!(
            fresh.resolve("Added", false),
            Resolved::Existing { .. }
        ));
        assert!(matches!(
            fresh.resolve("New Alias", false),
            Resolved::Alias { .. }
        ));
        assert_eq!(view_answers(&old), original_answers);
        let fresh_answers = view_answers(&fresh);
        store.close();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(view_answers(&old), original_answers);
        assert_eq!(view_answers(&fresh), fresh_answers);
    }

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
