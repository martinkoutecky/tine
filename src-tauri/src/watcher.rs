use crate::settings::{settings_path, update_settings};
use crate::state::{
    refresh_graph_for_label, slot_for_window, AppState, GraphSlot, LegacyGraphLease,
    RefreshLaneWait, RefreshOutcome,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{Emitter, Manager, State};
use tine_core::sync_runtime::{
    SyncAbsenceSweepEvent, SyncCheckpointPublicationDiagnostics, SyncHistoryRecoveryPhase,
    SyncHistoryRecoveryStatus, SyncRuntimeHandle, SyncRuntimeStatusSnapshot, SyncRuntimeTick,
    SyncWatcherObservation,
};
use tine_core::{
    model::GraphTextExactFeedPathClass, model::GraphTextExternalObservationTicket, model::PageKind,
    Graph,
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct GraphChange {
    name: String,
    kind: PageKind,
    created: bool,
    removed: bool,
}

// ---------------------------------------------------------------------------
// Bulk external revisions (Concord P2, GH #337 / spec L6)
// ---------------------------------------------------------------------------
// A VCS checkout, branch switch, or first big sync under a running Tine dumps
// one file event per touched page. Two per-file costs amplify that: the
// incremental reconcile branch pays two full reads + a parse per evented path
// (deliberately no stat shortcut, see `incremental_reconcile`), and the emit
// side sends one `graph-changed` per changed page, which the frontend answers
// with one dataRev bump and up to one `getPage` IPC each. Above the threshold
// both stop scaling per-file: the drained batch escalates to the stat-diff
// full branch (one consistent snapshot; an unchanged file costs one stat), and
// the changed pages are announced as ONE `graph-changed-bulk` event.

/// Boundary between "a burst of ordinary edits" and "an external revision".
///
/// Sizing: one atomic save produces at most 2 paths; a human-scale sync delta
/// (Syncthing propagating a session of edits) is single digits to low tens; a
/// checkout or big sync is typically hundreds. 32 sits between those clusters,
/// inside the 24–64 band the P2 design allows. The cost of escalating a batch
/// of 33 is one stat per unwatched-change file (microseconds each, ~2 ms even
/// on a 1,000-file graph) — cheaper than a single page's double-read — so the
/// exact value only needs to keep ordinary edits per-file, not be optimal.
const BULK_CHANGE_THRESHOLD: usize = 32;

/// Does a drained batch of this many owned event paths escalate to the full
/// stat-diff branch?
fn burst_escalates(owned_paths: usize) -> bool {
    owned_paths > BULK_CHANGE_THRESHOLD
}

/// Does a reconcile cycle that changed this many pages coalesce its frontend
/// notification into one aggregate event? Same boundary as `burst_escalates`,
/// deliberately: below it nothing about today's behavior changes.
fn emit_as_bulk(changed_pages: usize) -> bool {
    changed_pages > BULK_CHANGE_THRESHOLD
}

/// One aggregate frontend notification for a reconcile cycle that changed more
/// than `BULK_CHANGE_THRESHOLD` pages. Carries the full per-page change list so
/// the frontend can reload visible pages, run the dirty-page safety machinery,
/// and summarize the rest — without N events.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct GraphChangedBulk {
    changes: Vec<GraphChange>,
}

/// One coalesced cache-invalidation epoch for ordinary files below the graph's
/// approved assets capability. Paths are assets-relative and never expose the
/// user's absolute graph or external-assets location to the WebView.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct AssetChangedBatch {
    paths: Vec<String>,
}

/// Every sparse-runtime watcher event is scoped to the graph binding that
/// produced it. A window can be rebound to another graph while a watcher cycle
/// is in flight; the frontend must be able to drop that older cycle instead of
/// showing its status or failure for the new graph.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct SparseV2RuntimeStatusEvent {
    binding_generation: u64,
    runtime: crate::sync_runtime::SparseV2RuntimeStatusDto,
    application_page_admission: crate::state::ApplicationPageAdmission,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct AbsenceSweepChangedEvent {
    binding_generation: u64,
    sweep: SyncAbsenceSweepEvent,
}

fn relay_absence_sweep_events(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    handle: &SyncRuntimeHandle,
) {
    let Ok(events) = handle.subscribe_absence_sweep_events() else {
        return;
    };
    let app = app.clone();
    let label = label.to_owned();
    let _ = std::thread::Builder::new()
        .name(format!("tine-sweep-events-{label}"))
        .spawn(move || {
            while let Ok(sweep) = events.recv() {
                let _ = app.emit_to(
                    &label,
                    "absence-sweep-changed",
                    AbsenceSweepChangedEvent {
                        binding_generation,
                        sweep,
                    },
                );
            }
        });
}

/// Build the event from the one actor status observation that the watcher has
/// already obtained. A second `handle.status()` call could observe a different
/// lifecycle and briefly retain stale frontend write authority.
fn sparse_v2_runtime_status_event(
    binding_generation: u64,
    status: SyncRuntimeStatusSnapshot,
) -> SparseV2RuntimeStatusEvent {
    let application_page_admission =
        crate::state::ApplicationPageAdmission::from_managed_runtime_status(
            binding_generation,
            &status,
        );
    SparseV2RuntimeStatusEvent {
        binding_generation,
        runtime: crate::sync_runtime::runtime_status(status),
        application_page_admission,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct SparseV2TickEvent {
    binding_generation: u64,
    tick: crate::sync_runtime::SparseV2TickDto,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct SparseV2ErrorEvent {
    binding_generation: u64,
    message: String,
}

#[derive(Default)]
struct Pending {
    paths: HashSet<PathBuf>,
    full_paths: HashSet<PathBuf>,
    /// Exact ordinary-file events retained for the separate asset observer.
    /// This queue grants no graph-text or managed-operation admission: it is
    /// later intersected with each binding's approved assets capability and
    /// reduced to frontend cache invalidations only.
    asset_paths: HashSet<PathBuf>,
    /// Ambiguous/directory asset candidates. Only a candidate owned by an
    /// assets capability requests that capability's metadata-only rescan.
    asset_full_paths: HashSet<PathBuf>,
    /// Candidate `logseq/config.edn` paths. Configuration is not graph text --
    /// `incremental_page_paths` discards it a few lines below, which is why an
    /// external config edit was invisible until the next graph open -- so it
    /// needs its own queue rather than a place in `paths`.
    config_paths: HashSet<PathBuf>,
    /// Highest raw-callback frontier admitted into this pending batch for each
    /// Direct graph root. The callback records this while holding the same
    /// mutex used to add its notify event, so a drained batch can never
    /// acknowledge an event that is still waiting to enter the queue.
    legacy_observation_epochs: HashMap<PathBuf, GraphTextExternalObservationTicket>,
    need_full: bool,
    notify_error: bool,
    /// When the FIRST notify callback of the batch currently accumulating
    /// arrived (monotonic). Taken together with the paths at drain time, so a
    /// latency receipt can attribute callback→reconcile time (the coalescing
    /// window plus any scheduling delay). One `Instant` per batch — per-path
    /// stamps would allocate on the hot path for no diagnostic gain.
    first_event_at: Option<Instant>,
}

/// Resolve the filesystem watcher inputs for an existing Direct Files binding.
/// Sparse-v2 bindings use their actor; they never fall back to a second Direct
/// Files `Graph` or watcher here.
fn direct_watch_paths(
    slot: &GraphSlot,
) -> Result<(LegacyGraphLease, PathBuf), crate::command_error::CommandError> {
    let graph = slot
        .legacy_graph_cloned()
        .map_err(crate::command_error::CommandError::from)?;
    let root = slot.root_key.clone();
    Ok((graph, root))
}

impl Pending {
    fn note_event_arrival(&mut self) {
        if self.first_event_at.is_none() {
            self.first_event_at = Some(Instant::now());
        }
    }

    fn add_event(&mut self, event: notify::Event) {
        self.note_event_arrival();
        for path in event
            .paths
            .iter()
            .filter(|path| path_is_config_file_name(path))
        {
            self.config_paths.insert(path.clone());
        }
        if let Some(paths) = incremental_asset_paths(&event) {
            self.asset_paths.extend(paths);
        } else if !event.paths.is_empty() {
            self.asset_full_paths.extend(event.paths.iter().cloned());
        }
        if event.need_rescan() {
            if event.paths.is_empty() {
                self.need_full = true;
            } else {
                self.full_paths.extend(event.paths);
            }
            return;
        }
        if let Some(paths) = incremental_page_paths(&event) {
            self.paths.extend(paths);
        } else if event.paths.is_empty() {
            self.need_full = true;
        } else {
            // A directory move or genuinely unknown file operation needs a full
            // diff only for the graph that owns its reported path.
            self.full_paths.extend(event.paths);
        }
    }

    fn add_notify_error(&mut self) {
        self.note_event_arrival();
        self.need_full = true;
        self.notify_error = true;
    }

    fn add_legacy_observations(
        &mut self,
        observations: Vec<(PathBuf, GraphTextExternalObservationTicket)>,
    ) {
        for (root, ticket) in observations {
            self.legacy_observation_epochs
                .entry(root)
                .and_modify(|current| {
                    *current = current.later_for_same_instance(ticket).unwrap_or(ticket)
                })
                .or_insert(ticket);
        }
    }

    fn take_legacy_observation_epochs(
        &mut self,
    ) -> HashMap<PathBuf, GraphTextExternalObservationTicket> {
        std::mem::take(&mut self.legacy_observation_epochs)
    }
}

// ---------------------------------------------------------------------------
// Watcher latency receipts (GH #337 diagnosis)
// ---------------------------------------------------------------------------
// The reported 5–20 s external-change latency on Windows is unexplained by a
// pipeline whose design floor is ~200 ms (inotify coalescing) / 3 s (poll), and
// nothing measured the pipeline. These receipts are cheap and always on: one
// monotonic stamp when the first notify callback of a batch arrives, one when
// its post-debounce reconcile starts, one when its `graph-changed` events have
// been emitted. Each external-change batch logs one structured line (via
// `debug::diag`, so `--debug` captures it in the log file a reporter can send)
// and lands in a small in-memory ring the `watcher_latency_recent` command
// returns. No extra reads, no per-path allocation.

/// One external-change batch, as the reconcile loop experienced it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct WatcherLatencyReceipt {
    /// Monotonically increasing receipt number (process-wide).
    seq: u64,
    /// Wall-clock time the receipt was recorded (Unix ms) — to correlate with
    /// the reporter's "I saved the file at ...".
    at_unix_ms: u64,
    /// Graph label the batch was reconciled for.
    graph: String,
    /// "inotify" or "poll".
    mode: &'static str,
    /// `graph-changed` events emitted for this batch.
    pages: usize,
    /// Exact event paths this graph owned in the batch (0 for a pure full diff).
    event_paths: usize,
    /// Whether the full stat-diff branch was taken (poll cycle, unclassifiable
    /// event, kernel queue overflow, retry, or a burst-escalated batch above
    /// `BULK_CHANGE_THRESHOLD`).
    full_diff: bool,
    /// Reconcile errors in this batch (each schedules a backoff retry — a
    /// latency source worth seeing in a receipt trail).
    errors: usize,
    /// First notify callback → reconcile start (debounce + scheduling). `None`
    /// when no callback stamp exists for the batch: poll mode, or a cycle
    /// triggered by retry/control rather than an OS event.
    event_to_reconcile_ms: Option<u64>,
    /// Reconcile start → last `graph-changed` emitted (read + parse + emit).
    reconcile_ms: u64,
    /// First notify callback → last emit; the number GH #337 reports as 5–20 s.
    event_to_emit_ms: Option<u64>,
}

/// Report form of a watcher receipt. Graph labels are deliberately omitted:
/// they can contain a user-chosen graph name.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WatcherDiagnosticReceipt {
    mode: &'static str,
    pages: usize,
    event_paths: usize,
    full_diff: bool,
    errors: usize,
    event_to_reconcile_ms: Option<u64>,
    reconcile_ms: u64,
    event_to_emit_ms: Option<u64>,
}

/// Concord L0's reload-on-focus fallback. Some filesystems and sync clients
/// deliver no inotify edge at all (network mounts, a suspended app, a client
/// that writes through a path the kernel doesn't report), so the ONE thing the
/// user can always do — come back to the window — has to be able to ask.
///
/// This asks the watcher for one full stat-diff pass on its next cycle. It does
/// NOT invent a second freshness path: whatever the diff finds is emitted as
/// ordinary `graph-changed` / `graph-changed-bulk` events, so a page being
/// edited is deferred by the P1 replay machinery exactly as for a live event,
/// and a caret is never stolen.
static FULL_RESCAN_REQUESTED: AtomicU64 = AtomicU64::new(0);
static FULL_RESCAN_COMPLETED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
struct GraphRescanComplete {
    sequence: u64,
}

/// Request that one full rescan. Pair with `state::poke_watcher` — this only
/// arms the flag; the poke is what wakes the loop to read it.
pub(crate) fn request_full_rescan() -> u64 {
    FULL_RESCAN_REQUESTED.fetch_add(1, Ordering::SeqCst) + 1
}

/// Snapshot the newest explicit request not yet completed. Several focus and
/// visibility notifications may arrive before one watcher turn; one full pass
/// satisfies all of them and publishes the newest sequence.
fn pending_full_rescan() -> Option<u64> {
    let requested = FULL_RESCAN_REQUESTED.load(Ordering::SeqCst);
    (requested > FULL_RESCAN_COMPLETED.load(Ordering::SeqCst)).then_some(requested)
}

fn complete_full_rescan(app: &tauri::AppHandle, sequence: u64) {
    FULL_RESCAN_COMPLETED.fetch_max(sequence, Ordering::SeqCst);
    // Broadcast rather than target one graph window: a request can race a graph
    // rebind, and every frontend matches the exact sequence it requested.
    let _ = app.emit("graph-rescan-complete", GraphRescanComplete { sequence });
}

const LATENCY_RECEIPT_CAP: usize = 64;

static LATENCY_RECEIPTS: OnceLock<Mutex<VecDeque<WatcherLatencyReceipt>>> = OnceLock::new();
static LATENCY_RECEIPT_SEQ: AtomicU64 = AtomicU64::new(0);

fn latency_receipts() -> &'static Mutex<VecDeque<WatcherLatencyReceipt>> {
    LATENCY_RECEIPTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(LATENCY_RECEIPT_CAP)))
}

/// Pure builder so the duration arithmetic is unit-testable. `seq` and
/// `at_unix_ms` are stamped by `record_latency_receipt`.
#[allow(clippy::too_many_arguments)]
fn latency_receipt(
    graph: &str,
    inotify: bool,
    pages: usize,
    event_paths: usize,
    full_diff: bool,
    errors: usize,
    first_event_at: Option<Instant>,
    reconcile_started: Instant,
    emitted_at: Instant,
) -> WatcherLatencyReceipt {
    let since = |earlier: Instant, later: Instant| {
        later.saturating_duration_since(earlier).as_millis() as u64
    };
    WatcherLatencyReceipt {
        seq: 0,
        at_unix_ms: 0,
        graph: graph.to_string(),
        mode: if inotify { "inotify" } else { "poll" },
        pages,
        event_paths,
        full_diff,
        errors,
        event_to_reconcile_ms: first_event_at.map(|at| since(at, reconcile_started)),
        reconcile_ms: since(reconcile_started, emitted_at),
        event_to_emit_ms: first_event_at.map(|at| since(at, emitted_at)),
    }
}

fn push_latency_receipt(
    ring: &mut VecDeque<WatcherLatencyReceipt>,
    receipt: WatcherLatencyReceipt,
) {
    while ring.len() >= LATENCY_RECEIPT_CAP {
        ring.pop_front();
    }
    ring.push_back(receipt);
}

fn record_latency_receipt(mut receipt: WatcherLatencyReceipt) {
    receipt.seq = LATENCY_RECEIPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    receipt.at_unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let stage =
        |value: Option<u64>| value.map_or_else(|| "n/a".to_string(), |ms| format!("{ms}ms"));
    crate::debug::diag(format!(
        "watcher-latency seq={} graph={} mode={} pages={} event_paths={} full_diff={} errors={} event->reconcile={} reconcile={}ms event->emit={}",
        receipt.seq,
        receipt.graph,
        receipt.mode,
        receipt.pages,
        receipt.event_paths,
        receipt.full_diff,
        receipt.errors,
        stage(receipt.event_to_reconcile_ms),
        receipt.reconcile_ms,
        stage(receipt.event_to_emit_ms),
    ));
    crate::debug::record_watcher_latency(
        receipt.mode,
        u64::try_from(receipt.pages).unwrap_or(u64::MAX),
        u64::try_from(receipt.event_paths).unwrap_or(u64::MAX),
        receipt.full_diff,
        u64::try_from(receipt.errors).unwrap_or(u64::MAX),
        receipt.event_to_reconcile_ms,
        receipt.reconcile_ms,
        receipt.event_to_emit_ms,
    );
    if let Ok(mut ring) = latency_receipts().lock() {
        push_latency_receipt(&mut ring, receipt);
    }
}

pub(crate) fn diagnostic_latency_snapshot() -> Vec<WatcherDiagnosticReceipt> {
    latency_receipts()
        .lock()
        .map(|ring| {
            ring.iter()
                .map(|receipt| WatcherDiagnosticReceipt {
                    mode: receipt.mode,
                    pages: receipt.pages,
                    event_paths: receipt.event_paths,
                    full_diff: receipt.full_diff,
                    errors: receipt.errors,
                    event_to_reconcile_ms: receipt.event_to_reconcile_ms,
                    reconcile_ms: receipt.reconcile_ms,
                    event_to_emit_ms: receipt.event_to_emit_ms,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Debug command for bug reports: the last 64 external-change latency receipts,
/// oldest first. A reporter runs it from the devtools console and pastes the
/// result; no UI surface beyond that.
#[tauri::command]
pub(crate) fn watcher_latency_recent() -> Vec<WatcherLatencyReceipt> {
    latency_receipts()
        .lock()
        .map(|ring| ring.iter().cloned().collect())
        .unwrap_or_default()
}

const RETRY_BACKOFF: [Duration; 6] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// How long the inotify branch may block while a graph root it WANTS to watch is
/// still unwatched. Matches the poll branch's ceiling: this is a recovery cadence
/// for a root the kernel refused, not a polling strategy.
const UNWATCHED_ROOT_RETRY: Duration = Duration::from_secs(3);

/// How long to wait for the next cycle in the inotify branch.
///
/// A root whose `watch()` failed is retried on the next cycle — it is never
/// inserted into `watched` — but a cycle only begins when something wakes this
/// thread, and in the inotify branch that means an event from a root that IS
/// watched. With a single graph that is self-correcting: the failure leaves
/// `watched` empty, which takes the bounded poll branch instead. With two graphs
/// open it is not. inotify limits are per-user (`fs.inotify.max_user_watches`),
/// so opening a second large graph is exactly how one root fails while the other
/// is healthy — and then the failing graph stays invisible to external changes
/// until the healthy one happens to change, which on a quiet graph is never.
///
/// So bound the wait whenever a desired root is unwatched. (Direct Files
/// data-safety audit 2026-08-09, finding 16, in its reachable form: the blindness
/// is not permanent and there IS a poll fallback, but only when EVERY root fails.)
fn inotify_cycle_wait(retry_wait: Option<Duration>, unwatched_root: bool) -> Option<Duration> {
    match (retry_wait, unwatched_root) {
        (Some(wait), true) => Some(wait.min(UNWATCHED_ROOT_RETRY)),
        (None, true) => Some(UNWATCHED_ROOT_RETRY),
        (wait, false) => wait,
    }
}

#[derive(Default)]
struct RetrySchedule {
    failures: usize,
    due: Option<Instant>,
}

impl RetrySchedule {
    fn failed(&mut self, now: Instant) {
        let index = self.failures.min(RETRY_BACKOFF.len() - 1);
        self.failures = self.failures.saturating_add(1);
        self.due = Some(now + RETRY_BACKOFF[index]);
    }

    fn succeeded(&mut self) {
        self.failures = 0;
        self.due = None;
    }

    fn progressed(&mut self, now: Instant) {
        self.failures = 0;
        self.due = Some(now + Duration::from_millis(10));
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|due| due <= now) {
            self.due = None;
            true
        } else {
            false
        }
    }

    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.due.map(|due| due.saturating_duration_since(now))
    }
}

fn is_page_file_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md")
                || extension.eq_ignore_ascii_case("markdown")
                || extension.eq_ignore_ascii_case("org")
        })
}

fn path_is_existing_dir(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn is_tine_atomic_page_temp_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(mut stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    // Backup-restore publishes page files into the live graph through
    // `.tine-restore-{pid}-{seq}.tmp` temps (`backup.rs::atomic_copy_new_into_live`).
    // Recognize that shape too, or a rename event pairing such a temp with a
    // page path is dropped and the restored page never queued — previously
    // masked only by an unrelated `refresh_graph` call after restore (DUP-5).
    if let Some(rest) = stem.strip_prefix(".tine-restore-") {
        if let Some((pid, seq)) = rest.split_once('-') {
            if !pid.is_empty()
                && !seq.is_empty()
                && pid.chars().all(|value| value.is_ascii_digit())
                && seq.chars().all(|value| value.is_ascii_digit())
            {
                return true;
            }
        }
    }
    if let Some(without_projection) = stem.strip_suffix(".projection") {
        stem = without_projection;
    }
    if let Some(without_new) = stem.strip_suffix(".new") {
        stem = without_new;
    }
    let Some((before_seq, seq)) = stem.rsplit_once('.') else {
        return false;
    };
    let Some((page_name, pid)) = before_seq.rsplit_once('.') else {
        return false;
    };
    seq.chars().all(|value| value.is_ascii_digit())
        && pid.chars().all(|value| value.is_ascii_digit())
        && page_name.starts_with('.')
        && is_page_file_path(Path::new(page_name))
}

/// Directories whose churn a graph's watcher must never be woken by (Concord
/// P5). A repository or sync client parked inside the graph tree generates
/// thousands of events that CANNOT describe graph text — a `git gc`, an index
/// lock taken and dropped per command, a `.stversions` sweep — and every one of
/// them used to cross the channel, take the app-state read lock, lease each
/// graph and run two scope classifications before being discarded.
///
/// The scope is deliberately an explicit NAME LIST rather than "anything the
/// graph-text scope excludes": `.tine-sync` is also excluded from graph text,
/// but it is Tine's OWN provider tree and the managed lane's observations
/// depend on those events. A name here must be provably outside graph text on
/// its own — `vcs_and_tool_noise_dirs_can_never_hold_graph_text` asserts
/// exactly that against `GraphTextScope`, so this list can never hide a page.
///
/// Matched against components of the path RELATIVE to a watched graph root, so
/// a graph that itself lives under (say) `/repo/.git/notes` is unaffected.
const VCS_AND_TOOL_NOISE_DIRS: &[&str] = &[
    ".bzr",
    ".git",
    ".hg",
    ".jj",
    ".stfolder",
    ".stversions",
    ".svn",
    // NOT `_darcs`: it carries no leading dot and is not in the core's fixed
    // exclusions, so `_darcs/Page.md` IS eligible graph text. The guard test
    // below caught it on the first run — which is the whole reason this list is
    // asserted against `GraphTextScope` rather than assumed.
    "node_modules",
];

fn path_is_tool_noise(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    relative.components().any(|component| {
        let Some(name) = component.as_os_str().to_str() else {
            return false;
        };
        VCS_AND_TOOL_NOISE_DIRS.contains(&name)
    })
}

/// True when EVERY path this event reports is tool noise under some watched
/// root — the only case it is safe to drop the event outright.
///
/// Never true for a rescan-required event (a kernel queue overflow says nothing
/// about which paths were lost) or a pathless one, and never true for a
/// multi-path event with even one ordinary path: a rename that moves a file OUT
/// of `.git` reports both sides and must still be seen.
fn watch_event_is_tool_noise(event: &notify::Event, roots: &HashSet<PathBuf>) -> bool {
    if event.need_rescan() || event.paths.is_empty() || roots.is_empty() {
        return false;
    }
    event
        .paths
        .iter()
        .all(|path| roots.iter().any(|root| path_is_tool_noise(root, path)))
}

/// A watch event path that *might* be some graph's `logseq/config.edn`.
///
/// Only the filename, deliberately: which graph owns it -- and whether it sits
/// at the one graph-relative location that counts -- is
/// `tine_core::model::is_config_file_path`'s decision, made per root when the
/// batch drains. This is the cheap gate that keeps the pending set small.
fn path_is_config_file_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("config.edn"))
}

fn incremental_page_paths(event: &notify::Event) -> Option<Vec<PathBuf>> {
    use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

    let exact_file_event = matches!(
        event.kind,
        EventKind::Create(CreateKind::File)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Metadata(_))
            | EventKind::Remove(RemoveKind::File)
            // Windows ReadDirectoryChangesW supplies an exact path but no
            // create/modify sub-kind. The entry still exists, so the live
            // metadata check below is the file witness. `Remove(Any)` remains
            // deliberately absent: after removal we cannot distinguish a file
            // from a directory subtree.
            | EventKind::Create(CreateKind::Any)
            | EventKind::Modify(ModifyKind::Any)
    );
    let supported = exact_file_event
        || matches!(
            event.kind,
            EventKind::Modify(ModifyKind::Name(
                RenameMode::From | RenameMode::To | RenameMode::Both
            ))
        );
    if !supported || event.paths.is_empty() {
        return None;
    }
    if event.paths.iter().any(|path| path_is_existing_dir(path)) {
        return None;
    }
    let all_text_or_temp = event
        .paths
        .iter()
        .all(|path| is_page_file_path(path) || is_tine_atomic_page_temp_path(path));
    if !all_text_or_temp {
        // A rename without a file-kind witness may denote a directory subtree.
        if !exact_file_event {
            return None;
        }
        // An exact ordinary non-page file is outside Direct Files graph text.
        // Keep it out of both the incremental and full queues. The raw observer
        // likewise publishes no admission epoch for excluded paths.
        return Some(Vec::new());
    }
    Some(
        event
            .paths
            .iter()
            .filter(|path| is_page_file_path(path) && !path_is_existing_dir(path))
            .cloned()
            .collect(),
    )
}

