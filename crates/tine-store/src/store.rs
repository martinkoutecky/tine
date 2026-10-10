//! Synchronous graph I/O and stable published graph views. `Store::open`
//! lists page and journal files before returning; parsing then runs in the
//! background. `Store::whole_graph` waits for that work and returns a view
//! whose answers remain stable across later changes. `GraphRev` identifies
//! the publication captured by that view. A new view is needed to see edits.
//!
//! One `Store::subscribe` consumer receives later publications in order, without
//! replay. A new subscription ends
//! the previous one with `SubscriptionEnd::Displaced`; the queue has no size bound.
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
//! can wait for the graph-wide parse. A loaded view shares untouched page and
//! metadata trees; publishing one edited page patches its blocks and affected
//! names with O(log P) tree updates. Keep the caller's unsaved edits on a
//! refusal. Multi-file commit is not crash atomic.
//!
//! `Store::refresh(Depth::Stamps)` compares file modification time and length, so a
//! same-length edit with unchanged timestamp may remain unseen. It waits for
//! the initial parse. Poll mode scans O(P) file metadata and re-hashes
//! `logseq/config.edn` every three seconds; notification mode silently falls
//! back to polling if needed.
//! `Store::close` waits for an in-flight writer and ends observation and the
//! subscription. These calls have no general timeout. Run blocking calls off
//! a UI thread.

use crate::path_identity::canonical_existing_path;
mod page_identity;
mod save_failure;
use std::collections::{BTreeMap, VecDeque};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Instant, SystemTime};

use crate::model::{classify_legacy_trash_entry, trash_dir_kind, trash_root, TrashEntryKind};

use serde::{Deserialize, Serialize};
use tine_core::date::{JournalDate, JournalFormat};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockPreview, BoundedRefGroups, PageDto,
    PageEntry, PageKind, RefGroup, TemplateDto,
};
pub use tine_core::model::{FileId, PageId};
#[cfg(any(test, feature = "test-faults"))]
use tine_core::query::AdvancedResult;
use tine_core::query::{QueryExportBatch, QueryExportSpec};
use tine_core::query_plan::QueryExecution;

use crate::model::{CheckedOpenError, Graph, ReadSnapshot};

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
    /// A launch checkpoint is served while still Loading (ADR 0070).
    pub(crate) serving: AtomicBool,
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
            serving: AtomicBool::new(false),
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
    /// The app-data file holding this graph's launch checkpoint (ADR 0070);
    /// `None` keeps none. Never under the graph root.
    pub launch_checkpoint: Option<PathBuf>,
}

/// How deep a [`Store::refresh`] looks at the graph on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// Trust a file whose modification time, length and identity are
    /// unchanged; hash only files whose stamps moved. The watcher's and the
    /// window-focus rescan's depth.
    Stamps,
    /// Hash every graph file, publishing the ones whose bytes differ from the
    /// recorded revision as ordinary external changes (a sync client or a
    /// restore tool can rewrite a file and put its stamp back). For a
    /// consumer that must see every byte on disk, such as an export.
    Bytes,
    /// [`Self::Bytes`], then re-read and re-parse every file through the
    /// cold-launch build and replace the page cache, the name index and the
    /// read evaluator: the Settings "Rescan graph" button.
    Rebuild,
}

/// How the store observes graph files after opening.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WatchMode {
    /// Default: use filesystem notifications with a 200 ms debounce. A
    /// notified file is hashed even if its length and mtime match the previous
    /// observation. Failure to install notifications for the graph root falls
    /// back to three-second polling, retries the watch every cycle, and is
    /// reported through [`Subscription::observe_watch_status`]. Managed root
    /// identity is checked every three seconds; a deleted and recreated root
    /// is watched again, with one full reconcile for edits made in the gap.
    /// A backend that
    /// installs successfully but never emits events is not detected.
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
    /// Watcher, direct page read, or explicit scan reconciliation, including the initial
    /// load-completion publication with no file tuple.
    External,
}

/// Byte-level observation of a file across a publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// A file appeared, including an own save or create with a new target.
    Created,
    /// File bytes changed.
    Modified,
    /// Metadata changed while the observed byte revision stayed equal; refresh
    /// displayed metadata. This does not prove no unobserved write occurred.
    Touched,
    /// A file disappeared.
    Removed,
}

/// One published graph change; subscriptions deliver revisions in order.
/// `Change` is `Send + Sync` and can cross worker-thread boundaries.
/// Its Serialize implementation emits a bounded derived-answer wire: `rev`,
/// `inventoryChanged` (name/alias/reference-name or unreadable inventory inputs),
/// and `blockRefCounts` (final changed-target counts, zero for removal). Save
/// results and notifications use this same signal. Serialization costs O(changed
/// targets); publication computes it from changed parsed pages without inventory
/// materialization. Initial publication may include all counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Graph revision after this change.
    pub graph_rev: GraphRev,
    /// Whether this store or an external actor supplied the final bytes in
    /// this publication. Rollback can emit separate Own and External changes.
    pub origin: Origin,
    /// Affected graph files, including `logseq/config.edn` when observed, with
    /// resulting revisions when present. An external config edit enters this
    /// feed on the watcher cycle that observes it (an event naming the file, a
    /// rescan, or every poll cycle) or on `refresh()`, only when its bytes
    /// changed. An own config create or replace lists its config file tuple in
    /// the same transaction publication.
    /// Trash destinations are not listed. A committed transaction or restore
    /// lists an asset path here when its final bytes differ from the
    /// operation's starting bytes. The watcher also observes the graph's assets
    /// directory (the approved external target when `assets` is a link): an
    /// outside create, replace or delete of a file there is an
    /// `Origin::External` `assets/<path relative to that directory>` tuple with
    /// no revision, from file metadata only (cost: `asset_watch.rs`).
    /// Sync-conflict
    /// copies may appear as files while [`Self::page`] returns `None` for them;
    /// they are excluded from parsed search, backlinks, and page inventory.
    pub files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    pages: Vec<(FileId, PageKind, String)>,
    answers: (bool, BTreeMap<String, usize>),
    /// The watcher batch that produced this publication, for latency
    /// receipts (GH #337 diagnosis); `None` for every other publisher.
    pub watch: Option<WatchBatch>,
}

/// How one watcher cycle observed a publication: monotonic stamps and counts
/// only, no paths. The window adapter turns it into a latency receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchBatch {
    /// First notification of the drained batch; `None` for a poll cycle.
    pub first_event_at: Option<std::time::Instant>,
    /// When reconciliation of the batch began.
    pub reconcile_started: std::time::Instant,
    /// The cycle ran without live notifications (poll mode or refused watch).
    pub poll: bool,
    /// The full stat-diff branch ran (poll, unclassifiable event, burst).
    pub full_diff: bool,
    /// Exact event paths the batch carried (0 for a pure full diff).
    pub event_paths: usize,
}

impl Change {
    /// The parsed graph page altered by an external observation or an external
    /// writer whose bytes survived transaction undo, using its effective
    /// `title::`-aware name (the old name for a removal). Own writes
    /// list changed files but carry no parsed page entries, so this returns
    /// `None` for them. It also returns `None` for a file with no parsed page
    /// change, including a duplicate journal claimant or sync-conflict copy.
    /// A move lists its old file as removed and its destination as created,
    /// without a pairing identifier;
    /// `page()` can describe either only when parsed page evidence is present.
    /// Cost O(parsed page entries in this publication) per call; calling it
    /// for every file can be quadratic in a large publication.
    pub fn page(&self, file: &FileId) -> Option<(PageKind, &str)> {
        self.pages
            .iter()
            .find(|(id, _, _)| id == file)
            .map(|(_, kind, name)| (*kind, name.as_str()))
    }
}

/// Why a change subscription ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionEnd {
    /// The store closed.
    StoreClosed,
    /// A second subscriber took ownership of this stream.
    Displaced,
}

pub(crate) struct ChangeFeed {
    state: Mutex<FeedState>,
    ready: Condvar,
    graph: Arc<Graph>,
    journal_ids: Arc<Mutex<HashMap<Day, PageId>>>,
    config: Arc<RwLock<ConfigState>>,
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    /// Why live notifications are refused right now (`None` while watching or
    /// polling by choice), and the subscriber told when that changes.
    watch_status: Mutex<(Option<String>, Option<StatusObserver>)>,
    /// The launch checkpoint publisher, when this store keeps one.
    pub(crate) checkpoint: std::sync::OnceLock<Arc<checkpoint::Signal>>,
    #[cfg(test)]
    pub(crate) snapshot_publish_pause: Mutex<Option<TestPause>>,
}

/// Told `Some(reason)` when the OS refuses live notifications and `None`
/// when they work again after a refusal.
type StatusObserver = Arc<dyn Fn(Option<String>) + Send + Sync>;

struct FeedState {
    rev: u64,
    subscription: u64,
    queue: VecDeque<Change>,
    closed: bool,
}

use crate::model::persistent::{EntryList, Map as SharedMap};

struct Snapshot {
    graph: Arc<ReadSnapshot>,
    rev: GraphRev,
    cache_generation: u64,
    config: ConfigState,
    journal_format: Arc<JournalFormat>,
    list: Arc<EntryList>,
    claimants: Arc<SharedMap<(bool, String), Vec<PageEntry>>>,
    name_by_path: Arc<SharedMap<PathBuf, (PageKind, String)>>,
    unreadable: Arc<Vec<(FileId, String)>>,
    answers: (bool, BTreeMap<String, usize>),
    /// This publication only folded or unfolded blocks (GH #623 item 3).
    folds_only: bool,
}

mod answer_changes;
pub(crate) mod checkpoint;
mod diagnostics;
#[cfg(test)]
mod fold_save_tests;
mod inventory;
#[cfg(test)]
mod page_open_tests;
mod page_read;
mod snapshot;
pub(crate) use snapshot::PublishedObservations;

impl ChangeFeed {
    /// Record the watcher's live-notification state. A new refusal message is
    /// reported once (a retry failing the same way stays quiet); `restored`
    /// reports the end of a refusal, which a user-selected Poll mode does not.
    pub(crate) fn set_watch_refusal(&self, now: Option<String>, restored: bool) {
        let mut state = self.watch_status.lock().unwrap();
        if state.0 == now {
            return;
        }
        let status = match (&now, state.0.is_some()) {
            (Some(message), _) => Some(Some(message.clone())),
            (None, true) if restored => Some(None),
            (None, _) => None,
        };
        state.0 = now;
        let observer = state.1.clone();
        drop(state);
        if let (Some(status), Some(observer)) = (status, observer) {
            observer(status);
        }
    }

