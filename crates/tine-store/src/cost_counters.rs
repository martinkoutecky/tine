//! Test-only accounting for the waited save, print and export paths.
use std::sync::atomic::{AtomicU64, Ordering};

/// Source-graph work, keyed by canonical root so concurrent fixture graphs
/// and the publisher's private build graphs cannot contaminate the counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryExportWork {
    /// Store opens for this graph, including cold owner startup.
    pub opens: u64,
}

static QUERY_EXPORT_WORK: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, QueryExportWork>>,
> = std::sync::LazyLock::new(Default::default);

/// Take and reset this source graph's export counters.
pub fn take_query_export_work(root: &std::path::Path) -> QueryExportWork {
    QUERY_EXPORT_WORK
        .lock()
        .unwrap()
        .remove(root)
        .unwrap_or_default()
}

pub(crate) fn source_store_open(root: &std::path::Path) {
    QUERY_EXPORT_WORK
        .lock()
        .unwrap()
        .entry(root.to_owned())
        .or_default()
        .opens += 1;
}

static UUID_BLOCK_VISITS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<String, (u64, u64)>>,
> = std::sync::LazyLock::new(Default::default);

/// Count an actual block visit, keyed by the requested UUID for test isolation.
pub fn query_uuid_block(uuid: &str) {
    UUID_BLOCK_VISITS
        .lock()
        .unwrap()
        .entry(uuid.to_owned())
        .or_default()
        .0 += 1;
}

pub(crate) fn query_uuid_index_key(uuid: &str) {
    UUID_BLOCK_VISITS
        .lock()
        .unwrap()
        .entry(uuid.to_owned())
        .or_default()
        .1 += 1;
}

/// Take and reset (block visits, matching string-index keys) for this selector,
/// including work on owner threads.
pub fn take_query_uuid_blocks(uuid: &str) -> (u64, u64) {
    UUID_BLOCK_VISITS
        .lock()
        .unwrap()
        .remove(uuid)
        .unwrap_or_default()
}

static TRANSACTION_RECORD_PROBES: AtomicU64 = AtomicU64::new(0);
static READDIR: AtomicU64 = AtomicU64::new(0);
static FULL_READS: AtomicU64 = AtomicU64::new(0);
static PREAMBLE_READS: AtomicU64 = AtomicU64::new(0);
static OLD_SOURCE_PARSES: AtomicU64 = AtomicU64::new(0);
static PARSES: AtomicU64 = AtomicU64::new(0);
static CORPUS: AtomicU64 = AtomicU64::new(0);
static NAMESPACE_NAME_VISITS: AtomicU64 = AtomicU64::new(0);
static FSYNCS: AtomicU64 = AtomicU64::new(0);
static BYTES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static FILES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_REBUILDS: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_NANOS: AtomicU64 = AtomicU64::new(0);
static MEMO_PAGE_PROBES: AtomicU64 = AtomicU64::new(0);
static SHARED_TREE_NODE_COPIES: AtomicU64 = AtomicU64::new(0);
static CACHE_PAGE_COPIES: AtomicU64 = AtomicU64::new(0);
static ICON_PAGE_PROBES: AtomicU64 = AtomicU64::new(0);
static ALIAS_PAGE_PROBES: AtomicU64 = AtomicU64::new(0);
static SIGNATURE_BLOCK_PROBES: AtomicU64 = AtomicU64::new(0);
static QUERY_FACTS_COPIES: AtomicU64 = AtomicU64::new(0);
static QUERY_FACTS_DERIVED: AtomicU64 = AtomicU64::new(0);
static QUERY_CARRY_BLOCK_PROBES: AtomicU64 = AtomicU64::new(0);
static QUERY_REGISTRY_PAGES_READ: AtomicU64 = AtomicU64::new(0);
static HASH_READS: AtomicU64 = AtomicU64::new(0);
static STAMPS_BY_PATH: AtomicU64 = AtomicU64::new(0);
static ASSET_STAMPS_BY_PATH: AtomicU64 = AtomicU64::new(0);
static LISTING_STAMPS: AtomicU64 = AtomicU64::new(0);
static ASSET_LISTING_STAMPS: AtomicU64 = AtomicU64::new(0);
static STORE_READS: AtomicU64 = AtomicU64::new(0);
static NAME_INVENTORY_ENTRIES: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_REWRITES: AtomicU64 = AtomicU64::new(0);
static WATCHER_READS: AtomicU64 = AtomicU64::new(0);
static WATCHER_STAMPS_BY_PATH: AtomicU64 = AtomicU64::new(0);
static OBSERVATION_READS: AtomicU64 = AtomicU64::new(0);
static HOST_PUBLICATION_PROBES: AtomicU64 = AtomicU64::new(0);
static HELD_READ_FORWARDS: AtomicU64 = AtomicU64::new(0);
static ASSET_REF_DOCUMENTS: AtomicU64 = AtomicU64::new(0);
static ORPHAN_CHECK_DOCUMENTS: AtomicU64 = AtomicU64::new(0);
static ORPHAN_CHECK_NANOS: AtomicU64 = AtomicU64::new(0);
pub(crate) static HOST_RECONCILE_VISITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static HOST_DRAFT_RECORD_VISITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static HOST_CANDIDATE_VISITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static HOST_WORKER_KEY_VISITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static HOST_BOOK_VISITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static HOST_DEBOUNCED_SAVES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static JOURNAL_WORK: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
    /// Set on the watcher thread, whose re-reads of just-written files are
    /// counted apart from the operation that wrote them (contract §5.4, K5).
    static WATCHER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while the page host processes a watcher-forwarded observation.
    static OBSERVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Calling thread's precise page-lifecycle summary work (path probes).
    static LIFECYCLE_WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Calling thread's precise page-lifecycle summary work: one per changed
