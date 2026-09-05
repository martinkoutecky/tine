use crate::doc::{property_key_norm, DocBlock, Document};
use crate::model::{Format, PageEntry, PageKind, ReferenceKind};
use crate::oplog::query_lowering::drain_after;
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
    PhysicalProperty, PhysicalReferencePosting, PhysicalReferenceTarget, PhysicalTask,
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

#[derive(Clone)]
enum PageDelta {
    Replace(PageEntry, Arc<Document>, String),
    Delete(PageEntry),
}

#[derive(Default)]
struct PendingProjection {
    full: Option<(u64, PageSnapshot, PageRevisions)>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
    latest_generation: u64,
    stop: bool,
}

struct ProjectionShared {
    path: PathBuf,
    pending: Mutex<PendingProjection>,
    changed: Condvar,
    ready: AtomicBool,
    ready_generation: AtomicU64,
    reader: Mutex<Option<PhysicalGraphProjectionDatabase>>,
    worker_available: AtomicBool,
    worker_failed: AtomicBool,
    worker_busy: AtomicBool,
    #[cfg(test)]
    indexed_reads: AtomicU64,
    #[cfg(test)]
    fallback_reads: AtomicU64,
    #[cfg(test)]
    referenced_name_reads: AtomicU64,
    #[cfg(test)]
    fuzzy_candidate_reads: AtomicU64,
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
            worker_available: AtomicBool::new(true),
            worker_failed: AtomicBool::new(false),
            worker_busy: AtomicBool::new(false),
            #[cfg(test)]
            indexed_reads: AtomicU64::new(0),
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

    pub(crate) fn enqueue_full(
        &self,
        generation: u64,
        pages: PageSnapshot,
        revisions: PageRevisions,
    ) {
        self.shared.ready.store(false, Ordering::Release);
        self.shared.worker_failed.store(false, Ordering::Release);
        let mut pending = self.shared.pending.lock().unwrap();
        pending.full = Some((generation, pages, revisions));
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
    ) {
        self.enqueue_delta(generation, PageDelta::Replace(entry, document, revision));
    }

    pub(crate) fn enqueue_delete(&self, generation: u64, entry: PageEntry) {
        self.enqueue_delta(generation, PageDelta::Delete(entry));
    }

