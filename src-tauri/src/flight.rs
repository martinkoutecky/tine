//! Always-on, privacy-safe diagnostic flight recorder (GH #343; og port of
//! master ADR 0058, in-memory form).
//!
//! **Questions answered.** `diagnostic_report` — the fixed-shape events of the
//! current run plus build/platform facts, as reviewable JSON the user may copy.
//! **Operations accepted.** `record_*` (Rust callers) and the
//! `diagnostic_ipc_event` / `diagnostic_frontend_event` commands append one
//! event; `clear_diagnostics` drops every event.
//!
//! **Privacy boundary (I-5).** An event carries a fixed event name, catalogued
//! command names, closed-vocabulary tokens, counts, booleans and durations —
//! never a message, path, page title, query, URL, note content or credential.
//! Every frontend-supplied string is checked against a closed vocabulary here
//! and dropped (the whole event) when it does not match. The opt-in detailed
//! trace (`crate::debug::diag_private`, `TINE_DEBUG`) is a separate channel and
//! is never copied into a report.
//!
//! **Retention.** The recorder lives in process memory, bounded to
//! [`FLIGHT_MAX_BYTES`] of encoded events (the oldest are evicted first). Once
//! [`persist_init`] has run it is also persisted in app data by
//! [`crate::flight_store`]: this run's events are republished at most every
//! [`FLUSH_INTERVAL`] while new events arrive, at a panic, at an orderly exit
//! and when a mobile app is hidden; the previous run's events and whether it
//! ended cleanly are read once at launch (og ADR 0058).
//!
//! **Cost.** Recording is O(event size) plus O(evicted events); a report is
//! O(retained bytes) ≤ 1 MiB; clearing is O(1). A poisoned recorder lock loses
//! that event and never panics a caller. Callers need not know whether the
//! recorder is empty, full or cleared.

use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::state::AppState;

pub(crate) const FLIGHT_SCHEMA_VERSION: u8 = 1;
/// Upper bound on retained encoded event bytes, newline included — so also
/// the size bound of the persisted history file (one master segment).
pub(crate) const FLIGHT_MAX_BYTES: usize = 1024 * 1024;
/// A dirty recorder is republished at most this often (og ADR 0058).
pub(crate) const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
/// A save that finished faster than this and succeeded is not recorded, so
/// ordinary typing does not evict the rare events a report exists for.
const SAVE_EVENT_THRESHOLD_MS: u64 = 150;

struct FlightRing {
    lines: VecDeque<String>,
    bytes: usize,
}

impl FlightRing {
    const fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            bytes: 0,
        }
    }

    fn push(&mut self, line: String, max_bytes: usize) {
        self.bytes = self.bytes.saturating_add(line.len() + 1);
        self.lines.push_back(line);
        while self.bytes > max_bytes {
            let Some(old) = self.lines.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(old.len() + 1);
        }
    }

    /// The history file's bytes: every line newline-terminated, `bytes` long.
    fn encoded(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes);
        for line in &self.lines {
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out
    }
}

/// The persisted recorder of this process, once [`persist_init`] owns the
/// app-data directory. Lock order: `FLIGHT` is never held while taking this.
struct Persisted {
    store: crate::flight_store::FlightStore,
    previous: Vec<String>,
    previous_unclean: bool,
}

static PERSISTED: OnceLock<Mutex<Persisted>> = OnceLock::new();
static DIRTY: AtomicBool = AtomicBool::new(false);

static FLIGHT: Mutex<FlightRing> = Mutex::new(FlightRing::new());
static START: OnceLock<std::time::Instant> = OnceLock::new();

fn elapsed_ms() -> u64 {
    u64::try_from(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn encode(event: &'static str, fields: Map<String, Value>) -> Option<String> {
    let mut line = Map::new();
    line.insert("schemaVersion".into(), json!(FLIGHT_SCHEMA_VERSION));
    line.insert("elapsedMs".into(), json!(elapsed_ms()));
    line.insert("event".into(), json!(event));
    line.extend(fields);
    serde_json::to_string(&line).ok()
}

/// Append one fixed-shape event. `event` is a source literal; every value in
/// `fields` must already be a closed-vocabulary token, a count, a boolean or a
/// duration. O(event size + evictions).
fn record_fixed_event(event: &'static str, fields: Map<String, Value>) {
    let Some(line) = encode(event, fields) else {
        return;
    };
    if let Ok(mut ring) = FLIGHT.lock() {
        ring.push(line, FLIGHT_MAX_BYTES);
    }
    DIRTY.store(true, Ordering::Release);
}

/// Start the recorder's clock and record which build this run is: fixed tokens
/// only, so a report can tell an x86 session from an x64 one (GH #594). Call
/// once, first thing in `run()`.
pub(crate) fn flight_init() {
    START.get_or_init(std::time::Instant::now);
    record_fixed_event("runtime.started", runtime_started_fields());
}

fn runtime_started_fields() -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    fields.insert("arch".into(), json!(std::env::consts::ARCH));
    fields
}

/// Record a panic as location, thread name and payload kind — never the
/// payload. Called from the panic hook, so it only TRIES the lock: a panic
/// raised while the recorder lock is held loses this event instead of
/// deadlocking.
pub(crate) fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    let mut fields = Map::new();
    fields.insert(
        "location".into(),
        info.location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".into())
            .into(),
    );
    fields.insert(
        "thread".into(),
        std::thread::current().name().unwrap_or("unnamed").into(),
    );
    let message_kind = if info.payload().is::<&str>() {
        "str"
    } else if info.payload().is::<String>() {
        "string"
    } else {
        "non_string"
    };
    fields.insert("messageKind".into(), message_kind.into());
    let Some(line) = encode("runtime.panic", fields) else {
        return;
    };
    let Ok(mut ring) = FLIGHT.try_lock() else {
        return;
    };
    ring.push(line, FLIGHT_MAX_BYTES);
    let encoded = ring.encoded();
    drop(ring);
    // The process may be about to die: publish now, but never wait on a lock
    // the panicking thread might hold.
    if let Some(Ok(persisted)) = PERSISTED.get().map(Mutex::try_lock) {
        let _ = persisted.store.write_history(&encoded);
    }
}