    fn new(
        graph: Arc<Graph>,
        config: Arc<RwLock<ConfigState>>,
        journal_ids: Arc<Mutex<HashMap<Day, PageId>>>,
    ) -> Self {
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
            journal_ids,
            snapshot: RwLock::new(None),
            watch_status: Mutex::new((None, None)),
            checkpoint: std::sync::OnceLock::new(),
            #[cfg(test)]
            snapshot_publish_pause: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn initialize(&self) {
        let snapshot = Snapshot::capture(
            &self.graph,
            &self.config,
            None,
            &[],
            false,
            false,
            GraphRev(0),
            &HashMap::new(),
        );
        *self.snapshot.write().unwrap() = Some(Arc::new(snapshot));
    }

    pub(crate) fn publish(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
    ) -> GraphRev {
        self.publish_with(origin, files, config_changed, pages, || {})
    }

    /// [`Self::publish`] with page entries a transaction's publication named
    /// from the bytes it read, which the name index reuses (GH #623).
    pub(crate) fn publish_observed(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
        published: &HashMap<FileId, PageEntry>,
    ) -> GraphRev {
        self.publish_inner(
            origin,
            files,
            config_changed,
            pages,
            || {},
            None,
            false,
            published,
        )
    }

    // `before_notify` runs after the new snapshot exists, but before the
    // subscriber can receive its Change. It must only make a short state update.
    pub(crate) fn publish_with(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
        before_notify: impl FnOnce(),
    ) -> GraphRev {
        self.publish_watched(origin, files, config_changed, pages, before_notify, None)
    }

    pub(crate) fn publish_watched(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
        before_notify: impl FnOnce(),
        watch: Option<WatchBatch>,
    ) -> GraphRev {
        self.publish_inner(
            origin,
            files,
            config_changed,
            pages,
            before_notify,
            watch,
            false,
            &HashMap::new(),
        )
    }

    /// Publish after a forced rebuild (`WatchHandle::rebuild_all`): the new
    /// snapshot recomputes the name index and the read evaluator from the
    /// rebuilt cache instead of carrying the previous generation's.
    pub(crate) fn publish_rebuilt(&self, files: Vec<(FileId, ChangeKind, Option<FileRev>)>) {
        self.publish_inner(
            Origin::External,
            files,
            false,
            Vec::new(),
            || {},
            None,
            true,
            &HashMap::new(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_inner(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        pages: Vec<(FileId, PageKind, String)>,
        before_notify: impl FnOnce(),
        watch: Option<WatchBatch>,
        rebuild: bool,
        published: &HashMap<FileId, PageEntry>,
    ) -> GraphRev {
        let old = self.snapshot.read().unwrap().clone();
        let journals_dir = self.graph.current_config().journals_dir.clone();
        if old.is_none()
            || config_changed
            || rebuild
            || files.iter().any(|(id, kind, _)| {
                id.as_str().starts_with(&format!("{journals_dir}/"))
                    && matches!(kind, ChangeKind::Created | ChangeKind::Removed)
            })
        {
            *self.journal_ids.lock().unwrap() =
                journal_ids_from_entries(&self.graph, self.graph.list_pages_shared().as_ref());
        }
        let rev = GraphRev(self.rev().0 + 1);
        let snapshot = Arc::new(Snapshot::capture(
            &self.graph,
            &self.config,
            old.as_deref(),
            &files,
            config_changed,
            rebuild,
            rev,
            published,
        ));
        // A fold leaves the launch checkpoint's derived state true; its file
        // stamp is stale, so the next launch diff rereads that one file
        // (ADR 0070) instead of the whole checkpoint being rewritten for it.
        if (old.is_none() || config_changed || rebuild || !files.is_empty()) && !snapshot.folds_only
        {
            self.checkpoint.get().inspect(|signal| signal.published());
        }
        #[cfg(test)]
        pause_at_hook(&self.snapshot_publish_pause);
        let mut state = self.state.lock().unwrap();
        state.rev += 1;
        let rev = GraphRev(state.rev);
        debug_assert_eq!(snapshot.rev, rev);
        let answers = snapshot.answers.clone();
        *self.snapshot.write().unwrap() = Some(snapshot);
        before_notify();
        if !state.closed {
            state.queue.push_back(Change {
                graph_rev: rev,
                origin,
                files,
                pages,
                answers,
                watch,
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
    start: GraphRev,
}

impl Subscription {
    /// The last publication before this subscription began; it receives
    /// exactly the publications after it.
    pub fn start_rev(&self) -> GraphRev {
        self.start
    }

    /// Install the one observer told `Some(reason)` when the OS refuses live
    /// notifications for the graph root (the watcher then polls every three
    /// seconds and retries the watch each cycle) and `None` when they work
    /// again. A refusal already in force is reported at once. Replaces any
    /// earlier observer.
    pub fn observe_watch_status(&self, observer: impl Fn(Option<String>) + Send + Sync + 'static) {
        let observer: StatusObserver = Arc::new(observer);
        let mut state = self.feed.watch_status.lock().unwrap();
        state.1 = Some(Arc::clone(&observer));
        let refused = state.0.clone();
        drop(state);
        if let Some(message) = refused {
            observer(Some(message));
        }
    }

    /// Wait without a timeout for the next change; returns a typed end reason
    /// after close or displacement. A queued result is O(1) to dequeue.
    pub fn recv(&self) -> Result<Change, SubscriptionEnd> {
        let mut state = self.feed.state.lock().unwrap();
        loop {
            if state.closed {
                return Err(SubscriptionEnd::StoreClosed);
            }
            if state.subscription != self.number {
                return Err(SubscriptionEnd::Displaced);
            }
            if let Some(change) = state.queue.pop_front() {
                return Ok(change);
            }
            state = self.feed.ready.wait(state).unwrap();
        }
    }

    /// Take the next queued change without waiting, or `None` when none is
    /// queued. Returns a typed end reason after close or displacement. A test
    /// oracle (SURFACE.txt rule 4): production waits with `recv`.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn try_recv(&self) -> Result<Option<Change>, SubscriptionEnd> {
        let mut state = self.feed.state.lock().unwrap();
        if state.closed {
            return Err(SubscriptionEnd::StoreClosed);
        }
        if state.subscription != self.number {
            return Err(SubscriptionEnd::Displaced);
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
    pub fn approves_external_assets(&self, path: &Path) -> Result<bool, crate::IoError> {
        Ok(self.external_assets.as_ref()
            == Some(&canonical_existing_path(path).map_err(crate::IoError::from)?))
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
    /// Directory layout is unsafe for graph access, for example a configured
    /// page or journal path escaping the graph through a symlink.
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

/// Effective config. While `problem` is set, reads are read-only and mutations
/// refuse unknown destinations. `refresh()` retries the read after repair.
#[derive(Clone)]
pub struct ConfigState {
    /// Effective graph config, defaulted when loading config failed. A changed
    /// journal title format affects names and date claims in new views after
    /// publication; existing file bytes and reference text are not rewritten.
    /// Changing the journal filename format can leave old custom-stem files
    /// without a date claim if neither the new format nor a fallback parses
    /// their stem; callers must plan any file migration separately.
    pub config: Arc<tine_core::config::Config>,
    /// Config read error, if one occurred. Missing config is not an error.
    /// Unrecognized or malformed config values may default individually and do
    /// not set this field. Journal format strings are not validated here:
    /// unsupported format characters become literal text, and a format that
    /// cannot parse its own rendered title can make a new journal unresolvable
    /// by title. Callers setting formats must verify their intended examples.
    pub problem: Option<crate::IoError>,
    /// Final component of the validated assets directory, for backup layout.
    pub assets_directory_name: String,
    /// The recoverable asset trash location, for display in user-facing
    /// errors only; never a write target (graph writes take a [`FileId`]).
    pub asset_trash_location: PathBuf,
}

impl ConfigState {
    /// The effective config state of `graph` with its layout display fields.
    pub(crate) fn of(
        graph: &crate::model::Graph,
        config: Arc<tine_core::config::Config>,
        problem: Option<crate::IoError>,
    ) -> Self {
        Self {
            config,
            problem,
            assets_directory_name: graph
                .assets_path()
                .file_name()
                .and_then(|part| part.to_str())
                .unwrap_or("dir")
                .to_owned(),
            asset_trash_location: graph.root.join("logseq/.tine-trash/assets"),
        }
    }
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

/// Bytes of regular files under one trash entry. Iterative: a delivered tree
/// of any depth costs heap, not stack (I-22).
fn trash_entry_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    let mut bytes = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        let kind = fs::symlink_metadata(&path)?.file_type();
        if kind.is_file() {
            bytes += fs::metadata(&path)?.len();
        } else if kind.is_dir() {
            for child in fs::read_dir(&path)? {
                pending.push(child?.path());
            }
        }
    }
    Ok(bytes)
}

fn remove_trash_entry_counted(
    path: &Path,
    removed_bytes: &mut u64,
    remove_file: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    // Post-order with an explicit stack: a directory is removed after its
    // children, and depth costs heap, not stack (I-22).
    let mut pending = vec![(path.to_path_buf(), false)];
    while let Some((path, emptied)) = pending.pop() {
        if emptied {
            fs::remove_dir(&path)?;
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            pending.push((path.clone(), true));
            for child in fs::read_dir(&path)? {
                pending.push((child?.path(), false));
            }
        } else {
            remove_file(&path)?;
            if metadata.is_file() {
                *removed_bytes = removed_bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(())
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
                crate::model::compare_page_claimants(
                    a,
                    b,
                    &graph.current_journal_format(),
                    graph.current_config().file_name_format,
                )
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
        let config_state = Arc::new(RwLock::new(ConfigState::of(
            &graph,
            Arc::new(graph.config.clone()),
            None,
        )));
        let changes = Arc::new(ChangeFeed::new(
            Arc::clone(&graph),
            Arc::clone(&config_state),
            Arc::clone(&journal_ids),
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
            crate::watch::Baseline::Walk,
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

    /// Suggest the first unoccupied notes child. Any entry, including an empty
    /// directory or dangling symlink, occupies a name. Cost O(names probed).
    pub fn suggest_graph_name(parent: &Path) -> Result<String, OpenError> {
        if parent.as_os_str().is_empty() || !parent.is_dir() {
            return Err(OpenError::NotAFolder(parent.to_path_buf()));
        }
        for number in 1usize.. {
            let name = if number == 1 {
                "notes".to_string()
            } else {
                format!("notes-{number}")
            };
            let path = parent.join(&name);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(name),
                Ok(_) => {}
                Err(error) => {
                    return Err(OpenError::CreateFailed {
                        path,
                        cause: error.into(),
                    })
                }
            }
        }
        Err(OpenError::CreateFailed {
            path: parent.to_path_buf(),
            cause: std::io::Error::other("No unused graph folder name.").into(),
        })
    }

    /// Create only the named child, reusing it only when it is an empty real
    /// directory. Every file publication is no-clobber, including concurrent
    /// creators and external files arriving during scaffolding. Partial failures
    /// leave created files in place and identify the failing path. Cost O(seed bytes).
    pub fn create_graph(
        parent: &Path,
        name: &str,
        seed: &[(Area, String, Vec<u8>)],
    ) -> Result<PathBuf, OpenError> {
        if parent.as_os_str().is_empty() || !parent.is_dir() {
            return Err(OpenError::NotAFolder(parent.to_path_buf()));
        }
        let failed = |path: &Path, error: std::io::Error| OpenError::CreateFailed {
            path: path.to_path_buf(),
            cause: error.into(),
        };
        if name.chars().any(char::is_control)
            || name.ends_with(' ')
            || !crate::model::portable_component(name.trim())
        {
            return Err(failed(parent, std::io::Error::new(std::io::ErrorKind::InvalidInput,
                "Choose a non-empty graph name without path separators, control characters or <>:\"/\\|?*. Names cannot be . or .., end with a dot or space, or use a Windows device name (CON, NUL, COM1, etc.).")));
        }
        let root = parent.join(name.trim());
        match fs::create_dir(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&root).map_err(|error| failed(&root, error))?;
                if !metadata.is_dir()
                    || fs::read_dir(&root)
                        .map_err(|error| failed(&root, error))?
                        .next()
                        .transpose()
                        .map_err(|error| failed(&root, error))?
                        .is_some()
                {
                    return Err(failed(&root, std::io::Error::new(std::io::ErrorKind::AlreadyExists,
                        "This graph folder already exists and is not empty (or is not a directory). Choose another name.")));
                }
            }
            Err(error) => return Err(failed(&root, error)),
        }
        for area in ["logseq", "pages", "journals", "assets"] {
            let dir = root.join(area);
            fs::create_dir(&dir).map_err(|error| failed(&dir, error))?;
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
                Area::Trash | Area::Graph => {
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
    /// Cost: one canonicalization (O(path components) on its fallback) and a
    /// metadata check. Missing, linked-fallback or non-directory paths fail.
    pub fn canonical_root(root: &Path) -> Result<PathBuf, OpenError> {
        let canonical = canonical_existing_path(root).map_err(|error| OpenError::Unresolvable {
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
    /// Returns the store, graph metadata, and effective config. Lists journal
    /// identities before returning (O(J) file metadata); ordinary page titles
    /// are discovered by background parsing before graph-wide reads return.
    /// An unreadable subtree is omitted and later reported as unreadable; an exact
    /// destination guard cannot detect every unseen same-name claimant.
    /// A caller that applies the configured journal template must wait for
    /// `WholeGraph::templates()`; saving a new journal does not add it.
    /// [`Self::whole_graph`] waits for background parsing. Partial scans report
    /// unreadable entries; [`Self::refresh`] retries failed parsing.
    /// Direct reads/writes remain available after a parse failure, without
    /// publishing until `refresh()` succeeds. The returned
    /// `GraphMeta` is a snapshot of open-time settings. After a config change,
    /// callers can derive fresh display metadata with
    /// `GraphMeta::from_config` and `JournalFormat::new` from `Store::config()`;
    /// reopening also refreshes it but restarts this store's revision sequence.
    /// External observations during loading publish after the initial parse.
    /// On failed initial load, the watcher also defers external publication
    /// until `refresh()` successfully retries. Recovery's completion
    /// publication has no file tuples; reconciliation may publish observed
    /// differences separately. Use the recovered view to refresh graph-wide answers.
    /// Unsafe layouts, unapproved external targets, and I/O return [`OpenError`].
    /// With [`OpenOptions::launch_checkpoint`] (ADR 0070,
    /// `store/checkpoint.rs`), a valid checkpoint is served at once while the
    /// launch diff reconciles, and a background publisher replaces it after
    /// edits.
    pub fn open(
        root: &Path,
        opts: OpenOptions,
    ) -> Result<(Self, tine_core::model::GraphMeta, ConfigState), OpenError> {
        let checkpoint = opts.launch_checkpoint.clone();
        let root = Self::canonical_root(root)?;
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
        let journals = graph.scan_journal_names();
        let journal_ids = journal_ids_from_entries(&graph, &journals);
        let problem = graph.config_read_problem.clone();
        let graph = Arc::new(graph);
        let config = ConfigState::of(&graph, Arc::new(graph.config.clone()), problem);
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
            Arc::clone(&journal_ids),
        ));
        graph.diag.open_done();
        let watch = crate::watch::WatchHandle::start(
            Arc::clone(&graph),
            Arc::clone(&writer),
            Arc::clone(&load),
            Arc::clone(&changes),
            Arc::clone(&journal_ids),
            Arc::clone(&config_state),
            opts.watch,
            crate::watch::Baseline::FromLoad,
        );
        let worker_graph = Arc::clone(&graph);
        let worker_load = Arc::clone(&load);
        let worker_writer = Arc::clone(&writer);
        let worker_changes = Arc::clone(&changes);
        let worker_watch = watch.core_for_load();
        let worker_watch_wake = watch.wake_for_load();
        if let Some(path) = &checkpoint {
            checkpoint::Publisher::start(path, &graph, &writer, &load, &changes, &watch);
        }
        std::thread::Builder::new()
            .name("tine-graph-load".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                #[cfg(any(test, feature = "test-faults"))]
                while worker_graph.root.join(".tine-test-pause-load").exists()
                    && !worker_load.cancelled.load(Ordering::Acquire)
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let parts = (
                    &worker_graph,
                    &*worker_load,
                    &*worker_writer,
                    &*worker_changes,
                );
                if checkpoint.as_deref().is_some_and(|path| {
                    checkpoint::launch_from(path, parts, &worker_watch, &worker_watch_wake)
                }) {
                    return;
                }
                let completed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                    let completed = worker_graph
                        .warm_cache_cancellable(|| worker_load.cancelled.load(Ordering::Acquire));
                    if completed || worker_load.cancelled.load(Ordering::Acquire) {
                        break completed;
                    }
                }));
                let _writer = worker_writer.lock().unwrap();
                let mut found = crate::watch::Deferred::default();
                if matches!(completed, Ok(true)) {
                    // GH #623 / storage spec §5.1: the baseline is what the
                    // load pass observed, then one full stat diff catches
                    // every file that changed since its stamp was taken.
                    let written = match worker_graph.take_launch_observations() {
                        Some(mut observed) => {
                            let written = std::mem::take(&mut observed.announce);
                            worker_watch.install_launch_baseline(observed);
                            written
                        }
                        None => {
                            worker_watch.install_walked_baseline();
                            Vec::new()
                        }
                    };
                    match worker_watch.launch_diff() {
                        Ok(mut deferred) => {
                            worker_watch.announce_written_since_open(written, &mut deferred);
                            found = deferred;
                        }
                        // The root vanished or the store closed: the graph
                        // still opens on what was read; the next watcher
                        // cycle or rescan reconciles (an external-editor or
                        // sync race, not a reason to refuse the graph).
                        Err(_) => {}
                    }
                }
                if matches!(completed, Ok(true)) {
                    if matches!(*worker_load.status.lock().unwrap(), LoadStatus::Loading) {
                        let publish_began = std::time::Instant::now();
                        worker_changes.publish_with(
                            Origin::External,
                            found.files,
                            found.config_changed,
                            found.pages,
                            || *worker_load.status.lock().unwrap() = LoadStatus::Ready,
                        );
                        worker_graph.diag.ready(publish_began);
                        let _ = worker_watch_wake.send(());
                    }
                } else {
                    worker_graph.diag.load_stopped();
                    let mut status = worker_load.status.lock().unwrap();
                    if matches!(*status, LoadStatus::Loading) {
                        *status = LoadStatus::Failed("background graph load stopped".into());
                    }
                }
                worker_load.ready.notify_all();
            })
            .expect("spawn graph load worker");
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

    /// Current graph configuration: the one answer to "what is the config"
    /// (I-12). Does not wait for the graph parse, but may wait for a
    /// concurrent configuration update and its O(P + B) reparse. An external
    /// `logseq/config.edn` edit is taken in by the watcher cycle that sees it
    /// (or `refresh()`) when its bytes changed; one that names a page or
    /// journal directory escaping the graph is not taken in, and this keeps
    /// answering the last good config. Own config writes and restore reload
    /// before publication. The store accepts raw config EDN through
    /// `Transaction::replace`; it does not provide a config serializer.
    pub fn config(&self) -> ConfigState {
        self.config_state.read().unwrap().clone()
    }

    /// Test oracle: whether graph-wide answers are ready, without waiting
    /// for parsing or recovery. Production asks domain questions instead
    /// (`page_named` routes on readiness internally). `Ok(false)` means still loading; `Err(Failed)` means `page()`
    /// can still read files but graph publication waits for `refresh()`;
    /// `Err(Closed)` means this store is closed.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn is_graph_ready(&self) -> Result<bool, LoadError> {
        match &*self.load.status.lock().unwrap() {
            LoadStatus::Loading => Ok(false),
            LoadStatus::Ready => Ok(true),
            LoadStatus::Failed(reason) => Err(LoadError::Failed {
                reason: reason.clone(),
            }),
            LoadStatus::Closed => Err(LoadError::Closed),
        }
    }

    /// Stop observation, wait for an in-flight writer, end the subscription,
    /// release load waiters, and refuse later I/O. Idempotent; no timeout.
    pub fn close(&self) {
        self.changes
            .checkpoint
            .get()
            .inspect(|signal| signal.stop());
        self.watch.stop();
        let _writer = self.writer.lock().unwrap();
        self.load.closed.store(true, Ordering::Release);
        self.load.cancelled.store(true, Ordering::Release);
        *self.load.status.lock().unwrap() = LoadStatus::Closed;
        self.load.ready.notify_all();
        self.changes.close();
    }

    /// Start the sole app-wide change stream without replay. A second
    /// call replaces the first subscriber, which then receives `Displaced`, and
    /// discards queued changes. The queue is unbounded and also accumulates
    /// publications before any subscription exists; consumers must drain it.
    /// To avoid a subscription gap,
    /// subscribe first, then acquire `whole_graph()` and ignore changes with
    /// `graph_rev <= view.rev()`. Initial load completion is an
    /// `Origin::External` publication with no file tuples; a during-load save
    /// has a separate `Origin::Own` publication once a complete snapshot is
    /// available. Writer serialization orders these publications; whichever
    /// comes first has a view containing the save.
    /// A failed load is returned by `whole_graph()`, but no `Change` announces it;
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
            start: GraphRev(state.rev),
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
    /// then reconcile page, journal and config files at `depth`. Normal
    /// runtime calls also reconcile configured asset metadata (assets are
    /// metadata-only by contract at every depth); failed-load recovery leaves
    /// asset catch-up to the watcher.
    ///
    /// [`Depth::Stamps`] considers a graph-text file unchanged when its
    /// modification time and length both match the previous scan, so
    /// same-length edits with preserved timestamps can be missed. Cost O(P
    /// metadata + bytes of files detected as changed + config bytes hashed +
    /// asset metadata), plus load wait. Unchanged-config runtime graph-text
    /// walks run off-writer and retry on change. [`Depth::Bytes`] hashes
    /// every graph-text file instead: O(graph bytes). [`Depth::Rebuild`] does
    /// that and then re-reads and re-parses every file through the cold-launch
    /// build, replacing the page cache, the name index and the read evaluator
    /// (O(graph bytes) twice plus the cold-launch parse); it runs on the
    /// caller's thread, so call it from a worker. An open editor keeps its
    /// base revision, so a save made over a file a refresh found changed is
    /// refused like any other stale save.
    ///
    /// Config changes enumerate and reparse O(P + B) pages/blocks under the
    /// writer before publication. Recovery from a failed initial load parses
    /// the whole graph synchronously before reconciliation, at every depth
    /// (already a cold build). A concurrent edit can make that recovery parse
    /// fail; retry with another explicit call after edits settle. A
    /// previously unreadable subtree is retried on this full scan; newly
    /// accessible files can then enter the resulting view. After a failed
    /// initial load, a successful retry publishes a fresh completion
    /// generation with no file tuples even if no file changed during
    /// reconciliation or an older snapshot exists. Own writes made while
    /// failed update the watcher baseline and enter the recovered view
    /// without being relabeled as External file changes.
    /// Returns `LoadError::Closed` after close or `LoadError::Failed` for a
    /// lost root or unsafe config layout, such as a configured page directory
    /// that escapes the graph through a symlink. A failed initial load is
    /// retried only by this explicit call; watcher ticks do not retry it. A
    /// continuous stream of edits can keep the initial background parse in
    /// `Loading` indefinitely. A failed refresh after an earlier successful
    /// load returns an error but leaves the last published `WholeGraph` view
    /// available.
    ///
    /// Returns the change-feed revision through which this refresh's changes
    /// are published: a subscriber that has received it has received them all.
    pub fn refresh(&self, depth: Depth) -> Result<GraphRev, LoadError> {
        match depth {
            Depth::Stamps => self.watch.scan_refresh()?,
            Depth::Bytes => self
                .watch
                .scan_refresh_with(crate::launch_diag::DiffTrigger::Bytes)?,
            Depth::Rebuild => {
                self.watch.rebuild_all()?;
                self.changes
                    .checkpoint
                    .get()
                    .inspect(|signal| signal.request());
            }
        }
        Ok(self.changes.rev())
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.load.closed.load(Ordering::Acquire)
    }

    pub(crate) fn refresh_journal_ids(&self) {
        let found = journal_ids_from_entries(&self.graph, self.graph.list_pages_shared().as_ref());
        *self.journal_ids.lock().unwrap() = found;
    }

    /// Return a cached canonical journal id or propose a configured path (directory,
    /// filename format, preferred extension). The day index covers accessible files
    /// before open and refreshes before relevant publications: direct reads/writes,
    /// transactions, restore, watcher and refresh. Cost O(format + path bytes);
    /// no parse or disk read. Unreadable subtrees or unobserved external creations
    /// can yield a duplicate-day proposal. It does not indicate existence: `page(id)`
    /// may return `NotFound`. Invalid `Day` is not rejected and may yield nonsense.
    /// Use guarded `CreateNew` saves; handle conflicts, twins, I/O and target-safety
    /// refusals, including unreadable destination directories.
    pub fn journal_id(&self, day: Day) -> PageId {
        let date = JournalDate::from_ordinal(day.0);
        if let Some(id) = self.journal_ids.lock().unwrap().get(&day) {
            return id.clone();
        }
        proposed_journal_id(
            &self.graph.current_config(),
            self.graph.current_journal_format().file_stem(date),
        )
    }
    /// Count entries and bytes by kind in graph trash. `scan_area(Area::Trash)`
    /// can list files and `move_file` can move one to a live area with a guard,
    /// but there is no dedicated untrash workflow or recovery-root import.
    /// Only asset trash has an in-API purge; other recovery and trash entries
    /// accumulate until managed outside this API. Graph-side and asset-sidecar
    /// restore recovery roots count as `Legacy` with their contained bytes.
    /// Any stat or read error fails the call. Cost O(trash entries and files
    /// inside trashed directories). Each tuple is `(kind, count, bytes)`.
    pub fn trash_stats(&self) -> Result<Vec<(TrashKind, u64, u64)>, StoreError> {
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
        let trash = trash_root(&self.graph.root);
        let mut counts = [(0, 0); 5];
        let entries = match fs::read_dir(trash) {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(StoreError::from_io(error)),
        };
        for entry in entries.into_iter().flatten() {
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
                    let bytes = trash_entry_bytes(&entry.path()).map_err(StoreError::from_io)?;
                    add_trash_count(&mut counts, TrashKind::Legacy, bytes);
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
        let asset_recovery = self.graph.assets_path().join(".tine-restore-recovery");
        match fs::read_dir(asset_recovery) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(StoreError::from_io)?;
                    let bytes = trash_entry_bytes(&entry.path()).map_err(StoreError::from_io)?;
                    add_trash_count(&mut counts, TrashKind::Legacy, bytes);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::from_io(error)),
        }
        Ok(trash_counts(counts))
    }

    /// Permanently remove typed asset entries and loose legacy entries whose
    /// filenames classify as assets,
    /// including directories. Legacy pages and other kinds stay recoverable.
    /// On error, returns completed top-level entry count and all bytes already
    /// deleted, including bytes removed from a directory that was only partly
    /// purged. A partly purged directory is not counted as a completed entry.
    /// Cost: O(asset trash entries + bytes). The success tuple is
    /// `(completed entry count, deleted bytes)`; an error adds those two
    /// partial counts after the error value.
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
            Area::Graph => return self.graph_text_file_id(rel),
        };
        let id = FileId::from(format!("{directory}/{rel}"));
        self.validate_file(&id)?;
        Ok(id)
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
        #[cfg(feature = "test-faults")]
        crate::cost_counters::store_read();
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
        if let Some(limit) = max_bytes {
            input
                .take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(StoreError::from_io)?;
        } else {
            input.read_to_end(&mut bytes).map_err(StoreError::from_io)?;
        }
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
            return Err(StoreError::StreamSymlink(file.clone()));
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
    /// Results carry relative file paths, not decoded page display names;
    /// there is no page-name listing API while the initial graph load is failed.
    /// Stat/list failures, non-UTF-8 names and invalid file ids appear in
    /// `unreadable`; backup must refuse an incomplete listing. A failure on
    /// the starting directory also appears there while the call returns `Ok`.
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
                let live = match canonical_existing_path(&self.graph.root.join("assets")) {
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
            Area::Graph => self.graph.root.clone(),
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
        // One directory per call; subdirectories are queued, not recursed into,
        // so a delivered tree of any depth costs heap, not stack (I-22).
        fn walk(
            store: &Store,
            area: Area,
            root: &Path,
            dir: &Path,
            out: &mut Listing,
            pending: &mut Vec<PathBuf>,
        ) {
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
                    let rel = dir
                        .strip_prefix(root)
                        .unwrap_or(dir)
                        .to_string_lossy()
                        .into_owned();
                    #[cfg(windows)]
                    let rel = rel.replace('\\', "/");
                    out.unreadable.push((rel, error.into()));
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
                let Some(name) = name.to_str() else {
                    if area == Area::Graph
                        && !crate::file_kind::is_graph_text_path(Path::new(&name))
                    {
                        continue;
                    }
                    let path = entry.path();
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    #[cfg(windows)]
                    let rel = rel.replace('\\', "/");
                    out.unreadable.push((
                        rel,
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "non-UTF-8 file name")
                            .into(),
                    ));
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                #[cfg(windows)]
                let rel = rel.replace('\\', "/");
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
                if area == Area::Graph && !store.graph_text_listed(&rel, ty.is_dir()) {
                    continue;
                }
                if ty.is_dir() {
                    if area == Area::Graph || store.file_id(area, &rel).is_ok() {
                        pending.push(path);
                    } else {
                        out.unreadable.push((
                            rel,
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "invalid directory id",
                            )
                            .into(),
                        ));
                    }
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
                                    id,
                                    area,
                                    rel,
                                    meta: Some(FileMeta {
                                        len: meta.len(),
                                        mtime: meta.modified().ok(),
                                    }),
                                });
                            } else {
                                out.unreadable.push((
                                    rel,
                                    std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "invalid file id",
                                    )
                                    .into(),
                                ));
                            }
                        }
                        Err(error) => out.unreadable.push((rel, error.into())),
                    }
                }
            }
        }
        match fs::symlink_metadata(&start) {
            Ok(_) => {
                let mut pending = vec![start];
                while let Some(dir) = pending.pop() {
                    walk(self, area, &root, &dir, &mut listing, &mut pending);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => listing
                .unreadable
                .push((under.unwrap_or("").to_owned(), error.into())),
        }
        listing.files.sort_by(|a, b| a.rel.cmp(&b.rel));
        listing.unreadable.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(listing)
    }

    /// Read one physical page. `NotFound` alone means absence and permits an
    /// empty editable page; unsafe, parse, size, I/O and closed-store errors
    /// must surface as failures. From Ready on, every call holds the writer
    /// lock, even a miss. A new external file publishes `Origin::External`/`Created`; a changed
    /// known file publishes `Modified`. Equal bytes do not publish again.
    /// A readable duplicate-day stray is returned but never published or
    /// added to graph-wide answers. Missing files do not wait for initial
    /// parsing; present files remain directly readable after a failed initial
    /// parse, with publication deferred until successful `refresh()`.
    /// An accessible path below an unreadable directory is read directly.
    /// Observed edits may wait for parsing, another writer, restore or site
    /// publication. Case aliases return the file's actual disk spelling.
    /// Canonicality is answered from the published name index while it
    /// describes the current cache generation, so a read opens no other file
    /// (GH #623 BR3: rebuilding the live name index read every page preamble
    /// under the writer, ~2.8 s on Windows, and queued every other open
    /// behind it); a journal, or a stale publication, still asks the live
    /// index. Before Ready (initial parse, or a launch checkpoint served
    /// while the launch diff runs) a read takes no writer and publishes
    /// nothing: it parses the file directly, the revision comes from the
    /// same bytes, and the load or launch diff takes the file in, so a page
    /// opens in parse time whatever the graph is doing. Target parse costs
    /// O(bytes + blocks); publication adds O(P) metadata. Reads write no
    /// page bytes. A page a page host holds answers the bytes the host last
    /// indexed and publishes nothing (A-V4): a write parses the bytes its
    /// revision guards with [`Store::page_of`] instead.
    pub fn page(&self, id: &PageId) -> Result<PageRead, StoreError> {
        if matches!(*self.load.status.lock().unwrap(), LoadStatus::Loading) {
            if self.is_closed() {
                return Err(StoreError::Closed);
            }
            let (id, path, entry) = self.page_target(id)?;
            let doc = self.parse_page(&path, &entry, false)?;
            return self.page_read(id, doc);
        }
        let waiting = Instant::now();
        let _writer = self.writer.lock().unwrap();
        self.graph.diag.page_writer_wait(waiting.elapsed());
        let before_generation = self.graph.cache_generation();
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
        let (id, path, entry) = self.page_target(id)?;
        let canonical = self.canonical_claim(&entry);
        let read = self.page_read(id, self.parse_page(&path, &entry, canonical)?)?;
        if self.graph.cache_generation() != before_generation
            && !matches!(*self.load.status.lock().unwrap(), LoadStatus::Failed(_))
        {
            self.publish_observed(&read, &path, entry);
        }
        Ok(read)
    }

    /// Resolve a page name (or alias) and read its current file: the one
    /// page-by-name door (GH #623). Cost O(index lookup + page bytes).
    ///
    /// Before the graph is ready, a name some file claims opens from that
    /// file without waiting for the index (the file-name listing below):
    /// claimant order ranks a file named for the page above any `title::`
    /// claimant, so this is the page the index would pick. A name only the
    /// index can resolve (an alias) or one no file claims waits for the
    /// graph, so a name the index could still resolve never reads as absent.
    /// Once ready, the published index resolves the name; an alias reads its
    /// first owner. After a failed initial parse, the file-name listing
    /// answers. A closed store returns `StoreError::Closed`. A resolved file
    /// that has disappeared reads as `None`. It writes no page bytes.
    pub fn page_named(&self, name: &str, kind: PageKind) -> Result<Option<PageRead>, StoreError> {
        if matches!(*self.load.status.lock().unwrap(), LoadStatus::Loading) {
            if let Some(read) = self.page_named_from_files(name, kind)? {
                return Ok(Some(read));
            }
        }
        let resolved = match self.whole_graph() {
            Ok(view) => view.resolve(name, kind == PageKind::Journal),
            Err(LoadError::Failed { .. }) => return self.page_named_from_files(name, kind),
            Err(LoadError::Closed) => return Err(StoreError::Closed),
        };
        // `resolve` builds an alias only with at least one owner.
        let id = match resolved {
            Resolved::Existing { id, .. } => id,
            Resolved::Alias { owners } => match owners.into_iter().next() {
                Some(id) => id,
                None => return Ok(None),
            },
            Resolved::Absent { .. } => return Ok(None),
        };
        match self.page(&id) {
            Ok(read) => Ok(Some(read)),
            Err(StoreError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The file-name listing answer: read an ordinary page by effective name, or a journal by its file
    /// stem or parseable display title, without waiting for the initial
    /// parse. A page some file is named for is found from a listing of file
    /// names and the matching files; otherwise the first lookup or
    /// invalidation builds a live O(P) file/preamble index, seeing files
    /// created after `open`, and warm lookups reuse it. This remains available after a
    /// failed initial parse; aliases require a successful graph view and are
    /// not resolved here. When files claim the same name or journal day, it
    /// uses the same claimant ranking as `WholeGraph::resolve` (canonical
    /// date-stem journal first, then Markdown before Org). A missing name
    /// returns `None`, also while a file's name is undecodable: that file is
    /// listed in `unreadable_files()` and one bad file never blocks the graph.
    /// The selected file is read through `page()`, with the same safety, revision, and publication
    /// rules, including lock wait and read errors. It writes no page bytes.
    fn page_named_from_files(
        &self,
        name: &str,
        kind: PageKind,
    ) -> Result<Option<PageRead>, StoreError> {
        let lookup = if kind == PageKind::Journal {
            self.graph
                .current_journal_format()
                .parse(name)
                .map(|date| self.graph.current_journal_format().title(date))
                .unwrap_or_else(|| name.to_owned())
        } else {
            name.to_owned()
        };
        let Some(entry) = self.graph.find_entry_named_file_first(&lookup, kind) else {
            return Ok(None);
        };
        let id = PageId::from(self.graph.rel_path(&entry.path));
        match self.page(&id) {
            Ok(read) => Ok(Some(read)),
            Err(StoreError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Wait without a timeout for the initial parse (or close), then acquire
    /// the current stable publication. Acquisition and clone are O(1) after
    /// the wait; a held view never waits for later writes and its answers do
    /// not change. After a failed parse,
    /// later calls return `LoadError::Failed` without another parse attempt
    /// until `refresh()` retries it. Call this from a worker thread to
    /// wait.
    /// The initial load restarts if an external edit invalidates a parse pass;
    /// continuous edits can keep this wait open indefinitely.
    /// An own save during loading can build the first cache synchronously for
    /// its publication; the background worker then uses that cache rather
    /// than restarting the pass. The save can wait for that O(P + B) build.
    /// No partial graph generation is available after failure.
    /// A launch checkpoint (ADR 0070) is served before Ready, while the launch
    /// diff reconciles edits made while closed; an operation acting on the
    /// whole graph's answers waits for Ready first (`refresh` does;
    /// inside the crate, `whole_graph_reconciled`).
    pub fn whole_graph(&self) -> Result<WholeGraph, LoadError> {
        self.whole_graph_when(true)
    }

    /// [`Self::whole_graph`] that never serves the launch checkpoint before
    /// the launch diff: graph-wide destructive operations (delete, rename,
    /// merge, orphan cleanup) wait for it, since an external edit made while
    /// Tine was closed may have added a reference the checkpoint lacks.
    pub(crate) fn whole_graph_reconciled(&self) -> Result<WholeGraph, LoadError> {
        self.whole_graph_when(false)
    }

    fn whole_graph_when(&self, serve_loaded: bool) -> Result<WholeGraph, LoadError> {
        let mut status = self.load.status.lock().unwrap();
        while matches!(*status, LoadStatus::Loading)
            && !(serve_loaded && self.load.serving.load(Ordering::Acquire))
        {
            status = self.load.ready.wait(status).unwrap();
        }
        match &*status {
            LoadStatus::Closed => return Err(LoadError::Closed),
            LoadStatus::Failed(reason) => {
                return Err(LoadError::Failed {
                    reason: reason.clone(),
                })
            }
            LoadStatus::Ready | LoadStatus::Loading => {}
        }
        let snapshot = self
            .changes
            .snapshot
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .ok_or_else(|| LoadError::Failed {
                reason: "ready store has no graph snapshot".into(),
            })?;
        Ok(WholeGraph {
            graph: Arc::clone(&snapshot.graph),
            rev: snapshot.rev,
            unreadable: Arc::clone(&snapshot.unreadable),
            config: snapshot.config.clone(),
            journal_format: snapshot.journal_format.clone(),
            list: Arc::clone(&snapshot.list),
            claimants: Arc::clone(&snapshot.claimants),
            _snapshot: snapshot,
        })
    }
}

/// Graph area used with an area-relative input to form a graph-root-relative
/// file identity.
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
    /// Whole graph root limited to graph text (`graph_text_relative_eligible`);
    /// `rel` is graph-relative. Backup and restore use it (og-B).
    Graph,
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
    /// A selected page source is not a regular file or escapes its graph area.
    PageSource(String),
    /// A selected streaming asset is a symlink.
    StreamSymlink(FileId),
    /// Page bytes are invalid UTF-8, or page parse validation rejected source
    /// nesting deeper than 512 levels.
    Undecodable,
    /// A page parser panicked; message describes the caught failure. Ordinary
    /// malformed text usually remains parseable, while depth and byte limits
    /// use `Undecodable` and `TooLarge`.
    Unparseable(String),
    /// A bounded read exceeded its limit: either the caller's byte limit or
    /// the store's 64 MiB page parse limit.
    TooLarge {
        /// Applied maximum length.
        limit: u64,
        /// Observed file length.
        len: u64,
    },
    /// Other filesystem failure.
    Io(crate::IoError),
    /// Store was closed before this disk operation.
    Closed,
}

impl StoreError {
    pub(crate) fn from_io(error: std::io::Error) -> Self {
        if let Some(too_large) = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<crate::model::ParseInputTooLarge>())
        {
            return Self::TooLarge {
                limit: crate::model::PARSE_INPUT_MAX_BYTES,
                len: too_large.len,
            };
        }
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            _ => Self::Io(error.into()),
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
    /// The revision of exact bytes (FNV-1a-64 hex), as a guarded write compares it.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(format!("{:016x}", fnv_update(0xcbf2_9ce4_8422_2325, bytes)))
    }

    pub(crate) fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        #[cfg(feature = "test-faults")]
        if crate::file_kind::is_graph_text_path(path) {
            crate::cost_counters::hash_read();
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buf)?;
            if count == 0 {
                break;
            }
            hash = fnv_update(hash, &buf[..count]);
        }
        Ok(Self(format!("{hash:016x}")))
    }
}

fn fnv_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
#[test]
fn graph_content_and_raw_file_revisions_share_fnv_bytes() {
    let text = "a Unicode page 🐈\n";
    assert_eq!(
        crate::model::content_rev(text),
        String::from(FileRev::from_bytes(text.as_bytes()))
    );
    assert_ne!(
        FileRev::from_bytes(b"\xff"),
        FileRev::from_bytes("ÿ".as_bytes())
    );
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
    /// As `Existing`, for the one save that resolves VCS merge conflict markers the
    /// file carries (R-VCS-MARKERS); commit first stages the old bytes in conflict trash.
    ResolvingMarkers(FileRev),
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
    /// Existing base was requested, but the file disappeared. Retrying as `CreateNew` would
    /// recreate a page another device may have deleted. Keep the unsaved buffer; for a separate
    /// copy, choose a new name and use guarded `CreateNew` after checking its proposed id.
    Deleted,
    /// Existing page cannot be safely rewritten: the Org round-trip editability refusal, or
    /// unresolved VCS conflict markers (R-VCS-MARKERS). The reason is for display.
    ReadOnly(String),
    /// Another file claims the page name or journal day. Creation and moves check this; an
    /// ordinary guarded save to either existing claimant is allowed when its own revision guard
    /// and safety checks pass. If a twin appears after a `CreateNew` write, commit withdraws its
    /// newly written bytes during undo before returning this result; the caller must retain its DTO.
    Twin {
        /// Existing claimant.
        existing: PageId,
    },
    /// The same file was named twice in one page-save request.
    Repeated,
    /// A new page was refused because a graph-text file Tine cannot read
    /// could already be that page (R-CREATE-UNREADABLE-OWNER). Keep the
    /// buffer; the user repairs or moves `file`, then saves again.
    UnreadableOwner {
        /// The unreadable file or directory, graph-relative.
        file: crate::FileId,
    },
    /// Invalid or unsafe target or page content; reason is for display.
    InvalidTarget(String),
    /// Filesystem operation failed.
    Io(crate::IoError),
    /// Store was closed before the save.
    Closed,
    /// A `PageDto` with `guide: true` is refused before disk access, regardless
    /// of the supplied page id.
    GuideEphemeral,
}

impl SaveOutcome {
    fn from_failed_step(why: crate::Why, id: &PageId) -> Self {
        match why {
            crate::Why::Conflict {
                file,
                disk: Some(_),
            } if file != id.file() => SaveOutcome::Twin {
                existing: PageId::from(file.as_str()),
            },
            crate::Why::Conflict {
                disk: Some(disk), ..
            } => SaveOutcome::Conflict { disk },
            crate::Why::Conflict { disk: None, .. } => SaveOutcome::Deleted,
            crate::Why::Refused(crate::Refusal::ReadOnly(reason)) => SaveOutcome::ReadOnly(reason),
            crate::Why::Refused(crate::Refusal::Twin { existing }) => {
                SaveOutcome::Twin { existing }
            }
            crate::Why::Refused(crate::Refusal::InvalidTarget(reason)) => {
                SaveOutcome::InvalidTarget(reason)
            }
            crate::Why::Refused(crate::Refusal::Closed) => SaveOutcome::Closed,
            crate::Why::Refused(crate::Refusal::UnreadableOwner { file }) => {
                SaveOutcome::UnreadableOwner { file }
            }
            crate::Why::Refused(crate::Refusal::RepeatedFile(_)) => SaveOutcome::Repeated,
            // Only an orphan-only asset trash raises this; a page save never
            // does, so it reports as an I/O refusal rather than a page family.
            crate::Why::Refused(crate::Refusal::AssetReferenced) => SaveOutcome::Io(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "asset is referenced").into(),
            ),
            crate::Why::Refused(crate::Refusal::Undecodable) => {
                SaveOutcome::InvalidTarget("undecodable page".into())
            }
            crate::Why::Failed(error) => SaveOutcome::Io(error),
        }
    }
}

/// Result of one ordered page-save request.
#[derive(Debug)]
pub enum SavePagesOutcome {
    /// One result per input entry, in the same order.
    Ok {
        /// One Saved or Unchanged revision per entry.
        outcomes: Vec<SaveOutcome>,
        /// The save's published answer delta, or None for unchanged/unpublished saves.
        change: Option<Change>,
    },
    /// The failed entry and any entries whose rollback could not restore disk.
    Failed {
        /// Zero-based failed input entry.
        index: usize,
        /// Refusal or I/O failure for that entry.
        outcome: SaveOutcome,
        /// Graph-relative file locations whose earlier bytes could not be restored.
        undo_failed: Vec<FileId>,
        /// Graph-relative locations omitted from the final publication.
        publication_errors: Vec<FileId>,
    },
}

/// Parsed page and the raw-byte revision used for a guarded save.
pub struct PageRead {
    /// Exact disk path identity. A case-alias request on a case-insensitive
    /// volume returns the file's disk spelling.
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
    /// removing it can reveal another claimant with different content. In the
    /// next published view that claimant becomes the canonical participant in
    /// references, backlinks, and search. A
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

/// Which names a [`WholeGraph::inventory`] lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryScope<'a> {
    /// Every name: file-claimed names, aliases and reference-only names.
    All,
    /// Only the file-claimed names whose page key is this key or lies under
    /// `key/` (a page and its namespace descendants): what a rename moves.
    FilesAtOrUnder(&'a str),
}
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
    /// Page membership: names/aliases, contained block text, or both. Defaults
    /// to names when absent; a file-scoped request remains block-only.
    pub page_match_scope: Option<tine_core::query::ir::FriendlyPageMatchScope>,
    /// Effective Pages Display view; sort and sample apply before admission.
    pub page_view: Option<tine_core::query::ir::ViewSettings>,
    /// Effective Blocks Display view, independent of the Pages section.
    pub block_view: Option<tine_core::query::ir::ViewSettings>,
}
/// Test oracle: syntax for [`WholeGraph::query`], the legacy block-group bridge.
/// Production queries run through [`WholeGraph::query_ir`].
#[cfg(any(test, feature = "test-faults"))]
pub enum QueryDialect {
    /// Simple query expression.
    Simple,
    /// Advanced query expression.
    Advanced,
}
/// One request to [`WholeGraph::query_ir`].
#[derive(Debug, Clone, Copy)]
pub enum IrRequest<'a> {
    /// Evaluate a query (`query_run`).
    Run {
        /// The parsed query.
        query: &'a tine_core::query::ir::Query,
        /// Its view settings (sort, sample, statistics).
        view: &'a tine_core::query::ir::ViewSettings,
        /// The page it renders on, if any.
        context: &'a tine_core::query::ir::ExecutionContext,
    },
    /// Explain an empty answer (`query_explain_empty`).
    ExplainEmpty {
        /// The parsed query.
        query: &'a tine_core::query::ir::Query,
        /// The page it renders on, if any.
        context: &'a tine_core::query::ir::ExecutionContext,
    },
    /// The observed property registry (`query_registry`).
    Registry,
}

/// The answer to one [`IrRequest`], in the same variant.
#[derive(Debug, Clone)]
pub enum IrAnswer {
    /// Rows, totals and statistics.
    Result(Box<tine_core::query::ir::QueryResult>),
    /// One explanation row per root conjunct, each with the conjunct's count
    /// alone and the count without it (one row, with no `without` count, for a
    /// non-`And` root); no rows for a non-executable query (its diagnostics
    /// and report say why).
    ExplainEmpty(tine_core::query::ir::ExplainEmptyResult),
    /// This generation's registry.
    Registry(Arc<tine_core::query::registry::Registry>),
}

/// Consume an IR answer into wire rows without cloning or I/O (O(1)). Refuses
/// exceeded results with store limits; other answer variants are unexpected.
impl TryFrom<IrAnswer> for tine_core::query::ir::QueryResult {
    type Error = String;
    fn try_from(answer: IrAnswer) -> Result<Self, Self::Error> {
        let IrAnswer::Result(result) = answer else {
            return Err("query-run: unexpected answer".into());
        };
        if result.exceeded {
            let complete = result.matched_total.unwrap_or(result.total);
            return Err(format!("result-too-large: {complete} matching rows; narrow the query or add a sample (construction limits: {RESULT_BRIDGE_MAX_ROWS} rows / {RESULT_BRIDGE_MAX_BYTES} bytes)"));
        }
        Ok(*result)
    }
}

/// Test oracle: answer shape of [`WholeGraph::query`].
#[cfg(any(test, feature = "test-faults"))]
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
    /// `WholeGraph::unreadable_files`. The initial load reports this state if
    /// its worker stops or panics; explicit refresh can also fail for a lost
    /// root or unsafe config layout. Continuous edits instead keep the load
    /// in `Loading` while its parse restarts. After an initial failure, `page()` can still
    /// read a present file and guarded saves can write, but neither publishes
    /// a graph generation until successful `refresh()` recovery.
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
    fn bridge_result_too_large(what: Budget, count: usize, bytes: Option<usize>) -> Self {
        Self::ResultTooLarge {
            what,
            count,
            limit: RESULT_BRIDGE_MAX_ROWS,
            bytes,
            byte_limit: RESULT_BRIDGE_MAX_BYTES,
        }
    }

    /// Check the serialized group estimate including transport fields before
    /// an adapter sends it. Ordinary `WholeGraph` callers already receive
    /// bounded results from the query methods.
    #[cfg(test)]
    pub fn bridge_matching_blocks(rows: usize, bytes: usize) -> Option<Self> {
        (rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES).then_some(
            Self::bridge_result_too_large(Budget::BridgeMatchingBlocks, rows, Some(bytes)),
        )
    }

    /// Check serialized search bytes including adapter fields; direct
    /// `WholeGraph::search` callers already receive bounded results.
    pub fn bridge_search_hits(hits: usize, bytes: usize) -> Option<Self> {
        (hits > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES).then_some(
            Self::bridge_result_too_large(Budget::SearchHits, hits, Some(bytes)),
        )
    }
}

#[cfg(test)]
mod bridge_matching_blocks_tests {
    use super::{Budget, QueryError, RESULT_BRIDGE_MAX_BYTES, RESULT_BRIDGE_MAX_ROWS};
    use tine_core::model::{ref_groups_estimated_bytes, BlockDto, PageKind, RefGroup};

    fn group(blocks: Vec<BlockDto>) -> RefGroup {
        RefGroup {
            page: "Budget".into(),
            kind: PageKind::Page,
            blocks,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn rejects_oversized_result_count_before_ipc() {
        let groups = [group(vec![BlockDto::default(); RESULT_BRIDGE_MAX_ROWS + 1])];
        let rows = groups.iter().map(|group| group.blocks.len()).sum();
        let bytes = ref_groups_estimated_bytes(&groups);
        assert!(matches!(
            QueryError::bridge_matching_blocks(rows, bytes),
            Some(QueryError::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count,
                limit,
                bytes: Some(estimated),
                byte_limit,
            }) if count == RESULT_BRIDGE_MAX_ROWS + 1
                && limit == RESULT_BRIDGE_MAX_ROWS
                && estimated == bytes
                && byte_limit == RESULT_BRIDGE_MAX_BYTES
        ));
    }

    #[test]
    fn rejects_oversized_result_bytes_before_ipc() {
        let mut block = BlockDto::default();
        block.raw = "x".repeat(RESULT_BRIDGE_MAX_BYTES + 1);
        let groups = [group(vec![block])];
        let rows = groups.iter().map(|group| group.blocks.len()).sum();
        let bytes = ref_groups_estimated_bytes(&groups);
        assert!(matches!(
            QueryError::bridge_matching_blocks(rows, bytes),
            Some(QueryError::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count,
                limit,
                bytes: Some(estimated),
                byte_limit,
            }) if count == rows
                && limit == RESULT_BRIDGE_MAX_ROWS
                && estimated == bytes
                && byte_limit == RESULT_BRIDGE_MAX_BYTES
        ));
    }
}

/// Caller-owned cancellation flag, checked by `search` and `find_blocks`
/// before each search page and block. Other graph queries do not take it.
pub struct Cancel(pub Arc<AtomicBool>);

/// Facet answer policy. The two variants use different inputs and budgets;
/// both can cost O(B) over the view's blocks.
pub enum FacetPolicy {
    /// Block properties only, excluding internal keys, with raw values.
    /// Refuses more than 20,000 items or 32 MiB.
    Budgeted,
    /// Autocomplete facets from blocks and page pre-blocks, applying hidden
    /// property settings and trimmed values. Returns up to 2,000 items or
    /// 2 MiB without a truncation indicator.
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
    unreadable: Arc<Vec<(FileId, String)>>,
    pub(crate) config: ConfigState,
    journal_format: Arc<JournalFormat>,
    pub(crate) list: Arc<EntryList>,
    claimants: Arc<SharedMap<(bool, String), Vec<PageEntry>>>,
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
    /// Page files and subdirectories unreadable in this view, with a
    /// displayable reason. Files unreadable on initial load are absent from
    /// graph-wide answers. If a previously read directory becomes unreadable,
    /// its last published pages remain in the view until access returns; no
    /// `Removed` event is inferred from a failed listing. Cost O(1).
    pub fn unreadable_files(&self) -> &[(FileId, String)] {
        &self.unreadable
    }

    /// Return an owned `Corpus` of pages in this view for evaluation. Cost
    /// O(P); parsed documents are shared while the result is held.
    // The evaluator receives a copy of the parsed-page table.
    pub fn corpus(&self) -> tine_core::Corpus {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::corpus();
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
                tine_core::asset_refs::collect_document_asset_refs(doc, &mut names);
            }
        });
        Arc::new(names)
    }

