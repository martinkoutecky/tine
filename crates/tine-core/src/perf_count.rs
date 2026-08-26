//! Audit-only primitive counters (cost-model audit 2026-08-26; never integrates).
//!
//! Global, always-on atomic counters at the primitive layer, plus wall-time
//! accumulators for the primitives wrapped through [`timed`]. Overhead is a
//! few ns per hit; the audit build tolerates it. Read with [`snapshot`],
//! subtract snapshots, and render with [`Snapshot::report`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

macro_rules! primitives {
    ($($name:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum P { $($name),+ }
        pub const PRIMITIVE_COUNT: usize = [$(P::$name),+].len();
        pub const PRIMITIVE_NAMES: [&str; PRIMITIVE_COUNT] = [$(stringify!($name)),+];
    };
}

primitives!(
    FileRead,
    FileReadBytes,
    FileWrite,
    FileWriteBytes,
    Fsync,
    SyncFs,
    DirFsync,
    Rename,
    ParseCall,
    ParseBytes,
    SqliteStmt,
    SqliteTxnCommit,
    SqliteConnOpen,
    OplogAppend,
    OplogReplayRecord,
    FrontierReconstruct,
    GraphWalkPage,
    HashCall,
    HashBytes,
    SerializeCall,
    SerializeBytes,
    BlockClone,
);

static COUNTS: [AtomicU64; PRIMITIVE_COUNT] =
    [const { AtomicU64::new(0) }; PRIMITIVE_COUNT];
static TIME_US: [AtomicU64; PRIMITIVE_COUNT] =
    [const { AtomicU64::new(0) }; PRIMITIVE_COUNT];

#[inline]
pub fn inc(p: P) {
    COUNTS[p as usize].fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn add(p: P, n: u64) {
    COUNTS[p as usize].fetch_add(n, Ordering::Relaxed);
}

#[inline]
pub fn add_time_us(p: P, us: u64) {
    TIME_US[p as usize].fetch_add(us, Ordering::Relaxed);
}

/// Count one hit of `p` and accumulate the closure's wall time against it.
#[inline]
pub fn timed<T>(p: P, f: impl FnOnce() -> T) -> T {
    inc(p);
    let start = Instant::now();
    let out = f();
    add_time_us(p, start.elapsed().as_micros() as u64);
    out
}

#[derive(Clone)]
pub struct Snapshot {
    counts: [u64; PRIMITIVE_COUNT],
    time_us: [u64; PRIMITIVE_COUNT],
    taken: Instant,
}

pub fn snapshot() -> Snapshot {
    let mut counts: [u64; PRIMITIVE_COUNT] =
        std::array::from_fn(|i| COUNTS[i].load(Ordering::Relaxed));
    let mut time_us: [u64; PRIMITIVE_COUNT] =
        std::array::from_fn(|i| TIME_US[i].load(Ordering::Relaxed));
    // SQLite primitives are observed inside the patched tine-storage crate
    // (rusqlite profile/commit hooks at the only production connection sites).
    let (stmts, stmt_time_us, commits, conn_opens) = tine_storage::audit_counters::read();
    counts[P::SqliteStmt as usize] = stmts;
    time_us[P::SqliteStmt as usize] = stmt_time_us;
    counts[P::SqliteTxnCommit as usize] = commits;
    counts[P::SqliteConnOpen as usize] = conn_opens;
    // Filesystem barriers executed inside tine-storage (journal appends, temp
    // publishes, dir syncs) sum into the same slots as tine-core's own sites.
    let (fsyncs, fsync_us, dir_fsyncs, dir_fsync_us, renames) =
        tine_storage::audit_counters::read_fs();
    counts[P::Fsync as usize] += fsyncs;
    time_us[P::Fsync as usize] += fsync_us;
    counts[P::DirFsync as usize] += dir_fsyncs;
    time_us[P::DirFsync as usize] += dir_fsync_us;
    counts[P::Rename as usize] += renames;
    Snapshot {
        counts,
        time_us,
        taken: Instant::now(),
    }
}

impl Snapshot {
    /// Render everything that changed since `earlier`, one line per primitive:
    /// `name count accounted_time_us`, plus the wall time between snapshots.
    pub fn report_since(&self, earlier: &Snapshot, label: &str) -> String {
        let mut out = String::new();
        let wall_us = self.taken.duration_since(earlier.taken).as_micros();
        out.push_str(&format!("== perf_count [{label}] wall={wall_us}us ==\n"));
        let mut accounted: u64 = 0;
        for i in 0..PRIMITIVE_COUNT {
            let dc = self.counts[i] - earlier.counts[i];
            let dt = self.time_us[i] - earlier.time_us[i];
            if dc != 0 || dt != 0 {
                out.push_str(&format!(
                    "{:<20} count={:<10} time_us={}\n",
                    PRIMITIVE_NAMES[i], dc, dt
                ));
                accounted += dt;
            }
        }
        out.push_str(&format!(
            "accounted_time_us={accounted} (timed primitives only; nesting may double-count)\n"
        ));
        out
    }
}
