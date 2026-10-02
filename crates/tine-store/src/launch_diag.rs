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
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Entries kept in each recent-event ring.
const RECENT: usize = 16;
/// Load passes kept (a pass restarts only when a file changed while parsing).
const MAX_PASSES: usize = 8;

pub(crate) fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Microseconds as milliseconds with one decimal (sub-millisecond phases stay visible).
fn ms(us: u64) -> f64 {
    (us as f64 / 100.0).round() / 10.0
}

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
    /// Deliberate pacing sleeps (2 ms per 24 pages), so they are not mistaken for I/O.
    pub(crate) pace_us: u64,
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

pub(crate) const OUTCOME_INSTALLED: &str = "installed";
pub(crate) const OUTCOME_CACHE_ALREADY_BUILT: &str = "cache_already_built";
pub(crate) const OUTCOME_FILE_CHANGED: &str = "file_changed_during_parse";
pub(crate) const OUTCOME_INSTALL_DECLINED: &str = "install_declined";
pub(crate) const OUTCOME_CANCELLED: &str = "cancelled";

/// Why a full stat diff ran.
#[derive(Clone, Copy)]
pub(crate) enum DiffTrigger {
    /// `scan_refresh` on a ready graph: the Settings "Rescan graph" button and the
    /// rescan on return to the window share this trigger.
    Rescan,
    /// `scan_refresh` retrying a failed load.
    Recovery,
    /// The watcher (re)installed its OS watch and checked once.
    WatchInstall,
    /// The OS watch reported a rescan-required or pathless event.
    WatchEvent,
    /// Poll mode cycle (no OS watch).
    Poll,
    /// Test-only direct reconcile.
    #[cfg(test)]
    Test,
}

impl DiffTrigger {
    fn token(self) -> &'static str {
        match self {
            Self::Rescan => "rescan_command",
            Self::Recovery => "load_recovery",
            Self::WatchInstall => "watch_install",
            Self::WatchEvent => "watch_rescan_event",
            Self::Poll => "poll_cycle",
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
        "paceMs": ms(pass.pace_us),
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
}
