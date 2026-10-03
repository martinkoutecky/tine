//! Test-only accounting for the waited save and print paths.
use std::sync::atomic::{AtomicU64, Ordering};

static READDIR: AtomicU64 = AtomicU64::new(0);
static FULL_READS: AtomicU64 = AtomicU64::new(0);
static PREAMBLE_READS: AtomicU64 = AtomicU64::new(0);
static OLD_SOURCE_PARSES: AtomicU64 = AtomicU64::new(0);
static PARSES: AtomicU64 = AtomicU64::new(0);
static CORPUS: AtomicU64 = AtomicU64::new(0);
static FSYNCS: AtomicU64 = AtomicU64::new(0);
static BYTES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static FILES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_REBUILDS: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_NANOS: AtomicU64 = AtomicU64::new(0);
static MEMO_PAGE_PROBES: AtomicU64 = AtomicU64::new(0);
static SHARED_TREE_NODE_COPIES: AtomicU64 = AtomicU64::new(0);
static CACHE_PAGE_COPIES: AtomicU64 = AtomicU64::new(0);
static ICON_PAGE_PROBES: AtomicU64 = AtomicU64::new(0);
static SIGNATURE_BLOCK_PROBES: AtomicU64 = AtomicU64::new(0);
static QUERY_FACTS_COPIES: AtomicU64 = AtomicU64::new(0);
static QUERY_FACTS_DERIVED: AtomicU64 = AtomicU64::new(0);
static QUERY_CARRY_BLOCK_PROBES: AtomicU64 = AtomicU64::new(0);
static QUERY_REGISTRY_PAGES_READ: AtomicU64 = AtomicU64::new(0);
static HASH_READS: AtomicU64 = AtomicU64::new(0);
static STAMPS_BY_PATH: AtomicU64 = AtomicU64::new(0);
static STORE_READS: AtomicU64 = AtomicU64::new(0);

/// Primitive counts since the last reset. The fixture uses one process per case.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
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
    /// Whole-file reads made through `Store::read` (the feature crates' bounded
    /// file read: conflict markers, sync copies), which the load pass's own
    /// `full_reads` does not include.
    pub store_reads: u64,
}

/// Zero process-global counters. Concurrent activity contaminates measurements.
/// O(number of counters), using relaxed atomic stores.
pub fn reset() {
    for counter in [
        &READDIR,
        &FULL_READS,
        &PREAMBLE_READS,
        &PARSES,
        &OLD_SOURCE_PARSES,
        &CORPUS,
        &FSYNCS,
        &BYTES_WRITTEN,
        &FILES_WRITTEN,
        &SNAPSHOT_REBUILDS,
        &SNAPSHOT_NANOS,
        &MEMO_PAGE_PROBES,
        &CACHE_PAGE_COPIES,
        &SHARED_TREE_NODE_COPIES,
        &ICON_PAGE_PROBES,
        &SIGNATURE_BLOCK_PROBES,
        &QUERY_FACTS_COPIES,
        &QUERY_FACTS_DERIVED,
        &QUERY_CARRY_BLOCK_PROBES,
        &QUERY_REGISTRY_PAGES_READ,
        &HASH_READS,
        &STAMPS_BY_PATH,
        &STORE_READS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

/// Read process-global counters without resetting. This is not an atomic
/// multi-counter snapshot; concurrent activity may mix intervals. O(counters).
pub fn snapshot() -> Counts {
    Counts {
        readdir: READDIR.load(Ordering::Relaxed),
        full_reads: FULL_READS.load(Ordering::Relaxed),
        preamble_reads: PREAMBLE_READS.load(Ordering::Relaxed),
        parses: PARSES.load(Ordering::Relaxed),
        old_source_parses: OLD_SOURCE_PARSES.load(Ordering::Relaxed),
        corpus: CORPUS.load(Ordering::Relaxed),
        fsyncs: FSYNCS.load(Ordering::Relaxed),
        bytes_written: BYTES_WRITTEN.load(Ordering::Relaxed),
        files_written: FILES_WRITTEN.load(Ordering::Relaxed),
        snapshot_rebuilds: SNAPSHOT_REBUILDS.load(Ordering::Relaxed),
        snapshot_nanos: SNAPSHOT_NANOS.load(Ordering::Relaxed),
        memo_page_probes: MEMO_PAGE_PROBES.load(Ordering::Relaxed),
        cache_page_copies: CACHE_PAGE_COPIES.load(Ordering::Relaxed),
        shared_tree_node_copies: SHARED_TREE_NODE_COPIES.load(Ordering::Relaxed),
        icon_page_probes: ICON_PAGE_PROBES.load(Ordering::Relaxed),
        signature_block_probes: SIGNATURE_BLOCK_PROBES.load(Ordering::Relaxed),
        query_facts_copies: QUERY_FACTS_COPIES.load(Ordering::Relaxed),
        query_facts_derived: QUERY_FACTS_DERIVED.load(Ordering::Relaxed),
        query_carry_block_probes: QUERY_CARRY_BLOCK_PROBES.load(Ordering::Relaxed),
        query_registry_pages_read: QUERY_REGISTRY_PAGES_READ.load(Ordering::Relaxed),
        hash_reads: HASH_READS.load(Ordering::Relaxed),
        stamps_by_path: STAMPS_BY_PATH.load(Ordering::Relaxed),
        store_reads: STORE_READS.load(Ordering::Relaxed),
    }
}

pub(crate) fn store_read() {
    STORE_READS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn stamp_by_path() {
    STAMPS_BY_PATH.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn readdir() {
    READDIR.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn full_read() {
    FULL_READS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn preamble_read() {
    PREAMBLE_READS.fetch_add(1, Ordering::Relaxed);
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
    HASH_READS.fetch_add(1, Ordering::Relaxed);
}