    /// Pages whose explicit references name any of `names` (page keys, compared
    /// after `refs::page_key`). Supply display names; this method normalizes
    /// them, so pre-normalized keys are not required. Explicit means OG's
    /// `:block/refs`: page refs, tags, and parser-recognized page references
    /// in property values such as `tags::`, plus `{{embed}}`. Property keys do
    /// not select this method's scope; the parsed value's reference syntax
    /// does. This can include a property reference that `RenameMap`
    /// does not rewrite; pass only supported rewrite names to that map. It
    /// excludes arguments of
    /// `{{query}}` or other macros, so a page that mentions a name only inside a
    /// query is not a referrer. The answer comes from this snapshot's parse, not
    /// from disk. Cost ranges from O(P + matching references) to O(P + B) over the
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

    /// Resolve a name in this captured view. The caller supplies whether the
    /// name is a journal title; using the wrong kind searches that other
    /// namespace and may propose a new file. Real files win over aliases.
    /// Journal files with a configured date stem rank first, then Markdown
    /// before Org, then filename and full path lexicographically. Ordinary
    /// page claimants prefer the file named for the page, then use the latter
    /// three rules. Ordinary page claims use a preamble `title::` or Org title
    /// directive when present, then the decoded filename. After a
    /// journal-format change, a new view uses the new title format for date
    /// claims; the parser still tries its documented fallback formats. Old
    /// custom-format links are not rewritten. If an old title no longer parses
    /// under the configured or fallback formats, resolving it as an ordinary
    /// page can propose a new page rather than the former journal. To classify a clicked journal
    /// title, use the current `JournalFormat::parse`, which tries configured
    /// formats and fallbacks, then pass that result as `is_journal`. Cost
    /// O(1) indexed lookup after the snapshot's one-time O(all aliases)
    /// alias-key build when no real file wins.
    pub fn resolve(&self, name: &str, is_journal: bool) -> Resolved {
        let kind = if is_journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let lookup = if is_journal {
            self.journal_format
                .parse(name)
                .map(|date| self.journal_format.title(date))
                .unwrap_or_else(|| name.to_owned())
        } else {
            name.to_owned()
        };
        let entries = self.name_claimants(&lookup, kind);
        if !entries.is_empty() {
            let mut ids = entries
                .iter()
                .map(|entry| entry.rel_path.clone().expect("file claimant has a path"));
            return Resolved::Existing {
                id: ids.next().unwrap(),
                others: ids.collect(),
            };
        }
        if !is_journal {
            let owners: Vec<_> = self
                .graph
                .alias_owner_paths(&tine_core::refs::page_key(name))
                .map(|paths| paths.iter().cloned().map(PageId::from).collect())
                .unwrap_or_default();
            if !owners.is_empty() {
                return Resolved::Alias { owners };
            }
        }
        let config = &self.config.config;
        if is_journal {
            let stem = self
                .journal_format
                .parse(name)
                .map(|date| self.journal_format.file_stem(date))
                .unwrap_or_else(|| name.to_owned());
            return Resolved::Absent {
                id: proposed_journal_id(config, stem),
            };
        }
        let dir = &config.pages_dir;
        let stem = tine_core::model::encode_page_name(name, config.file_name_format);
        Resolved::Absent {
            id: PageId::from(format!("{dir}/{stem}.{}", config.preferred_format.ext())),
        }
    }