/// Own `<app data>/diagnostics`: read the previous run, arm this run's
/// unclean-exit marker, publish the events recorded so far and start the
/// flusher. Call once from `setup()` (after the single-instance plugin has
/// forwarded a second launch). A directory another live process owns, or any
/// I/O failure, leaves this run in memory only; startup never fails here.
pub(crate) fn persist_init(dir: PathBuf) {
    let opened = match crate::flight_store::FlightStore::open(&dir) {
        Ok(opened) => opened,
        Err(error) => {
            crate::debug::diag_private("flight-history-unavailable", error.to_string());
            return;
        }
    };
    let previous_unclean = opened.previous_unclean;
    let persisted = Persisted {
        store: opened.store,
        previous: opened.previous,
        previous_unclean,
    };
    if PERSISTED.set(Mutex::new(persisted)).is_err() {
        return;
    }
    if previous_unclean {
        record_fixed_event("runtime.previous_exit_unclean", Map::new());
    }
    flush_now();
    let _ = std::thread::Builder::new()
        .name("flight-flush".into())
        .spawn(|| loop {
            std::thread::sleep(FLUSH_INTERVAL);
            if DIRTY.load(Ordering::Acquire) {
                flush_now();
            }
        });
}

/// Republish this run's events if the recorder is persisted. O(retained
/// bytes) ≤ [`FLIGHT_MAX_BYTES`]; a failed write keeps the events dirty for
/// the next attempt and is noted only in the opt-in debug log.
fn flush_now() {
    let Some(persisted) = PERSISTED.get() else {
        return;
    };
    DIRTY.store(false, Ordering::Release);
    let encoded = match FLIGHT.lock() {
        Ok(ring) => ring.encoded(),
        Err(_) => return,
    };
    let Ok(persisted) = persisted.lock() else {
        return;
    };
    if let Err(error) = persisted.store.write_history(&encoded) {
        DIRTY.store(true, Ordering::Release);
        crate::debug::diag_private("flight-history-write-failed", error.to_string());
    }
}

fn set_session_active(active: bool) {
    let Some(persisted) = PERSISTED.get() else {
        return;
    };
    if let Ok(persisted) = persisted.lock() {
        if let Err(error) = persisted.store.set_session_active(active) {
            crate::debug::diag_private("flight-marker-write-failed", error.to_string());
        }
    }
}

/// The orderly end of a run (`RunEvent::Exit`): record it, publish the
/// history, then clear the unclean-exit marker.
pub(crate) fn mark_clean_shutdown() {
    record_fixed_event("runtime.clean_shutdown", Map::new());
    flush_now();
    set_session_active(false);
}

/// GH #426. A mobile OS reaps a hidden app without notice, so on Android and
/// iOS the recorded session follows visibility: it ends when the app is hidden
/// and restarts when the user returns. Desktop ignores this — a minimised
/// window is still a live session whose crash the recorder must report, and
/// `RunEvent::Exit` is its orderly end.
#[tauri::command]
pub(crate) fn diagnostic_session_active(active: bool) {
    if !cfg!(mobile) {
        return;
    }
    let mut fields = Map::new();
    fields.insert("active".into(), json!(active));
    record_fixed_event("runtime.session_active", fields);
    flush_now();
    set_session_active(active);
}

/// Whether the recorder is persisted, and whether the previous run ended
/// without an orderly shutdown. O(1).
pub(crate) fn persisted_state() -> (bool, bool) {
    match PERSISTED.get().map(Mutex::lock) {
        Some(Ok(persisted)) => (true, persisted.previous_unclean),
        _ => (false, false),
    }
}

/// The fixed family of a page-save result: the save wire's own closed tokens,
/// with an `io:<ErrorKind>` family split into `io` plus the bounded kind.
/// Anything else (never produced today) is recorded as `other`.
fn save_family_fields(family: &str, fields: &mut Map<String, Value>) {
    const FAMILIES: [&str; 10] = [
        "ok",
        "conflict",
        "deleted",
        "read-only",
        "invalid-target",
        "twin",
        "repeated",
        "closed",
        "asset-too-large",
        "publication-incomplete",
    ];
    if let Some(kind) = family.strip_prefix("io:") {
        fields.insert("outcome".into(), json!("io"));
        if !kind.is_empty() && kind.len() <= 40 && kind.bytes().all(|b| b.is_ascii_alphanumeric()) {
            fields.insert("ioKind".into(), json!(kind));
        }
    } else if let Some(token) = FAMILIES.iter().find(|token| **token == family) {
        fields.insert("outcome".into(), json!(token));
    } else {
        fields.insert("outcome".into(), json!("other"));
    }
}

