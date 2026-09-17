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
use fs2::FileExt as _;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tine_storage::sqlite::{
    PhysicalAliasDeclaration, PhysicalBlock, PhysicalEntityId, PhysicalGraphProjectionChange,
    PhysicalGraphProjectionDatabase, PhysicalGraphProjectionSourceRevision, PhysicalPage,
    PhysicalProjectionQuerySnapshot, PhysicalProperty, PhysicalQueryValue,
    PhysicalReferencePosting, PhysicalReferenceTarget, PhysicalTask,
};
use unicode_normalization::UnicodeNormalization;
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
/// R6: how many streamed warm deltas may wait in the queue before the warm
/// thread parses the next batch. It bounds what a cold or changed open retains
/// beyond the worker's current turn to one batch of documents, instead of the
/// whole parsed graph the full snapshot used to pin (plan §2D).
pub(crate) const WARM_STREAM_HIGH_WATER: usize = 64;

#[cfg(test)]
// Test receipts count only their own graph, including its worker threads.
static PHYSICAL_PAGE_LOWERINGS: Mutex<(Option<PathBuf>, u64)> = Mutex::new((None, 0));
/// R6 test receipt: the most deltas a warm stream ever left queued, so a test
/// can prove the stream never retained more than `WARM_STREAM_HIGH_WATER`.
#[cfg(test)]
static MAX_PENDING_DELTAS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static BEFORE_APPLY_PENDING: Mutex<Option<Box<dyn FnOnce() + Send>>> = Mutex::new(None);

#[cfg(test)]
fn run_before_apply_pending_hook() {
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
        /// `None` while a warm stream is open (R6): the rows carry no order
        /// position until the stream's closing order turn reconciles the
        /// whole `query_page_order` table, because a position written mid-stream
        /// could collide with a retained page's previous-session position.
        query_page_order: Option<u64>,
        identity: DeltaIdentity,
    },
    Delete {
        entry: PageEntry,
    },
}

/// R6 session identity rule (WARM-IDENTITY-ORDER-CONTRACT.md item 3): where a
/// replacement's runtime ids came from decides whether the page joins or
/// leaves `ProjectionShared::session_pages`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeltaIdentity {
    /// The document is this process's live one (a save, or a parsed-cache
    /// snapshot that may carry preserved ids): the stored `result_id`s ARE the
    /// public ids, so the page is added.
    Live,
    /// A fresh parse (the warm stream): the stored ids are structural, equal to
    /// what `doc_runtime_id_for_order` derives, so the page is removed — an
    /// external incompatible revision invalidates any live mapping it had.
    Structural,
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
}

/// R6 warm validation: the walk inventory with each page's exact content
/// revision, and nothing parsed. The worker compares it with
/// `direct_source_revisions`; an unchanged graph publishes readiness from this
/// alone, a changed one names the pages the warm thread must parse.
struct PendingWarm {
    sources: Vec<(PageEntry, String)>,
    parse_config: Arc<ParseConfig>,
}

/// What the worker's warm-validation turn decided (R6), read by the warm
/// thread through `wait_warm_outcome`.
#[derive(Clone, Debug)]
pub(crate) enum WarmOutcome {
    /// Every walk page's rows are current: readiness publishes without a parse.
    Clean,
    /// These pages' rows are missing or stale; the warm thread streams them.
    Replacements(Vec<PageEntry>),
    /// A full parsed snapshot arrived first and owns readiness.
    Superseded,
    /// The validation turn failed; the parser fallback owns readiness.
    Failed,
}

/// One page of the R6 warm stream, as the warm thread hands it over.
pub(crate) enum WarmStreamItem {
    Replace {
        entry: PageEntry,
        document: Arc<Document>,
        revision: String,
        identity: DeltaIdentity,
    },
    Delete {
        entry: PageEntry,
    },
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
    /// R6 warm validation queued for the worker.
    warm: Option<PendingWarm>,
    /// R6: the worker's verdict on the last warm validation.
    warm_outcome: Option<WarmOutcome>,
    /// R6: a warm stream is open at this generation. Readiness never publishes
    /// while it is `Some`, and deltas recorded meanwhile carry no order
    /// position (see `PageDelta::Replace::query_page_order`).
    warm_stream: Option<u64>,
    /// R6: the stream's closing turn — reconcile `query_page_order` over the
    /// queue's own inventory and then publish readiness.
    order: Option<u64>,
    /// R6: a full snapshot was queued after the warm; the stream must stop
    /// enqueueing (its deltas would drop the snapshot's order rows).
    warm_superseded: bool,
    /// R6: an abandoned stream left stale rows behind; only a full snapshot may
    /// publish readiness again (the worker turns this into
    /// `requires_full_rebuild`).
    needs_full: bool,
}