/// path read on both sides, per named path checked against them, and per
/// fileless name looked up on both sides (D-20261102-PERF row 3, review F2).
pub fn lifecycle_work() -> u64 {
    LIFECYCLE_WORK.with(std::cell::Cell::get)
}
pub(crate) fn lifecycle_probe() {
    LIFECYCLE_WORK.with(|work| work.set(work.get() + 1));
}

/// Calling thread's journal day visits, inventory sorts, and reference-name visits.
pub fn journal_work() -> (usize, usize, usize) {
    JOURNAL_WORK.with(std::cell::Cell::get)
}
pub(crate) fn journal_day_visit() {
    JOURNAL_WORK.with(|work| {
        let (days, sorts, refs) = work.get();
        work.set((days + 1, sorts, refs));
    });
}
pub(crate) fn inventory_sort() {
    JOURNAL_WORK.with(|work| {
        let (days, sorts, refs) = work.get();
        work.set((days, sorts + 1, refs));
    });
}
pub(crate) fn journal_reference_visit() {
    JOURNAL_WORK.with(|work| {
        let (days, sorts, refs) = work.get();
        work.set((days, sorts, refs + 1));
    });
}

/// Run `observe`, the page host's processing of an observation the watcher
/// forwarded (the echo of a held page's change, often its own save): its
/// whole-file reads go to `observation_reads`, billed by cause rather than
/// by thread (contract §5.4, O5).
pub(crate) fn observing<R>(observe: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            OBSERVING.with(|observing| observing.set(false));
        }
    }
    OBSERVING.with(|observing| observing.set(true));
    let _reset = Reset;
    observe()
}

/// Mark the current thread as the watcher's: its file reads (whole, preamble
/// and hash) and by-path stamps go to `watcher_reads` / `watcher_stamps_by_path`.
pub(crate) fn mark_watcher_thread() {
    WATCHER.with(|watcher| watcher.set(true));
}

/// Whether the current thread is the watcher's. Safe inside a global
/// allocator (no allocation; false while thread-locals are torn down), so a
/// cost test can bill the watcher's settlement of a write apart from the
/// operation that wrote it (K5), as the read counters do.
pub fn on_watcher() -> bool {
    WATCHER.try_with(|watcher| watcher.get()).unwrap_or(false)
}

/// Count a file read on the watcher thread; false elsewhere.
fn watcher_read() -> bool {
    let watcher = on_watcher();
    if watcher {
        WATCHER_READS.fetch_add(1, Ordering::Relaxed);
    }
    watcher
}