/// Record one `save_pages` call that failed or took at least 150 ms
/// (`direct.save`): its outcome family, how many pages it carried and its
/// duration. Page identity, paths and error prose never enter the event.
/// `failure` is the save wire's failure family, `None` for success.
pub(crate) fn record_save(failure: Option<&str>, pages: usize, elapsed: std::time::Duration) {
    let total_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    if failure.is_none() && total_ms < SAVE_EVENT_THRESHOLD_MS {
        return;
    }
    let mut fields = Map::new();
    save_family_fields(failure.unwrap_or("ok"), &mut fields);
    fields.insert("pages".into(), json!(pages));
    fields.insert("totalMs".into(), json!(total_ms));
    record_fixed_event("direct.save", fields);
}

/// How one external batch fared in the watcher: milliseconds and counts, never
/// a path. Carried by `watcher.batch` events and the watcher's devtools ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct WatcherTiming {
    /// "inotify" or "poll" (a poll cycle, chosen or after a refused watch).
    pub(crate) mode: &'static str,
    /// Exact event paths in the batch (0 for a pure full diff).
    pub(crate) event_paths: usize,
    /// Whether the full stat diff ran (poll, unclassifiable event, burst).
    pub(crate) full_diff: bool,
    /// First notification -> reconcile start; `None` for a poll cycle.
    pub(crate) event_to_reconcile_ms: Option<u64>,
    /// Reconcile start -> last window event emitted.
    pub(crate) reconcile_ms: u64,
    /// First notification -> last window event emitted.
    pub(crate) event_to_emit_ms: Option<u64>,
}

/// Record one externally caused graph publication that reached a window
/// (`watcher.batch`): how many page events it carried, whether the sync
/// conflict list changed and, when the watcher measured it, the batch's timing
/// (so a report can show that external edits arrived slowly). No page names or
/// paths. Unit cost: one ring line of about 200 bytes per external batch; an
/// edit of your own writes none.
pub(crate) fn record_watcher_batch(
    pages: usize,
    conflicts_changed: bool,
    timing: Option<&WatcherTiming>,
) {
    let mut fields = Map::new();
    fields.insert("pages".into(), json!(pages));
    fields.insert("conflictsChanged".into(), json!(conflicts_changed));
    if let Some(timing) = timing {
        fields.insert("mode".into(), json!(timing.mode));
        fields.insert("eventPaths".into(), json!(timing.event_paths));
        fields.insert("fullDiff".into(), json!(timing.full_diff));
        fields.insert(
            "eventToReconcileMs".into(),
            json!(timing.event_to_reconcile_ms),
        );
        fields.insert("reconcileMs".into(), json!(timing.reconcile_ms));
        fields.insert("eventToEmitMs".into(), json!(timing.event_to_emit_ms));
    }
    record_fixed_event("watcher.batch", fields);
}

/// The OS refused (or restored) live notifications for a graph root; the
/// watcher polls meanwhile (`watcher.refused`, fixed shape, no path or message).
pub(crate) fn record_watch_refused(refused: bool) {
    let mut fields = Map::new();
    fields.insert("refused".into(), json!(refused));
    record_fixed_event("watcher.refused", fields);
}

/// Commands whose own timing would only describe the recorder.
const SELF_COMMANDS: [&str; 8] = [
    "diagnostic_ipc_event",
    "diagnostic_frontend_event",
    "diagnostic_report",
    "diagnostic_session_active",
    "save_diagnostic_report",
    "clear_diagnostics",
    "debug_info",
    "debug_log",
];

/// Record that an IPC command crossed the slow threshold, completed after
/// being slow, or failed (`ipc.command`). The name must be a registered
/// command ([`crate::command_surface::is_known_command`]) and the phase one of
/// `slow`/`completed`/`failed`; anything else is dropped unrecorded.
#[tauri::command]
pub(crate) fn diagnostic_ipc_event(command: String, phase: String, elapsed_ms: u64) {
    if !crate::command_surface::is_known_command(&command)
        || SELF_COMMANDS.contains(&command.as_str())
        || !matches!(phase.as_str(), "slow" | "completed" | "failed")
    {
        return;
    }
    let mut fields = Map::new();
    fields.insert("command".into(), json!(command));
    fields.insert("phase".into(), json!(phase));
    fields.insert("elapsedMs".into(), json!(elapsed_ms));
    record_fixed_event("ipc.command", fields);
}

const UPDATER_STAGES: [&str; 7] = [
    "manifest_fetch",
    "manifest_parse",
    "target_selection",
    "download",
    "signature_verification",
    "install",
    "relaunch",
];
const UPDATER_CAUSES: [&str; 7] = [
    "network",
    "invalid_manifest",
    "unsupported_target",
    "invalid_signature",
    "install_failed",
    "relaunch_failed",
    "unknown",
];

