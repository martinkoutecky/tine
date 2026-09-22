use crate::config::ParseConfig;
use crate::doc::{property_key_norm, DocBlock, Document};
use crate::query::registry_cache::{CommittedRegistryCache, RegistryCapture};
use crate::query::registry_sql::{self, PageRegistryMetadata};
use crate::query::PropertyFacetAccumulator;
use crate::query_cursor::drain_after;
use crate::query_jobs::{
    OwnedAdmission, QueryJobOwner, DEFAULT_QUERY_JOB_CAPACITY, QUERY_JOB_WAIT,
};
use crate::vocab::{Format, PageEntry, PageKind, ReferenceKind};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use fs2::FileExt as _;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tine_storage::sqlite::{
    PhysicalAliasDeclaration, PhysicalBlock, PhysicalEntityCoordinate, PhysicalEntityId,
    PhysicalGraphProjectionChange, PhysicalGraphProjectionDatabase,
    PhysicalGraphProjectionSourceRevision, PhysicalName, PhysicalPage,
    PhysicalProjectionQuerySnapshot, PhysicalProperty, PhysicalQueryValue,
    PhysicalReferencePosting, PhysicalReferenceTarget, PhysicalTask,
};
use uuid::Uuid;

type PageSnapshot = Arc<Vec<(PageEntry, Arc<Document>)>>;
type PageRevisions = Arc<HashMap<PathBuf, String>>;

struct CommittedRegistryOwner {
    cache: CommittedRegistryCache,
    config: Arc<ParseConfig>,
}

type SharedCommittedRegistry = Arc<Mutex<Option<CommittedRegistryOwner>>>;

// This is the parser-fact extractor identity, not an on-disk schema version.
// Bump it whenever unchanged source bytes must be lowered into new/different
// physical facts. The source-revision delta then rebuilds each page once even
// when tine-storage's disposable SQLite schema itself remains compatible.
const DIRECT_PROJECTION_FACTS_VERSION: u32 = 2;
const REFERENCE_DELTA_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(test)]
// Test receipts count only their own graph, including its worker threads.
static PHYSICAL_PAGE_LOWERINGS: Mutex<(Option<PathBuf>, u64)> = Mutex::new((None, 0));

#[cfg(test)]
static BEFORE_APPLY_PENDING: Mutex<Option<Box<dyn FnOnce() + Send>>> = Mutex::new(None);

#[cfg(test)]
fn run_before_apply_deltas_hook() {
    if let Some(hook) = BEFORE_APPLY_PENDING.lock().unwrap().take() {
        hook();
    }
}

#[cfg(test)]
thread_local! {
    static REGISTRY_READ_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_registry_read_attempts() -> u64 {
    REGISTRY_READ_ATTEMPTS.with(|count| count.replace(0))
}

/// One queued page change. **The graph config travels INSIDE the work item**
/// (§5.8 M21, F11): every arm that lowers a page carries the exact
/// [`ParseConfig`] it must be lowered under, so the worker cannot reach a state
/// where queued work exists and the config that describes it does not.
///
/// The config used to sit beside the queue, and the worker read it as
/// `parse_config.clone().unwrap_or_else(|| Arc::new(ParseConfig::default()))`.
/// That fallback was unreachable — the stop check runs first and every enqueue
/// path set the config in the same critical section that inserted the work —
/// but if it had ever fired it would have lowered queued pages under the
/// DEFAULT config and stamped the result as current: silently wrong rows,
/// which is exactly what the stamp exists to prevent, reached from inside. A
/// `debug_assert` would have hidden the release-mode behaviour behind a passing
/// debug run, so the absence is removed by SHAPE — it can no longer be spelled.
///
/// `Delete` deliberately carries no config: it lowers nothing and stamps no
/// source revision, so a config on that arm would be a value with no reader.
#[derive(Clone)]
enum PageDelta {
    Replace {
        entry: PageEntry,
        document: Arc<Document>,
        revision: String,
        parse_config: Arc<ParseConfig>,
        page_position: Option<u64>,
    },
    Delete {
        entry: PageEntry,
    },
}

/// One page the caller has added, rewritten or removed, as the model layer
/// describes it. A mutation that changes the page SET — a rename, a merge, a
/// file rescue — hands its whole change over as one of these lists; see
/// [`DirectProjection::enqueue_page_set`].
pub(crate) enum PageSetChange {
    Replace {
        entry: PageEntry,
        document: Arc<Document>,
        revision: String,
    },
    Delete {
        entry: PageEntry,
    },
}

impl PageDelta {
    fn entry(&self) -> &PageEntry {
        match self {
            PageDelta::Replace { entry, .. } | PageDelta::Delete { entry } => entry,
        }
    }
}

/// A queued whole-graph snapshot and the config it must be lowered under. The
/// config is stamped into every page's `projection_source_revision`, so a
/// config edit re-lowers every page on the next snapshot instead of leaving
/// rows that answer a question the config no longer asks (J7, D-1: rebuild,
/// never migrate).
struct PendingFull {
    pages: PageSnapshot,
    revisions: PageRevisions,
    parse_config: Arc<ParseConfig>,
    /// Whether every physical page in the captured inventory was readable and
    /// parsed. A partial cache may still be useful to the app, but it must not
    /// replace a healthy complete projection and silently erase retained rows.
    source_complete: bool,
}

/// R6 warm validation: the walk inventory with each page's exact content
/// revision, and nothing parsed. The worker compares it with
/// `direct_source_revisions`; an unchanged graph publishes readiness from this
/// alone, while a changed graph asks the page-cache owner for one captured
/// parsed snapshot used by an unpublished fresh build.
struct PendingWarm {
    sources: Vec<(PageEntry, String)>,
    /// Pages the walk could not READ (an I/O error, not an absence). They are
    /// deliberately absent from `sources` because no revision could be taken
    /// for them, but they still exist: validation must leave their existing
    /// rows alone instead of reading their omission as a deletion (GH #543).
    retained: Vec<PageEntry>,
    /// Pages this session published after the walk read them, or created
    /// after it: each has an update queued beside the warm that brings its
    /// rows to the current bytes, so validation leaves the image's rows for
    /// it alone instead of calling the whole image stale (audit IT-03).
    published: Vec<String>,
    /// The pages an earlier attempt named `Changed`, parsed by the caller,
    /// applied before this attempt validates.
    repair: Option<WarmRepair>,
    parse_config: Arc<ParseConfig>,
}

/// A stale image brought current page by page (Martin, 2026-09-22): the
/// pages whose bytes changed, or that appeared, since the image was written,
/// and the pages that disappeared. Applied in one transaction, with the page
/// order reconciled to the walk; readiness still waits for the validation
/// that follows it.
pub(crate) struct WarmRepair {
    pub(crate) replacements: Vec<(PageEntry, Arc<Document>, String)>,
    pub(crate) deletions: Vec<String>,
}

/// Above this share of the walk, a stale image is rebuilt from a complete
/// parsed snapshot instead: it parses the same pages and writes a fresh file
/// rather than rewriting most of the old one in place.
const WARM_REPAIR_MAX_SHARE_DIVISOR: usize = 4;

/// What the worker's warm-validation turn decided (R6), read by the warm
/// thread through `wait_warm_outcome`.
#[derive(Clone, Debug)]
pub(crate) enum WarmOutcome {
    /// Every walk page's rows are current: readiness publishes without a parse.
    Clean,
    /// Some row, source revision, or page-set fact differs. A complete parsed
    /// snapshot must be built into a new unpublished database.
    FreshBuildRequired,
    /// A few pages differ from the walk (edited, added or removed while the
    /// image was not watching). The caller parses exactly the replacements
    /// and queues the warm again with them as its `WarmRepair`.
    Changed {
        replacements: Vec<String>,
        deletions: Vec<String>,
    },
    /// A full parsed snapshot arrived first and owns readiness.
    Superseded,
    /// The validation turn failed; the parser fallback owns readiness.
    Failed,
}

enum QueryCaptureRequirement {
    CurrentSnapshot,
    #[cfg(test)]
    StrictGeneration(u64),
}

struct PendingQueryCapture {
    requirement: QueryCaptureRequirement,
    registry_sensitivity: RegistrySensitivity,
    slot: crate::query_jobs::OwnedJobSlot,
    reply: std::sync::mpsc::SyncSender<QueryJobOpen>,
}

/// Whether a query can observe property type inference. Registry-sensitive
/// jobs freeze the cache input beside their SQL snapshot; all other jobs carry
/// no registry state at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrySensitivity {
    Insensitive,
    Required,
}

fn reject_query_captures(captures: Vec<PendingQueryCapture>) {
    for capture in captures {
        // Release admission before replying, so the receiver can immediately
        // retry without being held behind its own rejected capture.
        drop(capture.slot);
        let _ = capture.reply.send(QueryJobOpen::Cancelled);
    }
}

#[derive(Default)]
struct PendingProjection {
    // Each capture owns one slot from the shared two-job cap.
    captures: Vec<PendingQueryCapture>,
    full: Option<PendingFull>,
    rebuild: bool,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    latest_generation: u64,
    stop: bool,
    page_order: BTreeMap<String, u64>,
    next_page_order: u64,
    /// Whether this session's `page_order` has been seeded from a complete
    /// inventory (a full snapshot or a warm walk). Before that the queue does
    /// not know where a reopened image keeps its pages, so it hands out no
    /// positions at all: counting from 0 collided with the positions the
    /// image already stores (`UNIQUE constraint failed: pages.position`), and
    /// that failure rebuilt the whole projection on every launch (GH #550).
    order_seeded: bool,
    /// Updates for pages the reopened image does not hold, taken before the
    /// order was seeded. They cannot be placed without an inventory, so they
    /// wait here, outside `has_work`, and rejoin `deltas` the moment a warm or
    /// full snapshot seeds it. Before that nothing is validated, so readiness
    /// cannot be published without them. Refusing the turn instead latched a
    /// fresh build, and a page created during the warm read parsed the whole
    /// graph (GH #543, audit IT-03).
    unplaced: BTreeMap<String, (u64, PageDelta)>,
    /// R6 warm validation queued for the worker.
    warm: Option<PendingWarm>,
    /// R6: the worker's verdict on the last warm validation.
    /// R6: the decided warm's verdict, KEYED BY THE ATTEMPT that asked for
    /// it. One unkeyed slot let a warm that was descheduled before reading it
    /// take a later attempt's verdict, leaving that later attempt waiting for
    /// a producer that had already run — forever, while holding the
    /// process-wide warm mutex (GH #543, re-audit A2-B1).
    warm_outcome: Option<(u64, WarmOutcome)>,
    /// The attempt id of the most recent admitted warm. Monotonic; a waiter
    /// whose id is older has been superseded and must not wait.
    warm_attempt: u64,
}

impl PendingProjection {
    fn record_delta(&mut self, generation: u64, mut delta: PageDelta) {
        let key = delta.entry().rel_path.clone();
        match &mut delta {
            PageDelta::Replace { .. } if !self.order_seeded => {
                // No position: storage keeps a stored page's own, and the
                // worker refuses to place a page the image does not hold
                // (`settle_unseeded_deltas`).
            }
            PageDelta::Replace { page_position, .. } => {
                let position = if let Some(position) = self.page_order.get(&key) {
                    *position
                } else {
                    let position = self.next_page_order;
                    self.next_page_order += 1;
                    self.page_order.insert(key.clone(), position);
                    position
                };
                *page_position = Some(position);
            }
            PageDelta::Delete { .. } => {
                self.page_order.remove(&key);
            }
        }
        self.deltas.insert(key, (generation, delta));
        self.latest_generation = self.latest_generation.max(generation);
    }

    /// Seed the queue's page order from a complete inventory (a full snapshot
    /// or a warm walk), replacing whatever a cache-less session appended.
    fn seed_page_order<'a>(&mut self, inventory: impl ExactSizeIterator<Item = &'a str>) {
        let mut inventory = inventory.collect::<Vec<_>>();
        if self.rebuild {
            // Repair preserves the session's retained/append order. Stable
            // sorting leaves newly discovered paths in their inventory order,
            // after existing pages. Re-number both owners together below.
            inventory.sort_by_key(|path| self.page_order.get(*path).copied().unwrap_or(u64::MAX));
        }
        self.order_seeded = true;
        self.next_page_order = inventory.len() as u64;
        self.page_order = inventory
            .into_iter()
            .enumerate()
            .map(|(position, rel_path)| (rel_path.to_owned(), position as u64))
            .collect();
        for (path, parked) in std::mem::take(&mut self.unplaced) {
            // A newer update for the page supersedes the parked one.
            self.deltas.entry(path).or_insert(parked);
        }
    }

    /// Park updates the worker could not place (see `unplaced`). If the order
    /// was seeded while the worker held them, they are placed right away.
    fn park_unplaced(&mut self, parked: BTreeMap<String, (u64, PageDelta)>) {
        for (path, update) in parked {
            if self.deltas.contains_key(&path) {
                continue;
            }
            if self.order_seeded {
                self.deltas.insert(path, update);
            } else {
                self.unplaced.entry(path).or_insert(update);
            }
        }
        if self.order_seeded {
            self.place_unseeded_deltas();
        }
    }

    /// Forget the seeded order and every queued position: the image does not
    /// keep the order's positions (a warm named pages to repair). Updates
    /// then settle against the image as they do before any seed, and the
    /// repair's warm seeds the order again.
    fn unseed_page_order(&mut self) {
        self.order_seeded = false;
        self.next_page_order = 0;
        self.page_order.clear();
        for (_, (_, delta)) in self.deltas.iter_mut() {
            if let PageDelta::Replace { page_position, .. } = delta {
                *page_position = None;
            }
        }
    }

    /// After a warm repair reconciled the image to `order`, make the queue's
    /// order that same list and re-place every queued update against it.
    fn reseed_after_repair(&mut self, order: &[String]) {
        self.order_seeded = true;
        self.next_page_order = order.len() as u64;
        self.page_order = order
            .iter()
            .enumerate()
            .map(|(position, rel_path)| (rel_path.clone(), position as u64))
            .collect();
        for (_, (_, delta)) in self.deltas.iter_mut() {
            if let PageDelta::Replace { page_position, .. } = delta {
                *page_position = None;
            }
        }
        self.place_unseeded_deltas();
    }

    /// Positions for updates a worker turn has already taken, from the
    /// queue's current order.
    fn place_taken(&mut self, taken: &mut BTreeMap<String, (u64, PageDelta)>) {
        for (key, (_, delta)) in taken.iter_mut() {
            match delta {
                PageDelta::Replace { page_position, .. } => {
                    *page_position = Some(match self.page_order.get(key) {
                        Some(position) => *position,
                        None => {
                            let position = self.next_page_order;
                            self.next_page_order += 1;
                            self.page_order.insert(key.clone(), position);
                            position
                        }
                    });
                }
                PageDelta::Delete { .. } => {
                    self.page_order.remove(key);
                }
            }
        }
    }

    /// Give the updates queued before the order was seeded their positions.
    /// A page the inventory lists keeps its place; a page created after the
    /// inventory was read goes after it, as it would in a full snapshot.
    fn place_unseeded_deltas(&mut self) {
        for (key, (_, delta)) in self.deltas.iter_mut() {
            if let PageDelta::Replace {
                page_position: position @ None,
                ..
            } = delta
            {
                *position = Some(match self.page_order.get(key) {
                    Some(position) => *position,
                    None => {
                        let position = self.next_page_order;
                        self.next_page_order += 1;
                        self.page_order.insert(key.clone(), position);
                        position
                    }
                });
            }
        }
    }

    /// The queue's own page inventory in position order: the R6 order turn's
    /// authority. After a warm seed the map tracks every applied replacement
    /// and deletion, so it names exactly the pages the projection holds.
    fn ordered_inventory(&self) -> Vec<String> {
        let mut ordered = self
            .page_order
            .iter()
            .map(|(rel_path, position)| (*position, rel_path.clone()))
            .collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(position, _)| *position);
        ordered.into_iter().map(|(_, id)| id).collect()
    }

    fn has_work(&self) -> bool {
        self.full.is_some() || !self.deltas.is_empty() || self.warm.is_some()
    }
}