/// Primitive counts since the last reset. The fixture uses one process per case.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    /// Transaction plan/undo records inspected for final per-file publication.
    pub transaction_record_probes: u64,
    /// Directory enumerations.
    pub readdir: u64,
    /// Complete page file reads.
    pub full_reads: u64,
    /// Page files opened to discover their effective name.
    pub preamble_reads: u64,
    /// Document and outline parser calls made by the store save/read family,
    /// including formatting detection and layout heading checks. Core lazy
    /// block projections are outside this counter's boundary.
    pub parses: u64,
    /// Old-source document parses during save preparation.
    pub old_source_parses: u64,
    /// Owned corpus constructions.
    pub corpus: u64,
    /// Stored name claimants visited by namespace prefix lookups.
    pub namespace_name_visits: u64,
    /// File and parent directory sync calls.
    pub fsyncs: u64,
    /// Payload bytes written to temporary files.
    pub bytes_written: u64,
    /// Temporary payload files written.
    pub files_written: u64,
    /// Full reference-signature table rebuilds during snapshot capture.
    pub snapshot_rebuilds: u64,
    /// Time inside ReadSnapshot capture, for diagnostic measurement only.
    pub snapshot_nanos: u64,
    /// Page slots inspected while carrying query memos across a save.
    pub memo_page_probes: u64,
    /// Shared collection nodes copied along edited tree paths.
    pub shared_tree_node_copies: u64,
    /// Changed page values copied by cache mutation.
    pub cache_page_copies: u64,
    /// Page slots inspected while answering icon requests.
    pub icon_page_probes: u64,
    /// Pages examined while building or patching the alias relation: every
    /// page for a walk (cold build, alias-spelling listing), the changed
    /// pages only for a per-edit delta (D-20261018-M2c).
    pub alias_page_probes: u64,
    /// Blocks inspected while constructing changed-page reference signatures.
    pub signature_block_probes: u64,
    /// Query-index facts entries copied when a generation is patched.
    pub query_facts_copies: u64,
    /// Pages whose query facts were derived (index build, patch, memo carry).
    pub query_facts_derived: u64,
    /// Blocks evaluated while carrying query answers across an edit.
    pub query_carry_block_probes: u64,
    /// Page documents walked for property rows while building or patching a
    /// query registry.
    pub query_registry_pages_read: u64,
    /// Whole graph-text files read only to hash their bytes
    /// (`FileRev::from_file`; `logseq/config.edn` is not counted).
    pub hash_reads: u64,
    /// Graph-text files (`.md`/`.org`) stamped by path
    /// (`watch::stamp_metadata`, a per-file `symlink_metadata`, which opens
    /// the file on Windows) rather than from a directory listing's entries.
    pub stamps_by_path: u64,
    /// Non-graph-text (asset) files stamped by path (`watch::stamp_metadata`),
    /// which opens the file on Windows; the asset full walk uses the listing.
    pub asset_stamps_by_path: u64,
    /// Graph-text files a full stat walk stamped from its directory listing
    /// off the watcher thread (a `refresh`): a graph-sized term an
    /// operation pays itself. The watcher's own cycles are not counted.
    pub listing_stamps: u64,
    /// Asset files a full asset walk stamped off the watcher thread.
    pub asset_listing_stamps: u64,
    /// Whole-file reads made through `Store::read` (the feature crates' bounded
    /// file read: conflict markers, sync copies), which the load pass's own
    /// `full_reads` does not include.
    pub store_reads: u64,
    /// Entries built by the whole-graph name inventory
    /// (`WholeGraph::inventory`: every claimed, alias and reference-only
    /// name), which grows with the graph rather than with the operation.
    pub name_inventory_entries: u64,
    /// Reference rewrites computed inside a transaction (preflight, under the
    /// writer and page locks) rather than handed over prepared.
    pub transaction_rewrites: u64,
    /// `full_reads`, `preamble_reads` and `hash_reads` made on the watcher
    /// thread: the settle re-reads of files an operation just wrote (§5.4),
    /// off the operation's own path.
    pub watcher_reads: u64,
    /// `stamps_by_path` made on the watcher thread.
    pub watcher_stamps_by_path: u64,
    /// `full_reads` the page host made processing a watcher-forwarded
    /// observation: its re-read of a held page the watcher saw change, the
    /// echo of its own save included (§5.4, O5), off the operation's path.
    pub observation_reads: u64,
    /// The page host's publication bookkeeping (REVIEW-3b-P2b R5): reply
    /// slots and rename orders examined when a page publishes
    /// (`Order::published`), answered pages examined while a caller waits for
    /// their witness (`settle`), and index publications (`Bridge::deliver`).
    pub host_publication_probes: u64,
    /// Held pages' reads that asked their owner for an observation: the
    /// file's stamp moved since its owner's last indexed one (A-H1).
    pub held_read_forwards: u64,
    /// Page-host step work by primitive (M4, REVIEW-progress-plan R5), one
    /// count per element examined, rejected filter elements included:
    /// keys `Progress::reconcile` visits.
    pub host_reconcile_visits: u64,
    /// Draft records and vehicles examined to build or look up a logical
    /// draft, list a page's vehicles or tidy a worker's copies.
    pub host_draft_record_visits: u64,
    /// Save, draft, retire and deadline candidates examined by `poll` and
    /// `next_deadline`, queued requests examined per candidate, and owed
    /// observations examined by the driver.
    pub host_candidate_visits: u64,
    /// Keys copied, checked or locked for the draft worker's phases and the
    /// driver's lock acquisition and revalidation.
    pub host_worker_key_visits: u64,
    /// Page documents (indexed or set-aside) scanned for asset references.
    pub asset_ref_documents: u64,
    /// `asset_ref_documents` scanned inside the orphan-asset check, which
    /// holds the Store writer (a save issued meanwhile waits for it).
    pub orphan_check_documents: u64,
    /// Time inside the orphan-asset check under the writer, for diagnostic
    /// measurement only.
    pub orphan_check_nanos: u64,
    /// Keys the binding's `Book::collect` examines: claims, eviction
    /// candidates (with their queued requests and pending publications) and
    /// subscribed pages' notices.
    pub host_book_visits: u64,
    /// Page saves the host started at the edit debounce's deadline (the
    /// page idle for 400 ms, or dirty for 1 s), not at once: an operation's
    /// or a barrier's saves skip it (`progress.rs` `Timing::deadline`).
    pub host_debounced_saves: u64,
}

