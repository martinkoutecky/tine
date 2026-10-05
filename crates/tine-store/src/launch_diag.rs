//! In-memory timing recorder behind `Store::diagnostics` (GH #623 follow-up).
//!
//! Reporters cannot share their graphs, so the diagnostics dump has to carry
//! what a performance diagnosis needs. This recorder keeps the phase timings of
//! the last launch, the last full stat diffs, and the last saves.
//!
//! Privacy boundary (I-5): numbers and closed tokens only. Nothing here ever
//! holds a page name, a path, text, or a hash of any of them; the types have no
//! field that could (guarded by `store::diagnostics` tests, which plant a
//! distinctive name and body and search the dump for them).
//!
//! Cost: the hot loops accumulate local `Duration`s (two `Instant::now()` per
//! phase per file, no allocation) and hand one `PassStats` over per pass; the
//! mutex is taken once per pass, diff or save, never per file.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Entries kept in each recent-event ring.
const RECENT: usize = 16;
/// Load passes kept (a pass restarts only when a file changed while parsing).
const MAX_PASSES: usize = 8;

pub(crate) use tine_core::latency::micros;
use tine_core::latency::{ms, LatencyHist};

/// Accumulates the time spent inside an iterator's `next` (the directory
/// listing syscalls), leaving the loop body's own time (stat) to the caller.
pub(crate) struct TimedIter<'a, I> {
    inner: I,
    total: &'a mut Duration,
}

impl<'a, I> TimedIter<'a, I> {
    pub(crate) fn new(inner: I, total: &'a mut Duration) -> Self {
        Self { inner, total }
    }
}

impl<I: Iterator> Iterator for TimedIter<'_, I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<I::Item> {
        let began = Instant::now();
        let item = self.inner.next();
        *self.total += began.elapsed();
        item
    }
}

/// Listing and stat time of one directory walk (`full`/`changed` are filled
/// by `reconcile`: whether it walked the whole graph, and how many files differed).
#[derive(Clone, Copy, Default)]
pub(crate) struct CollectTimes {
    pub(crate) listing: Duration,
    pub(crate) stat: Duration,
    pub(crate) files: u64,
    pub(crate) full: bool,
    pub(crate) changed: u64,
}

/// One load pass of the background graph load.
#[derive(Clone, Default)]
pub(crate) struct PassStats {
    /// Closed token: why the pass ended (see `outcome_*` consts).
    pub(crate) outcome: &'static str,
    pub(crate) listing_us: u64,
    pub(crate) entries: u64,
    pub(crate) stat_us: u64,
    pub(crate) stat_files: u64,
    /// Open + read + input validation (read time is separable from parse time).
    pub(crate) read_us: u64,
    pub(crate) read_files: u64,
    pub(crate) read_bytes: u64,
    pub(crate) read_failed: u64,
    pub(crate) parse_us: u64,
    pub(crate) parsed_files: u64,
    /// Wall time of the parallel read+parse phase, and the worker shards it ran.
    /// `stat_us`, `read_us` and `parse_us` are SUMMED THREAD time across those
    /// workers (so they can exceed this wall time), which keeps reading
    /// separable from parsing without a per-file allocation.
    pub(crate) parallel_wall_us: u64,
    pub(crate) workers: u64,
    pub(crate) recheck_us: u64,
    /// Index/snapshot build: derived indexes, mtime stat, publication under the cache lock.
    pub(crate) install_us: u64,
    pub(crate) wall_us: u64,
    pub(crate) crlf_files: u64,
}

impl PassStats {
    /// Close the pass: stamp its outcome and wall time, hand it to the recorder
    /// and pass `result` through (the caller's return value).
    pub(crate) fn finish(
        mut self,
        diag: &DiagRecorder,
        began: Instant,
        outcome: &'static str,
        result: bool,
    ) -> bool {
        self.outcome = outcome;
        self.wall_us = micros(began.elapsed());
        diag.pass(self);
        result
    }
}

/// Per-file phase time of one load pass, shared by the parallel parse workers.
/// Relaxed atomics: a handful of `fetch_add`s per file, no allocation.
#[derive(Default)]
pub(crate) struct PassClock {
    stat_us: AtomicU64,
    stat_files: AtomicU64,
    read_us: AtomicU64,
    read_files: AtomicU64,
    read_bytes: AtomicU64,
    read_failed: AtomicU64,
    parse_us: AtomicU64,
    parsed_files: AtomicU64,
    crlf_files: AtomicU64,
}