/// Preserve exact ordinary-file events for the asset observer without deciding
/// here whether a path belongs to an assets capability. The callback has one
/// process-wide queue; binding ownership is deliberately resolved after drain.
fn incremental_asset_paths(event: &notify::Event) -> Option<Vec<PathBuf>> {
    use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

    let supported = matches!(
        event.kind,
        EventKind::Create(CreateKind::File)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Metadata(_))
            | EventKind::Remove(RemoveKind::File)
            | EventKind::Create(CreateKind::Any)
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Name(
                RenameMode::From | RenameMode::To | RenameMode::Both
            ))
    );
    if !supported || event.paths.is_empty() || event.need_rescan() {
        return None;
    }
    if event.paths.iter().any(|path| path_is_existing_dir(path)) {
        return None;
    }
    Some(event.paths.clone())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    len: u64,
    identity: u128,
    changed: i128,
}

fn metadata_stamp(md: &std::fs::Metadata) -> Option<FileStamp> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return Some(FileStamp {
            modified: md.modified().ok()?,
            len: md.len(),
            identity: ((md.dev() as u128) << 64) | md.ino() as u128,
            changed: (md.ctime() as i128) * 1_000_000_000 + md.ctime_nsec() as i128,
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return Some(FileStamp {
            modified: md.modified().ok()?,
            len: md.len(),
            identity: md.creation_time() as u128,
            changed: md.last_write_time() as i128,
        });
    }
    #[cfg(not(any(unix, windows)))]
    Some(FileStamp {
        modified: md.modified().ok()?,
        len: md.len(),
        identity: md
            .created()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
        changed: 0,
    })
}

/// The watcher's diff snapshot: every eligible graph-text file under the graph
/// root with its (mtime, len).
///
/// Scope authority is the core's `GraphTextScope` — the same one discovery
/// (`graph_text_inventory`) walks. Before GH #268 this scanned `journals/` and
/// `pages/` only, so a page at the graph root or in a custom folder was
/// discovered at open but never reconciled afterwards: the full-diff snapshot
/// it would have to differ against did not contain it.
/// `complete` is false when any part of the walk could not be read. A partial
/// snapshot would otherwise read as "every page under that directory was
/// deleted", and in poll mode it is also what decides whether the guarded
/// admission index may be updated from exact paths or has to be invalidated.
struct GraphTextSnapshot {
    files: HashMap<PathBuf, FileStamp>,
    complete: bool,
}

struct AssetSnapshot {
    files: HashMap<PathBuf, FileStamp>,
    complete: bool,
}

#[derive(Default)]
struct AssetWatchState {
    root: PathBuf,
    snap: HashMap<PathBuf, FileStamp>,
    baseline: bool,
    active: bool,
}

impl AssetWatchState {
    fn new(root: PathBuf) -> Self {
        // Capture before the OS watch is installed. The first reconcile after
        // installation then closes the binding→watch handoff: a sync client
        // that replaces an already-rendered image in that interval differs
        // from this baseline instead of silently becoming the baseline.
        let snapshot = collect_asset_files(&root);
        Self {
            root,
            snap: snapshot.files,
            baseline: true,
            active: true,
        }
    }

    fn active_root(&self) -> Option<&PathBuf> {
        self.active.then_some(&self.root)
    }
}

/// Tine's atomic asset publishers use hidden numeric temp names. They are
/// implementation artifacts, not logical assets; the final rename is the one
/// observation consumers care about.
fn is_tine_atomic_asset_temp_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    if !stem.starts_with('.') {
        return false;
    }
    let mut parts = stem.rsplit('.');
    let mut tail = parts.next().unwrap_or_default();
    if !tail.chars().all(|value| value.is_ascii_digit()) {
        // Replacement/copy helpers may name the publication purpose between
        // the numeric sequence and `.tmp`.
        tail = parts.next().unwrap_or_default();
    }
    if !tail.chars().all(|value| value.is_ascii_digit()) {
        return false;
    }
    parts
        .next()
        .is_some_and(|pid| !pid.is_empty() && pid.chars().all(|value| value.is_ascii_digit()))
}

fn collect_asset_files(root: &Path) -> AssetSnapshot {
    let mut files = HashMap::new();
    let mut complete = true;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let read_dir = match std::fs::read_dir(&directory) {
            Ok(read_dir) => read_dir,
            Err(error) if directory == root && error.kind() == std::io::ErrorKind::NotFound => {
                continue
            }
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in read_dir {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                complete = false;
                continue;
            };
            // No-follow, exactly like graph-text scans: an asset-directory
            // symlink cannot widen the already-approved capability or loop.
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() || is_tine_atomic_asset_temp_path(&path) {
                continue;
            }
            match entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata_stamp(&metadata))
            {
                Some(stamp) => {
                    files.insert(path, stamp);
                }
                None => complete = false,
            }
        }
    }
    AssetSnapshot { files, complete }
}

fn asset_relative_event_path(root: &Path, path: &Path) -> Option<String> {
    if is_tine_atomic_asset_temp_path(path) {
        return None;
    }
    let relative = path.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    relative
        .to_str()
        .map(|relative| relative.replace(std::path::MAIN_SEPARATOR, "/"))
}

#[derive(Clone, Copy)]
struct RecentAssetSelfWrite {
    stamp: Option<FileStamp>,
    recorded_at: Instant,
}

const ASSET_SELF_WRITE_TTL: Duration = Duration::from_secs(30);
const ASSET_SELF_WRITE_CAP: usize = 4096;
static RECENT_ASSET_SELF_WRITES: OnceLock<Mutex<HashMap<(String, PathBuf), RecentAssetSelfWrite>>> =
    OnceLock::new();

/// Assets already handed to a WebView can be rendered before the watcher loop
/// has installed that graph's OS watch. Retain the observed file identity long
/// enough for the watcher to use it as the cache's real baseline instead of
/// accidentally baselining a replacement that arrived during startup.
static RECENT_ASSET_READS: OnceLock<Mutex<HashMap<(String, PathBuf), RecentAssetSelfWrite>>> =
    OnceLock::new();

pub(crate) fn note_asset_read(window_label: &str, path: &Path) {
    let reads = RECENT_ASSET_READS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut reads) = reads.lock() else {
        return;
    };
    let now = Instant::now();
    reads.retain(|_, read| now.duration_since(read.recorded_at) <= ASSET_SELF_WRITE_TTL);
    if reads.len() >= ASSET_SELF_WRITE_CAP {
        if let Some(oldest) = reads
            .iter()
            .min_by_key(|(_, read)| read.recorded_at)
            .map(|(key, _)| key.clone())
        {
            reads.remove(&oldest);
        }
    }
    reads.insert(
        (window_label.to_owned(), path.to_path_buf()),
        RecentAssetSelfWrite {
            stamp: file_snapshot(path),
            recorded_at: now,
        },
    );
    if crate::debug::debug_enabled() {
        crate::debug::diag("asset observer recorded one WebView read");
    }
}

fn merge_recent_asset_reads(window_label: &str, state: &mut AssetWatchState) -> HashSet<PathBuf> {
    let Some(reads) = RECENT_ASSET_READS.get() else {
        return HashSet::new();
    };
    let Ok(mut reads) = reads.lock() else {
        return HashSet::new();
    };
    let now = Instant::now();
    let keys = reads
        .iter()
        .filter(|((label, path), read)| {
            now.duration_since(read.recorded_at) <= ASSET_SELF_WRITE_TTL
                && label == window_label
                && path.starts_with(&state.root)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let mut merged = HashSet::new();
    for key in keys {
        if let Some(read) = reads.remove(&key) {
            let path = key.1;
            match read.stamp {
                Some(stamp) => {
                    state.snap.insert(path.clone(), stamp);
                }
                None => {
                    state.snap.remove(&path);
                }
            }
            merged.insert(path);
        }
    }
    reads.retain(|_, read| now.duration_since(read.recorded_at) <= ASSET_SELF_WRITE_TTL);
    if !merged.is_empty() && crate::debug::debug_enabled() {
        crate::debug::diag(format!(
            "asset observer merged {} WebView-read baseline(s)",
            merged.len()
        ));
    }
    merged
}

/// Record the final filesystem state published by one window's own asset
/// command. The watcher suppresses that echo only for the originating WebView;
/// other windows still need the invalidation.
pub(crate) fn note_asset_self_write(window_label: &str, path: &Path) {
    let writes = RECENT_ASSET_SELF_WRITES.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut writes) = writes.lock() else {
        return;
    };
    let now = Instant::now();
    writes.retain(|_, write| now.duration_since(write.recorded_at) <= ASSET_SELF_WRITE_TTL);
    if writes.len() >= ASSET_SELF_WRITE_CAP {
        if let Some(oldest) = writes
            .iter()
            .min_by_key(|(_, write)| write.recorded_at)
            .map(|(key, _)| key.clone())
        {
            writes.remove(&oldest);
        }
    }
    let canonical_path = std::fs::canonicalize(path).ok().or_else(|| {
        let parent = std::fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    });
    let path = canonical_path.unwrap_or_else(|| path.to_path_buf());
    writes.insert(
        (window_label.to_owned(), path.clone()),
        RecentAssetSelfWrite {
            stamp: file_snapshot(&path),
            recorded_at: now,
        },
    );
}

fn take_matching_asset_self_write(
    window_label: &str,
    path: &Path,
    stamp: Option<FileStamp>,
) -> bool {
    let Some(writes) = RECENT_ASSET_SELF_WRITES.get() else {
        return false;
    };
    let Ok(mut writes) = writes.lock() else {
        return false;
    };
    let key = (window_label.to_owned(), path.to_path_buf());
    let Some(write) = writes.remove(&key) else {
        return false;
    };
    Instant::now().duration_since(write.recorded_at) <= ASSET_SELF_WRITE_TTL && write.stamp == stamp
}

// notify can label one watched inode through any recursively watched symlink.
// Add the canonical spelling from every live approved binding before routing
// to individual windows. Do not canonicalize event files: deleted entries no
// longer exist, and an event must never grant a new filesystem capability.
fn normalize_asset_event_aliases<'a>(
    bindings: impl Iterator<Item = (&'a Path, &'a AssetWatchState)>,
    exact_paths: &mut HashSet<PathBuf>,
    full_paths: &mut HashSet<PathBuf>,
) {
    if exact_paths.is_empty() && full_paths.is_empty() {
        return;
    }
    let mut exact_additions = HashSet::new();
    let mut full_additions = HashSet::new();
    for (graph_root, assets) in bindings {
        if !assets.active {
            continue;
        }
        let lexical = graph_root.join("assets");
        if lexical == assets.root {
            continue;
        }
        let map = |path: &Path| {
            let relative = path.strip_prefix(&lexical).ok()?;
            relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
                .then(|| assets.root.join(relative))
        };
        exact_additions.extend(exact_paths.iter().filter_map(|path| map(path)));
        for path in full_paths.iter() {
            if let Some(mapped) = map(path) {
                full_additions.insert(mapped);
            } else if lexical.starts_with(path) {
                full_additions.insert(assets.root.clone());
            }
        }
    }
    exact_paths.extend(exact_additions);
    full_paths.extend(full_additions);
}

fn asset_full_scan_owned(root: &Path, paths: &HashSet<PathBuf>) -> bool {
    paths
        .iter()
        .any(|path| path.starts_with(root) || root.starts_with(path))
}

fn reconcile_asset_observation(
    window_label: &str,
    state: &mut AssetWatchState,
    exact_paths: &HashSet<PathBuf>,
    full_paths: &HashSet<PathBuf>,
    force_full: bool,
    poll_mode: bool,
) -> Vec<String> {
    if !state.active {
        return Vec::new();
    }
    if !state.baseline {
        let snapshot = collect_asset_files(&state.root);
        state.snap = snapshot.files;
        state.baseline = true;
        return Vec::new();
    }

    // A read seed is an observation boundary, not a filesystem event. Compare
    // it to current metadata even if its intervening kernel event happened
    // before this watcher binding existed.
    let observed_reads = merge_recent_asset_reads(window_label, state);
    let need_full = force_full || poll_mode || asset_full_scan_owned(&state.root, full_paths);
    let mut changed = Vec::<(PathBuf, Option<FileStamp>)>::new();
    if need_full {
        let current = collect_asset_files(&state.root);
        for (path, stamp) in &current.files {
            if state.snap.get(path) != Some(stamp) {
                changed.push((path.clone(), Some(*stamp)));
            }
        }
        if current.complete {
            for path in state.snap.keys() {
                if !current.files.contains_key(path) {
                    changed.push((path.clone(), None));
                }
            }
            state.snap = current.files;
        } else {
            // A partial walk can prove present changes but never deletion.
            state.snap.extend(current.files);
        }
    } else {
        let mut exact = exact_paths.clone();
        exact.extend(observed_reads);
        for path in exact.iter().filter(|path| path.starts_with(&state.root)) {
            if asset_relative_event_path(&state.root, path).is_none() {
                continue;
            }
            let current = file_snapshot(path);
            if state.snap.get(path).copied() == current {
                continue;
            }
            match current {
                Some(stamp) => {
                    state.snap.insert(path.clone(), stamp);
                }
                None => {
                    state.snap.remove(path);
                }
            }
            changed.push((path.clone(), current));
        }
    }

    let mut logical = changed
        .into_iter()
        .filter(|(path, stamp)| !take_matching_asset_self_write(window_label, path, *stamp))
        .filter_map(|(path, _)| asset_relative_event_path(&state.root, &path))
        .collect::<Vec<_>>();
    logical.sort();
    logical.dedup();
    logical
}

fn collect_graph_text_files(graph: &Graph) -> GraphTextSnapshot {
    collect_scoped_text_files(
        &graph.root,
        &|path| graph.graph_text_watch_descend(path),
        &|path| graph.graph_text_watch_relevant(path),
    )
}

/// The stat sweep both regimes gate their poll cycle on.
///
/// `descend`/`relevant` are the only scope authority; the Direct lane passes the
/// graph's own `GraphTextScope` predicates and the managed lane passes the
/// deliberately widest ones (see `collect_managed_text_files`).
fn collect_scoped_text_files(
    root: &Path,
    descend: &dyn Fn(&Path) -> bool,
    relevant: &dyn Fn(&Path) -> bool,
) -> GraphTextSnapshot {
    let mut files: HashMap<PathBuf, FileStamp> = HashMap::new();
    let mut complete = true;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&directory) else {
            complete = false;
            continue;
        };
        for entry in read_dir {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            // `file_type`/`metadata` on a DirEntry do not traverse a symlink, so
            // a symlinked directory is never descended (no cycles, no escaping
            // the watched tree) and a `secret.md` symlink never contributes
            // outside bytes.
            let Ok(file_type) = entry.file_type() else {
                complete = false;
                continue;
            };
            if file_type.is_dir() {
                if descend(&path) {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() || !relevant(&path) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                complete = false;
                continue;
            };
            match metadata_stamp(&metadata) {
                Some(stamp) => {
                    files.insert(path, stamp);
                }
                None => complete = false,
            }
        }
    }
    GraphTextSnapshot { files, complete }
}

/// The managed lane has no `Graph` lease here — sparse-v2 owns its graph inside
/// the actor — so the poll gate cannot ask `Graph::graph_text_watch_relevant`
/// for the scope. It uses an **unconfigured** `GraphTextScope` instead, which is
/// a deliberate superset of every configured one: hidden prefixes only ever
/// remove paths, so a sweep with none configured covers every path any real
/// configuration could make eligible. Provider conflict copies are folded in for
/// the same reason the Direct predicate admits them.
///
/// A gate that is a superset can only over-arm a rescan. It cannot miss a change
/// the Direct lane's gate would catch, which is the one thing it must not do.
fn managed_poll_scope() -> &'static tine_core::graph_text_scope::GraphTextScope {
    static SCOPE: OnceLock<tine_core::graph_text_scope::GraphTextScope> = OnceLock::new();
    SCOPE.get_or_init(|| tine_core::graph_text_scope::GraphTextScope::new(&[], false))
}

fn managed_poll_relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let relative = relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
    (!relative.is_empty()).then_some(relative)
}

fn managed_poll_descend(root: &Path, path: &Path) -> bool {
    managed_poll_relative(root, path)
        .is_some_and(|relative| managed_poll_scope().should_descend(&relative))
}

fn managed_poll_relevant(root: &Path, path: &Path) -> bool {
    let Some(relative) = managed_poll_relative(root, path) else {
        return false;
    };
    if managed_poll_scope().is_eligible(&relative) {
        return true;
    }
    if !tine_core::model::path_is_sync_conflict(path) {
        return false;
    }
    let (parent, filename) = match relative.rsplit_once('/') {
        Some((parent, filename)) => (parent, filename),
        None => ("", relative.as_str()),
    };
    managed_poll_scope().should_descend(parent)
        && filename.rsplit_once('.').is_some_and(|(_, extension)| {
            extension.eq_ignore_ascii_case("md")
                || extension.eq_ignore_ascii_case("markdown")
                || extension.eq_ignore_ascii_case("org")
        })
}

fn collect_managed_text_files(root: &Path) -> GraphTextSnapshot {
    collect_scoped_text_files(root, &|path| managed_poll_descend(root, path), &|path| {
        managed_poll_relevant(root, path)
    })
}

/// One managed poll cycle's gate decision.
///
/// Returns true when this cycle must arm `RescanRequired`. The snapshot is
/// advanced in place exactly as `full_diff_reconcile` advances the Direct lane's,
/// so a delta is reported once and then becomes the new baseline.
fn managed_poll_rescan_required(
    root: &Path,
    snap: &mut HashMap<PathBuf, FileStamp>,
    baseline: &mut bool,
) -> bool {
    let current = collect_managed_text_files(root);
    // No baseline yet, or a walk that could not be read in full: arm. A partial
    // sweep must never be mistaken for "nothing changed" — same reason
    // `GraphTextSnapshot::complete` exists for the Direct lane.
    let armed = !*baseline || !current.complete || current.files != *snap;
    *snap = current.files;
    *baseline = true;
    armed
}

fn file_snapshot(path: &Path) -> Option<FileStamp> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() {
        return None;
    }
    metadata_stamp(&md)
}

fn full_diff_reconcile(
    graph: &Graph,
    snap: &mut HashMap<PathBuf, FileStamp>,
    mut current: HashMap<PathBuf, FileStamp>,
) -> (Vec<GraphChange>, bool, Vec<String>) {
    let mut changes: Vec<GraphChange> = Vec::new();
    let mut errors = Vec::new();
    let mut failed_paths = Vec::new();
    // A sync-tool conflict copy appearing/vanishing isn't a page change (it's
    // never cached), but the conflicts panel must refresh — track it and emit
    // `conflicts-changed` once.
    let mut conflicts_dirty = false;
    for (p, m) in &current {
        if snap.get(p) != Some(m) {
            let created = !snap.contains_key(p);
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_file_checked(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        created,
                        removed: false,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        failed_paths.push(p.clone());
                    }
                }
            }
        }
    }
    for p in snap.keys() {
        if !current.contains_key(p) {
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_deleted_file(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        created: false,
                        removed: true,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        failed_paths.push(p.clone());
                    }
                }
            }
        }
    }
    for path in failed_paths {
        match snap.get(&path).copied() {
            Some(previous) => {
                current.insert(path, previous);
            }
            None => {
                current.remove(&path);
            }
        }
    }
    *snap = current;
    (changes, conflicts_dirty, errors)
}

fn incremental_reconcile(
    graph: &Graph,
    snap: &mut HashMap<PathBuf, FileStamp>,
    paths: &HashSet<PathBuf>,
) -> (Vec<GraphChange>, bool, Vec<String>) {
    let mut changes: Vec<GraphChange> = Vec::new();
    let mut conflicts_dirty = false;
    let mut errors = Vec::new();

    // Reconcile present destinations before absent sources. A provider-delivered
    // external rename then lets the new path claim persisted block IDs before the
    // old page is tombstoned, preserving identity across the two snapshot events.
    let mut ordered: Vec<&PathBuf> = paths.iter().collect();
    ordered.sort_by_key(|path| file_snapshot(path).is_none());
    for p in ordered {
        if let Some(m) = file_snapshot(p) {
            let created = !snap.contains_key(p);
            // This path came from an explicit OS event. Always compare its
            // content even if a sync/copy tool preserved mtime and length;
            // the graph reconciliation already suppresses Tine's own/unchanged bytes.
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_file_checked(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        created,
                        removed: false,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        continue;
                    }
                }
            }
            snap.insert(p.clone(), m);
        } else if snap.contains_key(p) {
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_deleted_file(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        created: false,
                        removed: true,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        continue;
                    }
                }
            }
            snap.remove(p);
        }
    }

    (changes, conflicts_dirty, errors)
}

/// Every path whose on-disk stamp differs from the snapshot, in either
/// direction. This is what a completed graph-wide scan learned.
fn changed_since_snapshot(
    snap: &HashMap<PathBuf, FileStamp>,
    current: &HashMap<PathBuf, FileStamp>,
) -> Vec<PathBuf> {
    let mut changed: Vec<PathBuf> = current
        .iter()
        .filter(|(path, stamp)| snap.get(*path) != Some(*stamp))
        .map(|(path, _)| path.clone())
        .collect();
    changed.extend(
        snap.keys()
            .filter(|path| !current.contains_key(*path))
            .cloned(),
    );
    changed
}

/// Tell the guarded admission index what a poll-mode rescan just learned.
///
/// Poll mode has no per-event callback to keep that index current, so it used to
/// invalidate the whole thing at the top of every 3 s cycle. A poll-mode user
/// (NFS, SMB) could therefore never hold a warm index, and paid a full graph
/// rebuild on the next save — forever.
///
/// The rescan now covers exactly the scope the index covers (GH #268), so what
/// it found IS a complete account of what changed and can be applied as exact
/// observations. Publishing them here — after the scan, before reconciling
/// anything — keeps the index no staler than the scan that produced it, which is
/// the same window the inotify callback lane already has. A scan that could not
/// read part of the graph learned nothing complete, so it falls back to
/// invalidating, exactly as before.
fn publish_poll_observation(
    graph: &Graph,
    snap: &HashMap<PathBuf, FileStamp>,
    snapshot: &GraphTextSnapshot,
) {
    let (changed, uncertain) = poll_observation(snap, snapshot);
    let _ =
        graph.observe_graph_text_external_paths(changed.iter().map(PathBuf::as_path), uncertain);
}

/// Reduce a poll scan to the literal observation published to `tine-core`.
/// Keeping this calculation pure lets the watcher crate prove its routing
/// contract without requiring access to the core's private retained index.
fn poll_observation(
    snap: &HashMap<PathBuf, FileStamp>,
    snapshot: &GraphTextSnapshot,
) -> (Vec<PathBuf>, bool) {
    (
        changed_since_snapshot(snap, &snapshot.files),
        !snapshot.complete,
    )
}

fn reconcile_pending(
    graph: &Graph,
    snap: &mut HashMap<PathBuf, FileStamp>,
    paths: &HashSet<PathBuf>,
    need_full: bool,
    poll_mode: bool,
) -> (Vec<GraphChange>, bool, bool, Vec<String>) {
    if need_full || paths.is_empty() || burst_escalates(paths.len()) {
        let snapshot = collect_graph_text_files(graph);
        if poll_mode {
            publish_poll_observation(graph, snap, &snapshot);
        }
        let (changes, conflicts_dirty, errors) = full_diff_reconcile(graph, snap, snapshot.files);
        (changes, conflicts_dirty, true, errors)
    } else {
        let (changes, conflicts_dirty, errors) = incremental_reconcile(graph, snap, paths);
        (changes, conflicts_dirty, false, errors)
    }
}

/// Re-read `logseq/config.edn` for the graphs an event named, and refresh any
/// whose configuration actually moved.
///
/// A separate pass rather than a branch inside the reconcile loops, because
/// configuration is not graph text: it is the same plain file under both
/// storage engines -- never in `GraphTextScope`, never in the oplog, never
/// projected -- so one pass serves Direct and managed alike.
///
/// Returns true when a refresh was deferred and wants another cycle.
fn refresh_changed_configs(
    app: &tauri::AppHandle,
    labels_by_root: &[(String, PathBuf)],
    config_paths: &HashSet<PathBuf>,
    check_all: bool,
    recheck: &mut HashSet<String>,
    seen: &mut HashMap<PathBuf, Option<tine_core::model::ConfigDescription>>,
) -> bool {
    let mut deferred = false;
    let state = app.state::<AppState>();
    for (label, root) in labels_by_root {
        let named = check_all
            || recheck.contains(label)
            || config_paths
                .iter()
                .any(|path| tine_core::model::is_config_file_path(root, path));
        if !named {
            continue;
        }
        recheck.remove(label);
        let Ok(slot) = slot_for_window(&state, label) else {
            continue;
        };
        // A Direct graph carries a digest of the exact bytes it was opened
        // with, so this costs nothing after Tine's own settings write: that
        // command already refreshed the slot, and the reopened graph matches
        // disk. Skipping here is what keeps a settings toggle from paying for
        // a second whole-graph reopen -- which discards every cache the graph
        // has built.
        //
        // A managed slot retains no `Graph` to ask. Its refresh is a meta-only
        // reopen with no cache to lose, so it re-reads unconditionally and lets
        // the meta comparison below decide whether anything is worth announcing.
        let disk = tine_core::model::config_file_description(root);
        if slot.is_sparse_v2() {
            // A managed slot retains no `Graph` to interrogate, so the watcher
            // remembers the configuration it last saw for that root. Without
            // this, poll mode — which cannot name paths and therefore rechecks
            // every graph every cycle — would reopen a derived view every three
            // seconds forever.
            if seen.get(root).copied() == Some(disk) {
                continue;
            }
            seen.insert(root.clone(), disk);
        } else {
            let unchanged = slot.legacy_graph().is_ok_and(|lease| {
                // Either the graph was opened with these exact bytes, or it
                // published them itself. The second case is what keeps a star
                // toggled in the sidebar from reading as an outside change.
                lease.open_config_description() == disk || lease.recent_config_write() == disk
            });
            if unchanged {
                continue;
            }
        }
        let before = slot.graph_meta();
        drop(slot);
        match refresh_graph_for_label(&state, app, label, RefreshLaneWait::TryOnce) {
            Ok(RefreshOutcome::Deferred) => {
                recheck.insert(label.clone());
                deferred = true;
            }
            Ok(RefreshOutcome::Refreshed) => {
                let Ok(slot) = slot_for_window(&state, label) else {
                    continue;
                };
                let after = slot.graph_meta();
                // A rewrite that changed no setting we surface -- Logseq
                // touching an unrelated key, Syncthing redelivering identical
                // bytes with a new mtime -- announces nothing.
                if after != before {
                    let _ = app.emit_to(label, "graph-config-changed", after);
                }
            }
            Err(message) => {
                // Not silent: until this succeeds the window is serving stale
                // configuration, which is exactly the failure this whole pass
                // exists to prevent.
                let _ = app.emit_to(label, "graph-watch-error", &message);
            }
        }
    }
    deferred
}