    fn validated_page(&self, id: &PageId) -> Result<(), QueryError> {
        let path = id.as_str();
        let valid_area = crate::model::graph_text_relative_eligible(path, &self.config.config);
        let valid_name = !path.contains('\\')
            && !path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            && crate::file_kind::is_graph_text_path(Path::new(path))
            && Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| !tine_core::model::is_sync_conflict(stem));
        if !valid_area || !valid_name {
            return Err(QueryError::InvalidTarget(id.as_str().to_owned()));
        }
        Ok(())
    }

    /// Test oracle: the legacy simple/advanced bridge, kept to pin legacy-vs-IR
    /// parity and the bridge budgets in tests. Production runs every query
    /// through [`WholeGraph::query_ir`].
    ///
    /// Execute one simple or advanced query macro over this stable view.
    /// This entry carries no page context, so `:current-page` has no binding
    /// here; the IR path binds it through `ExecutionContext::on_page`. A simple
    /// source above 64 KiB or 128 parenthesis levels returns
    /// `QueryError::Parse`; advanced unsupported clauses appear in its
    /// diagnostics. Exceeding 20,000 rows or 32 MiB returns
    /// `QueryError::ResultTooLarge` without a partial answer. Query cost
    /// is O(P + B) graph-wide; exact page-name scopes visit only their owners
    /// unless property coercions or used-as-tag require graph-wide facts.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn query(&self, source: &str, dialect: QueryDialect) -> Result<QueryResult, QueryError> {
        if let Err(reason) = tine_core::query::admit_source(source) {
            return Err(QueryError::Parse(match reason {
                tine_core::query::SourceRefusal::TooLarge => format!(
                    "query exceeds {} bytes",
                    tine_core::query::QUERY_SOURCE_MAX_BYTES
                ),
                tine_core::query::SourceRefusal::TooDeep => {
                    "query nesting exceeds 128 levels".into()
                }
            }));
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
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err(QueryError::bridge_result_too_large(
                        Budget::AdvancedQueryMatches,
                        total,
                        None,
                    ))
                } else {
                    Ok(QueryResult::Advanced(result))
                }
            }
        }
    }

    /// The IR query front door (SPEC §7.1): `Run` binds the query to its
    /// context and today (§4.4) and evaluates it in memory under the fixed
    /// construction bounds, returning an over-bound answer with `exceeded` set
    /// (`TryFrom<IrAnswer>` refuses it for transport); `ExplainEmpty` counts each probe's rows
    /// without constructing any (N19); `Registry` is the observed property
    /// registry (§6.1). Only a statistics fold over its memory budget fails.
    ///
    /// Cost: graph-wide `Run`/`ExplainEmpty` is O(P + B) per plan/probe, memoized.
    /// Exact page-name scopes visit their owners unless global coercions/tag
    /// facts are needed. Registry build is O(P + B) once per generation, then shared.
    pub fn query_ir(
        &self,
        request: IrRequest<'_>,
    ) -> Result<IrAnswer, tine_core::query::statistics::StatisticsResourceLimit> {
        crate::query::exec::query_ir(&self.graph, request)
    }

    /// Graph search, with an optional syntactically checked file scope and
    /// caller cancellation. An absent scoped file yields an empty answer;
    /// check existence separately when it matters.
    /// The combined page and block hit allowance is 20,000. The page limit is
    /// clamped first, then the block limit to the remaining allowance; asking
    /// for 20,000 page hits leaves no block-hit allowance in an unscoped search.
    /// A file-scoped search returns only block hits and does not consume the
    /// allowance with `page_limit`. A cancelled call
    /// returns `QueryError::Cancelled`
    /// without a partial result. Inspect `QueryExecution::has_more` for
    /// omitted hits in enabled categories; a zero category limit is not
    /// checked. A successful result has `cancelled == false` through this
    /// API; the lower-level execution type also represents cancelled work.
    /// A cold search can scan O(P + B + text bytes); content page membership
    /// adds one block scan and keeps at most one candidate per matching physical
    /// page before applying the requested hit limits. An authored sort retains
    /// all matching candidates in memory, O(matches) space and O(matches log
    /// matches) ordering, before the independent section samples and bounds.
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
        let page_limit = if scope.is_some() {
            0
        } else {
            req.page_limit.min(RESULT_BRIDGE_MAX_ROWS)
        };
        let block_limit = req.block_limit.min(RESULT_BRIDGE_MAX_ROWS - page_limit);
        let result = self.graph.run_graph_search_latest_scoped(
            cancel,
            &req.text,
            page_limit,
            block_limit,
            scope,
            req.explain,
            req.page_match_scope
                .unwrap_or(tine_core::query::ir::FriendlyPageMatchScope::Names),
            req.page_view.clone(),
            req.block_view.clone(),
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
    /// result alone. Worst case O(B). Exceeding 20,000 rows or 32 MiB returns
    /// `QueryError::ResultTooLarge`, without a partial result or pagination.
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
    /// be routed directly to a page. More than 20,000 requested ids returns
    /// `ResultTooLarge` with `RequestedBlockRefs`; result data is also limited
    /// to 20,000 rows and 32 MiB.
    pub fn blocks(&self, uuids: &[String]) -> Result<Vec<Option<RefGroup>>, QueryError> {
        if uuids.len() > RESULT_BRIDGE_MAX_ROWS {
            return Err(QueryError::bridge_result_too_large(
                Budget::RequestedBlockRefs,
                uuids.len(),
                None,
            ));
        }
        let (groups, exceeded, total) = crate::query::resolve_blocks_bounded(
            &self.graph,
            uuids,
            RESULT_BRIDGE_MAX_ROWS,
            RESULT_BRIDGE_MAX_BYTES,
        );
        if exceeded {
            Err(QueryError::bridge_result_too_large(
                Budget::ResolvedBlockRows,
                total,
                None,
            ))
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
        let preview = crate::query::preview_block_with_budget(
            &self.graph,
            uuid,
            max_nodes.clamp(1, MAX_PREVIEW_NODES),
            PREVIEW_MAX_BYTES,
        );
        if let Some(value) = &preview {
            let groups = std::slice::from_ref(&value.group);
            let rows = groups.iter().map(|g| g.blocks.len()).sum::<usize>();
            let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
            if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
                return Err(QueryError::bridge_result_too_large(
                    Budget::BridgeMatchingBlocks,
                    rows,
                    Some(bytes),
                ));
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

    /// Counts keyed by persisted `id::` values. The first read materializes
    /// O(distinct referenced ids) entries; later reads clone that cached map.
    pub fn block_ref_counts(&self) -> Arc<HashMap<String, usize>> {
        self.graph.public_block_ref_counts()
    }

    /// `[[` completion over pages, journals, aliases and referenced names.
    /// A first call can traverse all page blocks and reference text to build
    /// indexes; later calls scan page, alias, and referenced-name candidates
    /// and rank or sort matches, returning at most `limit` entries. Once an
    /// index has been built, a later publication carries it forward by updating
    /// changed pages; the first completion after every save does not rescan B.
    pub fn complete_page_names(&self, text: &str, limit: usize) -> Vec<PageEntry> {
        crate::query::quick_switch(&self.graph, text, limit)
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
            Err(QueryError::bridge_result_too_large(
                Budget::BridgeMatchingBlocks,
                rows,
                Some(bytes),
            ))
        } else {
            Ok(groups)
        }
    }

    /// Selected query subtrees. Accepts at most 1,024 specs and 64 KiB total
    /// across keys and query sources, but evaluates only the first 64. The
    /// batch shares a 50-root, 2,000-node, and 8 MiB output budget; later
    /// results may omit roots or nodes even when small on their own.
    /// Inspect `omitted_queries`, `shown`, `total`, and `omitted_nodes`
    /// before treating `Ok` as complete. The final bridge-size check can
    /// return `ResultTooLarge` for the whole batch, with no partial `Ok`.
    /// Cost up to O(64 × B + selected nodes).
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

    /// Budgeted facets reject over 20,000 items or 32 MiB, reporting
    /// `ResultTooLarge` with an uninformative `count: 0`; truncated facets
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
                    Err(QueryError::bridge_result_too_large(
                        Budget::PropertyFacets,
                        0,
                        None,
                    ))
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
        crate::query::templates(&self.graph)
    }

    /// Icons for requested names from captured per-name trees, O(names log P
    /// + reachable alias rows). Initial publication builds O(P) rows; later
    /// publications patch only the changed page's icon and alias rows.
    pub fn page_icons(&self, names: &[String]) -> HashMap<String, String> {
        self.graph.page_icons(names)
    }

    /// Journal days with content in this view, excluding duplicate-day strays.
    /// O(P + journal blocks) because the scan visits all parsed pages before
    /// filtering for journals.
    pub fn journal_content_days(&self) -> Vec<Day> {
        self.graph
            .journal_content_days()
            .into_iter()
            .map(Day)
            .collect()
    }
}

/// Absent-journal path from live/captured config and a stem. O(path bytes), no I/O.
fn proposed_journal_id(config: &tine_core::config::Config, stem: String) -> PageId {
    PageId::from(format!(
        "{}/{}.{}",
        config.journals_dir,
        stem,
        config.preferred_format.ext()
    ))
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
mod graph_name_tests {
    use super::*;

    #[test]
    fn suggestion_skips_every_existing_entry() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(Store::suggest_graph_name(temp.path()).unwrap(), "notes");
        fs::create_dir(temp.path().join("notes")).unwrap();
        assert_eq!(Store::suggest_graph_name(temp.path()).unwrap(), "notes-2");
        fs::write(temp.path().join("notes-2"), b"keep").unwrap();
        assert_eq!(Store::suggest_graph_name(temp.path()).unwrap(), "notes-3");
    }

    #[test]
    fn graph_names_are_portable_and_trimmed() {
        for name in [
            "", " ", ".", "..", "a/b", "a\\b", "a<", "a>", "a:", "a\"", "a|", "a?", "a*", "a\0",
            "a\n", "a.", "a ", "CON", "nul.txt", "PRN", "AUX", "com1", "COM9.txt", "LPT1", "LPT9",
            "COM¹",
        ] {
            let temp = tempfile::tempdir().unwrap();
            assert!(
                Store::create_graph(temp.path(), name, &[]).is_err(),
                "{name:?}"
            );
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0, "{name:?}");
        }
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            Store::create_graph(temp.path(), "  My notes", &[]).unwrap(),
            temp.path().join("My notes")
        );
    }

    #[test]
    fn creation_refuses_nonempty_and_uses_empty_child() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("notes");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep"), b"untouched").unwrap();
        let error = Store::create_graph(temp.path(), "notes", &[]).unwrap_err();
        assert!(format!("{error}").contains("not empty"));
        assert_eq!(fs::read(root.join("keep")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::create_dir(temp.path().join("empty")).unwrap();
        assert_eq!(
            Store::create_graph(temp.path(), "empty", &[]).unwrap(),
            temp.path().join("empty")
        );
        assert!(temp.path().join("empty/logseq/config.edn").is_file());
        assert!(!temp.path().join("logseq").exists());
    }
}