impl PassClock {
    pub(crate) fn stat(&self, elapsed: Duration) {
        self.stat_us.fetch_add(micros(elapsed), Ordering::Relaxed);
        self.stat_files.fetch_add(1, Ordering::Relaxed);
    }
    /// `bytes` is `None` when the read failed.
    pub(crate) fn read(&self, elapsed: Duration, bytes: Option<usize>) {
        self.read_us.fetch_add(micros(elapsed), Ordering::Relaxed);
        match bytes {
            Some(bytes) => {
                self.read_files.fetch_add(1, Ordering::Relaxed);
                self.read_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
            }
            None => {
                self.read_failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    pub(crate) fn parse(&self, elapsed: Duration) {
        self.parse_us.fetch_add(micros(elapsed), Ordering::Relaxed);
        self.parsed_files.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn crlf(&self, is_crlf: bool) {
        self.crlf_files
            .fetch_add(u64::from(is_crlf), Ordering::Relaxed);
    }
    /// Move the totals into `pass` once the workers have joined.
    pub(crate) fn fold(&self, pass: &mut PassStats, parallel_wall: Duration, workers: usize) {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        pass.stat_us = get(&self.stat_us);
        pass.stat_files = get(&self.stat_files);
        pass.read_us = get(&self.read_us);
        pass.read_files = get(&self.read_files);
        pass.read_bytes = get(&self.read_bytes);
        pass.read_failed = get(&self.read_failed);
        pass.parse_us = get(&self.parse_us);
        pass.parsed_files = get(&self.parsed_files);
        pass.crlf_files = get(&self.crlf_files);
        pass.parallel_wall_us = micros(parallel_wall);
        pass.workers = workers as u64;
    }
}

pub(crate) const OUTCOME_INSTALLED: &str = "installed";
pub(crate) const OUTCOME_CACHE_ALREADY_BUILT: &str = "cache_already_built";
pub(crate) const OUTCOME_FILE_CHANGED: &str = "file_changed_during_parse";
pub(crate) const OUTCOME_INSTALL_DECLINED: &str = "install_declined";
pub(crate) const OUTCOME_CANCELLED: &str = "cancelled";

/// Why a full stat diff ran.
#[derive(Clone, Copy)]
pub(crate) enum DiffTrigger {
    /// `scan_refresh` on a ready graph: the rescan on return to the window.
    Rescan,
    /// `scan_refresh` retrying a failed load.
    Recovery,
    /// `rebuild_all`: the Settings "Rescan graph" button. Ignores every stamp
    /// and hashes every file.
    Rebuild,
    /// The watcher (re)installed its OS watch and checked once.
    WatchInstall,
    /// The OS watch reported a rescan-required or pathless event.
    WatchEvent,
    /// Poll mode cycle (no OS watch).
    Poll,
    /// The launch diff: the full stat diff against the load pass's own
    /// observations (or a restored checkpoint's), before Ready.
    Launch,
    /// The one follow-up about 2 s after a launch diff that left racy paths
    /// (storage spec §5.4).
    RacyFollowUp,
    /// Test-only direct reconcile.
    #[cfg(test)]
    Test,
}

impl DiffTrigger {
    fn token(self) -> &'static str {
        match self {
            Self::Rescan => "rescan_command",
            Self::Recovery => "load_recovery",
            Self::Rebuild => "rebuild_command",
            Self::WatchInstall => "watch_install",
            Self::WatchEvent => "watch_rescan_event",
            Self::Poll => "poll_cycle",
            Self::Launch => "launch_diff",
            Self::RacyFollowUp => "racy_follow_up",
            #[cfg(test)]
            Self::Test => "test",
        }
    }
}

#[derive(Clone)]
pub(crate) struct DiffStats {
    pub(crate) trigger: DiffTrigger,
    pub(crate) total_us: u64,
    pub(crate) listing_us: u64,
    pub(crate) stat_us: u64,
    pub(crate) files: u64,
    pub(crate) changed: u64,
}

impl DiffStats {
    pub(crate) fn new(trigger: DiffTrigger, total: Duration, walk: &CollectTimes) -> Self {
        Self {
            trigger,
            total_us: micros(total),
            listing_us: micros(walk.listing),
            stat_us: micros(walk.stat),
            files: walk.files,
            changed: walk.changed,
        }
    }
}

/// Baseline-derived file facts for the shape statistics (numbers only).
#[derive(Default)]
pub(crate) struct FileFacts {
    pub(crate) lens: Vec<u64>,
    pub(crate) conflict_named: u64,
    pub(crate) non_nfc_named: u64,
}

/// `fill_revs`: the second read pass that hashes every file for the baseline.
#[derive(Clone, Copy, Default)]
pub(crate) struct FillStats {
    pub(crate) wall_us: u64,
    pub(crate) stat_us: u64,
    pub(crate) read_us: u64,
    pub(crate) files: u64,
    pub(crate) bytes: u64,
}

#[derive(Clone)]
struct SaveStats {
    at_us: u64,
    total_us: u64,
    publication_wait_us: u64,
    writer_wait_us: u64,
    steps: u64,
    committed: bool,
}

/// Timing of one `Transaction::commit`, finished by `DiagRecorder::save`.
pub(crate) struct SaveTiming {
    began: Instant,
    publication_wait: Duration,
    writer_wait: Duration,
    steps: usize,
}

impl SaveTiming {
    pub(crate) fn start(steps: usize) -> Self {
        Self {
            began: Instant::now(),
            publication_wait: Duration::ZERO,
            writer_wait: Duration::ZERO,
            steps,
        }
    }

    /// Call right after waiting for earlier reference publication.
    pub(crate) fn publication_waited(&mut self) {
        self.publication_wait = self.began.elapsed();
    }

    /// Call right after acquiring the writer lock.
    pub(crate) fn writer_acquired(&mut self) {
        self.writer_wait = self.began.elapsed().saturating_sub(self.publication_wait);
    }
}

/// One full asset walk (`observe_assets(full)`): how long it waited for the
/// writer lock, how long it held it, and how many assets it reported changed.
#[derive(Clone, Copy)]
struct AssetWalkStats {
    at_us: u64,
    writer_wait_us: u64,
    held_us: u64,
    changed: u64,
}

/// A `Store::page` that waited at least this long for the writer lock is kept
/// verbatim with its launch-relative time (GH #623: a page click ~3 s after a
/// focus return).
const SLOW_PAGE_WAIT_US: u64 = 100_000;

#[derive(Clone, Copy)]
struct SlowPageWait {
    at_us: u64,
    wait_us: u64,
}

#[derive(Default)]
struct State {
    open_us: Option<u64>,
    baseline: Option<CollectTimes>,
    baseline_wall_us: u64,
    passes: Vec<PassStats>,
    passes_total: u64,
    restarts: u64,
    fill: Option<FillStats>,
    publish_us: Option<u64>,
    ready_us: Option<u64>,
    stopped: bool,
    crlf_at_last_load: Option<u64>,
    diffs: VecDeque<DiffStats>,
    diffs_total: u64,
    saves: VecDeque<SaveStats>,
    saves_total: u64,
    builds: u64,
    build_last_us: u64,
    build_total_us: u64,
    checkpoint_load: Option<(&'static str, u64, u64)>,
    serving_us: Option<u64>,
    checkpoint_writes: u64,
    checkpoint_last: Option<(&'static str, u64, u64, u64, u64)>,
    asset_walks: VecDeque<AssetWalkStats>,
    asset_walks_total: u64,
    page_waits: LatencyHist,
    slow_page_waits: VecDeque<SlowPageWait>,
}

/// Recorder owned by the `Graph`; one per opened store.
pub(crate) struct DiagRecorder {
    began: Instant,
    began_unix_ms: u64,
    state: Mutex<State>,
}

impl DiagRecorder {
    pub(crate) fn new() -> Self {
        Self {
            began: Instant::now(),
            began_unix_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64),
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn since_launch(&self) -> u64 {
        micros(self.began.elapsed())
    }

    /// `Store::open` finished everything before the background load starts.
    pub(crate) fn open_done(&self) {
        self.state().open_us = Some(self.since_launch());
    }

    /// The watcher's baseline directory walk at `WatchHandle::start`.
    pub(crate) fn baseline(&self, times: CollectTimes, wall: Duration) {
        let mut state = self.state();
        state.baseline = Some(times);
        state.baseline_wall_us = micros(wall);
    }

    pub(crate) fn pass(&self, stats: PassStats) {
        let mut state = self.state();
        state.passes_total += 1;
        if stats.outcome == OUTCOME_FILE_CHANGED || stats.outcome == OUTCOME_CANCELLED {
            state.restarts += 1;
        }
        if stats.outcome == OUTCOME_INSTALLED {
            state.crlf_at_last_load = Some(stats.crlf_files);
        }
        if state.passes.len() == MAX_PASSES {
            state.passes.remove(0);
        }
        state.passes.push(stats);
    }

    pub(crate) fn fill_revs(&self, stats: FillStats) {
        self.state().fill = Some(stats);
    }

    /// First publication done and status Ready; `publish_began` is when the
    /// worker started publishing (snapshot capture included).
    pub(crate) fn ready(&self, publish_began: Instant) {
        let mut state = self.state();
        state.publish_us = Some(micros(publish_began.elapsed()));
        state.ready_us = Some(micros(self.began.elapsed()));
    }

    /// The background load ended without becoming ready.
    pub(crate) fn load_stopped(&self) {
        self.state().stopped = true;
    }

    pub(crate) fn diff(&self, stats: DiffStats) {
        let mut state = self.state();
        state.diffs_total += 1;
        if state.diffs.len() == RECENT {
            state.diffs.pop_front();
        }
        state.diffs.push_back(stats);
    }

    /// One full asset walk finished (writer lock held for `held`).
    pub(crate) fn asset_walk(&self, writer_wait: Duration, held: Duration, changed: usize) {
        let mut state = self.state();
        state.asset_walks_total += 1;
        if state.asset_walks.len() == RECENT {
            state.asset_walks.pop_front();
        }
        let at_us = micros(self.began.elapsed());
        state.asset_walks.push_back(AssetWalkStats {
            at_us,
            writer_wait_us: micros(writer_wait),
            held_us: micros(held),
            changed: changed as u64,
        });
    }

    /// `Store::page` acquired the writer lock after waiting `waited`.
    pub(crate) fn page_writer_wait(&self, waited: Duration) {
        let mut state = self.state();
        state.page_waits.record(waited);
        let wait_us = micros(waited);
        if wait_us >= SLOW_PAGE_WAIT_US {
            if state.slow_page_waits.len() == RECENT {
                state.slow_page_waits.pop_front();
            }
            let at_us = micros(self.began.elapsed());
            state
                .slow_page_waits
                .push_back(SlowPageWait { at_us, wait_us });
        }
    }

    pub(crate) fn save(&self, timing: SaveTiming, committed: bool) {
        let mut state = self.state();
        state.saves_total += 1;
        if state.saves.len() == RECENT {
            state.saves.pop_front();
        }
        state.saves.push_back(SaveStats {
            at_us: micros(timing.began.duration_since(self.began)),
            total_us: micros(timing.began.elapsed()),
            publication_wait_us: micros(timing.publication_wait),
            writer_wait_us: micros(timing.writer_wait),
            steps: timing.steps as u64,
            committed,
        });
    }

    /// A whole-graph build triggered by a query rather than by the background load.
    pub(crate) fn on_demand_build(&self, wall: Duration) {
        let mut state = self.state();
        state.builds += 1;
        state.build_last_us = micros(wall);
        state.build_total_us += micros(wall);
    }

    /// The launch tried the checkpoint: closed outcome token (`loaded` or a
    /// fallback reason), wall time to read and validate it, file bytes.
    pub(crate) fn checkpoint_load(&self, outcome: &'static str, wall: Duration, bytes: u64) {
        self.state().checkpoint_load = Some((outcome, micros(wall), bytes));
    }

    /// A loaded checkpoint is being served (status still Loading).
    pub(crate) fn serving(&self) {
        self.state().serving_us = Some(self.since_launch());
    }

    /// One checkpoint attempt: closed outcome token, wall time, how long the
    /// capture held the writer lock, raw body bytes and file bytes (zero
    /// unless written).
    pub(crate) fn checkpoint_write(
        &self,
        outcome: &'static str,
        wall: Duration,
        writer_held: Duration,
        raw: u64,
        file: u64,
    ) {
        let mut state = self.state();
        state.checkpoint_writes += 1;
        state.checkpoint_last = Some((outcome, micros(wall), micros(writer_held), raw, file));
    }

    /// CRLF-file count of the last installed load pass (`None` before one).
    pub(crate) fn crlf_at_last_load(&self) -> Option<u64> {
        self.state().crlf_at_last_load
    }

    /// Everything except graph shape. `status` is a closed token from the caller.
    pub(crate) fn snapshot(&self, status: &'static str) -> Value {
        let state = self.state();
        let causes: Vec<&str> = state
            .passes
            .iter()
            .filter(|pass| pass.outcome != OUTCOME_INSTALLED)
            .map(|pass| pass.outcome)
            .collect();
        let now = self.since_launch();
        json!({
            "launch": {
                "startedUnixMs": self.began_unix_ms,
                "ageMs": ms(now),
                "status": status,
                "loadStopped": state.stopped,
                "openMs": state.open_us.map(ms),
                "baselineWalk": state.baseline.map(|walk| json!({
                    "wallMs": ms(state.baseline_wall_us),
                    "listingMs": ms(micros(walk.listing)),
                    "statMs": ms(micros(walk.stat)),
                    "files": walk.files,
                })),
                "loadPasses": state.passes.iter().map(pass_json).collect::<Vec<_>>(),
                "loadPassesTotal": state.passes_total,
                "loadRestarts": state.restarts,
                "loadPassOutcomes": causes,
                "fillRevs": state.fill.map(|fill| json!({
                    "wallMs": ms(fill.wall_us),
                    "statMs": ms(fill.stat_us),
                    "readMs": ms(fill.read_us),
                    "files": fill.files,
                    "bytes": fill.bytes,
                })),
                "publishMs": state.publish_us.map(ms),
                "readyMs": state.ready_us.map(ms),
            },
            "checkpoint": {
                "load": state.checkpoint_load.map(|(outcome, wall, bytes)| json!({
                    "outcome": outcome,
                    "wallMs": ms(wall),
                    "bytes": bytes,
                })),
                "servingMs": state.serving_us.map(ms),
                "writes": state.checkpoint_writes,
                "last": state.checkpoint_last.map(|(outcome, wall, held, raw, file)| json!({
                    "outcome": outcome,
                    "wallMs": ms(wall),
                    "writerHeldMs": ms(held),
                    "rawBytes": raw,
                    "fileBytes": file,
                })),
            },
            "onDemandBuilds": {
                "count": state.builds,
                "lastMs": ms(state.build_last_us),
                "totalMs": ms(state.build_total_us),
            },
            "fullDiffs": {
                "total": state.diffs_total,
                "recent": state.diffs.iter().map(|diff| json!({
                    "trigger": diff.trigger.token(),
                    "totalMs": ms(diff.total_us),
                    "listingMs": ms(diff.listing_us),
                    "statMs": ms(diff.stat_us),
                    "files": diff.files,
                    "changed": diff.changed,
                })).collect::<Vec<_>>(),
            },
            "assetWalks": {
                "total": state.asset_walks_total,
                "recent": state.asset_walks.iter().map(|walk| json!({
                    "atMsAfterLaunch": ms(walk.at_us),
                    "writerWaitMs": ms(walk.writer_wait_us),
                    "heldMs": ms(walk.held_us),
                    "changed": walk.changed,
                })).collect::<Vec<_>>(),
            },
            "pageWriterWaits": {
                "all": state.page_waits.to_json(),
                "slow": state.slow_page_waits.iter().map(|wait| json!({
                    "atMsAfterLaunch": ms(wait.at_us),
                    "waitMs": ms(wait.wait_us),
                })).collect::<Vec<_>>(),
            },
            "saves": {
                "total": state.saves_total,
                "recent": state.saves.iter().map(|save| json!({
                    "atMsAfterLaunch": ms(save.at_us),
                    "totalMs": ms(save.total_us),
                    "publicationWaitMs": ms(save.publication_wait_us),
                    "writerWaitMs": ms(save.writer_wait_us),
                    "steps": save.steps,
                    "committed": save.committed,
                })).collect::<Vec<_>>(),
            },
        })
    }
}

fn pass_json(pass: &PassStats) -> Value {
    json!({
        "outcome": pass.outcome,
        "wallMs": ms(pass.wall_us),
        "listing": { "ms": ms(pass.listing_us), "entries": pass.entries },
        "stat": { "ms": ms(pass.stat_us), "files": pass.stat_files },
        "read": {
            "ms": ms(pass.read_us),
            "files": pass.read_files,
            "bytes": pass.read_bytes,
            "failed": pass.read_failed,
        },
        "parse": { "ms": ms(pass.parse_us), "files": pass.parsed_files },
        // stat/read/parse above are summed worker-thread time; this is the wall
        // time of the parallel phase and the number of worker shards.
        "parallel": { "wallMs": ms(pass.parallel_wall_us), "workers": pass.workers },
        "recheckMs": ms(pass.recheck_us),
        "installMs": ms(pass.install_us),
        "crlfFiles": pass.crlf_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_iter_counts_only_next_and_ms_keeps_a_decimal() {
        let mut spent = Duration::ZERO;
        let items: Vec<u32> = TimedIter::new([1u32, 2, 3].into_iter(), &mut spent).collect();
        assert_eq!(items, [1, 2, 3]);
        assert_eq!(ms(1_234), 1.2);
        assert_eq!(ms(40), 0.0);
        assert_eq!(ms(60), 0.1);
    }

    #[test]
    fn rings_are_bounded_and_totals_keep_counting() {
        let diag = DiagRecorder::new();
        for _ in 0..(RECENT + 5) {
            diag.diff(DiffStats {
                trigger: DiffTrigger::Test,
                total_us: 1,
                listing_us: 0,
                stat_us: 0,
                files: 1,
                changed: 0,
            });
            diag.save(SaveTiming::start(1), true);
        }
        for _ in 0..(MAX_PASSES + 3) {
            diag.pass(PassStats {
                outcome: OUTCOME_FILE_CHANGED,
                ..PassStats::default()
            });
        }
        let dump = diag.snapshot("loading");
        assert_eq!(dump["fullDiffs"]["total"], RECENT + 5);
        assert_eq!(
            dump["fullDiffs"]["recent"].as_array().unwrap().len(),
            RECENT
        );
        assert_eq!(dump["saves"]["recent"].as_array().unwrap().len(), RECENT);
        assert_eq!(
            dump["launch"]["loadPasses"].as_array().unwrap().len(),
            MAX_PASSES
        );
        assert_eq!(dump["launch"]["loadPassesTotal"], MAX_PASSES + 3);
        assert_eq!(dump["launch"]["loadRestarts"], MAX_PASSES + 3);
    }

    #[test]
    fn latency_records_are_fixed_size_whatever_the_traffic() {
        let mut hist = LatencyHist::new();
        for millis in [0u64, 1, 2, 10, 11, 100, 101, 500, 501, 2000, 2001, 9000] {
            hist.record(Duration::from_millis(millis));
        }
        for _ in 0..1000 {
            hist.record(Duration::from_micros(5));
        }
        let json = hist.to_json();
        assert_eq!(json["count"], 1012);
        assert_eq!(json["buckets"].as_array().unwrap().len(), 6);
        assert_eq!(
            json["lastMs"].as_array().unwrap().len(),
            tine_core::latency::LATENCY_RECENT
        );
        assert_eq!(json["maxMs"], 9000.0);
        // 0 ms, 1 ms and the 1000 sub-ms records sit at or under the 1 ms bound.
        assert_eq!(json["buckets"][0], 1002);
        assert_eq!(
            json["buckets"][5], 2,
            "2001 ms and 9000 ms are over 2000 ms"
        );

        let diag = DiagRecorder::new();
        for _ in 0..(RECENT + 5) {
            diag.asset_walk(Duration::from_millis(1), Duration::from_millis(2), 0);
            diag.page_writer_wait(Duration::from_millis(250));
        }
        diag.page_writer_wait(Duration::from_millis(3));
        let dump = diag.snapshot("ready");
        assert_eq!(dump["assetWalks"]["total"], RECENT + 5);
        assert_eq!(
            dump["assetWalks"]["recent"].as_array().unwrap().len(),
            RECENT
        );
        assert_eq!(dump["pageWriterWaits"]["all"]["count"], RECENT + 6);
        assert_eq!(
            dump["pageWriterWaits"]["slow"].as_array().unwrap().len(),
            RECENT,
            "only waits of 100 ms or more are kept verbatim, bounded"
        );
    }
}
