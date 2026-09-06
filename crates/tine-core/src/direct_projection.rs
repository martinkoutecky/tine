use crate::config::ParseConfig;
use crate::doc::{property_key_norm, DocBlock, Document};
use crate::model::{Format, PageEntry, PageKind, ReferenceKind};
use crate::oplog::query_lowering::drain_after;
use crate::query::sql::QueryRegexProgram;
use crate::query::{
    run_parser_sparse_task_query_bounded, sparse_task_query_eligibility,
    ApplicationSparseQueryPage, BoundedGroups, ParserSparseQueryCandidate,
    PropertyFacetAccumulator, SimpleQueryCandidatePlan,
};
use fs2::FileExt as _;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tine_storage::sqlite::{
    PhysicalAliasDeclaration, PhysicalBlock, PhysicalEntityId, PhysicalGraphProjectionChange,
    PhysicalGraphProjectionDatabase, PhysicalGraphProjectionSourceRevision, PhysicalPage,
    PhysicalProjectionQueryReader, PhysicalProperty, PhysicalQueryValue, PhysicalReferencePosting,
    PhysicalReferenceTarget, PhysicalTask,
};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

type PageSnapshot = Arc<Vec<(PageEntry, Arc<Document>)>>;
type PageRevisions = Arc<HashMap<PathBuf, String>>;

// This is the parser-fact extractor identity, not an on-disk schema version.
// Bump it whenever unchanged source bytes must be lowered into new/different
// physical facts. The source-revision delta then rebuilds each page once even
// when tine-storage's disposable SQLite schema itself remains compatible.
const DIRECT_PROJECTION_FACTS_VERSION: u32 = 2;
const REFERENCE_DELTA_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(test)]
static PHYSICAL_PAGE_LOWERINGS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static BEFORE_APPLY_PENDING: Mutex<Option<Box<dyn FnOnce() + Send>>> = Mutex::new(None);

#[cfg(test)]
fn run_before_apply_pending_hook() {
    if let Some(hook) = BEFORE_APPLY_PENDING.lock().unwrap().take() {
        hook();
    }
}

/// The registry's snapshot-scoped page identity on the Direct Files projection
/// side. The Managed side uses `page:<uuid>` and the cold walk the page's
/// relative path; all three are opaque to `build_registry`, which only ever
/// looks a row's page up in the map that came with it.
fn direct_registry_page_key(page_id: [u8; 16]) -> String {
    format!("page:{}", hex16(page_id))
}

