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
//! [`FLIGHT_MAX_BYTES`] of encoded events; the oldest events are evicted first.
//! It is lost when the process exits: og does not yet persist a previous run
//! (a persisted recorder is a new persisted format, OG-RULES Rule 8), so a
//! report never says whether the previous exit was clean.
//!
//! **Cost.** Recording is O(event size) plus O(evicted events); a report is
//! O(retained bytes) ≤ 1 MiB; clearing is O(1). A poisoned recorder lock loses
//! that event and never panics a caller. Callers need not know whether the
//! recorder is empty, full or cleared.

use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::state::AppState;

const FLIGHT_SCHEMA_VERSION: u8 = 1;
/// Upper bound on retained encoded event bytes (one master segment).
pub(crate) const FLIGHT_MAX_BYTES: usize = 1024 * 1024;
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
        self.bytes = self.bytes.saturating_add(line.len());
        self.lines.push_back(line);
        while self.bytes > max_bytes && self.lines.len() > 1 {
            if let Some(old) = self.lines.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.len());
            }
        }
    }
}

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
    if let Ok(mut ring) = FLIGHT.try_lock() {
        ring.push(line, FLIGHT_MAX_BYTES);
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

/// Record one externally caused graph publication that reached a window
/// (`watcher.batch`): how many page events it carried and whether the sync
/// conflict list changed. No page names or paths.
pub(crate) fn record_watcher_batch(pages: usize, conflicts_changed: bool) {
    let mut fields = Map::new();
    fields.insert("pages".into(), json!(pages));
    fields.insert("conflictsChanged".into(), json!(conflicts_changed));
    record_fixed_event("watcher.batch", fields);
}

/// Commands whose own timing would only describe the recorder.
const SELF_COMMANDS: [&str; 6] = [
    "diagnostic_ipc_event",
    "diagnostic_frontend_event",
    "diagnostic_report",
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
/// `failed`/`still-saving` and a page count — GH #540). Any other kind, or a
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
    build_commit: String,
    build_time: String,
) -> DiagnosticReport {
    let events: Vec<Value> = FLIGHT
        .lock()
        .map(|ring| {
            ring.lines
                .iter()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default();
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
            "containsPaths": false,
            "containsPageTitles": false,
            "containsQueriesOrUrls": false,
            "containsCredentials": false,
            "verboseDebugLogIncluded": false,
        },
        "runtime": {
            "recorderActive": true,
            "retainedAcrossRuns": false,
            "verboseDebugEnabled": crate::debug::debug_enabled(),
            "graphStateUnavailable": graph_bindings.is_none(),
            "graphBindings": graph_bindings.unwrap_or(0),
        },
        "sessions": { "current": events },
    });
    DiagnosticReport {
        text: serde_json::to_string_pretty(&report).unwrap_or_else(|_| {
            "{\"schemaVersion\":1,\"error\":\"report_serialization_failed\"}".into()
        }),
        suggested_file_name: format!("tine-diagnostics-{generated_at}.json"),
    }
}

/// Build the report: app version, build commit/time (dropped unless they are a
/// hex commit and an ISO timestamp), OS/arch, privacy flags, the number of open
/// graph bindings, and every retained event of this run. O(retained bytes).
/// Never fails; an unreadable graph registry is reported as a flag.
#[tauri::command]
pub(crate) fn diagnostic_report(
    state: tauri::State<'_, AppState>,
    build_commit: String,
    build_time: String,
) -> DiagnosticReport {
    let graph_bindings = state.graphs.read().ok().map(|graphs| graphs.len());
    build_diagnostic_report(graph_bindings, build_commit, build_time)
}

/// Drop every retained event, then record `diagnostics.cleared`. O(1).
#[tauri::command]
pub(crate) fn clear_diagnostics() {
    if let Ok(mut ring) = FLIGHT.lock() {
        *ring = FlightRing::new();
    }
    record_fixed_event("diagnostics.cleared", Map::new());
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
        let report = build_diagnostic_report(Some(1), "abcdef1".into(), "/home/x".into());
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
}