/// Which pending event paths this graph should reconcile incrementally.
///
/// The predicate is the core's graph-wide text scope, not `journals/` +
/// `pages/`. Discovery went graph-wide in the #246 fix; event routing did not
/// follow, so an external edit to a page at the graph root or in a custom folder
/// was watched and delivered and then dropped here (GH #268).
fn pending_for_graph(paths: &HashSet<PathBuf>, graph: &Graph) -> HashSet<PathBuf> {
    paths
        .iter()
        .filter(|path| graph.graph_text_watch_relevant(path))
        .cloned()
        .collect()
}

/// Which pending *unclassified* paths this graph owns — directory moves and
/// event kinds the watcher cannot resolve. These force a full diff, so the
/// question is only whether anything eligible could live at or under the path;
/// an excluded tree (`assets/`, a dot-directory) answers no and costs nothing.
fn full_scan_owner_for_graph(paths: &HashSet<PathBuf>, graph: &Graph) -> HashSet<PathBuf> {
    paths
        .iter()
        .filter(|path| graph.graph_text_watch_could_contain(path))
        .cloned()
        .collect()
}

#[derive(Default)]
struct LegacyGraphTextObservation {
    exact_paths: Vec<PathBuf>,
    uncertain: bool,
    relevant: bool,
}

fn relative_graph_text_event_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty() {
        return None;
    }
    relative
        .to_str()
        .map(|relative| relative.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn legacy_graph_text_observation(
    graph: &Graph,
    root: &Path,
    event: Option<&notify::Event>,
) -> LegacyGraphTextObservation {
    use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind};

    let Some(event) = event else {
        return LegacyGraphTextObservation {
            uncertain: true,
            relevant: true,
            ..LegacyGraphTextObservation::default()
        };
    };
    if event.paths.is_empty() {
        return LegacyGraphTextObservation {
            uncertain: true,
            relevant: true,
            ..LegacyGraphTextObservation::default()
        };
    }

    let owned = event
        .paths
        .iter()
        .filter(|path| path.starts_with(root))
        .collect::<Vec<_>>();
    if owned.is_empty() {
        return LegacyGraphTextObservation::default();
    }

    let mut observation = LegacyGraphTextObservation {
        relevant: true,
        uncertain: event.need_rescan(),
        ..LegacyGraphTextObservation::default()
    };
    if observation.uncertain {
        return observation;
    }

    // Windows ReadDirectoryChangesW cannot report a sub-kind: notify maps every
    // non-rename Windows event to `Create(Any)` / `Modify(Any)` / `Remove(Any)`.
    // Those fell to the catch-all below and marked the observation uncertain,
    // which invalidates the ENTIRE guarded graph-text identity index -- so on
    // Windows any external file event made the next save rebuild the whole
    // graph. The event does carry exact paths; only the sub-kind is missing.
    //
    // Create/Modify are recoverable because the path still exists, so the arm
    // below discriminates file from directory against the live filesystem
    // exactly as it does for an explicit kind.
    //
    // `Remove(Any)` is NOT recoverable and deliberately stays uncertain: once
    // the entry is gone we cannot prove it was a file rather than a directory
    // of pages, and treating a removed directory as "not a page file" would
    // silently drop every page under it.
    let ambiguous_but_resolvable = matches!(
        event.kind,
        EventKind::Create(CreateKind::Any) | EventKind::Modify(ModifyKind::Any)
    );
    let explicit_file_event = ambiguous_but_resolvable
        || matches!(
            event.kind,
            EventKind::Create(CreateKind::File)
                | EventKind::Modify(ModifyKind::Data(_))
                | EventKind::Modify(ModifyKind::Metadata(_))
                | EventKind::Remove(RemoveKind::File)
        );
    let rename_event = matches!(event.kind, EventKind::Modify(ModifyKind::Name(_)));
    let rename_has_file_witness = rename_event
        && event
            .paths
            .iter()
            .any(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
        && !event
            .paths
            .iter()
            .any(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()));

    for path in owned {
        let Some(relative) = relative_graph_text_event_path(root, path) else {
            observation.uncertain = true;
            break;
        };
        let class = match graph.classify_graph_text_exact_feed_path(&relative) {
            Ok(class) => class,
            Err(_) => {
                observation.uncertain = true;
                break;
            }
        };
        match class {
            GraphTextExactFeedPathClass::Excluded => continue,
            GraphTextExactFeedPathClass::Configuration => {
                observation.uncertain = true;
                break;
            }
            GraphTextExactFeedPathClass::RetainedFile => {}
            _ => {
                observation.uncertain = true;
                break;
            }
        }
        let descendants_excluded = graph
            .classify_graph_text_exact_feed_path(&format!(
                "{relative}/__tine_watcher_descendant__.md"
            ))
            .is_ok_and(|class| class == GraphTextExactFeedPathClass::Excluded);

        if explicit_file_event {
            if path_is_existing_dir(path) {
                if descendants_excluded {
                    continue;
                }
                observation.uncertain = true;
                break;
            }
            if is_page_file_path(path) {
                observation.exact_paths.push(path.clone());
            }
        } else if rename_event {
            if !rename_has_file_witness {
                if descendants_excluded {
                    continue;
                }
                observation.uncertain = true;
                break;
            }
            if is_page_file_path(path) {
                observation.exact_paths.push(path.clone());
            }
        } else {
            if descendants_excluded {
                continue;
            }
            observation.uncertain = true;
            break;
        }
    }

    if observation.uncertain {
        observation.exact_paths.clear();
    }
    observation
}

fn observe_legacy_graph_text_event(
    graph: &Graph,
    root: &Path,
    event: Option<&notify::Event>,
) -> bool {
    let observation = legacy_graph_text_observation(graph, root, event);
    if !observation.relevant {
        return false;
    }
    // The raw platform callback is normally only an admission barrier. Reading
    // and semantically parsing arbitrary exact paths here defeated debounce, so
    // external paths still publish one O(1) pending epoch and are read/parsed by
    // the debounced reconciler. The one bounded exception is a candidate echo of
    // a completed Tine publication: core reopens that exact path twice under the
    // writer's identity + page locks and requires both its content revision and
    // physical identity to match Tine's publication receipt (or the identical
    // already-admitted cache state). Windows can emit Create(Any), Modify(Any)
    // and rename for one atomic publication; none of those self echoes should
    // strand the next new page. Any mismatch remains an external observation.
    // Only genuinely ambiguous events invalidate the retained identity index.
    if observation.uncertain {
        graph.note_graph_text_external_observation();
        let _ = graph.observe_graph_text_external_paths(std::iter::empty::<&Path>(), true);
    } else if !observation.exact_paths.is_empty() {
        let all_match_tine = observation
            .exact_paths
            .iter()
            .all(|path| graph.exact_graph_text_event_matches_tine_state(path));
        if !all_match_tine {
            graph.note_graph_text_external_observation();
        }
    }
    true
}

/// Linearize a platform callback with guarded graph-text writes before the
/// watcher's debounce/reconciliation delay. External callbacks publish an
/// admission epoch (and invalidate retained identity only when ambiguous) under
/// the same resource-scoped mutation authority that `Graph::save_page` uses.
/// Exact candidates for a Tine self echo take a bounded two-open identity+bytes
/// proof; debounced reconciliation still captures each final path once.
fn observe_legacy_graph_text_callback(
    app: &tauri::AppHandle,
    event: Option<&notify::Event>,
) -> Vec<(PathBuf, GraphTextExternalObservationTicket)> {
    let state = app.state::<AppState>();
    let entries = match state.graphs.read() {
        Ok(graphs) => graphs.entries(),
        Err(_) => return Vec::new(),
    };
    let mut observations = Vec::new();
    for (_, slot) in entries {
        let Ok((graph, root)) = direct_watch_paths(&slot) else {
            continue;
        };
        if observe_legacy_graph_text_event(&graph, &root, event) {
            observations.push((root, graph.graph_text_external_observation_ticket()));
        }
    }
    observations
}

fn sparse_observations(
    root: &Path,
    paths: &HashSet<PathBuf>,
    full_paths: &HashSet<PathBuf>,
    need_full: bool,
    notify_error: bool,
) -> Vec<SyncWatcherObservation> {
    let mut observations = Vec::new();
    let mut unknown = false;
    for path in paths.iter().chain(full_paths.iter()) {
        if !path.starts_with(root) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Some(relative) = relative
            .to_str()
            .map(|relative| relative.replace(std::path::MAIN_SEPARATOR, "/"))
        else {
            unknown = true;
            continue;
        };
        // Exact native callbacks must use the same fixed graph-text exclusions
        // as the poll/full-scan lane. Otherwise Tine's own Logseq backups (and
        // other excluded trees) bypass the scope as exact ManagedPath events,
        // then fail when reconciliation correctly refuses to read them.
        if !managed_poll_scope().should_descend(&relative) {
            continue;
        }
        let observation = managed_poll_relevant(root, path)
            .then(|| SyncWatcherObservation::managed_path(relative))
            .transpose()
            .ok()
            .flatten();
        match observation {
            Some(observation) => observations.push(observation),
            None => unknown = true,
        }
    }
    if unknown {
        observations.push(SyncWatcherObservation::UnknownPath);
    }
    if need_full {
        observations.push(SyncWatcherObservation::RescanRequired);
    }
    if notify_error {
        observations.push(SyncWatcherObservation::NotifyError);
    }
    observations
}

fn sparse_provider_observations(
    root: &Path,
    paths: &HashSet<PathBuf>,
    full_paths: &HashSet<PathBuf>,
) -> (Vec<String>, bool) {
    let provider = root.join(".tine-sync/v2/shared");
    let outbox = provider.join("outbox");
    let mut exact = Vec::new();
    let mut imprecise = full_paths.iter().any(|path| path.starts_with(&provider));
    for path in paths.iter().filter(|path| path.starts_with(&provider)) {
        let Ok(relative) = path.strip_prefix(&outbox) else {
            imprecise = true;
            continue;
        };
        let Some(relative) = relative.to_str() else {
            imprecise = true;
            continue;
        };
        let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
        let namespace = relative.split('/').next().unwrap_or_default();
        if matches!(namespace, ".part" | "removed" | "rename-evidence") {
            // These are transport-owned retry/retirement namespaces. Their
            // exact churn is not provider ingress and must not turn a local
            // commit-last rename into graph-wide reconciliation.
            continue;
        }
        if relative.is_empty()
            || !relative.contains('/')
            || relative.starts_with('/')
            || relative.contains('\\')
            || !matches!(
                namespace,
                "enrollment"
                    | "frontier-heads-v1"
                    | "publication-intents-v1"
                    | "manifest-recovery-links-v1"
                    | "manifest-recovery-blobs-v1"
                    | "manifests"
                    | "objects"
            )
            || relative
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            imprecise = true;
        } else {
            exact.push(relative);
        }
    }
    exact.sort();
    exact.dedup();
    (exact, imprecise)
}

fn sparse_provider_lane_is_active(
    shared_phase: Option<tine_core::sync_runtime::SyncSharedPhase>,
) -> bool {
    shared_phase == Some(tine_core::sync_runtime::SyncSharedPhase::Active)
}

/// Does this watcher turn owe the shared provider an imprecise observation?
///
/// Exact notify paths are sufficient while the app is running. They are not
/// sufficient for the first turn after a SharedActive actor is installed: a
/// file-sync provider may have delivered bytes while Tine was stopped, before
/// an inotify watch existed. That first turn must therefore scan the provider
/// namespace just as poll mode does. Local-only managed storage deliberately
/// remains outside the provider lane even if another device's namespace is
/// present in the graph.
fn sparse_provider_rescan_required(
    provider_lane_active: bool,
    initial_tick: bool,
    provider_imprecise: bool,
    provider_poll: bool,
) -> bool {
    provider_lane_active && (initial_tick || provider_imprecise || provider_poll)
}

/// A graph reconciliation and a provider rescan may be queued in the same
/// watcher turn. The actor deliberately admits graph bytes first, so an
/// `Admitted*` result from that turn does not mean the provider obligation was
/// consumed. Schedule exactly one continuation; once provider work begins its
/// ordinary `Recovering` result owns subsequent continuation turns.
///
/// `actor_has_runnable_work` is the durable half of that rule
/// (`SyncRuntimeStatusSnapshot::has_runnable_work`): work the actor already
/// KNOWS about — a newer watcher epoch queued behind a completed scan, or
/// provider evidence another device delivered as bytes on disk — is itself a
/// runnable work source. One tick's result describes only the lane that tick
/// took, so a receiving device whose tick settled a watcher epoch can still be
/// holding delivered provider manifests. Nothing on a quiet graph will produce a
/// later filesystem edge to wake them, so a scheduler that consulted only the
/// tick result slept forever with the peer's edit undelivered.
fn sparse_tick_needs_continuation(
    tick: &SyncRuntimeTick,
    provider_rescan_queued: bool,
    actor_has_runnable_work: bool,
) -> bool {
    actor_has_runnable_work
        || matches!(
            tick,
            SyncRuntimeTick::LocalMutation(_)
                | SyncRuntimeTick::ProviderMutation { .. }
                | SyncRuntimeTick::Recovering
                | SyncRuntimeTick::RetryFull
        )
        || (provider_rescan_queued
            && matches!(
                tick,
                SyncRuntimeTick::Idle
                    | SyncRuntimeTick::AdmittedNoop { .. }
                    | SyncRuntimeTick::AdmittedComplete { .. }
            ))
}

/// The anti-hot-loop half of the same contract.
///
/// The provider lane reports `Idle` when its ready queue is empty but pending
/// batches remain blocked on causal dependencies whose bytes have not been
/// delivered yet (`tick_provider`'s `ready_front()` miss). That is known work no
/// tick can advance right now, so the 10ms progress cadence would become a poll
/// loop against the disk. Retry it on the ordinary backoff schedule instead —
/// bounded polling, never permanent sleep, and identical to how a
/// `RecoveryBlocked` provider turn is already paced.
fn sparse_tick_is_blocked_without_progress(
    tick: &SyncRuntimeTick,
    actor_has_runnable_work: bool,
) -> bool {
    actor_has_runnable_work && matches!(tick, SyncRuntimeTick::Idle)
}

/// Has the condition the last emitted error described actually ended?
///
/// One tick settles ONE lane. A blocked provider lane and a healthy local lane
/// therefore interleave: `RecoveryBlocked`, `Idle`, `RecoveryBlocked`, … Reading
/// any non-blocked tick as "the failure is over" reset the repeat suppression
/// on every cycle, so ONE permanently blocked condition emitted a fresh
/// `sparse-v2-error` — and a fresh red toast — for as long as it lasted. The
/// frontend's own notice de-duplication (`managedStorageRuntime.ts`) cannot see
/// past this: it is handed genuinely new error events and correctly reports
/// each one.
///
/// `actor_has_runnable_work` is the durable signal that the actor still knows
/// about work it has not been able to finish, which is exactly the state a
/// blocked lane holds. A graph that truly recovered drains to no runnable work,
/// and the next failure after that reports again.
fn sparse_error_condition_ended(tick: &SyncRuntimeTick, actor_has_runnable_work: bool) -> bool {
    !matches!(
        tick,
        SyncRuntimeTick::RecoveryBlocked(_)
            | SyncRuntimeTick::Blocked(_)
            | SyncRuntimeTick::Terminal(_)
            | SyncRuntimeTick::Failed(_)
    ) && !actor_has_runnable_work
}

fn sparse_tick_is_reportable_failure(
    tick: &SyncRuntimeTick,
    waiting_for_recovery_delivery: bool,
) -> bool {
    matches!(
        tick,
        SyncRuntimeTick::RecoveryBlocked(_)
            | SyncRuntimeTick::Blocked(_)
            | SyncRuntimeTick::Terminal(_)
            | SyncRuntimeTick::Failed(_)
    ) && !(waiting_for_recovery_delivery && matches!(tick, SyncRuntimeTick::RecoveryBlocked(_)))
}

fn take_sparse_initial_tick(pending: &mut bool) -> bool {
    std::mem::take(pending)
}

struct WatchedGraph {
    legacy_graph: LegacyGraphLease,
    root: PathBuf,
    assets: AssetWatchState,
    snap: HashMap<PathBuf, FileStamp>,
    baseline: bool,
    last_reconcile_error: Option<String>,
    retry: RetrySchedule,
    /// Frontier already drained from `Pending` but not yet reconciled
    /// successfully. It survives retry cycles and is acknowledged only after
    /// the matching graph batch succeeds.
    pending_observation_epoch: Option<GraphTextExternalObservationTicket>,
}

fn asset_root_for_slot(app: &tauri::AppHandle, slot: &GraphSlot) -> Option<PathBuf> {
    let root = &slot.root_key;
    match Graph::external_assets_target(root) {
        Ok(Some(live)) => crate::settings::approved_external_assets(app, root)
            .and_then(|approved| std::fs::canonicalize(approved).ok())
            .filter(|approved| approved == &live)
            .map(|_| live),
        Ok(None) => Some(root.join("assets")),
        Err(_) => None,
    }
}

fn route_drained_direct_frontiers(
    graphs: &mut HashMap<String, WatchedGraph>,
    latest_entries: Vec<(String, Arc<GraphSlot>)>,
    drained: &HashMap<PathBuf, GraphTextExternalObservationTicket>,
    asset_root_for: impl Fn(&GraphSlot) -> Option<PathBuf>,
) {
    for (label, slot) in latest_entries {
        let asset_root = asset_root_for(&slot);
        let Ok((latest_graph, root)) = direct_watch_paths(&slot) else {
            continue;
        };
        let Some(ticket) = drained.get(&root).copied() else {
            continue;
        };
        if !latest_graph.owns_graph_text_external_observation_ticket(ticket) {
            continue;
        }
        match graphs.get_mut(&label) {
            Some(current) if current.root == root => {
                if !current
                    .legacy_graph
                    .owns_graph_text_external_observation_ticket(ticket)
                {
                    current.assets = asset_root
                        .clone()
                        .map(AssetWatchState::new)
                        .unwrap_or_default();
                    current.legacy_graph = latest_graph;
                    current.snap.clear();
                    current.baseline = false;
                    current.last_reconcile_error = None;
                    current.retry = RetrySchedule::default();
                    current.pending_observation_epoch = None;
                }
            }
            _ => {
                graphs.insert(
                    label,
                    WatchedGraph {
                        assets: asset_root
                            .clone()
                            .map(AssetWatchState::new)
                            .unwrap_or_default(),
                        legacy_graph: latest_graph,
                        root,
                        snap: HashMap::new(),
                        baseline: false,
                        last_reconcile_error: None,
                        retry: RetrySchedule::default(),
                        pending_observation_epoch: None,
                    },
                );
            }
        }
    }
}