impl PendingProjection {
    fn record_delta(&mut self, generation: u64, mut delta: PageDelta) {
        let key = delta.entry().rel_path.clone();
        match &mut delta {
            PageDelta::Replace {
                query_page_order, ..
            } => {
                let position = if let Some(position) = self.page_order.get(&key) {
                    *position
                } else {
                    let position = self.next_page_order;
                    self.next_page_order += 1;
                    self.page_order.insert(key.clone(), position);
                    position
                };
                *query_page_order = self.warm_stream.is_none().then_some(position);
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
        self.next_page_order = inventory.len() as u64;
        self.page_order = inventory
            .into_iter()
            .enumerate()
            .map(|(position, rel_path)| (rel_path.to_owned(), position as u64))
            .collect();
    }

    /// The queue's own page inventory in position order: the R6 order turn's
    /// authority. After a warm seed the map tracks every applied replacement
    /// and deletion, so it names exactly the pages the projection holds.
    fn ordered_inventory(&self) -> Vec<[u8; 16]> {
        let mut ordered = self
            .page_order
            .iter()
            .map(|(rel_path, position)| (*position, page_id(rel_path)))
            .collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(position, _)| *position);
        ordered.into_iter().map(|(_, id)| id).collect()
    }

    fn has_work(&self) -> bool {
        self.full.is_some()
            || !self.deltas.is_empty()
            || self.warm.is_some()
            || self.order.is_some()
            || self.needs_full
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
    /// `query_block_results.result_id` is the live runtime id the parsed
    /// document carried when the row was written. Every other page's rows
    /// survived from an earlier session, and a fresh parse of an unchanged
    /// page assigns STRUCTURAL runtime ids, so their public id is derived from
    /// `(path, order_key)` through `model::doc_runtime_id_for_order` instead.
    /// Copy-on-write: the worker swaps a new `Arc` after each successful
    /// apply, and a job clones the `Arc` at snapshot acquisition — never a
    /// live lookup during output.
    session_pages: Mutex<Arc<HashSet<[u8; 16]>>>,
    committed_registry: SharedCommittedRegistry,
    worker_available: AtomicBool,
    worker_failed: AtomicBool,
    worker_busy: AtomicBool,
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
    #[cfg(test)]
    fuzzy_candidate_reads: AtomicU64,
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
        if applied.lowered.is_empty()
            && applied.deleted.is_empty()
            && applied.relowered_structurally.is_empty()
        {
            return;
        }
        let mut current = self.session_pages.lock().unwrap();
        let mut next: HashSet<[u8; 16]> = (**current).clone();
        next.extend(applied.lowered.iter().copied());
        for page in applied
            .deleted
            .iter()
            .chain(applied.relowered_structurally.iter())
        {
            next.remove(page);
        }
        *current = Arc::new(next);
    }
}

/// Completeness gate shared by admission and producer snapshot capture.
/// Live queries may read an older committed image, but a partial warm stream,
/// replacement or failed write must not be mistaken for a complete graph.
fn query_capture_available(
    shared: &ProjectionShared,
    requirement: &QueryCaptureRequirement,
) -> bool {
    match requirement {
        QueryCaptureRequirement::CurrentSnapshot => {
            let pending = shared.pending.lock().unwrap();
            !pending.stop
                && !pending.rebuild
                && !pending.needs_full
                && pending.warm_stream.is_none()
                && shared.validated.load(Ordering::Acquire)
                && !shared.worker_failed.load(Ordering::Acquire)
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
    pub(crate) session_pages: Arc<HashSet<[u8; 16]>>,
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
                    page_id(&entry.rel_path),
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
        let read = self.snapshot.visit_projection_query(
            "SELECT p.page_id, s.revision FROM pages p \
             LEFT JOIN direct_source_revisions s ON s.page_id = p.page_id \
             ORDER BY p.page_id",
            &[],
            |row| {
                let [PhysicalQueryValue::Blob(id), PhysicalQueryValue::Text(revision)] = row else {
                    malformed = true;
                    return Ok(std::ops::ControlFlow::Break(()));
                };
                if id.len() != 16 {
                    malformed = true;
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                if !expected
                    .get(at)
                    .is_some_and(|(wanted_id, wanted_revision)| {
                        wanted_id.as_slice() == id.as_slice() && wanted_revision == revision
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
    pub blocks: Option<std::collections::HashSet<[u8; 16]>>,
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
            worker_finished: AtomicBool::new(false),
            worker_resources: Mutex::new(Some(Vec::new())),
            validated: AtomicBool::new(false),
            #[cfg(test)]
            after_sql_commit: Mutex::new(None),
            #[cfg(test)]
            capture_thread: Mutex::new(None),
            #[cfg(test)]
            indexed_reads: AtomicU64::new(0),
            #[cfg(test)]
            statement_reads: AtomicU64::new(0),
            #[cfg(test)]
            registry_capture_attempts: AtomicU64::new(0),
            repairs_in_flight: AtomicUsize::new(0),
            #[cfg(test)]
            inject_read_failure: AtomicBool::new(false),
            #[cfg(test)]
            fallback_reads: AtomicU64::new(0),
            #[cfg(test)]
            referenced_name_reads: AtomicU64::new(0),
            #[cfg(test)]
            fuzzy_candidate_reads: AtomicU64::new(0),
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

    /// True while the last worker turn failed. The flag clears on the next
    /// successful turn, so it names a projection that owes a reset — not one
    /// that has merely never started.
    pub(crate) fn worker_failed(&self) -> bool {
        self.shared.worker_failed.load(Ordering::Acquire)
    }

    pub(crate) fn request_rebuild(&self) {
        let mut pending = self.shared.pending.lock().unwrap();
        pending.rebuild = true;
        self.shared.ready.store(false, Ordering::Release);
    }

    pub(crate) fn enqueue_full(
        &self,
        generation: u64,
        pages: PageSnapshot,
        revisions: PageRevisions,
        parse_config: Arc<ParseConfig>,
    ) {
        self.shared.ready.store(false, Ordering::Release);
        self.shared.worker_failed.store(false, Ordering::Release);
        let mut pending = self.shared.pending.lock().unwrap();
        pending.seed_page_order(pages.iter().map(|(entry, _)| entry.rel_path.as_str()));
        pending.full = Some(PendingFull {
            pages,
            revisions,
            parse_config,
        });
        pending.deltas.clear();
        pending.latest_generation = generation;
        // R6: a complete parsed snapshot owns readiness from here. A warm
        // validation or stream still in flight must not lower beside it — its
        // deltas carry no order positions and would erase the snapshot's.
        if pending.warm.is_some() || pending.warm_stream.is_some() || pending.order.is_some() {
            pending.warm = None;
            pending.warm_stream = None;
            pending.order = None;
            pending.warm_superseded = true;
            pending.warm_outcome = Some(WarmOutcome::Superseded);
        }
        pending.needs_full = false;
        self.shared.changed.notify_all();
    }

    /// R6 warm validation: hand the worker the walk inventory with exact
    /// content revisions and nothing parsed. Refused (`false`) when a newer
    /// mutation or queued work already outranks this generation — the caller
    /// then leaves readiness to the parser fallback, exactly as
    /// `install_built` does on generation drift.
    pub(crate) fn enqueue_warm(
        &self,
        generation: u64,
        sources: Vec<(PageEntry, String)>,
        parse_config: Arc<ParseConfig>,
    ) -> bool {
        if !self.shared.worker_available.load(Ordering::Acquire) {
            return false;
        }
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.has_work()
            || pending.warm_stream.is_some()
            || pending.latest_generation > generation
            || (self.shared.worker_failed.load(Ordering::Acquire) && !pending.rebuild)
        {
            return false;
        }
        self.shared.ready.store(false, Ordering::Release);
        pending.seed_page_order(sources.iter().map(|(entry, _)| entry.rel_path.as_str()));
        // Optimistically open the stream now so every delta recorded from here
        // until the outcome carries no order position; the worker closes it
        // again in the same turn when the outcome is `Clean`.
        pending.warm_stream = Some(generation);
        pending.warm_outcome = None;
        pending.warm_superseded = false;
        pending.warm = Some(PendingWarm {
            sources,
            parse_config,
        });
        pending.latest_generation = generation;
        self.shared.changed.notify_all();
        true
    }

    /// Block until the worker has decided the queued warm validation.
    pub(crate) fn wait_warm_outcome(&self) -> WarmOutcome {
        let mut pending = self.shared.pending.lock().unwrap();
        loop {
            if let Some(outcome) = pending.warm_outcome.take() {
                return outcome;
            }
            if !self.shared.worker_available.load(Ordering::Acquire) || pending.stop {
                return WarmOutcome::Failed;
            }
            pending = self.shared.changed.wait(pending).unwrap();
        }
    }

    /// R6 stream back-pressure: wait until fewer than `WARM_STREAM_HIGH_WATER`
    /// deltas are queued, so the warm thread never parses further ahead than
    /// one batch beyond the worker's current turn. `false` when the stream is
    /// no longer this thread's to feed (superseded, failed, or drifted).
    pub(crate) fn warm_stream_admit(&self, generation: u64, batch_len: usize) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        loop {
            if pending.warm_superseded
                || pending.warm_stream != Some(generation)
                || pending.latest_generation > generation
                || pending.stop
                || !self.shared.worker_available.load(Ordering::Acquire)
                || self.shared.worker_failed.load(Ordering::Acquire)
            {
                return false;
            }
            if pending.deltas.len() + batch_len <= WARM_STREAM_HIGH_WATER {
                return true;
            }
            pending = self.shared.changed.wait(pending).unwrap();
        }
    }

    /// Queue one parsed batch of the warm stream. The session identity owner
    /// marks exact-revision restored IDs as Live, fresh IDs as Structural. A page that failed to
    /// parse is deleted from the projection. `false` means the batch was
    /// refused: a newer mutation outranks this generation, a full snapshot
    /// superseded the stream, or the worker failed — the caller abandons.
    pub(crate) fn enqueue_warm_stream(
        &self,
        generation: u64,
        batch: Vec<WarmStreamItem>,
        parse_config: Arc<ParseConfig>,
    ) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.warm_superseded
            || pending.warm_stream != Some(generation)
            || pending.latest_generation > generation
            || self.shared.worker_failed.load(Ordering::Acquire)
        {
            return false;
        }
        for item in batch {
            let delta = match item {
                WarmStreamItem::Replace {
                    entry,
                    document,
                    revision,
                    identity,
                } => PageDelta::Replace {
                    entry,
                    document,
                    revision,
                    parse_config: Arc::clone(&parse_config),
                    query_page_order: None,
                    identity,
                },
                WarmStreamItem::Delete { entry } => PageDelta::Delete { entry },
            };
            pending.record_delta(generation, delta);
        }
        #[cfg(test)]
        MAX_PENDING_DELTAS.fetch_max(pending.deltas.len() as u64, Ordering::Relaxed);
        self.shared.changed.notify_all();
        true
    }

    /// Close the warm stream (R6): the worker reconciles `query_page_order`
    /// over the queue's inventory and then publishes readiness. `false` when
    /// the stream is no longer this thread's; a superseding snapshot owns
    /// readiness in that case and nothing is owed.
    pub(crate) fn finish_warm_stream(&self, generation: u64) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.warm_superseded {
            return true;
        }
        if pending.warm_stream != Some(generation)
            || pending.latest_generation > generation
            || self.shared.worker_failed.load(Ordering::Acquire)
        {
            return false;
        }
        pending.order = Some(generation);
        self.shared.changed.notify_all();
        true
    }

    /// Abandon an open warm stream (R6: cancellation, drift, or a refused
    /// batch). Rows already validated or streamed are consistent, but the
    /// replacements not yet streamed are stale; only a full snapshot may
    /// publish readiness again. In-scope scenario: a save racing the warm.
    /// Returns whether a full snapshot superseded the stream — in which case
    /// that snapshot owns readiness and the caller has nothing to fall back to.
    pub(crate) fn abandon_warm_stream(&self, generation: u64) -> bool {
        let mut pending = self.shared.pending.lock().unwrap();
        if pending.warm_superseded {
            return true;
        }
        if pending.warm_stream != Some(generation) {
            return false;
        }
        pending.warm_stream = None;
        pending.order = None;
        pending.needs_full = true;
        self.shared.ready.store(false, Ordering::Release);
        self.shared.changed.notify_all();
        false
    }

    /// R6: the projected page inventory as `(name, path, text_kind)` rows,
    /// read through `drain_after` from the ready projection. `list_pages`
    /// rebuilds `PageEntry`s from it instead of parsing every file.
    pub(crate) fn page_inventory(
        &self,
        cache_generation: u64,
    ) -> Option<Vec<(String, String, i64)>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut rows = Vec::new();
        drain_after(
            |cursor: Option<([u8; 16], String)>, batch| {
                read.navigation_pages_after_with_header_validation(
                    cursor.as_ref().map(|(_, path)| path.as_str()),
                    cursor.as_ref().map(|(id, _)| id),
                    batch,
                    |_, kind| match kind {
                        0 | 1 => Ok(()),
                        _ => Err(tine_storage::sqlite::MaterializationError::Corrupt(
                            format!("unknown Direct Files text kind {kind}"),
                        )),
                    },
                )
            },
            |row| (row.page_id, row.path.clone()),
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
                query_page_order: None, // Filled under the queue lock, before coalescing.
                identity: DeltaIdentity::Live,
            },
        );
    }

    pub(crate) fn enqueue_delete(&self, generation: u64, entry: PageEntry) {
        self.enqueue_delta(generation, PageDelta::Delete { entry });
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
            if !pending.has_work()
                && pending.warm_stream.is_none()
                && !self.shared.worker_busy.load(Ordering::Acquire)
            {
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
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut accumulator = if autocomplete {
            PropertyFacetAccumulator::autocomplete(hidden_properties, max_items, max_bytes)
        } else {
            PropertyFacetAccumulator::query_builder(max_items, max_bytes)
        };
        drain_after(
            |cursor, batch| read.property_facet_rows_after(!autocomplete, cursor, batch),
            |row| (row.owner, row.source_name.clone(), row.ordinal),
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

        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();

        // The page map and the rows are read from the SAME `read`, i.e. the same
        // snapshot: a row naming a page the map does not have is a
        // snapshot-consistency defect and fails the build (§6.2), never a
        // silent fallback to Markdown.
        let mut pages: HashMap<String, PageMeta> = HashMap::new();
        drain_after(
            |cursor: Option<([u8; 16], String)>, batch| {
                read.navigation_pages_after_with_header_validation(
                    cursor.as_ref().map(|(_, path)| path.as_str()),
                    cursor.as_ref().map(|(id, _)| id),
                    batch,
                    |_, kind| match kind {
                        0 | 1 => Ok(()),
                        _ => Err(tine_storage::sqlite::MaterializationError::Corrupt(
                            format!("unknown Direct Files text kind {kind}"),
                        )),
                    },
                )
            },
            |row| (row.page_id, row.path.clone()),
            |row| {
                pages.insert(
                    crate::query::registry_sql::page_key(row.page_id),
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
            |row| (row.owner, row.source_name.clone(), row.ordinal),
            |row| {
                let (owner_type, owner_id) = match row.owner {
                    PhysicalEntityId::Page(id) => (
                        OwnerType::Page,
                        format!("p:{}", crate::query::registry_sql::hex16(id)),
                    ),
                    PhysicalEntityId::Block(id) => (
                        OwnerType::Block,
                        format!("b:{}", crate::query::registry_sql::hex16(id)),
                    ),
                };
                rows.push(OwnerRow {
                    owner_type,
                    owner_id,
                    page_id: crate::query::registry_sql::page_key(row.page_id),
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
                    !pending.rebuild
                        && !pending.needs_full
                        && pending.warm_stream.is_none()
                        && self.shared.validated.load(Ordering::Acquire)
                        && !self.shared.worker_failed.load(Ordering::Acquire)
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
    pub(crate) fn session_pages_test(&self) -> Arc<HashSet<[u8; 16]>> {
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

    pub(crate) fn referenced_page_names(&self, cache_generation: u64) -> Option<Vec<String>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut names = std::collections::HashMap::<String, String>::new();
        drain_after(
            |after: Option<(String, String, String, [u8; 16])>, batch| {
                read.navigation_reference_names_after(
                    after.as_ref().map(|(path, raw, normalized, id)| {
                        (path.as_str(), raw.as_str(), normalized.as_str(), id)
                    }),
                    batch,
                )
            },
            |row| {
                (
                    row.owner_path.clone(),
                    row.raw_name.clone(),
                    row.normalized_name.clone(),
                    row.source_page_id,
                )
            },
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
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut aliases = Vec::new();
        drain_after(
            |after: Option<(String, String, [u8; 16])>, batch| {
                read.navigation_aliases_after(
                    after
                        .as_ref()
                        .map(|(path, alias, id)| (path.as_str(), alias.as_str(), id)),
                    batch,
                )
            },
            |row| {
                (
                    row.owner_path.clone(),
                    row.normalized_alias.clone(),
                    row.source_page_id,
                )
            },
            |row| {
                aliases.push((row.normalized_alias, row.owner_name, row.owner_path));
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        self.ready_at(cache_generation).then_some(aliases)
    }

    pub(crate) fn real_page_names(
        &self,
        cache_generation: u64,
    ) -> Option<crate::query::RealPageNames> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut names = crate::query::RealPageNames::new();
        drain_after(
            |after: Option<(String, [u8; 16])>, batch| {
                read.navigation_pages_after_with_header_validation(
                    after.as_ref().map(|(path, _)| path.as_str()),
                    after.as_ref().map(|(_, id)| id),
                    batch,
                    |_, _| Ok(()),
                )
            },
            |row| (row.path.clone(), row.page_id),
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
        kind: ReferenceKind,
    ) -> Option<ReferenceCandidateIndex> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        if !reference_narrowing_supported(names_norm, kind) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut page_ids = std::collections::BTreeSet::new();
        let mut blocks = std::collections::HashSet::new();
        let mut blocks_are_complete = true;
        for name in names_norm {
            match kind {
                ReferenceKind::Explicit => {
                    drain_after(
                        |after, batch| read.page_referrer_candidates_after(name, after, batch),
                        |row| (row.source_page_id, row.source),
                        |row| {
                            page_ids.insert(row.source_page_id);
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
                    // FTS narrows to pages here; `plain_text_candidate_pages_after`
                    // projects `owner.page_id` and does not expose the owning
                    // entity, so the walk still classifies every block of a
                    // candidate page.
                    blocks_are_complete = false;
                    drain_after(
                        |after, batch| read.plain_text_candidate_pages_after(name, after, batch),
                        |row| row.page_id,
                        |row| {
                            page_ids.insert(row.page_id);
                            Ok(())
                        },
                        |_, _| None,
                    )
                    .ok()?;
                }
            }
        }
        let mut paths = std::collections::BTreeSet::new();
        for page_id in page_ids {
            let page = read
                .page_with_header_validation(page_id, |_, _| Ok(()))
                .ok()??;
            paths.insert(PathBuf::from(page.path));
        }
        self.ready_at(cache_generation)
            .then_some(ReferenceCandidateIndex {
                paths,
                blocks: blocks_are_complete.then_some(blocks),
            })
    }

    /// Outer `None` means projection unavailable/stale and requires parser
    /// fallback. Inner `None` is an exact current-generation miss.
    pub(crate) fn block_page_hint(
        &self,
        cache_generation: u64,
        uuid: &str,
    ) -> Option<Option<String>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let uuid = Uuid::parse_str(uuid).ok()?.into_bytes();
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let block = match read.block(uuid).ok()? {
            Some(block) => crate::query::logseq_uuid_owner([block], false),
            None => {
                crate::query::logseq_uuid_owner(read.blocks_by_logseq_uuid(uuid, 2).ok()?, false)
            }
        };
        let page = match block {
            Some(block) => read
                .page_with_header_validation(block.page_id, |_, _| Ok(()))
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
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
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
        if !self.ready_at(cache_generation) {
            return None;
        }
        let uuid = Uuid::parse_str(uuid).ok()?.into_bytes();
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut page_ids = std::collections::BTreeSet::new();
        drain_after(
            |after, batch| read.block_referrer_candidates_after(uuid, after, batch),
            |row| (row.source_page_id, row.source_block_id),
            |row| {
                page_ids.insert(row.source_page_id);
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        let mut paths = std::collections::BTreeSet::new();
        for page_id in page_ids {
            let page = read
                .page_with_header_validation(page_id, |_, _| Ok(()))
                .ok()??;
            paths.insert(PathBuf::from(page.path));
        }
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
        if pending.rebuild || pending.needs_full || pending.full.is_some() {
            return ProjectionProgress::Working(Reason::Recovering);
        }
        if self.shared.repairs_in_flight.load(Ordering::Acquire) > 0 {
            return ProjectionProgress::Working(Reason::Recovering);
        }
        if pending.warm.is_some() || pending.warm_stream.is_some() || pending.order.is_some() {
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

    /// Test diagnostic: the queue and readiness state in one line, for a
    /// convergence failure that would otherwise be a bare timeout.
    pub(crate) fn debug_state_test(&self) -> String {
        let pending = self.shared.pending.lock().unwrap();
        let state = format!(
            "ready={} validated={} ready_generation={} latest_generation={} full={} deltas={} warm={} warm_outcome={:?} warm_stream={:?} order={:?} superseded={} needs_full={} rebuild={} stop={} page_order={} worker_available={} worker_failed={} worker_busy={}",
            self.shared.ready.load(Ordering::Acquire),
            self.shared.validated.load(Ordering::Acquire),
            self.shared.ready_generation.load(Ordering::Acquire),
            pending.latest_generation,
            pending.full.is_some(),
            pending.deltas.len(),
            pending.warm.is_some(),
            pending.warm_outcome.as_ref().map(|outcome| match outcome {
                WarmOutcome::Clean => "Clean".to_owned(),
                WarmOutcome::Replacements(pages) => format!("Replacements({})", pages.len()),
                WarmOutcome::Superseded => "Superseded".to_owned(),
                WarmOutcome::Failed => "Failed".to_owned(),
            }),
            pending.warm_stream,
            pending.order,
            pending.warm_superseded,
            pending.needs_full,
            pending.rebuild,
            pending.stop,
            pending.page_order.len(),
            self.shared.worker_available.load(Ordering::Acquire),
            self.shared.worker_failed.load(Ordering::Acquire),
            self.shared.worker_busy.load(Ordering::Acquire),
        );
        format!(
            "{state} repairs_in_flight={}",
            self.shared.repairs_in_flight.load(Ordering::Acquire)
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
    pub(crate) fn fuzzy_candidate_reads(&self) -> u64 {
        self.shared.fuzzy_candidate_reads.load(Ordering::Relaxed)
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
/// projection WRITE path, and that path names the graph — `apply_pending`
/// formats `entry.rel_path` straight into its error string, and
/// `MaterializationError`'s payloads are free-form `String`s produced while
/// storing parsed page text. I-9: the family still reaches the always-on
/// record, because a user who is not running under `TINE_DEBUG` otherwise sees
/// only an unavailable index. The prose stays on the directed debug channel.
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
/// channel. It is the ordinary cold-open handoff: an edit or an external write
/// (Syncthing, an external editor) raced the warm stream, `stream_warm_replacements`
/// abandoned it, `abandon_warm_stream` set `needs_full`, and the next turn
/// carried only deltas. The parser fallback is ALREADY on its way with the full
/// snapshot that repairs this; nothing is wrong and nothing is owed by the user.
/// Reporting it printed `PROJECTION_UPDATE_FAILURE` once per delta turn, so a
/// cold open under sync traffic emitted the alarming line hundreds of times
/// (Martin, 2026-09-10) while queries answered correctly throughout.
enum ProjectionRefusal {
    AwaitingFullInventory,
    Failed(String),
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
        }
    }
}

/// Lives for one repair attempt; see `DirectProjection::begin_repair`.
pub(crate) struct RepairInFlight(Arc<ProjectionShared>);

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
    let mut writer_slot = match open_projection_database(&shared.path) {
        Ok(database) => Some(database),
        Err(error) => {
            report_projection_failure("disabled: its database could not be opened", &error);
            shared.worker_available.store(false, Ordering::Release);
            shared.changed.notify_all();
            return;
        }
    };
    // The lock file is app-private disposable state. Retain its exclusive lock
    // for the complete writer lifetime so another Graph instance cannot replace
    // this database's facts behind a locally-ready generation watermark.
    let _lease = lease;
    let mut requires_full_rebuild = false;
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
            if std::mem::take(&mut pending.needs_full) {
                requires_full_rebuild = true;
            }
            let rebuild = (pending.full.is_some() || pending.warm.is_some())
                && std::mem::take(&mut pending.rebuild);
            // R6: a full snapshot queued beside a warm validation owns
            // readiness; the warm is dropped as superseded.
            let warm = if pending.full.is_some() {
                if pending.warm.take().is_some() {
                    pending.warm_stream = None;
                    pending.warm_superseded = true;
                    pending.warm_outcome = Some(WarmOutcome::Superseded);
                }
                None
            } else {
                pending.warm.take()
            };
            let order = pending.order.take();
            let deltas = std::mem::take(&mut pending.deltas);
            let unordered = deltas.values().any(|(_, delta)| {
                matches!(
                    delta,
                    PageDelta::Replace {
                        query_page_order: None,
                        ..
                    }
                )
            });
            let inventory = (order.is_some() || warm.is_some() || unordered)
                .then(|| pending.ordered_inventory());
            WorkerTurn {
                full: pending.full.take(),
                warm,
                deltas,
                order,
                inventory,
                stream_open: pending.warm_stream.is_some(),
                latest_generation: pending.latest_generation,
                rebuild,
            }
        };
        let WorkerTurn {
            full,
            warm,
            deltas,
            order,
            inventory,
            stream_open,
            latest_generation,
            rebuild,
        } = turn;
        let had_full = full.is_some();
        let had_warm = warm.is_some();
        let stream_closed = order.is_some();
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
        let registry_reset = had_full || had_warm || rebuild || requires_full_rebuild;
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
            .map(|(_, delta)| page_id(&delta.entry().rel_path))
            .collect::<std::collections::BTreeSet<_>>();
        #[cfg(test)]
        run_before_apply_pending_hook();
        let applied: Result<AppliedTurn, ProjectionRefusal> = if requires_full_rebuild
            && !had_full
            && !had_warm
        {
            // NOT necessarily a prior failure: `abandon_warm_stream` sets
            // `needs_full` on ordinary generation drift. See `ProjectionRefusal`.
            Err(ProjectionRefusal::AwaitingFullInventory)
        } else {
            (|| {
                if config_changed && !(rebuild || requires_full_rebuild || writer_slot.is_none()) {
                    let fence = shared.cancel_queued_captures(false);
                    shared.query_jobs.wait_for_drain(fence);
                }
                if rebuild || requires_full_rebuild || writer_slot.is_none() {
                    // R3: interrupt and drain every query job first, so no
                    // owned snapshot retains a handle to the file about to be
                    // reset or removed, and the rebuild never waits on a read
                    // nobody will finish. In-scope scenario: a torn projection
                    // rebuilt under a live reader (D-3).
                    let fence = shared.cancel_queued_captures(false);
                    shared.query_jobs.wait_for_drain(fence);
                    // Drop every connection before the disposable file can be
                    // replaced; a reader must not retain an old file handle.
                    let mut reader = shared.reader.lock().unwrap();
                    reader.take();
                    writer_slot.take();
                    let mut database = open_projection_database(&shared.path)
                        .map_err(|error| error.to_string())?;
                    // Even repaired DDL leaves unchanged source stamps behind.
                    // Reset them so the complete inventory relowers every source page.
                    database.reset().map_err(|error| error.to_string())?;
                    writer_slot = Some(database);
                }
                let registry_before = if !registry_reset
                    && !touched_pages.is_empty()
                    && shared.committed_registry.lock().unwrap().is_some()
                {
                    let mut snapshot =
                        PhysicalProjectionQuerySnapshot::open_direct(&shared.path, || Ok(()))
                            .map_err(|error| error.to_string())?;
                    registry_sql::read_page_registry_metadata(&mut snapshot, &touched_pages)
                        .map_err(|error| error.to_string())?
                } else {
                    PageRegistryMetadata::new()
                };
                let diagnostic_turn = std::time::Instant::now();
                if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
                    eprintln!("DIAG543 WORKER begin full={had_full} warm={had_warm} deltas={} rebuild={rebuild}", deltas.len());
                }
                let mut applied =
                    apply_pending(writer_slot.as_mut().unwrap(), full, warm.as_ref(), deltas)?;
                if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
                    eprintln!("DIAG543 WORKER applied {:?}", diagnostic_turn.elapsed());
                }
                // R6: the stream's closing turn (or a `Clean` warm turn, or a
                // turn that lowered mid-stream deltas without positions)
                // reconciles the order table over the queue's inventory. The
                // queue's map tracks every applied replacement and deletion
                // since its seed, so it names exactly the projected pages.
                let warm_clean = matches!(applied.warm_outcome, Some(WarmOutcome::Clean));
                applied.stream_open = if had_warm {
                    matches!(applied.warm_outcome, Some(WarmOutcome::Replacements(_)))
                } else {
                    stream_open && !stream_closed
                };
                if !applied.stream_open
                    && (stream_closed || warm_clean || applied.unordered_replacements)
                {
                    let inventory = inventory.ok_or_else(|| {
                        "the order turn ran without its queue inventory".to_owned()
                    })?;
                    writer_slot
                        .as_mut()
                        .unwrap()
                        .apply_with_source_revisions_aliases_and_page_order(
                            &PhysicalGraphProjectionChange {
                                replacements: Vec::new(),
                                deletions: Vec::new(),
                                reference_postings: Vec::new(),
                            },
                            &[],
                            &[],
                            &inventory,
                        )
                        .map_err(|error| error.to_string())?;
                }
                // All SQL writes, including the separate ordering transaction,
                // have completed. No maintenance transaction survives a write.
                let revision = {
                    let mut snapshot =
                        PhysicalProjectionQuerySnapshot::open_direct(&shared.path, || Ok(()))
                            .map_err(|error| error.to_string())?;
                    snapshot
                        .query_revision()
                        .map_err(|error| error.to_string())?
                };
                #[cfg(test)]
                if let Some(hook) = shared.after_sql_commit.lock().unwrap().take() {
                    hook();
                }
                shared.record_session_pages(&applied.pages);
                let changes =
                    registry_sql::registry_changes(&registry_before, &applied.registry_pages);
                let mut registry = shared.committed_registry.lock().unwrap();
                let config = registry_config
                    .as_ref()
                    .cloned()
                    .or_else(|| registry.as_ref().map(|owner| Arc::clone(&owner.config)));
                if let Some(config) = config {
                    match registry.as_mut() {
                        Some(owner)
                            if !registry_reset && owner.config.digest() == config.digest() =>
                        {
                            owner
                                .cache
                                .committed(
                                    revision,
                                    changes.normalized_keys,
                                    changes.declaration_page_names,
                                )
                                .map_err(|error| error.to_string())?;
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
                drop(registry);
                Ok(applied)
            })()
            .map_err(ProjectionRefusal::Failed)
        };
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                shared.committed_registry.lock().unwrap().take();
                requires_full_rebuild = true;
                shared.ready.store(false, Ordering::Release);
                shared.worker_failed.store(true, Ordering::Release);
                {
                    let mut pending = shared.pending.lock().unwrap();
                    if had_warm {
                        pending.warm_outcome = Some(WarmOutcome::Failed);
                    }
                    if had_warm || stream_closed {
                        pending.warm_stream = None;
                        pending.order = None;
                    }
                }
                shared.worker_busy.store(false, Ordering::Release);
                shared.changed.notify_all();
                if error.is_reportable_failure() {
                    report_projection_failure(PROJECTION_UPDATE_FAILURE, &error);
                } else if crate::backend_error::runtime_debug_diagnostics_enabled() {
                    eprintln!("[tine] Direct Files SQLite projection deferred this turn: {error}");
                }
                continue;
            }
        };
        if had_full || had_warm {
            requires_full_rebuild = false;
        }
        if had_full || stream_closed || matches!(applied.warm_outcome, Some(WarmOutcome::Clean)) {
            shared.validated.store(true, Ordering::Release);
        }
        shared.worker_failed.store(false, Ordering::Release);
        let mut pending = shared.pending.lock().unwrap();
        shared.worker_busy.store(false, Ordering::Release);
        if had_warm {
            // A `Replacements` outcome keeps the stream open at its generation
            // and readiness waits for the closing order turn; any other
            // outcome closes the stream this warm opened.
            if !applied.stream_open {
                pending.warm_stream = None;
            }
            if pending.warm_outcome.is_none() && !pending.warm_superseded {
                pending.warm_outcome = applied.warm_outcome.clone();
            }
        }
        if stream_closed {
            pending.warm_stream = None;
        }
        if !pending.rebuild
            && !pending.has_work()
            && pending.warm_stream.is_none()
            && pending.latest_generation == latest_generation
            && shared.validated.load(Ordering::Acquire)
        {
            shared
                .ready_generation
                .store(latest_generation, Ordering::Release);
            shared.ready.store(true, Ordering::Release);
        }
        drop(pending);
        shared.changed.notify_all();
        // Source-change events may have preceded this commit. Wake the existing
        // application watcher after every serving-image publication, without
        // retaining a query or requiring any edit to be covered by its read.
        if shared.validated.load(Ordering::Acquire) && !applied.stream_open {
            shared.commit_notification.fetch_add(1, Ordering::Release);
            if let Some(wake) = shared.commit_waker.lock().unwrap().as_ref() {
                let _ = wake.send(());
            }
        }
    }
}

/// One worker turn's queued work (R6 widened it beyond full + deltas).
struct WorkerTurn {
    full: Option<PendingFull>,
    warm: Option<PendingWarm>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    order: Option<u64>,
    /// The queue's inventory captured with the deltas, so the order turn
    /// reconciles exactly the pages this turn leaves projected.
    inventory: Option<Vec<[u8; 16]>>,
    /// Whether a warm stream was open when the turn was taken.
    stream_open: bool,
    latest_generation: u64,
    rebuild: bool,
}

fn open_projection_database(
    path: &Path,
) -> Result<PhysicalGraphProjectionDatabase, tine_storage::sqlite::MaterializationError> {
    let database = PhysicalGraphProjectionDatabase::open_writable(path)?;
    if database.validate_schema().is_ok() && database.quick_check().is_ok() {
        return Ok(database);
    }
    if database.initialize_schema().is_ok()
        && database.validate_schema().is_ok()
        && database.quick_check().is_ok()
    {
        return Ok(database);
    }
    drop(database);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let database = PhysicalGraphProjectionDatabase::open_writable(path)?;
    database.initialize_schema()?;
    database.validate_schema()?;
    Ok(database)
}

/// Which pages one worker turn actually WROTE (R3 identity policy): the pages
/// whose rows now carry this process's live runtime ids, and the pages whose
/// rows are gone. A full snapshot on a warm reopen reuses every unchanged
/// page's rows, so "a full snapshot was applied" is not "every page was
/// lowered" — only the source delta's replacements were.
#[derive(Default)]
struct AppliedPages {
    lowered: Vec<[u8; 16]>,
    deleted: Vec<[u8; 16]>,
    /// R6: pages relowered from a fresh parse; they leave the session set.
    relowered_structurally: Vec<[u8; 16]>,
}

#[derive(Default)]
struct AppliedTurn {
    pages: AppliedPages,
    registry_pages: PageRegistryMetadata,
    /// R6: the warm validation's verdict, when this turn ran one.
    warm_outcome: Option<WarmOutcome>,
    /// R6: this turn lowered replacements that carried no order position
    /// (queued while a stream was open), so the order table must be
    /// reconciled once the stream is closed.
    unordered_replacements: bool,
    stream_open: bool,
}

/// R6 warm validation inside one worker turn: compare the walk inventory's
/// exact revisions with `direct_source_revisions`, delete what the walk no
/// longer has, and name what must be relowered. Nothing here parses.
fn validate_warm(
    database: &mut PhysicalGraphProjectionDatabase,
    warm: &PendingWarm,
    applied: &mut AppliedPages,
) -> Result<WarmOutcome, String> {
    let config_digest = warm.parse_config.digest();
    let sources = warm
        .sources
        .iter()
        .map(|(entry, revision)| PhysicalGraphProjectionSourceRevision {
            page_id: page_id(&entry.rel_path),
            revision: projection_source_revision(revision, config_digest),
        })
        .collect::<Vec<_>>();
    let source_delta = database
        .source_delta(&sources)
        .map_err(|error| error.to_string())?;
    if !source_delta.deletions.is_empty() {
        applied
            .deleted
            .extend(source_delta.deletions.iter().copied());
        database
            .apply_with_source_revisions_and_aliases(
                &PhysicalGraphProjectionChange {
                    replacements: Vec::new(),
                    deletions: source_delta.deletions,
                    reference_postings: Vec::new(),
                },
                &[],
                &[],
            )
            .map_err(|error| error.to_string())?;
    }
    if source_delta.replacements.is_empty() {
        return Ok(WarmOutcome::Clean);
    }
    let needed = source_delta
        .replacements
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    Ok(WarmOutcome::Replacements(
        warm.sources
            .iter()
            .filter(|(entry, _)| needed.contains(&page_id(&entry.rel_path)))
            .map(|(entry, _)| entry.clone())
            .collect(),
    ))
}

fn apply_pending(
    database: &mut PhysicalGraphProjectionDatabase,
    full: Option<PendingFull>,
    warm: Option<&PendingWarm>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<AppliedTurn, String> {
    let diagnostic_start = std::time::Instant::now();
    let mut turn = AppliedTurn::default();
    let applied = &mut turn.pages;
    if let Some(PendingFull {
        pages,
        revisions,
        parse_config,
    }) = full
    {
        let parse_config = parse_config.as_ref();
        let config_digest = parse_config.digest();
        let sources = pages
            .iter()
            .map(|(entry, _)| {
                Ok(PhysicalGraphProjectionSourceRevision {
                    page_id: page_id(&entry.rel_path),
                    revision: projection_source_revision(
                        revisions.get(&entry.path).ok_or_else(|| {
                            format!(
                                "parsed page has no exact source revision: {}",
                                entry.rel_path
                            )
                        })?,
                        config_digest,
                    ),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let source_delta = database
            .source_delta(&sources)
            .map_err(|error| error.to_string())?;
        let replacements_needed = source_delta
            .replacements
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let inventory = sources
            .iter()
            .map(|source| source.page_id)
            .collect::<Vec<_>>();
        let lowered = pages
            .iter()
            .enumerate()
            .filter(|(_, (entry, _))| replacements_needed.contains(&page_id(&entry.rel_path)))
            .map(|(position, (entry, document))| {
                let (mut page, postings, aliases) = physical_page(entry, document, parse_config)?;
                page.query_page_order = Some(position as u64);
                Ok::<_, String>((page, postings, aliases))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut replacements = Vec::with_capacity(lowered.len());
        let mut reference_postings = Vec::new();
        let mut aliases = Vec::new();
        for (page, mut postings, mut page_aliases) in lowered {
            replacements.push(page);
            reference_postings.append(&mut postings);
            aliases.append(&mut page_aliases);
        }
        let replacement_sources = sources
            .into_iter()
            .filter(|source| replacements_needed.contains(&source.page_id))
            .collect::<Vec<_>>();
        applied.lowered.extend(replacements_needed.iter().copied());
        applied
            .deleted
            .extend(source_delta.deletions.iter().copied());
        database
            .apply_with_source_revisions_aliases_and_page_order(
                &PhysicalGraphProjectionChange {
                    replacements,
                    deletions: source_delta.deletions,
                    reference_postings,
                },
                &replacement_sources,
                &aliases,
                &inventory,
            )
            .map_err(|error| error.to_string())?;
    }
    if let Some(warm) = warm {
        turn.warm_outcome = Some(validate_warm(database, warm, applied)?);
    }
    if !deltas.is_empty() {
        let diagnostic_count = deltas.len();
        let mut replacements = Vec::new();
        let mut reference_postings = Vec::new();
        let mut aliases = Vec::new();
        let mut replacement_sources = Vec::new();
        let mut deletions = Vec::new();
        for (_, (_, delta)) in deltas {
            match delta {
                // Each replacement lowers under the config it was queued with,
                // never under a later page's or a default (F11).
                PageDelta::Replace {
                    entry,
                    document,
                    revision,
                    parse_config,
                    query_page_order,
                    identity,
                } => {
                    replacement_sources.push(PhysicalGraphProjectionSourceRevision {
                        page_id: page_id(&entry.rel_path),
                        revision: projection_source_revision(&revision, parse_config.digest()),
                    });
                    let (mut page, mut postings, mut page_aliases) =
                        physical_page(&entry, &document, &parse_config)?;
                    page.query_page_order = query_page_order;
                    if query_page_order.is_none() {
                        turn.unordered_replacements = true;
                    }
                    match identity {
                        DeltaIdentity::Live => applied.lowered.push(page.page_id),
                        DeltaIdentity::Structural => {
                            applied.relowered_structurally.push(page.page_id)
                        }
                    }
                    turn.registry_pages.insert(
                        page.page_id,
                        registry_sql::registry_metadata_from_physical_page(&page)?,
                    );
                    replacements.push(page);
                    reference_postings.append(&mut postings);
                    aliases.append(&mut page_aliases);
                }
                PageDelta::Delete { entry } => {
                    let id = page_id(&entry.rel_path);
                    applied.deleted.push(id);
                    deletions.push(id);
                }
            }
        }
        if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
            eprintln!(
                "DIAG543 STAGE deltas={diagnostic_count} lower={:?}",
                diagnostic_start.elapsed()
            );
        }
        let diagnostic_sql = std::time::Instant::now();
        database
            .apply_with_source_revisions_and_aliases(
                &PhysicalGraphProjectionChange {
                    replacements,
                    deletions,
                    reference_postings,
                },
                &replacement_sources,
                &aliases,
            )
            .map_err(|error| error.to_string())?;
        if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
            eprintln!(
                "DIAG543 STAGE deltas={diagnostic_count} sql={:?} total={:?}",
                diagnostic_sql.elapsed(),
                diagnostic_start.elapsed()
            );
        }
    }
    Ok(turn)
}

/// The revision Direct Files compares to decide whether a page's rows are still
/// current. Folding the parse-config digest in is what makes a config edit a
/// full re-lowering (§5.8 J7): reconciliation compares only source revisions,
/// so without it an unchanged file would keep rows built under the old config.
fn projection_source_revision(
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
    let id = page_id(&entry.rel_path);
    let format = Format::from_path(Path::new(&entry.rel_path));
    let is_org = format == Format::Org;
    // `Format::from_path` and never `reference_source_is_org`: the latter is a
    // case-sensitive `ends_with(".org")` and would type an `Outline.ORG` page
    // Markdown here while Direct Files types it Org (§5.8 E4).
    let atom_format = crate::query::atom::AtomFormat::from(format);
    let (preamble_search, properties, tags) = document
        .pre_block
        .as_deref()
        .map(|raw| facets(raw, is_org))
        .unwrap_or_default();
    let searchable_text = if preamble_search.is_empty() {
        entry.name.clone()
    } else {
        format!("{} {preamble_search}", entry.name)
    };
    let mut blocks = Vec::new();
    let mut reference_postings = Vec::new();
    let aliases = crate::query::document_aliases(document)
        .into_iter()
        .enumerate()
        .map(|(ordinal, alias)| {
            Ok(PhysicalAliasDeclaration {
                source_page_id: id,
                source_entity: PhysicalEntityId::Page(id),
                source_locator: b"page-alias".to_vec(),
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| "one page exceeds u32::MAX aliases".to_string())?,
                raw_alias: alias.clone(),
                normalized_alias: alias,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(preamble) = document.pre_block.as_deref() {
        append_reference_postings(
            &mut reference_postings,
            id,
            PhysicalEntityId::Page(id),
            b"preamble",
            std::iter::empty(),
            crate::doc::property_reference_page_names(preamble).into_iter(),
        )?;
    }
    let mut block_refs_norm: Vec<Vec<String>> = Vec::new();
    lower_blocks(
        &document.roots,
        id,
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
    let flat = blocks
        .iter()
        .zip(block_refs_norm.iter())
        .map(|(block, refs)| crate::query::path_refs::PathRefBlock {
            id: block.block_id,
            parent: block.parent,
            refs: refs.as_slice(),
        })
        .collect::<Vec<_>>();
    let mut path_refs = crate::query::derived::path_ref_rows(&entry.name, &flat);
    for block in &mut blocks {
        block.path_refs = path_refs.remove(&block.block_id).unwrap_or_default();
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
            page_id: id,
            query_page_order: None,
            home_document_id: id,
            name: entry.name.clone(),
            name_key: crate::refs::page_key(&entry.name),
            path: entry.rel_path.clone(),
            text_kind: page_kind_to_sql(entry.kind),
            journal_day: journal_days.day(&entry.rel_path, entry.kind == PageKind::Journal),
            preamble: document.pre_block.clone(),
            normalized_searchable_text: searchable_text.to_lowercase().nfc().collect(),
            searchable_text,
            references: Vec::new(),
            properties,
            tags: crate::query::derived::tag_rows(&tags),
            property_atoms: page_property_atoms,
            blocks,
        },
        reference_postings,
        aliases,
    ))
}

/// The 16-byte key a block's rows are stored under.
///
/// A parsed page carries structural UUIDs. A page saved from the editor keeps
/// the FRONTEND's live ids (`src/store.ts` `freshId()`: `b<base36 time>-<n>`),
/// which `cache_upsert_inner` deliberately preserves so the editor can keep
/// addressing the block; the public identity of a row is `query_result_id`,
/// the id STRING, so a non-UUID live id only needs a deterministic key here.
/// This used to be a refusal, and a refusal scoped to the whole page: one
/// block created in the editor failed the page's delta, the failed turn
/// latched a full rebuild, and every rebuild re-lowered the same live document
/// and failed the same way — so after one such save every query in the app
/// answered "Rebuilding the query index…" until restart (2026-09-11,
/// master d61cfb3d). `a_block_created_in_the_editor_keeps_queries_answering`
/// (`tests/search_edit.rs`) pins the user outcome.
fn block_projection_key(runtime_id: &str) -> [u8; 16] {
    match Uuid::parse_str(runtime_id) {
        Ok(uuid) => uuid.into_bytes(),
        Err(_) => crate::vocab::live_runtime_id_key(runtime_id).into_bytes(),
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_blocks(
    source: &[DocBlock],
    page_id: [u8; 16],
    parent: Option<[u8; 16]>,
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
        let block_id = block_projection_key(&block.uuid);
        let projection = block.projection();
        let order = structural_path
            .iter()
            .map(|part| format!("{part:08x}"))
            .collect::<Vec<_>>()
            .join("/");
        append_reference_postings(
            reference_postings,
            page_id,
            PhysicalEntityId::Block(block_id),
            order.as_bytes(),
            projection.refs_page.iter().cloned(),
            crate::doc::property_reference_page_names(&block.raw).into_iter(),
        )?;
        for raw_claim in &projection.block_refs {
            let Ok(raw_claim) = Uuid::parse_str(raw_claim) else {
                continue;
            };
            reference_postings.push(PhysicalReferencePosting {
                source_page_id: page_id,
                source_entity: PhysicalEntityId::Block(block_id),
                source_locator: order.as_bytes().to_vec(),
                ordinal: u32::try_from(reference_postings.len())
                    .map_err(|_| "one page exceeds u32::MAX reference postings".to_string())?,
                kind: 6,
                target: PhysicalReferenceTarget::ExternalUuid {
                    raw_claim: raw_claim.into_bytes(),
                    resolved_block_id: None,
                },
            });
        }
        let searchable_text = projection
            .visible
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // The query columns are the EXACT visible text and its fold, never the
        // whitespace-collapsed `searchable_text` beside them (§5.10).
        // `visible_lower` is exactly `search_query::canonical_fold(visible)`.
        let (query_visible, query_visible_folded) =
            (projection.visible.clone(), projection.visible_lower.clone());
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
            block_id,
            query_result_id: block.uuid.clone(),
            own_refs: projection.refs_norm.clone(),
            home_document_id: page_id,
            parent,
            order,
            content: block.raw.clone(),
            normalized_searchable_text: searchable_text.to_lowercase().nfc().collect(),
            searchable_text,
            query_visible,
            query_visible_folded,
            heading_level: projection.heading_level,
            collapsed: block.collapsed(),
            logseq_uuid,
            logseq_identity_origin: logseq_uuid.map(|_| 0),
            references: Vec::new(),
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
            page_id,
            Some(block_id),
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
    page_id: [u8; 16],
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
                source_page_id: page_id,
                source_entity: source,
                source_locator: source_locator.to_vec(),
                ordinal,
                kind,
                target: PhysicalReferenceTarget::PageName {
                    normalized_name: crate::refs::page_key(&raw_name),
                    raw_name,
                    resolved_page_id: None,
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
    let searchable = block
        .visible_text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
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

pub(crate) fn page_id(relative_path: &str) -> [u8; 16] {
    let mut digest = Sha256::new();
    digest.update(b"tine-direct-page-v1\0");
    digest.update(relative_path.as_bytes());
    let bytes = digest.finalize();
    let mut id = [0; 16];
    id.copy_from_slice(&bytes[..16]);
    id
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
/// Drive projection recovery the way the app does: by retrying.
///
/// `direct_projection_recover_after_failed_read` is ONE attempt and is allowed
/// to accomplish nothing -- `model.rs` says so itself where it turns "the
/// repair did not take" into `Unavailable(ReadFailed)`. The mechanism is that
/// recovery latches `pending.rebuild`, but the worker consumes that flag only
/// together with a `full` or `warm` payload (`rebuild = (full|warm) &&
/// take(rebuild)`). If the turn carrying that payload fails, the payload is
/// gone and the rebuild stays latched with nothing left to ride in on, so the
/// projection stays failed until something enqueues work again. In the running
/// app that something is the user's next query, which calls recovery again.
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

#[cfg(test)]
#[path = "direct_projection_tests.rs"]
mod tests;