/// Zero process-global counters. Concurrent activity contaminates measurements.
/// O(number of counters), using relaxed atomic stores.
pub fn reset() {
    for counter in [
        &TRANSACTION_RECORD_PROBES,
        &READDIR,
        &FULL_READS,
        &PREAMBLE_READS,
        &PARSES,
        &OLD_SOURCE_PARSES,
        &CORPUS,
        &NAMESPACE_NAME_VISITS,
        &FSYNCS,
        &BYTES_WRITTEN,
        &FILES_WRITTEN,
        &SNAPSHOT_REBUILDS,
        &SNAPSHOT_NANOS,
        &MEMO_PAGE_PROBES,
        &CACHE_PAGE_COPIES,
        &SHARED_TREE_NODE_COPIES,
        &ICON_PAGE_PROBES,
        &ALIAS_PAGE_PROBES,
        &SIGNATURE_BLOCK_PROBES,
        &QUERY_FACTS_COPIES,
        &QUERY_FACTS_DERIVED,
        &QUERY_CARRY_BLOCK_PROBES,
        &QUERY_REGISTRY_PAGES_READ,
        &HASH_READS,
        &STAMPS_BY_PATH,
        &ASSET_STAMPS_BY_PATH,
        &LISTING_STAMPS,
        &ASSET_LISTING_STAMPS,
        &STORE_READS,
        &NAME_INVENTORY_ENTRIES,
        &TRANSACTION_REWRITES,
        &WATCHER_READS,
        &WATCHER_STAMPS_BY_PATH,
        &OBSERVATION_READS,
        &HOST_PUBLICATION_PROBES,
        &HELD_READ_FORWARDS,
        &HOST_RECONCILE_VISITS,
        &HOST_DRAFT_RECORD_VISITS,
        &HOST_CANDIDATE_VISITS,
        &HOST_WORKER_KEY_VISITS,
        &HOST_BOOK_VISITS,
        &HOST_DEBOUNCED_SAVES,
        &ASSET_REF_DOCUMENTS,
        &ORPHAN_CHECK_DOCUMENTS,
        &ORPHAN_CHECK_NANOS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

/// Read process-global counters without resetting. This is not an atomic
/// multi-counter snapshot; concurrent activity may mix intervals. O(counters).
pub fn snapshot() -> Counts {
    Counts {
        transaction_record_probes: TRANSACTION_RECORD_PROBES.load(Ordering::Relaxed),
        readdir: READDIR.load(Ordering::Relaxed),
        full_reads: FULL_READS.load(Ordering::Relaxed),
        preamble_reads: PREAMBLE_READS.load(Ordering::Relaxed),
        parses: PARSES.load(Ordering::Relaxed),
        old_source_parses: OLD_SOURCE_PARSES.load(Ordering::Relaxed),
        corpus: CORPUS.load(Ordering::Relaxed),
        namespace_name_visits: NAMESPACE_NAME_VISITS.load(Ordering::Relaxed),
        fsyncs: FSYNCS.load(Ordering::Relaxed),
        bytes_written: BYTES_WRITTEN.load(Ordering::Relaxed),
        files_written: FILES_WRITTEN.load(Ordering::Relaxed),
        snapshot_rebuilds: SNAPSHOT_REBUILDS.load(Ordering::Relaxed),
        snapshot_nanos: SNAPSHOT_NANOS.load(Ordering::Relaxed),
        memo_page_probes: MEMO_PAGE_PROBES.load(Ordering::Relaxed),
        cache_page_copies: CACHE_PAGE_COPIES.load(Ordering::Relaxed),
        shared_tree_node_copies: SHARED_TREE_NODE_COPIES.load(Ordering::Relaxed),
        icon_page_probes: ICON_PAGE_PROBES.load(Ordering::Relaxed),
        alias_page_probes: ALIAS_PAGE_PROBES.load(Ordering::Relaxed),
        signature_block_probes: SIGNATURE_BLOCK_PROBES.load(Ordering::Relaxed),
        query_facts_copies: QUERY_FACTS_COPIES.load(Ordering::Relaxed),
        query_facts_derived: QUERY_FACTS_DERIVED.load(Ordering::Relaxed),
        query_carry_block_probes: QUERY_CARRY_BLOCK_PROBES.load(Ordering::Relaxed),
        query_registry_pages_read: QUERY_REGISTRY_PAGES_READ.load(Ordering::Relaxed),
        hash_reads: HASH_READS.load(Ordering::Relaxed),
        stamps_by_path: STAMPS_BY_PATH.load(Ordering::Relaxed),
        asset_stamps_by_path: ASSET_STAMPS_BY_PATH.load(Ordering::Relaxed),
        listing_stamps: LISTING_STAMPS.load(Ordering::Relaxed),
        asset_listing_stamps: ASSET_LISTING_STAMPS.load(Ordering::Relaxed),
        store_reads: STORE_READS.load(Ordering::Relaxed),
        name_inventory_entries: NAME_INVENTORY_ENTRIES.load(Ordering::Relaxed),
        transaction_rewrites: TRANSACTION_REWRITES.load(Ordering::Relaxed),
        watcher_reads: WATCHER_READS.load(Ordering::Relaxed),
        watcher_stamps_by_path: WATCHER_STAMPS_BY_PATH.load(Ordering::Relaxed),
        observation_reads: OBSERVATION_READS.load(Ordering::Relaxed),
        host_publication_probes: HOST_PUBLICATION_PROBES.load(Ordering::Relaxed),
        held_read_forwards: HELD_READ_FORWARDS.load(Ordering::Relaxed),
        host_reconcile_visits: HOST_RECONCILE_VISITS.load(Ordering::Relaxed),
        host_draft_record_visits: HOST_DRAFT_RECORD_VISITS.load(Ordering::Relaxed),
        host_candidate_visits: HOST_CANDIDATE_VISITS.load(Ordering::Relaxed),
        host_worker_key_visits: HOST_WORKER_KEY_VISITS.load(Ordering::Relaxed),
        host_book_visits: HOST_BOOK_VISITS.load(Ordering::Relaxed),
        host_debounced_saves: HOST_DEBOUNCED_SAVES.load(Ordering::Relaxed),
        asset_ref_documents: ASSET_REF_DOCUMENTS.load(Ordering::Relaxed),
        orphan_check_documents: ORPHAN_CHECK_DOCUMENTS.load(Ordering::Relaxed),
        orphan_check_nanos: ORPHAN_CHECK_NANOS.load(Ordering::Relaxed),
    }
}

pub(crate) fn asset_ref_document() {
    ASSET_REF_DOCUMENTS.fetch_add(1, Ordering::Relaxed);
}
/// Bill the asset-reference scans and time of one orphan check made under
/// the writer: `scan` returns its result, and the documents it scanned are
/// the counter's growth meanwhile (one caller at a time in cost tests).
pub(crate) fn orphan_check<R>(scan: impl FnOnce() -> R) -> R {
    let before = ASSET_REF_DOCUMENTS.load(Ordering::Relaxed);
    let started = std::time::Instant::now();
    let result = scan();
    let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
    ORPHAN_CHECK_NANOS.fetch_add(nanos, Ordering::Relaxed);
    let after = ASSET_REF_DOCUMENTS.load(Ordering::Relaxed);
    ORPHAN_CHECK_DOCUMENTS.fetch_add(after.saturating_sub(before), Ordering::Relaxed);
    result
}

pub(crate) fn held_read_forward() {
    HELD_READ_FORWARDS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn host_publication_probes(count: usize) {
    HOST_PUBLICATION_PROBES.fetch_add(count as u64, Ordering::Relaxed);
}

pub(crate) fn name_inventory_entries(count: usize) {
    NAME_INVENTORY_ENTRIES.fetch_add(count as u64, Ordering::Relaxed);
}
pub(crate) fn transaction_rewrite() {
    TRANSACTION_REWRITES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn store_read() {
    STORE_READS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn asset_stamp_by_path() {
    ASSET_STAMPS_BY_PATH.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn listing_stamp() {
    if !on_watcher() {
        LISTING_STAMPS.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn asset_listing_stamp() {
    if !on_watcher() {
        ASSET_LISTING_STAMPS.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn stamp_by_path() {
    if on_watcher() {
        WATCHER_STAMPS_BY_PATH.fetch_add(1, Ordering::Relaxed);
        return;
    }
    STAMPS_BY_PATH.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn readdir() {
    READDIR.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn full_read() {
    if watcher_read() {
        return;
    }
    if OBSERVING.with(|observing| observing.get()) {
        OBSERVATION_READS.fetch_add(1, Ordering::Relaxed);
    } else {
        FULL_READS.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn preamble_read() {
    if !watcher_read() {
        PREAMBLE_READS.fetch_add(1, Ordering::Relaxed);
    }
    let delay = PREAMBLE_OPEN_DELAY_US.load(Ordering::Relaxed);
    if delay > 0 {
        std::thread::sleep(std::time::Duration::from_micros(delay));
    }
}

static PREAMBLE_OPEN_DELAY_US: AtomicU64 = AtomicU64::new(0);

/// Make every page-preamble open take `micros` longer, as on Windows where
/// an antivirus scanner holds each first open for ~250 µs (GH #623 BR3).
/// Not cleared by [`reset`].
pub fn set_preamble_open_delay(micros: u64) {
    PREAMBLE_OPEN_DELAY_US.store(micros, Ordering::Relaxed);
}
pub(crate) fn parse() {
    PARSES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn old_source_parse() {
    OLD_SOURCE_PARSES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn corpus() {
    CORPUS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn namespace_name_visit() {
    NAMESPACE_NAME_VISITS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn fsync() {
    FSYNCS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn wrote(bytes: usize) {
    BYTES_WRITTEN.fetch_add(bytes as u64, Ordering::Relaxed);
    FILES_WRITTEN.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn snapshot_rebuild() {
    SNAPSHOT_REBUILDS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn snapshot_elapsed(duration: std::time::Duration) {
    SNAPSHOT_NANOS.fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
}
/// Add count to the process-global relaxed-atomic probe total.
pub(crate) fn memo_page_probes(count: u64) {
    MEMO_PAGE_PROBES.fetch_add(count, Ordering::Relaxed);
}
pub(crate) fn shared_tree_node_copy() {
    SHARED_TREE_NODE_COPIES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn cache_page_copies(count: u64) {
    CACHE_PAGE_COPIES.fetch_add(count, Ordering::Relaxed);
}
pub(crate) fn icon_page_probe() {
    ICON_PAGE_PROBES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn alias_page_probe() {
    ALIAS_PAGE_PROBES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn signature_block_probe() {
    SIGNATURE_BLOCK_PROBES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn query_facts_copies(count: u64) {
    QUERY_FACTS_COPIES.fetch_add(count, Ordering::Relaxed);
}
pub(crate) fn query_facts_derived() {
    QUERY_FACTS_DERIVED.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn query_carry_block_probe() {
    QUERY_CARRY_BLOCK_PROBES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn query_registry_pages_read() {
    QUERY_REGISTRY_PAGES_READ.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn hash_read() {
    if !watcher_read() {
        HASH_READS.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(feature = "test-faults")]
pub(crate) fn transaction_record_probe() {
    TRANSACTION_RECORD_PROBES.fetch_add(1, Ordering::Relaxed);
}