/// Watch the graph dirs for external changes (Logseq, Syncthing) and reconcile
/// them into the cache, emitting `graph-changed` so the UI can reload. Two
/// mechanisms, switchable at runtime via the device-local `watch_mode` setting:
///
///   - **"inotify" (default):** a real OS filesystem watcher (the `notify`
///     crate — inotify on Linux). Idle = *zero* periodic wakeups; the thread
///     blocks until the kernel reports a change. Matches OG Logseq (chokidar)
///     and is the right choice on a normal local disk.
///   - **"poll":** a 3-second mtime scan. Robust on filesystems where inotify is
///     unreliable (some NFS / network mounts), at the cost of constant periodic
///     wakeups. Use this only when inotify misses external edits.
///
/// In both modes the reconcile is identical and suppresses Tine's *own* writes
/// via the cache comparison inside `sync_file`. A control channel (poked by
/// `load_graph` on a graph switch and by `set_watch_mode`) lets the thread
/// re-target or switch mechanism at once, without polling for those either.
pub(crate) fn start_watcher(app: tauri::AppHandle) {
    use notify::Watcher;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let pending = Arc::new(Mutex::new(Pending::default()));
    // The roots the OS watcher currently covers, shared with its callback so
    // the callback can drop VCS/tool noise before it costs anything. Written by
    // the loop each cycle, read once per event.
    let watched_roots: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
    if let Ok(mut slot) = app.state::<AppState>().watch_ctl.lock() {
        *slot = Some(tx.clone());
    }
    std::thread::spawn(move || {
        struct WatchedSparse {
            handle: SyncRuntimeHandle,
            root: PathBuf,
            assets: AssetWatchState,
            binding_generation: u64,
            last_error: Option<String>,
            last_history_recovery_diagnostic: Option<SyncHistoryRecoveryStatus>,
            last_checkpoint_diagnostic: Option<SyncCheckpointPublicationDiagnostics>,
            retry: RetrySchedule,
            initial_tick_pending: bool,
            sweep_deadline_remaining: Option<Duration>,
            // The managed twin of `WatchedGraph::snap`/`baseline`. Poll mode
            // used to push `RescanRequired` every cycle unconditionally; it now
            // arms on the same (mtime, len) stat diff the Direct lane has always
            // used.
            snap: HashMap<PathBuf, FileStamp>,
            baseline: bool,
        }

        // Last observed disposable image per live window binding. This only
        // coalesces UI invalidations; no query ever waits on these revisions.
        let mut query_images: HashMap<String, (u64, Option<u64>)> = HashMap::new();
        let mut graphs: HashMap<String, WatchedGraph> = HashMap::new();
        let mut sparse_graphs: HashMap<String, WatchedSparse> = HashMap::new();
        // Windows whose configuration still needs re-reading: named by an event
        // this cycle could not act on because the storage transition lane was
        // busy. Carried across cycles so a deferral cannot lose the change.
        let mut config_recheck: HashSet<String> = HashSet::new();
        // Last configuration seen per MANAGED root. Direct graphs need no such
        // memory: their own `Graph` instance is the witness.
        let mut config_seen: HashMap<PathBuf, Option<tine_core::model::ConfigDescription>> =
            HashMap::new();
        let mut watcher: Option<notify::RecommendedWatcher> = None;
        let mut watched: HashSet<PathBuf> = HashSet::new();
        // Last surfaced `watch()` failure per graph root, so a root that keeps
        // failing reports once instead of every cycle.
        let mut watch_failures: HashMap<PathBuf, String> = HashMap::new();
        let mut forced_sparse_tick = false;
        loop {
            let force_sparse_tick = std::mem::take(&mut forced_sparse_tick);
            let inotify = watch_mode(&app) != "poll";
            let entries = app.state::<AppState>().graphs.read().unwrap().entries();
            let live: HashSet<String> = entries.iter().map(|(label, _)| label.clone()).collect();
            query_images.retain(|label, _| live.contains(label));
            graphs.retain(|label, _| live.contains(label));
            sparse_graphs.retain(|label, _| live.contains(label));
            for (label, slot) in entries {
                let asset_root = asset_root_for_slot(&app, &slot);
                if let Some(handle) = slot.sparse_runtime().cloned() {
                    query_images.remove(&label);
                    graphs.remove(&label);
                    match sparse_graphs.get_mut(&label) {
                        Some(current)
                            if current.root == slot.root_key
                                && current.binding_generation == slot.binding_generation =>
                        {
                            current.handle = handle;
                            if asset_root.as_ref() != current.assets.active_root() {
                                current.assets = asset_root
                                    .clone()
                                    .map(AssetWatchState::new)
                                    .unwrap_or_default();
                            }
                        }
                        _ => {
                            relay_absence_sweep_events(
                                &app,
                                &label,
                                slot.binding_generation,
                                &handle,
                            );
                            sparse_graphs.insert(
                                label,
                                WatchedSparse {
                                    handle,
                                    root: slot.root_key.clone(),
                                    assets: asset_root
                                        .clone()
                                        .map(AssetWatchState::new)
                                        .unwrap_or_default(),
                                    binding_generation: slot.binding_generation,
                                    last_error: None,
                                    last_history_recovery_diagnostic: None,
                                    last_checkpoint_diagnostic: None,
                                    retry: RetrySchedule::default(),
                                    // Activation has already proved the exact
                                    // managed inventory. Start the mandatory
                                    // watcher-handoff scan immediately; the
                                    // actor now advances it in bounded turns,
                                    // so application and enrollment work can
                                    // run between those turns.
                                    initial_tick_pending: true,
                                    sweep_deadline_remaining: None,
                                    snap: HashMap::new(),
                                    baseline: false,
                                },
                            );
                        }
                    }
                    continue;
                }
                sparse_graphs.remove(&label);
                let Ok((legacy_graph, root)) = direct_watch_paths(&slot) else {
                    // Sparse-v2 owns its actor in the slot. This legacy watcher
                    // must not retain or reopen a Graph for it.
                    graphs.remove(&label);
                    continue;
                };
                let image = legacy_graph.observe_direct_projection_commits(tx.clone());
                let observed = (slot.binding_generation, image);
                if query_images.insert(label.clone(), observed) != Some(observed) && image.is_some()
                {
                    let _ =
                        app.emit_to(&label, "query-projection-changed", slot.binding_generation);
                }
                match graphs.get_mut(&label) {
                    Some(current) if current.root == root => {
                        if current.pending_observation_epoch.is_some_and(|ticket| {
                            !legacy_graph.owns_graph_text_external_observation_ticket(ticket)
                        }) {
                            current.pending_observation_epoch = None;
                        }
                        current.legacy_graph = legacy_graph;
                        if asset_root.as_ref() != current.assets.active_root() {
                            current.assets = asset_root
                                .clone()
                                .map(AssetWatchState::new)
                                .unwrap_or_default();
                        }
                    }
                    _ => {
                        graphs.insert(
                            label,
                            WatchedGraph {
                                assets: asset_root
                                    .clone()
                                    .map(AssetWatchState::new)
                                    .unwrap_or_default(),
                                legacy_graph,
                                root,
                                snap: HashMap::new(),
                                baseline: false,
                                last_reconcile_error: None,
                                retry: RetrySchedule::default(),
                                pending_observation_epoch: None,
                            },
                        );
                    }
                }
            }

            let labels_by_root: Vec<(String, PathBuf)> = graphs
                .iter()
                .map(|(label, graph)| (label.clone(), graph.root.clone()))
                .chain(
                    sparse_graphs
                        .iter()
                        .map(|(label, graph)| (label.clone(), graph.root.clone())),
                )
                .collect();
            let watch_labels = labels_by_root
                .iter()
                .cloned()
                .chain(
                    graphs
                        .iter()
                        .filter(|(_, graph)| {
                            graph.assets.active && !graph.assets.root.starts_with(&graph.root)
                        })
                        .map(|(label, graph)| (label.clone(), graph.assets.root.clone())),
                )
                .chain(
                    sparse_graphs
                        .iter()
                        .filter(|(_, graph)| {
                            graph.assets.active && !graph.assets.root.starts_with(&graph.root)
                        })
                        .map(|(label, graph)| (label.clone(), graph.assets.root.clone())),
                )
                .collect::<Vec<_>>();
            let desired: HashSet<PathBuf> =
                watch_labels.iter().map(|(_, root)| root.clone()).collect();
            if let Ok(mut roots) = watched_roots.lock() {
                if *roots != desired {
                    roots.clone_from(&desired);
                }
            }

            // Bring the OS watcher in line with the current mode + graph roots.
            let mut newly_watched = HashSet::new();
            if inotify {
                if watcher.is_none() {
                    let txc = tx.clone();
                    let pendingc = pending.clone();
                    let appc = app.clone();
                    let rootsc = watched_roots.clone();
                    watcher =
                        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                            // A repository's own churn is not a graph change.
                            // Dropped here, before the app-state lock, the graph
                            // leases, the scope classifications and the wake.
                            if let Ok(event) = &res {
                                if rootsc
                                    .lock()
                                    .is_ok_and(|roots| watch_event_is_tool_noise(event, &roots))
                                {
                                    return;
                                }
                            }
                            if let Ok(mut p) = pendingc.lock() {
                                let observations = match &res {
                                    Ok(event) => {
                                        observe_legacy_graph_text_callback(&appc, Some(event))
                                    }
                                    Err(_) => observe_legacy_graph_text_callback(&appc, None),
                                };
                                p.add_legacy_observations(observations);
                                match res {
                                    Ok(event) => p.add_event(event),
                                    Err(_) => p.add_notify_error(),
                                }
                            }
                            let _ = txc.send(());
                        })
                        .ok();
                    watched.clear();
                }
                if let Some(w) = watcher.as_mut() {
                    for dir in watched.difference(&desired).cloned().collect::<Vec<_>>() {
                        let _ = w.unwatch(&dir);
                        watched.remove(&dir);
                    }
                    for dir in desired.difference(&watched).cloned().collect::<Vec<_>>() {
                        // Recursive so the guarded-identity boundary includes
                        // eligible graph text outside configured cache roots.
                        match w.watch(&dir, notify::RecursiveMode::Recursive) {
                            Ok(()) => {
                                watch_failures.remove(&dir);
                                newly_watched.insert(dir.clone());
                                watched.insert(dir);
                            }
                            Err(error) => {
                                // A failure here is retried next cycle (the root
                                // is never inserted into `watched`), so this is
                                // not permanent -- but it was completely silent,
                                // and until it succeeds every external change to
                                // that graph is invisible. Surface it once per
                                // distinct message so a retry loop cannot spam.
                                let message = error.to_string();
                                if watch_failures.get(&dir) != Some(&message) {
                                    for (label, root) in watch_labels.iter() {
                                        if root == &dir {
                                            let _ =
                                                app.emit_to(label, "graph-watch-error", &message);
                                        }
                                    }
                                    watch_failures.insert(dir, message);
                                }
                            }
                        }
                    }
                }
            } else if watcher.is_some() {
                watcher = None; // poll mode → release the OS watcher
                watched.clear();
                watch_failures.clear();
            }
            watch_failures.retain(|dir, _| desired.contains(dir));

            // --- reconcile (identical in both modes) ---
            let (
                paths,
                full_paths,
                asset_paths,
                asset_full_paths,
                config_paths,
                drained_observation_epochs,
                event_need_full,
                notify_error,
                first_event_at,
            ) = if inotify {
                if let Ok(mut p) = pending.lock() {
                    let paths = std::mem::take(&mut p.paths);
                    let full_paths = std::mem::take(&mut p.full_paths);
                    let asset_paths = std::mem::take(&mut p.asset_paths);
                    let asset_full_paths = std::mem::take(&mut p.asset_full_paths);
                    let config_paths = std::mem::take(&mut p.config_paths);
                    let observation_epochs = p.take_legacy_observation_epochs();
                    let need_full = p.need_full;
                    let notify_error = p.notify_error;
                    let first_event_at = p.first_event_at.take();
                    p.need_full = false;
                    p.notify_error = false;
                    (
                        paths,
                        full_paths,
                        asset_paths,
                        asset_full_paths,
                        config_paths,
                        observation_epochs,
                        need_full,
                        notify_error,
                        first_event_at,
                    )
                } else {
                    (
                        HashSet::new(),
                        HashSet::new(),
                        HashSet::new(),
                        HashSet::new(),
                        HashSet::new(),
                        HashMap::new(),
                        true,
                        true,
                        None,
                    )
                }
            } else {
                (
                    HashSet::new(),
                    HashSet::new(),
                    HashSet::new(),
                    HashSet::new(),
                    HashSet::new(),
                    HashMap::new(),
                    false,
                    false,
                    None,
                )
            };
            // The callback reads AppState independently from this loop. A
            // same-root refresh can therefore publish a ticket for the new
            // Graph after this cycle took its initial slot snapshot but before
            // it drained Pending. Re-read only when a Direct frontier was
            // drained and route it to the exact instance that minted it. A
            // stale WatchedGraph must never consume the path while silently
            // discarding the replacement's ticket.
            if !drained_observation_epochs.is_empty() {
                let latest_entries = app.state::<AppState>().graphs.read().unwrap().entries();
                route_drained_direct_frontiers(
                    &mut graphs,
                    latest_entries,
                    &drained_observation_epochs,
                    |slot| asset_root_for_slot(&app, slot),
                );
            }
            // A focus-driven rescan demands the same full stat diff a kernel
            // rescan does, for the Direct lane and the managed lane alike.
            let explicit_rescan = pending_full_rescan();
            let event_need_full = event_need_full || explicit_rescan.is_some();

            let mut asset_paths = asset_paths;
            let mut asset_full_paths = asset_full_paths;
            normalize_asset_event_aliases(
                graphs
                    .values()
                    .map(|graph| (graph.root.as_path(), &graph.assets))
                    .chain(
                        sparse_graphs
                            .values()
                            .map(|graph| (graph.root.as_path(), &graph.assets)),
                    ),
                &mut asset_paths,
                &mut asset_full_paths,
            );

            // Assets are ordinary externally synchronized files, not managed
            // graph text. Observe only their metadata here and emit one
            // assets-relative cache-invalidation batch; this lane never calls
            // Graph reconciliation, the sparse actor, or the provider API.
            for (label, graph) in graphs.iter_mut() {
                let watch_handoff = newly_watched
                    .iter()
                    .any(|root| graph.assets.root.starts_with(root));
                let changed = reconcile_asset_observation(
                    label,
                    &mut graph.assets,
                    &asset_paths,
                    &asset_full_paths,
                    event_need_full || notify_error || watch_handoff,
                    !inotify,
                );
                if !changed.is_empty() {
                    if crate::debug::debug_enabled() {
                        crate::debug::diag(format!(
                            "asset observer emitting {} invalidation(s)",
                            changed.len()
                        ));
                    }
                    let _ =
                        app.emit_to(label, "asset-changed", AssetChangedBatch { paths: changed });
                }
            }
            for (label, graph) in sparse_graphs.iter_mut() {
                let watch_handoff = newly_watched
                    .iter()
                    .any(|root| graph.assets.root.starts_with(root));
                let changed = reconcile_asset_observation(
                    label,
                    &mut graph.assets,
                    &asset_paths,
                    &asset_full_paths,
                    event_need_full || notify_error || watch_handoff,
                    !inotify,
                );
                if !changed.is_empty() {
                    if crate::debug::debug_enabled() {
                        crate::debug::diag(format!(
                            "asset observer emitting {} invalidation(s)",
                            changed.len()
                        ));
                    }
                    let _ =
                        app.emit_to(label, "asset-changed", AssetChangedBatch { paths: changed });
                }
            }
            for (label, graph) in graphs.iter_mut() {
                if let Some(epoch) = drained_observation_epochs.get(&graph.root).copied() {
                    if graph
                        .legacy_graph
                        .owns_graph_text_external_observation_ticket(epoch)
                    {
                        graph.pending_observation_epoch =
                            Some(graph.pending_observation_epoch.map_or(epoch, |pending| {
                                pending.later_for_same_instance(epoch).unwrap_or(epoch)
                            }));
                    }
                }
                let initial_cycle = !graph.baseline;
                if initial_cycle {
                    // No baseline yet, so nothing about the graph's text identity
                    // is known. Once per graph, not once per cycle.
                    let _ = graph
                        .legacy_graph
                        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true);
                    graph.snap = collect_graph_text_files(&graph.legacy_graph).files;
                    graph.baseline = true;
                }
                let retry_due = graph.retry.take_due(Instant::now());
                let owned = pending_for_graph(&paths, &graph.legacy_graph);
                let full_owned = full_scan_owner_for_graph(&full_paths, &graph.legacy_graph);
                let need_full = event_need_full || !inotify || !full_owned.is_empty() || retry_due;
                let mut cycle_failed = false;
                let mut attempted = false;
                if need_full || !owned.is_empty() {
                    attempted = true;
                    let reconcile_started = Instant::now();
                    let (changes, conflicts_dirty, used_full, errors) = reconcile_pending(
                        &graph.legacy_graph,
                        &mut graph.snap,
                        &owned,
                        need_full,
                        !inotify,
                    );
                    let pages = changes.len();
                    if emit_as_bulk(pages) {
                        // One epoch, one notification: the frontend answers with
                        // one dataRev bump and reloads only visible pages,
                        // instead of N events → N bumps → up to N getPage IPCs.
                        let _ =
                            app.emit_to(label, "graph-changed-bulk", GraphChangedBulk { changes });
                    } else {
                        for change in changes {
                            let _ = app.emit_to(label, "graph-changed", change);
                        }
                    }
                    // Receipts only for batches that surfaced something: a
                    // change reaching the frontend, or an error scheduling a
                    // backoff retry (itself a latency source). Quiet cycles —
                    // echo-suppressed self-writes, poll scans that found
                    // nothing — would drown the 64-slot ring in no-ops.
                    if pages > 0 || !errors.is_empty() {
                        record_latency_receipt(latency_receipt(
                            label,
                            inotify,
                            pages,
                            owned.len(),
                            // The branch actually taken — a burst-escalated
                            // batch reads as full_diff in the receipt trail.
                            used_full,
                            errors.len(),
                            if initial_cycle { None } else { first_event_at },
                            reconcile_started,
                            Instant::now(),
                        ));
                    }
                    if !errors.is_empty() {
                        cycle_failed = true;
                        let message = errors.join("; ");
                        if graph.last_reconcile_error.as_deref() != Some(&message) {
                            // This is a Direct Files reconcile failure. Sparse-v2
                            // bindings are handled by their actor lane below.
                            let _ = app.emit_to(label, "graph-watch-error", &message);
                            graph.last_reconcile_error = Some(message);
                        }
                    }
                    if conflicts_dirty {
                        let _ = app.emit_to(label, "conflicts-changed", ());
                    }
                }
                if cycle_failed {
                    graph.retry.failed(Instant::now());
                } else if attempted {
                    graph.retry.succeeded();
                    graph.last_reconcile_error = None;
                    if let Some(epoch) = graph.pending_observation_epoch.take() {
                        graph
                            .legacy_graph
                            .acknowledge_graph_text_external_observations(epoch);
                    }
                }
            }
            for (label, graph) in sparse_graphs.iter_mut() {
                let retry_due = graph.retry.take_due(Instant::now());
                let initial_tick = take_sparse_initial_tick(&mut graph.initial_tick_pending);
                let poll_cycle = !inotify && !retry_due;
                // The managed poll gate. Before this, every poll cycle armed a
                // whole-graph `RescanRequired` whether or not anything on disk
                // had changed, so a quiet graph paid a full actor-lane rescan
                // every cycle. The Direct lane thirty lines above has always
                // gated on a (mtime, len) stat diff; this is that same contract,
                // not a second mechanism. Kept unconditional on `initial_tick`,
                // on `event_need_full` (which carries the `rescan_graph_now`
                // focus refresh), and on an incomplete sweep.
                let poll_rescan = if inotify {
                    // Poll snapshots go stale the moment the OS watcher owns the
                    // graph; drop the baseline so a later switch back to poll
                    // re-arms once instead of trusting a stale sweep.
                    graph.baseline = false;
                    graph.snap.clear();
                    false
                } else {
                    managed_poll_rescan_required(&graph.root, &mut graph.snap, &mut graph.baseline)
                };
                // A graph may contain another device's shared provider tree
                // while this device has enabled only local managed storage.
                // Provider callbacks are advisory and belong exclusively to a
                // SharedActive actor; routing poll/event noise to a local-only
                // actor makes the actor correctly refuse it, but then turns a
                // harmless on-disk namespace into an endless retry/toast loop.
                // Ignore provider observations until this device has actually
                // joined or initiated sharing. The SharedActive transition
                // schedules its own initial provider scan, and later poll or
                // exact events continue through this lane.
                let actor_status = graph.handle.status().ok();
                graph.sweep_deadline_remaining = actor_status
                    .as_ref()
                    .and_then(|status| status.sweep_deadline_remaining);
                let sweep_deadline_due = actor_status
                    .as_ref()
                    .is_some_and(|status| status.sweep_deadline_due);
                let provider_lane_active = actor_status
                    .as_ref()
                    .is_some_and(|status| sparse_provider_lane_is_active(status.shared_phase));
                // The actor's startup scan can finish before this thread has
                // replaced the legacy directory watches with the recursive
                // graph-root watch. One scan after watch installation closes
                // that handoff interval; later steady-state events stay exact.
                let observations = sparse_observations(
                    &graph.root,
                    &paths,
                    &full_paths,
                    event_need_full || initial_tick || poll_rescan,
                    notify_error,
                );
                let (provider_paths, provider_imprecise) =
                    sparse_provider_observations(&graph.root, &paths, &full_paths);
                let provider_poll = poll_cycle && provider_lane_active;
                let provider_rescan = sparse_provider_rescan_required(
                    provider_lane_active,
                    initial_tick,
                    provider_imprecise,
                    provider_poll,
                );
                if observations.is_empty()
                    && provider_paths.is_empty()
                    && !provider_imprecise
                    && !provider_poll
                    && !retry_due
                    && !initial_tick
                    && !force_sparse_tick
                    && !sweep_deadline_due
                {
                    continue;
                }

                let result = (|| {
                    if provider_lane_active && (!provider_paths.is_empty() || provider_rescan) {
                        graph
                            .handle
                            .observe_provider_paths(provider_paths, provider_rescan)?;
                    }
                    if !observations.is_empty() {
                        graph.handle.observe_watcher(observations)?;
                    }
                    graph.handle.tick()
                })();
                match result {
                    Ok(tick) => {
                        // Completed local, provider and watcher batches wake
                        // readers; pending recovery and quiet admissions do not.
                        let changed = tick.committed_observable_change();
                        // A bounded tick settles one lane. The tick result
                        // describes only that lane, so continuation must also
                        // consult the actor's post-tick status: a newer watcher
                        // epoch can be queued behind a completed full scan, and
                        // provider evidence another device delivered as bytes on
                        // disk is work this device never performed and no
                        // inotify edge will announce again. Otherwise a quiet
                        // graph sleeps holding a peer's edit forever.
                        let status = graph.handle.status().ok();
                        let actor_has_runnable_work = status
                            .as_ref()
                            .is_some_and(SyncRuntimeStatusSnapshot::has_runnable_work);
                        let waiting_for_delivery = status.as_ref().is_some_and(|status| {
                            status.history_recovery.as_ref().is_some_and(|recovery| {
                                recovery.phase == SyncHistoryRecoveryPhase::WaitingForDelivery
                            })
                        });
                        match &tick {
                            SyncRuntimeTick::RecoveryBlocked(_) | SyncRuntimeTick::Failed(_) => {
                                graph.retry.failed(Instant::now())
                            }
                            tick if sparse_tick_is_blocked_without_progress(
                                tick,
                                actor_has_runnable_work,
                            ) =>
                            {
                                graph.retry.failed(Instant::now())
                            }
                            tick if sparse_tick_needs_continuation(
                                tick,
                                provider_rescan,
                                actor_has_runnable_work,
                            ) =>
                            {
                                graph.retry.progressed(Instant::now())
                            }
                            _ => graph.retry.succeeded(),
                        }
                        if waiting_for_delivery
                            && matches!(tick, SyncRuntimeTick::RecoveryBlocked(_))
                        {
                            // Missing peer delivery is a live wait condition,
                            // not a failed retry. The paced tick loop remains
                            // responsible for polling; there is no inert button.
                            graph.last_error = None;
                        } else if sparse_tick_is_reportable_failure(&tick, waiting_for_delivery) {
                            let message = format!("{tick:?}");
                            if graph.last_error.as_deref() != Some(&message) {
                                let _ = app.emit_to(
                                    label,
                                    "sparse-v2-error",
                                    SparseV2ErrorEvent {
                                        binding_generation: graph.binding_generation,
                                        message: message.clone(),
                                    },
                                );
                                graph.last_error = Some(message);
                            }
                        } else if sparse_error_condition_ended(&tick, actor_has_runnable_work) {
                            graph.last_error = None;
                        }
                        let _ = app.emit_to(
                            label,
                            "sparse-v2-tick",
                            SparseV2TickEvent {
                                binding_generation: graph.binding_generation,
                                tick: crate::sync_runtime::tick_dto(tick),
                            },
                        );
                        if changed {
                            let _ = app.emit_to(label, "sparse-v2-changed", ());
                        }
                        if let Some(status) = status {
                            if status.history_recovery_diagnostics
                                != graph.last_history_recovery_diagnostic
                            {
                                if let Some(recovery) = &status.history_recovery_diagnostics {
                                    crate::debug::record_history_recovery(recovery);
                                }
                                graph.last_history_recovery_diagnostic =
                                    status.history_recovery_diagnostics.clone();
                            }
                            if status.checkpoint_diagnostics != graph.last_checkpoint_diagnostic {
                                if let Some(checkpoint) = &status.checkpoint_diagnostics {
                                    crate::debug::record_checkpoint_publication(checkpoint);
                                }
                                graph.last_checkpoint_diagnostic =
                                    status.checkpoint_diagnostics.clone();
                            }
                            let _ = app.emit_to(
                                label,
                                "sparse-v2-status",
                                sparse_v2_runtime_status_event(graph.binding_generation, status),
                            );
                        }
                    }
                    Err(error) => {
                        graph.retry.failed(Instant::now());
                        let message = error.to_string();
                        if graph.last_error.as_deref() != Some(&message) {
                            let _ = app.emit_to(
                                label,
                                "sparse-v2-error",
                                SparseV2ErrorEvent {
                                    binding_generation: graph.binding_generation,
                                    message: message.clone(),
                                },
                            );
                            graph.last_error = Some(message);
                        }
                        // The success arm is not the only place the frontend's
                        // page admission may move. Without this, a persistently
                        // failing actor emits nothing at all after its first
                        // error — the repeat suppression above sees to that —
                        // and the last `managed_writable` admission stays live
                        // indefinitely, letting bulk edits accumulate behind a
                        // writer that cannot save them (GH #324). The status
                        // call is the authority on writability and fails closed
                        // on its own when no application handle is active; the
                        // frontend additionally revokes writability the moment
                        // it sees the error, so a status this call cannot obtain
                        // does not leave the old one standing.
                        if let Ok(status) = graph.handle.status() {
                            let _ = app.emit_to(
                                label,
                                "sparse-v2-status",
                                sparse_v2_runtime_status_event(graph.binding_generation, status),
                            );
                        }
                    }
                }
            }

            // This is the focus-freshness boundary: every graph lane has
            // finished the requested full pass and all ordinary change events
            // were emitted before this completion marker. The frontend still
            // waits for its asynchronous handlers before admitting edits.
            // Configuration, for every graph this cycle could have touched. A
            // kernel rescan or notify error carries no usable paths, and poll
            // mode has none at all, so both re-check every graph -- one file
            // read and one digest each, against a stat scan they already pay.
            if refresh_changed_configs(
                &app,
                &labels_by_root,
                &config_paths,
                event_need_full || notify_error || !inotify,
                &mut config_recheck,
                &mut config_seen,
            ) {
                // A deferral means the lane was busy, not that the change went
                // away. Wake again; the 200 ms coalescing sleep below bounds
                // how fast this can retry while a transition holds the lane.
                let _ = tx.send(());
            }

            if let Some(sequence) = explicit_rescan {
                complete_full_rescan(&app, sequence);
            }

            // --- wait for the next cycle ---
            if inotify && !watched.is_empty() {
                // Block until the kernel reports a change (or a control poke).
                // Coalesce the several events produced by one atomic save.
                let now = Instant::now();
                let retry_wait = graphs
                    .values()
                    .filter_map(|graph| graph.retry.remaining(now))
                    .chain(
                        sparse_graphs
                            .values()
                            .filter_map(|graph| graph.retry.remaining(now)),
                    )
                    .chain(
                        sparse_graphs
                            .values()
                            .filter_map(|graph| graph.sweep_deadline_remaining),
                    )
                    .min();
                let wait_for =
                    inotify_cycle_wait(retry_wait, desired.difference(&watched).next().is_some());
                let woke_for_event = match wait_for {
                    Some(wait) => rx.recv_timeout(wait).is_ok(),
                    None => rx.recv().is_ok(),
                };
                if woke_for_event {
                    // Control pokes do not carry filesystem paths. Retain one
                    // explicit sparse actor turn for the next loop so a local
                    // managed save can drain its queued derivative work even
                    // when inotify correctly suppresses or coalesces its own
                    // projection write.
                    forced_sparse_tick = true;
                    std::thread::sleep(Duration::from_millis(200));
                    while rx.try_recv().is_ok() {}
                }
            } else {
                let now = Instant::now();
                let retry_wait = sparse_graphs
                    .values()
                    .filter_map(|graph| {
                        match (graph.retry.remaining(now), graph.sweep_deadline_remaining) {
                            (Some(retry), Some(sweep)) => Some(retry.min(sweep)),
                            (Some(retry), None) => Some(retry),
                            (None, Some(sweep)) => Some(sweep),
                            (None, None) => None,
                        }
                    })
                    .min()
                    .unwrap_or(Duration::from_secs(3))
                    .min(Duration::from_secs(3));
                let _ = rx.recv_timeout(retry_wait);
                while rx.try_recv().is_ok() {}
            }
        }
    });
}

/// How the file-watcher detects external changes (device-local, in
/// tine-settings.json): "inotify" (the default) → a real OS watcher, no idle
/// wakeups; "poll" → a 3s mtime scan for filesystems where native events are
/// unavailable or unreliable. Both feed the same full-diff reconciliation.
fn watch_mode(app: &tauri::AppHandle) -> String {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("watch_mode")
                .and_then(|x| x.as_str().map(String::from))
        })
        .filter(|m| m == "poll" || m == "inotify")
        .unwrap_or_else(|| default_watch_mode().to_string())
}

fn default_watch_mode() -> &'static str {
    "inotify"
}

#[tauri::command]
pub(crate) fn get_watch_mode(app: tauri::AppHandle) -> String {
    watch_mode(&app)
}