struct ProjectionShared {
    path: PathBuf,
    pending: Mutex<PendingProjection>,
    changed: Condvar,
    ready: AtomicBool,
    ready_generation: AtomicU64,
    /// Presentation invalidation only; never a requested edit or read target.
    commit_notification: AtomicU64,
    commit_waker: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    reader: Mutex<Option<PhysicalGraphProjectionDatabase>>,
    /// R3: the ONE admission/cancellation owner for database-owned query jobs
    /// (plan §2B). Capacity is taken before a snapshot is opened; the worker
    /// drains every job before it replaces or resets the file, and `Drop`
    /// drains before the worker is stopped.
    query_jobs: Arc<QueryJobOwner>,
    /// R3 identity policy (WARM-IDENTITY-ORDER-CONTRACT.md §"Chosen strategy"
    /// 2–3): the pages whose rows THIS process lowered. Their stored
    /// `blocks.result_id` is the live runtime id the parsed
    /// document carried when the row was written. Every other page's rows
    /// survived from an earlier session, and a fresh parse of an unchanged
    /// page assigns STRUCTURAL runtime ids, so their public id is derived from
    /// `(path, order_key)` through `model::doc_runtime_id_for_order` instead.
    /// Copy-on-write: the worker swaps a new `Arc` after each successful
    /// apply, and a job clones the `Arc` at snapshot acquisition — never a
    /// live lookup during output.
    session_pages: Mutex<Arc<HashSet<String>>>,
    committed_registry: SharedCommittedRegistry,
    worker_available: AtomicBool,
    worker_failed: AtomicBool,
    worker_busy: AtomicBool,
    /// True while the worker is EXECUTING a turn that carries a build — a full
    /// snapshot build or a warm validation. The
    /// queue empties the moment the worker takes that payload, so testing the
    /// queue alone reported `None` (idle) for the whole SQL transaction, and a
    /// surface that reruns on the completion edge announced a build finished
    /// in its most loaded moment (GH #543, re-audit A2-F1). `worker_busy` on
    /// its own is too broad: an ordinary one-page save turn is not a build.
    worker_building: AtomicBool,
    /// The writer worker has RETURNED, and every resource it owned — the
    /// SQLite writer connection and the exclusive writer lease — is closed.
    ///
    /// `worker_available` says only that the worker will take no further work;
    /// it is stored before those two locals drop. A caller that must remove the
    /// database's directory needs the stronger fact, so this flag is published
    /// by a guard declared FIRST in `projection_worker` and therefore dropped
    /// LAST. See [`DirectProjection::close_and_wait_for_worker`].
    worker_finished: AtomicBool,
    /// Resources whose destruction must follow the writer connection and
    /// lease. None closes registration once worker teardown starts.
    worker_resources: Mutex<Option<Vec<Arc<dyn Send + Sync>>>>,
    /// R6: this session has validated the complete page inventory against
    /// the projection at least once (a full snapshot, or a warm validation's
    /// `Clean` or closing order turn). Until then a live delta keeps the file
    /// converging but must not publish readiness: rows of pages this session
    /// has never compared to disk could be stale from an earlier session.
    /// In-scope scenario: an external edit between two sessions, followed by
    /// a save of some other page before the warm runs.
    validated: AtomicBool,
    #[cfg(test)]
    after_sql_commit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    after_fresh_build_batch: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    before_fresh_publication: Mutex<Option<Box<dyn FnOnce() -> Result<(), String> + Send>>>,
    #[cfg(test)]
    after_fresh_publication: Mutex<Option<Box<dyn FnOnce() -> Result<(), String> + Send>>>,
    #[cfg(test)]
    before_shared_reader_admission: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    after_shared_reader_admission: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    before_shared_reader_drain_lock: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    after_shared_reader_drain_lock: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    serving_writer_cache_budget: AtomicU64,
    #[cfg(test)]
    projection_health_checks: AtomicU64,
    #[cfg(test)]
    capture_thread: Mutex<Option<std::sync::mpsc::Sender<std::thread::ThreadId>>>,
    #[cfg(test)]
    indexed_reads: AtomicU64,
    /// §5.9's dispatched statements: how many times the lowering ANSWERED a
    /// user query through the seam. Separate from `indexed_reads`, which counts
    /// every seam read including the FTS-readiness probe, so a route guard can
    /// say "exactly one statement per query" and mean it.
    #[cfg(test)]
    statement_reads: AtomicU64,
    #[cfg(test)]
    registry_capture_attempts: AtomicU64,
    /// Repairs currently computing the payload they will enqueue. A repair
    /// that has not reached its `enqueue_*` yet has published nothing, so
    /// without this a query racing it reads the queue as idle and stale and
    /// its own repair attempt as one that "did not take" — a terminal
    /// `Unavailable(ReadFailed)` on a projection that is being repaired
    /// perfectly well by the thread beside it.
    repairs_in_flight: AtomicUsize,
    /// Warm validations currently reading page bytes for the inventory they
    /// will enqueue (GH #543). Same shape as `repairs_in_flight`: until
    /// `enqueue_warm` the warm has published nothing, so a query landing in
    /// that window read the projection as idle and stale and started its own
    /// repair — a second validation on the query thread, racing the open path's.
    /// Before this marker, that race could trigger a redundant whole-graph
    /// reconstruction on the query thread.
    warms_in_flight: AtomicUsize,
    /// Pages written by the running fresh build, for the indexing progress
    /// bar only (GH #543).
    build_progress: crate::indexing_progress::ProgressCounter,
    /// §5.9's failed-read injection: one read through the seam fails, exactly as
    /// a torn or truncated projection file, a disk error or a resource limit
    /// makes it fail. It exists because the obligation a failed read carries —
    /// note the fallback AND schedule the full-snapshot recovery — is invisible
    /// on a healthy projection, and an obligation nothing can observe is one a
    /// future arm silently drops (M9).
    #[cfg(test)]
    inject_read_failure: AtomicBool,
    #[cfg(test)]
    fallback_reads: AtomicU64,
    #[cfg(test)]
    referenced_name_reads: AtomicU64,
}

impl ProjectionShared {
    fn ready_at(&self, generation: u64) -> bool {
        self.ready.load(Ordering::Acquire)
            && self.ready_generation.load(Ordering::Acquire) == generation
    }

    fn cancel_queued_captures(&self, close: bool) -> crate::query_jobs::QueryDrainFence {
        let (fence, captures) = {
            let mut pending = self.pending.lock().unwrap();
            let fence = if close {
                pending.stop = true;
                self.query_jobs.begin_close()
            } else {
                // Reset/config replacement invalidates existing reader jobs.
                // Ordinary page deltas never enter this lifecycle boundary.
                self.query_jobs.begin_drain()
            };
            (fence, std::mem::take(&mut pending.captures))
        };
        reject_query_captures(captures);
        self.changed.notify_all();
        fence
    }

    /// R3 identity policy bookkeeping, run by the worker after every
    /// successful apply: the pages just lowered carry this process's live ids;
    /// the pages just deleted carry nothing.
    fn record_session_pages(&self, applied: &AppliedPages) {
        if applied.lowered.is_empty() && applied.deleted.is_empty() {
            return;
        }
        let mut current = self.session_pages.lock().unwrap();
        let membership_changed = applied.lowered.iter().any(|page| !current.contains(page))
            || applied.deleted.iter().any(|page| current.contains(page));
        if !membership_changed {
            return;
        }
        let mut next: HashSet<String> = (**current).clone();
        next.extend(applied.lowered.iter().cloned());
        for page in &applied.deleted {
            next.remove(page);
        }
        *current = Arc::new(next);
    }
}

/// Admission gate shared by admission and producer snapshot capture.
///
/// Live queries may read an older complete committed image while ordinary
/// page edits are queued. They may not read during validation of an unknown
/// image or while a replacement image is being built/published.
fn query_capture_admissible(shared: &ProjectionShared, pending: &PendingProjection) -> bool {
    !pending.stop
        && !pending.rebuild
        && !shared.worker_failed.load(Ordering::Acquire)
        && !shared.worker_building.load(Ordering::Acquire)
        && shared.validated.load(Ordering::Acquire)
}

fn query_capture_available(
    shared: &ProjectionShared,
    requirement: &QueryCaptureRequirement,
) -> bool {
    match requirement {
        QueryCaptureRequirement::CurrentSnapshot => {
            let pending = shared.pending.lock().unwrap();
            query_capture_admissible(shared, &pending)
        }
        #[cfg(test)]
        QueryCaptureRequirement::StrictGeneration(generation) => shared.ready_at(*generation),
    }
}

fn capture_query_job(
    shared: &ProjectionShared,
    requirement: QueryCaptureRequirement,
    registry_sensitivity: RegistrySensitivity,
    slot: crate::query_jobs::OwnedJobSlot,
) -> QueryJobOpen {
    #[cfg(test)]
    if let Some(observed) = shared.capture_thread.lock().unwrap().take() {
        observed.send(std::thread::current().id()).unwrap();
    }
    if slot.is_cancelled() {
        return QueryJobOpen::Cancelled;
    }
    let validate = || {
        if query_capture_available(shared, &requirement) {
            Ok(())
        } else {
            Err(tine_storage::sqlite::MaterializationError::Incomplete(
                "projection became unavailable during snapshot acquisition".into(),
            ))
        }
    };
    let mut snapshot = match PhysicalProjectionQuerySnapshot::open_direct(&shared.path, validate) {
        Ok(snapshot) => snapshot,
        // The validator is the only `Incomplete` this call can produce and
        // it means the acquisition condition changed, not a corrupt read.
        // Anything else is an unopenable or unreadable file.
        Err(_) if !query_capture_available(shared, &requirement) => return QueryJobOpen::NotReady,
        Err(_) => return QueryJobOpen::Failed,
    };
    if !slot.register(snapshot.cancellation()) {
        return QueryJobOpen::Cancelled;
    }
    let session_pages = Arc::clone(&shared.session_pages.lock().unwrap());
    let query_revision = match snapshot.query_revision() {
        Ok(revision) => revision,
        Err(_) => return QueryJobOpen::Failed,
    };
    // Config is an input to every query, even when no property registry is
    // needed. Capture it beside this transaction, never from the live Graph.
    let (config, registry) = {
        let owner = shared.committed_registry.lock().unwrap();
        let Some(owner) = owner.as_ref() else {
            return QueryJobOpen::NotReady;
        };
        let registry = match registry_sensitivity {
            RegistrySensitivity::Insensitive => None,
            RegistrySensitivity::Required => {
                #[cfg(test)]
                shared
                    .registry_capture_attempts
                    .fetch_add(1, Ordering::Relaxed);
                let Ok(capture) = owner.cache.capture(query_revision, &owner.config) else {
                    return QueryJobOpen::Failed;
                };
                Some(capture)
            }
        };
        (Arc::clone(&owner.config), registry)
    };
    if slot.is_cancelled() {
        return QueryJobOpen::Cancelled;
    }
    if !query_capture_available(shared, &requirement) {
        return QueryJobOpen::NotReady;
    }
    #[cfg(test)]
    shared.statement_reads.fetch_add(1, Ordering::Relaxed);
    QueryJobOpen::Job(DirectQueryJob {
        _slot: slot,
        snapshot,
        session_pages,
        config,
        #[cfg(test)]
        query_revision,
        registry,
        registry_owner: Arc::clone(&shared.committed_registry),
    })
}

/// One admitted, snapshot-owning Direct query job (R3). Everything the result
/// read needs is captured here at a complete producer boundary: the pinned read transaction, the compiled-regex program already
/// installed on its connection, and the identity policy input. Dropping the
/// job releases the transaction and the capacity slot.
pub(crate) struct DirectQueryJob {
    // Field drop order is a lifecycle boundary: release the SQLite transaction
    // before the admission slot can wake a projection replacement drain.
    pub(crate) snapshot: PhysicalProjectionQuerySnapshot,
    /// Held for its `Drop`: releasing the slot is the job's only exit.
    _slot: crate::query_jobs::OwnedJobSlot,
    /// The pages whose rows this process lowered (see
    /// `ProjectionShared::session_pages`), as of the snapshot.
    pub(crate) session_pages: Arc<HashSet<String>>,
    pub(crate) config: Arc<ParseConfig>,
    /// Actual acquired SQL image, distinct from the admission target.
    #[cfg(test)]
    pub(crate) query_revision: u64,
    registry: Option<RegistryCapture>,
    registry_owner: SharedCommittedRegistry,
}

impl DirectQueryJob {
    /// Check the existing static publisher's fresh source capture against this
    /// pinned projection image. Ordinary query reads do not call this method.
    /// These fingerprints are disposable metadata, not a second source authority.
    pub(crate) fn publication_sources_match(
        &mut self,
        sources: &[(PageEntry, String)],
        capture_config: &ParseConfig,
    ) -> Result<bool, crate::query::results::ResultReadError> {
        use crate::query::results::{sql_or_cancelled, ResultReadError};
        let config_digest = capture_config.digest();
        let mut expected = sources
            .iter()
            .map(|(entry, revision)| {
                (
                    entry.rel_path.clone(),
                    projection_source_revision(revision, config_digest),
                )
            })
            .collect::<Vec<_>>();
        expected.sort_unstable();
        if expected.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(ResultReadError::Corrupt(
                "duplicate physical page in publication capture".into(),
            ));
        }
        let mut at = 0;
        let mut matches = true;
        let mut malformed = false;
        let read = crate::query::projection_sql::visit(
            &mut self.snapshot,
            "SELECT p.path, s.revision FROM pages p \
             LEFT JOIN direct_source_revisions s ON s.path = p.path \
             ORDER BY p.path",
            &[],
            |row| {
                let [PhysicalQueryValue::Text(path), PhysicalQueryValue::Text(revision)] = row
                else {
                    malformed = true;
                    return Ok(std::ops::ControlFlow::Break(()));
                };
                if !expected
                    .get(at)
                    .is_some_and(|(wanted_path, wanted_revision)| {
                        wanted_path == path && wanted_revision == revision
                    })
                {
                    matches = false;
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                at += 1;
                Ok(std::ops::ControlFlow::Continue(()))
            },
        );
        read.map_err(|error| sql_or_cancelled(&self.snapshot, error))?;
        if self.snapshot.cancellation().is_cancelled() {
            return Err(ResultReadError::Cancelled);
        }
        if malformed {
            return Err(ResultReadError::Corrupt(
                "publication source fingerprint is absent or malformed".into(),
            ));
        }
        Ok(matches && at == expected.len())
    }

    /// Registry input and selection share this owned transaction. This scans
    /// metadata, not result payload; inference remains build_registry's job.
    pub(crate) fn read_registry(
        &mut self,
        config: &ParseConfig,
    ) -> Result<Arc<crate::query::registry::Registry>, crate::query::QueryExecutionError> {
        #[cfg(test)]
        REGISTRY_READ_ATTEMPTS.with(|count| count.set(count.get() + 1));
        let capture =
            self.registry
                .as_ref()
                .ok_or(crate::query::QueryExecutionError::Unavailable(
                    crate::query::QueryUnavailableReason::InvalidSnapshot,
                ))?;
        let built = capture.build(&mut self.snapshot, config)?;
        let mut owner = self.registry_owner.lock().unwrap();
        // A failed/replaced producer cannot seed its successor from this job.
        // The captured result itself remains coherent with the owned read.
        match owner.as_mut() {
            Some(owner) => owner.cache.publish(capture.clone(), built),
            None => Ok(built),
        }
    }
}

#[cfg(test)]
impl DirectQueryJob {
    /// True once a drain (rebuild, reset, close) has cancelled this job. The
    /// production read checks the snapshot's own sticky flag between batches;
    /// this is the slot's view, for the drain tests.
    pub(crate) fn is_cancelled(&self) -> bool {
        self._slot.is_cancelled()
    }
}

/// What one attempt to open a query job produced (R3; the §5.9 states plus
/// the two the job owner adds).
pub(crate) enum QueryJobOpen {
    Job(DirectQueryJob),
    /// Not ready at this generation, or the generation moved while the
    /// snapshot was being pinned. Nothing is wrong with the projection;
    /// `ProjectionProgress` decides whether readiness is on its way.
    NotReady,
    /// No capacity slot freed within the admission wait (R3). Distinct from
    /// `NotReady`: the projection IS ready and other jobs are draining, so the
    /// caller owes a retry and never a repair.
    Busy,
    /// The snapshot could not be opened or the regex program could not be
    /// installed: a failed read, owed recovery.
    Failed,
    /// A drain or close cancelled the job before it ran. No recovery is owed
    /// against a projection that is being replaced on purpose.
    Cancelled,
}

/// Whether a query that found the projection NOT READY can expect readiness to
/// arrive on its own, needs one repair, or must stop retrying (RET2).
///
/// The vocabulary is deliberately the queue's own: this reads the existing
/// `pending` queue plus `worker_available` / `worker_failed` / `worker_busy`
/// and translates them into the three answers the public boundary can act on.
/// It adds no state of its own, because a second opinion about whether the
/// worker is making progress is exactly the twin D-14 forbids.
#[derive(Debug)]
pub(crate) enum ProjectionProgress {
    /// Ready at this generation by the time the question was asked: the two
    /// reads straddled a save. Retryable.
    Ready,
    /// Queued or in-flight work will publish readiness. Retryable, with the
    /// reason the queue is holding it.
    Working(crate::query::QueryReadinessReason),
    /// Nothing is queued, the worker is idle, and the projection is stale at
    /// this generation. Only a repair can make it ready.
    Stale,
    /// The worker thread is gone — it never started, lost the writer lease, or
    /// returned. No repair this graph can schedule will be picked up, so a
    /// retry loop here would never end.
    Stopped,
}

/// Whether the projection can narrow THIS reference target at all.
///
/// A `Plain` (unlinked) target with no alphanumeric character has no usable
/// posting to look up, so the index cannot say which pages might contain it and
/// the exact parser walk is the only answer. This is a property of the TARGET,
/// not of the projection's readiness, which is why it is a free function: the
/// caller that decides between "wait for the index" and "walk every page" must
/// ask the same question the read itself asks, and one definition is the only
/// way those two can stay in agreement.
pub(crate) fn reference_narrowing_supported(names_norm: &[String], kind: ReferenceKind) -> bool {
    kind != ReferenceKind::Plain
        || names_norm
            .iter()
            .all(|name| name.chars().any(char::is_alphanumeric))
}

/// One reference target's candidate set, as the index can name it.
///
/// `blocks` is `Some` only when the index enumerated the referring blocks for
/// EVERY name in the target's equivalence class. A `None` there means "classify
/// every block of every candidate page", which is what the walk did before this
/// existed, so a partial index can never silently drop a row.
pub(crate) struct ReferenceCandidateIndex {
    pub paths: std::collections::BTreeSet<PathBuf>,
    pub blocks: Option<std::collections::HashSet<String>>,
    /// Page entities admitted by an Interactive verified window. `None` keeps
    /// the established Exhaustive/explicit/fallback behavior: every loaded
    /// candidate page may contribute its property preamble. A containing path
    /// admitted only by one of its blocks is deliberately absent from `Some`.
    pub page_owners: Option<std::collections::HashSet<PathBuf>>,
}