/// Record one frontend event. Accepted kinds and their fields:
/// `uncaught_error`/`unhandled_rejection`/`heartbeat_delay` (`frontend.health`:
/// line, column, delay), `updater_failure` (`updater.failure`: stage and cause
/// from closed lists), `updater_manual_only` (`updater.manual_only`: a build
/// that updates manually by policy, not a failure — GH #594) and
/// `close_discarded_unsaved` (`runtime.close_discarded_unsaved`: reason
/// `failed`/`still-saving` and a page count — GH #540) and `error_toast`
/// (`frontend.error_toast`: that an error toast was shown; never its text,
/// which may name pages — the opt-in debug log has it). Any other kind, or a
/// token outside its list, drops the event.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn diagnostic_frontend_event(
    kind: String,
    line: Option<u64>,
    column: Option<u64>,
    delay_ms: Option<u64>,
    updater_stage: Option<String>,
    updater_cause: Option<String>,
    close_reason: Option<String>,
    pages: Option<u64>,
) {
    if let Some((event, fields)) = frontend_event_fields(
        &kind,
        line,
        column,
        delay_ms,
        updater_stage.as_deref(),
        updater_cause.as_deref(),
        close_reason.as_deref(),
        pages,
    ) {
        record_fixed_event(event, fields);
    }
}

#[allow(clippy::too_many_arguments)]
fn frontend_event_fields(
    kind: &str,
    line: Option<u64>,
    column: Option<u64>,
    delay_ms: Option<u64>,
    updater_stage: Option<&str>,
    updater_cause: Option<&str>,
    close_reason: Option<&str>,
    pages: Option<u64>,
) -> Option<(&'static str, Map<String, Value>)> {
    let mut fields = Map::new();
    match kind {
        "close_discarded_unsaved" => {
            let reason =
                close_reason.filter(|value| matches!(*value, "failed" | "still-saving"))?;
            fields.insert("reason".into(), json!(reason));
            fields.insert("pages".into(), json!(pages.unwrap_or(0)));
            Some(("runtime.close_discarded_unsaved", fields))
        }
        "error_toast" => Some(("frontend.error_toast", fields)),
        "updater_manual_only" => {
            fields.insert("reason".into(), json!("x86"));
            Some(("updater.manual_only", fields))
        }
        "updater_failure" => {
            let stage = updater_stage.filter(|value| UPDATER_STAGES.contains(value))?;
            let cause = updater_cause.filter(|value| UPDATER_CAUSES.contains(value))?;
            fields.insert("stage".into(), json!(stage));
            fields.insert("cause".into(), json!(cause));
            Some(("updater.failure", fields))
        }
        "uncaught_error" | "unhandled_rejection" | "heartbeat_delay" => {
            fields.insert("kind".into(), json!(kind));
            fields.insert("line".into(), json!(line));
            fields.insert("column".into(), json!(column));
            fields.insert("delayMs".into(), json!(delay_ms));
            Some(("frontend.health", fields))
        }
        _ => None,
    }
}

/// The CPU architecture this binary was built for (`x86`, `x86_64`,
/// `aarch64`, …). The updater uses it to keep the experimental 32-bit Windows
/// build on manual updates. O(1); never fails.
#[tauri::command]
pub(crate) fn app_architecture() -> &'static str {
    std::env::consts::ARCH
}

fn safe_build_commit(value: String) -> Option<String> {
    (value.len() >= 7 && value.len() <= 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(value)
}

fn safe_build_time(value: String) -> Option<String> {
    (value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'-' | b':' | b'.' | b'T' | b'Z' | b'+')
        }))
    .then_some(value)
}

/// A diagnostic report the user reviews before sharing it.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticReport {
    pub(crate) text: String,
    pub(crate) suggested_file_name: String,
}

fn build_diagnostic_report(
    graph_bindings: Option<usize>,
    graphs: Vec<Value>,
    build_commit: String,
    build_time: String,
) -> DiagnosticReport {
    let parse = |lines: &mut dyn Iterator<Item = &String>| -> Vec<Value> {
        lines
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    };
    let events = FLIGHT
        .lock()
        .map(|ring| parse(&mut ring.lines.iter()))
        .unwrap_or_default();
    let (retained, previous_unclean) = persisted_state();
    let previous = match PERSISTED.get().map(Mutex::lock) {
        Some(Ok(persisted)) => parse(&mut persisted.previous.iter()),
        _ => Vec::new(),
    };
    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let report = json!({
        "schemaVersion": FLIGHT_SCHEMA_VERSION,
        "generatedAtUnixMs": generated_at,
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "buildCommit": safe_build_commit(build_commit),
            "buildTime": safe_build_time(build_time),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "privacy": {
            "automaticUpload": false,
            "containsGraphContent": false,
            // Counts, quantiles and durations of the open graphs (the `graphs`
            // section): numbers only, never a name, text or hash of either (I-5).
            "containsGraphStatistics": true,
            "containsPaths": false,
            "containsPageTitles": false,
            "containsQueriesOrUrls": false,
            "containsCredentials": false,
            "verboseDebugLogIncluded": false,
        },
        "runtime": {
            "recorderActive": true,
            "retainedAcrossRuns": retained,
            "previousExitUnclean": previous_unclean,
            "verboseDebugEnabled": crate::debug::debug_enabled(),
            "graphStateUnavailable": graph_bindings.is_none(),
            "graphBindings": graph_bindings.unwrap_or(0),
        },
        "graphs": graphs,
        "sessions": { "previous": previous, "current": events },
    });
    DiagnosticReport {
        text: serde_json::to_string_pretty(&report).unwrap_or_else(|_| {
            "{\"schemaVersion\":1,\"error\":\"report_serialization_failed\"}".into()
        }),
        suggested_file_name: format!("tine-diagnostics-{generated_at}.json"),
    }
}

