//! Test-only accounting for the waited save and print paths.
use std::sync::atomic::{AtomicU64, Ordering};

static READDIR: AtomicU64 = AtomicU64::new(0);
static FULL_READS: AtomicU64 = AtomicU64::new(0);
static PARSES: AtomicU64 = AtomicU64::new(0);
static CORPUS: AtomicU64 = AtomicU64::new(0);
static FSYNCS: AtomicU64 = AtomicU64::new(0);
static BYTES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static FILES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_REBUILDS: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_NANOS: AtomicU64 = AtomicU64::new(0);

/// Primitive counts since the last reset. The fixture uses one process per case.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    /// Directory enumerations.
    pub readdir: u64,
    /// Complete page file reads.
    pub full_reads: u64,
    /// Page parses.
    pub parses: u64,
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
}

/// Reset all primitive counters immediately before a measured operation.
pub fn reset() {
    for counter in [
        &READDIR,
        &FULL_READS,
        &PARSES,
        &CORPUS,
        &FSYNCS,
        &BYTES_WRITTEN,
        &FILES_WRITTEN,
        &SNAPSHOT_REBUILDS,
        &SNAPSHOT_NANOS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

/// Capture the primitive counts.
pub fn snapshot() -> Counts {
    Counts {
        readdir: READDIR.load(Ordering::Relaxed),
        full_reads: FULL_READS.load(Ordering::Relaxed),
        parses: PARSES.load(Ordering::Relaxed),
        corpus: CORPUS.load(Ordering::Relaxed),
        fsyncs: FSYNCS.load(Ordering::Relaxed),
        bytes_written: BYTES_WRITTEN.load(Ordering::Relaxed),
        files_written: FILES_WRITTEN.load(Ordering::Relaxed),
        snapshot_rebuilds: SNAPSHOT_REBUILDS.load(Ordering::Relaxed),
        snapshot_nanos: SNAPSHOT_NANOS.load(Ordering::Relaxed),
    }
}

pub(crate) fn readdir() {
    READDIR.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn full_read() {
    FULL_READS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn parse() {
    PARSES.fetch_add(1, Ordering::Relaxed);
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