    fn enqueue_delta(&self, generation: u64, delta: PageDelta) {
        self.shared.ready.store(false, Ordering::Release);
        let key = match &delta {
            PageDelta::Replace(entry, _, _) | PageDelta::Delete(entry) => entry.rel_path.clone(),
        };
        let mut pending = self.shared.pending.lock().unwrap();
        pending.deltas.insert(key, (generation, delta));
        pending.latest_generation = pending.latest_generation.max(generation);
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
    let mut database = match open_projection_database(&shared.path) {
        Ok(database) => database,
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
        let (full, deltas, latest_generation) = {
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
            (
                pending.full.take(),
                std::mem::take(&mut pending.deltas),
                pending.latest_generation,
            )
        };
        let had_full = full.is_some();
        #[cfg(test)]
        run_before_apply_pending_hook();
        let applied = if requires_full_rebuild && !had_full {
            Err("a prior projection failure requires a complete parser snapshot".into())
        } else {
            apply_pending(&mut database, full, deltas)
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
        if pending.full.is_none()
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
    full: Option<(u64, PageSnapshot, PageRevisions)>,
    deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<(), String> {
    if let Some((_, pages, revisions)) = full {
        let sources = pages
            .iter()
            .map(|(entry, _)| {
                Ok(PhysicalGraphProjectionSourceRevision {
                    page_id: page_id(&entry.rel_path),
                    revision: projection_source_revision(revisions.get(&entry.path).ok_or_else(
                        || {
                            format!(
                                "parsed page has no exact source revision: {}",
                                entry.rel_path
                            )
                        },
                    )?),
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
        let lowered = pages
            .iter()
            .filter(|(entry, _)| replacements_needed.contains(&page_id(&entry.rel_path)))
            .map(|(entry, document)| physical_page(entry, document))
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
            .apply_with_source_revisions_and_aliases(
                &PhysicalGraphProjectionChange {
                    replacements,
                    deletions: source_delta.deletions,
                    reference_postings,
                },
                &replacement_sources,
                &aliases,
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
                PageDelta::Replace(entry, document, revision) => {
                    replacement_sources.push(PhysicalGraphProjectionSourceRevision {
                        page_id: page_id(&entry.rel_path),
                        revision: projection_source_revision(&revision),
                    });
                    let (page, mut postings, mut page_aliases) = physical_page(&entry, &document)?;
                    replacements.push(page);
                    reference_postings.append(&mut postings);
                    aliases.append(&mut page_aliases);
                }
                PageDelta::Delete(entry) => deletions.push(page_id(&entry.rel_path)),
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

fn projection_source_revision(content_revision: &str) -> String {
    format!("direct-facts-v{DIRECT_PROJECTION_FACTS_VERSION}:{content_revision}")
}

fn physical_page(
    entry: &PageEntry,
    document: &Document,
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
    let is_org = Format::from_path(Path::new(&entry.rel_path)) == Format::Org;
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
    lower_blocks(
        &document.roots,
        id,
        None,
        &mut Vec::new(),
        &mut blocks,
        &mut reference_postings,
    )?;
    Ok((
        PhysicalPage {
            page_id: id,
            home_document_id: id,
            name: entry.name.clone(),
            name_key: crate::refs::page_key(&entry.name),
            path: entry.rel_path.clone(),
            text_kind: page_kind_to_sql(entry.kind),
            preamble: document.pre_block.clone(),
            normalized_searchable_text: searchable_text.to_lowercase().nfc().collect(),
            searchable_text,
            references: Vec::new(),
            properties,
            tags,
            blocks,
        },
        reference_postings,
        aliases,
    ))
}

fn lower_blocks(
    source: &[DocBlock],
    page_id: [u8; 16],
    parent: Option<[u8; 16]>,
    structural_path: &mut Vec<u32>,
    out: &mut Vec<PhysicalBlock>,
    reference_postings: &mut Vec<PhysicalReferencePosting>,
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
        let properties = projection
            .properties
            .iter()
            .map(|(name, value)| PhysicalProperty {
                name: name.clone(),
                normalized_name: property_key_norm(name),
                value: value.clone(),
            })
            .collect();
        let logseq_uuid = block
            .property("id")
            .and_then(|value| Uuid::parse_str(value.trim()).ok())
            .map(Uuid::into_bytes);
        out.push(PhysicalBlock {
            block_id,
            home_document_id: page_id,
            parent,
            order,
            content: block.raw.clone(),
            normalized_searchable_text: searchable_text.to_lowercase().nfc().collect(),
            searchable_text,
            heading_level: projection.heading_level,
            collapsed: block.collapsed(),
            logseq_uuid,
            logseq_identity_origin: logseq_uuid.map(|_| 0),
            references: Vec::new(),
            properties,
            tags: projection.tags.clone(),
            task: projection.marker.as_ref().map(|marker| PhysicalTask {
                marker: marker.to_ascii_uppercase(),
                priority: projection.priority.clone(),
                scheduled: projection.scheduled.clone(),
                deadline: projection.deadline.clone(),
            }),
        });
        lower_blocks(
            &block.children,
            page_id,
            Some(block_id),
            structural_path,
            out,
            reference_postings,
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

        for query in [
            "(and (task TODO) (page source))",
            "(property status active)",
            "(page-property category work)",
            "(page source)",
            "(namespace Project)",
            "(journal)",
            "(and (property status active) (page source))",
            "(or (page source) (page Target))",
        ] {
            let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
            let expected_paths = graph
                .direct_projection_candidate_paths_test(
                    &crate::query::simple_query_candidate_plan(query),
                    usize::MAX,
                )
                .unwrap();
            let indexed_before = graph.direct_projection_indexed_reads_test();
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
            assert_eq!(graph.direct_projection_indexed_reads_test(), indexed_before + 1, "{query}: exactly one candidate query must complete; expected candidate paths {expected_paths:?}");
            assert_eq!(
                crate::query::full_graph_query_evaluations(),
                0,
                "{query}: production invocation entered the forbidden full-graph evaluator"
            );
            assert_eq!(
                graph.direct_projection_fallback_reads_test(),
                fallback_before,
                "{query}: ready candidate route fell back"
            );
            assert_eq!(
                graph
                    .direct_projection_candidate_evaluated_paths_test()
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected_paths,
                "{query}: production must evaluate exactly the lowering's candidate paths"
            );
        }

        let indexed_before = graph.direct_projection_indexed_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let empty = graph.run_query_bounded("(", 100, 1_000_000);
        assert!(empty.groups.is_empty());
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "Plan::Empty must not enter the graph evaluator"
        );
        assert_eq!(
            graph.direct_projection_indexed_reads_test(),
            indexed_before,
            "Plan::Empty must not touch the projection"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "Plan::Empty must not record fallback access"
        );

        graph.reset_direct_projection_candidate_probe_test();
        let _ = graph.run_query_bounded("\"points\"", 100, 1_000_000);
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            1,
            "Plan::All alone uses the parser whole-graph evaluator"
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

    /// An `Indexed` plan whose candidate set is a large fraction of the graph
    /// abandons the projection and hands the query back to the parser walk.
    ///
    /// The user outcome this protects: on a real graph, `(journal)` and
    /// non-sparse `(and (task ...) ...)` name most of the pages, and
    /// materializing every one of them through SQLite made those query blocks
    /// 7x and 10x SLOWER than the walk they replaced. The hatch is what keeps a
    /// query block from stalling typing on the very shapes the route cannot
    /// help. It must fire on the unselective shape and must NOT fire on a
    /// selective one in the same graph.
    #[test]
    fn b4_unselective_candidate_set_abandons_the_projection_for_the_parser_walk() {
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

        // The lowering itself still produces the whole unselective candidate
        // set; the hatch is a routing decision, not a change to the lowering.
        let unselective = "(journal)";
        let raw = graph
            .direct_projection_candidate_paths_test(
                &crate::query::simple_query_candidate_plan(unselective),
                usize::MAX,
            )
            .expect("the lowering answers the unselective plan");
        assert!(
            raw.len() > 32,
            "fixture must exceed the cutoff; got {} candidates",
            raw.len()
        );

        // Run the oracle BEFORE resetting the probes, so the oracle's own walk
        // is not counted as the production invocation's route evidence.
        let oracle = crate::query::run_query_bounded(&graph, unselective, 500, 4_000_000);
        let indexed_before = graph.direct_projection_indexed_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let abandoned = graph.run_query_bounded(unselective, 500, 4_000_000);

        assert_eq!(
            signature(&abandoned.groups),
            signature(&oracle.groups),
            "abandoning must not change the answer"
        );
        assert_eq!(
            (abandoned.total, abandoned.exceeded),
            (oracle.total, oracle.exceeded),
            "abandoning must not change the bound outcome"
        );
        assert_eq!(
            graph.direct_projection_indexed_reads_test(),
            indexed_before,
            "an abandoned plan must complete no candidate query"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before + 1,
            "abandoning must record exactly one fallback read on the existing hatch"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            1,
            "an abandoned plan takes the parser whole-graph walk exactly once"
        );
        assert!(
            graph
                .direct_projection_candidate_evaluated_paths_test()
                .is_empty(),
            "an abandoned plan must materialize no candidate pages"
        );

        // Same graph, same readiness: a selective plan still routes.
        let selective = "(page-ref Target)";
        let selective_paths = graph
            .direct_projection_candidate_paths_test(
                &crate::query::simple_query_candidate_plan(selective),
                usize::MAX,
            )
            .expect("the lowering answers the selective plan");
        assert!(selective_paths.len() <= 32, "selective fixture drifted");
        let selective_oracle = crate::query::run_query_bounded(&graph, selective, 500, 4_000_000);
        let indexed_before = graph.direct_projection_indexed_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let routed = graph.run_query_bounded(selective, 500, 4_000_000);
        assert_eq!(
            signature(&routed.groups),
            signature(&selective_oracle.groups)
        );
        assert_eq!(
            graph.direct_projection_indexed_reads_test(),
            indexed_before + 1,
            "a selective plan must still complete exactly one candidate query"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "a selective plan must not fall back"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "a selective plan must not enter the full-graph evaluator"
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
        let projected = projection_source_revision(source);
        assert_eq!(projected, "direct-facts-v2:sha256:unchanged-source");
        assert_ne!(projected, source);
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
        for sentence in [
            "every\n`SimpleQueryCandidatePlan::Indexed` plan obtains its candidate page set from the\nshared lowering and evaluates only those pages",
            "larger than one thirty-second of the graph's page count or 32 pages, whichever\nis greater, in which case the projection read is abandoned and the parser\nfallback runs instead",
            "`Empty` returns without projection or graph access.",
            "`All`\nuses the parser whole-graph evaluator.",
            "An unavailable, stale, failed, or raced\nprojection uses the parser fallback.",
        ] {
            assert!(
                section.contains(sentence),
                "§1.3 must state the Indexed routing rule verbatim: {sentence}"
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