/// Launch timings and graph-shape statistics of every open graph (GH #623):
/// reporters cannot share a graph, so the dump carries the numbers that
/// diagnose its performance instead. Statistics only, built by
/// `Store::diagnostics`. Runs on a blocking thread: the shape pass walks every
/// parsed page and may pay a one-time projection parse.
async fn collect_graph_diagnostics(app: &tauri::AppHandle) -> (Option<usize>, Vec<Value>) {
    let slots = {
        let state = tauri::Manager::state::<AppState>(app);
        state.graphs.read().ok().map(|graphs| graphs.entries())
    };
    let Some(slots) = slots else {
        return (None, Vec::new());
    };
    let bindings = slots.len();
    let graphs = tauri::async_runtime::spawn_blocking(move || {
        slots
            .iter()
            .map(|(_, slot)| slot.store.diagnostics())
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    (Some(bindings), graphs)
}

/// Build the report: app version, build commit/time (dropped unless they are a
/// hex commit and an ISO timestamp), OS/arch, privacy flags, the number of open
/// graph bindings, per-graph launch timings and shape statistics, and every
/// retained event of this run. Never fails; an unreadable graph registry is
/// reported as a flag.
#[tauri::command]
pub(crate) async fn diagnostic_report(
    app: tauri::AppHandle,
    build_commit: String,
    build_time: String,
) -> DiagnosticReport {
    let (graph_bindings, graphs) = collect_graph_diagnostics(&app).await;
    build_diagnostic_report(graph_bindings, graphs, build_commit, build_time)
}

/// Save a freshly built report where the user chooses (desktop save dialog).
/// `false` when the user cancelled. Mobile has no save dialog: Copy report.
#[tauri::command]
pub(crate) async fn save_diagnostic_report(
    app: tauri::AppHandle,
    build_commit: String,
    build_time: String,
) -> Result<bool, String> {
    let (graph_bindings, graphs) = collect_graph_diagnostics(&app).await;
    let report = build_diagnostic_report(graph_bindings, graphs, build_commit, build_time);
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt as _;
        let suggested = report.suggested_file_name.clone();
        let chosen = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_file_name(suggested)
                .add_filter("JSON", &["json"])
                .blocking_save_file()
        })
        .await
        .map_err(|_| "The save dialog failed.".to_owned())?;
        let Some(chosen) = chosen else {
            return Ok(false);
        };
        let path = chosen
            .into_path()
            .map_err(|_| "The chosen destination is not a local file.".to_owned())?;
        crate::flight_store::FlightStore::save_report(&path, &report.text).map_err(|error| {
            crate::debug::diag_private("diagnostic-report-save-failed", error.to_string());
            "The diagnostic report could not be saved.".to_owned()
        })?;
        Ok(true)
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, report);
        Err("Save report is available on desktop; use Copy report on this device.".into())
    }
}