#[tauri::command]
pub(crate) fn set_watch_mode(
    mode: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), crate::command_error::CommandError> {
    let mode = if mode == "poll" { "poll" } else { "inotify" };
    update_settings(&app, |json| {
        json["watch_mode"] = serde_json::json!(mode);
    })?;
    // Wake the watcher so it switches mechanism right away.
    if let Some(tx) = state.watch_ctl.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn unset_watch_mode_prefers_native_events_on_every_platform() {
        assert_eq!(default_watch_mode(), "inotify");
    }
    use tine_core::model::{BlockDto, Format, PageDto};
    use tine_core::sync_runtime::SyncRuntimeLifecycle;

    fn runtime_snapshot(lifecycle: SyncRuntimeLifecycle) -> SyncRuntimeStatusSnapshot {
        SyncRuntimeStatusSnapshot {
            lifecycle: lifecycle.clone(),
            recovery: None,
            watcher: Default::default(),
            last_tick: None,
            detail: None,
            shared_role: None,
            shared_phase: None,
            provider_pending: 0,
            provider_runnable: false,
            search_index_building: false,
            move_episode_cleanup_pending: false,
            managed_local_pending: 0,
            managed_local_checkpointed_sequence: 0,
            managed_local_next_sequence: 0,
            managed_local_stage: None,
            history_recovery: None,
            history_recovery_diagnostics: None,
            application_pages_writable: matches!(lifecycle, SyncRuntimeLifecycle::Active),
            checkpoint_diagnostics: None,
            sweep_deadline_remaining: None,
            sweep_deadline_due: false,
        }
    }

    /// One permanently blocked lane must report ONCE. The desktop repeated
    /// `RecoveryBlocked("unsafe provider entry: enrollment: …")` because the
    /// local lane's healthy ticks kept clearing the repeat suppression between
    /// two identical failures (GH: desktop pairing, 2026-08-18).
    #[test]
    fn a_healthy_tick_from_another_lane_does_not_re_arm_a_live_failure() {
        let blocked = SyncRuntimeTick::RecoveryBlocked("unsafe provider entry: enrollment".into());
        let healthy = [
            SyncRuntimeTick::Idle,
            SyncRuntimeTick::Recovering,
            SyncRuntimeTick::AdmittedNoop { epoch: 4 },
            SyncRuntimeTick::AdmittedComplete { epoch: 5 },
        ];

        for tick in &healthy {
            assert!(
                !sparse_error_condition_ended(tick, true),
                "{tick:?} settled one lane while the actor still holds work it cannot finish"
            );
        }
        for tick in &healthy {
            assert!(
                sparse_error_condition_ended(tick, false),
                "{tick:?} on a drained actor is a real recovery and must report again"
            );
        }
        for runnable in [false, true] {
            assert!(
                !sparse_error_condition_ended(&blocked, runnable),
                "a blocked tick never ends its own condition"
            );
        }

        // The emission rule the watcher loop applies, replayed over the tick
        // sequence a stuck provider lane actually produces.
        let mut last_error: Option<String> = None;
        let mut emitted = Vec::new();
        for (tick, runnable) in [
            (&blocked, true),
            (&healthy[0], true),
            (&blocked, true),
            (&healthy[0], true),
            (&blocked, true),
        ] {
            if matches!(
                tick,
                SyncRuntimeTick::RecoveryBlocked(_)
                    | SyncRuntimeTick::Blocked(_)
                    | SyncRuntimeTick::Terminal(_)
                    | SyncRuntimeTick::Failed(_)
            ) {
                let message = format!("{tick:?}");
                if last_error.as_deref() != Some(&message) {
                    emitted.push(message.clone());
                    last_error = Some(message);
                }
            } else if sparse_error_condition_ended(tick, runnable) {
                last_error = None;
            }
        }
        assert_eq!(emitted.len(), 1, "one condition, one report: {emitted:?}");
    }

    #[test]
    fn sparse_status_event_derives_admission_from_its_single_status_observation() {
        for (lifecycle, authority) in [
            (SyncRuntimeLifecycle::Active, "managed_writable"),
            (SyncRuntimeLifecycle::StoppedSafe, "managed_unavailable"),
            (SyncRuntimeLifecycle::StoppedCrashed, "managed_unavailable"),
            (SyncRuntimeLifecycle::Terminal, "managed_unavailable"),
        ] {
            let event = sparse_v2_runtime_status_event(73, runtime_snapshot(lifecycle));
            let wire = serde_json::to_value(event).unwrap();
            assert_eq!(wire["binding_generation"], 73);
            assert_eq!(wire["application_page_admission"]["binding_generation"], 73);
            assert_eq!(wire["application_page_admission"]["authority"], authority);
        }
    }

    #[test]
    fn active_history_recovery_withdraws_admission_and_serializes_its_phase() {
        let mut snapshot = runtime_snapshot(SyncRuntimeLifecycle::Active);
        snapshot.application_pages_writable = false;
        snapshot.history_recovery = Some(tine_core::sync_runtime::SyncHistoryRecoveryStatus {
            attempt: 1,
            reason: tine_core::sync_runtime::SyncHistoryRecoveryReason::DependencyBelowFloor,
            phase: tine_core::sync_runtime::SyncHistoryRecoveryPhase::WaitingForDelivery,
            triggering_batch_id: tine_core::oplog::BatchId::new(),
            triggering_document_id: tine_core::oplog::DocumentId::new(),
            requested_floor: Vec::new(),
            actual_floor: Vec::new(),
            retry_class: None,
            retry_cause: None,
            diagnostics: tine_core::sync_runtime::SyncHistoryRecoveryDiagnostics {
                journal_fences: Vec::new(),
                accepted_count: 0,
                pending_count: 1,
                replayed_count: 0,
                waiting_ms: 0,
                reconstruction_ms: 0,
                checkpoint_bytes: None,
                publication_edge:
                    tine_core::sync_runtime::SyncHistoryRecoveryPublicationEdge::CustodyDurable,
                preservation_check:
                    tine_core::sync_runtime::SyncHistoryRecoveryPreservationCheck::Pending,
            },
        });
        snapshot.last_tick = Some(SyncRuntimeTick::RecoveryBlocked(
            "automatic full-history reconstruction is waiting for delivery".into(),
        ));
        let event = sparse_v2_runtime_status_event(74, snapshot);
        let wire = serde_json::to_value(event).unwrap();
        assert_eq!(
            wire["application_page_admission"]["authority"],
            "managed_unavailable"
        );
        assert_eq!(
            wire["runtime"]["history_recovery"]["phase"],
            "waiting_for_delivery"
        );
        assert_eq!(
            wire["runtime"]["history_recovery"]["reason"],
            "dependency_below_floor"
        );
    }

    #[test]
    fn recovery_waiting_for_peer_delivery_is_not_reported_as_a_failure() {
        let waiting = SyncRuntimeTick::RecoveryBlocked(
            "automatic full-history reconstruction is waiting for delivery".into(),
        );
        assert!(!sparse_tick_is_reportable_failure(&waiting, true));
        assert!(sparse_tick_is_reportable_failure(&waiting, false));
        assert!(sparse_tick_is_blocked_without_progress(&waiting, true) == false);
    }

    #[test]
    fn atomic_page_save_temp_events_stay_incremental() {
        use notify::event::{EventKind, ModifyKind, RenameMode};

        let page = PathBuf::from("/graphs/a/pages/one.md");
        // `create_projection_temp` uses this exact suffix. Keeping the producer's
        // spelling here matters: the old test used a retired `.tmp` form and let
        // every real atomic rename fall through to a full-graph scan.
        let temp = PathBuf::from("/graphs/a/pages/.one.md.123.7.projection.tmp");
        let mut pending = Pending::default();
        pending.add_event(notify::Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![temp, page.clone()],
            attrs: Default::default(),
        });

        assert!(
            !pending.need_full,
            "Tine's own temp rename must not request a full scan"
        );
        assert_eq!(pending.paths, HashSet::from([page]));
    }

    /// GH #366. Windows reports ordinary file writes as `Create(Any)` or
    /// `Modify(Any)`. The raw admission classifier already knew these were exact
    /// page paths, but the queued classifier sent the same event through a full
    /// graph diff. On a large graph that kept creation's external-change barrier
    /// raised past every frontend retry.
    #[test]
    fn windows_exact_page_events_stay_incremental_through_the_pending_queue() {
        use notify::event::{CreateKind, EventKind, ModifyKind};

        let graph_dir = TempGraph::new("windows-pending-exact-page");
        graph_dir.write("pages/TINE版本更新提示词.md", "- 中文内容\n");
        let page = graph_dir.path("pages/TINE版本更新提示词.md");

        for kind in [
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Any),
        ] {
            let mut pending = Pending::default();
            pending.add_event(event(kind, vec![page.clone()]));
            assert_eq!(pending.paths, HashSet::from([page.clone()]), "{kind:?}");
            assert!(pending.full_paths.is_empty(), "{kind:?}");
            assert!(!pending.need_full, "{kind:?}");
        }
    }

    #[test]
    fn windows_unicode_event_reconciles_and_releases_new_page_creation() {
        use notify::event::{CreateKind, EventKind};

        let graph_dir = TempGraph::new("windows-unicode-frontier-release");
        graph_dir.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&graph_dir.root);
        warm_direct_graph(&graph);
        let mut snap = collect_graph_text_files(&graph).files;

        graph_dir.write("pages/TINE版本更新提示词.md", "- 中文内容\n");
        let path = graph_dir.path("pages/TINE版本更新提示词.md");
        let create = event(EventKind::Create(CreateKind::Any), vec![path.clone()]);
        assert!(observe_legacy_graph_text_event(
            &graph,
            &graph_dir.root,
            Some(&create),
        ));
        let frontier = graph.graph_text_external_observation_ticket();
        assert_new_page_waits_for_reconciliation(&graph, "Blocked During Windows Event");

        let mut pending = Pending::default();
        pending.add_event(create);
        let (changes, _conflicts, used_full, errors) =
            reconcile_pending(&graph, &mut snap, &pending.paths, false, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!used_full, "one exact Windows page event must stay O(page)");
        assert_eq!(changes.len(), 1);
        assert!(graph.acknowledge_graph_text_external_observations(frontier));

        graph
            .save_page(&new_page("Creation After Windows Event"), None)
            .unwrap();
        assert!(graph_dir
            .path("pages/Creation After Windows Event.md")
            .exists());
    }

    /// GH #374 negative follow-up to #366. Windows reports the atomic
    /// publication of Tine's own new journal as an exact graph-text event. That
    /// echo must not raise the external-change admission frontier and strand the
    /// next new page before the debounced reconciler sees the journal bytes.
    #[test]
    fn windows_tine_owned_create_echoes_do_not_block_following_pages_or_journals() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RenameMode};

        let cases = [
            (
                "journal-page",
                new_journal("Aug 25th, 2026"),
                new_page("20260825100915"),
            ),
            ("page-page-unicode", new_page("第一页"), new_page("第二页")),
            (
                "page-journal",
                new_page("Before Journal"),
                new_journal("Aug 24th, 2026"),
            ),
        ];
        for (case, first, second) in cases {
            let graph_dir = TempGraph::new(&format!("windows-owned-{case}"));
            graph_dir.write("pages/Anchor.md", "- anchor\n");
            let graph = Graph::open(&graph_dir.root);
            warm_direct_graph(&graph);

            graph.save_page(&first, None).unwrap();
            let first_path = graph
                .find_entry(&first.name, first.kind)
                .expect("created first entry")
                .path;
            let before = graph.graph_text_external_observation_ticket();
            // ReadDirectoryChangesW can surface several exact shapes for the
            // one atomic publication before (and occasionally just after) the
            // debounce pass. Every one must validate the same exact receipt.
            for kind in [
                EventKind::Create(CreateKind::Any),
                EventKind::Modify(ModifyKind::Any),
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            ] {
                let paths = if matches!(&kind, EventKind::Modify(ModifyKind::Name(_))) {
                    let filename = first_path.file_name().unwrap().to_string_lossy();
                    vec![
                        first_path.with_file_name(format!(".{filename}.123.7.projection.tmp")),
                        first_path.clone(),
                    ]
                } else {
                    vec![first_path.clone()]
                };
                assert!(observe_legacy_graph_text_event(
                    &graph,
                    &graph_dir.root,
                    Some(&event(kind, paths)),
                ));
                assert_eq!(
                    graph.graph_text_external_observation_ticket(),
                    before,
                    "{case}: Tine's exact publication echo must not become an external frontier"
                );
            }
            graph
                .sync_file_checked(&first_path)
                .expect("debounced self-write reconciliation");
            assert!(observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                Some(&event(EventKind::Modify(ModifyKind::Any), vec![first_path],)),
            ));
            assert_eq!(
                graph.graph_text_external_observation_ticket(),
                before,
                "{case}: a delayed duplicate matching the admitted cache state remains a no-op"
            );

            graph
                .save_page(&second, None)
                .unwrap_or_else(|error| panic!("{case}: following creation failed: {error}"));
        }
    }

    #[test]
    fn windows_external_replacement_of_tine_publication_keeps_creation_blocked() {
        use notify::event::{EventKind, ModifyKind};

        for same_bytes in [false, true] {
            let graph_dir = TempGraph::new(if same_bytes {
                "windows-external-same-bytes-new-identity"
            } else {
                "windows-external-different-bytes"
            });
            graph_dir.write("pages/Anchor.md", "- anchor\n");
            let graph = Graph::open(&graph_dir.root);
            warm_direct_graph(&graph);
            let first = new_page("First Publication");
            graph.save_page(&first, None).unwrap();
            let first_path = graph
                .find_entry(&first.name, first.kind)
                .expect("created page entry")
                .path;
            if same_bytes {
                let bytes = std::fs::read(&first_path).unwrap();
                let replacement = graph_dir.path("external-winner.tmp");
                std::fs::write(&replacement, bytes).unwrap();
                std::fs::remove_file(&first_path).unwrap();
                std::fs::rename(replacement, &first_path).unwrap();
            } else {
                std::fs::write(&first_path, "- external winner\n").unwrap();
            }

            assert!(observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                Some(&event(EventKind::Modify(ModifyKind::Any), vec![first_path],)),
            ));
            assert_new_page_waits_for_reconciliation(
                &graph,
                if same_bytes {
                    "Blocked By New Physical Owner"
                } else {
                    "Blocked By Different External Bytes"
                },
            );
        }
    }

    #[test]
    fn unknown_path_event_requests_full_scan_only_for_its_owner() {
        use notify::event::{CreateKind, EventKind};

        let unknown = PathBuf::from("/graphs/a/pages/new-directory");
        let mut pending = Pending::default();
        pending.add_event(notify::Event {
            kind: EventKind::Create(CreateKind::Folder),
            paths: vec![unknown.clone()],
            attrs: Default::default(),
        });

        assert!(!pending.need_full);
        assert_eq!(pending.full_paths, HashSet::from([unknown]));
        assert!(pending.paths.is_empty());
    }

    #[test]
    fn pending_stamps_the_first_event_of_a_batch_once() {
        use notify::event::{CreateKind, EventKind};

        let mut pending = Pending::default();
        assert!(pending.first_event_at.is_none());
        pending.add_event(notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/graphs/a/pages/one.md")],
            attrs: Default::default(),
        });
        let first = pending
            .first_event_at
            .expect("first event stamps the batch");
        pending.add_event(notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/graphs/a/pages/two.md")],
            attrs: Default::default(),
        });
        assert_eq!(
            pending.first_event_at,
            Some(first),
            "later events in the same batch must not move the batch stamp"
        );
        pending.add_notify_error();
        assert_eq!(pending.first_event_at, Some(first));
    }

    #[test]
    fn latency_receipt_measures_the_three_stages() {
        let first = Instant::now();
        let reconcile_started = first + Duration::from_millis(200);
        let emitted_at = reconcile_started + Duration::from_millis(35);

        let receipt = latency_receipt(
            "graph-a",
            true,
            3,
            4,
            false,
            1,
            Some(first),
            reconcile_started,
            emitted_at,
        );

        assert_eq!(receipt.graph, "graph-a");
        assert_eq!(receipt.mode, "inotify");
        assert_eq!(receipt.pages, 3);
        assert_eq!(receipt.event_paths, 4);
        assert!(!receipt.full_diff);
        assert_eq!(receipt.errors, 1);
        assert_eq!(receipt.event_to_reconcile_ms, Some(200));
        assert_eq!(receipt.reconcile_ms, 35);
        assert_eq!(receipt.event_to_emit_ms, Some(235));
    }

    #[test]
    fn latency_receipt_without_a_callback_stamp_reports_only_reconcile_time() {
        let reconcile_started = Instant::now();
        let emitted_at = reconcile_started + Duration::from_millis(12);

        let receipt = latency_receipt(
            "graph-a",
            false,
            2,
            0,
            true,
            0,
            None,
            reconcile_started,
            emitted_at,
        );

        assert_eq!(receipt.mode, "poll");
        assert!(receipt.full_diff);
        assert_eq!(receipt.event_to_reconcile_ms, None);
        assert_eq!(receipt.event_to_emit_ms, None);
        assert_eq!(receipt.reconcile_ms, 12);
    }

    #[test]
    fn latency_receipt_ring_keeps_the_newest_sixty_four() {
        let now = Instant::now();
        let mut ring = VecDeque::new();
        for seq in 1..=(LATENCY_RECEIPT_CAP as u64 + 6) {
            let mut receipt = latency_receipt("graph-a", true, 1, 1, false, 0, None, now, now);
            receipt.seq = seq;
            push_latency_receipt(&mut ring, receipt);
        }
        assert_eq!(ring.len(), LATENCY_RECEIPT_CAP);
        assert_eq!(ring.front().map(|receipt| receipt.seq), Some(7));
        assert_eq!(
            ring.back().map(|receipt| receipt.seq),
            Some(LATENCY_RECEIPT_CAP as u64 + 6)
        );
    }

    /// The receipt is a diagnostic wire format a reporter pastes into an issue;
    /// its field names are part of that contract.
    #[test]
    fn latency_receipt_wire_shape_is_stable() {
        let now = Instant::now();
        let receipt = latency_receipt("graph-a", true, 1, 1, false, 0, Some(now), now, now);
        let wire = serde_json::to_value(&receipt).unwrap();
        for key in [
            "seq",
            "at_unix_ms",
            "graph",
            "mode",
            "pages",
            "event_paths",
            "full_diff",
            "errors",
            "event_to_reconcile_ms",
            "reconcile_ms",
            "event_to_emit_ms",
        ] {
            assert!(wire.get(key).is_some(), "missing receipt field {key}");
        }
    }

    #[test]
    fn explicit_unmanaged_file_events_do_not_schedule_graph_scans() {
        use notify::event::{CreateKind, EventKind, RemoveKind};

        for (kind, path, asset_paths) in [
            (
                EventKind::Create(CreateKind::File),
                PathBuf::from("/graphs/a/assets/image.png"),
                1,
            ),
            (
                EventKind::Remove(RemoveKind::File),
                PathBuf::from("/graphs/a/logseq/config.edn"),
                1,
            ),
        ] {
            let mut pending = Pending::default();
            pending.add_event(notify::Event {
                kind,
                paths: vec![path],
                attrs: Default::default(),
            });
            assert!(!pending.need_full);
            assert!(pending.full_paths.is_empty());
            assert!(pending.paths.is_empty());
            assert_eq!(
                pending.asset_paths.len(),
                asset_paths,
                "ordinary exact files must remain available to the separate asset observer"
            );
        }
    }

    #[test]
    fn asset_observation_names_only_paths_inside_the_asset_capability() {
        let root = Path::new("/graphs/a/assets");
        assert_eq!(
            asset_relative_event_path(root, Path::new("/graphs/a/assets/diagrams/flow.svg")),
            Some("diagrams/flow.svg".into())
        );
        assert_eq!(
            asset_relative_event_path(root, Path::new("/graphs/a/pages/flow.svg")),
            None
        );
        assert_eq!(asset_relative_event_path(root, root), None);
    }

    #[cfg(unix)]
    #[test]
    fn external_asset_alias_events_refresh_replacement_and_deletion() {
        let graph = TempGraph::new("asset-watch-lexical-alias");
        let canonical = graph.path("external");
        let lexical = graph.path("assets");
        std::fs::create_dir_all(&canonical).unwrap();
        std::os::unix::fs::symlink(&canonical, &lexical).unwrap();
        let image = canonical.join("pixel.png");
        std::fs::write(&image, b"same bytes").unwrap();
        let mut state = AssetWatchState::new(canonical.clone());
        let empty = HashSet::new();
        // notify's recursive graph watch follows this symlink and may assign
        // its lexical path to the same descriptor as the canonical watch.
        let mut exact = HashSet::from([lexical.join("pixel.png")]);
        normalize_asset_event_aliases(
            std::iter::once((graph.path("").as_path(), &state)),
            &mut exact,
            &mut HashSet::new(),
        );
        std::fs::write(canonical.join("replacement"), b"same bytes").unwrap();
        std::fs::rename(canonical.join("replacement"), &image).unwrap();
        assert_eq!(
            reconcile_asset_observation("alias", &mut state, &exact, &empty, false, false),
            vec!["pixel.png"],
        );
        std::fs::remove_file(&image).unwrap();
        let mut exact = HashSet::from([lexical.join("pixel.png")]);
        normalize_asset_event_aliases(
            std::iter::once((graph.path("").as_path(), &state)),
            &mut exact,
            &mut HashSet::new(),
        );
        assert_eq!(
            reconcile_asset_observation("alias", &mut state, &exact, &empty, false, false),
            vec!["pixel.png"],
        );
    }

    #[test]
    fn asset_alias_routing_refreshes_all_shared_root_windows_and_keeps_scope() {
        let graph = TempGraph::new("asset-alias-shared");
        let first_root = graph.path("first");
        let second_root = graph.path("second");
        let inactive_root = graph.path("inactive");
        let canonical = graph.path("external");
        std::fs::create_dir_all(&canonical).unwrap();
        let image = canonical.join("pixel.png");
        std::fs::write(&image, b"old").unwrap();
        let mut first = AssetWatchState::new(canonical.clone());
        let mut second = AssetWatchState::new(canonical.clone());
        let inactive = AssetWatchState::default();
        let lexical_event = second_root.join("assets/pixel.png");
        let mut exact = HashSet::from([
            lexical_event.clone(),
            first_root.join("assets/../outside.png"),
            inactive_root.join("assets/unapproved.png"),
            graph.path("unbound/assets/unknown.png"),
        ]);
        normalize_asset_event_aliases(
            [
                (first_root.as_path(), &first),
                (second_root.as_path(), &second),
                (inactive_root.as_path(), &inactive),
            ]
            .into_iter(),
            &mut exact,
            &mut HashSet::new(),
        );
        assert_eq!(exact.len(), 5, "only the approved pixel alias adds a path");
        assert!(
            exact.contains(&lexical_event),
            "preserve the original event"
        );
        assert!(exact.contains(&image));
        std::fs::write(&image, b"replacement image").unwrap();
        for (label, state) in [("first", &mut first), ("second", &mut second)] {
            assert_eq!(
                reconcile_asset_observation(label, state, &exact, &HashSet::new(), false, false),
                vec!["pixel.png"]
            );
        }
    }

    #[test]
    fn asset_alias_directory_deletion_routes_without_following_missing_paths() {
        let graph = TempGraph::new("asset-alias-directory");
        let root = graph.path("graph");
        let canonical = graph.path("external");
        std::fs::create_dir_all(canonical.join("nested")).unwrap();
        std::fs::write(canonical.join("nested/image.png"), b"image").unwrap();
        let mut state = AssetWatchState::new(canonical.clone());
        std::fs::remove_dir_all(canonical.join("nested")).unwrap();
        let mut full = HashSet::from([root.join("assets/nested")]);
        normalize_asset_event_aliases(
            std::iter::once((root.as_path(), &state)),
            &mut HashSet::new(),
            &mut full,
        );
        assert!(full.contains(&canonical.join("nested")));
        assert_eq!(
            reconcile_asset_observation(
                "directory",
                &mut state,
                &HashSet::new(),
                &full,
                false,
                false
            ),
            vec!["nested/image.png"]
        );
        let mut parent = HashSet::from([root.clone()]);
        normalize_asset_event_aliases(
            std::iter::once((root.as_path(), &state)),
            &mut HashSet::new(),
            &mut parent,
        );
        assert!(
            parent.contains(&canonical),
            "uncertain root events include the approved asset root"
        );
    }

    #[test]
    fn asset_observation_refreshes_peers_but_suppresses_the_originating_self_write() {
        let graph = TempGraph::new("asset-self-write");
        let assets = graph.path("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let mut origin = AssetWatchState::new(assets.clone());
        let mut peer = AssetWatchState::new(assets.clone());
        let empty = HashSet::new();
        assert!(
            reconcile_asset_observation("main", &mut origin, &empty, &empty, true, false,)
                .is_empty()
        );
        assert!(
            reconcile_asset_observation("second", &mut peer, &empty, &empty, true, false,)
                .is_empty()
        );

        let image = assets.join("nested/image.png");
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, b"new image bytes").unwrap();
        note_asset_self_write("main", &image);
        let exact = HashSet::from([image]);

        assert_eq!(
            reconcile_asset_observation("second", &mut peer, &exact, &empty, false, false),
            vec!["nested/image.png"]
        );
        assert!(
            reconcile_asset_observation("main", &mut origin, &exact, &empty, false, false,)
                .is_empty()
        );
    }

    #[test]
    fn asset_read_before_watcher_binding_remains_the_refresh_baseline() {
        let graph = TempGraph::new("asset-read-before-watch");
        let assets = graph.path("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let image = assets.join("startup.png");
        std::fs::write(&image, b"bytes rendered before watcher binding").unwrap();
        note_asset_read("main", &image);

        // An external synchronizer replaces the file before AssetWatchState is
        // constructed. A fresh directory snapshot alone would baseline these
        // new bytes and lose the invalidation for the already-rendered image.
        let replacement = assets.join("startup.replacement");
        std::fs::write(&replacement, b"replacement bytes").unwrap();
        std::fs::rename(&replacement, &image).unwrap();
        let mut state = AssetWatchState::new(assets);

        assert_eq!(
            reconcile_asset_observation(
                "main",
                &mut state,
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
            ),
            vec!["startup.png"]
        );
    }

    #[test]
    fn asset_read_handoff_compares_only_the_assets_that_were_read() {
        let graph = TempGraph::new("asset-read-exact-handoff");
        let assets = graph.path("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let rendered = assets.join("rendered.png");
        let unrelated = assets.join("unrelated.png");
        std::fs::write(&rendered, b"rendered old").unwrap();
        std::fs::write(&unrelated, b"unrelated old").unwrap();
        let mut state = AssetWatchState::new(assets);
        note_asset_read("exact-handoff", &rendered);

        std::fs::write(&rendered, b"rendered replacement").unwrap();
        std::fs::write(&unrelated, b"unrelated replacement").unwrap();

        assert_eq!(
            reconcile_asset_observation(
                "exact-handoff",
                &mut state,
                &HashSet::new(),
                &HashSet::new(),
                false,
                false,
            ),
            vec!["rendered.png"],
            "a WebView read seed must not trigger a whole asset-tree scan"
        );
    }

    #[cfg(unix)]
    #[test]
    fn self_write_marker_canonicalizes_an_approved_external_assets_link() {
        use std::os::unix::fs::symlink;

        let graph = TempGraph::new("asset-self-write-external");
        let external = TempGraph::new("asset-self-write-external-target");
        let external_assets = external.path("approved-assets");
        std::fs::create_dir_all(&external_assets).unwrap();
        symlink(&external_assets, graph.path("assets")).unwrap();
        let mut state = AssetWatchState::new(external_assets.clone());
        let image = external_assets.join("linked.png");
        std::fs::write(&image, b"linked image").unwrap();
        note_asset_self_write("main", &graph.path("assets/linked.png"));

        assert!(reconcile_asset_observation(
            "main",
            &mut state,
            &HashSet::from([image]),
            &HashSet::new(),
            false,
            false,
        )
        .is_empty());
    }

    #[test]
    fn uncertain_asset_rescan_is_metadata_only_and_does_not_invent_deletions() {
        let source = include_str!("watcher.rs");
        let body = source
            .split_once("fn collect_asset_files(")
            .unwrap()
            .1
            .split_once("fn asset_relative_event_path(")
            .unwrap()
            .0;
        assert!(!body.contains("std::fs::read("));
        assert!(!body.contains("Graph::"));
        assert!(!body.contains("observe_watcher"));

        let graph = TempGraph::new("asset-full-diff");
        let assets = graph.path("assets");
        std::fs::create_dir_all(&assets).unwrap();
        graph.write("assets/one.png", "one");
        let mut state = AssetWatchState::new(assets);
        let empty = HashSet::new();
        assert!(
            reconcile_asset_observation("main", &mut state, &empty, &empty, true, false,)
                .is_empty()
        );
        graph.write("assets/one.png", "changed-length");
        graph.write("assets/two.svg", "two");
        assert_eq!(
            reconcile_asset_observation("main", &mut state, &empty, &empty, true, false),
            vec!["one.png", "two.svg"]
        );
    }

    #[test]
    fn markdown_and_case_variant_text_events_stay_incremental() {
        use notify::event::{CreateKind, EventKind};

        let paths = vec![
            PathBuf::from("/graphs/a/archive/one.markdown"),
            PathBuf::from("/graphs/a/archive/two.MD"),
            PathBuf::from("/graphs/a/archive/three.ORG"),
        ];
        let mut pending = Pending::default();
        pending.add_event(notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: paths.clone(),
            attrs: Default::default(),
        });
        assert_eq!(pending.paths, paths.into_iter().collect());
        assert!(pending.full_paths.is_empty());
        assert!(!pending.need_full);
    }

    /// Concord P5: what the watcher admits from `.git/**` and its equivalents,
    /// stated explicitly and tested. A repository parked in the graph tree is
    /// the loudest event source a Direct Files user has; none of its churn can
    /// describe graph text, so none of it may cost anything.
    #[test]
    fn vcs_and_tool_churn_never_wakes_the_watcher() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind};

        let roots = HashSet::from([PathBuf::from("/graphs/a")]);
        let noise: &[(EventKind, &str)] = &[
            (
                EventKind::Create(CreateKind::File),
                "/graphs/a/.git/index.lock",
            ),
            (
                EventKind::Remove(RemoveKind::File),
                "/graphs/a/.git/index.lock",
            ),
            (
                EventKind::Modify(ModifyKind::Any),
                "/graphs/a/.git/objects/ab/cdef0123456789",
            ),
            (
                EventKind::Create(CreateKind::Any),
                "/graphs/a/.git/refs/heads/main",
            ),
            (
                EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
                "/graphs/a/.hg/store/data/page.md.i",
            ),
            (
                EventKind::Create(CreateKind::File),
                "/graphs/a/.jj/repo/op_store/x",
            ),
            (EventKind::Create(CreateKind::File), "/graphs/a/.svn/wc.db"),
            (
                EventKind::Create(CreateKind::File),
                "/graphs/a/.stversions/pages/Note~20260818.md",
            ),
            (
                EventKind::Modify(ModifyKind::Any),
                "/graphs/a/.stfolder/marker",
            ),
            (
                EventKind::Create(CreateKind::File),
                "/graphs/a/node_modules/pkg/readme.md",
            ),
        ];
        for (kind, path) in noise {
            let event = notify::Event {
                kind: *kind,
                paths: vec![PathBuf::from(path)],
                attrs: Default::default(),
            };
            assert!(
                watch_event_is_tool_noise(&event, &roots),
                "{path} must not wake the watcher"
            );
        }

        // Everything else still gets through, including the cases a name list
        // is most likely to over-reach on.
        let admitted: &[(EventKind, Vec<&str>)] = &[
            // Tine's own provider tree is hidden from graph text too, but the
            // managed lane's observations are made of exactly these events.
            (
                EventKind::Create(CreateKind::File),
                vec!["/graphs/a/.tine-sync/v2/shared/outbox/0001"],
            ),
            // An ordinary page, and configuration.
            (
                EventKind::Create(CreateKind::File),
                vec!["/graphs/a/pages/Note.md"],
            ),
            (
                EventKind::Modify(ModifyKind::Any),
                vec!["/graphs/a/logseq/config.edn"],
            ),
            // A rename OUT of .git reports both sides: one ordinary path is
            // enough to keep the whole event.
            (
                EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both)),
                vec!["/graphs/a/.git/tmp_obj_x", "/graphs/a/pages/Note.md"],
            ),
            // A path under no watched root is not ours to judge.
            (
                EventKind::Create(CreateKind::File),
                vec!["/elsewhere/.git/index"],
            ),
        ];
        for (kind, paths) in admitted {
            let event = notify::Event {
                kind: *kind,
                paths: paths.iter().map(PathBuf::from).collect(),
                attrs: Default::default(),
            };
            assert!(
                !watch_event_is_tool_noise(&event, &roots),
                "{paths:?} must still be seen"
            );
        }

        // A kernel queue overflow says nothing about which paths were lost, so
        // its rescan demand survives even when its paths look like noise.
        let mut overflow = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/graphs/a/.git/objects")],
            attrs: Default::default(),
        };
        overflow = overflow.set_flag(notify::event::Flag::Rescan);
        assert!(!watch_event_is_tool_noise(&overflow, &roots));

        // With no watched root there is nothing to strip a prefix against.
        let event = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/graphs/a/.git/index.lock")],
            attrs: Default::default(),
        };
        assert!(!watch_event_is_tool_noise(&event, &HashSet::new()));
    }

    /// The list is only safe because every name on it is provably outside graph
    /// text on its own authority — the core's `GraphTextScope`, which discovery
    /// and the full-diff walk also use. If a name ever became page-bearing,
    /// dropping its events would hide pages; this fails first.
    #[test]
    fn vcs_and_tool_noise_dirs_can_never_hold_graph_text() {
        let scope = tine_core::graph_text_scope::GraphTextScope::new(&[], false);
        for name in VCS_AND_TOOL_NOISE_DIRS {
            assert!(
                !scope.should_descend(name),
                "{name} must be outside graph text"
            );
            assert!(
                !scope.is_eligible(&format!("{name}/Page.md")),
                "{name}/Page.md must never be a page"
            );
            assert!(
                !scope.is_eligible(&format!("pages/{name}/Page.md")),
                "pages/{name}/Page.md must never be a page"
            );
        }
        // ...and the one Tine-owned hidden tree that is deliberately NOT here.
        assert!(!VCS_AND_TOOL_NOISE_DIRS.contains(&".tine-sync"));
    }

    /// A graph that itself lives inside a repository's directory must not have
    /// every one of its own events dropped.
    #[test]
    fn a_graph_under_a_noise_directory_is_judged_relative_to_its_root() {
        use notify::event::{CreateKind, EventKind};

        let roots = HashSet::from([PathBuf::from("/repo/.git/notes")]);
        let event = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/repo/.git/notes/pages/Note.md")],
            attrs: Default::default(),
        };
        assert!(!watch_event_is_tool_noise(&event, &roots));
    }

    #[test]
    fn hidden_sync_events_do_not_schedule_direct_files_reconciliation() {
        use notify::event::{CreateKind, EventKind};

        let chunk =
            PathBuf::from("/graphs/a/.tine-sync/v1/devices/device/sessions/session/0001.chunk");
        let mut pending = Pending::default();
        pending.add_event(notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![chunk.clone()],
            attrs: Default::default(),
        });

        assert!(!pending.need_full);
        assert!(pending.full_paths.is_empty());
        assert!(pending.paths.is_empty());
    }

    #[test]
    fn sparse_watcher_routes_nested_unicode_nonstandard_text_and_fault_observations() {
        let root = PathBuf::from("/graphs/研究");
        let nested = root.join("archive/層/計画.markdown");
        let org = root.join("nonstandard/deep/日記.org");
        let unknown = root.join("config.edn");
        let outside = PathBuf::from("/graphs/other/pages/ignored.md");
        let paths = HashSet::from([nested, org, outside]);
        let full_paths = HashSet::from([unknown]);

        let observations = sparse_observations(&root, &paths, &full_paths, true, true);
        assert!(observations
            .contains(&SyncWatcherObservation::managed_path("archive/層/計画.markdown").unwrap()));
        assert!(observations
            .contains(&SyncWatcherObservation::managed_path("nonstandard/deep/日記.org").unwrap()));
        assert!(observations.contains(&SyncWatcherObservation::UnknownPath));
        assert!(observations.contains(&SyncWatcherObservation::RescanRequired));
        assert!(observations.contains(&SyncWatcherObservation::NotifyError));
        assert_eq!(
            observations
                .iter()
                .filter(|observation| matches!(observation, SyncWatcherObservation::ManagedPath(_)))
                .count(),
            2
        );
    }

    #[test]
    fn notify_failures_remain_distinct_from_rescan_obligations() {
        let mut pending = Pending::default();
        pending.add_notify_error();
        assert!(pending.need_full);
        assert!(pending.notify_error);

        let observations = sparse_observations(
            Path::new("/graph"),
            &HashSet::new(),
            &HashSet::new(),
            pending.need_full,
            pending.notify_error,
        );
        assert_eq!(
            observations,
            vec![
                SyncWatcherObservation::RescanRequired,
                SyncWatcherObservation::NotifyError
            ]
        );
    }

    #[test]
    fn sparse_watcher_does_not_reimport_its_private_archive_writes() {
        let root = PathBuf::from("/graph");
        let paths = HashSet::from([root.join(".tine-sync/v2/objects/immutable")]);
        assert!(sparse_observations(&root, &paths, &HashSet::new(), false, false).is_empty());
    }

    #[test]
    fn sparse_watcher_never_routes_asset_observation_into_the_managed_actor() {
        let root = PathBuf::from("/graph");
        let exact = HashSet::from([root.join("assets/image.png")]);
        let ambiguous = HashSet::from([root.join("assets/nested")]);
        assert!(sparse_observations(&root, &exact, &ambiguous, false, false).is_empty());
    }

    #[test]
    fn sparse_watcher_never_routes_logseq_backup_observation_into_the_managed_actor() {
        let root = PathBuf::from("/graph");
        let backup =
            root.join("logseq/bak/journals/2026_08_31/2026-08-31T15_34_34.562Z.Desktop.md");
        let exact = HashSet::from([backup.clone()]);
        let ambiguous = HashSet::from([backup.parent().unwrap().to_path_buf()]);

        assert!(sparse_observations(&root, &exact, &HashSet::new(), false, false).is_empty());
        assert!(sparse_observations(&root, &HashSet::new(), &ambiguous, false, false).is_empty());
    }

    #[test]
    fn sparse_provider_poll_and_exact_events_stay_out_of_graph_reconciliation() {
        let root = PathBuf::from("/graphs/研究");
        let manifest = root.join(
            ".tine-sync/v2/shared/outbox/manifests/12345678-1234-1234-1234-123456789abc.manifest",
        );
        let paths = HashSet::from([manifest]);
        assert!(
            sparse_observations(&root, &paths, &HashSet::new(), false, false).is_empty(),
            "provider polling must not trigger graph-wide reconciliation"
        );
        let (provider_paths, imprecise) =
            sparse_provider_observations(&root, &paths, &HashSet::new());
        assert_eq!(
            provider_paths,
            vec!["manifests/12345678-1234-1234-1234-123456789abc.manifest".to_owned()]
        );
        assert!(!imprecise);

        let recovery = HashSet::from([
            root.join(
                ".tine-sync/v2/shared/outbox/manifest-recovery-links-v1/12345678-1234-1234-1234-123456789abc.link",
            ),
            root.join(
                ".tine-sync/v2/shared/outbox/manifest-recovery-blobs-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.manifest",
            ),
        ]);
        let (provider_paths, imprecise) =
            sparse_provider_observations(&root, &recovery, &HashSet::new());
        assert_eq!(
            provider_paths,
            vec![
                "manifest-recovery-blobs-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.manifest"
                    .to_owned(),
                "manifest-recovery-links-v1/12345678-1234-1234-1234-123456789abc.link"
                    .to_owned(),
            ]
        );
        assert!(!imprecise);

        let head = root.join(
            ".tine-sync/v2/shared/outbox/frontier-heads-v1/12345678-1234-1234-1234-123456789abc-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.head",
        );
        let (provider_paths, imprecise) =
            sparse_provider_observations(&root, &HashSet::from([head]), &HashSet::new());
        assert_eq!(
            provider_paths,
            vec![
                "frontier-heads-v1/12345678-1234-1234-1234-123456789abc-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.head"
                    .to_owned()
            ]
        );
        assert!(!imprecise);

        let internal = HashSet::from([
            root.join(".tine-sync/v2/shared/outbox/.part/local-write"),
            root.join(".tine-sync/v2/shared/outbox/removed/retired"),
        ]);
        let (provider_paths, imprecise) =
            sparse_provider_observations(&root, &internal, &HashSet::new());
        assert!(provider_paths.is_empty());
        assert!(!imprecise);

        // Poll mode has no filesystem paths; its caller independently sets
        // provider_imprecise=true while requesting a graph rescan only on the
        // ordinary three-second graph poll.
        let (provider_paths, imprecise) =
            sparse_provider_observations(&root, &HashSet::new(), &HashSet::new());
        assert!(provider_paths.is_empty());
        assert!(!imprecise);
    }

    #[test]
    fn shared_actor_first_watcher_turn_rescans_provider_bytes_delivered_while_stopped() {
        assert!(sparse_provider_rescan_required(true, true, false, false));
        assert!(sparse_provider_rescan_required(true, false, true, false));
        assert!(sparse_provider_rescan_required(true, false, false, true));

        assert!(
            !sparse_provider_rescan_required(false, true, true, true),
            "local-only managed storage must not adopt a graph-local provider namespace",
        );
        assert!(
            !sparse_provider_rescan_required(true, false, false, false),
            "a steady exact-event turn must not broaden into a full provider scan",
        );

        assert!(sparse_tick_needs_continuation(
            &SyncRuntimeTick::AdmittedNoop { epoch: 7 },
            true,
            false,
        ));
        assert!(sparse_tick_needs_continuation(
            &SyncRuntimeTick::AdmittedComplete { epoch: 8 },
            true,
            false,
        ));
        assert!(sparse_tick_needs_continuation(
            &SyncRuntimeTick::ProviderMutation {
                batch_id: tine_core::oplog::BatchId::from_uuid(uuid::Uuid::from_u128(8)),
            },
            false,
            false,
        ));
        assert!(
            !sparse_tick_needs_continuation(
                &SyncRuntimeTick::AdmittedNoop { epoch: 9 },
                false,
                false,
            ),
            "ordinary quiet graph admission must not become an idle hot loop",
        );
        assert!(
            sparse_tick_needs_continuation(
                &SyncRuntimeTick::AdmittedNoop { epoch: 9 },
                false,
                true,
            ),
            "work the actor still names must drain without another filesystem edge",
        );
        assert!(
            !sparse_tick_needs_continuation(
                &SyncRuntimeTick::RecoveryBlocked("waiting for provider bytes".into()),
                true,
                false,
            ),
            "blocked provider work keeps the existing backoff policy",
        );
    }

    fn shared_active_snapshot(
        watcher_pending: bool,
        provider_runnable: bool,
    ) -> SyncRuntimeStatusSnapshot {
        let mut snapshot = runtime_snapshot(SyncRuntimeLifecycle::Active);
        snapshot.shared_role = Some(tine_core::sync_runtime::SyncSharedRole::Initiator);
        snapshot.shared_phase = Some(tine_core::sync_runtime::SyncSharedPhase::Active);
        snapshot.watcher.pending = watcher_pending;
        // The broad protocol inventory a receiving device really reported at the
        // observed failure: three delivered provider items behind an idle
        // watcher. It is deliberately NOT what the scheduler reads.
        snapshot.provider_pending = if provider_runnable { 3 } else { 0 };
        snapshot.provider_runnable = provider_runnable;
        snapshot
    }

    /// The exact device-A failure state from the two-device journey: the peer's
    /// edit was delivered as provider bytes on disk, the receiving actor knows
    /// about it, the watcher epochs are settled, and no further filesystem event
    /// is coming. The scheduler must schedule the work anyway.
    #[test]
    fn delivered_provider_work_is_its_own_runnable_work_source() {
        let delivered = shared_active_snapshot(false, true);
        assert!(
            delivered.has_runnable_work(),
            "an idle watcher over delivered provider evidence is not an idle actor",
        );
        assert!(
            sparse_tick_needs_continuation(
                &SyncRuntimeTick::AdmittedNoop { epoch: 2 },
                false,
                delivered.has_runnable_work(),
            ),
            "a tick that settled a watcher epoch while provider evidence remains \
             delivered must schedule its own continuation: nothing on a quiet \
             graph will produce a later filesystem event for bytes another \
             device wrote",
        );
        assert!(
            sparse_tick_needs_continuation(
                &SyncRuntimeTick::AdmittedComplete { epoch: 2 },
                false,
                delivered.has_runnable_work(),
            ),
            "an admitted local change must not consume the provider obligation",
        );
    }

    #[test]
    fn a_due_sweep_deadline_forces_the_quiet_watcher_turn() {
        let mut due = runtime_snapshot(SyncRuntimeLifecycle::Active);
        due.sweep_deadline_due = true;
        assert!(due.has_runnable_work());
        assert!(sparse_tick_needs_continuation(
            &SyncRuntimeTick::Idle,
            false,
            due.has_runnable_work(),
        ));

        let source = include_str!("watcher.rs");
        assert!(source.contains("&& !sweep_deadline_due"));
        assert!(source.contains("graph.sweep_deadline_remaining"));
    }

    /// Provider arrival while the receiving actor is already busy with its own
    /// lanes: the drain must continue across every non-terminal tick shape until
    /// the actor itself reports no runnable work.
    #[test]
    fn provider_drain_continues_until_the_actor_reports_no_runnable_work() {
        for remaining in [
            SyncRuntimeTick::AdmittedNoop { epoch: 4 },
            SyncRuntimeTick::AdmittedComplete { epoch: 4 },
            SyncRuntimeTick::Recovering,
            SyncRuntimeTick::ProviderMutation {
                batch_id: tine_core::oplog::BatchId::from_uuid(uuid::Uuid::from_u128(9)),
            },
        ] {
            assert!(
                sparse_tick_needs_continuation(&remaining, false, true),
                "{remaining:?} left runnable provider work behind",
            );
        }
        let drained = shared_active_snapshot(false, false);
        assert!(
            !drained.has_runnable_work(),
            "a drained shared actor names no runnable work",
        );
        assert!(
            !sparse_tick_needs_continuation(
                &SyncRuntimeTick::AdmittedNoop { epoch: 5 },
                false,
                drained.has_runnable_work(),
            ),
            "the last provider item draining to zero must return the scheduler to sleep",
        );
    }

    /// The anti-hot-loop direction, at the schedule rather than the predicate:
    /// with both lanes empty the scheduler must arm no timer at all, so the
    /// inotify branch blocks on the kernel instead of polling.
    #[test]
    fn an_empty_watcher_and_empty_provider_lane_arm_no_timer() {
        let quiet = shared_active_snapshot(false, false);
        assert!(!quiet.has_runnable_work());
        let mut retry = RetrySchedule::default();
        let now = Instant::now();
        match &(SyncRuntimeTick::AdmittedNoop { epoch: 6 }) {
            tick if sparse_tick_is_blocked_without_progress(tick, quiet.has_runnable_work()) => {
                retry.failed(now)
            }
            tick if sparse_tick_needs_continuation(tick, false, quiet.has_runnable_work()) => {
                retry.progressed(now)
            }
            _ => retry.succeeded(),
        }
        assert_eq!(
            retry.remaining(now),
            None,
            "a genuinely quiet shared graph must sleep, not poll",
        );
    }

    /// Known-but-unadvanceable provider work — pending batches blocked on causal
    /// dependencies whose bytes have not arrived — must be paced by backoff, not
    /// by the 10ms progress cadence, while still never sleeping forever.
    #[test]
    fn provider_work_that_cannot_advance_backs_off_instead_of_polling() {
        assert!(
            sparse_tick_is_blocked_without_progress(&SyncRuntimeTick::Idle, true),
            "an Idle tick that left runnable provider work made no progress",
        );
        assert!(
            !sparse_tick_is_blocked_without_progress(&SyncRuntimeTick::Idle, false),
            "an Idle tick over an empty actor is ordinary quiescence",
        );
        assert!(
            !sparse_tick_is_blocked_without_progress(&SyncRuntimeTick::Recovering, true),
            "a Recovering provider turn is progress and keeps the fast cadence",
        );

        let mut retry = RetrySchedule::default();
        let now = Instant::now();
        retry.failed(now);
        let backoff = retry.remaining(now).expect("blocked work stays scheduled");
        assert!(
            backoff >= Duration::from_millis(250),
            "blocked provider work must not be retried at the progress cadence: {backoff:?}",
        );
    }

    /// Ordinary page reads and saves share the actor's serialized request lane
    /// with provider work. The scheduler must therefore keep handing the actor
    /// bounded turns rather than one unbounded drain, so an application request
    /// is never queued behind a whole provider backlog.
    #[test]
    fn provider_continuations_stay_bounded_single_turns() {
        let mut retry = RetrySchedule::default();
        let now = Instant::now();
        retry.progressed(now);
        let gap = retry
            .remaining(now)
            .expect("a provider continuation stays scheduled");
        assert!(
            gap <= Duration::from_millis(10),
            "provider continuation must resume promptly: {gap:?}",
        );
        assert!(
            retry.take_due(now + Duration::from_millis(10)),
            "each continuation is one further bounded turn, not a drain loop",
        );
        assert_eq!(
            retry.remaining(now + Duration::from_millis(10)),
            None,
            "a consumed continuation must not re-arm itself without another tick result",
        );
    }

    /// The managed poll cycle arms a whole-graph rescan only when the graph
    /// actually changed — the same stat-diff contract the Direct lane has always
    /// used, not a second mechanism.
    ///
    /// Before this, `sparse_observations` was handed `need_full = poll_cycle`,
    /// so poll mode (the Android default) pushed `RescanRequired` every 3 s
    /// whether or not anything on disk had moved, and a quiet 1,000-page graph
    /// paid a whole-graph actor rescan every cycle on the one lane every read
    /// needs.
    #[test]
    fn a_quiet_managed_graph_does_not_arm_a_poll_rescan() {
        let graph = TempGraph::new("managed-poll-gate");
        graph.write("pages/Quiet.md", "- quiet\n");
        graph.write("journals/2026_08_18.md", "- journal\n");
        let mut snap = HashMap::new();
        let mut baseline = false;

        assert!(
            managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "the first cycle has no baseline and must arm"
        );
        assert!(
            !managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "a quiet graph must not arm a whole-graph rescan"
        );
        assert!(
            !managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "and must keep not arming"
        );
    }

    /// Everything the Direct lane's gate catches, the managed gate must catch.
    /// A gate that is a superset can only over-arm; one that is a subset loses
    /// external changes, which is the one outcome forbidden here.
    #[test]
    fn the_managed_poll_gate_arms_on_every_shape_the_direct_gate_catches() {
        let graph = TempGraph::new("managed-poll-gate-shapes");
        graph.write("pages/Existing.md", "- existing\n");
        let mut snap = HashMap::new();
        let mut baseline = false;
        assert!(managed_poll_rescan_required(
            &graph.root,
            &mut snap,
            &mut baseline
        ));

        // A create, anywhere an eligible document can live -- including outside
        // the configured page/journal roots (GH #268).
        graph.write("notes/Created.md", "- created\n");
        assert!(
            managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "a create outside pages/ and journals/ must arm"
        );
        assert!(!managed_poll_rescan_required(
            &graph.root,
            &mut snap,
            &mut baseline
        ));

        // An edit that changes length.
        graph.write("pages/Existing.md", "- existing, edited externally\n");
        assert!(
            managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "an external edit must arm"
        );
        assert!(!managed_poll_rescan_required(
            &graph.root,
            &mut snap,
            &mut baseline
        ));

        // A delete.
        std::fs::remove_file(graph.path("notes/Created.md")).unwrap();
        assert!(
            managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "a delete must arm"
        );
        assert!(!managed_poll_rescan_required(
            &graph.root,
            &mut snap,
            &mut baseline
        ));

        // A provider conflict copy: never eligible text, but its appearance
        // still has to reach reconciliation, exactly as on the Direct lane.
        graph.write(
            "pages/Existing.sync-conflict-20260818-000000-ABCDEFG.md",
            "- conflict copy\n",
        );
        assert!(
            managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "a sync conflict copy must arm"
        );

        // Excluded trees must NOT arm: an image drop cannot cost a rescan.
        graph.write("assets/note.md", "- not graph text\n");
        graph.write("logseq/bak/pages/Existing.md", "- backup\n");
        assert!(
            !managed_poll_rescan_required(&graph.root, &mut snap, &mut baseline),
            "excluded trees must not arm a whole-graph rescan"
        );
    }

    #[test]
    fn managed_slot_handoff_scan_is_scheduled_exactly_once_without_a_timer_guess() {
        let mut pending = true;
        assert!(take_sparse_initial_tick(&mut pending));
        assert!(!pending);
        assert!(!take_sparse_initial_tick(&mut pending));
    }

    #[test]
    fn provider_events_are_routed_only_after_this_device_activates_sharing() {
        use tine_core::sync_runtime::SyncSharedPhase;

        assert!(!sparse_provider_lane_is_active(None));
        assert!(!sparse_provider_lane_is_active(Some(
            SyncSharedPhase::SharePrepared
        )));
        assert!(!sparse_provider_lane_is_active(Some(
            SyncSharedPhase::Joining
        )));
        assert!(sparse_provider_lane_is_active(Some(
            SyncSharedPhase::Active
        )));
    }

    #[test]
    fn failed_reconciliation_retries_without_another_filesystem_event() {
        let start = Instant::now();
        let mut retry = RetrySchedule::default();
        retry.failed(start);
        assert!(!retry.take_due(start));
        assert!(retry.take_due(start + RETRY_BACKOFF[0]));

        retry.failed(start + RETRY_BACKOFF[0]);
        assert_eq!(
            retry.remaining(start + RETRY_BACKOFF[0]),
            Some(RETRY_BACKOFF[1])
        );
        retry.succeeded();
        assert_eq!(retry.remaining(start), None);
    }

    #[test]
    fn reconciliation_backoff_is_capped_but_keeps_scheduling() {
        let start = Instant::now();
        let mut retry = RetrySchedule::default();
        for offset in 0..20 {
            retry.failed(start + Duration::from_secs(offset));
        }
        assert_eq!(
            retry.remaining(start + Duration::from_secs(19)),
            Some(*RETRY_BACKOFF.last().unwrap())
        );
    }

    #[test]
    fn recovering_progress_retries_promptly_without_failure_backoff() {
        let start = Instant::now();
        let mut retry = RetrySchedule::default();
        retry.failed(start);
        retry.progressed(start);
        assert_eq!(retry.failures, 0);
        assert!(!retry.take_due(start));
        assert!(retry.take_due(start + Duration::from_millis(10)));
    }

    // Direct Files data-safety audit 2026-08-09, finding 16, in its reachable
    // form. The blindness the audit predicted is not permanent and there IS a
    // fallback — but only when EVERY root fails, which empties `watched` and
    // takes the bounded poll branch. The reachable case is mixed: with two
    // graphs open (inotify watch limits are per-user, so a second large graph is
    // exactly how one root fails while the other is fine), the inotify branch
    // blocked forever on an event from the HEALTHY root, and the failing graph
    // stayed invisible until that other graph happened to change.
    //
    // Two gaps, stated rather than papered over. These cover the wait POLICY,
    // not a real kernel `watch()` failure — exhausting fs.inotify.max_user_watches
    // needs privileges this box does not have — and not the wiring that feeds
    // `desired.difference(&watched)` into it. They are therefore a specification
    // of the rule, not fail-before evidence: `inotify_cycle_wait` did not exist
    // before this change, so there is no earlier build they could have failed
    // against. Do not read them as a regression guard for the loop itself.
    #[test]
    fn an_unwatched_root_bounds_the_wait_so_its_retry_actually_runs() {
        // Everything watched: block until the kernel says something, as before.
        assert_eq!(inotify_cycle_wait(None, false), None);
        // A root we want and do not have: never block indefinitely.
        assert_eq!(inotify_cycle_wait(None, true), Some(UNWATCHED_ROOT_RETRY));
    }

    #[test]
    fn an_unwatched_root_never_delays_a_sooner_scheduled_retry() {
        let sooner = Duration::from_millis(250);
        assert_eq!(inotify_cycle_wait(Some(sooner), true), Some(sooner));
        assert_eq!(inotify_cycle_wait(Some(sooner), false), Some(sooner));
        let later = UNWATCHED_ROOT_RETRY * 4;
        assert_eq!(
            inotify_cycle_wait(Some(later), true),
            Some(UNWATCHED_ROOT_RETRY)
        );
        assert_eq!(inotify_cycle_wait(Some(later), false), Some(later));
    }

    #[test]
    fn pending_paths_are_dispatched_only_to_the_owning_graph() {
        let a = TempGraph::new("owner-a");
        let b = TempGraph::new("owner-b");
        a.write("pages/one.md", "- one\n");
        b.write("journals/2026_07_10.md", "- two\n");
        let graph_a = Graph::open(&a.root);
        let graph_b = Graph::open(&b.root);
        let paths = HashSet::from([a.path("pages/one.md"), b.path("journals/2026_07_10.md")]);
        assert_eq!(pending_for_graph(&paths, &graph_a).len(), 1);
        assert_eq!(pending_for_graph(&paths, &graph_b).len(), 1);
        assert!(pending_for_graph(&paths, &graph_a)
            .iter()
            .all(|path| path.starts_with(&a.root)));
    }

    struct TempGraph {
        root: PathBuf,
    }

    impl TempGraph {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "tine-watch-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("journals")).unwrap();
            std::fs::create_dir_all(root.join("pages")).unwrap();
            Self { root }
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.path(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }

        fn remove(&self, rel: &str) {
            std::fs::remove_file(self.path(rel)).unwrap();
        }

        fn rename(&self, from: &str, to: &str) {
            let to = self.path(to);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::rename(self.path(from), to).unwrap();
        }
    }

    impl Drop for TempGraph {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn direct_watch_paths_keep_the_bound_graph_root() {
        let temp = TempGraph::new("direct-authority");
        let slot = GraphSlot::new(Graph::open(&temp.root), temp.root.clone());

        let (graph, root) = direct_watch_paths(&slot).unwrap();
        assert_eq!(graph.root, temp.root);
        assert_eq!(root, temp.root);
    }

    fn event(kind: notify::event::EventKind, paths: Vec<PathBuf>) -> notify::Event {
        notify::Event {
            kind,
            paths,
            attrs: Default::default(),
        }
    }

    /// Configuration is deliberately not graph text, so `incremental_page_paths`
    /// throws it away — which is exactly why an outside edit to `config.edn` was
    /// invisible until the next graph open. It has to be queued separately, for
    /// every shape a writer can produce: an in-place write, and the temp+rename
    /// that Tine, Logseq and Syncthing all actually use.
    #[test]
    fn a_config_edn_write_is_queued_even_though_it_is_not_graph_text() {
        use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RenameMode};
        let config = PathBuf::from("/graph/logseq/config.edn");

        for kind in [
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            EventKind::Create(CreateKind::File),
        ] {
            let mut pending = Pending::default();
            pending.add_event(event(kind, vec![config.clone()]));
            assert!(
                pending.paths.is_empty(),
                "{kind:?}: configuration is not graph text and must not enter the page queue"
            );
            assert!(
                pending.config_paths.contains(&config),
                "{kind:?}: but it must reach the configuration queue"
            );
        }
    }

    #[test]
    fn an_ordinary_page_write_queues_no_configuration_work() {
        use notify::event::{DataChange, EventKind, ModifyKind};
        let mut pending = Pending::default();
        pending.add_event(event(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            vec![PathBuf::from("/graph/pages/Alpha.md")],
        ));
        assert!(pending.config_paths.is_empty());
        // And an unrelated EDN file is not configuration either. The filename
        // gate is cheap and rough; `is_config_file_path` is the decision.
        let mut pending = Pending::default();
        pending.add_event(event(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            vec![PathBuf::from("/graph/logseq/pages-metadata.edn")],
        ));
        assert!(pending.config_paths.is_empty());
    }

    fn new_page(name: &str) -> PageDto {
        PageDto {
            activation: None,
            name: name.to_owned(),
            kind: PageKind::Page,
            title: name.to_owned(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: format!("watcher-{}", name.replace(' ', "-")),
                raw: "local".to_owned(),
                ..BlockDto::default()
            }],
            rev: None,
            format: Format::Md,
            read_only: false,
            path: String::new(),
            guide: false,
        }
    }

    fn new_journal(name: &str) -> PageDto {
        let mut page = new_page(name);
        page.kind = PageKind::Journal;
        page
    }

    /// Warm the parsed page cache and exercise one ordinary Direct save. This
    /// deliberately does not build the core's optional complete identity index.
    fn warm_direct_graph(graph: &Graph) {
        warm_cache(graph);
        let mut anchor = graph.load_by_path("pages/Anchor.md").unwrap().unwrap();
        anchor.blocks[0].raw = "warm guarded identity".to_owned();
        graph
            .save_page(&anchor, anchor.rev.as_deref())
            .expect("warm guarded identity save");
    }

    #[test]
    fn drained_frontier_routes_to_a_same_root_replacement_instance() {
        let graph_dir = TempGraph::new("watcher-drained-frontier-refresh");
        graph_dir.write("pages/Anchor.md", "- anchor\n");

        let old_slot = GraphSlot::new(Graph::open(&graph_dir.root), graph_dir.root.clone());
        let old_graph = old_slot.legacy_graph_cloned().unwrap();
        warm_direct_graph(&old_graph);
        let old_ticket = old_graph.note_graph_text_external_observation();
        let mut graphs = HashMap::from([(
            "main".to_owned(),
            WatchedGraph {
                assets: AssetWatchState::new(old_graph.assets_path()),
                legacy_graph: old_graph,
                root: graph_dir.root.clone(),
                snap: HashMap::new(),
                baseline: true,
                last_reconcile_error: Some("retired retry".to_owned()),
                retry: RetrySchedule::default(),
                pending_observation_epoch: Some(old_ticket),
            },
        )]);

        let replacement_slot = Arc::new(GraphSlot::new(
            Graph::open(&graph_dir.root),
            graph_dir.root.clone(),
        ));
        let replacement = replacement_slot.legacy_graph_cloned().unwrap();
        warm_direct_graph(&replacement);
        graph_dir.write(
            "pages/Replacement Event.md",
            "- external replacement event\n",
        );
        let external = graph_dir.path("pages/Replacement Event.md");
        let replacement_ticket = replacement.note_graph_text_external_observation();
        let drained = HashMap::from([(graph_dir.root.clone(), replacement_ticket)]);

        route_drained_direct_frontiers(
            &mut graphs,
            vec![("main".to_owned(), replacement_slot)],
            &drained,
            |slot| slot.legacy_graph().ok().map(|graph| graph.assets_path()),
        );

        let routed = graphs.get_mut("main").unwrap();
        assert!(routed
            .legacy_graph
            .owns_graph_text_external_observation_ticket(replacement_ticket));
        assert!(!routed.baseline);
        assert!(routed.last_reconcile_error.is_none());
        assert!(routed.pending_observation_epoch.is_none());

        routed.pending_observation_epoch = Some(replacement_ticket);
        routed.legacy_graph.sync_file_checked(&external).unwrap();
        let reconciled = routed.pending_observation_epoch.take().unwrap();
        assert!(routed
            .legacy_graph
            .acknowledge_graph_text_external_observations(reconciled));
        routed
            .legacy_graph
            .save_page(&new_page("Creation After Refresh"), None)
            .unwrap();
        assert!(graph_dir.path("pages/Creation After Refresh.md").exists());
    }

    /// A quiet poll must publish an exact empty observation rather than an
    /// uncertain one. Whether that preserves a live guarded index belongs to
    /// `tine-core` and is tested beside the private index builder there.
    #[test]
    fn quiet_poll_cycles_publish_an_exact_empty_observation() {
        let tg = TempGraph::new("poll-warm");
        tg.write("pages/Anchor.md", "- anchor\n");
        tg.write("Root note.md", "- root\n");
        let graph = Graph::open(&tg.root);
        let snap = collect_graph_text_files(&graph).files;
        for _ in 0..3 {
            let current = collect_graph_text_files(&graph);
            let (changed, uncertain) = poll_observation(&snap, &current);
            assert!(changed.is_empty());
            assert!(!uncertain);
        }
    }

    /// The other half of the same policy: a poll cycle that did observe an
    /// external change must publish its exact path. Applying that path to a
    /// live guarded index is tested inside `tine-core`.
    #[test]
    fn a_poll_cycle_reports_what_it_actually_observed() {
        let tg = TempGraph::new("poll-observe");
        tg.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&tg.root);
        let snap = collect_graph_text_files(&graph).files;
        tg.write("Root note.md", "title:: Root note\n\n- external\n");
        let current = collect_graph_text_files(&graph);
        let (changed, uncertain) = poll_observation(&snap, &current);
        assert!(!uncertain);
        assert_eq!(changed, vec![tg.path("Root note.md")]);
    }

    /// The fallback half. If the rescan could not read part of the graph, what
    /// it found is NOT a complete account of what changed, and the index must be
    /// invalidated rather than exactly updated -- otherwise a save would trust
    /// an index that is missing whatever lives behind the unreadable directory.
    #[test]
    fn an_incomplete_poll_scan_publishes_uncertainty() {
        let tg = TempGraph::new("poll-incomplete");
        tg.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&tg.root);
        let snap = collect_graph_text_files(&graph).files;
        let mut incomplete = collect_graph_text_files(&graph);
        incomplete.complete = false;
        let (changed, uncertain) = poll_observation(&snap, &incomplete);
        assert!(changed.is_empty());
        assert!(uncertain);
    }

    /// The same thing end to end, where the walk itself decides. Root ignores
    /// directory permissions, so this can only be demonstrated when the test
    /// user is not root; it self-skips rather than passing vacuously.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_makes_the_scan_report_itself_incomplete() {
        use std::os::unix::fs::PermissionsExt;

        let tg = TempGraph::new("poll-unreadable");
        tg.write("Archive/Filed.md", "- filed\n");
        tg.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&tg.root);
        assert!(collect_graph_text_files(&graph).complete);

        let blocked = tg.path("Archive");
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = std::fs::read_dir(&blocked).is_ok();
        let observed = collect_graph_text_files(&graph);
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755)).unwrap();

        if readable_anyway {
            return; // running as root; permissions prove nothing here
        }
        assert!(
            !observed.complete,
            "a directory the scan could not read must make the scan report itself incomplete"
        );
    }

    fn assert_new_page_refused(graph: &Graph, name: &str) {
        assert_eq!(
            graph.save_page(&new_page(name), None).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists,
            "{name} must remain owned by the observed external graph text"
        );
    }

    fn assert_new_page_waits_for_reconciliation(graph: &Graph, name: &str) {
        assert_eq!(
            graph.save_page(&new_page(name), None).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "{name} creation must not race the watcher debounce window"
        );
    }

    fn reconcile_external_path(graph: &Graph, path: &Path) {
        let epoch = graph.graph_text_external_observation_ticket();
        graph
            .sync_file_checked(path)
            .expect("debounced exact-path reconciliation");
        graph.acknowledge_graph_text_external_observations(epoch);
    }

    #[test]
    fn legacy_graph_root_text_create_delete_rename_and_semantics_reach_guarded_identity() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

        for extension in ["md", "org"] {
            let graph_dir = TempGraph::new(&format!("root-text-{extension}"));
            graph_dir.write("pages/Anchor.md", "- anchor\n");
            let graph = Graph::open(&graph_dir.root);
            warm_direct_graph(&graph);

            let created_rel = format!("nonstandard/deep/Physical Name.{extension}");
            graph_dir.write(
                &created_rel,
                &format!("title:: Created {extension}\n\n- external\n"),
            );
            assert!(observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                Some(&event(
                    EventKind::Create(CreateKind::File),
                    vec![graph_dir.path(&created_rel)],
                )),
            ));
            assert_new_page_waits_for_reconciliation(&graph, &format!("Created {extension}"));
            reconcile_external_path(&graph, &graph_dir.path(&created_rel));
            assert_new_page_refused(&graph, &format!("Created {extension}"));

            let deleted_rel = format!("nonstandard/deep/Delete {extension}.{extension}");
            graph_dir.write(&deleted_rel, "- external\n");
            observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                Some(&event(
                    EventKind::Create(CreateKind::File),
                    vec![graph_dir.path(&deleted_rel)],
                )),
            );
            assert_new_page_waits_for_reconciliation(&graph, &format!("Delete {extension}"));
            reconcile_external_path(&graph, &graph_dir.path(&deleted_rel));
            assert_new_page_refused(&graph, &format!("Delete {extension}"));
            graph_dir.remove(&deleted_rel);
            let delete_event = event(
                EventKind::Remove(RemoveKind::File),
                vec![graph_dir.path(&deleted_rel)],
            );
            let deletion =
                legacy_graph_text_observation(&graph, &graph_dir.root, Some(&delete_event));
            assert!(!deletion.uncertain);
            assert_eq!(deletion.exact_paths, vec![graph_dir.path(&deleted_rel)]);
            observe_legacy_graph_text_event(&graph, &graph_dir.root, Some(&delete_event));
            assert_new_page_waits_for_reconciliation(&graph, &format!("Delete {extension}"));
            let delete_epoch = graph.graph_text_external_observation_ticket();
            graph
                .sync_deleted_file(&graph_dir.path(&deleted_rel))
                .expect("debounced deletion reconciliation");
            graph.acknowledge_graph_text_external_observations(delete_epoch);

            let old_rel = format!("nonstandard/deep/Old {extension}.{extension}");
            let new_rel = format!("nonstandard/deep/New {extension}.{extension}");
            graph_dir.write(&old_rel, "- external\n");
            observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                Some(&event(
                    EventKind::Create(CreateKind::File),
                    vec![graph_dir.path(&old_rel)],
                )),
            );
            assert_new_page_waits_for_reconciliation(&graph, &format!("Old {extension}"));
            reconcile_external_path(&graph, &graph_dir.path(&old_rel));
            assert_new_page_refused(&graph, &format!("Old {extension}"));
            graph_dir.rename(&old_rel, &new_rel);
            let rename_event = event(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                vec![graph_dir.path(&old_rel), graph_dir.path(&new_rel)],
            );
            let rename =
                legacy_graph_text_observation(&graph, &graph_dir.root, Some(&rename_event));
            assert!(!rename.uncertain);
            assert_eq!(
                rename.exact_paths,
                vec![graph_dir.path(&old_rel), graph_dir.path(&new_rel)]
            );
            observe_legacy_graph_text_event(&graph, &graph_dir.root, Some(&rename_event));
            assert_new_page_waits_for_reconciliation(&graph, &format!("New {extension}"));
            let rename_epoch = graph.graph_text_external_observation_ticket();
            graph
                .sync_deleted_file(&graph_dir.path(&old_rel))
                .expect("debounced rename source reconciliation");
            graph
                .sync_file_checked(&graph_dir.path(&new_rel))
                .expect("debounced rename destination reconciliation");
            graph.acknowledge_graph_text_external_observations(rename_epoch);
            assert_new_page_refused(&graph, &format!("New {extension}"));
        }
    }

    /// A callback arriving after batch A was drained belongs to batch B. Batch
    /// A must acknowledge only its captured frontier, never the graph's newer
    /// global epoch, or creation could race B before B is reconciled.
    #[test]
    fn drained_batch_cannot_acknowledge_a_callback_in_the_next_batch() {
        use notify::event::{CreateKind, EventKind};

        let graph_dir = TempGraph::new("watcher-drain-frontier-race");
        graph_dir.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&graph_dir.root);
        warm_direct_graph(&graph);
        let mut pending = Pending::default();

        graph_dir.write("pages/External A.md", "- external A\n");
        let path_a = graph_dir.path("pages/External A.md");
        assert!(observe_legacy_graph_text_event(
            &graph,
            &graph_dir.root,
            Some(&event(
                EventKind::Create(CreateKind::File),
                vec![path_a.clone()],
            )),
        ));
        pending.add_legacy_observations(vec![(
            graph_dir.root.clone(),
            graph.graph_text_external_observation_ticket(),
        )]);
        let batch_a = pending.take_legacy_observation_epochs();

        graph_dir.write("pages/External B.md", "- external B\n");
        let path_b = graph_dir.path("pages/External B.md");
        assert!(observe_legacy_graph_text_event(
            &graph,
            &graph_dir.root,
            Some(&event(
                EventKind::Create(CreateKind::File),
                vec![path_b.clone()],
            )),
        ));
        pending.add_legacy_observations(vec![(
            graph_dir.root.clone(),
            graph.graph_text_external_observation_ticket(),
        )]);

        graph.sync_file_checked(&path_a).unwrap();
        graph.acknowledge_graph_text_external_observations(batch_a[&graph_dir.root]);
        assert_new_page_waits_for_reconciliation(&graph, "Still Blocked By B");

        let batch_b = pending.take_legacy_observation_epochs();
        graph.sync_file_checked(&path_b).unwrap();
        graph.acknowledge_graph_text_external_observations(batch_b[&graph_dir.root]);
        graph.save_page(&new_page("Now Reconciled"), None).unwrap();
    }

    #[test]
    fn legacy_uncertain_graph_root_events_block_creation_until_reconciliation() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RenameMode};
        use notify::event::{EventAttributes, Flag};

        for case in [
            "config",
            "root-create",
            "directory-rename",
            "rescan",
            "notify-error",
        ] {
            let graph_dir = TempGraph::new(&format!("uncertain-{case}"));
            graph_dir.write("pages/Anchor.md", "- anchor\n");
            let graph = Graph::open(&graph_dir.root);
            warm_direct_graph(&graph);
            graph_dir.write(
                "nonstandard/deep/Physical.md",
                &format!("title:: Epoch {case}\n\n- external\n"),
            );

            let event = match case {
                "config" => {
                    graph_dir.write("logseq/config.edn", "{}\n");
                    Some(event(
                        EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
                        vec![graph_dir.path("logseq/config.edn")],
                    ))
                }
                "root-create" => Some(event(
                    EventKind::Create(CreateKind::Folder),
                    vec![graph_dir.root.clone()],
                )),
                "directory-rename" => {
                    std::fs::create_dir_all(graph_dir.path("nonstandard/from.md")).unwrap();
                    graph_dir.rename("nonstandard/from.md", "nonstandard/to.md");
                    Some(event(
                        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                        vec![
                            graph_dir.path("nonstandard/from.md"),
                            graph_dir.path("nonstandard/to.md"),
                        ],
                    ))
                }
                "rescan" => {
                    let mut attrs = EventAttributes::new();
                    attrs.set_flag(Flag::Rescan);
                    Some(notify::Event {
                        kind: EventKind::Other,
                        paths: Vec::new(),
                        attrs,
                    })
                }
                "notify-error" => None,
                _ => unreachable!(),
            };
            let observation =
                legacy_graph_text_observation(&graph, &graph_dir.root, event.as_ref());
            assert!(observation.relevant, "{case}");
            assert!(observation.uncertain, "{case}");
            assert!(observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                event.as_ref(),
            ));
            assert_new_page_waits_for_reconciliation(&graph, &format!("Epoch {case}"));
        }
    }

    /// Windows ReadDirectoryChangesW reports no sub-kind, so notify emits
    /// `Create(Any)` / `Modify(Any)` / `Remove(Any)` for everything except a
    /// rename. Those used to fall to the catch-all and mark the observation
    /// uncertain, which invalidates the whole guarded identity index -- meaning
    /// every external file event on Windows made the next save rebuild the
    /// entire graph. With an active sync client that is every save.
    ///
    /// Create/Modify are resolvable (the path still exists) and must now take
    /// the exact-path arm. `Remove(Any)` must stay uncertain: the entry is gone,
    /// so we cannot prove it was a file rather than a directory of pages.
    #[test]
    fn ambiguous_windows_event_kinds_take_the_exact_path_arm_except_removal() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind};

        let graph_dir = TempGraph::new("windows-any-kinds");
        graph_dir.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&graph_dir.root);
        warm_direct_graph(&graph);

        graph_dir.write("pages/External.md", "- external\n");
        let path = graph_dir.path("pages/External.md");

        for kind in [
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Any),
        ] {
            let observation = legacy_graph_text_observation(
                &graph,
                &graph_dir.root,
                Some(&event(kind, vec![path.clone()])),
            );
            assert!(observation.relevant, "{kind:?}");
            assert!(
                !observation.uncertain,
                "{kind:?} carries an exact path and must not poison the whole index"
            );
            assert_eq!(observation.exact_paths, vec![path.clone()], "{kind:?}");
        }

        // A directory event still cannot be treated as a page file, even when
        // the sub-kind is missing -- the arm discriminates against the live
        // filesystem, not against the event kind.
        std::fs::create_dir_all(graph_dir.path("pages/sub")).unwrap();
        let directory = legacy_graph_text_observation(
            &graph,
            &graph_dir.root,
            Some(&event(
                EventKind::Create(CreateKind::Any),
                vec![graph_dir.path("pages/sub")],
            )),
        );
        assert!(
            directory.uncertain,
            "an ambiguous event on a directory is not provably a page file"
        );

        // Removal stays conservative: the path is gone, so its kind is unknowable.
        std::fs::remove_file(&path).unwrap();
        let removed = legacy_graph_text_observation(
            &graph,
            &graph_dir.root,
            Some(&event(EventKind::Remove(RemoveKind::Any), vec![path])),
        );
        assert!(
            removed.uncertain,
            "Remove(Any) must stay uncertain -- a removed directory of pages must not be \
             mistaken for a non-page path"
        );
    }

    #[test]
    fn legacy_graph_root_observation_routes_only_to_the_owning_graph() {
        use notify::event::{CreateKind, EventKind};

        let graph_a_dir = TempGraph::new("root-owner-a");
        let graph_b_dir = TempGraph::new("root-owner-b");
        graph_a_dir.write("pages/Anchor.md", "- anchor A\n");
        graph_b_dir.write("pages/Anchor.md", "- anchor B\n");
        let graph_a = Graph::open(&graph_a_dir.root);
        let graph_b = Graph::open(&graph_b_dir.root);

        graph_b_dir.write(
            "nonstandard/deep/Stale.md",
            "title:: Must Stay Stale\n\n- external without a B callback\n",
        );
        graph_a_dir.write("nonstandard/deep/A.md", "- observed A\n");
        let event_a = event(
            EventKind::Create(CreateKind::File),
            vec![graph_a_dir.path("nonstandard/deep/A.md")],
        );
        assert!(observe_legacy_graph_text_event(
            &graph_a,
            &graph_a_dir.root,
            Some(&event_a),
        ));
        let graph_b_observation =
            legacy_graph_text_observation(&graph_b, &graph_b_dir.root, Some(&event_a));
        assert!(!graph_b_observation.relevant);
        assert!(!graph_b_observation.uncertain);
        assert!(graph_b_observation.exact_paths.is_empty());
        assert!(!observe_legacy_graph_text_event(
            &graph_b,
            &graph_b_dir.root,
            Some(&event_a),
        ));
    }

    #[test]
    fn excluded_private_text_and_exact_non_text_events_are_not_published() {
        use notify::event::{CreateKind, EventKind};

        let graph_dir = TempGraph::new("excluded-private");
        graph_dir.write("pages/Anchor.md", "- anchor\n");
        let graph = Graph::open(&graph_dir.root);
        for (relative, claimed) in [
            (".tine-sync/private/Sync.md", "Excluded Sync"),
            ("assets/private/Asset.org", "Excluded Asset"),
            ("logseq/bak/recovery.md", "Excluded Recovery"),
            (".hidden/private.md", "Excluded Hidden"),
        ] {
            graph_dir.write(relative, &format!("title:: {claimed}\n\n- private\n"));
            let event = event(
                EventKind::Create(CreateKind::File),
                vec![graph_dir.path(relative)],
            );
            let observation = legacy_graph_text_observation(&graph, &graph_dir.root, Some(&event));
            assert!(observation.relevant, "{relative}");
            assert!(!observation.uncertain, "{relative}");
            assert!(observation.exact_paths.is_empty(), "{relative}");
            assert!(observe_legacy_graph_text_event(
                &graph,
                &graph_dir.root,
                Some(&event)
            ));
        }

        graph_dir.write(
            "nonstandard/deep/Stale.md",
            "title:: Exact Non Text Is Harmless\n\n- unobserved\n",
        );
        graph_dir.write("nonstandard/deep/image.png", "not graph text\n");
        let non_text = event(
            EventKind::Create(CreateKind::File),
            vec![graph_dir.path("nonstandard/deep/image.png")],
        );
        let observation = legacy_graph_text_observation(&graph, &graph_dir.root, Some(&non_text));
        assert!(!observation.uncertain);
        assert!(observation.exact_paths.is_empty());
        assert!(observe_legacy_graph_text_event(
            &graph,
            &graph_dir.root,
            Some(&non_text)
        ));

        for relative in [".tine-sync", "assets", "logseq/bak", ".hidden"] {
            let private_directory = event(
                EventKind::Create(CreateKind::Folder),
                vec![graph_dir.path(relative)],
            );
            let observation =
                legacy_graph_text_observation(&graph, &graph_dir.root, Some(&private_directory));
            assert!(!observation.uncertain, "{relative}");
            assert!(observation.exact_paths.is_empty(), "{relative}");
        }
    }

    fn warm_cache(graph: &Graph) {
        let _ = graph.search("__watcher_warm_cache__", 1);
    }

    fn sorted_changes(mut changes: Vec<GraphChange>) -> Vec<GraphChange> {
        fn kind_key(kind: PageKind) -> &'static str {
            match kind {
                PageKind::Journal => "journal",
                PageKind::Page => "page",
            }
        }
        changes.sort_by(|a, b| {
            (a.removed, kind_key(a.kind), a.name.as_str()).cmp(&(
                b.removed,
                kind_key(b.kind),
                b.name.as_str(),
            ))
        });
        changes
    }

    fn rel_paths(tg: &TempGraph, rels: &[&str]) -> HashSet<PathBuf> {
        rels.iter().map(|rel| tg.path(rel)).collect()
    }

    fn assert_incremental_matches_full(
        name: &str,
        setup: impl FnOnce(&TempGraph),
        mutate: impl FnOnce(&TempGraph) -> HashSet<PathBuf>,
    ) {
        let tg = TempGraph::new(name);
        setup(&tg);

        let inc_graph = Graph::open(&tg.root);
        let full_graph = Graph::open(&tg.root);
        warm_cache(&inc_graph);
        warm_cache(&full_graph);

        let mut inc_snap = collect_graph_text_files(&inc_graph).files;
        let mut full_snap = inc_snap.clone();

        let paths = mutate(&tg);

        let (inc_changes, inc_conflicts_dirty, inc_errors) =
            incremental_reconcile(&inc_graph, &mut inc_snap, &paths);
        let fresh = collect_graph_text_files(&inc_graph).files;
        let (full_changes, full_conflicts_dirty, full_errors) =
            full_diff_reconcile(&full_graph, &mut full_snap, fresh.clone());

        assert_eq!(inc_snap, fresh, "incremental snap must match full scan");
        assert_eq!(full_snap, fresh, "full snap must match fresh scan");
        assert_eq!(inc_conflicts_dirty, full_conflicts_dirty);
        assert!(inc_errors.is_empty());
        assert!(full_errors.is_empty());
        assert_eq!(
            sorted_changes(inc_changes),
            sorted_changes(full_changes),
            "incremental changes must match full-diff changes"
        );
    }

    fn snapshot_relative_names(tg: &TempGraph, graph: &Graph) -> Vec<String> {
        let mut names: Vec<String> = collect_graph_text_files(graph)
            .files
            .keys()
            .map(|path| {
                path.strip_prefix(&tg.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn snapshot_covers_exactly_what_discovery_covers() {
        // #21: page files in sub-folders must be in the snapshot, so an edit
        // there is reconciled rather than invisible until a graph reopen.
        // GH #268: so must a page at the graph ROOT or in a custom folder --
        // `graph_text_inventory` walks graph-wide through `GraphTextScope`, and
        // a snapshot narrower than discovery makes those pages permanently
        // unreconcilable. Excluded trees stay excluded, by the same authority.
        let tg = TempGraph::new("snapshot-scope");
        tg.write("top.md", "- t\n");
        tg.write("pages/Page.md", "- p\n");
        tg.write("journals/2026_08_06.md", "- j\n");
        tg.write("Archive/mid.org", "* m\n");
        tg.write("Archive/Deep/Deeper/deep.md", "- d\n");
        tg.write("Archive/notes.txt", "ignored\n");
        tg.write(".hidden/skip.md", "- s\n");
        tg.write("assets/embedded.md", "- a\n");
        tg.write("logseq/bak/old.md", "- b\n");

        assert_eq!(
            snapshot_relative_names(&tg, &Graph::open(&tg.root)),
            vec![
                "Archive/Deep/Deeper/deep.md",
                "Archive/mid.org",
                "journals/2026_08_06.md",
                "pages/Page.md",
                "top.md",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_does_not_follow_page_symlinks() {
        use std::os::unix::fs::symlink;

        let tg = TempGraph::new("snapshot-symlink");
        let outside =
            std::env::temp_dir().join(format!("tine-watch-outside-{}.md", std::process::id()));
        let _ = std::fs::remove_file(&outside);
        std::fs::write(&outside, "- outside\n").unwrap();
        symlink(&outside, tg.path("pages/secret.md")).unwrap();

        assert!(snapshot_relative_names(&tg, &Graph::open(&tg.root)).is_empty());

        std::fs::remove_file(&outside).ok();
    }

    #[test]
    fn graph_wide_external_paths_are_routed_like_discovery_routes_them() {
        // GH #268, the event-routing half. The watch is installed recursively on
        // the graph ROOT, so these events all arrive; the reconcile lane used to
        // filter them against `journals/` + `pages/` and silently drop the rest.
        let tg = TempGraph::new("event-scope");
        tg.write("top.md", "- t\n");
        tg.write("Archive/mid.md", "- m\n");
        tg.write("pages/Page.md", "- p\n");
        tg.write("assets/embedded.md", "- a\n");
        tg.write(".hidden/skip.md", "- s\n");
        let graph = Graph::open(&tg.root);

        use notify::event::{DataChange, EventKind, ModifyKind};
        let mut pending = Pending::default();
        for relative in [
            "top.md",
            "Archive/mid.md",
            "pages/Page.md",
            "assets/embedded.md",
            ".hidden/skip.md",
        ] {
            pending.add_event(event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                vec![tg.path(relative)],
            ));
        }

        let mut routed: Vec<String> = pending_for_graph(&pending.paths, &graph)
            .iter()
            .map(|path| {
                path.strip_prefix(&tg.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        routed.sort();
        assert_eq!(routed, vec!["Archive/mid.md", "pages/Page.md", "top.md"]);
    }

    #[test]
    fn unclassified_paths_force_a_full_scan_only_where_pages_could_live() {
        // A directory move reports a path whose nature we cannot know, so it
        // forces a full diff. Excluded trees must not: dropping an image into
        // `assets/` cannot change the text inventory, and rescanning the graph
        // for it is the amplification this program exists to remove.
        let tg = TempGraph::new("full-scan-owner");
        let graph = Graph::open(&tg.root);
        let owned = full_scan_owner_for_graph(
            &HashSet::from([
                tg.path("pages/Moved"),
                tg.path("Archive"),
                tg.path("assets"),
                tg.path("assets/pictures"),
                tg.path(".git/objects"),
                PathBuf::from("/somewhere/else/pages"),
            ]),
            &graph,
        );
        let mut owned: Vec<String> = owned
            .iter()
            .map(|path| {
                path.strip_prefix(&tg.root)
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned())
            })
            .collect();
        owned.sort();
        assert_eq!(owned, vec!["Archive", "pages/Moved"]);
    }

    #[test]
    fn incremental_create_top_level_file_matches_full_diff() {
        assert_incremental_matches_full(
            "create-top",
            |tg| tg.write("pages/Seed.md", "- seed\n"),
            |tg| {
                tg.write("pages/New.md", "- new\n");
                rel_paths(tg, &["pages/New.md"])
            },
        );
    }

    #[test]
    fn incremental_create_is_identified_as_inventory_change() {
        let tg = TempGraph::new("create-inventory");
        tg.write("pages/Seed.md", "- seed\n");
        let graph = Graph::open(&tg.root);
        warm_cache(&graph);
        let mut snap = collect_graph_text_files(&graph).files;
        let path = tg.path("pages/New.md");
        tg.write("pages/New.md", "- new\n");

        let (changes, conflicts_dirty, errors) =
            incremental_reconcile(&graph, &mut snap, &HashSet::from([path]));

        assert!(!conflicts_dirty);
        assert!(errors.is_empty());
        assert_eq!(changes.len(), 1);
        assert!(changes[0].created);
        assert!(!changes[0].removed);
    }

    #[test]
    fn incremental_create_nested_file_matches_full_diff() {
        assert_incremental_matches_full(
            "create-nested",
            |tg| tg.write("pages/Seed.md", "- seed\n"),
            |tg| {
                tg.write("pages/sub/New.md", "- nested\n");
                rel_paths(tg, &["pages/sub/New.md"])
            },
        );
    }

    #[test]
    fn incremental_modify_len_change_matches_full_diff() {
        assert_incremental_matches_full(
            "modify-len",
            |tg| tg.write("pages/Edit.md", "- one\n"),
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                tg.write("pages/Edit.md", "- one\n- two\n");
                rel_paths(tg, &["pages/Edit.md"])
            },
        );
    }

    #[test]
    fn incremental_modify_same_len_mtime_change_matches_full_diff() {
        assert_incremental_matches_full(
            "modify-same-len",
            |tg| tg.write("pages/Edit.md", "- alpha\n"),
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                tg.write("pages/Edit.md", "- beta!\n");
                rel_paths(tg, &["pages/Edit.md"])
            },
        );
    }

    #[test]
    fn explicit_event_reconciles_even_when_snapshot_metadata_is_equal() {
        let tg = TempGraph::new("explicit-same-metadata");
        tg.write("pages/Edit.md", "- alpha\n");
        let graph = Graph::open(&tg.root);
        warm_cache(&graph);
        let path = tg.path("pages/Edit.md");
        tg.write("pages/Edit.md", "- bravo\n"); // equal byte length
        let stamp = file_snapshot(&path).unwrap();
        // Simulate a sync tool preserving every snapshot field: the explicit
        // notify path must still reach Graph::sync_file's content comparison.
        let mut snap = HashMap::from([(path.clone(), stamp)]);
        let (changes, conflicts_dirty, errors) =
            incremental_reconcile(&graph, &mut snap, &HashSet::from([path]));
        assert!(!conflicts_dirty);
        assert!(errors.is_empty());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "Edit");
        assert_eq!(changes[0].kind, PageKind::Page);
        assert!(!changes[0].created);
        assert!(!changes[0].removed);
    }

    #[test]
    fn incremental_remove_top_level_file_matches_full_diff() {
        assert_incremental_matches_full(
            "remove-top",
            |tg| {
                tg.write("pages/Keep.md", "- keep\n");
                tg.write("pages/Delete.md", "- delete\n");
            },
            |tg| {
                tg.remove("pages/Delete.md");
                rel_paths(tg, &["pages/Delete.md"])
            },
        );
    }

    #[test]
    fn incremental_remove_nested_file_matches_full_diff() {
        assert_incremental_matches_full(
            "remove-nested",
            |tg| {
                tg.write("pages/Keep.md", "- keep\n");
                tg.write("pages/sub/Delete.md", "- delete\n");
            },
            |tg| {
                tg.remove("pages/sub/Delete.md");
                rel_paths(tg, &["pages/sub/Delete.md"])
            },
        );
    }

    #[test]
    fn incremental_rename_within_pages_matches_full_diff() {
        assert_incremental_matches_full(
            "rename-within-pages",
            |tg| tg.write("pages/Old.md", "- renamed\n"),
            |tg| {
                tg.rename("pages/Old.md", "pages/New.md");
                rel_paths(tg, &["pages/Old.md", "pages/New.md"])
            },
        );
    }

    #[test]
    fn incremental_rename_across_tree_matches_full_diff() {
        assert_incremental_matches_full(
            "rename-across-tree",
            |tg| tg.write("pages/JournalMove.md", "- moved\n"),
            |tg| {
                tg.rename("pages/JournalMove.md", "journals/2026_07_10.md");
                rel_paths(tg, &["pages/JournalMove.md", "journals/2026_07_10.md"])
            },
        );
    }

    #[test]
    fn incremental_burst_union_matches_full_diff() {
        assert_incremental_matches_full(
            "burst-union",
            |tg| {
                tg.write("pages/Edit.md", "- edit before\n");
                tg.write("pages/Delete.md", "- delete\n");
                tg.write("pages/Keep.md", "- keep\n");
            },
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                tg.write("pages/Edit.md", "- edit after\n");
                tg.remove("pages/Delete.md");
                tg.write("pages/Create.md", "- create\n");
                tg.write("pages/sub/Nested.md", "- nested\n");
                rel_paths(
                    tg,
                    &[
                        "pages/Edit.md",
                        "pages/Delete.md",
                        "pages/Create.md",
                        "pages/sub/Nested.md",
                    ],
                )
            },
        );
    }

    #[test]
    fn bulk_threshold_boundary_is_exclusive_on_both_sides() {
        assert!(!burst_escalates(0));
        assert!(!burst_escalates(BULK_CHANGE_THRESHOLD));
        assert!(burst_escalates(BULK_CHANGE_THRESHOLD + 1));
        assert!(!emit_as_bulk(BULK_CHANGE_THRESHOLD));
        assert!(emit_as_bulk(BULK_CHANGE_THRESHOLD + 1));
    }

    /// `graph-changed-bulk` is a frontend wire contract: the aggregate carries
    /// the same per-page change shape the `graph-changed` event carries.
    #[test]
    fn graph_changed_bulk_wire_shape_is_stable() {
        let wire = serde_json::to_value(GraphChangedBulk {
            changes: vec![GraphChange {
                name: "Page".to_owned(),
                kind: PageKind::Page,
                created: true,
                removed: false,
            }],
        })
        .unwrap();
        let changes = wire.get("changes").and_then(|value| value.as_array());
        let first = changes.and_then(|list| list.first()).expect("one change");
        for key in ["name", "kind", "created", "removed"] {
            assert!(first.get(key).is_some(), "missing bulk change field {key}");
        }
    }

    #[test]
    fn a_drained_batch_above_the_threshold_escalates_to_the_full_branch() {
        // Concord P2 (GH #337 / spec L6): a VCS checkout or first big sync dumps
        // N file events; processing them per-file costs two reads + a parse per
        // path with deliberately no stat shortcut. Above the threshold the batch
        // must take the stat-diff full branch instead.
        let tg = TempGraph::new("burst-escalation");
        tg.write("pages/Seed.md", "- seed\n");
        let graph = Graph::open(&tg.root);
        warm_cache(&graph);
        let mut snap = collect_graph_text_files(&graph).files;

        let mut paths = HashSet::new();
        for index in 0..(BULK_CHANGE_THRESHOLD + 1) {
            let rel = format!("pages/Bulk {index}.md");
            tg.write(&rel, &format!("- bulk {index}\n"));
            paths.insert(tg.path(&rel));
        }

        let (changes, _, used_full, errors) =
            reconcile_pending(&graph, &mut snap, &paths, false, false);
        assert!(
            used_full,
            "a batch of {} paths (> {BULK_CHANGE_THRESHOLD}) must escalate to the full stat-diff branch",
            paths.len()
        );
        assert!(errors.is_empty());
        assert_eq!(changes.len(), BULK_CHANGE_THRESHOLD + 1);
    }

    #[test]
    fn a_drained_batch_at_the_threshold_stays_incremental() {
        // The complement: ordinary bursts (a save, a small sync delta) keep the
        // per-file branch, whose explicit-event semantics deliberately bypass
        // the stat shortcut (see `explicit_event_reconciles_even_when_snapshot_
        // metadata_is_equal`).
        let tg = TempGraph::new("burst-no-escalation");
        tg.write("pages/Seed.md", "- seed\n");
        let graph = Graph::open(&tg.root);
        warm_cache(&graph);
        let mut snap = collect_graph_text_files(&graph).files;

        let mut paths = HashSet::new();
        for index in 0..BULK_CHANGE_THRESHOLD {
            let rel = format!("pages/Bulk {index}.md");
            tg.write(&rel, &format!("- bulk {index}\n"));
            paths.insert(tg.path(&rel));
        }

        let (changes, _, used_full, errors) =
            reconcile_pending(&graph, &mut snap, &paths, false, false);
        assert!(
            !used_full,
            "a batch of exactly {BULK_CHANGE_THRESHOLD} paths must keep the incremental branch"
        );
        assert!(errors.is_empty());
        assert_eq!(changes.len(), BULK_CHANGE_THRESHOLD);
    }

    /// The correctness invariant behind the escalation: whichever branch a burst
    /// takes, the result is the same. Same family as
    /// `incremental_burst_union_matches_full_diff`, sized across the threshold.
    #[test]
    fn incremental_burst_above_threshold_union_matches_full_diff() {
        assert_incremental_matches_full(
            "burst-union-above-threshold",
            |tg| {
                for index in 0..BULK_CHANGE_THRESHOLD {
                    tg.write(&format!("pages/Edit {index}.md"), "- before\n");
                }
                tg.write("pages/Delete.md", "- delete\n");
                tg.write("pages/Keep.md", "- keep\n");
            },
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                let mut rels: Vec<String> = Vec::new();
                for index in 0..BULK_CHANGE_THRESHOLD {
                    let rel = format!("pages/Edit {index}.md");
                    tg.write(&rel, "- after\n");
                    rels.push(rel);
                }
                tg.remove("pages/Delete.md");
                rels.push("pages/Delete.md".to_owned());
                for index in 0..4 {
                    let rel = format!("pages/sub/Created {index}.md");
                    tg.write(&rel, "- created\n");
                    rels.push(rel);
                }
                rels.iter().map(|rel| tg.path(rel)).collect()
            },
        );
    }

    /// Bulk-change measurement + generous regression gate (Concord P2).
    ///
    /// Ignored: the fixture is a few hundred generated files — deliberately NOT
    /// part of the fast unit corpus. Run explicitly:
    ///   cargo nextest run -p tine --run-ignored ignored-only -E 'test(bulk_reconcile)'
    ///
    /// Measures a checkout-shaped change (many files replaced at once under a
    /// running watcher) through both reconcile branches, prints the numbers, and
    /// asserts only an order-of-magnitude ceiling — never a tight timing bound.
    #[test]
    #[ignore = "bulk fixture (hundreds of generated files); run explicitly"]
    fn bulk_reconcile_bench_and_gate() {
        const TOTAL: usize = 800;
        const CHANGED: usize = 400;

        let tg = TempGraph::new("bulk-bench");
        for index in 0..TOTAL {
            tg.write(
                &format!("pages/Bulk {index}.md"),
                &format!("- bulk page {index}\n- second line {index}\n"),
            );
        }
        let inc_graph = Graph::open(&tg.root);
        let full_graph = Graph::open(&tg.root);
        warm_cache(&inc_graph);
        warm_cache(&full_graph);
        let mut inc_snap = collect_graph_text_files(&inc_graph).files;
        let mut full_snap = inc_snap.clone();

        // The external revision: a checkout replaces CHANGED files' contents.
        std::thread::sleep(Duration::from_millis(20));
        let mut paths = HashSet::new();
        for index in 0..CHANGED {
            let rel = format!("pages/Bulk {index}.md");
            tg.write(&rel, &format!("- bulk page {index} switched\n"));
            paths.insert(tg.path(&rel));
        }

        let incremental_started = Instant::now();
        let (inc_changes, _, inc_errors) = incremental_reconcile(&inc_graph, &mut inc_snap, &paths);
        let incremental_elapsed = incremental_started.elapsed();

        let full_started = Instant::now();
        let snapshot = collect_graph_text_files(&full_graph);
        let (full_changes, _, full_errors) =
            full_diff_reconcile(&full_graph, &mut full_snap, snapshot.files);
        let full_elapsed = full_started.elapsed();

        assert!(inc_errors.is_empty());
        assert!(full_errors.is_empty());
        assert_eq!(inc_changes.len(), CHANGED);
        assert_eq!(full_changes.len(), CHANGED);
        println!(
            "bulk-reconcile bench: {CHANGED} changed of {TOTAL} files — \
             incremental branch {}ms, full stat-diff branch {}ms",
            incremental_elapsed.as_millis(),
            full_elapsed.as_millis(),
        );

        // Generous gate: the escalated (full) branch reconciling a 400-file
        // change over an 800-file graph measured 95 ms on the 2026-08 dev box
        // (incremental branch: 94 ms — the branches cost the same for genuinely
        // changed files; escalation buys one consistent snapshot and one
        // aggregate emit, not reconcile speed). 10 s ≈ 100× measured: it catches
        // an order-of-magnitude regression (e.g. an accidental whole-graph
        // reparse per changed file) without ever flaking under load.
        let ceiling = Duration::from_secs(10);
        assert!(
            full_elapsed < ceiling,
            "escalated bulk reconcile took {}ms (ceiling {}ms)",
            full_elapsed.as_millis(),
            ceiling.as_millis(),
        );
    }

    #[test]
    fn reconcile_pending_need_full_uses_full_scan_branch() {
        let tg = TempGraph::new("need-full");
        tg.write("pages/Seed.md", "- seed\n");

        let inc_graph = Graph::open(&tg.root);
        let full_graph = Graph::open(&tg.root);
        warm_cache(&inc_graph);
        warm_cache(&full_graph);

        let mut inc_snap = collect_graph_text_files(&inc_graph).files;
        let mut full_snap = inc_snap.clone();

        tg.write(
            "pages/sub/CreatedByDirEvent.md",
            "- created through dir op\n",
        );
        let incomplete_paths = rel_paths(&tg, &["pages/Seed.md"]);
        let (inc_changes, inc_conflicts_dirty, used_full, inc_errors) =
            reconcile_pending(&inc_graph, &mut inc_snap, &incomplete_paths, true, false);
        let fresh = collect_graph_text_files(&inc_graph).files;
        let (full_changes, full_conflicts_dirty, full_errors) =
            full_diff_reconcile(&full_graph, &mut full_snap, fresh.clone());

        assert!(used_full, "need_full must bypass incremental reconcile");
        assert!(inc_errors.is_empty());
        assert!(full_errors.is_empty());
        assert_eq!(inc_snap, fresh);
        assert_eq!(full_snap, fresh);
        assert_eq!(inc_conflicts_dirty, full_conflicts_dirty);
        assert_eq!(sorted_changes(inc_changes), sorted_changes(full_changes));
    }

    /// DUP-5: every temp shape a Tine writer can rename INTO the live graph
    /// must be recognized here, or the rename event that publishes the real
    /// page is dropped. The restore shape was invisible until 2026-08-25.
    #[test]
    fn recognizes_every_tine_temp_shape_that_lands_in_the_live_graph() {
        for recognized in [
            ".Foo.md.1234.7.tmp",
            ".Foo.md.1234.7.new.tmp",
            ".Foo.md.1234.7.projection.tmp",
            ".tine-restore-1234-7.tmp",
        ] {
            assert!(
                is_tine_atomic_page_temp_path(Path::new(recognized)),
                "{recognized} must be recognized as a Tine atomic temp"
            );
        }
        for foreign in [
            ".tine-restore-x.tmp",
            ".tine-restore-12.tmp",
            "tine-restore-1234-7.tmp",
            ".Foo.md.restore.tmp",
            "Foo.md",
        ] {
            assert!(
                !is_tine_atomic_page_temp_path(Path::new(foreign)),
                "{foreign} must NOT read as a Tine atomic temp"
            );
        }
    }
}