fn hex16(id: [u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in id {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
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
        query_page_order: u64,
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
}

#[derive(Default)]
struct PendingProjection {
    full: Option<PendingFull>,
    rebuild: bool,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    latest_generation: u64,
    stop: bool,
    page_order: BTreeMap<String, u64>,
    next_page_order: u64,
}

impl PendingProjection {
    fn record_delta(&mut self, generation: u64, mut delta: PageDelta) {
        let key = delta.entry().rel_path.clone();
        match &mut delta {
            PageDelta::Replace {
                query_page_order, ..
            } => {
                *query_page_order = if let Some(position) = self.page_order.get(&key) {
                    *position
                } else {
                    let position = self.next_page_order;
                    self.next_page_order += 1;
                    self.page_order.insert(key.clone(), position);
                    position
                };
            }
            PageDelta::Delete { .. } => {
                self.page_order.remove(&key);
            }
        }
        self.deltas.insert(key, (generation, delta));
        self.latest_generation = self.latest_generation.max(generation);
    }
}

struct ProjectionShared {
    path: PathBuf,
    pending: Mutex<PendingProjection>,
    changed: Condvar,
    ready: AtomicBool,
    ready_generation: AtomicU64,
    reader: Mutex<Option<PhysicalGraphProjectionDatabase>>,
    /// The D-15 statement seam, opened lazily beside the typed reader above.
    ///
    /// Named `statement_seam` rather than `…_reader` on purpose: the field above
    /// holds the WRITE-CAPABLE `PhysicalGraphProjectionDatabase` under the name
    /// `reader`, and the tine-storage boundary census attributes a method call
    /// to the receiver NAME by substring. A `query_reader` here would file every
    /// read-only statement under the writable handle in that inventory, which is
    /// exactly the distinction D-15 rests on.
    ///
    /// It is a SECOND read-only connection because that is what the seam is: a
    /// separate, read-only handle over the disposable projection, with no way to
    /// reach a writable one (D-15's enforcement is the handle, not a validator).
    /// It answers §5.9's dispatched query and nothing else.
    statement_seam: Mutex<Option<PhysicalProjectionQueryReader>>,
    /// The generation at which §5.10's FTS-building signal was last observed
    /// READY. Readiness is monotonic within one projection file — the index
    /// owner finishes the build and never un-finishes it, and a rebuild
    /// publishes a new generation — so a `true` may be remembered and a `false`
    /// never is. That keeps the signal one probe per generation instead of one
    /// per query (I-15) without ever stranding a query on a stale `false`.
    fts_ready_at: AtomicU64,
    fts_ever_ready: AtomicBool,
    worker_available: AtomicBool,
    worker_failed: AtomicBool,
    worker_busy: AtomicBool,
    #[cfg(test)]
    indexed_reads: AtomicU64,
    /// §5.9's dispatched statements: how many times the lowering ANSWERED a
    /// user query through the seam. Separate from `indexed_reads`, which counts
    /// every seam read including the FTS-readiness probe, so a route guard can
    /// say "exactly one statement per query" and mean it.
    #[cfg(test)]
    statement_reads: AtomicU64,
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

/// What one attempt to answer through the D-15 statement seam produced
/// (SPEC §5.9). See [`DirectProjection::run_statement`].
pub(crate) enum StatementRead {
    Rows(Vec<Vec<PhysicalQueryValue>>),
    NotReady,
    Failed,
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
    pub(crate) fn start(path: PathBuf) -> std::io::Result<Self> {
        let shared = Arc::new(ProjectionShared {
            path,
            pending: Mutex::new(PendingProjection::default()),
            changed: Condvar::new(),
            ready: AtomicBool::new(false),
            ready_generation: AtomicU64::new(0),
            reader: Mutex::new(None),
            statement_seam: Mutex::new(None),
            fts_ready_at: AtomicU64::new(0),
            fts_ever_ready: AtomicBool::new(false),
            worker_available: AtomicBool::new(true),
            worker_failed: AtomicBool::new(false),
            worker_busy: AtomicBool::new(false),
            #[cfg(test)]
            indexed_reads: AtomicU64::new(0),
            #[cfg(test)]
            statement_reads: AtomicU64::new(0),
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

    /// Keep the repair request until a complete parser snapshot is available.
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
        pending.page_order = pages
            .iter()
            .enumerate()
            .map(|(position, (entry, _))| (entry.rel_path.clone(), position as u64))
            .collect();
        pending.next_page_order = pages.len() as u64;
        pending.full = Some(PendingFull {
            pages,
            revisions,
            parse_config,
        });
        pending.deltas.clear();
        pending.latest_generation = generation;
        self.shared.changed.notify_one();
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
                query_page_order: 0, // Filled under the queue lock, before coalescing.
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
            if pending.full.is_none()
                && pending.deltas.is_empty()
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sparse_task_query(
        &self,
        graph_root: &Path,
        journal_format: &crate::date::JournalFormat,
        cache_generation: u64,
        pages: &[(PageEntry, Arc<Document>)],
        query_src: &str,
        max_rows: usize,
        max_bytes: usize,
        config: &crate::config::ParseConfig,
        registry: &crate::query::registry::Registry,
    ) -> Option<BoundedGroups> {
        let eligibility = sparse_task_query_eligibility(query_src)?;
        if !self.shared.ready.load(Ordering::Acquire)
            || self.shared.ready_generation.load(Ordering::Acquire) != cache_generation
        {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut by_block = BTreeMap::new();
        let uses_recency = eligibility.uses_recency;
        for marker in eligibility.markers {
            drain_after(
                |after, batch| read.task_candidate_locators_after(&marker, after, batch),
                |row| (row.page_id, row.block_id),
                |row| {
                    by_block.entry(row.block_id).or_insert(row);
                    Ok(())
                },
                |_, _| None,
            )
            .ok()?;
        }
        if self.shared.ready_generation.load(Ordering::Acquire) != cache_generation
            || !self.shared.ready.load(Ordering::Acquire)
        {
            return None;
        }
        let mut page_recencies = HashMap::<String, i64>::new();
        struct CandidateMetadata {
            block_id: String,
            parent_identity: Option<String>,
            order: Vec<String>,
            page: ApplicationSparseQueryPage,
        }
        let metadata = by_block
            .into_values()
            .map(|row| {
                let recency = if uses_recency {
                    *page_recencies
                        .entry(row.page_path.clone())
                        .or_insert_with(|| {
                            page_recency(
                                graph_root,
                                &row.page_name,
                                &row.page_path,
                                row.page_text_kind,
                                journal_format,
                            )
                        })
                } else {
                    i64::MIN
                };
                CandidateMetadata {
                    block_id: Uuid::from_bytes(row.block_id).to_string(),
                    parent_identity: row.parent.map(|id| Uuid::from_bytes(id).to_string()),
                    order: vec![row.order, Uuid::from_bytes(row.block_id).to_string()],
                    page: ApplicationSparseQueryPage {
                        name: row.page_name,
                        path: row.page_path.clone(),
                        kind: page_kind_from_sql(row.page_text_kind)?,
                        is_org: Format::from_path(Path::new(&row.page_path)) == Format::Org,
                        recency,
                    },
                }
                .into()
            })
            .collect::<Option<Vec<_>>>()?;
        let documents = pages
            .iter()
            .map(|(entry, document)| (entry.rel_path.as_str(), document.as_ref()))
            .collect::<HashMap<_, _>>();
        let candidates = metadata
            .iter()
            .map(|candidate| {
                let document = documents.get(candidate.page.path.as_str())?;
                let block = block_at_order(&document.roots, &candidate.order[0])?;
                (block.uuid == candidate.block_id).then_some(ParserSparseQueryCandidate {
                    block,
                    identity: &candidate.block_id,
                    page: &candidate.page,
                    parent_identity: candidate.parent_identity.as_deref(),
                    dfs_order: &candidate.order,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let result = run_parser_sparse_task_query_bounded(
            &candidates,
            query_src,
            max_rows,
            max_bytes,
            config,
            registry,
        )
        .ok()?;
        let current = (self.shared.ready.load(Ordering::Acquire)
            && self.shared.ready_generation.load(Ordering::Acquire) == cache_generation)
            .then_some(result);
        #[cfg(test)]
        if current.is_some() {
            self.shared.indexed_reads.fetch_add(1, Ordering::Relaxed);
        }
        current
    }

    /// Abandon the projection read when the lowering's candidate set is not
    /// selective enough to beat the parser walk it would replace.
    ///
    /// The walk costs one cheap in-memory predicate per page of the whole
    /// graph, so its cost is proportional to the graph. The projection route
    /// costs a SQL scan plus, per candidate, a SQLite point read, a page DTO
    /// construction and a document clone — each far more expensive than one
    /// walk step. So the route only wins while the candidate set is a small
    /// FRACTION of the graph, which is why the cutoff scales with the graph
    /// rather than being an absolute count.
    ///
    /// `1/32` is taken from the measured corpus (1,049 pages, 14,538 blocks;
    /// `tine-agents/evidence/wave4/b4b/`). Every class the route made faster
    /// there produced at most 3 candidates (0.29% of the graph); the two
    /// classes it made dramatically slower produced 91 and 104 (8.7% and 9.9%,
    /// costing 1.08 -> 11.19 ms and 0.46 -> 3.31 ms). `1/32` sits about 10x
    /// above every measured winner and about 2.8x below every measured loser.
    /// The floor keeps small graphs — including test fixtures — on the route,
    /// where the absolute cost of materializing a few candidates is trivial.
    ///
    /// Abandoning is the safe direction: it returns exactly today's behaviour.
    /// A cutoff set too low forfeits a speedup; one set too high reintroduces a
    /// 10x stall on the typing path.
    fn candidate_cutoff(graph_page_count: usize) -> usize {
        const SELECTIVE_FRACTION: usize = 32;
        const SMALL_GRAPH_FLOOR: usize = 32;
        (graph_page_count / SELECTIVE_FRACTION).max(SMALL_GRAPH_FLOOR)
    }

    pub(crate) fn simple_query_candidate_paths(
        &self,
        cache_generation: u64,
        plan: &SimpleQueryCandidatePlan,
        graph_page_count: usize,
    ) -> Option<std::collections::BTreeSet<PathBuf>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let lowered = crate::oplog::query_lowering::lower_simple_query_candidate_plan(
            &read,
            plan,
            &std::collections::HashSet::new(),
        )
        .ok()?;

        // RETIREMENT-CANDIDATE: the candidate-count escape hatch below, together
        // with the Direct whole-graph parser walk it hands the query back to.
        //
        // WHAT MAY BE DELETED: this `candidate_cutoff` test and the
        // `run_query`/`run_query_bounded` fallback arms that call
        // `Graph::direct_projection_note_fallback_read` after it fires. Deleting
        // them makes every ready `SimpleQueryCandidatePlan::Indexed` plan
        // unconditionally candidate-only.
        //
        // CONDITION FOR DELETION: the hatch exists only because
        // `lower_simple_query_candidate_plan` returns a page SUPERSET rather
        // than the answer — `and` takes the first leaf instead of intersecting,
        // `Page`/`Namespace`/`Journal` full-scan `navigation_pages`, values never
        // push down, and the block ids SQL already returned are discarded at the
        // trait boundary. When the lowering returns the ANSWER, the candidate set
        // is selective by construction, this test can never fire, and it goes.
        // That work is card `PVTI_lAHOAAbLVc4BhPsyzg5VyLk`, not this packet.
        //
        // WHAT CURRENTLY BLOCKS DELETION — read this before deleting the walk
        // along with the hatch: the parser walk is not merely the fallback, it is
        // the CORRECTNESS ORACLE for the real lowering that would replace it, and
        // no external oracle exists (Logseq's DB version evaluates in in-memory
        // DataScript with SQLite as a mere datom store; Dataview is frozen; Bases
        // is closed). The walk answers every query from the parsed documents in
        // ~1 ms over the 1,045-file anonymized graph, so the acceptance gate for
        // a real lowering is DIFFERENTIAL AGAINST THE WALK — the shape
        // `crate::query::tests::sparse_task_query_runner_matches_existing_page_evaluator`
        // already uses. The walk therefore outlives the lowering by at least one
        // release as a test-only oracle; it is NOT deletable the moment SQL
        // works. Retire the hatch first, keep the walk, and retire the walk only
        // after a release of differential agreement.
        if lowered.page_ids.len() > Self::candidate_cutoff(graph_page_count) {
            return None;
        }

        let mut paths = std::collections::BTreeSet::new();
        for page_id in lowered.page_ids {
            let page = read
                .page_with_header_validation(page_id, |_, kind| match kind {
                    0 | 1 => Ok(()),
                    _ => Err(tine_storage::sqlite::MaterializationError::Corrupt(
                        format!("unknown Direct Files text kind {kind}"),
                    )),
                })
                .ok()??;
            paths.insert(PathBuf::from(page.path));
        }
        let current = self.ready_at(cache_generation).then_some(paths);
        #[cfg(test)]
        if current.is_some() {
            self.shared.indexed_reads.fetch_add(1, Ordering::Relaxed);
        }
        current
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
    /// registry producer: it yields the same [`OwnerRow`] stream the Managed
    /// materialized read and the cold document iterator yield, and the
    /// aggregator downstream is byte-for-byte the same function.
    ///
    /// `None` means "not ready, or the read refused" — the caller falls back to
    /// the document iterator, exactly as §5.9's dispatch does for queries.
    pub(crate) fn property_owner_rows(
        &self,
        cache_generation: u64,
    ) -> Option<(
        Vec<crate::query::registry::OwnerRow>,
        HashMap<String, crate::query::registry::PageMeta>,
    )> {
        use crate::query::registry::{OwnerRow, OwnerType, PageMeta};

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
                    direct_registry_page_key(row.page_id),
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
                    PhysicalEntityId::Page(id) => (OwnerType::Page, format!("p:{}", hex16(id))),
                    PhysicalEntityId::Block(id) => (OwnerType::Block, format!("b:{}", hex16(id))),
                };
                rows.push(OwnerRow {
                    owner_type,
                    owner_id,
                    page_id: direct_registry_page_key(row.page_id),
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

    /// §5.9's dispatched read: run ONE lowered statement through the D-15 seam
    /// at the current cache generation, and say which of the three §5.9 states
    /// the attempt landed in.
    ///
    /// The three are not interchangeable and the caller acts differently on
    /// each, which is why they are not collapsed into `Option` the way every
    /// other reader here collapses them:
    ///
    /// * [`StatementRead::NotReady`] — open reconciliation, a full rebuild, or
    ///   the milliseconds after a save while the delta applies. Nothing is
    ///   wrong; the walk answers and the worker is already on its way.
    /// * [`StatementRead::Failed`] — the read was ATTEMPTED and did not answer
    ///   (a SQL error, a resource limit, an unopenable file). The projection is
    ///   disposable derived state (D-3), so the answer is recovery, not refusal:
    ///   the caller notes the fallback and schedules a full snapshot.
    /// * [`StatementRead::Rows`] — the answer.
    pub(crate) fn run_statement(
        &self,
        cache_generation: u64,
        sql: &str,
        parameters: &[PhysicalQueryValue],
        regexes: &QueryRegexProgram,
    ) -> StatementRead {
        // The injection lives here and not in `seam_read`, so it fails the
        // DISPATCHED statement and not whichever readiness probe happened to
        // reach the seam first.
        #[cfg(test)]
        if self
            .shared
            .inject_read_failure
            .swap(false, Ordering::AcqRel)
        {
            return StatementRead::Failed;
        }
        let read = self.seam_read(cache_generation, sql, parameters, Some(regexes));
        #[cfg(test)]
        if matches!(read, StatementRead::Rows(_)) {
            self.shared.statement_reads.fetch_add(1, Ordering::Relaxed);
        }
        read
    }

    /// One read through the D-15 seam. [`DirectProjection::run_statement`] is
    /// this plus §5.9's dispatched-statement census and §4.3.2's regex
    /// registration; [`DirectProjection::fts_ready`] is this without either,
    /// because a readiness probe is not an answer and binds no pattern.
    ///
    /// `regexes` is `Some` for exactly the dispatched statements, and it is
    /// installed INSIDE the seam lock together with the read it belongs to:
    /// this connection is pooled and reused, so a statement's compiled-regex
    /// table has to REPLACE the previous statement's rather than be added to
    /// it, and no other execution may run between the two.
    fn seam_read(
        &self,
        cache_generation: u64,
        sql: &str,
        parameters: &[PhysicalQueryValue],
        regexes: Option<&QueryRegexProgram>,
    ) -> StatementRead {
        if !self.ready_at(cache_generation) {
            return StatementRead::NotReady;
        }
        // Named `seam`, not `reader`, for the reason the field is (see
        // `ProjectionShared::statement_seam`).
        let mut seam = self.shared.statement_seam.lock().unwrap();
        if seam.is_none() {
            *seam = PhysicalProjectionQueryReader::open(&self.shared.path).ok();
        }
        let Some(seam) = seam.as_ref() else {
            return StatementRead::Failed;
        };
        // Unconditional for a dispatched statement, including the empty table:
        // installing nothing would leave the PREVIOUS execution's IDs bound on
        // a reused connection, and a stale ID that answered would be a wrong
        // result rather than a failed read.
        if let Some(regexes) = regexes {
            if seam.set_query_regex_predicate(regexes.predicate()).is_err() {
                return StatementRead::Failed;
            }
        }
        let Ok(rows) = seam.run_projection_query(sql, parameters) else {
            return StatementRead::Failed;
        };
        // A snapshot that straddles a rebuild is not a snapshot — the same
        // re-check every other reader here makes. The generation moving is not a
        // projection defect, so it is `NotReady` and not `Failed`.
        if !self.ready_at(cache_generation) {
            return StatementRead::NotReady;
        }
        #[cfg(test)]
        self.shared.indexed_reads.fetch_add(1, Ordering::Relaxed);
        StatementRead::Rows(rows)
    }

    /// The EXISTING FTS-building signal (§5.10), read on the SAME materialized
    /// read and generation as the query it accelerates and SEPARATELY from
    /// projection readiness. `false` means the transient building phase, where
    /// the compiler omits candidate bounds and evaluates the same exact
    /// predicates on the ready block columns.
    ///
    /// A read that cannot answer reports `false`, which costs a bound and never
    /// an answer.
    pub(crate) fn fts_ready(&self, cache_generation: u64) -> bool {
        if self.shared.fts_ever_ready.load(Ordering::Acquire)
            && self.shared.fts_ready_at.load(Ordering::Acquire) == cache_generation
        {
            return true;
        }
        let ready = self.probe_fts_ready(cache_generation);
        if ready {
            self.shared
                .fts_ready_at
                .store(cache_generation, Ordering::Release);
            self.shared.fts_ever_ready.store(true, Ordering::Release);
        }
        ready
    }

    fn probe_fts_ready(&self, cache_generation: u64) -> bool {
        matches!(
            self.seam_read(
                cache_generation,
                "SELECT phase FROM search_fts_build WHERE singleton = 1",
                &[],
                None,
            ),
            StatementRead::Rows(rows)
                if matches!(
                    rows.first().and_then(|row| row.first()),
                    Some(PhysicalQueryValue::Integer(1))
                )
        )
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

    pub(crate) fn fuzzy_candidate_paths(
        &self,
        cache_generation: u64,
        normalized_needle: &str,
    ) -> Option<std::collections::HashSet<String>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut paths = std::collections::HashSet::new();
        drain_after(
            |after, batch| {
                read.fuzzy_subsequence_candidate_pages_after(normalized_needle, after, batch)
            },
            |row| row.page_id,
            |row| {
                paths.insert(row.path);
                Ok(())
            },
            |_, _| None,
        )
        .ok()?;
        let current = self.ready_at(cache_generation).then_some(paths);
        #[cfg(test)]
        if current.is_some() {
            self.shared
                .fuzzy_candidate_reads
                .fetch_add(1, Ordering::Relaxed);
        }
        current
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

    pub(crate) fn reference_candidate_paths(
        &self,
        cache_generation: u64,
        names_norm: &[String],
        kind: ReferenceKind,
    ) -> Option<std::collections::BTreeSet<PathBuf>> {
        if !self.ready_at(cache_generation) {
            return None;
        }
        if kind == ReferenceKind::Plain
            && names_norm
                .iter()
                .any(|name| !name.chars().any(char::is_alphanumeric))
        {
            return None;
        }
        let mut reader = self.shared.reader.lock().unwrap();
        if reader.is_none() {
            *reader = PhysicalGraphProjectionDatabase::open_read_only(&self.shared.path).ok();
        }
        let read = reader.as_ref()?.read();
        let mut page_ids = std::collections::BTreeSet::new();
        for name in names_norm {
            match kind {
                ReferenceKind::Explicit => {
                    drain_after(
                        |after, batch| read.page_referrer_candidates_after(name, after, batch),
                        |row| (row.source_page_id, row.source),
                        |row| {
                            page_ids.insert(row.source_page_id);
                            Ok(())
                        },
                        |_, _| None,
                    )
                    .ok()?;
                }
                ReferenceKind::Plain => {
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
        self.ready_at(cache_generation).then_some(paths)
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
        self.shared.ready.load(Ordering::Acquire)
            && self.shared.ready_generation.load(Ordering::Acquire) == generation
    }

    #[cfg(test)]
    pub(crate) fn indexed_reads(&self) -> u64 {
        self.shared.indexed_reads.load(Ordering::Relaxed)
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
}

fn block_at_order<'a>(roots: &'a [DocBlock], order: &str) -> Option<&'a DocBlock> {
    let mut siblings = roots;
    let mut found = None;
    for component in order.split('/') {
        if component.len() != 8 {
            return None;
        }
        let index = usize::try_from(u32::from_str_radix(component, 16).ok()?).ok()?;
        let block = siblings.get(index)?;
        found = Some(block);
        siblings = &block.children;
    }
    found
}

impl Drop for DirectProjection {
    fn drop(&mut self) {
        let mut pending = self.shared.pending.lock().unwrap();
        pending.stop = true;
        self.shared.changed.notify_one();
    }
}

/// Report a Direct Files projection failure that leaves the parser fallback in
/// charge.
///
/// The always-on line names the failure family in fixed words and carries
/// nothing else. I-5: the detail at both call sites is free-form prose from the
/// projection WRITE path, and that path names the graph — `apply_pending`
/// formats `entry.rel_path` straight into its error string, and
/// `MaterializationError`'s payloads are free-form `String`s produced while
/// storing parsed page text. I-9: the family still reaches the always-on
/// record, because a user who is not running under `TINE_DEBUG` otherwise sees
/// only a silently slower graph. The prose stays on the directed debug channel.
fn report_projection_failure(family: &str, detail: &dyn std::fmt::Display) {
    eprintln!("[tine] Direct Files SQLite projection {family}");
    if crate::sync_runtime::runtime_debug_diagnostics_enabled() {
        eprintln!("[tine] Direct Files SQLite projection {family}; directed detail: {detail}");
    }
}

fn projection_worker(shared: Arc<ProjectionShared>) {
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
        let (full, deltas, latest_generation, rebuild) = {
            let mut pending = shared.pending.lock().unwrap();
            while pending.full.is_none() && pending.deltas.is_empty() && !pending.stop {
                pending = shared.changed.wait(pending).unwrap();
            }
            if pending.stop {
                shared.worker_available.store(false, Ordering::Release);
                shared.changed.notify_all();
                return;
            }
            shared.worker_busy.store(true, Ordering::Release);
            let rebuild = pending.full.is_some() && std::mem::take(&mut pending.rebuild);
            (
                pending.full.take(),
                std::mem::take(&mut pending.deltas),
                pending.latest_generation,
                rebuild,
            )
        };
        let had_full = full.is_some();
        #[cfg(test)]
        run_before_apply_pending_hook();
        let applied = if requires_full_rebuild && !had_full {
            Err("a prior projection failure requires a complete parser snapshot".into())
        } else {
            (|| {
                if rebuild || requires_full_rebuild || writer_slot.is_none() {
                    // Drop every connection before the disposable file can be
                    // replaced; a reader must not retain an old file handle.
                    let mut reader = shared.reader.lock().unwrap();
                    let mut seam = shared.statement_seam.lock().unwrap();
                    reader.take();
                    seam.take();
                    shared.fts_ever_ready.store(false, Ordering::Release);
                    writer_slot.take();
                    let mut database = open_projection_database(&shared.path)
                        .map_err(|error| error.to_string())?;
                    // Even repaired DDL leaves unchanged source stamps behind.
                    // Reset them so the full snapshot lowers every source page.
                    database.reset().map_err(|error| error.to_string())?;
                    writer_slot = Some(database);
                }
                apply_pending(writer_slot.as_mut().unwrap(), full, deltas)
            })()
        };
        if let Err(error) = applied {
            requires_full_rebuild = true;
            shared.ready.store(false, Ordering::Release);
            shared.worker_failed.store(true, Ordering::Release);
            shared.worker_busy.store(false, Ordering::Release);
            shared.changed.notify_all();
            report_projection_failure("is stale; using parser fallback", &error);
            continue;
        }
        if had_full {
            requires_full_rebuild = false;
        }
        shared.worker_failed.store(false, Ordering::Release);
        let pending = shared.pending.lock().unwrap();
        shared.worker_busy.store(false, Ordering::Release);
        if !pending.rebuild
            && pending.full.is_none()
            && pending.deltas.is_empty()
            && pending.latest_generation == latest_generation
        {
            shared
                .ready_generation
                .store(latest_generation, Ordering::Release);
            shared.ready.store(true, Ordering::Release);
            shared.changed.notify_all();
        }
    }
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

fn apply_pending(
    database: &mut PhysicalGraphProjectionDatabase,
    full: Option<PendingFull>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<(), String> {
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
    if !deltas.is_empty() {
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
                } => {
                    replacement_sources.push(PhysicalGraphProjectionSourceRevision {
                        page_id: page_id(&entry.rel_path),
                        revision: projection_source_revision(&revision, parse_config.digest()),
                    });
                    let (mut page, mut postings, mut page_aliases) =
                        physical_page(&entry, &document, &parse_config)?;
                    page.query_page_order = Some(query_page_order);
                    replacements.push(page);
                    reference_postings.append(&mut postings);
                    aliases.append(&mut page_aliases);
                }
                PageDelta::Delete { entry } => deletions.push(page_id(&entry.rel_path)),
            }
        }
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
    }
    Ok(())
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
/// exact function emits against the Managed Storage producer's and the walk's,
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
    PHYSICAL_PAGE_LOWERINGS.fetch_add(1, Ordering::Relaxed);
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
        let block_id = Uuid::parse_str(&block.uuid)
            .map_err(|_| {
                format!(
                    "block has no assigned runtime UUID in projection: {}",
                    block.uuid
                )
            })?
            .into_bytes();
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
        let (query_visible, query_visible_folded) = crate::query::derived::query_visible_columns(
            &projection.visible,
            Some(&projection.visible_lower),
        );
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
    let mut block = DocBlock::new(raw);
    block.is_org = is_org;
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

fn page_kind_from_sql(kind: i64) -> Option<PageKind> {
    match kind {
        0 => Some(PageKind::Page),
        1 => Some(PageKind::Journal),
        _ => None,
    }
}

fn page_recency(
    root: &Path,
    name: &str,
    relative_path: &str,
    kind: i64,
    journal_format: &crate::date::JournalFormat,
) -> i64 {
    journal_format.page_recency_secs(kind == 1, name, &root.join(relative_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Graph;
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    static PROJECTION_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-direct-projection-{tag}-{}", Uuid::new_v4()))
    }

    fn reset_lowerings() {
        PHYSICAL_PAGE_LOWERINGS.store(0, Ordering::Relaxed);
    }

    fn lowerings() -> u64 {
        PHYSICAL_PAGE_LOWERINGS.load(Ordering::Relaxed)
    }

    fn signature(groups: &[crate::model::RefGroup]) -> Vec<(String, Vec<(String, String)>)> {
        groups
            .iter()
            .map(|group| {
                (
                    group.page.clone(),
                    group
                        .blocks
                        .iter()
                        .map(|block| (block.id.clone(), block.raw.clone()))
                        .collect(),
                )
            })
            .collect()
    }

    fn wait_ready(graph: &Graph) {
        let started = Instant::now();
        while !graph.direct_projection_ready_test() {
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "Direct Files projection did not converge"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn direct_projection_matches_parser_tasks_and_tracks_replace_delete() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("task-parity");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(
            root.join("pages/tasks.md"),
            "- TODO [#A] parent\n\t- TODO child\n- TODO other\n  SCHEDULED: <2026-08-13 Thu>\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/org.org"), "* TODO [#B] org task\n").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        for query in [
            "(task TODO)",
            "(and (task TODO) (priority A))",
            "(and (task TODO) (scheduled))",
            "(and (task TODO) (sort-by priority desc))",
        ] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let indexed = graph.run_query_bounded(query, 100, 1_000_000);
            assert_eq!(
                signature(&indexed.groups),
                signature(&oracle.groups),
                "{query}"
            );
            assert_eq!(
                (indexed.total, indexed.exceeded),
                (oracle.total, oracle.exceeded)
            );
        }
        assert!(graph.direct_projection_indexed_reads_test() >= 4);
        let indexed_reads = graph.direct_projection_indexed_reads_test();
        let repeated = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(
            signature(&repeated.groups),
            signature(
                &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
            )
        );
        assert_eq!(
            graph.direct_projection_indexed_reads_test(),
            indexed_reads,
            "the generation-keyed presentation memo must avoid repeated SQL/parser work"
        );

        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "tasks")
            .unwrap();
        let mut page = graph.load_page(&entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0].raw = "DONE [#A] parent".into();
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        for query in ["(task TODO)", "(task DONE)"] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let indexed = graph.run_query_bounded(query, 100, 1_000_000);
            assert_eq!(
                signature(&indexed.groups),
                signature(&oracle.groups),
                "{query}"
            );
        }

        graph.delete_page("org", PageKind::Page).unwrap();
        wait_ready(&graph);
        let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
        let indexed = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn b4_page_ref_and_property_facets_record_indexed_reads() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("b4-indexed-reads");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/source.md"),
            "category:: work\ntags:: work\n\n- TODO points to [[Target]]\n  status:: active\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();
        std::fs::write(root.join("pages/Project___Child.md"), "- namespace child\n").unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(root.join("journals/2026_09_03.md"), "- journal block\n").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        let indexed_before = graph.direct_projection_indexed_reads_test();
        for query in [
            "(page-ref Target)",
            "(and (page-ref Target) \"points\")",
            "(and \"points\" (page-ref Target))",
        ] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let indexed = graph.run_query_bounded(query, 100, 1_000_000);
            assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
            assert_eq!(
                (indexed.total, indexed.exceeded),
                (oracle.total, oracle.exceeded)
            );
        }
        assert_eq!(
            graph.property_facets(),
            crate::query::property_facets(&graph)
        );
        assert_eq!(
            graph.autocomplete_property_facets_bounded(100, 1_000_000),
            crate::query::autocomplete_property_facets_bounded(&graph, 100, 1_000_000)
        );
        assert!(
            graph.direct_projection_indexed_reads_test() >= indexed_before + 5,
            "PageRef and both property-facet entry points must use the generation-bound SQLite read"
        );

        // **SPEC §5.9's ready shape.** When the projection is ready and the
        // statement lowers, the STATEMENT answers: exactly one dispatched read,
        // no walk, no fallback — and the same answer the walk gives, including
        // `total` and `exceeded`. This replaces the candidate-plan route, which
        // selected a page SUPERSET and then walked it; the statement selects the
        // answer. There is no cost test in front of this and no fourth route:
        // `(journal)` and `"points"` below are deliberately in the list because
        // one is unselective and the other is an unbounded content predicate,
        // and §5.9 routes both to the statement anyway.
        for query in [
            "(and (task TODO) (page source))",
            "(property status active)",
            "(page-property category work)",
            "(page source)",
            "(namespace Project)",
            "(journal)",
            "(and (property status active) (page source))",
            "(or (page source) (page Target))",
            "\"points\"",
        ] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let statements_before = graph.direct_projection_statement_reads_test();
            let fallback_before = graph.direct_projection_fallback_reads_test();
            graph.reset_direct_projection_candidate_probe_test();
            let actual = graph.run_query_bounded(query, 100, 1_000_000);
            assert_eq!(
                signature(&actual.groups),
                signature(&oracle.groups),
                "{query}"
            );
            assert_eq!(
                (actual.total, actual.exceeded),
                (oracle.total, oracle.exceeded),
                "{query}"
            );
            assert_eq!(
                graph.direct_projection_statement_reads_test(),
                statements_before + 1,
                "{query}: exactly one dispatched statement must answer"
            );
            assert_eq!(
                crate::query::full_graph_query_evaluations(),
                0,
                "{query}: production invocation entered the forbidden full-graph evaluator"
            );
            assert_eq!(
                graph.direct_projection_fallback_reads_test(),
                fallback_before,
                "{query}: ready dispatch fell back"
            );
        }

        let statements_before = graph.direct_projection_statement_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let empty = graph.run_query_bounded("(", 100, 1_000_000);
        assert!(empty.groups.is_empty());
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "a refused source must not enter the graph evaluator"
        );
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before,
            "a refused source must not run a statement"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "a refused source must not record fallback access"
        );

        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.direct_projection_mark_stale_test();
        let fallback_query = "(and (page-ref Target) (not (page Missing)))";
        let oracle = crate::query::run_query_bounded(&graph, fallback_query, 100, 1_000_000);
        let fallback = graph.run_query_bounded(fallback_query, 100, 1_000_000);
        assert_eq!(signature(&fallback.groups), signature(&oracle.groups));
        assert_eq!(
            graph.property_facets(),
            crate::query::property_facets(&graph)
        );
        assert!(
            graph.direct_projection_fallback_reads_test() >= fallback_before + 2,
            "stale PageRef and facet reads must record parser fallbacks"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_damaged_query_table_is_rebuilt_without_a_source_edit() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("damaged-query-table");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/source.md"), "- TODO links [[Target]]\n").unwrap();
        let path = root.join("private/projection.sqlite");
        let graph = Graph::open(&root);
        graph.attach_direct_projection(path.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        // Persistently unavailable projection data, unlike a one-shot seam
        // error on a healthy file. Recovery must actually rebuild the cache.
        let damaged = rusqlite::Connection::open(&path).unwrap();
        damaged.execute("DROP TABLE block_path_refs", []).unwrap();
        drop(damaged);
        let query = "(page-ref Target)";
        let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
        let answer = graph.run_query_bounded(query, 100, 1_000_000);
        assert_eq!(signature(&answer.groups), signature(&oracle.groups));
        wait_ready(&graph);
        let statements_before = graph.direct_projection_statement_reads_test();
        // A different memo key must reach the repaired SQL table.
        let next = "(and (page-ref Target) (task TODO))";
        let oracle = crate::query::run_query_bounded(&graph, next, 100, 1_000_000);
        let answer = graph.run_query_bounded(next, 100, 1_000_000);
        assert_eq!(signature(&answer.groups), signature(&oracle.groups));
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before + 1
        );
        drop(graph);
        let _ = std::fs::remove_dir_all(root);
    }

    /// **SPEC §5.9's failed-read shape.** A read that was ATTEMPTED and did not
    /// answer owes two things, and today's code is why they are asserted rather
    /// than assumed: `run_query`'s sparse-task arm fell back with a bare
    /// `map_or_else` and never called `note_fallback_read`, so a failed read
    /// scheduled no recovery and the projection could sit unusable until the
    /// user happened to save a page.
    ///
    /// **In-scope scenario** (AGENTS §5): a torn or truncated projection file
    /// after a crash or power loss, a disk error, or a projection whose page set
    /// has drifted from the parsed cache. The projection is disposable derived
    /// state (D-3), so the answer is recovery and never refusal — the user's
    /// query is answered by the walk, no refusal reaches them, and `ready`
    /// returns WITHOUT a user edit.
    #[test]
    fn a_failed_statement_read_answers_by_walking_and_schedules_its_own_recovery() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("failed-read-recovers");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/source.md"),
            "- TODO points to [[Target]]\n  status:: active\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        let query = "(page-ref Target)";
        let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
        let fallbacks_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        graph.direct_projection_inject_read_failure_test();

        let answered = graph.run_query_bounded(query, 100, 1_000_000);
        assert_eq!(
            signature(&answered.groups),
            signature(&oracle.groups),
            "a failed read must be answered by the walk, not refused"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallbacks_before + 1,
            "a failed read must be counted exactly once"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            1,
            "the walk answers the failed read exactly once"
        );

        // The recovery obligation: `mark_stale` alone would only clear `ready`
        // and strand the projection. The full-snapshot enqueue is scheduled from
        // the already-parsed cache, so it needs no reparse, no disk read, and no
        // user action — `ready` comes back on its own.
        wait_ready(&graph);
        // A DIFFERENT query, because the walk's answer for the first one is now
        // in the derived cache under the same IR key — correctly, since the two
        // engines answer identically, so a cached walk result is a cached
        // answer and not a stale route.
        let after = "(property status active)";
        let after_oracle = crate::query::run_query_bounded(&graph, after, 100, 1_000_000);
        let statements_before = graph.direct_projection_statement_reads_test();
        let fallbacks_before = graph.direct_projection_fallback_reads_test();
        let recovered = graph.run_query_bounded(after, 100, 1_000_000);
        assert_eq!(
            signature(&recovered.groups),
            signature(&after_oracle.groups)
        );
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before + 1,
            "the recovered projection must answer through the statement again"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallbacks_before,
            "the recovered projection must not fall back"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    /// **SPEC §5.3's base order and hydration, together.**
    ///
    /// The statement carries no `ORDER BY` — the walk's base order is its page
    /// SOURCE's enumeration order and no projection column reproduces it — so
    /// order is the CALLER's, reproduced in the result construction. An identity
    /// gate that compares SETS cannot see an ordering regression, and today's
    /// gates compare sets; `signature` here compares the ordered page list AND
    /// each page's ordered block list.
    ///
    /// Two visible-order paths are covered because they are different paths and
    /// a feed-only repro misses real bugs: a routed NAMED page (nested blocks,
    /// document order within the page) and the JOURNAL feed (kind rank, journal
    /// before page at the same display name).
    ///
    /// The hydration claim rides along: the pages a dispatched query loads a
    /// `Document` for are exactly the pages its RESULT names (I-13, I-15). A
    /// hydration that loaded a candidate superset and filtered in Rust would be
    /// the whole-graph walk this campaign exists to delete, wearing a hat.
    #[test]
    fn the_dispatched_result_reproduces_the_walks_order_and_loads_only_result_pages() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("dispatch-order");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        // A routed named page with NESTED matches, so within-page document order
        // is observable: `tree/filter-top-level-blocks` keeps the outer match and
        // the grandchild, and the ordered comparison sees which comes first.
        std::fs::write(
            root.join("pages/Alpha.md"),
            "- TODO alpha one\n\t- plain middle\n\t\t- TODO alpha three\n- TODO alpha four\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/Beta.md"), "- TODO beta one\n").unwrap();
        // Never matches: it must not be hydrated.
        std::fs::write(root.join("pages/Gamma.md"), "- ordinary prose\n").unwrap();
        std::fs::write(
            root.join("journals/2026_06_28.md"),
            "- TODO journal one\n- TODO journal two\n",
        )
        .unwrap();
        std::fs::write(
            root.join("journals/2026_06_29.md"),
            "- TODO journal three\n",
        )
        .unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        let feed = "(task TODO)";
        let oracle = crate::query::run_query_bounded(&graph, feed, 100, 1_000_000);
        graph.reset_direct_projection_candidate_probe_test();
        let dispatched = graph.run_query_bounded(feed, 100, 1_000_000);
        assert_eq!(
            signature(&dispatched.groups),
            signature(&oracle.groups),
            "the journal feed must match the walk INCLUDING order"
        );
        // The fixture has to be able to fail: more than one page, and a page with
        // more than one block, or the ordered comparison proves nothing.
        assert!(
            dispatched.groups.len() >= 4,
            "fixture must span several pages: {:?}",
            dispatched
                .groups
                .iter()
                .map(|g| &g.page)
                .collect::<Vec<_>>()
        );
        assert!(
            dispatched.groups.iter().any(|g| g.blocks.len() > 1),
            "fixture must have a page with several ordered matches"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "the feed must not enter the whole-graph evaluator"
        );
        // I-13/I-15: exactly the RESULT's pages were hydrated. `Gamma` matches
        // nothing and must not be loaded.
        let hydrated = graph
            .direct_projection_hydrated_pages_test()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            hydrated.len(),
            dispatched.groups.len(),
            "pages loaded must equal result pages: {hydrated:?}"
        );
        assert!(
            !hydrated.iter().any(|path| path.ends_with("Gamma.md")),
            "a page the result does not name must not be hydrated: {hydrated:?}"
        );

        // The routed named-page path: the same query scoped to one page, whose
        // within-page order is document order and not any projection column.
        let routed = "(and (task TODO) (page Alpha))";
        let routed_oracle = crate::query::run_query_bounded(&graph, routed, 100, 1_000_000);
        graph.reset_direct_projection_candidate_probe_test();
        let routed_dispatched = graph.run_query_bounded(routed, 100, 1_000_000);
        assert_eq!(
            signature(&routed_dispatched.groups),
            signature(&routed_oracle.groups),
            "a routed named page must match the walk INCLUDING order"
        );
        assert_eq!(
            routed_dispatched.groups.len(),
            1,
            "the routed query names exactly one page"
        );
        assert_eq!(
            routed_dispatched.groups[0].blocks.len(),
            3,
            "Alpha contributes the outer match, its grandchild and its sibling, \
             in document order"
        );
        assert_eq!(
            graph.direct_projection_hydrated_pages_test().len(),
            1,
            "a one-page result hydrates exactly one page"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    /// **SPEC §5.9: an unselective shape takes the statement too — there is no
    /// fourth route.**
    ///
    /// This fixture exists because of the route it USED to prove. The candidate
    /// plan materialized a page SUPERSET and then walked it, which on `(journal)`
    /// meant materializing most of the graph and running 7–10× slower than the
    /// walk; a candidate-count hatch abandoned the projection on exactly that
    /// shape. §5.9 removes the reason for the hatch rather than the hatch's
    /// symptom: the statement selects the ANSWER, so an unselective shape costs
    /// what its answer costs and there is nothing to abandon.
    ///
    /// The obligation the hatch protected is kept as an assertion, not as a
    /// route: on the unselective shape the dispatched path must load exactly the
    /// RESULT's pages and must not enter the whole-graph evaluator. The hatch
    /// itself is still alive for Managed Storage's candidate route and goes with
    /// it (P1-e).
    #[test]
    fn an_unselective_shape_answers_through_the_statement_without_a_candidate_superset() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("b4-candidate-cutoff");
        std::fs::create_dir_all(root.join("journals")).unwrap();
        // 50 real journal dates, comfortably past the 32-page small-graph
        // floor. Two months, because a date that does not exist (2026-09-31)
        // is not a journal and would not become a candidate.
        for (month, days) in [(9, 30), (10, 20)] {
            for day in 1..=days {
                std::fs::write(
                    root.join(format!("journals/2026_{month:02}_{day:02}.md")),
                    "- journal block\n",
                )
                .unwrap();
            }
        }
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/source.md"),
            "- TODO points to [[Target]]\n  status:: active\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        // The shape the hatch existed for: its candidate set is most of the
        // graph. The fixture is pinned through the SURVIVING candidate lowering
        // so a fixture that stopped being unselective fails here rather than
        // silently testing nothing.
        let unselective = "(journal)";
        let raw = graph
            .direct_projection_candidate_paths_test(
                &crate::query::simple_query_candidate_plan(unselective),
                usize::MAX,
            )
            .expect("the lowering answers the unselective plan");
        assert!(
            raw.len() > 32,
            "fixture must exceed the old cutoff; got {} candidates",
            raw.len()
        );

        // Run the oracle BEFORE resetting the probes, so the oracle's own walk
        // is not counted as the production invocation's route evidence.
        let oracle = crate::query::run_query_bounded(&graph, unselective, 500, 4_000_000);
        let statements_before = graph.direct_projection_statement_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let dispatched = graph.run_query_bounded(unselective, 500, 4_000_000);

        assert_eq!(
            signature(&dispatched.groups),
            signature(&oracle.groups),
            "the statement must answer an unselective shape identically"
        );
        assert_eq!(
            (dispatched.total, dispatched.exceeded),
            (oracle.total, oracle.exceeded),
            "the statement must reproduce the walk's bound outcome"
        );
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before + 1,
            "an unselective shape is still answered by exactly one statement"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "a ready projection must not fall back on an unselective shape"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "an unselective shape must not enter the whole-graph evaluator"
        );
        // **The I-13/I-15 obligation the hatch used to buy with a route.** The
        // dispatched path loads exactly the pages the RESULT names — here every
        // journal, because every journal matches — and never a superset. The
        // number that mattered was "pages materialized that the answer does not
        // contain", and it is zero by construction now.
        assert_eq!(
            dispatched.groups.len(),
            oracle.groups.len(),
            "the dispatched result must name the walk's pages"
        );

        // Same graph, same readiness: a selective shape is the same one route.
        let selective = "(page-ref Target)";
        let selective_oracle = crate::query::run_query_bounded(&graph, selective, 500, 4_000_000);
        let statements_before = graph.direct_projection_statement_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let routed = graph.run_query_bounded(selective, 500, 4_000_000);
        assert_eq!(
            signature(&routed.groups),
            signature(&selective_oracle.groups)
        );
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before + 1,
            "a selective shape is answered by exactly one statement"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "a selective shape must not fall back"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "a selective shape must not enter the full-graph evaluator"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "manual B4 corpus gate; set TINE_B4_QUERY_CORPUS"]
    fn b4_corpus_page_ref_and_facets_match_oracle_with_route_evidence() {
        fn copy_tree(source: &Path, target: &Path) {
            std::fs::create_dir_all(target).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let kind = entry.file_type().unwrap();
                let destination = target.join(entry.file_name());
                if kind.is_dir() {
                    copy_tree(&entry.path(), &destination);
                } else if kind.is_file() {
                    std::fs::copy(entry.path(), destination).unwrap();
                }
            }
        }

        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let source = PathBuf::from(
            std::env::var("TINE_B4_QUERY_CORPUS").expect("TINE_B4_QUERY_CORPUS is required"),
        );
        let root = scratch("b4-corpus");
        if source.is_dir() {
            copy_tree(&source, &root);
        } else {
            std::fs::create_dir_all(root.join("pages")).unwrap();
            std::fs::copy(&source, root.join("pages/corpus-fixture.md")).unwrap();
        }
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/B4 Indexed Source.md"),
            "b4-page-facet:: yes\ntags:: b4-tag\n\n- TODO synthetic [[B4 Indexed Target]]\n  b4-facet:: yes\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pages/B4___Namespace.md"),
            "- synthetic namespace\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(root.join("journals/2026_09_03.md"), "- synthetic journal\n").unwrap();
        std::fs::write(
            root.join("pages/B4 Indexed Target.md"),
            "- synthetic target\n",
        )
        .unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join(".b4-private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        // The real graph is the only place the candidate-count escape hatch can
        // be observed end to end: `(journal)` lowers to a candidate set the size
        // of the journal directory, which no synthetic fixture reproduces at
        // scale. Both sides of the hatch are asserted, and the oracle equality
        // below holds on BOTH — that equality is what makes the parser walk the
        // correctness oracle the retirement marker names.
        let graph_page_count = graph.with_pages(|pages| pages.len());
        let cutoff = DirectProjection::candidate_cutoff(graph_page_count);
        let mut routed = 0usize;
        let mut abandoned = 0usize;
        for query in [
            "(page-ref \"B4 Indexed Target\")",
            "(and (task TODO) (page \"B4 Indexed Source\"))",
            "(property b4-facet yes)",
            "(page-property b4-page-facet yes)",
            "(page \"B4 Indexed Source\")",
            "(namespace B4)",
            "(journal)",
            "(and (property b4-facet yes) (page \"B4 Indexed Source\"))",
            "(or (page \"B4 Indexed Source\") (page \"B4 Indexed Target\"))",
        ] {
            let plan = crate::query::simple_query_candidate_plan(query);
            let oracle = crate::query::run_query_bounded(&graph, query, 20_000, 32 * 1024 * 1024);
            // `usize::MAX` asks for the raw lowering result; the production
            // cutoff then decides whether that set is worth materializing.
            let raw_paths = graph
                .direct_projection_candidate_paths_test(&plan, usize::MAX)
                .unwrap();
            let hatch_fires = raw_paths.len() > cutoff;
            // Probe the production cutoff itself, before any counter is
            // captured, so the probe's own read cannot skew the assertions.
            let routed_paths =
                graph.direct_projection_candidate_paths_test(&plan, graph_page_count);
            assert_eq!(
                routed_paths.is_none(),
                hatch_fires,
                "{query}: {} candidates against cutoff {cutoff} must decide the route",
                raw_paths.len()
            );
            let indexed_before = graph.direct_projection_indexed_reads_test();
            let fallback_before = graph.direct_projection_fallback_reads_test();
            graph.reset_direct_projection_candidate_probe_test();
            let indexed = graph.run_query_bounded(query, 20_000, 32 * 1024 * 1024);
            assert_eq!(
                signature(&indexed.groups),
                signature(&oracle.groups),
                "{query}: routed result must equal the parser oracle (hatch_fires={hatch_fires})"
            );
            assert_eq!(
                (indexed.total, indexed.exceeded),
                (oracle.total, oracle.exceeded)
            );
            if hatch_fires {
                abandoned += 1;
                assert_eq!(
                    graph.direct_projection_indexed_reads_test(),
                    indexed_before,
                    "{query}: an abandoned candidate set must not count an indexed read"
                );
                assert_eq!(
                    graph.direct_projection_fallback_reads_test(),
                    fallback_before + 1,
                    "{query}: an abandoned candidate set must note exactly one fallback read"
                );
                assert_eq!(
                    crate::query::full_graph_query_evaluations(),
                    1,
                    "{query}: an abandoned candidate set must take the parser walk"
                );
                assert!(
                    graph
                        .direct_projection_candidate_evaluated_paths_test()
                        .is_empty(),
                    "{query}: an abandoned candidate set must materialize no pages"
                );
            } else {
                routed += 1;
                assert_eq!(
                    graph.direct_projection_indexed_reads_test(),
                    indexed_before + 1
                );
                assert_eq!(
                    graph.direct_projection_fallback_reads_test(),
                    fallback_before
                );
                assert_eq!(crate::query::full_graph_query_evaluations(), 0);
                assert_eq!(
                    graph
                        .direct_projection_candidate_evaluated_paths_test()
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>(),
                    raw_paths
                );
            }
        }
        // Neither branch may go vacuous: a corpus that never routes proves
        // nothing about the projection, and one that never abandons proves
        // nothing about the hatch.
        assert!(
            routed > 0 && abandoned > 0,
            "the corpus gate must exercise both sides of the hatch \
             (routed={routed}, abandoned={abandoned}, cutoff={cutoff}, pages={graph_page_count})"
        );
        assert!(
            graph.property_facets() == crate::query::property_facets(&graph),
            "corpus query-builder facets differ from the parser oracle"
        );
        assert!(
            graph.autocomplete_property_facets_bounded(20_000, 32 * 1024 * 1024)
                == crate::query::autocomplete_property_facets_bounded(
                    &graph,
                    20_000,
                    32 * 1024 * 1024,
                ),
            "corpus autocomplete facets differ from the parser oracle"
        );
        // One indexed read per routed query, plus the two facet families above.
        assert!(graph.direct_projection_indexed_reads_test() >= (routed + 2) as u64);

        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.direct_projection_mark_stale_test();
        let fallback_query = "(and (page-ref \"B4 Indexed Target\") \"synthetic\")";
        let oracle =
            crate::query::run_query_bounded(&graph, fallback_query, 20_000, 32 * 1024 * 1024);
        let fallback = graph.run_query_bounded(fallback_query, 20_000, 32 * 1024 * 1024);
        assert!(
            signature(&fallback.groups) == signature(&oracle.groups),
            "corpus stale fallback differs from the parser oracle"
        );
        assert!(graph.direct_projection_fallback_reads_test() > fallback_before);

        let pages = graph.with_pages(|pages| pages.len());
        println!(
            "b4_corpus_gate pages={pages} indexed_reads={} fallback_reads={}",
            graph.direct_projection_indexed_reads_test(),
            graph.direct_projection_fallback_reads_test()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_projection_matches_fuzzy_search_and_virtual_reference_names() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("search-reference-parity");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/one.md"),
            "tags:: Page Tag, [[Property Page]]\nalias:: Alias Page\nquoted:: untouched\n\n- Characteristically useful [[Inline Page]]\n  aliases:: #Block Alias\n- c% literal\n",
        )
        .unwrap();
        std::fs::write(root.join("pages/two.md"), "- unrelated content\n").unwrap();
        let graph = Graph::open(&root);
        graph.warm_cache();
        let oracle = crate::query::search(&graph, "cly", 20);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        wait_ready(&graph);

        let candidate_pages = graph
            .direct_projection_fuzzy_candidate_pages("cly")
            .unwrap();
        assert_eq!(candidate_pages.len(), 1);
        assert_eq!(candidate_pages[0].0.rel_path, "pages/one.md");
        assert_eq!(signature(&graph.search("cly", 20)), signature(&oracle));
        assert!(graph.direct_projection_fuzzy_candidate_reads_test() > 0);
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            names,
            [
                "page tag",
                "property page",
                "alias page",
                "inline page",
                "block",
                "block alias",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
        assert!(graph.direct_projection_referenced_name_reads_test() > 0);

        let fuzzy_reads = graph.direct_projection_fuzzy_candidate_reads_test();
        let name_reads = graph.direct_projection_referenced_name_reads_test();
        graph.direct_projection_mark_stale_test();
        assert_eq!(signature(&graph.search("cly", 20)), signature(&oracle));
        assert_eq!(
            graph
                .referenced_page_names()
                .into_iter()
                .map(|name| crate::refs::page_key(&name))
                .collect::<std::collections::BTreeSet<_>>(),
            names
        );
        assert_eq!(
            graph.direct_projection_fuzzy_candidate_reads_test(),
            fuzzy_reads,
            "a stale generation must use the parser fallback"
        );
        assert_eq!(
            graph.direct_projection_referenced_name_reads_test(),
            name_reads,
            "a stale generation must not read reference names from SQLite"
        );

        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "one")
            .unwrap();
        let mut page = graph.load_page(&entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0].raw = "Nothing matching [[Replacement Page]]".into();
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        assert!(graph.search("cly", 20).is_empty());
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("replacement page"));
        assert!(!names.contains("inline page"));

        std::fs::write(
            root.join("pages/one.md"),
            "tags:: External Tag\n\n- Externally changed fuzzy [[External Page]]\n",
        )
        .unwrap();
        graph.sync_file_checked(&root.join("pages/one.md")).unwrap();
        wait_ready(&graph);
        assert!(!graph.search("ecf", 20).is_empty());
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("external tag"));
        assert!(names.contains("external page"));
        assert!(!names.contains("replacement page"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_projection_matches_parser_reference_family_and_stale_fallback() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("reference-family-parity");
        let target_id = "11111111-2222-4333-8444-555555555555";
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/target.md"),
            format!("alias:: Alias Target\n\n- target\n  id:: {target_id}\n"),
        )
        .unwrap();
        std::fs::write(
            root.join("pages/referrer.md"),
            format!(
                "- [[Alias Target]] and plain Alias Target and (({target_id})) (({target_id}))\n- another (({target_id}))\n"
            ),
        )
        .unwrap();
        std::fs::write(root.join("pages/unrelated.md"), "- unrelated\n").unwrap();

        let graph = Graph::open(&root);
        graph.warm_cache();
        let parser_aliases = crate::query::page_aliases_with_owners(&graph);
        let parser_backlinks = crate::query::backlinks(&graph, "target");
        let parser_unlinked = crate::query::unlinked_refs(&graph, "target");
        let parser_referrers = crate::query::block_referrers(&graph, target_id);
        let parser_resolved = crate::query::resolve_block(&graph, target_id);
        let parser_counts = graph.block_ref_counts().unwrap();

        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        wait_ready(&graph);

        assert_eq!(graph.page_aliases_with_owners(), parser_aliases);
        let explicit_candidates = graph.reference_candidate_pages(
            &[
                crate::refs::page_key("target"),
                crate::refs::page_key("Alias Target"),
            ],
            ReferenceKind::Explicit,
        );
        assert!(explicit_candidates.indexed);
        assert!(explicit_candidates.pages.len() < explicit_candidates.full_page_count);
        assert_eq!(
            signature(&crate::query::backlinks(&graph, "target")),
            signature(&parser_backlinks)
        );
        assert_eq!(
            signature(&crate::query::unlinked_refs(&graph, "target")),
            signature(&parser_unlinked)
        );
        assert_eq!(
            signature(&crate::query::block_referrers(&graph, target_id)),
            signature(&parser_referrers)
        );
        assert_eq!(
            crate::query::resolve_block(&graph, target_id)
                .as_ref()
                .map(|group| signature(std::slice::from_ref(group))),
            parser_resolved
                .as_ref()
                .map(|group| signature(std::slice::from_ref(group)))
        );
        assert_eq!(
            graph.block_ref_counts().unwrap().as_ref(),
            parser_counts.as_ref()
        );
        assert_eq!(graph.block_ref_counts().unwrap().get(target_id), Some(&2));

        let custom_path = root.join("pages/custom.md");
        std::fs::write(&custom_path, "- custom identity\n  id:: not-a-uuid\n").unwrap();
        assert!(graph.sync_file(&custom_path).is_some());
        wait_ready(&graph);
        assert_eq!(
            crate::query::resolve_block(&graph, "not-a-uuid")
                .and_then(|group| group.blocks.into_iter().next())
                .map(|block| block.raw),
            Some("custom identity\nid:: not-a-uuid".to_string())
        );

        graph.direct_projection_mark_stale_test();
        assert_eq!(graph.page_aliases_with_owners(), parser_aliases);
        assert_eq!(
            signature(&crate::query::backlinks(&graph, "target")),
            signature(&parser_backlinks)
        );
        assert_eq!(
            signature(&crate::query::block_referrers(&graph, target_id)),
            signature(&parser_referrers)
        );
        assert_eq!(
            graph.block_ref_counts().unwrap().as_ref(),
            parser_counts.as_ref()
        );

        let target_path = root.join("pages/target.md");
        std::fs::write(
            &target_path,
            format!("alias:: Changed Alias\n\n- target\n  id:: {target_id}\n"),
        )
        .unwrap();
        assert!(graph.sync_file(&target_path).is_some());
        wait_ready(&graph);
        let changed_aliases = graph.page_aliases_with_owners();
        assert!(changed_aliases
            .iter()
            .any(|(alias, owner, _)| alias == "changed alias" && owner == "target"));
        assert!(!changed_aliases
            .iter()
            .any(|(alias, _, _)| alias == "alias target"));

        graph.delete_page("target", PageKind::Page).unwrap();
        wait_ready(&graph);
        assert!(!graph
            .page_aliases_with_owners()
            .iter()
            .any(|(alias, _, _)| alias == "changed alias"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// GH #400. An ordinary edit has already published its parsed page and
    /// queued the exact one-page SQLite delta. A reference read which overlaps
    /// that short worker turn must not immediately turn into a whole-graph
    /// parser scan. Waiting for this already-running bounded delta preserves the
    /// same semantics and avoids the reported multi-second fallback.
    #[test]
    fn reference_lookup_waits_for_an_inflight_one_page_projection_delta() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("reference-delta-handoff");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();
        std::fs::write(root.join("pages/source.md"), "- unrelated\n").unwrap();

        let graph = Arc::new(Graph::open(&root));
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);

        let (worker_paused_tx, worker_paused_rx) = mpsc::channel();
        let (release_worker_tx, release_worker_rx) = mpsc::channel();
        *BEFORE_APPLY_PENDING.lock().unwrap() = Some(Box::new(move || {
            worker_paused_tx.send(()).unwrap();
            release_worker_rx.recv().unwrap();
        }));

        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "source")
            .unwrap();
        let mut page = graph.load_page(&entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0].raw = "plain target mention".into();
        graph.save_page(&page, baseline.as_deref()).unwrap();
        worker_paused_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the one-page projection delta reached the worker");

        let reader = Arc::clone(&graph);
        let (result_tx, result_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let candidates = reader.reference_candidate_pages(
                &[crate::refs::page_key("target")],
                ReferenceKind::Plain,
            );
            result_tx.send(candidates.indexed).unwrap();
        });

        match result_rx.recv_timeout(Duration::from_millis(100)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            result => {
                let _ = release_worker_tx.send(());
                panic!(
                    "reference lookup escaped to parser fallback before its queued delta completed: {result:?}"
                );
            }
        }
        release_worker_tx.send(()).unwrap();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            true,
            "the converged lookup must use current indexed candidates"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reference_wait_is_zero_cost_when_no_projection_work_exists() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("reference-no-work-wait");
        let projection = DirectProjection::start(root.join("projection.sqlite")).unwrap();
        let started = Instant::now();
        assert!(!projection.wait_for_reference_generation(1));
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "an unavailable projection must fall back immediately"
        );
        drop(projection);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_projection_preserves_external_uuid_ambiguity_for_parser_resolution() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("external-uuid-ambiguity");
        let target_id = "11111111-2222-4333-8444-555555555555";
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/alpha.md"),
            format!("- alpha claimant\n  id:: {target_id}\n"),
        )
        .unwrap();
        std::fs::write(
            root.join("pages/beta.md"),
            format!("- beta claimant\n  id:: {target_id}\n"),
        )
        .unwrap();

        let graph = Graph::open(&root);
        graph.warm_cache();
        let parser_resolution = crate::query::resolve_block(&graph, target_id)
            .map(|group| signature(std::slice::from_ref(&group)));
        let projection_path = root.join("private/projection.sqlite");
        graph
            .attach_direct_projection(projection_path.clone())
            .unwrap();
        wait_ready(&graph);

        let database = PhysicalGraphProjectionDatabase::open_read_only(&projection_path).unwrap();
        let claim = Uuid::parse_str(target_id).unwrap().into_bytes();
        assert_eq!(
            database
                .read()
                .blocks_by_logseq_uuid(claim, 2)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            crate::query::resolve_block(&graph, target_id)
                .map(|group| signature(std::slice::from_ref(&group))),
            parser_resolution,
            "SQLite must not choose one external UUID owner from an ambiguous graph"
        );
        drop(database);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reference_family_has_no_second_in_memory_semantic_index() {
        let model = include_str!("model.rs");
        for removed in [
            "alias_cache",
            "reference_candidate_index",
            "block_ref_count_cache",
            "block_index: RwLock",
        ] {
            assert!(
                !model.contains(removed),
                "Direct Files reference family reintroduced {removed} beside SQLite"
            );
        }
    }

    #[test]
    fn direct_projection_fuzzy_candidates_preserve_parser_corpus_semantics() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("search-corpus-parity");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/search.md"),
            "- Characteristically useful\n  - descendant Needle\n- Café and cafe\u{301}\n- 100% under_score back\\slash\n- MixedCASE\n- x a y b z\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pages/other.md"),
            "- Another characteristically useful result\n",
        )
        .unwrap();
        let cases = [
            ("", 20),
            ("   ", 20),
            ("cly", 20),
            ("needle", 20),
            ("CAFÉ", 20),
            ("cafe\u{301}", 20),
            ("%", 20),
            ("_", 20),
            ("\\", 20),
            ("mixedcase", 20),
            ("xyz", 20),
            ("cly", 1),
        ];
        let oracle_graph = Graph::open(&root);
        oracle_graph.warm_cache();
        let oracle = cases
            .iter()
            .map(|(query, limit)| signature(&crate::query::search(&oracle_graph, query, *limit)))
            .collect::<Vec<_>>();
        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(root.join("private/projection.sqlite"))
            .unwrap();
        assert!(
            graph.warm_cache_cancellable(|| false),
            "corpus cache failed to warm: {:?}",
            graph.page_index_failures()
        );
        wait_ready(&graph);
        for ((query, limit), expected) in cases.into_iter().zip(oracle) {
            assert_eq!(
                signature(&graph.search(query, limit)),
                expected,
                "{query:?}"
            );
        }
        let cancellation_checks = std::cell::Cell::new(0);
        assert!(crate::query::search_cancellable(&graph, "cly", 20, || {
            cancellation_checks.set(cancellation_checks.get() + 1);
            cancellation_checks.get() > 1
        })
        .is_empty());

        graph.rename_page("search", "renamed search").unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(
            signature(&graph.search("needle", 20)),
            signature(&crate::query::search(&graph, "needle", 20))
        );
        graph.delete_page("renamed search", PageKind::Page).unwrap();
        wait_ready(&graph);
        assert!(graph.search("needle", 20).is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unavailable_projection_keeps_direct_files_query_semantics() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("fallback");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(
            root.join("pages/tasks.md"),
            "- TODO Characteristically readable [[Inline Only]]\n  alias:: #Alias Only\n",
        )
        .unwrap();
        let blocked_parent = root.join("not-a-directory");
        std::fs::write(&blocked_parent, b"ordinary file").unwrap();

        let graph = Graph::open(&root);
        graph
            .attach_direct_projection(blocked_parent.join("projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        std::thread::sleep(Duration::from_millis(30));
        let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
        let fallback = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(signature(&fallback.groups), signature(&oracle.groups));
        assert_eq!(graph.direct_projection_indexed_reads_test(), 0);
        assert_eq!(
            signature(&graph.search("cly", 20)),
            signature(&crate::query::search(&graph, "cly", 20))
        );
        let names = graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("inline only"));
        assert!(names.contains("alias only"));
        assert_eq!(graph.direct_projection_fuzzy_candidate_reads_test(), 0);
        assert_eq!(graph.direct_projection_referenced_name_reads_test(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_graph_instance_cannot_replace_ready_projection_facts() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("single-writer");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/tasks.md"), "- TODO one\n").unwrap();
        let database = scratch("single-writer-db").join("projection.sqlite");

        let owner = Graph::open(&root);
        owner.attach_direct_projection(database.clone()).unwrap();
        owner.warm_cache();
        wait_ready(&owner);

        let fallback = Graph::open(&root);
        fallback.attach_direct_projection(database.clone()).unwrap();
        fallback.warm_cache();
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !fallback.direct_projection_ready_test(),
            "a second graph instance must not publish into the first instance's ready database"
        );
        let oracle = crate::query::run_query_bounded(&fallback, "(task TODO)", 100, 1_000_000);
        let actual = fallback.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(signature(&actual.groups), signature(&oracle.groups));
        assert_eq!(fallback.direct_projection_indexed_reads_test(), 0);

        let owner_oracle = crate::query::run_query_bounded(&owner, "(task TODO)", 100, 1_000_000);
        let owner_actual = owner.run_query_bounded("(task TODO)", 100, 1_000_000);
        assert_eq!(
            signature(&owner_actual.groups),
            signature(&owner_oracle.groups)
        );
        assert!(owner.direct_projection_indexed_reads_test() > 0);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }

    #[test]
    fn coalesced_edits_keep_first_insertion_page_order_and_readds_append() {
        let entry = |name: &str| PageEntry {
            name: name.into(),
            kind: PageKind::Page,
            date_key: None,
            rel_path: format!("pages/{name}.md"),
            path: PathBuf::from(format!("pages/{name}.md")),
        };
        let replacement = |name: &str| PageDelta::Replace {
            entry: entry(name),
            document: Arc::new(crate::doc::parse("- text")),
            revision: "exact-revision".into(),
            parse_config: Arc::new(ParseConfig::default()),
            query_page_order: 0,
        };
        let position = |pending: &PendingProjection, name: &str| match &pending.deltas
            [&format!("pages/{name}.md")]
            .1
        {
            PageDelta::Replace {
                query_page_order, ..
            } => *query_page_order,
            _ => panic!("replacement expected"),
        };
        let mut pending = PendingProjection::default();
        pending.record_delta(1, replacement("z-first"));
        pending.record_delta(2, replacement("a-second"));
        pending.record_delta(3, replacement("z-first"));
        assert_eq!(position(&pending, "z-first"), 0);
        assert_eq!(position(&pending, "a-second"), 1);
        assert_eq!(pending.deltas.len(), 2, "first page edit is coalesced");
        pending.record_delta(
            4,
            PageDelta::Delete {
                entry: entry("z-first"),
            },
        );
        pending.record_delta(5, replacement("z-first"));
        assert_eq!(position(&pending, "a-second"), 1);
        assert_eq!(position(&pending, "z-first"), 2);
    }

    #[test]
    fn clean_reopen_reuses_sqlite_and_external_edit_relowers_only_one_page() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("reopen-revisions");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
        std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
        let database = scratch("reopen-revisions-db").join("projection.sqlite");

        reset_lowerings();
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(lowerings(), 2);
        }
        std::thread::sleep(Duration::from_millis(20));

        reset_lowerings();
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(lowerings(), 0, "unchanged pages must stay inside SQLite");
        }
        std::thread::sleep(Duration::from_millis(20));

        std::fs::write(root.join("pages/one.md"), "- TODO one changed\n").unwrap();
        reset_lowerings();
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(
                lowerings(),
                1,
                "one changed page must produce one SQL delta"
            );
            assert_eq!(
                signature(
                    &graph
                        .run_query_bounded("(task TODO)", 100, 1_000_000)
                        .groups
                ),
                signature(
                    &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
                )
            );
        }

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }

    #[test]
    fn extractor_version_participates_in_disposable_source_revision() {
        let source = "sha256:unchanged-source";
        let digest = ParseConfig::default().digest();
        let projected = projection_source_revision(source, digest);
        let hex = digest
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            projected,
            format!("direct-facts-v2:{hex}:sha256:unchanged-source")
        );
        assert_ne!(projected, source);
    }

    /// Guard 4, Direct Files half (§5.8 J7). Reconciliation compares only
    /// source revisions, so a config edit that changes no file byte must still
    /// change the revision it compares -- otherwise every unchanged page keeps
    /// rows derived under the old config forever.
    #[test]
    fn a_parse_config_change_moves_every_source_revision() {
        let source = "sha256:unchanged-source";
        let mut edited = ParseConfig::default();
        edited.separated_by_commas.push("authors".to_owned());
        assert_ne!(ParseConfig::default().digest(), edited.digest());
        assert_ne!(
            projection_source_revision(source, ParseConfig::default().digest()),
            projection_source_revision(source, edited.digest()),
        );
    }

    /// **F11.** The parse config travels inside each queued work item, so two
    /// replacements coalesced into one worker turn are each lowered and stamped
    /// under the config they were queued with -- never under whichever config
    /// the last enqueue happened to leave beside the queue, and never under a
    /// default that absence could stand in for.
    ///
    /// The stamp is what reconciliation compares, so a page carrying another
    /// page's config digest is a page whose rows answer a question the config
    /// no longer asks and which no later reopen will notice (J7, D-1).
    #[test]
    fn each_queued_page_lowers_under_the_config_it_was_queued_with() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = scratch("per-item-parse-config");
        std::fs::create_dir_all(&root).unwrap();
        let mut database = open_projection_database(&root.join("projection.sqlite")).unwrap();

        let default_config = Arc::new(ParseConfig::default());
        let edited_config = Arc::new({
            let mut edited = ParseConfig::default();
            edited.separated_by_commas.push("authors".to_owned());
            edited
        });
        assert_ne!(default_config.digest(), edited_config.digest());

        let queued = |rel_path: &str, parse_config: &Arc<ParseConfig>| {
            (
                rel_path.to_owned(),
                (
                    1_u64,
                    PageDelta::Replace {
                        entry: PageEntry {
                            name: rel_path.trim_end_matches(".md").to_owned(),
                            kind: PageKind::Page,
                            date_key: None,
                            rel_path: rel_path.to_owned(),
                            path: root.join(rel_path),
                        },
                        document: Arc::new({
                            let mut document = crate::doc::parse("- authors:: ada, grace\n");
                            crate::model::assign_doc_runtime_ids(&mut document.roots, rel_path);
                            document
                        }),
                        revision: format!("sha256:{rel_path}"),
                        parse_config: Arc::clone(parse_config),
                        query_page_order: u64::from(rel_path == "beta.md"),
                    },
                ),
            )
        };
        let deltas = BTreeMap::from([
            queued("alpha.md", &default_config),
            queued("beta.md", &edited_config),
        ]);
        apply_pending(&mut database, None, deltas).unwrap();

        let stamped = |alpha: &Arc<ParseConfig>, beta: &Arc<ParseConfig>| {
            database
                .source_delta(&[
                    PhysicalGraphProjectionSourceRevision {
                        page_id: page_id("alpha.md"),
                        revision: projection_source_revision("sha256:alpha.md", alpha.digest()),
                    },
                    PhysicalGraphProjectionSourceRevision {
                        page_id: page_id("beta.md"),
                        revision: projection_source_revision("sha256:beta.md", beta.digest()),
                    },
                ])
                .unwrap()
                .replacements
        };
        assert!(
            stamped(&default_config, &edited_config).is_empty(),
            "each page must carry the digest of the config it was queued with"
        );
        // Not vacuous: the two stamps really are distinct, so the assertion
        // above could have failed.
        assert_eq!(
            stamped(&edited_config, &default_config).len(),
            2,
            "swapping the two configs must make both pages stale"
        );
        drop(database);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn storage_contract_names_the_generation_bound_cutover() {
        let contract = include_str!("../../../docs/storage-sync-contract.md");
        assert!(contract.contains("direct-files-projections/<canonical-graph-path-digest>.sqlite"));
        assert!(contract.contains("sparse_task_query_eligibility"));
        assert!(contract.contains("shared\nproperty-facet rows"));
        assert!(contract.contains("PageRef simple-query candidate plan"));
        assert!(contract.contains("same SQL read family in\nboth storage regimes"));
        assert!(contract.contains("literal fuzzy-search candidate"));
        assert!(contract.contains("referenced-page\ninventory"));
        assert!(contract.contains("retains no separate semantic memo"));
        assert!(contract.contains("exact current parser-cache\ngeneration"));
        assert!(contract.contains("Direct fact-extractor version"));
        assert!(contract.contains("app-private graph-fact projection contains no managed state"));
        assert!(contract.contains("clean\nreopen lowers none"));
        assert!(
            contract.contains("memo of already-shaped frontend result DTOs remains Tine-native")
        );
        assert!(contract.contains("grants no\n   authority"));

        // The routing rule is asserted inside its own section, not anywhere in
        // the document: a whole-document `contains` passes with the sentence
        // parked under an unrelated heading, which is exactly how a contract
        // stops describing the subsystem it claims to describe.
        let heading = "### 1.3 Direct Files disposable graph projection";
        let start = contract.find(heading).expect("Direct projection section");
        let body = &contract[start + heading.len()..];
        let section = body
            .find("\n## ")
            .map_or(body, |end| &body[..end])
            .to_owned();
        // SPEC §5.9's Direct Files route, and the three things a reader has to
        // be able to check without reading the code: which reads a query
        // performs, what a failed read owes and for which named failure, and
        // what a cached result is keyed by.
        for sentence in [
            // Three shapes, and no fourth. The negative clause is pinned too,
            // because a route policy is exactly the kind of sentence that gets
            // softened into "usually".
            "ONE lowered SQL\nstatement answers a simple `{{query ...}}` or advanced datalog query, whatever\nthat query's shape",
            "There is no cost test and no selectivity hatch in front of\nthat decision.",
            "answered by the tree walk over the same query IR, with nothing scheduled",
            // The reads (I-13, I-15).
            "One statement, plus one `Document` load per\npage the RESULT names",
            "Pages loaded equals result pages",
            "the statement therefore carries no `ORDER BY`, and the\ndispatched result equals the walk's result including order",
            "remembered once per generation, never once per query",
            // The failed-read obligation, with its in-scope scenario named.
            "a torn or truncated projection file after a crash or power loss, a disk error, a\nresource limit, or a projection whose page set has drifted from the parsed\ncache",
            "the same\nfull-snapshot enqueue the open path uses is scheduled from the already-parsed\npage cache",
            "Clearing readiness alone would not do",
            "An unavailable, stale, failed, or raced\nprojection uses the parser fallback.",
            // The cache key.
            "memoized PRE-VIEW",
            "under the resolved normalized query IR, the\nparser-cache generation, the execution day, the construction bounds, the\nparse-config digest, and, when the query names a property, the observed-registry\ngeneration",
            "The parse-config digest is unconditional",
            // Managed storage still owns the candidate plan and its cutoff, and
            // the contract says which backend each rule is about.
            "Managed storage still routes a `SimpleQueryCandidatePlan::Indexed` query through\nthe candidate page set the shared lowering returns",
            "larger than one thirty-second of the graph's page\ncount or 32 pages, whichever is greater, in which case the projection read is\nabandoned and the parser fallback runs instead",
            "`Empty` returns without\nprojection or graph access.",
            "`All` uses the parser whole-graph evaluator.",
        ] {
            assert!(
                section.contains(sentence),
                "§1.3 must state the Direct Files query route verbatim: {sentence}"
            );
        }
    }

    #[test]
    #[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
    fn real_corpus_projection_converges_and_matches_task_query() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = PathBuf::from(
            std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
                .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
        );
        let database = scratch("real-corpus").join("projection.sqlite");
        let oracle_graph = Graph::open(&root);
        oracle_graph.warm_cache();
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        let started = Instant::now();
        graph.warm_cache();
        let warm = started.elapsed();
        wait_ready(&graph);
        let converged = started.elapsed();
        let oracle_started = Instant::now();
        let oracle =
            crate::query::run_query_bounded(&oracle_graph, "(task TODO)", 20_000, 32 << 20);
        let oracle_elapsed = oracle_started.elapsed();
        let query_started = Instant::now();
        let indexed = graph.run_query_bounded("(task TODO)", 20_000, 32 << 20);
        let indexed_elapsed = query_started.elapsed();
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        let indexed_reads = graph.direct_projection_indexed_reads_test();
        let memo_started = Instant::now();
        let repeated = graph.run_query_bounded("(task TODO)", 20_000, 32 << 20);
        let memo_elapsed = memo_started.elapsed();
        assert_eq!(signature(&repeated.groups), signature(&oracle.groups));
        assert_eq!(graph.direct_projection_indexed_reads_test(), indexed_reads);
        let mut fuzzy_indexed = Duration::ZERO;
        let mut fuzzy_oracle = Duration::ZERO;
        for value in ["a", "todo", "http", "2026", "%", "_", "é"] {
            let indexed_started = Instant::now();
            let indexed_search = graph.search(value, 5_000);
            fuzzy_indexed += indexed_started.elapsed();
            let oracle_started = Instant::now();
            let oracle_search = crate::query::search(&oracle_graph, value, 5_000);
            fuzzy_oracle += oracle_started.elapsed();
            assert_eq!(
                signature(&indexed_search),
                signature(&oracle_search),
                "real-corpus fuzzy search diverged for a bounded probe"
            );
        }
        eprintln!(
            "direct projection fuzzy receipt: indexed_total_ms={} oracle_total_ms={}",
            fuzzy_indexed.as_millis(),
            fuzzy_oracle.as_millis(),
        );
        let normalize_names = |mut names: Vec<String>| {
            names.sort_by_key(|name| crate::refs::page_key(name));
            names
        };
        assert_eq!(
            normalize_names(graph.referenced_page_names()),
            normalize_names(oracle_graph.referenced_page_names()),
            "real-corpus referenced-page inventory diverged"
        );
        assert!(graph.direct_projection_fuzzy_candidate_reads_test() > 0);
        assert!(graph.direct_projection_referenced_name_reads_test() > 0);
        let task_candidates = PhysicalGraphProjectionDatabase::open_read_only(&database)
            .unwrap()
            .read()
            .task_candidate_blocks_after("TODO", None, 10_000)
            .unwrap()
            .len();
        eprintln!(
            "direct projection receipt: warm_ms={} projection_total_ms={} oracle_query_us={} indexed_query_us={} repeated_query_us={} pages={} task_candidates={}",
            warm.as_millis(),
            converged.as_millis(),
            oracle_elapsed.as_micros(),
            indexed_elapsed.as_micros(),
            memo_elapsed.as_micros(),
            graph.list_pages().len(),
            task_candidates,
        );
    }

    #[test]
    #[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
    fn real_corpus_clean_reopen_reuses_projected_pages() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = PathBuf::from(
            std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
                .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
        );
        let database = scratch("real-corpus-reopen").join("projection.sqlite");
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
        }
        std::thread::sleep(Duration::from_millis(20));

        reset_lowerings();
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        let started = Instant::now();
        graph.warm_cache();
        let warm = started.elapsed();
        wait_ready(&graph);
        let converged = started.elapsed();
        let query_started = Instant::now();
        let indexed = graph.run_query_bounded("(task TODO)", 20_000, 32 << 20);
        let indexed_elapsed = query_started.elapsed();
        let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 20_000, 32 << 20);
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        assert_eq!(
            lowerings(),
            0,
            "clean reopen must not lower unchanged pages"
        );
        eprintln!(
            "direct projection clean-reopen receipt: warm_ms={} projection_total_ms={} projection_tail_ms={} indexed_query_us={} pages_lowered={}",
            warm.as_millis(),
            converged.as_millis(),
            converged.saturating_sub(warm).as_millis(),
            indexed_elapsed.as_micros(),
            lowerings(),
        );
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }

    #[test]
    #[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
    fn real_corpus_reference_family_matches_parser_oracle() {
        let _serial = PROJECTION_TEST_LOCK.lock().unwrap();
        let root = PathBuf::from(
            std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
                .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
        );
        let database = scratch("real-corpus-reference-family").join("projection.sqlite");
        let oracle = Graph::open(&root);
        oracle.warm_cache();
        let aliases = crate::query::page_aliases_with_owners(&oracle);
        let alias_target = aliases.first().map(|(alias, _, _)| alias.clone());
        let oracle_backlinks = alias_target
            .as_deref()
            .map(|target| crate::query::backlinks(&oracle, target));
        let oracle_unlinked_started = Instant::now();
        let oracle_unlinked = alias_target
            .as_deref()
            .map(|target| crate::query::unlinked_refs(&oracle, target));
        let oracle_unlinked_elapsed = oracle_unlinked_started.elapsed();
        let oracle_count_started = Instant::now();
        let oracle_counts = oracle.block_ref_counts().unwrap();
        let oracle_count_elapsed = oracle_count_started.elapsed();
        let block_claim = oracle.with_pages(|pages| {
            pages.iter().find_map(|(_, document)| {
                let mut claim = None;
                fn visit(blocks: &[DocBlock], claim: &mut Option<String>) {
                    for block in blocks {
                        if claim.is_none() {
                            *claim = block.projection().block_refs.first().cloned();
                        }
                        visit(&block.children, claim);
                    }
                }
                visit(&document.roots, &mut claim);
                claim
            })
        });
        let oracle_referrers = block_claim
            .as_deref()
            .map(|claim| crate::query::block_referrers(&oracle, claim));
        let oracle_resolved = block_claim
            .as_deref()
            .and_then(|claim| crate::query::resolve_block(&oracle, claim));

        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(graph.page_aliases_with_owners(), aliases);
        let projected_count_started = Instant::now();
        let projected_counts = graph.block_ref_counts().unwrap();
        let projected_count_elapsed = projected_count_started.elapsed();
        assert_eq!(projected_counts.as_ref(), oracle_counts.as_ref());
        eprintln!(
            "real-corpus-reference counts={} parser_count_us={} sqlite_count_us={}",
            projected_counts.len(),
            oracle_count_elapsed.as_micros(),
            projected_count_elapsed.as_micros(),
        );
        if let Some(target) = alias_target.as_deref() {
            let indexed_unlinked_started = Instant::now();
            let indexed_unlinked = crate::query::unlinked_refs(&graph, target);
            let indexed_unlinked_elapsed = indexed_unlinked_started.elapsed();
            assert_eq!(
                signature(&crate::query::backlinks(&graph, target)),
                signature(oracle_backlinks.as_deref().unwrap())
            );
            assert_eq!(
                signature(&indexed_unlinked),
                signature(oracle_unlinked.as_deref().unwrap())
            );
            let candidates = graph.reference_candidate_pages(
                &[crate::refs::page_key(target)],
                ReferenceKind::Explicit,
            );
            assert!(candidates.indexed);
            eprintln!(
                "real-corpus-reference explicit_candidates={} full_pages={} parser_unlinked_us={} indexed_unlinked_us={}",
                candidates.pages.len(),
                candidates.full_page_count,
                oracle_unlinked_elapsed.as_micros(),
                indexed_unlinked_elapsed.as_micros(),
            );
        }
        if let Some(claim) = block_claim.as_deref() {
            assert_eq!(
                signature(&crate::query::block_referrers(&graph, claim)),
                signature(oracle_referrers.as_deref().unwrap())
            );
            assert_eq!(
                crate::query::resolve_block(&graph, claim)
                    .as_ref()
                    .map(|group| signature(std::slice::from_ref(group))),
                oracle_resolved
                    .as_ref()
                    .map(|group| signature(std::slice::from_ref(group)))
            );
        }
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }

    /// Child half of the two `retired_class_c_*` probes. Emits BOTH retired
    /// class-(c) reports, each with its own planted marker, through the exact
    /// production reporter and the exact error types the call sites hand it.
    #[test]
    #[ignore = "child process for the retired class-(c) stderr probe"]
    fn w4_i5b_projection_failure_marker_child() {
        if std::env::var("TINE_I5B_SET_FLAG").as_deref() == Ok("1") {
            crate::sync_runtime::set_runtime_debug_diagnostics(true);
        }
        // Exactly what `open_projection_database` returns: a free-form
        // `MaterializationError` payload.
        report_projection_failure(
            "disabled: its database could not be opened",
            &tine_storage::sqlite::MaterializationError::Sqlite(
                "planted-open-marker-Zq7Page".to_owned(),
            ),
        );
        // Exactly what `apply_pending` returns: a `String` naming the
        // graph-relative page it was projecting.
        report_projection_failure(
            "is stale; using parser fallback",
            &"parsed page has no exact source revision: pages/planted-apply-marker-Zq7Page.md"
                .to_owned(),
        );
    }

    fn projection_failure_child_stderr(set_flag: &str) -> String {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "direct_projection::tests::w4_i5b_projection_failure_marker_child",
                "--nocapture",
            ])
            .env_remove("TINE_DEBUG")
            .env("TINE_I5B_SET_FLAG", set_flag)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "projection-failure child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// I-5, retired class-(c) row `direct_projection.rs` "projection database
    /// could not be opened": the always-on line carried a free-form
    /// `MaterializationError` payload.
    #[test]
    fn retired_class_c_projection_database_open_emits_no_planted_marker() {
        let marker = "planted-open-marker-Zq7Page";
        assert!(
            !projection_failure_child_stderr("0").contains(marker),
            "I-5: the always-on projection-open failure still carried its error prose. \
             The always-on line names the failure family only; the detail belongs behind \
             `runtime_debug_diagnostics_enabled()` (I-9 keeps the family, not the prose)."
        );
        assert!(
            projection_failure_child_stderr("1").contains(marker),
            "the directed debug channel must still carry the detail, or this probe proves \
             nothing about where the prose went"
        );
    }

    /// I-5, retired class-(c) row `direct_projection.rs` "projection is stale;
    /// using parser fallback": `apply_pending` formats the graph-relative page
    /// path into the error this line used to print always-on.
    #[test]
    fn retired_class_c_projection_apply_failure_emits_no_planted_marker() {
        let marker = "planted-apply-marker-Zq7Page";
        assert!(
            source_of_this_file().contains("parsed page has no exact source revision: {}"),
            "non-vacuity: this probe exists because `apply_pending` names the page it was \
             projecting in its error string. If that error no longer does, re-derive the row's \
             class before relaxing the probe."
        );
        assert!(
            !projection_failure_child_stderr("0").contains(marker),
            "I-5: the always-on parser-fallback line still carried the graph-relative page \
             path from `apply_pending`. The always-on line names the failure family only."
        );
        assert!(
            projection_failure_child_stderr("1").contains(marker),
            "the directed debug channel must still carry the detail, or this probe proves \
             nothing about where the prose went"
        );
    }

    fn source_of_this_file() -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/direct_projection.rs"),
        )
        .unwrap()
    }
}