#[cfg(test)]
mod rev5_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn alias_index_matches_legacy_scan_for_inventory_and_name_variants() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-alias-index-{unique}"));
        let pages = root.join("pages");
        fs::create_dir_all(&pages).unwrap();
        fs::write(
            pages.join("Owner A.md"),
            "alias:: Team/Alpha, Café\n- [[Orphan/Leaf]]\n",
        )
        .unwrap();
        fs::write(
            pages.join("Owner B.md"),
            "alias:: team/alpha, Cafe\u{301}\n- [[TEAM/ALPHA]]\n",
        )
        .unwrap();
        fs::write(pages.join("Café.md"), "- file wins over alias\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let view = store.whole_graph().unwrap();
        let inventory = view.inventory(crate::InventoryScope::All);
        let ordered_names: Vec<_> = inventory.0.iter().map(|entry| entry.name.clone()).collect();
        let mut legacy_order = ordered_names.clone();
        legacy_order.sort_by(|a, b| {
            tine_core::refs::page_key(a)
                .cmp(&tine_core::refs::page_key(b))
                .then_with(|| a.cmp(b))
        });
        assert_eq!(ordered_names, legacy_order);
        let alias_rows = view.graph.page_aliases_with_owners();
        assert_eq!(
            alias_rows
                .iter()
                .filter(|(alias, _, _)| tine_core::refs::same_page(alias, "TEAM/ALPHA"))
                .count(),
            2
        );
        assert_eq!(
            alias_rows
                .iter()
                .filter(|(alias, _, _)| tine_core::refs::same_page(alias, "Cafe\u{301}"))
                .count(),
            2
        );
        let names = inventory.0.iter().map(|entry| entry.name.as_str()).chain([
            "TEAM/ALPHA",
            "Team/Alpha",
            "team/alpha",
            "Café",
            "Cafe\u{301}",
            "café",
            "Orphan/Leaf",
            "orphan/leaf",
        ]);
        for name in names {
            let legacy: Vec<PageId> = view
                .graph
                .page_aliases_with_owners()
                .into_iter()
                .filter(|(alias, _, _)| tine_core::refs::same_page(alias, name))
                .map(|(_, _, path)| PageId::from(path))
                .collect();
            match view.resolve(name, false) {
                Resolved::Alias { owners } => assert_eq!(owners, legacy, "alias owners for {name}"),
                Resolved::Absent { .. } => assert!(legacy.is_empty(), "lost alias for {name}"),
                Resolved::Existing { .. } => assert!(
                    tine_core::refs::same_page(name, "Café")
                        || name == "Owner A"
                        || name == "Owner B",
                    "unexpected file for {name}"
                ),
            }
        }
        for entry in &inventory.0 {
            let actual = view.resolve(&entry.name, entry.is_journal);
            let target = &entry.target;
            match (actual, target) {
                (Resolved::Alias { owners: a }, Resolved::Alias { owners: b }) => assert_eq!(&a, b),
                (
                    Resolved::Existing { id: a, others: aa },
                    Resolved::Existing { id: b, others: bb },
                ) => {
                    assert_eq!(&a, b);
                    assert_eq!(&aa, bb);
                }
                (Resolved::Absent { id: a }, Resolved::Absent { id: b }) => assert_eq!(&a, b),
                _ => panic!("inventory differs from resolve for {}", entry.name),
            }
        }
        store.close();
        drop(view);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

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
            store.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
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
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.save_page(
            &[crate::EditKind::ReplacePage],
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        );
        assert!(
            matches!(tx.commit(), crate::TxOutcome::Committed { graph_rev, .. } if graph_rev == prior)
        );
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
        store.refresh(crate::Depth::Stamps).unwrap();
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
        assert!(matches!(store.is_graph_ready(), Ok(false)));
        *store.load.status.lock().unwrap() = LoadStatus::Failed("injected failure".into());
        store.load.ready.notify_all();
        assert!(
            matches!(store.is_graph_ready(), Err(LoadError::Failed { reason }) if reason == "injected failure")
        );
        fs::write(root.join("pages/A.md"), "- changed outside\n").unwrap();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        assert!(read.doc.blocks[0].raw.contains("changed outside"));
        assert!(store.page_named("A", PageKind::Page).unwrap().is_some());
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
        assert!(changes.try_recv().unwrap().is_none());

        let mut doc = read.doc;
        doc.blocks[0].raw = "changed here".into();
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        assert!(changes.try_recv().unwrap().is_none());
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));

        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(matches!(store.is_graph_ready(), Ok(true)));
        let view = store.whole_graph().unwrap();
        assert!(view.corpus().pages.iter().any(|page| {
            page.name == "A" && page.document.roots[0].raw().contains("changed here")
        }));
        assert!(store.page(&id).unwrap().doc.blocks[0]
            .raw
            .contains("changed here"));
        assert!(changes.try_recv().unwrap().is_some());
        fs::remove_file(pause).unwrap();
        store.close();
        assert!(matches!(store.is_graph_ready(), Err(LoadError::Closed)));
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
        let recovery =
            std::thread::spawn(move || recovery_store.refresh(crate::Depth::Stamps).unwrap());
        wait_hook(&pause);
        let save_store = Arc::clone(&store);
        let (attempting, attempted) = mpsc::channel();
        let save = std::thread::spawn(move || {
            attempting.send(()).unwrap();
            save_store.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            )
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
    fn recovery_does_not_report_ready_before_snapshot_exists() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-recovery-snapshot-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join(".tine-test-pause-load"), "").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        *store.load.status.lock().unwrap() = LoadStatus::Failed("injected failure".into());
        store.load.ready.notify_all();
        let subscription = store.subscribe();
        let observer_store = Arc::clone(&store);
        let observer = std::thread::spawn(move || {
            let change = subscription.recv().unwrap();
            assert_eq!(change.origin, Origin::External);
            assert!(observer_store.is_graph_ready().unwrap());
            observer_store.whole_graph().unwrap().rev()
        });
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.changes.snapshot_publish_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let recovery_store = Arc::clone(&store);
        let recovery = std::thread::spawn(move || recovery_store.refresh(crate::Depth::Stamps));
        wait_hook(&pause);
        assert!(matches!(
            store.is_graph_ready(),
            Err(LoadError::Failed { .. })
        ));
        release_hook(&pause);
        assert!(recovery.join().unwrap().is_ok());
        assert!(store.whole_graph().is_ok());
        assert_eq!(observer.join().unwrap(), store.whole_graph().unwrap().rev());
        store.close();
        fs::remove_file(root.join(".tine-test-pause-load")).unwrap();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn readiness_poll_does_not_wait_for_initial_snapshot_capture() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-ready-poll-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let hold_load = root.join(".tine-test-pause-load");
        fs::write(&hold_load, "").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.changes.snapshot_publish_pause.lock().unwrap() = Some(Arc::clone(&pause));
        fs::remove_file(hold_load).unwrap();
        wait_hook(&pause);
        let poll_store = Arc::clone(&store);
        let (send, receive) = mpsc::channel();
        let poll = std::thread::spawn(move || send.send(poll_store.is_graph_ready()).unwrap());
        let observed = receive.recv_timeout(Duration::from_secs(2));
        release_hook(&pause);
        poll.join().unwrap();
        assert!(matches!(observed, Ok(Ok(false))));
        assert!(store.whole_graph().is_ok());
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn own_save_during_initial_parse_does_not_restart() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-load-delta-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- old A\n").unwrap();
        fs::write(root.join("pages/B.md"), "- old B\n").unwrap();
        let hold_load = root.join(".tine-test-pause-load");
        fs::write(&hold_load, "").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&pause));
        fs::remove_file(hold_load).unwrap();
        wait_hook(&pause);
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "new A".into();
        fs::write(root.join("pages/B.md"), "- external B\n").unwrap();
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        release_hook(&pause);
        let view = store.whole_graph().unwrap();
        assert!(view
            .corpus()
            .pages
            .iter()
            .any(|page| page.name == "A" && page.document.roots[0].raw().contains("new A")));
        assert!(view
            .corpus()
            .pages
            .iter()
            .any(|page| page.name == "B" && page.document.roots[0].raw().contains("external B")));
        assert_eq!(store.graph.warm_passes.load(Ordering::Relaxed), 1);
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_during_cold_load_is_in_first_complete_view() {
        use tine_core::model::{BlockDto, PageDto};

        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-cold-create-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- existing\n").unwrap();
        let hold_load = root.join(".tine-test-pause-load");
        fs::write(&hold_load, "").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let warm_pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&warm_pause));
        let installed_pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.warm_after_install_pause.lock().unwrap() = Some(Arc::clone(&installed_pause));
        let save_pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.cold_cache_reconcile_pause.lock().unwrap() = Some(Arc::clone(&save_pause));
        fs::remove_file(hold_load).unwrap();
        wait_hook(&warm_pause);

        let saving = Arc::clone(&store);
        let save = std::thread::spawn(move || {
            saving.save(
                crate::EditKind::ReplacePage,
                &PageId::from("pages/Meta.md"),
                SaveBase::CreateNew,
                &PageDto {
                    name: "Meta".into(),
                    kind: PageKind::Page,
                    title: "Meta".into(),
                    pre_block: None,
                    blocks: vec![BlockDto {
                        raw: "a perfectly ordinary block".into(),
                        ..Default::default()
                    }],
                    rev: None,
                    format: Default::default(),
                    read_only: false,
                    guide: false,
                },
            )
        });
        wait_hook(&save_pause);
        release_hook(&warm_pause);
        wait_hook(&installed_pause);
        release_hook(&save_pause);
        release_hook(&installed_pause);
        assert!(matches!(save.join().unwrap(), SaveOutcome::Saved(_)));
        let view = store.whole_graph().unwrap();
        assert!(view.corpus().pages.iter().any(|page| page.name == "Meta"));
        assert!(matches!(
            view.resolve("Meta", false),
            Resolved::Existing { .. }
        ));
        assert!(view
            .inventory(crate::InventoryScope::All)
            .0
            .iter()
            .any(|entry| entry.name == "Meta"));
        assert_eq!(
            view.search(
                &SearchRequest {
                    text: "ordinary".into(),
                    within: None,
                    page_limit: 10,
                    block_limit: 10,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &Cancel(Arc::new(AtomicBool::new(false))),
            )
            .unwrap()
            .hits
            .len(),
            1
        );
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn own_page_writes_stay_in_view_with_notify_and_poll() {
        for mode in [WatchMode::Notify, WatchMode::Poll] {
            let unique = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!("tine-own-view-{mode:?}-{unique}"));
            fs::create_dir_all(root.join("pages")).unwrap();
            fs::create_dir_all(root.join("journals")).unwrap();
            fs::create_dir_all(root.join("assets")).unwrap();
            let store = Store::open(
                &root,
                OpenOptions {
                    watch: mode,
                    ..Default::default()
                },
            )
            .unwrap()
            .0;
            store.whole_graph().unwrap();

            let a = PageId::from("pages/A.md");
            let mut create = store.transaction(Some(crate::EditKind::ReplacePage));
            create.create(
                &a.file(),
                crate::Content::Bytes(b"- initial word\n".to_vec()),
            );
            assert!(matches!(
                create.commit(),
                crate::TxOutcome::Committed { .. }
            ));
            assert!(matches!(
                store.whole_graph().unwrap().resolve("A", false),
                Resolved::Existing { .. }
            ));

            let read = store.page(&a).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "saved word".into();
            assert!(matches!(
                store.save(
                    crate::EditKind::ReplacePage,
                    &a,
                    SaveBase::Existing(read.rev),
                    &doc
                ),
                SaveOutcome::Saved(_)
            ));
            assert!(store
                .whole_graph()
                .unwrap()
                .corpus()
                .pages
                .iter()
                .any(|page| {
                    page.name == "A" && page.document.roots[0].raw().contains("saved word")
                }));

            let b = PageId::from("pages/B.md");
            let mut moving = store.transaction(Some(crate::EditKind::ReplacePage));
            moving.move_file(
                &a.file(),
                FileRev::from_file(&root.join("pages/A.md")).unwrap(),
                &b.file(),
                None,
            );
            assert!(matches!(
                moving.commit(),
                crate::TxOutcome::Committed { .. }
            ));
            let moved = store.whole_graph().unwrap();
            assert!(matches!(
                moved.resolve("B", false),
                Resolved::Existing { .. }
            ));
            assert!(!matches!(
                moved.resolve("A", false),
                Resolved::Existing { .. }
            ));

            let source = root.join("restore-source.md");
            fs::write(&source, "- restored word\n").unwrap();
            store
                .restore(
                    crate::EditKind::ReplacePage,
                    vec![crate::RestoreFile {
                        area: Area::Pages,
                        rel: "C.md".into(),
                        source: File::open(&source).unwrap(),
                        len: fs::metadata(&source).unwrap().len(),
                    }],
                    None,
                )
                .unwrap();
            let restored = store.whole_graph().unwrap();
            assert!(matches!(
                restored.resolve("C", false),
                Resolved::Existing { .. }
            ));
            assert!(!matches!(
                restored.resolve("B", false),
                Resolved::Existing { .. }
            ));
            assert!(restored.corpus().pages.iter().any(|page| {
                page.name == "C" && page.document.roots[0].raw().contains("restored word")
            }));
            store.close();
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn close_during_recovery_returns_closed_without_reviving_status() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-close-recovery-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        *store.load.status.lock().unwrap() = LoadStatus::Failed("injected".into());
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store
            .watch
            .core_for_load()
            .recovery_warm_pause
            .lock()
            .unwrap() = Some(Arc::clone(&pause));
        let recovery_store = Arc::clone(&store);
        let recovery = std::thread::spawn(move || recovery_store.refresh(crate::Depth::Stamps));
        wait_hook(&pause);
        store.close();
        release_hook(&pause);
        assert!(matches!(recovery.join().unwrap(), Err(LoadError::Closed)));
        assert!(matches!(store.is_graph_ready(), Err(LoadError::Closed)));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_write_between_direct_read_and_stamp_is_observed() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-direct-stamp-race-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/A.md");
        fs::write(&path, "- before\n").unwrap();
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    watch: WatchMode::Poll,
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph().unwrap();
        // Let the load-completion wake finish its first poll before writing.
        std::thread::sleep(Duration::from_millis(250));
        fs::write(&path, "- first external\n").unwrap();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.watch.core_for_load().note_own_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let reader_store = Arc::clone(&store);
        let reader = std::thread::spawn(move || reader_store.page(&PageId::from("pages/A.md")));
        wait_hook(&pause);
        fs::write(&path, "- second external\n").unwrap();
        release_hook(&pause);
        assert!(reader.join().unwrap().is_ok());
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| {
                page.name == "A" && page.document.roots[0].raw().contains("second external")
            }));
        let read = store.page(&PageId::from("pages/A.md")).unwrap();
        assert!(read.doc.blocks[0].raw.contains("second external"));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_write_between_own_save_and_stamp_is_observed() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-save-stamp-race-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/A.md");
        fs::write(&path, "- before\n").unwrap();
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    watch: WatchMode::Poll,
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph().unwrap();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "own save".into();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.watch.core_for_load().note_own_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let save_store = Arc::clone(&store);
        let save = std::thread::spawn(move || {
            save_store.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            )
        });
        wait_hook(&pause);
        fs::write(&path, "- external winner\n").unwrap();
        release_hook(&pause);
        assert!(matches!(save.join().unwrap(), SaveOutcome::Saved(_)));
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| {
                page.name == "A" && page.document.roots[0].raw().contains("external winner")
            }));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn revert_and_delete_after_own_publication_are_external() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-own-race-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/A.md");
        fs::write(&path, "- before\n").unwrap();
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    watch: WatchMode::Poll,
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let id = PageId::from("pages/A.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "own save".into();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.watch.core_for_load().note_own_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let saving = Arc::clone(&store);
        let save = std::thread::spawn(move || {
            saving.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            )
        });
        wait_hook(&pause);
        fs::write(&path, "- before\n").unwrap();
        release_hook(&pause);
        assert!(matches!(save.join().unwrap(), SaveOutcome::Saved(_)));
        let observed: Vec<_> = std::iter::from_fn(|| changes.try_recv().unwrap()).collect();
        assert!(observed
            .iter()
            .any(|change| change.origin == Origin::External
                && change
                    .files
                    .iter()
                    .any(|(file, kind, _)| file.as_str() == "pages/A.md"
                        && *kind == ChangeKind::Modified)));
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| page.name == "A" && page.document.roots[0].raw().contains("before")));
        let read = store.page(&PageId::from("pages/A.md")).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "second own save".into();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.watch.core_for_load().note_own_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let saving = Arc::clone(&store);
        let save = std::thread::spawn(move || {
            saving.save(
                crate::EditKind::ReplacePage,
                &PageId::from("pages/A.md"),
                SaveBase::Existing(read.rev),
                &doc,
            )
        });
        wait_hook(&pause);
        fs::remove_file(&path).unwrap();
        release_hook(&pause);
        assert!(matches!(save.join().unwrap(), SaveOutcome::Saved(_)));
        let observed: Vec<_> = std::iter::from_fn(|| changes.try_recv().unwrap()).collect();
        assert!(observed
            .iter()
            .any(|change| change.origin == Origin::External
                && change
                    .files
                    .iter()
                    .any(|(file, kind, _)| file.as_str() == "pages/A.md"
                        && *kind == ChangeKind::Removed)));
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .all(|page| page.name != "A"));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn notify_rewatches_recreated_managed_directory() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-rewatch-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::write(root.join("pages/A.md"), "- before\n").unwrap();
        let store = Store::open(
            &root,
            OpenOptions {
                watch: WatchMode::Notify,
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        fs::remove_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/B.md"), "- after\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut observed = false;
        while std::time::Instant::now() < deadline {
            if let Some(change) = changes.try_recv().unwrap() {
                if change.origin == Origin::External
                    && change.files.iter().any(|(id, kind, _)| {
                        id.as_str() == "pages/B.md" && *kind == ChangeKind::Created
                    })
                {
                    observed = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(observed, "recreated directory contents must be observed");
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| page.name == "B"));
        let b = PageId::from("pages/B.md");
        let read = store.page(&b).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "saved after rewatch".into();
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &b,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| {
                page.name == "B" && page.document.roots[0].raw().contains("saved after rewatch")
            }));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_save_external_delete_is_external_and_own_trash_stays_own() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-delete-origin-{unique}"));
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
        let mut doc = read_a.doc;
        doc.blocks[0].raw = "new A".into();
        store.inject_fault(crate::FaultPoint::Stage2ExternalDelete);
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &a,
                SaveBase::Existing(read_a.rev),
                &doc
            ),
            SaveOutcome::Deleted
        ));
        let deleted = changes
            .try_recv()
            .unwrap()
            .expect("external deletion change");
        assert_eq!(deleted.origin, Origin::External);
        assert!(deleted
            .files
            .iter()
            .any(|(id, kind, _)| { id == &a.file() && *kind == ChangeKind::Removed }));
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.trash(&b.file(), read_b.rev, crate::TrashIf::Any);
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        let trashed = changes.try_recv().unwrap().expect("own trash change");
        assert_eq!(trashed.origin, Origin::Own);
        assert!(trashed
            .files
            .iter()
            .any(|(id, kind, _)| { id == &b.file() && *kind == ChangeKind::Removed }));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streamed_create_uses_recorded_revision_for_own_publication() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-stream-own-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        let source = root.join("stream-source.bin");
        fs::write(&source, b"stream content").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let stream_id = store.file_id(Area::Assets, "stream.bin").unwrap();
        let bytes_id = store.file_id(Area::Assets, "bytes.bin").unwrap();
        store.inject_fault(crate::FaultPoint::RemoveStreamStageAfterWrite);
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.create(
            &stream_id,
            crate::Content::Stream {
                source: File::open(&source).unwrap(),
                max_bytes: 1024,
            },
        );
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        let stream_change = changes.try_recv().unwrap().unwrap();
        assert_eq!(stream_change.origin, Origin::Own);
        assert!(stream_change
            .files
            .iter()
            .any(|(id, _, _)| id == &stream_id));
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            changes.try_recv().unwrap().is_none(),
            "present streamed asset was removed by a full watcher scan"
        );
        store.set_watch_mode(WatchMode::Poll);
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.create(&bytes_id, crate::Content::Bytes(b"byte content".to_vec()));
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        assert_eq!(changes.try_recv().unwrap().unwrap().origin, Origin::Own);
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            changes.try_recv().unwrap().is_none(),
            "present byte asset was removed by a full watcher scan"
        );
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.replace(
            &bytes_id,
            FileRev::from_file(&root.join("assets/bytes.bin")).unwrap(),
            b"replaced content".to_vec(),
        );
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        assert_eq!(changes.try_recv().unwrap().unwrap().origin, Origin::Own);
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            changes.try_recv().unwrap().is_none(),
            "present replaced asset was removed by a full watcher scan"
        );
        let moved_id = store.file_id(Area::Assets, "moved.bin").unwrap();
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.move_file(
            &bytes_id,
            FileRev::from_file(&root.join("assets/bytes.bin")).unwrap(),
            &moved_id,
            None,
        );
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        assert_eq!(changes.try_recv().unwrap().unwrap().origin, Origin::Own);
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            changes.try_recv().unwrap().is_none(),
            "present moved asset was removed by a full watcher scan"
        );
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.trash(
            &moved_id,
            FileRev::from_file(&root.join("assets/moved.bin")).unwrap(),
            crate::TrashIf::Any,
        );
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        assert_eq!(changes.try_recv().unwrap().unwrap().origin, Origin::Own);
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            changes.try_recv().unwrap().is_none(),
            "trashed asset was reported twice"
        );
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn page_reappearing_after_full_scan_remains_resolvable() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-page-scan-race-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/Present.md");
        fs::write(&path, "- present\n").unwrap();
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    watch: WatchMode::Poll,
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store
            .watch
            .core_for_load()
            .after_collect_pause
            .lock()
            .unwrap() = Some(Arc::clone(&pause));
        fs::remove_file(&path).unwrap();
        let scanning = Arc::clone(&store);
        let scan = std::thread::spawn(move || scanning.refresh(crate::Depth::Stamps));
        wait_hook(&pause);
        fs::write(&path, "- present\n").unwrap();
        release_hook(&pause);
        scan.join().unwrap().unwrap();
        *store
            .watch
            .core_for_load()
            .after_collect_pause
            .lock()
            .unwrap() = None;
        let observed: Vec<_> = std::iter::from_fn(|| changes.try_recv().unwrap()).collect();
        assert!(
            observed.iter().all(|change| change
                .files
                .iter()
                .all(|(id, kind, _)| id.as_str() != "pages/Present.md"
                    || *kind != ChangeKind::Removed)),
            "present page was published as removed: {observed:?}"
        );
        assert!(store
            .whole_graph()
            .unwrap()
            .inventory(crate::InventoryScope::All)
            .0
            .iter()
            .any(|entry| entry.name == "Present"));
        assert!(matches!(
            store.whole_graph().unwrap().resolve("Present", false),
            Resolved::Existing { .. }
        ));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn own_config_replace_is_not_removed_by_page_inventory_scan() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-config-own-scan-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("logseq")).unwrap();
        let path = root.join("logseq/config.edn");
        fs::write(&path, "{:foo 1}\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let id = store.file_id(Area::Meta, "config.edn").unwrap();
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.replace(
            &id,
            FileRev::from_file(&path).unwrap(),
            b"{:foo 2}\n".to_vec(),
        );
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        assert_eq!(changes.try_recv().unwrap().unwrap().origin, Origin::Own);
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            changes.try_recv().unwrap().is_none(),
            "present config was removed by page inventory scan"
        );
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn raced_approved_external_asset_is_modified_not_removed() {
        use std::os::unix::fs::symlink;

        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-approved-asset-race-{unique}"));
        let outside = root.with_extension("assets");
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("assets")).unwrap();
        let store = Arc::new(
            Store::open(
                &root,
                OpenOptions {
                    approved_external_assets: Some(outside.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .0,
        );
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.watch.core_for_load().note_own_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let id = store.file_id(Area::Assets, "raced.bin").unwrap();
        let writing = Arc::clone(&store);
        let tx = std::thread::spawn(move || {
            let mut tx = writing.transaction(Some(crate::EditKind::ReplacePage));
            tx.create(&id, crate::Content::Bytes(b"own".to_vec()));
            tx.commit()
        });
        wait_hook(&pause);
        fs::write(outside.join("raced.bin"), b"external").unwrap();
        release_hook(&pause);
        assert!(matches!(
            tx.join().unwrap(),
            crate::TxOutcome::Committed { .. }
        ));
        let own = changes.try_recv().unwrap().unwrap();
        let external = changes.try_recv().unwrap().unwrap();
        assert_eq!(own.origin, Origin::Own);
        assert_eq!(external.origin, Origin::External);
        assert!(
            external
                .files
                .iter()
                .any(|(id, kind, _)| id.as_str() == "assets/raced.bin"
                    && *kind == ChangeKind::Modified),
            "race change: {external:?}"
        );
        assert_eq!(fs::read(outside.join("raced.bin")).unwrap(), b"external");
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn create_new_twin_after_write_withdraws_own_bytes() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-create-twin-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/Small.md"), "- edit stays in caller\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let mut doc = store.page(&PageId::from("pages/Small.md")).unwrap().doc;
        doc.name = "New".into();
        doc.title = "New".into();
        store.inject_fault(crate::FaultPoint::TwinAfterPublish);
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &PageId::from("pages/New.md"),
                SaveBase::CreateNew,
                &doc
            ),
            SaveOutcome::Twin { .. }
        ));
        assert!(!root.join("pages/New.md").exists());
        assert_eq!(
            fs::read(root.join("pages/New.org")).unwrap(),
            b"external twin"
        );
        assert!(doc.blocks[0].raw.contains("edit stays in caller"));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_page_refuses_save_but_moves_and_trashes_as_opaque_file() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tine-opaque-page-{unique}"));
        fs::create_dir_all(root.join("pages")).unwrap();
        let a_path = root.join("pages/A.md");
        let b_path = root.join("pages/B.md");
        for path in [&a_path, &b_path] {
            let file = File::create(path).unwrap();
            file.set_len(crate::PARSE_INPUT_MAX_BYTES + 1).unwrap();
        }
        fs::write(root.join("pages/Small.md"), "- small\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let changes = store.subscribe();
        let a = PageId::from("pages/A.md");
        let a_rev = FileRev::from_file(&a_path).unwrap();
        let b_rev = FileRev::from_file(&b_path).unwrap();
        let doc = store.page(&PageId::from("pages/Small.md")).unwrap().doc;
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &a,
                SaveBase::Existing(a_rev.clone()),
                &doc
            ),
            SaveOutcome::InvalidTarget(_)
        ));
        let mut move_tx = store.transaction(Some(crate::EditKind::ReplacePage));
        let moved = FileId::from("pages/Moved.md".to_string());
        move_tx.move_file(&a.file(), a_rev, &moved, None);
        assert!(matches!(
            move_tx.commit(),
            crate::TxOutcome::Committed { .. }
        ));
        assert_eq!(
            fs::metadata(root.join("pages/Moved.md")).unwrap().len(),
            crate::PARSE_INPUT_MAX_BYTES + 1
        );
        let change = changes.try_recv().unwrap().unwrap();
        assert_eq!(change.origin, Origin::Own);
        assert!(change
            .files
            .iter()
            .any(|(id, kind, _)| id == &moved && *kind == ChangeKind::Created));
        let view = store.whole_graph().unwrap();
        assert!(view.unreadable_files().iter().any(|(id, _)| id == &moved));
        assert!(!view
            .unreadable_files()
            .iter()
            .any(|(id, _)| id == &a.file()));
        let mut trash_tx = store.transaction(Some(crate::EditKind::ReplacePage));
        let b = PageId::from("pages/B.md");
        trash_tx.trash(&b.file(), b_rev, crate::TrashIf::Any);
        assert!(matches!(
            trash_tx.commit(),
            crate::TxOutcome::Committed { .. }
        ));
        assert!(!b_path.exists());
        assert_eq!(changes.try_recv().unwrap().unwrap().origin, Origin::Own);
        assert!(!store
            .whole_graph()
            .unwrap()
            .unreadable_files()
            .iter()
            .any(|(id, _)| id == &b.file()));
        let d_path = root.join("pages/D.md");
        let file = File::create(&d_path).unwrap();
        file.set_len(crate::PARSE_INPUT_MAX_BYTES + 1).unwrap();
        let d_rev = FileRev::from_file(&d_path).unwrap();
        store.inject_fault(crate::FaultPoint::MidStepIo);
        let mut rollback_tx = store.transaction(Some(crate::EditKind::ReplacePage));
        rollback_tx.move_file(
            &FileId::from("pages/D.md".to_string()),
            d_rev,
            &FileId::from("pages/E.md".to_string()),
            None,
        );
        assert!(matches!(
            rollback_tx.commit(),
            crate::TxOutcome::NotCommitted { .. }
        ));
        assert!(d_path.exists());
        assert!(!root.join("pages/E.md").exists());
        store.close();
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
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.save_page(
            &[crate::EditKind::ReplacePage],
            &a,
            SaveBase::Existing(read_a.rev),
            &doc_a,
        );
        tx.save_page(
            &[crate::EditKind::ReplacePage],
            &b,
            SaveBase::Existing(read_b.rev),
            &doc_b,
        );
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
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !state.0 {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "test pause hook was not reached");
            state = ready.wait_timeout(state, remaining).unwrap().0;
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
                writer_store.save(
                    crate::EditKind::ReplacePage,
                    &writer_id,
                    SaveBase::Existing(read.rev),
                    &doc,
                )
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
            store.save(
                crate::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
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
                    let _ = view.query("[[Target]]", QueryDialect::Simple).unwrap();
                    let _ = view.resolve("External", false);
                    let _ = view.inventory(crate::InventoryScope::All);
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
                    writer_store.save(
                        crate::EditKind::ReplacePage,
                        &id,
                        SaveBase::Existing(read.rev),
                        &doc
                    ),
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
                watch_store.refresh(crate::Depth::Stamps).unwrap();
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
        let query_rows = |dialect| match view.query("[[Target]]", dialect).unwrap() {
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
                    view.inventory(crate::InventoryScope::All)
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
                            explain: false,
                            page_match_scope: None,
                            page_view: None,
                            block_view: None,
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
                        dialect: Default::default(),
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
        let old_inventory = old.inventory(crate::InventoryScope::All).0.len();
        assert!(matches!(
            old.resolve("Added", false),
            Resolved::Absent { .. }
        ));

        let source = PageId::from("pages/Source.md");
        let read = store.page(&source).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "[[Target]] after save".into();
        assert!(matches!(
            store.save(
                crate::EditKind::ReplacePage,
                &source,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        assert_eq!(view_answers(&old), original_answers);
        let fresh_after_save = store.whole_graph().unwrap();
        assert_ne!(view_answers(&fresh_after_save), original_answers);

        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
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
                crate::EditKind::ReplacePage,
                &PageId::from("pages/AliasOwner.md"),
                SaveBase::CreateNew,
                &alias_page
            ),
            SaveOutcome::Saved(_)
        ));
        assert_eq!(view_answers(&old), original_answers);

        fs::write(root.join("pages/Added.md"), "- [[Target]] after\n").unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        let fresh = store.whole_graph().unwrap();
        assert!(fresh.rev() != old.rev());
        assert_eq!(old.corpus().pages.len(), old_corpus);
        assert_eq!(old.backlinks("Target").unwrap().len(), old_backlinks);
        assert_eq!(
            old.inventory(crate::InventoryScope::All).0.len(),
            old_inventory
        );
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

    #[cfg(unix)]
    #[test]
    fn scan_reports_non_utf8_names_for_backup_completeness() {
        use std::os::unix::ffi::OsStringExt;
        let root = std::env::temp_dir().join(format!("tine-scan-nonutf8-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pages")).unwrap();
        let name = std::ffi::OsString::from_vec(b"lost-\xff.md".to_vec());
        fs::write(root.join("pages").join(name), b"- text\n").unwrap();
        fs::write(root.join("pages/invalid\\name.md"), b"- text\n").unwrap();
        fs::create_dir_all(root.join("pages/invalid\\directory")).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let listing = store.scan_area(Area::Pages, None).unwrap();
        assert_eq!(listing.unreadable.len(), 3);
        assert!(listing
            .unreadable
            .iter()
            .all(|(_, error)| error.kind == std::io::ErrorKind::InvalidData));
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