/// Drop every retained event of this run and the previous one, record
/// `diagnostics.cleared` and republish the history. O(1) plus one flush.
#[tauri::command]
pub(crate) fn clear_diagnostics() {
    if let Ok(mut ring) = FLIGHT.lock() {
        *ring = FlightRing::new();
    }
    if let Some(Ok(mut persisted)) = PERSISTED.get().map(Mutex::lock) {
        persisted.previous.clear();
    }
    record_fixed_event("diagnostics.cleared", Map::new());
    flush_now();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn production() -> &'static str {
        include_str!("flight.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production precedes tests")
    }

    #[test]
    fn the_ring_evicts_oldest_events_and_stays_bounded() {
        let mut ring = FlightRing::new();
        for index in 0..10 {
            ring.push(format!("{index:0>40}"), 100);
        }
        assert!(ring.bytes <= 100, "{}", ring.bytes);
        assert_eq!(ring.lines.len(), 2);
        assert!(ring.lines.back().unwrap().ends_with('9'));
    }

    #[test]
    fn a_report_carries_recorded_events_and_the_build_but_no_free_text() {
        let _ = diagnostic_ipc_event("save_pages".into(), "slow".into(), 612);
        record_save(
            Some("io:PermissionDenied"),
            2,
            std::time::Duration::from_millis(3),
        );
        record_save(
            Some("/home/someone/graph/pages/secret.md"),
            1,
            std::time::Duration::ZERO,
        );
        let report =
            build_diagnostic_report(Some(1), Vec::new(), "abcdef1".into(), "/home/x".into());
        let parsed: Value = serde_json::from_str(&report.text).unwrap();
        let events = parsed["sessions"]["current"].as_array().unwrap();
        assert!(events.iter().any(|event| event["event"] == "ipc.command"
            && event["command"] == "save_pages"
            && event["elapsedMs"] == 612));
        assert!(events.iter().any(|event| event["event"] == "direct.save"
            && event["outcome"] == "io"
            && event["ioKind"] == "PermissionDenied"));
        assert!(events
            .iter()
            .any(|event| event["event"] == "direct.save" && event["outcome"] == "other"));
        assert_eq!(parsed["app"]["buildCommit"], "abcdef1");
        assert!(parsed["app"]["buildTime"].is_null());
        assert_eq!(parsed["runtime"]["retainedAcrossRuns"], false);
        assert!(!report.text.contains("/home/"), "{}", report.text);
        assert!(!report.text.contains("secret"), "{}", report.text);
    }

    #[test]
    fn the_report_carries_each_graphs_statistics_and_says_so() {
        let graph = json!({ "launch": { "readyMs": 4200 }, "shape": { "pages": 1075 } });
        let report =
            build_diagnostic_report(Some(1), vec![graph.clone()], String::new(), String::new());
        let parsed: Value = serde_json::from_str(&report.text).unwrap();
        assert_eq!(parsed["graphs"], json!([graph]));
        assert_eq!(parsed["privacy"]["containsGraphStatistics"], true);
        // Statistics are not content: the other flags stay false.
        assert_eq!(parsed["privacy"]["containsGraphContent"], false);
        assert_eq!(parsed["privacy"]["containsPageTitles"], false);
        let empty = build_diagnostic_report(None, Vec::new(), String::new(), String::new());
        let parsed: Value = serde_json::from_str(&empty.text).unwrap();
        assert_eq!(parsed["graphs"], json!([]));
    }

    #[test]
    fn unknown_commands_phases_and_self_timings_are_not_recorded() {
        diagnostic_ipc_event("My secret page".into(), "slow".into(), 1);
        diagnostic_ipc_event("save_pages".into(), "a path /tmp/x".into(), 1);
        diagnostic_ipc_event("diagnostic_report".into(), "slow".into(), 1);
        let ring = FLIGHT.lock().unwrap();
        for forbidden in [
            "My secret page",
            "/tmp/x",
            "\"command\":\"diagnostic_report\"",
        ] {
            assert!(
                !ring.lines.iter().any(|line| line.contains(forbidden)),
                "{forbidden}"
            );
        }
    }

    /// Master 271885b20: a report shows how slowly external edits arrived.
    /// The batch event carries timings and counts, never a path or a name.
    #[test]
    fn a_report_carries_watcher_batch_timings_but_no_paths() {
        let timing = WatcherTiming {
            mode: "inotify",
            event_paths: 2,
            full_diff: false,
            event_to_reconcile_ms: Some(731_001),
            reconcile_ms: 17,
            event_to_emit_ms: Some(731_018),
        };
        record_watcher_batch(3, true, Some(&timing));
        let report = build_diagnostic_report(Some(1), Vec::new(), String::new(), String::new());
        let report: Value = serde_json::from_str(&report.text).unwrap();
        let batch = report["sessions"]["current"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["event"] == "watcher.batch" && event["eventToEmitMs"] == 731_018)
            .expect("a timed watcher.batch event");
        assert_eq!(batch["mode"], "inotify");
        assert_eq!(batch["eventPaths"], 2);
        assert_eq!(batch["fullDiff"], false);
        assert_eq!(batch["eventToReconcileMs"], 731_001);
        assert_eq!(batch["reconcileMs"], 17);
        assert_eq!(batch["pages"], 3);
        assert_eq!(batch["conflictsChanged"], true);
    }

    #[test]
    fn a_fast_successful_save_is_not_an_event() {
        let before: Vec<String> = FLIGHT.lock().unwrap().lines.iter().cloned().collect();
        record_save(None, 1, std::time::Duration::from_millis(3));
        let after: Vec<String> = FLIGHT.lock().unwrap().lines.iter().cloned().collect();
        let added: Vec<_> = after.iter().filter(|line| !before.contains(line)).collect();
        assert!(!added
            .iter()
            .any(|line| line.contains("\"outcome\":\"ok\"") && line.contains("\"totalMs\":3")));
    }

    #[test]
    fn a_session_start_names_its_version_and_arch() {
        let fields = runtime_started_fields();
        assert_eq!(
            fields.get("version"),
            Some(&json!(env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(fields.get("arch"), Some(&json!(std::env::consts::ARCH)));
        assert_eq!(fields.len(), 2);
        assert!(production()
            .contains("record_fixed_event(\"runtime.started\", runtime_started_fields())"));
    }

    /// GH #594: the x86 build's manual-update policy is its own event, not an
    /// `updater.failure unsupported_target` on every launch.
    #[test]
    fn a_manual_only_build_is_not_an_updater_failure() {
        let (event, fields) = frontend_event_fields(
            "updater_manual_only",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(event, "updater.manual_only");
        assert_eq!(fields.get("reason"), Some(&json!("x86")));
        assert_eq!(fields.len(), 1);
    }

    /// OG-TOAST: every error toast leaves a fixed-shape trace in the persisted
    /// recorder, with no text field at all.
    #[test]
    fn an_error_toast_is_recorded_without_its_text() {
        let (event, fields) =
            frontend_event_fields("error_toast", None, None, None, None, None, None, None).unwrap();
        assert_eq!(event, "frontend.error_toast");
        assert!(fields.is_empty());
    }

    #[test]
    fn updater_failures_accept_only_fixed_stage_and_cause_tokens() {
        let (event, fields) = frontend_event_fields(
            "updater_failure",
            None,
            None,
            None,
            Some("download"),
            Some("network"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(event, "updater.failure");
        assert_eq!(fields.get("stage"), Some(&json!("download")));
        for (stage, cause) in [
            (Some("download"), Some("GET https://example.test/x failed")),
            (Some("C:\\Users\\me"), Some("network")),
            (None, Some("network")),
        ] {
            assert!(frontend_event_fields(
                "updater_failure",
                None,
                None,
                None,
                stage,
                cause,
                None,
                None
            )
            .is_none());
        }
    }

    /// GH #540: a close that discarded drafts records only a fixed reason and a
    /// page count, never page titles.
    #[test]
    fn a_close_that_discards_drafts_records_only_a_fixed_reason_and_a_count() {
        let (event, fields) = frontend_event_fields(
            "close_discarded_unsaved",
            None,
            None,
            None,
            None,
            None,
            Some("failed"),
            Some(3),
        )
        .unwrap();
        assert_eq!(event, "runtime.close_discarded_unsaved");
        assert_eq!(fields.get("reason"), Some(&json!("failed")));
        assert_eq!(fields.get("pages"), Some(&json!(3)));
        assert_eq!(fields.len(), 2);
        for refused in [
            None,
            Some(""),
            Some("My secret page"),
            Some("/home/someone/graph"),
        ] {
            assert!(frontend_event_fields(
                "close_discarded_unsaved",
                None,
                None,
                None,
                None,
                None,
                refused,
                Some(1),
            )
            .is_none());
        }
        assert!(
            frontend_event_fields("free text kind", None, None, None, None, None, None, None)
                .is_none()
        );
    }

    #[test]
    fn build_metadata_rejects_strings_that_could_smuggle_report_content() {
        assert_eq!(
            safe_build_commit("abcdef1".into()).as_deref(),
            Some("abcdef1")
        );
        assert_eq!(safe_build_commit("page title".into()), None);
        assert_eq!(
            safe_build_time("2026-08-25T10:00:00.000Z".into()).as_deref(),
            Some("2026-08-25T10:00:00.000Z")
        );
        assert_eq!(safe_build_time("/home/person/graph".into()), None);
    }

    /// I-5: no event helper takes a free-form message, path or detail field,
    /// and the recorder never writes a file (retention is in memory).
    #[test]
    fn fixed_event_shape_contains_no_free_form_message_fields() {
        let production = production();
        for forbidden in [
            "fields.insert(\"message\"",
            "fields.insert(\"path\"",
            "fields.insert(\"detail\"",
            "fields.insert(\"name\"",
            "std::fs::",
            "File::",
        ] {
            assert!(!production.contains(forbidden), "{forbidden}");
        }
        assert!(production.contains("\"verboseDebugLogIncluded\": false"));
    }

    #[test]
    fn a_panic_is_recorded_as_location_thread_and_kind_without_its_payload() {
        // The hook only TRIES the recorder lock (a panic under the lock must
        // not deadlock), so a concurrent test holding it can drop one probe;
        // repeat until one lands.
        let recorded = || {
            FLIGHT
                .lock()
                .unwrap()
                .lines
                .iter()
                .find(|line| line.contains("flight-panic-probe"))
                .cloned()
        };
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| record_panic(info)));
        for _ in 0..200 {
            let caught = std::thread::Builder::new()
                .name("flight-panic-probe".into())
                .spawn(|| {
                    std::panic::catch_unwind(|| {
                        std::panic::panic_any(String::from("PRIVATE_PAYLOAD_42"))
                    })
                })
                .unwrap()
                .join()
                .unwrap();
            assert!(caught.is_err());
            if recorded().is_some() {
                break;
            }
        }
        std::panic::set_hook(previous);
        let line = recorded().expect("panic event");
        let ring = FLIGHT.lock().unwrap();
        assert!(line.contains("\"messageKind\":\"string\""), "{line}");
        assert!(line.contains("flight.rs:"), "{line}");
        assert!(!ring
            .lines
            .iter()
            .any(|line| line.contains("PRIVATE_PAYLOAD_42")));
    }

    /// One launch of the persisted recorder, run as a child process so each
    /// launch gets fresh process globals. Inert unless a probe parent sets
    /// `TINE_FLIGHT_PROBE_DIR`.
    #[test]
    fn persisted_run_child_probe() {
        let Some(dir) = std::env::var_os("TINE_FLIGHT_PROBE_DIR") else {
            return;
        };
        flight_init();
        record_watcher_batch(3, false, None); // recorded before the store exists
        persist_init(PathBuf::from(dir));
        match std::env::var("TINE_FLIGHT_PROBE_MODE").unwrap().as_str() {
            "report" => {
                let report =
                    build_diagnostic_report(None, Vec::new(), String::new(), String::new());
                let compact: Value = serde_json::from_str(&report.text).unwrap();
                println!("PROBE-REPORT {compact}");
                mark_clean_shutdown();
            }
            "killed" => std::process::abort(),
            "edits" => {
                // Ordinary saves of a 1-block and a 60-block page: fast, ok.
                for _ in 0..500 {
                    record_save(None, 1, std::time::Duration::from_millis(4));
                    record_save(None, 60, std::time::Duration::from_millis(40));
                }
                println!("PROBE-DIRTY {}", DIRTY.load(Ordering::Acquire));
                mark_clean_shutdown();
            }
            "panic" => {
                crate::debug::install_panic_logger();
                std::panic::panic_any(String::from("PRIVATE_PROBE_PAYLOAD"));
            }
            "flood" => {
                for pages in 0..40_000 {
                    record_watcher_batch(pages, pages % 2 == 0, None);
                }
                flush_now();
                std::process::abort();
            }
            other => panic!("unknown probe mode {other}"),
        }
    }

    fn launch(dir: &std::path::Path, mode: &str) -> Option<Value> {
        launch_output(dir, mode)
            .lines()
            .find_map(|line| line.split_once("PROBE-REPORT ").map(|(_, json)| json))
            .map(|json| serde_json::from_str(json).unwrap())
    }

    fn launch_output(dir: &std::path::Path, mode: &str) -> String {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "flight::tests::persisted_run_child_probe",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TINE_FLIGHT_PROBE_DIR", dir)
            .env("TINE_FLIGHT_PROBE_MODE", mode)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// I-25 unit cost: an edit writes nothing to diagnostics. Fast successful
    /// saves record no event, so the recorder never becomes dirty and the
    /// flusher never rewrites the history because of typing.
    #[test]
    fn ordinary_saves_cost_no_diagnostic_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        let output = launch_output(&dir, "edits");
        assert!(output.contains("PROBE-DIRTY false"), "{output}");
        let history = std::fs::read_to_string(dir.join(crate::flight_store::HISTORY_FILE)).unwrap();
        assert!(!history.contains("direct.save"), "{history}");
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        let mut files = files;
        files.sort();
        // A clean exit leaves the lock and the history; no temp, no marker.
        assert_eq!(files, ["history.jsonl", "process.lock"]);
        println!(
            "UNIT-COST history after one clean launch: {} bytes",
            history.len()
        );
    }

    fn events<'a>(report: &'a Value, session: &str) -> Vec<&'a str> {
        report["sessions"][session]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn an_unclean_exit_is_reported_on_the_next_launch_and_a_clean_one_is_not() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        let first = launch(&dir, "report").expect("first launch report");
        assert_eq!(first["runtime"]["previousExitUnclean"], false);
        assert_eq!(first["runtime"]["retainedAcrossRuns"], true);

        launch(&dir, "killed");
        let after_kill = launch(&dir, "report").expect("report after a kill");
        assert_eq!(after_kill["runtime"]["previousExitUnclean"], true);
        assert_eq!(
            events(&after_kill, "previous"),
            ["runtime.started", "watcher.batch"]
        );
        assert!(events(&after_kill, "current").contains(&"runtime.previous_exit_unclean"));

        // That launch ended through mark_clean_shutdown, the RunEvent::Exit path.
        let after_clean = launch(&dir, "report").expect("report after a clean exit");
        assert_eq!(after_clean["runtime"]["previousExitUnclean"], false);
        assert_eq!(
            events(&after_clean, "previous").last(),
            Some(&"runtime.clean_shutdown")
        );
    }

    #[test]
    fn a_panic_reaches_the_next_launch_without_its_payload() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        launch(&dir, "panic");
        let report = launch(&dir, "report").expect("report after a panic");
        assert_eq!(report["runtime"]["previousExitUnclean"], true);
        assert!(events(&report, "previous").contains(&"runtime.panic"));
        let history = std::fs::read_to_string(dir.join(crate::flight_store::HISTORY_FILE));
        assert!(!report.to_string().contains("PRIVATE_PROBE_PAYLOAD"));
        assert!(!history.unwrap().contains("PRIVATE_PROBE_PAYLOAD"));
    }

    #[test]
    fn the_persisted_history_stays_within_its_bound() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        launch(&dir, "flood");
        let history = dir.join(crate::flight_store::HISTORY_FILE);
        let size = std::fs::metadata(&history).unwrap().len();
        assert!(size <= FLIGHT_MAX_BYTES as u64, "{size}");
        assert!(
            size > FLIGHT_MAX_BYTES as u64 - 200,
            "the flood fills it: {size}"
        );
        let report = launch(&dir, "report").expect("report after a flood");
        let previous = events(&report, "previous");
        assert!(previous.len() > 10_000 && !previous.contains(&"runtime.started"));
    }

    #[test]
    fn a_truncated_history_is_discarded_and_rebuilt_at_launch() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        std::fs::create_dir_all(&dir).unwrap();
        let history = dir.join(crate::flight_store::HISTORY_FILE);
        std::fs::write(&history, "{\"schemaVersion\":1,\"elapsedMs\":0,\"ev").unwrap();
        let report = launch(&dir, "report").expect("launch over a torn history");
        assert!(events(&report, "previous").is_empty());
        let rebuilt = std::fs::read_to_string(&history).unwrap();
        assert!(rebuilt.ends_with('\n') && !rebuilt.contains("\"ev\n"));
        for line in rebuilt.lines() {
            let event: Value = serde_json::from_str(line).unwrap();
            assert!(event["event"].is_string());
        }
    }

    /// og ADR 0058 states the persisted bounds; the code must agree.
    #[test]
    fn the_adr_states_the_persisted_bounds_the_code_enforces() {
        let adr = include_str!("../../docs/adr/0058-privacy-safe-diagnostic-flight-recorder.md");
        assert_eq!(FLIGHT_MAX_BYTES, 1024 * 1024);
        assert_eq!(
            crate::flight_store::HISTORY_READ_CAP,
            FLIGHT_MAX_BYTES as u64
        );
        for fact in [
            "at most 1 MiB (1,048,576 bytes)".to_owned(),
            format!("every {} s", FLUSH_INTERVAL.as_secs()),
            format!("`{}`", crate::flight_store::HISTORY_FILE),
            format!("`{}`", crate::flight_store::MARKER_FILE),
            format!("`{}`", crate::flight_store::LOCK_FILE),
            "`<app data>/diagnostics/`".to_owned(),
        ] {
            assert!(adr.contains(&fact), "og ADR 0058 must state {fact}");
        }
    }
}