#[cfg(test)]
thread_local! {
    static PLAIN_REFERENCE_EXACT_CALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PLAIN_REFERENCE_QUERY_PLANS: std::cell::RefCell<Vec<Vec<String>>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn reset_plain_reference_query_instrumentation() {
    PLAIN_REFERENCE_EXACT_CALLBACKS.with(|count| count.set(0));
    PLAIN_REFERENCE_QUERY_PLANS.with(|plans| plans.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn plain_reference_query_instrumentation() -> (usize, Vec<Vec<String>>) {
    let callbacks = PLAIN_REFERENCE_EXACT_CALLBACKS.with(std::cell::Cell::get);
    let plans = PLAIN_REFERENCE_QUERY_PLANS.with(|plans| plans.borrow().clone());
    (callbacks, plans)
}

#[cfg(test)]
fn capture_plain_reference_query_plan(
    path: &Path,
    sql: &str,
    params: &[PhysicalQueryValue],
) -> Option<()> {
    let plan_reader = tine_storage::sqlite::PhysicalProjectionQueryReader::open(path).ok()?;
    plan_reader.set_query_rank_function(|_, _| Ok(None)).ok()?;
    let plan = plan_reader.explain_query_plan(sql, params).ok()?;
    PLAIN_REFERENCE_QUERY_PLANS.with(|plans| plans.borrow_mut().push(plan));
    Some(())
}

/// Direct Files' disposable parser-fact projection.
///
/// The foreground only publishes already-parsed `Arc<Document>` snapshots into
/// a coalescing page map. One worker owns SQLite, so an editor save never waits
/// for schema work, SQL, disk flushes, or a graph-sized rebuild. Read paths may
/// use the database only at the exact current cache generation.
pub(crate) struct DirectProjection {
    shared: Arc<ProjectionShared>,
}

impl DirectProjection {
    /// Subscribe the existing application watcher before observing the latest
    /// published image, so a commit cannot fall between registration and read.
    pub(crate) fn observe_commits(&self, wake: std::sync::mpsc::Sender<()>) -> u64 {
        let mut notifier = self.shared.commit_waker.lock().unwrap();
        *notifier = Some(wake);
        self.shared.commit_notification.load(Ordering::Acquire)
    }

    pub(crate) fn start(path: PathBuf) -> std::io::Result<Self> {
        let shared = Arc::new(ProjectionShared {
            path,
            pending: Mutex::new(PendingProjection::default()),
            changed: Condvar::new(),
            ready: AtomicBool::new(false),
            ready_generation: AtomicU64::new(0),
            commit_notification: AtomicU64::new(0),
            commit_waker: Mutex::new(None),
            reader: Mutex::new(None),
            query_jobs: Arc::new(QueryJobOwner::new(DEFAULT_QUERY_JOB_CAPACITY)),
            session_pages: Mutex::new(Arc::new(HashSet::new())),
            committed_registry: Arc::new(Mutex::new(None)),
            worker_available: AtomicBool::new(true),
            worker_failed: AtomicBool::new(false),
            worker_busy: AtomicBool::new(false),
            worker_building: AtomicBool::new(false),
            worker_finished: AtomicBool::new(false),
            worker_resources: Mutex::new(Some(Vec::new())),
            validated: AtomicBool::new(false),
            #[cfg(test)]
            after_sql_commit: Mutex::new(None),
            #[cfg(test)]
            after_fresh_build_batch: Mutex::new(None),
            #[cfg(test)]
            before_fresh_publication: Mutex::new(None),
            #[cfg(test)]
            after_fresh_publication: Mutex::new(None),
            #[cfg(test)]
            before_shared_reader_admission: Mutex::new(None),
            #[cfg(test)]
            after_shared_reader_admission: Mutex::new(None),
            #[cfg(test)]
            before_shared_reader_drain_lock: Mutex::new(None),
            #[cfg(test)]
            after_shared_reader_drain_lock: Mutex::new(None),
            #[cfg(test)]
            serving_writer_cache_budget: AtomicU64::new(0),
            #[cfg(test)]
            projection_health_checks: AtomicU64::new(0),
            #[cfg(test)]
            capture_thread: Mutex::new(None),
            #[cfg(test)]
            indexed_reads: AtomicU64::new(0),
            #[cfg(test)]
            statement_reads: AtomicU64::new(0),
            #[cfg(test)]
            registry_capture_attempts: AtomicU64::new(0),
            repairs_in_flight: AtomicUsize::new(0),
            warms_in_flight: AtomicUsize::new(0),
            build_progress: Default::default(),
            #[cfg(test)]
            inject_read_failure: AtomicBool::new(false),
            #[cfg(test)]
            fallback_reads: AtomicU64::new(0),
            #[cfg(test)]
            referenced_name_reads: AtomicU64::new(0),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("tine-direct-projection".into())
            .spawn(move || projection_worker(worker))?;
        Ok(Self { shared })
    }

    /// Keep repair requested until a complete source inventory or parser snapshot arrives.
    /// Publish that a repair is computing its payload. `progress_at` reports
    /// `Working(Recovering)` for as long as the returned guard lives, so a
    /// concurrent query waits for it instead of declaring the repair failed.
    pub(crate) fn begin_repair(&self) -> RepairInFlight {
        self.shared.repairs_in_flight.fetch_add(1, Ordering::AcqRel);
        RepairInFlight(Arc::clone(&self.shared))
    }

    /// Announce a warm validation BEFORE it reads a single page byte, so a
    /// query landing during that read reports `NotReady(Indexing)` and
    /// retries instead of repairing an "idle" projection (GH #543). Held
    /// until the warm has enqueued (from then on the queue itself says
    /// Indexing) or given up.
    pub(crate) fn begin_warm(&self) -> WarmInFlight {
        self.shared.warms_in_flight.fetch_add(1, Ordering::AcqRel);
        WarmInFlight(Arc::clone(&self.shared))
    }

    /// True while some thread has announced a warm and not yet finished it.
    /// `projection_recovery` serializes repairs against each other but not
    /// against the open path's warm, so a query arriving during a cold open
    /// could start a SECOND warm of the same graph (GH #543).
    pub(crate) fn warm_in_flight(&self) -> bool {
        self.shared.warms_in_flight.load(Ordering::Acquire) > 0
    }

    /// True while the last worker turn failed. The flag clears on the next
    /// successful turn, so it names a projection that owes a reset — not one
    /// that has merely never started.
    pub(crate) fn worker_failed(&self) -> bool {
        self.shared.worker_failed.load(Ordering::Acquire)
    }

    /// True while the writer worker accepts work (it holds the lease and has
    /// not been told to stop).
    pub(crate) fn worker_available(&self) -> bool {
        self.shared.worker_available.load(Ordering::Acquire)
    }

    pub(crate) fn request_rebuild(&self) {
        let mut pending = self.shared.pending.lock().unwrap();
        pending.rebuild = true;
        self.shared.ready.store(false, Ordering::Release);
    }

    /// Withdraw a rebuild request that found no payload to carry it.
    ///
    /// `rebuild` is an obligation, not a state: [`Self::request_rebuild`]
    /// promises a full snapshot or a warm inventory will follow, because the
    /// worker consumes the flag ONLY beside one of those two payloads, and
    /// [`PendingProjection::has_work`] does not count it. A request nobody can
    /// discharge is therefore permanent: the worker sleeps, every capture is
    /// refused by [`query_capture_admissible`], and [`Self::progress_at`]
    /// reports `Working(Recovering)` forever — the user watches "Rebuilding
    /// the search index…" on an idle process (GH #543).
    ///
    /// That is reachable without any filesystem activity: a query read fails,
    /// repair sets the flag, and a newer producer refuses the payload that was
    /// going to carry it. Withdrawing lets the next query's repair try again;
    /// holding the flag does not.
    ///
    /// Returns whether the request was withdrawn. A payload that arrived in
    /// the meantime owns the rebuild, so the flag stays.
    pub(crate) fn withdraw_rebuild_request(&self) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.full.is_some() || pending.warm.is_some() {
            return false;
        }
        let withdrawn = std::mem::take(&mut pending.rebuild);
        if withdrawn {
            projection_diag(|| "rebuild request withdrawn: no payload followed".to_owned());
        }
        withdrawn
    }

    pub(crate) fn enqueue_full(
        &self,
        generation: u64,
        pages: PageSnapshot,
        revisions: PageRevisions,
        parse_config: Arc<ParseConfig>,
        source_complete: bool,
    ) {
        let mut pending = self.shared.pending.lock().unwrap();
        // A snapshot older than the queue is not a supersession, it is a
        // rollback. `install_built` validates the generation under the cache
        // write lock and then RELEASES that lock before enqueueing here, so a
        // save can publish G+1 and queue its delta in the gap; applying this
        // snapshot would then answer search from text the user has already
        // replaced, and `latest_generation` would say otherwise.
        //
        // A pending rebuild is NOT an exception to that, though it was written
        // as one. The fear was that refusing the payload leaves an emptied
        // index with nothing queued to fill it — but `Database::reset` has a
        // single call site, inside the worker turn that consumes the rebuild
        // BESIDE this payload (`a_reset_has_one_call_site_and_the_worker_owns_it`).
        // Nothing has been reset when the payload is refused, so the index the
        // refusal keeps is the one the deltas have been maintaining all along.
        //
        // Keeping the payload and replaying the queued deltas on top of it is
        // not enough either: the worker DRAINS a delta the instant it takes
        // the turn, so a save that has already been applied is in neither the
        // queue nor the snapshot, and the reset erased it while readiness was
        // still published at the newer watermark — search then answered from
        // the old text with no producer queued to correct it, and the file on
        // disk disagreed with the index indefinitely (third audit A3-N1).
        //
        // So refuse, and withdraw the obligation with the payload it promised:
        // a rebuild latched with nothing to ride in on refuses every later
        // query for the lifetime of the graph (GH #543, B1). The next repair
        // assembles a snapshot at the current generation and rides in on that.
        if generation < pending.latest_generation {
            let withdrawn = std::mem::take(&mut pending.rebuild);
            projection_diag(|| {
                format!(
                    "full refused: snapshot generation={generation} older than queue {}{}",
                    pending.latest_generation,
                    if withdrawn {
                        "; rebuild request withdrawn with it"
                    } else {
                        ""
                    }
                )
            });
            return;
        }
        self.shared.ready.store(false, Ordering::Release);
        self.shared.worker_failed.store(false, Ordering::Release);
        pending.seed_page_order(pages.iter().map(|(entry, _)| entry.rel_path.as_str()));
        pending.full = Some(PendingFull {
            pages,
            revisions,
            parse_config,
            source_complete,
        });
        pending.deltas.clear();
        pending.latest_generation = generation;

        // A complete parsed snapshot owns readiness from here. A validation
        // still in flight is superseded and its waiter is released.
        if pending.warm.take().is_some() {
            pending.warm = None;
            pending.warm_outcome = Some((pending.warm_attempt, WarmOutcome::Superseded));
        }
        self.shared.changed.notify_all();
    }

    /// R6 warm validation: hand the worker the walk inventory with exact
    /// content revisions and nothing parsed. Refused (`None`) when a newer
    /// mutation or a full snapshot already outranks this generation — the
    /// caller then leaves readiness to the parser fallback, exactly as
    /// `install_built` does on generation drift.
    ///
    /// Queued page updates at or below this generation do not refuse it. The
    /// caller accepted them as matching what it read, and the worker takes
    /// them in the same turn, validating the warm first and applying the
    /// updates after it. Refusing turned every page opened at launch into a
    /// whole-graph parse whenever the worker had not yet taken its update
    /// (GH #543, audit IT-03).
    #[cfg(test)]
    pub(crate) fn enqueue_warm(
        &self,
        generation: u64,
        sources: Vec<(PageEntry, String)>,
        retained: Vec<PageEntry>,
        published: Vec<String>,
        walk_order: Vec<String>,
        parse_config: Arc<ParseConfig>,
        text_bytes: u64,
    ) -> Option<u64> {
        self.enqueue_warm_with_repair(
            generation,
            sources,
            retained,
            published,
            walk_order,
            None,
            parse_config,
            text_bytes,
        )
    }

    /// `enqueue_warm`, carrying the pages a previous attempt named `Changed`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_warm_with_repair(
        &self,
        generation: u64,
        sources: Vec<(PageEntry, String)>,
        retained: Vec<PageEntry>,
        published: Vec<String>,
        walk_order: Vec<String>,
        repair: Option<WarmRepair>,
        parse_config: Arc<ParseConfig>,
        _text_bytes: u64,
    ) -> Option<u64> {
        if !self.shared.worker_available.load(Ordering::Acquire) {
            return None;
        }
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.full.is_some()
            || pending.warm.is_some()
            || pending.latest_generation > generation
            || (self.shared.worker_failed.load(Ordering::Acquire) && !pending.rebuild)
        {
            projection_diag(|| {
                format!(
                    "warm refused generation={generation} full={} deltas={} warm={} latest={} failed={}",
                    pending.full.is_some(),
                    pending.deltas.len(),
                    pending.warm.is_some(),
                    pending.latest_generation,
                    self.shared.worker_failed.load(Ordering::Acquire),
                )
            });
            return None;
        }
        let sources_len = sources.len();
        self.shared.ready.store(false, Ordering::Release);
        // The walk's own order, pages it could not read included: the image
        // stores positions in that order, and appending the unreadable pages
        // instead shifted every page after them onto a position another page
        // still held (GH #543).
        pending.seed_page_order(walk_order.iter().map(String::as_str));
        pending.place_unseeded_deltas();
        pending.warm_outcome = None;
        pending.warm_attempt += 1;
        let attempt = pending.warm_attempt;
        pending.warm = Some(PendingWarm {
            sources,
            retained,
            published,
            repair,
            parse_config,
        });
        pending.latest_generation = generation;
        self.shared.changed.notify_all();
        projection_diag(|| {
            format!(
                "warm queued attempt={attempt} generation={generation} pages={sources_len} text_mib={:.1}",
                _text_bytes as f64 / (1024.0 * 1024.0)
            )
        });
        Some(attempt)
    }

    /// Block until the worker has decided the warm `attempt` asked for.
    ///
    /// `attempt` is the id [`Self::enqueue_warm`] returned. A waiter only ever
    /// takes ITS OWN verdict: the slot used to be a single unkeyed value, so a
    /// warm descheduled before reading it could take a later attempt's verdict
    /// and leave that later attempt waiting for a producer that had already
    /// run. Nothing scheduled another — and the loser holds the process-wide
    /// warm mutex, so every later graph warm queued behind it and the app
    /// stopped indexing altogether (GH #543, re-audit A2-B1).
    pub(crate) fn wait_warm_outcome(&self, attempt: u64) -> WarmOutcome {
        let mut pending = self.shared.pending.lock().unwrap();
        loop {
            match pending.warm_outcome.as_ref().map(|(id, _)| *id) {
                // This attempt's own verdict.
                Some(id) if id == attempt => {
                    return pending.warm_outcome.take().expect("just observed").1;
                }
                // Somebody else's. Leave it for them; a verdict for a LATER
                // attempt also proves this one was superseded.
                Some(_) => return WarmOutcome::Superseded,
                None => {}
            }
            if !self.shared.worker_available.load(Ordering::Acquire) || pending.stop {
                return WarmOutcome::Failed;
            }
            // A later warm was admitted, which cleared the slot and took over
            // validation. This attempt's verdict is never coming.
            if pending.warm_attempt != attempt {
                projection_diag(|| {
                    format!(
                        "warm attempt={attempt} superseded by attempt={}",
                        pending.warm_attempt
                    )
                });
                return WarmOutcome::Superseded;
            }
            // Belt and braces for an attempt that is still current but has
            // nothing left to produce its verdict: nothing queued and an idle
            // worker means no outcome is coming.
            if pending.warm.is_none() && !self.shared.worker_busy.load(Ordering::Acquire) {
                projection_diag(|| {
                    "warm outcome taken by another attempt; nothing left to wait for".to_owned()
                });
                return WarmOutcome::Superseded;
            }
            pending = self.shared.changed.wait(pending).unwrap();
        }
    }

    /// R6: the projected page inventory as `(name, path, text_kind)` rows,
    /// read through `drain_after` from the ready projection. `list_pages`
    /// rebuilds `PageEntry`s from it instead of parsing every file.
    pub(crate) fn page_inventory(
        &self,
        cache_generation: u64,
    ) -> Option<Vec<(String, String, i64)>> {
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let mut rows = Vec::new();
        drain_after(
            |cursor: Option<(i64, String)>, batch| {
                read.navigation_pages_after_with_header_validation(
                    cursor.as_ref().map(|(_, path)| path.as_str()),
                    cursor.as_ref().map(|(id, _)| *id),
                    batch,
                    |_, kind| match kind {
                        0 | 1 => Ok(()),
                        _ => Err(tine_storage::sqlite::MaterializationError::Corrupt(
                            format!("unknown Direct Files text kind {kind}"),
                        )),
                    },
                )
            },
            |row| (row.cursor, row.path.clone()),
            |row| {
                rows.push((row.name, row.path, row.text_kind));
                Ok(())
            },
            |error, batch| {
                matches!(
                    error,
                    tine_storage::sqlite::MaterializationError::ResourceLimit { .. }
                )
                .then(|| (batch / 2).max(1))
            },
        )
        .ok()?;
        self.ready_at(cache_generation).then_some(rows)
    }

    /// Bounded wait for readiness at `generation` (R6): the whole-graph derived
    /// reads that would otherwise fall to a full parse in the milliseconds
    /// after a save or a warm turn wait for that bounded worker turn first.
    /// Same ceiling and same non-authority as `wait_for_reference_generation`.
    pub(crate) fn wait_ready_at(&self, generation: u64) -> bool {
        self.wait_for_reference_generation(generation)
    }

    /// Unbounded wait for readiness at `generation`, for the background warm
    /// task only (GH #543): a validation or staged build on a large graph can
    /// outlast the bounded wait used by derived-map reads. Returns `false` when
    /// readiness at this generation is no longer coming: a newer generation,
    /// the worker gone or failed, an idle queue that did not publish, or
    /// `cancelled`.
    pub(crate) fn wait_until_ready_at(
        &self,
        generation: u64,
        cancelled: &impl Fn() -> bool,
    ) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        loop {
            if self.ready_at(generation) {
                return true;
            }
            if cancelled()
                || pending.stop
                || !self.shared.worker_available.load(Ordering::Acquire)
                || self.shared.worker_failed.load(Ordering::Acquire)
                || pending.latest_generation > generation
            {
                return false;
            }
            if !pending.has_work() && !self.shared.worker_busy.load(Ordering::Acquire) {
                return false;
            }
            pending = self
                .shared
                .changed
                .wait_timeout(pending, std::time::Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }

    pub(crate) fn enqueue_replace(
        &self,
        generation: u64,
        entry: PageEntry,
        document: Arc<Document>,
        revision: String,
        parse_config: Arc<ParseConfig>,
    ) {
        self.enqueue_delta(
            generation,
            PageDelta::Replace {
                entry,
                document,
                revision,
                parse_config,
                page_position: None, // Filled under the queue lock, before coalescing.
            },
        );
    }

    pub(crate) fn enqueue_delete(&self, generation: u64, entry: PageEntry) {
        self.enqueue_delta(generation, PageDelta::Delete { entry });
    }

    /// One page-set change published as ONE queue transaction.
    ///
    /// The worker drains whatever is queued the moment it wakes, so a producer
    /// that enqueues its deltas one at a time can have the queue empty
    /// underneath it: a rename's `Delete` was drained on its own, the watermark
    /// already read as the new generation, and readiness was published over an
    /// image whose `Replace` had not been enqueued yet — search answered
    /// "complete" over a graph that was missing the page entirely (fourth audit
    /// A4-N2). Taking the lock once makes the whole change one step.
    ///
    /// This does not gate ADMISSION to an existing complete committed image:
    /// ordinary pending deltas may keep serving that coherent older image. It
    /// gates only the claim that the current generation is complete.
    pub(crate) fn enqueue_page_set(
        &self,
        generation: u64,
        changes: Vec<PageSetChange>,
        parse_config: Arc<ParseConfig>,
    ) {
        if changes.is_empty() {
            return;
        }
        self.shared.ready.store(false, Ordering::Release);
        let mut pending = self.shared.pending.lock().unwrap();
        for change in changes {
            let delta = match change {
                PageSetChange::Replace {
                    entry,
                    document,
                    revision,
                } => PageDelta::Replace {
                    entry,
                    document,
                    revision,
                    parse_config: Arc::clone(&parse_config),
                    page_position: None, // Filled under this lock, before coalescing.
                },
                PageSetChange::Delete { entry } => PageDelta::Delete { entry },
            };
            pending.record_delta(generation, delta);
        }
        self.shared.changed.notify_all();
    }

    fn enqueue_delta(&self, generation: u64, delta: PageDelta) {
        self.shared.ready.store(false, Ordering::Release);
        let mut pending = self.shared.pending.lock().unwrap();
        pending.record_delta(generation, delta);
        self.shared.changed.notify_one();
    }

    pub(crate) fn mark_stale(&self) {
        self.shared.ready.store(false, Ordering::Release);
        // Source-oriented navigation waits for reconciliation; live queries
        // can still read the complete committed image.
    }

    /// A reference read which races an already-queued one-page fact delta is
    /// much cheaper if it waits for that bounded worker turn than if it scans
    /// every parsed page. The timeout is a latency ceiling, not an authority:
    /// failure, worker loss, a newer generation, or expiry all return `false`
    /// and the caller uses the exact parser fallback.
    pub(crate) fn wait_for_reference_generation(&self, generation: u64) -> bool {
        if self.ready_at(generation) {
            return true;
        }
        let deadline = std::time::Instant::now() + REFERENCE_DELTA_WAIT;
        let mut pending = self.shared.pending.lock().unwrap();
        loop {
            if self.ready_at(generation) {
                return true;
            }
            if !self.shared.worker_available.load(Ordering::Acquire)
                || self.shared.worker_failed.load(Ordering::Acquire)
                || self.shared.ready_generation.load(Ordering::Acquire) > generation
                || pending.latest_generation > generation
            {
                return false;
            }
            if !pending.has_work() && !self.shared.worker_busy.load(Ordering::Acquire) {
                return false;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let (next, timeout) = self
                .shared
                .changed
                .wait_timeout(pending, deadline - now)
                .unwrap();
            pending = next;
            if timeout.timed_out() && !self.ready_at(generation) {
                return false;
            }
        }
    }

    pub(crate) fn property_facets(
        &self,
        cache_generation: u64,
        autocomplete: bool,
        hidden_properties: &[String],
        max_items: usize,
        max_bytes: usize,
    ) -> Option<(Vec<(String, Vec<String>)>, bool)> {
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let mut accumulator = if autocomplete {
            PropertyFacetAccumulator::autocomplete(hidden_properties, max_items, max_bytes)
        } else {
            PropertyFacetAccumulator::query_builder(max_items, max_bytes)
        };
        drain_after(
            |cursor, batch| read.property_facet_rows_after(!autocomplete, cursor, batch),
            |row| {
                let owner = match row.owner {
                    PhysicalEntityId::Page(_) => PhysicalEntityCoordinate::Page(row.owner_cursor),
                    PhysicalEntityId::Block(_) => PhysicalEntityCoordinate::Block(row.owner_cursor),
                };
                (owner, row.name_cursor, row.ordinal)
            },
            |row| {
                accumulator.offer(&row.normalized_name, &row.value);
                Ok(())
            },
            |error, batch| {
                matches!(
                    error,
                    tine_storage::sqlite::MaterializationError::ResourceLimit { .. }
                )
                .then(|| (batch / 2).max(1))
            },
        )
        .ok()?;
        if !self.ready_at(cache_generation) {
            return None;
        }
        #[cfg(test)]
        self.shared.indexed_reads.fetch_add(1, Ordering::Relaxed);
        Some(accumulator.finish())
    }

    /// The §6.2 registry row source for **Direct Files, projection ready**: the
    /// ready raw property stream plus the same-snapshot `page_id → (format,
    /// name)` map, taken under ONE read of the projection database.
    ///
    /// **CLOSURE §4 rejects deferring this to the document walk.** The wrapper
    /// above (`property_facets`) aggregates owner identity away, so it cannot
    /// serve a registry that reports cardinality and distinct-owner counts; and
    /// answering a registry read by walking every hydrated document is exactly
    /// the graph-wide scan the ready projection exists to avoid. This is an
    /// ADAPTER onto the one `build_registry` aggregator, not a competing
    /// registry producer: it yields the same [`OwnerRow`] stream the cold document
    /// iterator yields, and the
    /// aggregator downstream is byte-for-byte the same function.
    ///
    /// `None` means "not ready, or the read refused" — the caller falls back to
    /// the document iterator, exactly as §5.9's dispatch does for queries.
    #[cfg(test)]
    pub(crate) fn property_owner_rows(
        &self,
        cache_generation: u64,
    ) -> Option<(
        Vec<crate::query::registry::OwnerRow>,
        HashMap<String, crate::query::registry::PageMeta>,
    )> {
        use crate::query::registry::{OwnerRow, OwnerType, PageMeta};
        #[cfg(test)]
        REGISTRY_READ_ATTEMPTS.with(|count| count.set(count.get() + 1));

        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();

        // The page map and the rows are read from the SAME `read`, i.e. the same
        // snapshot: a row naming a page the map does not have is a
        // snapshot-consistency defect and fails the build (§6.2), never a
        // silent fallback to Markdown.
        let mut pages: HashMap<String, PageMeta> = HashMap::new();
        let mut page_ids = HashMap::new();
        drain_after(
            |cursor: Option<(i64, String)>, batch| {
                read.navigation_pages_after_with_header_validation(
                    cursor.as_ref().map(|(_, path)| path.as_str()),
                    cursor.as_ref().map(|(id, _)| *id),
                    batch,
                    |_, kind| match kind {
                        0 | 1 => Ok(()),
                        _ => Err(tine_storage::sqlite::MaterializationError::Corrupt(
                            format!("unknown Direct Files text kind {kind}"),
                        )),
                    },
                )
            },
            |row| (row.cursor, row.path.clone()),
            |row| {
                page_ids.insert(row.path.clone(), row.cursor);
                pages.insert(
                    crate::query::registry_sql::page_key(row.cursor),
                    PageMeta {
                        // §6.2 E4: `Format::from_path`, case-insensitive —
                        // never `reference_source_is_org`.
                        format: Format::from_path(Path::new(&row.path)).into(),
                        name: row.name,
                    },
                );
                Ok(())
            },
            |error, batch| {
                matches!(
                    error,
                    tine_storage::sqlite::MaterializationError::ResourceLimit { .. }
                )
                .then(|| (batch / 2).max(1))
            },
        )
        .ok()?;

        let mut rows: Vec<OwnerRow> = Vec::new();
        drain_after(
            |cursor, batch| read.property_facet_rows_after(false, cursor, batch),
            |row| {
                let owner = match row.owner {
                    PhysicalEntityId::Page(_) => PhysicalEntityCoordinate::Page(row.owner_cursor),
                    PhysicalEntityId::Block(_) => PhysicalEntityCoordinate::Block(row.owner_cursor),
                };
                (owner, row.name_cursor, row.ordinal)
            },
            |row| {
                let (owner_type, owner_id) = match row.owner {
                    PhysicalEntityId::Page(_) => {
                        (OwnerType::Page, format!("p:{}", row.owner_cursor))
                    }
                    PhysicalEntityId::Block(_) => {
                        (OwnerType::Block, format!("b:{}", row.owner_cursor))
                    }
                };
                let page_id = page_ids.get(&row.page_path).ok_or_else(|| {
                    tine_storage::sqlite::MaterializationError::Corrupt(
                        "registry property names an absent page path".into(),
                    )
                })?;
                rows.push(OwnerRow {
                    owner_type,
                    owner_id,
                    page_id: crate::query::registry_sql::page_key(*page_id),
                    source_name: row.source_name,
                    normalized_name: row.normalized_name,
                    ordinal: row.ordinal,
                    value: row.value,
                });
                Ok(())
            },
            |error, batch| {
                matches!(
                    error,
                    tine_storage::sqlite::MaterializationError::ResourceLimit { .. }
                )
                .then(|| (batch / 2).max(1))
            },
        )
        .ok()?;

        // The generation must still hold AFTER both scans, or the two halves
        // could straddle a rebuild — the same re-check `property_facets` makes.
        if !self.ready_at(cache_generation) {
            return None;
        }
        #[cfg(test)]
        self.shared.indexed_reads.fetch_add(1, Ordering::Relaxed);
        Some((rows, pages))
    }

    /// R3: open a database-owned query job at the current cache generation.
    ///
    /// Capacity is acquired on the caller before enqueueing a capture. The
    /// producer opens the snapshot and captures session identity between write
    /// turns, validating `ready_at(generation)` around the transaction. A
    /// registry-sensitive request also freezes its registry input there; an
    /// insensitive request carries none. The producer registers cancellation
    /// before handing the owned job back. Selection and payload construction
    /// then execute on the caller, off the producer.
    /// The statement's compiled-regex program is installed by
    /// `query::results::read_results` on the job's own connection — the ONE
    /// install site — so a job carries no regex state of its own.
    #[cfg(test)]
    pub(crate) fn open_query_job_for(
        &self,
        cache_generation: u64,
        registry_sensitivity: RegistrySensitivity,
    ) -> QueryJobOpen {
        if !self.ready_at(cache_generation) {
            return QueryJobOpen::NotReady;
        }
        self.enqueue_query_capture(
            QueryCaptureRequirement::StrictGeneration(cache_generation),
            registry_sensitivity,
        )
    }

    /// Existing reader lifecycle identity; ordinary saves do not advance it.
    pub(crate) fn query_epoch(&self) -> crate::query_jobs::QueryJobEpoch {
        self.shared.query_jobs.capture_epoch()
    }

    /// Acquire the current complete projection without waiting for a saved edit.
    /// Ordinary queued deltas do not invalidate the committed image.
    pub(crate) fn open_current_query_job(
        &self,
        registry_sensitivity: RegistrySensitivity,
    ) -> QueryJobOpen {
        if !self.shared.worker_available.load(Ordering::Acquire)
            || !query_capture_available(&self.shared, &QueryCaptureRequirement::CurrentSnapshot)
        {
            return QueryJobOpen::NotReady;
        }
        self.enqueue_query_capture(
            QueryCaptureRequirement::CurrentSnapshot,
            registry_sensitivity,
        )
    }

    fn enqueue_query_capture(
        &self,
        requirement: QueryCaptureRequirement,
        registry_sensitivity: RegistrySensitivity,
    ) -> QueryJobOpen {
        #[cfg(test)]
        if self
            .shared
            .inject_read_failure
            .swap(false, Ordering::AcqRel)
        {
            return QueryJobOpen::Failed;
        }
        let slot = match self
            .shared
            .query_jobs
            .acquire_owned_at_within(self.shared.query_jobs.capture_epoch(), QUERY_JOB_WAIT)
        {
            OwnedAdmission::Slot(slot) => slot,
            OwnedAdmission::Cancelled => return QueryJobOpen::Cancelled,
            // RET2: capacity, not readiness. The projection is ready and other
            // jobs are draining, so this is the one `NotReady` the public
            // boundary may retry without ever considering a repair.
            OwnedAdmission::Busy => return QueryJobOpen::Busy,
        };
        let (reply, result) = std::sync::mpsc::sync_channel(1);
        {
            let mut pending = self.shared.pending.lock().unwrap();
            if pending.stop || slot.is_cancelled() {
                return QueryJobOpen::Cancelled;
            }
            if !self.shared.worker_available.load(Ordering::Acquire) {
                return QueryJobOpen::Failed;
            }
            let available = match &requirement {
                QueryCaptureRequirement::CurrentSnapshot => {
                    query_capture_admissible(&self.shared, &pending)
                }
                #[cfg(test)]
                QueryCaptureRequirement::StrictGeneration(generation) => self.ready_at(*generation),
            };
            if !available {
                return QueryJobOpen::NotReady;
            }
            pending.captures.push(PendingQueryCapture {
                requirement,
                registry_sensitivity,
                slot,
                reply,
            });
        }
        self.shared.changed.notify_all();
        result.recv().unwrap_or(QueryJobOpen::Failed)
    }

    /// Existing low-level fixtures exercise the stronger registry-bearing job.
    #[cfg(test)]
    pub(crate) fn open_query_job(&self, cache_generation: u64) -> QueryJobOpen {
        self.open_query_job_for(cache_generation, RegistrySensitivity::Required)
    }

    #[cfg(test)]
    pub(crate) fn session_pages_test(&self) -> Arc<HashSet<String>> {
        Arc::clone(&self.shared.session_pages.lock().unwrap())
    }

    #[cfg(test)]
    pub(crate) fn active_query_jobs_test(&self) -> usize {
        self.shared.query_jobs.active()
    }

    pub(crate) fn note_fallback_read(&self) {
        #[cfg(test)]
        self.shared.fallback_reads.fetch_add(1, Ordering::Relaxed);
    }

    /// Admit a pooled projection read under the same mutex publication drains.
    ///
    /// The fast check avoids taking the mutex for a known-stale generation, but
    /// the check under the mutex is authoritative: a caller paused after the
    /// first check must not reopen the destination after replacement withdrew
    /// readiness. The returned guard stays alive for the complete read, making
    /// every SQLite handle visible to the publication drain.
    fn shared_reader_at(
        &self,
        cache_generation: u64,
    ) -> Option<std::sync::MutexGuard<'_, Option<PhysicalGraphProjectionDatabase>>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        #[cfg(test)]
        if let Some(hook) = self
            .shared
            .before_shared_reader_admission
            .lock()
            .unwrap()
            .take()
        {
            hook();
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if !self.ready_at(cache_generation) {
            return None;
        }
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        reader.as_ref()?;
        #[cfg(test)]
        if let Some(hook) = self
            .shared
            .after_shared_reader_admission
            .lock()
            .unwrap()
            .take()
        {
            hook();
        }
        Some(reader)
    }

    pub(crate) fn referenced_page_names(&self, cache_generation: u64) -> Option<Vec<String>> {
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let mut names = std::collections::HashMap::<String, String>::new();
        drain_after(
            |after: Option<(String, String)>, batch| {
                read.navigation_reference_names_after(
                    after
                        .as_ref()
                        .map(|(normalized, raw)| (normalized.as_str(), raw.as_str())),
                    batch,
                )
            },
            |row| (row.normalized_name.clone(), row.raw_name.clone()),
            |row| {
                names
                    .entry(crate::refs::page_key(&row.raw_name))
                    .or_insert(row.raw_name);
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut names = names.into_values().collect::<Vec<_>>();
        names.sort_by_key(|name| crate::refs::page_key(name));
        #[cfg(test)]
        self.shared
            .referenced_name_reads
            .fetch_add(1, Ordering::Relaxed);
        Some(names)
    }

    pub(crate) fn page_aliases_with_owners(
        &self,
        cache_generation: u64,
    ) -> Option<Vec<(String, String, String)>> {
        let _reader = self.shared_reader_at(cache_generation)?;
        let mut aliases = Vec::new();
        let mut snapshot =
            PhysicalProjectionQuerySnapshot::open_direct(&self.shared.path, || Ok(())).ok()?;
        crate::query::projection_sql::visit(
            &mut snapshot,
            "SELECT alias.raw, owner.raw, p.path \
             FROM reference_alias_declarations d \
             JOIN pages p ON p.page_id = d.source_page_id \
             JOIN names owner ON owner.name_id = p.name_id \
             JOIN names alias ON alias.name_id = d.alias_name_id \
             WHERE d.source_entity_type = 0 AND d.source_entity_id = d.source_page_id \
             ORDER BY d.source_page_id, d.ordinal, alias.raw",
            &[],
            |row| {
                let values = row
                    .iter()
                    .map(|value| match value {
                        PhysicalQueryValue::Text(text) => Ok(text.clone()),
                        _ => Err(tine_storage::sqlite::MaterializationError::InvalidQuery(
                            "page alias ownership row contains non-text data".into(),
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if values.len() != 3 {
                    return Err(tine_storage::sqlite::MaterializationError::InvalidQuery(
                        "page alias ownership row has the wrong width".into(),
                    ));
                }
                aliases.push((values[0].clone(), values[1].clone(), values[2].clone()));
                Ok(std::ops::ControlFlow::Continue(()))
            },
        )
        .ok()?;
        self.ready_at(cache_generation).then_some(aliases)
    }

    pub(crate) fn real_page_names(
        &self,
        cache_generation: u64,
    ) -> Option<crate::query::RealPageNames> {
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let mut names = crate::query::RealPageNames::new();
        drain_after(
            |after: Option<(String, i64)>, batch| {
                read.navigation_pages_after_with_header_validation(
                    after.as_ref().map(|(path, _)| path.as_str()),
                    after.as_ref().map(|(_, id)| *id),
                    batch,
                    |_, _| Ok(()),
                )
            },
            |row| (row.path.clone(), row.cursor),
            |row| {
                let path = PathBuf::from(&row.path);
                match names.get_mut(&row.name_key) {
                    Some((winner_path, winner_name)) if path < *winner_path => {
                        *winner_path = path;
                        *winner_name = row.name;
                    }
                    Some(_) => {}
                    None => {
                        names.insert(row.name_key, (path, row.name));
                    }
                }
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        self.ready_at(cache_generation).then_some(names)
    }

    /// The candidate set for one reference target: the pages that may contain a
    /// match and, when the index can name them, the BLOCKS.
    ///
    /// The block set is not an optimization bolted on afterwards — it is what
    /// `page_referrer_candidates_after` already returns. Its rows are
    /// `(source_page_id, source_entity)` and the caller used to drop the
    /// entity, so the read narrowed to 184 pages and then re-parsed all 3,434
    /// of their blocks to find the 412 that referred to the target (measured on
    /// the anonymized graph; see `sql_gates_tests.rs`'s narrowing receipt).
    /// Carrying the entity through spends nothing extra in SQL and removes
    /// roughly nine of every ten per-block parses.
    ///
    /// The block set is a SUPERSET filter and never the answer: the parser
    /// still decides membership, exactly as before. It is `None` whenever the
    /// index cannot name blocks for this target, and then every block of every
    /// candidate page is classified as it was.
    pub(crate) fn reference_candidates(
        &self,
        cache_generation: u64,
        names_norm: &[String],
        self_page: &str,
        kind: ReferenceKind,
        mode: crate::query::candidate::CandidateMode,
        config: &crate::config::Config,
    ) -> Option<ReferenceCandidateIndex> {
        if !reference_narrowing_supported(names_norm, kind) {
            return None;
        }
        let reader = self.shared_reader_at(cache_generation)?;
        let mut paths = std::collections::BTreeSet::new();
        let mut blocks = std::collections::HashSet::new();
        let mut page_owners = matches!(
            (kind, mode),
            (
                ReferenceKind::Plain,
                crate::query::candidate::CandidateMode::Interactive { .. }
            )
        )
        .then(std::collections::HashSet::new);
        let read = reader.as_ref()?.read();
        // All title/alias needles observe one committed image. Reopening per
        // spelling could otherwise union candidates from opposite sides of an
        // edit even though each individual query was coherent.
        let mut plain_snapshot = if kind == ReferenceKind::Plain {
            Some(PhysicalProjectionQuerySnapshot::open_direct(&self.shared.path, || Ok(())).ok()?)
        } else {
            None
        };
        for name in names_norm {
            match kind {
                ReferenceKind::Explicit => {
                    drain_after(
                        |after, batch| read.page_referrer_candidates_after(name, after, batch),
                        |row| {
                            let entity = match row.source {
                                PhysicalEntityId::Page(_) => {
                                    PhysicalEntityCoordinate::Page(row.entity_cursor)
                                }
                                PhysicalEntityId::Block(_) => {
                                    PhysicalEntityCoordinate::Block(row.entity_cursor)
                                }
                            };
                            (row.page_cursor, entity)
                        },
                        |row| {
                            paths.insert(PathBuf::from(row.source_page_path));
                            match row.source {
                                PhysicalEntityId::Block(block_id) => {
                                    blocks.insert(block_id);
                                }
                                // A page-level posting names no block. The
                                // page-property pseudo-block it stands for is
                                // built from the page preamble and never
                                // classified through the block walk, so the
                                // block set stays complete for the walk.
                                PhysicalEntityId::Page(_) => {}
                            }
                            Ok(())
                        },
                        |_, _| None,
                    )
                    .ok()?;
                }
                ReferenceKind::Plain => {
                    let folded = crate::search_query::canonical_fold(name);
                    let plan = crate::query::candidate::scalar_trigram_expression(&folded);
                    let mut params = plan
                        .as_ref()
                        .map(|expression| vec![PhysicalQueryValue::Text(expression.clone())])
                        .unwrap_or_default();
                    let snapshot = plain_snapshot.as_mut()?;
                    let exclusions = crate::refs::ReferenceSourceExclusions::new(
                        self_page,
                        config.favorites_page.as_deref(),
                    );
                    let mut indexed_filters = Vec::new();
                    let mut scan_filters = Vec::new();
                    for key in exclusions.keys() {
                        params.push(PhysicalQueryValue::Text(key.clone()));
                        indexed_filters.push(format!("owner_key <> ?{}", params.len()));
                        scan_filters.push(format!("n.key <> ?{}", params.len()));
                    }
                    let indexed_filter = indexed_filters.join(" AND ");
                    let scan_filter = scan_filters.join(" AND ");
                    let mut limit_sql = String::new();
                    match mode {
                        crate::query::candidate::CandidateMode::Exhaustive => {}
                        crate::query::candidate::CandidateMode::Interactive { window } => {
                            let needle = vec![name.clone()];
                            let config = config.clone();
                            snapshot
                                .set_query_rank_function(move |entity_type, framed| {
                                    #[cfg(test)]
                                    PLAIN_REFERENCE_EXACT_CALLBACKS
                                        .with(|count| count.set(count.get().saturating_add(1)));
                                    let (raw, path) = crate::query::rank::decode_pair(framed)?;
                                    let is_org = Format::from_path(Path::new(path)) == Format::Org;
                                    let matched = if entity_type == 0 {
                                        crate::query::page_preamble_has_reference(
                                            raw,
                                            is_org,
                                            &needle,
                                            ReferenceKind::Plain,
                                            &config,
                                        )
                                    } else {
                                        let block = DocBlock::preamble(raw, is_org);
                                        let projection = block.projection();
                                        crate::reference_evidence::has_occurrence_kind(
                                            raw,
                                            &projection.reference_source,
                                            &needle,
                                            ReferenceKind::Plain,
                                            &config,
                                        )
                                    };
                                    Ok(matched.then(Vec::new))
                                })
                                .ok()?;
                            params.push(PhysicalQueryValue::Integer(
                                i64::try_from(window).unwrap_or(i64::MAX),
                            ));
                            limit_sql = format!(" LIMIT ?{}", params.len());
                        }
                    }
                    let sql = if let Some(_) = plan {
                        let candidate_source =
                            "(SELECT c.rowid AS entity_id, \
                                     CASE WHEN ep.page_id IS NOT NULL THEN 0 ELSE 1 END AS entity_type, \
                                     owner.path, b.result_id, \
                                     CASE WHEN ep.page_id IS NOT NULL \
                                          THEN COALESCE(pt.preamble, '') ELSE bt.content END AS raw, \
                                     owner_name.key AS owner_key \
                              FROM (SELECT rowid FROM search_fts \
                                    WHERE search_fts MATCH ?1 ORDER BY rowid DESC) c \
                              LEFT JOIN pages ep ON ep.page_id = c.rowid \
                              LEFT JOIN blocks b ON b.block_id = c.rowid \
                              JOIN pages owner ON owner.page_id = COALESCE(ep.page_id, b.page_id) \
                              JOIN names owner_name ON owner_name.name_id = owner.name_id \
                              LEFT JOIN page_text pt ON pt.page_id = ep.page_id \
                              LEFT JOIN block_text bt ON bt.block_id = b.block_id \
                              WHERE ep.page_id IS NOT NULL OR b.block_id IS NOT NULL) candidates";
                        let mut conditions = indexed_filter;
                        if matches!(
                            mode,
                            crate::query::candidate::CandidateMode::Interactive { .. }
                        ) {
                            let frame = crate::query::text::framed_pair_sql("raw", "path");
                            if !conditions.is_empty() {
                                conditions.push_str(" AND ");
                            }
                            conditions.push_str(&format!(
                                "tine_query_rank(entity_type, {frame}) IS NOT NULL"
                            ));
                        }
                        let where_sql = (!conditions.is_empty())
                            .then(|| format!(" WHERE {conditions}"))
                            .unwrap_or_default();
                        format!(
                            "SELECT path, result_id, entity_id, entity_type \
                             FROM {candidate_source}{where_sql} \
                             ORDER BY entity_id DESC{limit_sql}"
                        )
                    } else {
                        let page_frame = crate::query::text::framed_pair_sql(
                            "COALESCE(pt.preamble, '')",
                            "p.path",
                        );
                        let block_frame =
                            crate::query::text::framed_pair_sql("bt.content", "p.path");
                        let mut page_conditions = scan_filter.clone();
                        let mut block_conditions = scan_filter;
                        if matches!(
                            mode,
                            crate::query::candidate::CandidateMode::Interactive { .. }
                        ) {
                            if !page_conditions.is_empty() {
                                page_conditions.push_str(" AND ");
                                block_conditions.push_str(" AND ");
                            }
                            page_conditions
                                .push_str(&format!("tine_query_rank(0, {page_frame}) IS NOT NULL"));
                            block_conditions.push_str(&format!(
                                "tine_query_rank(1, {block_frame}) IS NOT NULL"
                            ));
                        }
                        let page_where = (!page_conditions.is_empty())
                            .then(|| format!(" WHERE {page_conditions}"))
                            .unwrap_or_default();
                        let block_where = (!block_conditions.is_empty())
                            .then(|| format!(" WHERE {block_conditions}"))
                            .unwrap_or_default();
                        format!(
                            "SELECT p.path AS path, NULL AS result_id, \
                                    p.page_id AS entity_id, 0 AS entity_type \
                             FROM pages p JOIN names n ON n.name_id = p.name_id \
                             LEFT JOIN page_text pt ON pt.page_id = p.page_id{page_where} \
                             UNION ALL \
                             SELECT p.path, b.result_id, b.block_id, 1 \
                             FROM blocks b JOIN block_text bt ON bt.block_id = b.block_id \
                             JOIN pages p ON p.page_id = b.page_id \
                             JOIN names n ON n.name_id = p.name_id{block_where} \
                             ORDER BY entity_id DESC{limit_sql}"
                        )
                    };
                    #[cfg(test)]
                    if matches!(
                        mode,
                        crate::query::candidate::CandidateMode::Interactive { .. }
                    ) {
                        capture_plain_reference_query_plan(&self.shared.path, &sql, &params)?;
                    }
                    crate::query::projection_sql::visit(snapshot, &sql, &params, |row| {
                        let Some(PhysicalQueryValue::Text(path)) = row.first() else {
                            return Err(tine_storage::sqlite::MaterializationError::InvalidQuery(
                                "plain-reference candidate has no page path".into(),
                            ));
                        };
                        let path = PathBuf::from(path);
                        paths.insert(path.clone());
                        match row.get(1) {
                            Some(PhysicalQueryValue::Text(result_id)) => {
                                blocks.insert(result_id.clone());
                            }
                            Some(PhysicalQueryValue::Null) => {}
                            _ => {
                                return Err(
                                    tine_storage::sqlite::MaterializationError::InvalidQuery(
                                        "plain-reference candidate has an invalid identity".into(),
                                    ),
                                );
                            }
                        }
                        match row.get(3) {
                            Some(PhysicalQueryValue::Integer(0)) => {
                                if let Some(page_owners) = page_owners.as_mut() {
                                    page_owners.insert(path);
                                }
                            }
                            Some(PhysicalQueryValue::Integer(1)) => {}
                            _ => {
                                return Err(
                                    tine_storage::sqlite::MaterializationError::InvalidQuery(
                                        "plain-reference candidate has an invalid entity kind"
                                            .into(),
                                    ),
                                );
                            }
                        }
                        Ok(std::ops::ControlFlow::Continue(()))
                    })
                    .ok()?;
                }
            }
        }
        self.ready_at(cache_generation)
            .then_some(ReferenceCandidateIndex {
                paths,
                blocks: Some(blocks),
                page_owners,
            })
    }

    /// Outer `None` means projection unavailable/stale and requires parser
    /// fallback. Inner `None` is an exact current-generation miss.
    pub(crate) fn block_page_hint(
        &self,
        cache_generation: u64,
        uuid: &str,
    ) -> Option<Option<String>> {
        let parsed_uuid = Uuid::parse_str(uuid).ok()?.into_bytes();
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let block = match read.block(uuid).ok()? {
            Some(block) => crate::query::logseq_uuid_owner([block], false),
            None => crate::query::logseq_uuid_owner(
                read.blocks_by_logseq_uuid(parsed_uuid, 2).ok()?,
                false,
            ),
        };
        let page = match block {
            Some(block) => read
                .page_with_header_validation(&block.page_path, |_, _| Ok(()))
                .ok()?
                .map(|page| page.name),
            None => None,
        };
        self.ready_at(cache_generation).then_some(page)
    }

    pub(crate) fn block_ref_counts(
        &self,
        cache_generation: u64,
    ) -> Option<std::collections::HashMap<String, usize>> {
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let mut counts = std::collections::HashMap::new();
        drain_after(
            |after, batch| read.block_reference_counts_after(after, batch),
            |row| row.raw_uuid_claim,
            |row| {
                let distinct = usize::try_from(row.distinct_source_blocks).map_err(|_| {
                    tine_storage::sqlite::MaterializationError::Corrupt(
                        "block reference count exceeds usize".into(),
                    )
                })?;
                counts.insert(Uuid::from_bytes(row.raw_uuid_claim).to_string(), distinct);
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        self.ready_at(cache_generation).then_some(counts)
    }

    pub(crate) fn block_referrer_candidate_paths(
        &self,
        cache_generation: u64,
        uuid: &str,
    ) -> Option<std::collections::BTreeSet<PathBuf>> {
        let uuid = Uuid::parse_str(uuid).ok()?.into_bytes();
        let reader = self.shared_reader_at(cache_generation)?;
        let read = reader.as_ref()?.read();
        let mut paths = std::collections::BTreeSet::new();
        drain_after(
            |after, batch| read.block_referrer_candidates_after(uuid, after, batch),
            |row| (row.page_cursor, row.block_cursor),
            |row| {
                paths.insert(PathBuf::from(row.source_page_path));
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        self.ready_at(cache_generation).then_some(paths)
    }

    pub(crate) fn ready_at(&self, generation: u64) -> bool {
        self.shared.ready_at(generation)
    }

    /// RET2's readiness lifecycle: why this generation is not ready, and what
    /// the caller may do about it.
    ///
    /// The order of the tests is the order of authority.
    ///
    /// * `worker_available` is stored `false` exactly where the worker thread
    ///   gives up for good — no parent directory, an unopenable database, a
    ///   writer lease another instance owns, or a `stop` turn. Nothing this
    ///   graph enqueues afterwards is ever taken, so retrying is endless by
    ///   construction and the caller owes a bounded error instead.
    /// * A queued turn is progress even when the LAST turn failed:
    ///   `worker_failed` stays set until the next successful turn, and the
    ///   repair that clears it is exactly the `full`/`rebuild` work below.
    /// * A failed worker with an EMPTY queue is the stale-idle case: the turn
    ///   failed, `requires_full_rebuild` latched inside the worker, and until
    ///   a complete source inventory arrives every further delta turn refuses.
    ///   That is a repair, not a wait.
    pub(crate) fn progress_at(&self, generation: u64) -> ProjectionProgress {
        use crate::query::QueryReadinessReason as Reason;
        if self.ready_at(generation) {
            return ProjectionProgress::Ready;
        }
        let pending = self.shared.pending.lock().unwrap();
        if pending.stop || !self.shared.worker_available.load(Ordering::Acquire) {
            return ProjectionProgress::Stopped;
        }
        if pending.rebuild || pending.full.is_some() {
            return ProjectionProgress::Working(Reason::Recovering);
        }
        if self.shared.repairs_in_flight.load(Ordering::Acquire) > 0 {
            return ProjectionProgress::Working(Reason::Recovering);
        }
        if pending.warm.is_some() || self.shared.warms_in_flight.load(Ordering::Acquire) > 0 {
            return ProjectionProgress::Working(Reason::Indexing);
        }
        if !pending.deltas.is_empty() {
            return ProjectionProgress::Working(Reason::PendingEdits);
        }
        if self.shared.worker_failed.load(Ordering::Acquire) {
            // The queue is empty and the last turn failed: nothing is coming.
            return ProjectionProgress::Stale;
        }
        if self.shared.worker_busy.load(Ordering::Acquire) {
            return ProjectionProgress::Working(Reason::Busy);
        }
        ProjectionProgress::Stale
    }

    /// Wait until the worker has drained its queue and finished its turn, and
    /// report whether that turn failed.
    #[cfg(test)]
    pub(crate) fn wait_drained_test(&self) -> bool {
        let started = std::time::Instant::now();
        loop {
            {
                let pending = self.shared.pending.lock().unwrap();
                if !pending.has_work() && !self.shared.worker_busy.load(Ordering::Acquire) {
                    return !self.shared.worker_failed.load(Ordering::Acquire);
                }
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(15),
                "projection worker did not drain"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Test diagnostic: the queue and readiness state in one line, for a
    /// convergence failure that would otherwise be a bare timeout.
    #[cfg(test)]
    pub(crate) fn debug_state_test(&self) -> String {
        let pending = self.shared.pending.lock().unwrap();
        format!(
            "ready={} validated={} ready_generation={} latest_generation={} full={} deltas={} warm={} warm_outcome={:?} rebuild={} stop={} page_order={} worker_available={} worker_failed={} worker_busy={}",
            self.shared.ready.load(Ordering::Acquire),
            self.shared.validated.load(Ordering::Acquire),
            self.shared.ready_generation.load(Ordering::Acquire),
            pending.latest_generation,
            pending.full.is_some(),
            pending.deltas.len(),
            pending.warm.is_some(),
            pending.warm_outcome.as_ref().map(|(_, outcome)| match outcome {
                WarmOutcome::Clean => "Clean".to_owned(),
                WarmOutcome::FreshBuildRequired => "FreshBuildRequired".to_owned(),
                WarmOutcome::Changed {
                    replacements,
                    deletions,
                } => format!(
                    "Changed(replacements={}, deletions={})",
                    replacements.len(),
                    deletions.len()
                ),
                WarmOutcome::Superseded => "Superseded".to_owned(),
                WarmOutcome::Failed => "Failed".to_owned(),
            }),
            pending.rebuild,
            pending.stop,
            pending.page_order.len(),
            self.shared.worker_available.load(Ordering::Acquire),
            self.shared.worker_failed.load(Ordering::Acquire),
            self.shared.worker_busy.load(Ordering::Acquire),
        )
    }

    #[cfg(test)]
    pub(crate) fn indexed_reads(&self) -> u64 {
        self.shared.indexed_reads.load(Ordering::Relaxed)
    }

    /// Close this projection's query-job admission, the way `Drop` does when a
    /// graph is closing. Every later `open_query_job` is `Cancelled`, which is
    /// the ONE §5.9 state a public query must never repair or retry.
    #[cfg(test)]
    pub(crate) fn close_query_jobs_test(&self) {
        let fence = self.shared.query_jobs.begin_close();
        self.shared.query_jobs.wait_for_drain(fence);
    }

    #[cfg(test)]
    pub(crate) fn inject_next_statement_failure(&self) {
        self.shared
            .inject_read_failure
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn statement_reads(&self) -> u64 {
        self.shared.statement_reads.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn take_registry_capture_attempts(&self) -> u64 {
        self.shared
            .registry_capture_attempts
            .swap(0, Ordering::AcqRel)
    }

    #[cfg(test)]
    pub(crate) fn fallback_reads(&self) -> u64 {
        self.shared.fallback_reads.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn referenced_name_reads(&self) -> u64 {
        self.shared.referenced_name_reads.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn serving_writer_cache_budget_test(&self) -> u64 {
        self.shared
            .serving_writer_cache_budget
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn reset_projection_health_checks_test(&self) {
        self.shared
            .projection_health_checks
            .store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn projection_health_checks_test(&self) -> u64 {
        self.shared.projection_health_checks.load(Ordering::Relaxed)
    }

    /// R3: refuse new jobs, interrupt the active ones and wait for their slots
    /// before the worker is told to stop, so no snapshot outlives the
    /// projection that admitted it. Idempotent — `stop` and a closed admission
    /// owner are both terminal, so a caller that closes explicitly and then
    /// drops pays a second no-op drain and nothing else.
    fn close(&self) {
        let fence = self.shared.cancel_queued_captures(true);
        self.shared.query_jobs.wait_for_drain(fence);
    }

    /// Retain a resource until the writer has released its connection and lease.
    /// The publication root also has a foreground owner until its graph drops.
    #[cfg(test)]
    pub(crate) fn retain_worker_resource(&self, resource: Arc<dyn Send + Sync>) {
        if let Some(resources) = self.shared.worker_resources.lock().unwrap().as_mut() {
            resources.push(resource);
        }
    }

    /// [`DirectProjection::close`], then wait until the writer worker has
    /// actually RETURNED — up to `timeout`. `true` when it did.
    ///
    /// The only caller that needs this is one that owns the database's
    /// directory and is about to remove it: on Windows an open handle refuses
    /// the delete, and on every platform a worker still finishing its turn can
    /// recreate the file under a directory that was just removed. Ordinary
    /// graph close does NOT wait — an app teardown must not block on SQLite —
    /// which is why the wait is an explicit call and not part of `Drop`.
    ///
    /// A `stop` turn is taken as soon as the worker reaches the top of its
    /// loop, so the bound is one in-flight apply, never a queue.
    pub(crate) fn close_and_wait_for_worker(&self, timeout: std::time::Duration) -> bool {
        self.close();
        let started = std::time::Instant::now();
        while !self.shared.worker_finished.load(Ordering::Acquire) {
            if started.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        true
    }
}

impl Drop for DirectProjection {
    fn drop(&mut self) {
        self.close();
    }
}

/// Publish the worker's exit AFTER every resource it owns has been released.
///
/// Declared as the FIRST local in [`projection_worker`], so it drops LAST —
/// after the writer connection and the exclusive writer lease. `Drop` is the
/// only correct place for it: the worker has five early returns and one
/// steady-state one, and a flag stored at each of them is a flag the next arm
/// forgets.
struct ProjectionWorkerExit(Arc<ProjectionShared>);

impl Drop for ProjectionWorkerExit {
    fn drop(&mut self) {
        self.0.worker_available.store(false, Ordering::Release);
        let captures = std::mem::take(&mut self.0.pending.lock().unwrap().captures);
        reject_query_captures(captures);
        let resources = self.0.worker_resources.lock().unwrap().take();
        drop(resources);
        self.0.worker_finished.store(true, Ordering::Release);
        self.0.changed.notify_all();
    }
}

const PROJECTION_UPDATE_FAILURE: &str = "is stale; indexed reads are unavailable";

/// Report a Direct Files projection write failure. Each read surface owns its
/// readiness/error policy; this writer cannot claim that a query will traverse.
///
/// The always-on line names the failure family in fixed words and carries
/// nothing else. I-5: the detail at both call sites is free-form prose from the
/// projection WRITE path, and that path names the graph — `apply_deltas`
/// formats `entry.rel_path` straight into its error string, and
/// `MaterializationError`'s payloads are free-form `String`s produced while
/// storing parsed page text. I-9: the family still reaches the always-on
/// record, because a user who is not running under `TINE_DEBUG` otherwise sees
/// only an unavailable index. The prose stays on the directed debug channel.
/// Process-relative clock for [`projection_diag`], started at the first line.
static PROJECTION_DIAG_EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Lines emitted this process, so the gating is checkable rather than asserted
/// in a comment (I-11); a test cannot read stderr.
#[cfg(test)]
static PROJECTION_DIAG_LINES: AtomicU64 = AtomicU64::new(0);

/// One directed projection-lifecycle line, on the SAME opt-in channel as every
/// other runtime diagnostic (`TINE_DEBUG=1` / `--debug`, I-12).
///
/// GH #543: the Windows verify probe could say only that a 10,000-page cold
/// open took 139 s with the switcher stuck on "Indexing 0 of 10,001"; the
/// app's debug log stopped at "Direct Files publish" and the next 105 s were
/// unobserved, so no run could say which worker turn was running or why. The
/// message is built behind a `FnOnce` so a process without diagnostics pays
/// one relaxed atomic load and formats nothing.
pub(crate) fn projection_diag(message: impl FnOnce() -> String) {
    if !crate::backend_error::runtime_debug_diagnostics_enabled() {
        return;
    }
    #[cfg(test)]
    PROJECTION_DIAG_LINES.fetch_add(1, Ordering::Relaxed);
    let elapsed = PROJECTION_DIAG_EPOCH
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    eprintln!("[tine] projection +{elapsed}ms {}", message());
}

#[cfg(test)]
pub(crate) fn projection_diag_lines_test() -> u64 {
    PROJECTION_DIAG_LINES.load(Ordering::Relaxed)
}

fn report_projection_failure(family: &str, detail: &dyn std::fmt::Display) {
    #[cfg(test)]
    REPORTED_PROJECTION_FAILURES.fetch_add(1, Ordering::Relaxed);
    eprintln!("[tine] Direct Files SQLite projection {family}");
    if crate::backend_error::runtime_debug_diagnostics_enabled() {
        eprintln!("[tine] Direct Files SQLite projection {family}; directed detail: {detail}");
    }
}

/// How many times the always-on failure family has been printed this process.
///
/// The counter exists because the ONLY difference between a reported failure and
/// a silent handoff is which `eprintln!` runs, and a test cannot read stderr.
/// It is what makes [`ProjectionRefusal`]'s split checkable rather than merely
/// asserted in a comment.
#[cfg(test)]
static REPORTED_PROJECTION_FAILURES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(crate) fn reported_projection_failures_test() -> u64 {
    REPORTED_PROJECTION_FAILURES.load(Ordering::Relaxed)
}

/// Why one worker turn produced no serving image.
///
/// The two arms leave the SAME state behind — `requires_full_rebuild` latched,
/// `worker_failed` set, readiness withdrawn — because in both cases only a
/// complete source inventory may publish readiness again. They differ in ONE
/// thing: whether a user is told the index broke.
///
/// `AwaitingFullInventory` is not a failure and must never reach the always-on
/// channel. It is the ordinary cold-open handoff when a delta arrives before
/// the captured parsed snapshot that will build the first complete image.
/// Nothing is wrong and nothing is owed by the user.
enum ProjectionRefusal {
    AwaitingFullInventory,
    Failed(String),
    Stopped,
}

impl ProjectionRefusal {
    /// Whether this refusal is a genuine write failure the user must be told
    /// about on the always-on channel.
    fn is_reportable_failure(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

impl std::fmt::Display for ProjectionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AwaitingFullInventory => {
                f.write_str("a complete source inventory is owed before deltas can lower again")
            }
            Self::Failed(error) => f.write_str(error),
            Self::Stopped => f.write_str("projection stopped before staged publication"),
        }
    }
}

/// Lives for one repair attempt; see `DirectProjection::begin_repair`.
pub(crate) struct RepairInFlight(Arc<ProjectionShared>);

/// Lives from a warm validation's first page read until it has enqueued or
/// given up; see `DirectProjection::begin_warm`.
pub(crate) struct WarmInFlight(Arc<ProjectionShared>);

impl Drop for WarmInFlight {
    fn drop(&mut self) {
        self.0.warms_in_flight.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_all();
    }
}

impl Drop for RepairInFlight {
    fn drop(&mut self) {
        self.0.repairs_in_flight.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_all();
    }
}

fn projection_worker(shared: Arc<ProjectionShared>) {
    // FIRST local, so it is the LAST thing dropped: the writer connection and
    // the exclusive lease below are both released before the exit is published.
    let _exit = ProjectionWorkerExit(Arc::clone(&shared));
    let Some(parent) = shared.path.parent() else {
        shared.worker_available.store(false, Ordering::Release);
        shared.changed.notify_all();
        return;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        eprintln!("[tine] Direct Files SQLite projection disabled: create directory: {error}");
        shared.worker_available.store(false, Ordering::Release);
        shared.changed.notify_all();
        return;
    }
    let lease_path = shared.path.with_extension("sqlite.writer.lock");
    let lease = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lease_path)
        .and_then(|file| {
            file.try_lock_exclusive()?;
            Ok(file)
        }) {
        Ok(lease) => lease,
        Err(error) => {
            eprintln!(
                "[tine] Direct Files SQLite projection unavailable; another graph instance owns it or its lease cannot be opened: {error}"
            );
            shared.worker_available.store(false, Ordering::Release);
            shared.changed.notify_all();
            return;
        }
    };
    // The lock file is app-private disposable state. Retain its exclusive lock
    // for the complete writer lifetime so another Graph instance cannot replace
    // this database's facts behind a locally-ready generation watermark.
    let _lease = lease;
    let publication_directory =
        projection_publication_names(&shared.path).and_then(|(parent, _, destination)| {
            let directory = Dir::open_ambient_dir(&parent, ambient_authority())
                .map_err(|error| error.to_string())?;
            cleanup_projection_stages(&directory, &destination)
        });
    if let Err(error) = publication_directory {
        report_projection_failure(PROJECTION_UPDATE_FAILURE, &error);
        shared.worker_available.store(false, Ordering::Release);
        shared.changed.notify_all();
        return;
    }
    let mut writer_slot = open_existing_projection_database(&shared);
    let mut requires_full_rebuild = writer_slot.is_none();
    loop {
        let turn = {
            let mut pending = shared.pending.lock().unwrap();
            while !pending.has_work() && pending.captures.is_empty() && !pending.stop {
                pending = shared.changed.wait(pending).unwrap();
            }
            if pending.stop {
                shared.worker_available.store(false, Ordering::Release);
                shared.changed.notify_all();
                return;
            }
            let captures = std::mem::take(&mut pending.captures);
            drop(pending);
            for capture in captures {
                let job = capture_query_job(
                    &shared,
                    capture.requirement,
                    capture.registry_sensitivity,
                    capture.slot,
                );
                let _ = capture.reply.send(job);
            }
            let mut pending = shared.pending.lock().unwrap();
            if pending.stop {
                return;
            }
            if !pending.has_work() {
                continue;
            }
            shared.worker_busy.store(true, Ordering::Release);
            shared.worker_building.store(
                pending.full.is_some() || pending.warm.is_some(),
                Ordering::Release,
            );
            let rebuild = (pending.full.is_some() || pending.warm.is_some())
                && std::mem::take(&mut pending.rebuild);
            // R6: a full snapshot queued beside a warm validation owns
            // readiness; the warm is dropped as superseded.
            let warm = if pending.full.is_some() {
                if pending.warm.take().is_some() {
                    pending.warm_outcome = Some((pending.warm_attempt, WarmOutcome::Superseded));
                }
                None
            } else {
                pending.warm.take()
            };
            let deltas = std::mem::take(&mut pending.deltas);
            let full = pending.full.take();
            let inventory = full.as_ref().map(|_| pending.ordered_inventory());
            WorkerTurn {
                full,
                warm,
                deltas,
                inventory,
                latest_generation: pending.latest_generation,
                rebuild,
            }
        };
        let WorkerTurn {
            full,
            mut warm,
            mut deltas,
            inventory,
            latest_generation,
            rebuild,
        } = turn;
        let had_full = full.is_some();
        let had_warm = warm.is_some();
        let turn_started = std::time::Instant::now();
        projection_diag(|| {
            format!(
                "turn begin full={had_full} warm={} deltas={} rebuild={rebuild} generation={latest_generation} needs_rebuild={requires_full_rebuild}",
                warm.as_ref().map_or(0, |warm| warm.sources.len()),
                deltas.len(),
            )
        });
        let registry_config = full
            .as_ref()
            .map(|full| Arc::clone(&full.parse_config))
            .or_else(|| warm.as_ref().map(|warm| Arc::clone(&warm.parse_config)))
            .or_else(|| {
                deltas
                    .values()
                    .filter_map(|(generation, delta)| match delta {
                        PageDelta::Replace { parse_config, .. } => Some((generation, parse_config)),
                        PageDelta::Delete { .. } => None,
                    })
                    .max_by_key(|(generation, _)| *generation)
                    .map(|(_, config)| Arc::clone(config))
            });
        // Opening the serving writer already validates an existing image. A
        // full parsed snapshot is the later boundary at which integrity is
        // re-established before deciding whether that complete image can be
        // reused. Ordinary one-page deltas must never turn into a whole-file
        // quick_check.
        let existing_image_healthy = if had_full {
            writer_slot
                .as_ref()
                .is_some_and(|database| projection_image_is_healthy(&shared, database))
        } else {
            writer_slot.is_some()
        };
        let reuse_full = full.as_ref().is_some_and(|full| {
            !rebuild
                && !requires_full_rebuild
                && existing_image_healthy
                && full_sources_match(writer_slot.as_ref().expect("healthy writer"), full)
        });
        let fresh_build = had_full && !reuse_full;
        let registry_reset = fresh_build || had_warm;
        let config_changed = registry_config.as_ref().is_some_and(|config| {
            shared
                .committed_registry
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|owner| owner.config.digest() != config.digest())
        });
        let touched_pages = deltas
            .values()
            .map(|(_, delta)| delta.entry().rel_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        #[cfg(test)]
        run_before_apply_deltas_hook();
        let applied: Result<AppliedTurn, ProjectionRefusal> = (|| {
            if config_changed && !had_full {
                let fence = shared.cancel_queued_captures(false);
                shared.query_jobs.wait_for_drain(fence);
            }
            let registry_before = if !registry_reset
                && !touched_pages.is_empty()
                && shared.committed_registry.lock().unwrap().is_some()
            {
                let mut snapshot =
                    PhysicalProjectionQuerySnapshot::open_direct(&shared.path, || Ok(()))
                        .map_err(|error| ProjectionRefusal::Failed(error.to_string()))?;
                registry_sql::read_page_registry_metadata(&mut snapshot, &touched_pages)
                    .map_err(|error| ProjectionRefusal::Failed(error.to_string()))?
            } else {
                PageRegistryMetadata::new()
            };

            let mut applied = if let Some(full) = full {
                let inventory = inventory.ok_or_else(|| {
                    ProjectionRefusal::Failed(
                        "fresh build has no captured page inventory".to_owned(),
                    )
                })?;
                if reuse_full {
                    apply_deltas(
                        writer_slot
                            .as_mut()
                            .ok_or(ProjectionRefusal::AwaitingFullInventory)?,
                        deltas,
                    )
                    .map_err(ProjectionRefusal::Failed)?
                } else if !full.source_complete && existing_image_healthy {
                    // A transiently unreadable page is absent from this parsed
                    // cache. Replacing a healthy complete image would turn
                    // that absence into a deletion. Keep the old coherent
                    // image and wait for a complete readable capture instead.
                    return Err(ProjectionRefusal::AwaitingFullInventory);
                } else {
                    let (database, applied) = build_and_publish_fresh_projection(
                        &shared,
                        writer_slot.take(),
                        full,
                        deltas,
                        &inventory,
                    )
                    .map_err(|error| match error {
                        FreshBuildError::Stopped => ProjectionRefusal::Stopped,
                        FreshBuildError::Failed(error) => ProjectionRefusal::Failed(error),
                    })?;
                    writer_slot = Some(database);
                    applied
                }
            } else if let Some(warm) = warm.as_mut() {
                let outcome = if rebuild || requires_full_rebuild {
                    WarmOutcome::FreshBuildRequired
                } else if let Some(database) = writer_slot.as_mut() {
                    if let Some(repair) = warm.repair.take() {
                        let repaired = repair
                            .replacements
                            .iter()
                            .map(|(entry, _, revision)| (entry.rel_path.clone(), revision.clone()))
                            .collect::<HashMap<_, _>>();
                        let config_digest = warm.parse_config.digest();
                        // An update for exactly the bytes the repair just
                        // wrote would lower the page a second time.
                        deltas.retain(|path, (_, delta)| {
                            !matches!(
                                delta,
                                PageDelta::Replace { revision, parse_config, .. }
                                    if repaired.get(path) == Some(revision)
                                        && parse_config.digest() == config_digest
                            )
                        });
                        let order =
                            apply_warm_repair(database, &shared, repair, &warm.parse_config)
                                .map_err(ProjectionRefusal::Failed)?;
                        // The image now keeps the order's positions; so does
                        // the queue, and so do the updates taken with it.
                        let mut pending = shared.pending.lock().unwrap();
                        pending.reseed_after_repair(&order);
                        pending.place_taken(&mut deltas);
                    }
                    validate_warm(database, warm).map_err(ProjectionRefusal::Failed)?
                } else {
                    WarmOutcome::FreshBuildRequired
                };
                if !matches!(outcome, WarmOutcome::Clean) {
                    // The warm owns no validated image, so the updates taken
                    // beside it are not applied. Put them back: the full
                    // snapshot or repair that follows takes them, and until
                    // then a later warm or turn must not publish readiness
                    // without them.
                    let mut pending = shared.pending.lock().unwrap();
                    for (path, (generation, delta)) in deltas {
                        match pending.deltas.entry(path) {
                            std::collections::btree_map::Entry::Vacant(slot) => {
                                slot.insert((generation, delta));
                            }
                            // A newer update for the page arrived meanwhile.
                            std::collections::btree_map::Entry::Occupied(_) => {}
                        }
                    }
                    if matches!(outcome, WarmOutcome::Changed { .. }) {
                        // The walk's order includes pages the image does not
                        // hold yet, so its positions collide with the stored
                        // ones until the repair reconciles them. Updates that
                        // run before the repair go by the image's own
                        // positions instead (`settle_unseeded_deltas`).
                        pending.unseed_page_order();
                    }
                    drop(pending);
                    return Ok(AppliedTurn {
                        warm_outcome: Some(outcome),
                        ..AppliedTurn::default()
                    });
                }
                let mut applied = if deltas.is_empty() {
                    AppliedTurn::default()
                } else {
                    apply_deltas(
                        writer_slot
                            .as_mut()
                            .ok_or(ProjectionRefusal::AwaitingFullInventory)?,
                        deltas,
                    )
                    .map_err(ProjectionRefusal::Failed)?
                };
                applied.warm_outcome = Some(outcome);
                applied
            } else {
                if requires_full_rebuild {
                    return Err(ProjectionRefusal::AwaitingFullInventory);
                }
                let database = writer_slot
                    .as_mut()
                    .ok_or(ProjectionRefusal::AwaitingFullInventory)?;
                let (deltas, unplaced) =
                    settle_unseeded_deltas(database, deltas).map_err(ProjectionRefusal::Failed)?;
                if !unplaced.is_empty() {
                    shared.pending.lock().unwrap().park_unplaced(unplaced);
                }
                apply_deltas(database, deltas).map_err(ProjectionRefusal::Failed)?
            };

            let (revision, registry_after) = {
                let mut snapshot =
                    PhysicalProjectionQuerySnapshot::open_direct(&shared.path, || Ok(()))
                        .map_err(|error| ProjectionRefusal::Failed(error.to_string()))?;
                let revision = snapshot
                    .query_revision()
                    .map_err(|error| ProjectionRefusal::Failed(error.to_string()))?;
                let registry_after = if !registry_reset
                    && !touched_pages.is_empty()
                    && shared.committed_registry.lock().unwrap().is_some()
                {
                    registry_sql::read_page_registry_metadata(&mut snapshot, &touched_pages)
                        .map_err(|error| ProjectionRefusal::Failed(error.to_string()))?
                } else {
                    PageRegistryMetadata::new()
                };
                (revision, registry_after)
            };
            applied.registry_pages = registry_after;
            #[cfg(test)]
            if let Some(hook) = shared.after_sql_commit.lock().unwrap().take() {
                hook();
            }
            shared.record_session_pages(&applied.pages);
            let changes = registry_sql::registry_changes(&registry_before, &applied.registry_pages);
            let mut registry = shared.committed_registry.lock().unwrap();
            let config = registry_config
                .as_ref()
                .cloned()
                .or_else(|| registry.as_ref().map(|owner| Arc::clone(&owner.config)));
            if let Some(config) = config {
                match registry.as_mut() {
                    Some(owner) if !registry_reset && owner.config.digest() == config.digest() => {
                        owner
                            .cache
                            .committed(
                                revision,
                                changes.normalized_keys,
                                changes.declaration_page_names,
                            )
                            .map_err(|error| ProjectionRefusal::Failed(error.to_string()))?;
                    }
                    Some(owner) => {
                        owner.cache.reset(revision, &config);
                        owner.config = config;
                    }
                    None => {
                        *registry = Some(CommittedRegistryOwner {
                            cache: CommittedRegistryCache::new(revision, &config),
                            config,
                        })
                    }
                }
            }
            Ok(applied)
        })();
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                shared.ready.store(false, Ordering::Release);
                match error {
                    // Nothing was written and nothing is broken: the
                    // committed image and its registry stand, readiness is
                    // withdrawn until an inventory arrives, and that inventory
                    // is applied without a reset.
                    ProjectionRefusal::AwaitingFullInventory => {
                        requires_full_rebuild = true;
                    }
                    ProjectionRefusal::Failed(_) => {
                        shared.committed_registry.lock().unwrap().take();
                        requires_full_rebuild = true;
                        shared.worker_failed.store(true, Ordering::Release);
                    }
                    ProjectionRefusal::Stopped => {
                        shared.worker_building.store(false, Ordering::Release);
                        shared.worker_busy.store(false, Ordering::Release);
                        shared.worker_available.store(false, Ordering::Release);
                        shared.changed.notify_all();
                        return;
                    }
                }
                {
                    let mut pending = shared.pending.lock().unwrap();
                    if had_warm {
                        pending.warm_outcome = Some((pending.warm_attempt, WarmOutcome::Failed));
                    }
                }
                shared.worker_building.store(false, Ordering::Release);
                shared.worker_busy.store(false, Ordering::Release);
                shared.changed.notify_all();
                if error.is_reportable_failure() {
                    report_projection_failure(PROJECTION_UPDATE_FAILURE, &error);
                } else {
                    projection_diag(|| {
                        format!(
                            "turn deferred after {}ms: {error}",
                            turn_started.elapsed().as_millis()
                        )
                    });
                }
                continue;
            }
        };
        if had_full {
            requires_full_rebuild = false;
        } else if had_warm {
            requires_full_rebuild =
                matches!(applied.warm_outcome, Some(WarmOutcome::FreshBuildRequired));
        }
        if had_full || matches!(applied.warm_outcome, Some(WarmOutcome::Clean)) {
            shared.validated.store(true, Ordering::Release);
        }
        shared.worker_failed.store(false, Ordering::Release);
        projection_diag(|| {
            format!(
                "turn applied in {}ms lowered={} deleted={} fresh_build={fresh_build}",
                turn_started.elapsed().as_millis(),
                applied.pages.lowered.len(),
                applied.pages.deleted.len(),
            )
        });
        let mut pending = shared.pending.lock().unwrap();
        shared.worker_building.store(false, Ordering::Release);
        shared.worker_busy.store(false, Ordering::Release);
        if had_warm {
            if pending.warm_outcome.is_none() {
                pending.warm_outcome = applied
                    .warm_outcome
                    .clone()
                    .map(|outcome| (pending.warm_attempt, outcome));
            }
        }
        if !pending.rebuild
            && !pending.has_work()
            && pending.latest_generation == latest_generation
            && shared.validated.load(Ordering::Acquire)
        {
            shared
                .ready_generation
                .store(latest_generation, Ordering::Release);
            shared.ready.store(true, Ordering::Release);
            projection_diag(|| format!("ready at generation={latest_generation}"));
        }
        drop(pending);
        shared.changed.notify_all();
        // Source-change events may have preceded this commit. Wake the existing
        // application watcher after every serving-image publication, without
        // retaining a query or requiring any edit to be covered by its read.
        if shared.validated.load(Ordering::Acquire) {
            shared.commit_notification.fetch_add(1, Ordering::Release);
            if let Some(wake) = shared.commit_waker.lock().unwrap().as_ref() {
                let _ = wake.send(());
            }
        }
    }
}

/// One worker turn's queued work.
struct WorkerTurn {
    full: Option<PendingFull>,
    warm: Option<PendingWarm>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    /// The queue's inventory captured with a full snapshot and its coalesced
    /// deltas, so the unpublished image receives final page positions.
    inventory: Option<Vec<String>>,
    latest_generation: u64,
    rebuild: bool,
}

fn projection_image_is_healthy(
    _shared: &ProjectionShared,
    database: &PhysicalGraphProjectionDatabase,
) -> bool {
    #[cfg(test)]
    _shared
        .projection_health_checks
        .fetch_add(1, Ordering::Relaxed);
    database.validate_schema().is_ok() && database.quick_check().is_ok()
}

fn full_sources_match(database: &PhysicalGraphProjectionDatabase, full: &PendingFull) -> bool {
    if !full.source_complete {
        return false;
    }
    let config_digest = full.parse_config.digest();
    let mut sources = Vec::with_capacity(full.pages.len());
    for (entry, _) in full.pages.iter() {
        let Some(revision) = full.revisions.get(&entry.path) else {
            return false;
        };
        sources.push(PhysicalGraphProjectionSourceRevision {
            path: entry.rel_path.clone(),
            revision: projection_source_revision(revision, config_digest),
        });
    }
    database
        .source_delta(&sources)
        .is_ok_and(|delta| delta.replacements.is_empty() && delta.deletions.is_empty())
}

fn open_existing_projection_database(
    shared: &ProjectionShared,
) -> Option<PhysicalGraphProjectionDatabase> {
    if !shared.path.exists() {
        return None;
    }
    let database = PhysicalGraphProjectionDatabase::open_writable(&shared.path).ok()?;
    projection_image_is_healthy(shared, &database).then_some(database)
}

const PROJECTION_STAGE_MARKER: &str = ".tine-projection-build-";

fn projection_publication_names(path: &Path) -> Result<(PathBuf, String, String), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "projection path has no parent directory".to_owned())?
        .to_path_buf();
    let destination = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "projection filename is not UTF-8".to_owned())?
        .to_owned();
    let source = format!(
        ".{destination}{PROJECTION_STAGE_MARKER}{}",
        Uuid::new_v4().simple()
    );
    Ok((parent, source, destination))
}

fn cleanup_projection_stages(directory: &Dir, destination: &str) -> Result<(), String> {
    let prefix = format!(".{destination}{PROJECTION_STAGE_MARKER}");
    for entry in directory.entries().map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let base = name
            .strip_suffix("-wal")
            .or_else(|| name.strip_suffix("-shm"))
            .or_else(|| name.strip_suffix("-journal"))
            .unwrap_or(name);
        let Some(id) = base.strip_prefix(&prefix) else {
            continue;
        };
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let metadata = directory
            .symlink_metadata(name)
            .map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "projection staging entry is not a regular file: {name}"
            ));
        }
        directory
            .remove_file(name)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn cleanup_projection_stage_artifacts(directory: &Dir, stage_name: &str) -> Result<(), String> {
    for name in [
        stage_name.to_owned(),
        format!("{stage_name}-wal"),
        format!("{stage_name}-shm"),
        format!("{stage_name}-journal"),
    ] {
        let metadata = match directory.symlink_metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "projection staging artifact is not a regular file: {name}"
            ));
        }
        directory
            .remove_file(&name)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn remove_projection_sidecar(directory: &Dir, name: &str) -> Result<(), String> {
    let metadata = match directory.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("projection sidecar is not a regular file: {name}"));
    }
    directory
        .remove_file(name)
        .map_err(|error| error.to_string())
}

enum FreshBuildError {
    Stopped,
    Failed(String),
}

fn fresh_build_stopped(shared: &ProjectionShared) -> bool {
    shared.pending.lock().unwrap().stop
}

fn build_and_publish_fresh_projection(
    shared: &ProjectionShared,
    old_writer: Option<PhysicalGraphProjectionDatabase>,
    full: PendingFull,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    inventory: &[String],
) -> Result<(PhysicalGraphProjectionDatabase, AppliedTurn), FreshBuildError> {
    const BUILD_BATCH: usize = 32;

    let (parent, stage_name, destination_name) =
        projection_publication_names(&shared.path).map_err(FreshBuildError::Failed)?;
    let directory = Dir::open_ambient_dir(&parent, ambient_authority())
        .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
    cleanup_projection_stages(&directory, &destination_name).map_err(FreshBuildError::Failed)?;
    let stage_path = parent.join(&stage_name);

    let build = (|| {
        if fresh_build_stopped(shared) {
            return Err(FreshBuildError::Stopped);
        }
        let stage_publication = tine_storage::DurableDirectoryPublication::open(&directory)
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        let mut database =
            PhysicalGraphProjectionDatabase::create_fresh_build(&stage_path, stage_publication)
                .map_err(|error| FreshBuildError::Failed(error.to_string()))?;

        let PendingFull {
            pages,
            revisions,
            parse_config,
            source_complete: _,
        } = full;
        let config_digest = parse_config.digest();
        let mut applied = AppliedTurn::default();
        let mut text_bytes = 0u64;
        let progress = shared.build_progress.begin(
            crate::indexing_progress::IndexingPhase::Indexing,
            pages.len(),
        );
        for chunk in pages.chunks(BUILD_BATCH) {
            progress.advance(chunk.len());
            if fresh_build_stopped(shared) {
                return Err(FreshBuildError::Stopped);
            }
            let mut replacements = Vec::with_capacity(chunk.len());
            let mut postings = Vec::new();
            let mut aliases = Vec::new();
            let mut sources = Vec::with_capacity(chunk.len());
            for (entry, document) in chunk {
                let (mut page, mut page_postings, mut page_aliases) =
                    physical_page(entry, document, &parse_config)
                        .map_err(FreshBuildError::Failed)?;
                page.position = None;
                text_bytes =
                    text_bytes.saturating_add(projected_text_bytes(std::slice::from_ref(&page)));
                sources.push(PhysicalGraphProjectionSourceRevision {
                    path: entry.rel_path.clone(),
                    revision: projection_source_revision(
                        revisions.get(&entry.path).ok_or_else(|| {
                            FreshBuildError::Failed(format!(
                                "parsed page has no exact source revision: {}",
                                entry.rel_path
                            ))
                        })?,
                        config_digest,
                    ),
                });
                applied.pages.lowered.push(entry.rel_path.clone());
                replacements.push(page);
                postings.append(&mut page_postings);
                aliases.append(&mut page_aliases);
            }
            database
                .set_page_cache_budget(crate::projection_budget::build_page_cache_budget(
                    text_bytes,
                    crate::projection_budget::physical_memory_bytes(),
                ))
                .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
            database
                .append_with_source_revisions_and_aliases(
                    &PhysicalGraphProjectionChange {
                        replacements,
                        deletions: Vec::new(),
                        reference_postings: postings,
                    },
                    &sources,
                    &aliases,
                )
                .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
            #[cfg(test)]
            if let Some(hook) = shared.after_fresh_build_batch.lock().unwrap().take() {
                hook();
            }
            if fresh_build_stopped(shared) {
                return Err(FreshBuildError::Stopped);
            }
        }

        let delta = lower_deltas(deltas).map_err(FreshBuildError::Failed)?;
        applied.pages.lowered.extend(delta.applied.pages.lowered);
        applied.pages.deleted.extend(delta.applied.pages.deleted);
        if fresh_build_stopped(shared) {
            return Err(FreshBuildError::Stopped);
        }
        let finalized = database
            .finish(&delta.change, &delta.revisions, &delta.aliases, inventory)
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        Ok((finalized, applied, text_bytes))
    })();

    let (finalized, applied, text_bytes) = match build {
        Ok(built) => built,
        Err(error) => {
            let _ = cleanup_projection_stage_artifacts(&directory, &stage_name);
            return Err(error);
        }
    };

    let publication = (|| {
        if fresh_build_stopped(shared) {
            return Err(FreshBuildError::Stopped);
        }
        let fence = shared.cancel_queued_captures(false);
        shared.query_jobs.wait_for_drain(fence);
        #[cfg(test)]
        if let Some(hook) = shared
            .before_shared_reader_drain_lock
            .lock()
            .unwrap()
            .take()
        {
            hook();
        }
        let mut reader = shared.reader.lock().unwrap();
        #[cfg(test)]
        if let Some(hook) = shared.after_shared_reader_drain_lock.lock().unwrap().take() {
            hook();
        }
        reader.take();
        drop(reader);
        if let Some(database) = old_writer {
            database
                .checkpoint_truncate()
                .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
            drop(database);
        }
        remove_projection_sidecar(&directory, &format!("{destination_name}-wal"))
            .map_err(FreshBuildError::Failed)?;
        remove_projection_sidecar(&directory, &format!("{destination_name}-shm"))
            .map_err(FreshBuildError::Failed)?;

        #[cfg(test)]
        if let Some(hook) = shared.before_fresh_publication.lock().unwrap().take() {
            hook().map_err(FreshBuildError::Failed)?;
        }
        // This is the cancellation boundary. Once the storage primitive below
        // starts, its atomic name operation owns the outcome; a stop arriving
        // after this check may leave the complete new image installed.
        if fresh_build_stopped(shared) {
            return Err(FreshBuildError::Stopped);
        }

        finalized
            .publish_replace_single_writer(&destination_name)
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;

        #[cfg(test)]
        if let Some(hook) = shared.after_fresh_publication.lock().unwrap().take() {
            hook().map_err(FreshBuildError::Failed)?;
        }
        if fresh_build_stopped(shared) {
            return Err(FreshBuildError::Stopped);
        }

        let database = PhysicalGraphProjectionDatabase::open_writable(&shared.path)
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        database
            .validate_schema()
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        database
            .quick_check()
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        database
            .checkpoint_truncate()
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        database
            .shrink_page_cache_budget(crate::projection_budget::resting_page_cache_budget(
                text_bytes,
            ))
            .map_err(|error| FreshBuildError::Failed(error.to_string()))?;
        #[cfg(test)]
        shared.serving_writer_cache_budget.store(
            database
                .page_cache_budget()
                .map_err(|error| FreshBuildError::Failed(error.to_string()))?,
            Ordering::Release,
        );
        Ok(database)
    })();
    match publication {
        Ok(database) => {
            cleanup_projection_stage_artifacts(&directory, &stage_name)
                .map_err(FreshBuildError::Failed)?;
            Ok((database, applied))
        }
        Err(error) => {
            let _ = cleanup_projection_stage_artifacts(&directory, &stage_name);
            Err(error)
        }
    }
}

/// Which pages one worker turn actually WROTE (R3 identity policy): the pages
/// whose rows now carry this process's live runtime ids, and the pages whose
/// rows are gone.
#[derive(Default)]
struct AppliedPages {
    lowered: Vec<String>,
    deleted: Vec<String>,
}

#[derive(Default)]
struct AppliedTurn {
    pages: AppliedPages,
    registry_pages: PageRegistryMetadata,
    /// The warm validation's verdict, when this turn ran one.
    warm_outcome: Option<WarmOutcome>,
}

/// R6 warm validation inside one worker turn: compare the walk inventory's
/// exact revisions with `direct_source_revisions`, delete what the walk no
/// longer has, and name what must be relowered. Nothing here parses.
fn validate_warm(
    database: &PhysicalGraphProjectionDatabase,
    warm: &PendingWarm,
) -> Result<WarmOutcome, String> {
    let config_digest = warm.parse_config.digest();
    let sources = warm
        .sources
        .iter()
        .map(|(entry, revision)| PhysicalGraphProjectionSourceRevision {
            path: entry.rel_path.clone(),
            revision: projection_source_revision(revision, config_digest),
        })
        .collect::<Vec<_>>();
    let mut source_delta = database
        .source_delta(&sources)
        .map_err(|error| error.to_string())?;
    if !warm.retained.is_empty() {
        // A page whose bytes could not be read is absent from `sources`, so
        // `source_delta` names it a deletion — and warm validation would drop
        // the rows of a page that still exists (GH #543). An unreadable page
        // keeps its previous rows AND its previous stored revision, so the
        // next warm names it a replacement and re-reads it once the read
        // succeeds. In-scope scenarios: a transient disk error, and a
        // Windows/macOS sharing violation while another process holds the file.
        let retained = warm
            .retained
            .iter()
            .map(|entry| entry.rel_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        source_delta.deletions.retain(|id| !retained.contains(id));
    }
    if !warm.published.is_empty() {
        let published = warm
            .published
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        source_delta.deletions.retain(|id| !published.contains(id));
        source_delta
            .replacements
            .retain(|id| !published.contains(id));
    }
    if source_delta.replacements.is_empty() && source_delta.deletions.is_empty() {
        return Ok(WarmOutcome::Clean);
    }
    let walk = warm.sources.len() + warm.retained.len();
    let changed = source_delta.replacements.len() + source_delta.deletions.len();
    // An incomplete walk (a page it could not read) is never repaired: the
    // repaired image would mix fresh pages with a page whose current bytes
    // nobody has seen, so the old coherent image stays until a complete
    // reconstruction succeeds.
    if !warm.retained.is_empty() || changed * WARM_REPAIR_MAX_SHARE_DIVISOR > walk {
        return Ok(WarmOutcome::FreshBuildRequired);
    }
    Ok(WarmOutcome::Changed {
        replacements: source_delta.replacements,
        deletions: source_delta.deletions,
    })
}

/// Apply a `WarmRepair` in one transaction and reconcile every stored page's
/// position to the queue's order (the walk, then pages this session created),
/// restricted to the pages the image holds afterwards. Returns that order.
fn apply_warm_repair(
    database: &mut PhysicalGraphProjectionDatabase,
    shared: &ProjectionShared,
    repair: WarmRepair,
    parse_config: &Arc<ParseConfig>,
) -> Result<Vec<String>, String> {
    let mut deltas = BTreeMap::new();
    for (entry, document, revision) in repair.replacements {
        deltas.insert(
            entry.rel_path.clone(),
            (
                0,
                PageDelta::Replace {
                    entry,
                    document,
                    revision,
                    parse_config: Arc::clone(parse_config),
                    page_position: None,
                },
            ),
        );
    }
    let lowered = lower_deltas(deltas)?;
    let mut change = lowered.change;
    change.deletions = repair.deletions;
    // Against an empty inventory every stored page reads as a deletion: that
    // is the set of paths the image holds.
    let mut after = database
        .source_delta(&[])
        .map_err(|error| error.to_string())?
        .deletions
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    for path in &change.deletions {
        after.remove(path);
    }
    after.extend(change.replacements.iter().map(|page| page.path.clone()));
    let mut order = shared
        .pending
        .lock()
        .unwrap()
        .ordered_inventory()
        .into_iter()
        .filter(|path| after.remove(path))
        .collect::<Vec<_>>();
    // Nothing should be left; anything that is still gets a place at the end.
    order.extend(after);
    projection_diag(|| {
        format!(
            "warm repair: relowering {} page(s), deleting {}",
            change.replacements.len(),
            change.deletions.len()
        )
    });
    database
        .apply_with_source_revisions_aliases_and_page_order(
            &change,
            &lowered.revisions,
            &lowered.aliases,
            &order,
        )
        .map_err(|error| error.to_string())?;
    Ok(order)
}

/// GH #550: settle the deltas a session published before it had seeded its
/// page order (they carry no position). Launch reads -- the Journals feed
/// loading its first days -- publish every page they read, and they arrive
/// before the warm has validated the reopened image.
///
/// - A page the image already holds at this exact source revision is
///   dropped: re-lowering it writes rows identical to the ones stored.
/// - A changed page the image holds is applied without a position, so it
///   keeps its stored one.
/// - A page the image does not hold cannot be placed without the session's
///   inventory. It is returned second, to wait for that inventory instead of
///   inventing a position (`PendingProjection::unplaced`).
#[allow(clippy::type_complexity)]
fn settle_unseeded_deltas(
    database: &PhysicalGraphProjectionDatabase,
    mut deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<
    (
        BTreeMap<String, (u64, PageDelta)>,
        BTreeMap<String, (u64, PageDelta)>,
    ),
    String,
> {
    let unseeded = deltas
        .values()
        .filter_map(|(_, delta)| match delta {
            PageDelta::Replace {
                entry,
                revision,
                parse_config,
                page_position: None,
                ..
            } => Some(PhysicalGraphProjectionSourceRevision {
                path: entry.rel_path.clone(),
                revision: projection_source_revision(revision, parse_config.digest()),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut unplaced = BTreeMap::new();
    if unseeded.is_empty() {
        return Ok((deltas, unplaced));
    }
    // Against an empty inventory every stored page reads as a deletion: that
    // is the set of paths the image holds.
    let stored = database
        .source_delta(&[])
        .map_err(|error| error.to_string())?
        .deletions
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let changed = database
        .source_delta(&unseeded)
        .map_err(|error| error.to_string())?
        .replacements
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    for source in unseeded {
        if !stored.contains(&source.path) {
            if let Some(update) = deltas.remove(&source.path) {
                unplaced.insert(source.path, update);
            }
            continue;
        }
        if !changed.contains(&source.path) {
            deltas.remove(&source.path);
        }
    }
    Ok((deltas, unplaced))
}

fn apply_deltas(
    database: &mut PhysicalGraphProjectionDatabase,
    deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<AppliedTurn, String> {
    if deltas.is_empty() {
        return Ok(AppliedTurn::default());
    }
    let lowered = lower_deltas(deltas)?;
    database
        .apply_with_source_revisions_and_aliases(
            &lowered.change,
            &lowered.revisions,
            &lowered.aliases,
        )
        .map_err(|error| error.to_string())?;
    Ok(lowered.applied)
}

struct LoweredDeltas {
    applied: AppliedTurn,
    change: PhysicalGraphProjectionChange,
    revisions: Vec<PhysicalGraphProjectionSourceRevision>,
    aliases: Vec<PhysicalAliasDeclaration>,
}

fn lower_deltas(deltas: BTreeMap<String, (u64, PageDelta)>) -> Result<LoweredDeltas, String> {
    let mut turn = AppliedTurn::default();
    let mut replacements = Vec::new();
    let mut reference_postings = Vec::new();
    let mut aliases = Vec::new();
    let mut replacement_sources = Vec::new();
    let mut deletions = Vec::new();
    for (_, (_, delta)) in deltas {
        match delta {
            PageDelta::Replace {
                entry,
                document,
                revision,
                parse_config,
                page_position,
            } => {
                replacement_sources.push(PhysicalGraphProjectionSourceRevision {
                    path: entry.rel_path.clone(),
                    revision: projection_source_revision(&revision, parse_config.digest()),
                });
                let (mut page, mut postings, mut page_aliases) =
                    physical_page(&entry, &document, &parse_config)?;
                page.position = page_position;
                turn.pages.lowered.push(page.path.clone());
                replacements.push(page);
                reference_postings.append(&mut postings);
                aliases.append(&mut page_aliases);
            }
            PageDelta::Delete { entry } => {
                let id = entry.rel_path;
                turn.pages.deleted.push(id.clone());
                deletions.push(id);
            }
        }
    }
    Ok(LoweredDeltas {
        applied: turn,
        change: PhysicalGraphProjectionChange {
            replacements,
            deletions,
            reference_postings,
        },
        revisions: replacement_sources,
        aliases,
    })
}

/// The revision Direct Files compares to decide whether a page's rows are still
/// current. Folding the parse-config digest in is what makes a config edit a
/// full re-lowering (§5.8 J7): reconciliation compares only source revisions,
/// so without it an unchanged file would keep rows built under the old config.
pub(crate) fn projection_source_revision(
    content_revision: &str,
    parse_config_digest: tine_storage::ContentDigest,
) -> String {
    let digest = parse_config_digest
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("direct-facts-v{DIRECT_PROJECTION_FACTS_VERSION}:{digest}:{content_revision}")
}

/// The page and block text a snapshot projects — the input to
/// `projection_budget::build_page_cache_budget`, because the build's cache
/// working set is a schema-fixed multiple of it, not of the page count.
fn projected_text_bytes(pages: &[tine_storage::sqlite::PhysicalPage]) -> u64 {
    pages
        .iter()
        .map(|page| {
            let own = page.search_tokens.len() as u64
                + page.preamble.as_ref().map_or(0, |text| text.len() as u64);
            own + page
                .blocks
                .iter()
                .map(|block| (block.content.len() + block.search_tokens.len()) as u64)
                .sum::<u64>()
        })
        .sum()
}

/// The Direct Files producer, reachable from the cross-backend parity guard.
///
/// Named as a seam rather than widened: the guard has to compare the rows this
/// exact function emits against the walk's,
/// and a reimplementation in the test would prove only that the test agrees
/// with itself (§5.8 G1, I-19).
#[cfg(test)]
pub(crate) fn physical_page_for_test(
    entry: &PageEntry,
    document: &Document,
    parse_config: &ParseConfig,
) -> Result<PhysicalPage, String> {
    physical_page(entry, document, parse_config).map(|(page, _, _)| page)
}

fn physical_page(
    entry: &PageEntry,
    document: &Document,
    parse_config: &ParseConfig,
) -> Result<
    (
        PhysicalPage,
        Vec<PhysicalReferencePosting>,
        Vec<PhysicalAliasDeclaration>,
    ),
    String,
> {
    #[cfg(test)]
    {
        let mut receipt = PHYSICAL_PAGE_LOWERINGS.lock().unwrap();
        if receipt
            .0
            .as_ref()
            .is_some_and(|root| entry.path.starts_with(root))
        {
            receipt.1 += 1;
        }
    }
    let path = entry.rel_path.as_str();
    let format = Format::from_path(Path::new(&entry.rel_path));
    let is_org = format == Format::Org;
    // `Format::from_path` and never `reference_source_is_org`: the latter is a
    // case-sensitive `ends_with(".org")` and would type an `Outline.ORG` page
    // Markdown here while Direct Files types it Org (§5.8 E4).
    let atom_format = crate::query::atom::AtomFormat::from(format);
    let (preamble_visible, properties, tags) = document
        .pre_block
        .as_deref()
        .map(|raw| facets(raw, is_org))
        .unwrap_or_default();
    let visible_search_text = if preamble_visible.is_empty() {
        entry.name.clone()
    } else {
        format!("{} {preamble_visible}", entry.name)
    };
    let mut blocks = Vec::new();
    let mut reference_postings = Vec::new();
    let aliases = crate::query::document_alias_spellings(document)
        .into_iter()
        .enumerate()
        .map(|(ordinal, (raw_alias, normalized_alias))| {
            Ok(PhysicalAliasDeclaration {
                source_page_path: path.to_owned(),
                source_entity: PhysicalEntityId::Page(path.to_owned()),
                source_locator: b"page-alias".to_vec(),
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| "one page exceeds u32::MAX aliases".to_string())?,
                raw_alias,
                normalized_alias,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(preamble) = document.pre_block.as_deref() {
        append_reference_postings(
            &mut reference_postings,
            path,
            PhysicalEntityId::Page(path.to_owned()),
            b"preamble",
            std::iter::empty(),
            crate::doc::property_reference_page_names(preamble).into_iter(),
        )?;
    }
    let mut block_refs_norm: Vec<Vec<String>> = Vec::new();
    lower_blocks(
        &document.roots,
        path,
        None,
        &mut Vec::new(),
        &mut blocks,
        &mut reference_postings,
        &mut block_refs_norm,
        parse_config,
        atom_format,
    )?;
    // The two derived tables come from the ONE tine-core computation (§5.8):
    // this side only hands it the block's own `refs_norm` and its parent.
    let block_indices = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.result_id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let flat = blocks
        .iter()
        .enumerate()
        .zip(block_refs_norm.iter())
        .map(
            |((index, block), refs)| crate::query::path_refs::PathRefBlock {
                id: index,
                parent: block
                    .parent
                    .as_deref()
                    .and_then(|parent| block_indices.get(parent).copied()),
                refs: refs.as_slice(),
            },
        )
        .collect::<Vec<_>>();
    let mut path_refs = crate::query::derived::path_ref_rows(&entry.name, &flat);
    for (index, block) in blocks.iter_mut().enumerate() {
        block.path_refs = path_refs
            .remove(&index)
            .unwrap_or_default()
            .into_iter()
            .map(|key| PhysicalName {
                raw: key.clone(),
                key,
            })
            .collect();
    }
    let journal_days = crate::query::derived::JournalDays::new(parse_config);
    let page_property_atoms = crate::query::derived::property_atom_rows(
        &properties
            .iter()
            .map(|property| (property.name.clone(), property.value.clone()))
            .collect::<Vec<_>>(),
        atom_format,
        parse_config,
    );
    Ok((
        PhysicalPage {
            position: None,
            name: entry.name.clone(),
            name_key: crate::refs::page_key(&entry.name),
            path: entry.rel_path.clone(),
            text_kind: page_kind_to_sql(entry.kind),
            journal_day: journal_days.day(&entry.rel_path, entry.kind == PageKind::Journal),
            preamble: document.pre_block.clone(),
            search_tokens: crate::search_query::canonical_fold(&visible_search_text),
            properties,
            tags: crate::query::derived::tag_rows(&tags),
            property_atoms: page_property_atoms,
            blocks,
        },
        reference_postings,
        aliases,
    ))
}

#[allow(clippy::too_many_arguments)]
fn lower_blocks(
    source: &[DocBlock],
    page_path: &str,
    parent: Option<&str>,
    structural_path: &mut Vec<u32>,
    out: &mut Vec<PhysicalBlock>,
    reference_postings: &mut Vec<PhysicalReferencePosting>,
    refs_norm: &mut Vec<Vec<String>>,
    parse_config: &ParseConfig,
    atom_format: crate::query::atom::AtomFormat,
) -> Result<(), String> {
    for (position, block) in source.iter().enumerate() {
        let position = u32::try_from(position)
            .map_err(|_| "page has more than u32::MAX sibling blocks".to_string())?;
        structural_path.push(position);
        let projection = block.projection();
        let order = structural_path
            .iter()
            .map(|part| format!("{part:08x}"))
            .collect::<Vec<_>>()
            .join("/");
        append_reference_postings(
            reference_postings,
            page_path,
            PhysicalEntityId::Block(block.uuid.clone()),
            order.as_bytes(),
            projection.refs_page.iter().cloned(),
            crate::doc::property_reference_page_names(&block.raw).into_iter(),
        )?;
        for raw_claim in &projection.block_refs {
            let Ok(raw_claim) = Uuid::parse_str(raw_claim) else {
                continue;
            };
            reference_postings.push(PhysicalReferencePosting {
                source_page_path: page_path.to_owned(),
                source_entity: PhysicalEntityId::Block(block.uuid.clone()),
                source_locator: order.as_bytes().to_vec(),
                ordinal: u32::try_from(reference_postings.len())
                    .map_err(|_| "one page exceeds u32::MAX reference postings".to_string())?,
                kind: 6,
                target: PhysicalReferenceTarget::ExternalUuid {
                    raw_claim: raw_claim.into_bytes(),
                },
            });
        }
        let properties = projection
            .properties
            .iter()
            .map(|(name, value)| PhysicalProperty {
                name: name.clone(),
                normalized_name: property_key_norm(name),
                value: value.clone(),
            })
            .collect();
        let property_atoms = crate::query::derived::property_atom_rows(
            &projection.properties,
            atom_format,
            parse_config,
        );
        refs_norm.push(projection.refs_norm.clone());
        let logseq_uuid = block
            .property("id")
            .and_then(|value| Uuid::parse_str(value.trim()).ok())
            .map(Uuid::into_bytes);
        out.push(PhysicalBlock {
            result_id: block.uuid.clone(),
            own_refs: projection
                .refs_norm
                .iter()
                .map(|key| PhysicalName {
                    raw: key.clone(),
                    key: key.clone(),
                })
                .collect(),
            parent: parent.map(str::to_owned),
            order,
            content: block.raw.clone(),
            search_tokens: projection.visible_lower.clone(),
            heading_level: projection.heading_level,
            collapsed: block.collapsed(),
            logseq_uuid,
            logseq_identity_origin: logseq_uuid.map(|_| 0),
            properties,
            tags: crate::query::derived::tag_rows(&projection.tags),
            task: projection.marker.as_ref().map(|marker| PhysicalTask {
                marker: marker.to_ascii_uppercase(),
                priority: projection.priority.clone(),
                scheduled: projection.scheduled.clone(),
                deadline: projection.deadline.clone(),
            }),
            // Written from the three projection fields alone, so a markerless
            // block gets a row exactly as a marked one does (§3.2 M2).
            planning: crate::query::derived::planning_row(
                projection.priority.as_deref(),
                projection.scheduled.as_deref(),
                projection.deadline.as_deref(),
            ),
            // Filled once per page, after the whole flat block list exists.
            path_refs: Vec::new(),
            property_atoms,
        });
        lower_blocks(
            &block.children,
            page_path,
            Some(block.uuid.as_str()),
            structural_path,
            out,
            reference_postings,
            refs_norm,
            parse_config,
            atom_format,
        )?;
        structural_path.pop();
    }
    Ok(())
}

fn append_reference_postings(
    out: &mut Vec<PhysicalReferencePosting>,
    page_path: &str,
    source: PhysicalEntityId,
    source_locator: &[u8],
    inline_names: impl IntoIterator<Item = String>,
    property_names: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    let mut ordinal = 0_u32;
    for (kind, names) in [
        (0_i64, inline_names.into_iter().collect::<Vec<_>>()),
        (3_i64, property_names.into_iter().collect::<Vec<_>>()),
    ] {
        for raw_name in names {
            out.push(PhysicalReferencePosting {
                source_page_path: page_path.to_owned(),
                source_entity: source.clone(),
                source_locator: source_locator.to_vec(),
                ordinal,
                kind,
                target: PhysicalReferenceTarget::PageName {
                    normalized_name: crate::refs::page_key(&raw_name),
                    raw_name,
                },
            });
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| "one reference source exceeds u32::MAX postings".to_string())?;
        }
    }
    Ok(())
}

fn facets(raw: &str, is_org: bool) -> (String, Vec<PhysicalProperty>, Vec<String>) {
    let block = DocBlock::preamble(raw, is_org);
    let searchable = block.visible_text().to_owned();
    let properties = block
        .projection()
        .properties
        .iter()
        .map(|(name, value)| PhysicalProperty {
            name: name.clone(),
            normalized_name: property_key_norm(name),
            value: value.clone(),
        })
        .collect();
    (searchable, properties, block.projection().tags.clone())
}

fn page_kind_to_sql(kind: PageKind) -> i64 {
    match kind {
        PageKind::Page => 0,
        PageKind::Journal => 1,
    }
}

/// `pages.text_kind` back to the parser's `PageKind`. A value outside the two
/// the producer writes is projection damage, not a third kind, so every reader
/// treats `None` as a failed read (D-3).
pub(crate) fn page_kind_from_sql(kind: i64) -> Option<PageKind> {
    match kind {
        0 => Some(PageKind::Page),
        1 => Some(PageKind::Journal),
        _ => None,
    }
}

#[cfg(test)]
/// Release a projection so the SAME database can be reattached.
///
/// `Drop` deliberately does NOT wait for the worker (see
/// [`DirectProjection::close_and_wait_for_worker`]: an app teardown must not
/// block on SQLite), and the worker releases its exclusive writer lease only
/// just before it publishes its exit. So a fixture that drops one graph and
/// immediately reattaches the same path can find the lease still held. The
/// second instance then never becomes ready -- by design, proven by
/// `concurrent_graph_instance_cannot_replace_ready_projection_facts` -- and
/// `wait_ready` spins its whole 15s before panicking "did not converge" with
/// `cache_generation=0`, naming the reopen rather than the handoff.
///
/// That is not hypothetical: it is what took down the Linux release
/// selection on 2026-09-09, on a loaded hosted runner, in two tests that
/// pass locally in 0.05s. Every fixture that reopens a projection database
/// calls this first.
pub(crate) fn release_projection<G: crate::query::graph::QueryGraph>(graph: &G) {
    let Some(projection) = graph.direct_projection_test() else {
        return;
    };
    assert!(
        projection.close_and_wait_for_worker(std::time::Duration::from_secs(15)),
        "the projection worker did not release its writer lease, so reattaching \
         the same database would race it"
    );
}

#[cfg(test)]
/// Retry projection recovery until it converges.
///
/// `direct_projection_recover_after_failed_read` is ONE attempt and is allowed
/// to accomplish nothing -- `model.rs` says so itself where it turns "the
/// repair did not take" into `Unavailable(ReadFailed)`. If the turn carrying a
/// rebuild's payload fails, the payload is gone and the attempt achieved
/// nothing, so the projection stays failed until something enqueues work again.
///
/// This helper calls the recovery entry point DIRECTLY, which the running app
/// does not do: the app issues ordinary queries and the dispatcher decides. So
/// this cannot be the proof that public recovery converges, and it once hid the
/// fact that it did not — a latched `rebuild` made every query retryable rather
/// than repairing, and nothing called recovery again (GH #543). That property
/// is proved separately through the public query route by
/// `a_failed_statement_read_repairs_and_retries_the_same_statement`. Keep it
/// that way: do not "fix" a convergence failure by reaching for this helper.
///
/// A fixture that calls recovery once and then waits has assumed a convergence
/// guarantee the contract does not make. It fails about one run in twenty on a
/// loaded machine and passes every time on an idle one, which is how
/// `current_snapshot_write_failure_recovers_from_authoritative_source` took
/// down the Linux release selection on 2026-09-10 after passing all night.
pub(crate) fn recover_until_ready<G: crate::query::graph::QueryGraph>(graph: &G) {
    let started = std::time::Instant::now();
    let budget = std::time::Duration::from_secs(30);
    loop {
        graph.direct_projection_recover_after_failed_read();
        let attempt = std::time::Instant::now();
        while !graph.direct_projection_ready_test() {
            if attempt.elapsed() >= std::time::Duration::from_millis(500)
                || started.elapsed() >= budget
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if graph.direct_projection_ready_test() {
            return;
        }
        assert!(
            started.elapsed() < budget,
            "Direct Files projection did not converge across repeated recovery attempts: \
             cache_generation={} {}",
            graph.cache_generation(),
            graph
                .direct_projection_test()
                .map(|projection| projection.debug_state_test())
                .unwrap_or_else(|| "no projection".to_owned())
        );
    }
}

pub(crate) mod derived_reads;
#[cfg(test)]
#[path = "direct_projection_tests.rs"]
mod tests;

mod observers;
