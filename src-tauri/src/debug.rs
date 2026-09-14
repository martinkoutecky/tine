use fs2::FileExt as _;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::state::{AppState, ApplicationPageAdmissionAuthority};

// `diag` is the existing opt-in detailed trace (`TINE_DEBUG=1` / `--debug`).
// It may contain a path or an OS error chosen for a directed investigation, so
// it is NEVER copied into the automatic report. The flight recorder is always
// on and accepts only fixed enums, catalogued command names, and numeric/boolean
// measurements. This separation is the privacy boundary behind GH #343.
const FLIGHT_SCHEMA_VERSION: u8 = 1;
const FLIGHT_SEGMENT_MAX_BYTES: u64 = 1024 * 1024;
const EARLY_EVENT_CAP: usize = 64;

static DEBUG_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();
static DEBUG_START: OnceLock<std::time::Instant> = OnceLock::new();
static FLIGHT: OnceLock<Option<Mutex<FlightRecorder>>> = OnceLock::new();
static EARLY_EVENTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static PREVIOUS_EXIT_UNCLEAN: AtomicBool = AtomicBool::new(false);

fn early_events() -> &'static Mutex<Vec<String>> {
    EARLY_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn elapsed_ms() -> u64 {
    let elapsed = DEBUG_START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    u64::try_from(elapsed).unwrap_or(u64::MAX)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Parse the process-level debug opt-in. The ONLY production reader of
/// `TINE_DEBUG` / `--debug`: `debug_init` calls it once, at startup, and pushes
/// the answer into `tine-core`, which owns the flag because core cannot call
/// src-tauri (I-12). Nothing else may re-derive it from the environment.
fn debug_opt_in_requested() -> bool {
    matches!(std::env::var("TINE_DEBUG"), Ok(v) if !v.is_empty() && v != "0")
        || std::env::args().any(|a| a == "--debug")
}

/// Thin delegate to the one producer, `tine-core`'s
/// `runtime_debug_diagnostics_enabled()`. Kept because 15 src-tauri callers
/// read better against a local name; it computes nothing of its own.
pub(crate) fn debug_enabled() -> bool {
    tine_core::sync_runtime::runtime_debug_diagnostics_enabled()
}

fn debug_log_path() -> PathBuf {
    std::env::var_os("TINE_DEBUG_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("tine-debug.log"))
}

/// Initialize the directed, detailed trace before Tauri exists. It remains
/// opt-in and retains its historical path/format contract.
pub(crate) fn debug_init() {
    // The one place the opt-in is parsed, and the one place the flag is set.
    // Everything downstream — in this crate and in `tine-core` — reads it back
    // through `runtime_debug_diagnostics_enabled()`.
    tine_core::sync_runtime::set_runtime_debug_diagnostics(debug_opt_in_requested());
    DEBUG_START.get_or_init(std::time::Instant::now);
    DEBUG_LOG.get_or_init(|| {
        if !debug_enabled() {
            return None;
        }
        let path = debug_log_path();
        match private_write_options(true, false).open(&path) {
            Ok(f) => {
                eprintln!("[tine] DEBUG logging to {}", path.display());
                Some(Mutex::new(f))
            }
            Err(e) => {
                eprintln!("[tine] could not open debug log {}: {e}", path.display());
                None
            }
        }
    });
}

/// Emit one detailed line to stderr and, when explicitly enabled, the directed
/// debug file. This function does not write to the privacy-safe flight recorder.
pub(crate) fn diag(msg: impl std::fmt::Display) {
    if !debug_enabled() {
        return;
    }
    let msg = msg.to_string();
    eprintln!("[tine] {msg}");
    if let Some(Some(lock)) = DEBUG_LOG.get() {
        if let Ok(mut file) = lock.lock() {
            let _ = writeln!(file, "[+{:>7}ms] {msg}", elapsed_ms());
            let _ = file.flush();
        }
    }
}

pub(crate) fn debug_header() {
    if !debug_enabled() {
        return;
    }
    diag(format!(
        "Tine {} starting — {}/{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let env_of = |key: &str| std::env::var(key).unwrap_or_else(|_| "<unset>".into());
    for key in [
        "TINE_GRAPH",
        "TINE_GPU",
        "WEBVIEW2_USER_DATA_FOLDER",
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
        "XDG_SESSION_TYPE",
        "WAYLAND_DISPLAY",
        "APPIMAGE",
        "LD_PRELOAD",
        "GDK_BACKEND",
    ] {
        diag(format!("env {key}={}", env_of(key)));
    }
}

pub(crate) fn install_panic_logger() {
    if debug_enabled() && std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let current_thread = std::thread::current();
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
            current_thread.name().unwrap_or("unnamed").into(),
        );
        let message_kind = if info.payload().is::<&str>() {
            "str"
        } else if info.payload().is::<String>() {
            "string"
        } else {
            "non_string"
        };
        fields.insert("message_kind".into(), message_kind.into());
        record_fixed_event("runtime.panic", fields);
        if debug_enabled() {
            diag(format!("PANIC: {info}"));
            diag(format!(
                "backtrace:\n{}",
                std::backtrace::Backtrace::force_capture()
            ));
        }
        default(info);
    }));
}

// ---------------------------------------------------------------------------
// Always-on privacy-safe flight recorder
// ---------------------------------------------------------------------------

fn private_write_options(truncate: bool, append: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .create(true)
        .write(true)
        .truncate(truncate)
        .append(append);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    options
}

fn private_read(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(0x0020_0000);
    }
    options.open(path)
}

fn prepare_private_dir(path: &Path) -> std::io::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "diagnostics location is not a private directory",
            ));
        }
    } else {
        std::fs::create_dir_all(path)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

struct FlightRecorder {
    dir: PathBuf,
    current: Option<File>,
    current_bytes: u64,
    segment_max_bytes: u64,
    process_lock: File,
}

/// Release the single-writer lock explicitly, rather than as a side effect of
/// dropping the descriptor.
///
/// `flock` belongs to the OPEN FILE DESCRIPTION, not to the descriptor, so
/// closing one copy releases nothing while any other copy of that description
/// is still open. This process forks — every `Command::spawn` duplicates every
/// open description into the child, and the copy lives until the child reaches
/// `exec` and `O_CLOEXEC` closes it. A recorder dropped inside that window
/// leaves its lock held by the child's copy, and the next `open` on the same
/// directory gets `EAGAIN` from `try_lock_exclusive`. That is a race against
/// nothing but scheduler luck, which is why it only ever showed up under
/// parallel test load — and why it cost this project four extra `cargo test -p
/// tine` runs in one lane. `unlock` acts on the description itself, so it
/// releases the lock for every copy at once and orders the release before the
/// drop returns.
impl Drop for FlightRecorder {
    fn drop(&mut self) {
        let _ = self.process_lock.unlock();
    }
}

impl FlightRecorder {
    fn open(dir: PathBuf, segment_max_bytes: u64) -> std::io::Result<(Self, bool)> {
        prepare_private_dir(&dir)?;
        let process_lock = private_write_options(false, false).open(dir.join("process.lock"))?;
        process_lock.try_lock_exclusive()?;

        let marker = dir.join("session-active");
        let previous_unclean = marker.exists();
        let _marker_file = private_write_options(true, false).open(&marker)?;

        let previous_old = dir.join("previous-old.jsonl");
        let previous = dir.join("previous.jsonl");
        let current_old = dir.join("current-old.jsonl");
        let current = dir.join("current.jsonl");
        let _ = std::fs::remove_file(&previous_old);
        if current_old.exists() {
            std::fs::rename(&current_old, &previous_old)?;
        }
        let _ = std::fs::remove_file(&previous);
        if current.exists() {
            std::fs::rename(&current, &previous)?;
        }
        let file = private_write_options(true, false).open(&current)?;
        Ok((
            Self {
                dir,
                current: Some(file),
                current_bytes: 0,
                segment_max_bytes,
                process_lock,
            },
            previous_unclean,
        ))
    }

    fn append(&mut self, line: &str) -> std::io::Result<()> {
        let line_bytes = u64::try_from(line.len().saturating_add(1)).unwrap_or(u64::MAX);
        if self.current_bytes > 0
            && self.current_bytes.saturating_add(line_bytes) > self.segment_max_bytes
        {
            if let Some(mut file) = self.current.take() {
                file.flush()?;
            }
            let old = self.dir.join("current-old.jsonl");
            let current = self.dir.join("current.jsonl");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&current, &old)?;
            self.current = Some(private_write_options(true, false).open(current)?);
            self.current_bytes = 0;
        }
        let Some(file) = self.current.as_mut() else {
            return Err(std::io::Error::other("flight recorder file is unavailable"));
        };
        writeln!(file, "{line}")?;
        file.flush()?;
        self.current_bytes = self.current_bytes.saturating_add(line_bytes);
        Ok(())
    }

    fn clear(&mut self) -> std::io::Result<()> {
        if let Some(mut file) = self.current.take() {
            file.flush()?;
        }
        for name in [
            "current.jsonl",
            "current-old.jsonl",
            "previous.jsonl",
            "previous-old.jsonl",
        ] {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
        self.current =
            Some(private_write_options(true, false).open(self.dir.join("current.jsonl"))?);
        self.current_bytes = 0;
        Ok(())
    }

    /// The `session-active` marker is the whole unclean-exit oracle: it is
    /// written when a session starts and removed when the session reaches an
    /// end we consider orderly, so the next launch reads its presence as "the
    /// last run stopped without warning". Clearing it therefore means "from
    /// here on, dying is expected"; re-arming means "we are live again, and a
    /// death from here IS worth reporting".
    fn set_session_active(&mut self, active: bool) {
        if let Some(file) = self.current.as_mut() {
            let _ = file.flush();
        }
        let marker = self.dir.join("session-active");
        if active {
            let _ = private_write_options(true, false).open(&marker);
        } else {
            let _ = std::fs::remove_file(&marker);
        }
    }

    fn mark_clean_shutdown(&mut self) {
        self.set_session_active(false);
    }
}

pub(crate) fn flight_init(dir: PathBuf) {
    let recorder = FLIGHT.get_or_init(|| {
        match FlightRecorder::open(dir, FLIGHT_SEGMENT_MAX_BYTES) {
            Ok((mut recorder, previous_unclean)) => {
                PREVIOUS_EXIT_UNCLEAN.store(previous_unclean, Ordering::Release);
                if let Ok(mut early) = early_events().lock() {
                    for line in early.drain(..) {
                        let _ = recorder.append(&line);
                    }
                }
                Some(Mutex::new(recorder))
            }
            Err(error) => {
                // A forwarded second launch legitimately loses the process
                // lock and must not rotate the primary process's evidence.
                eprintln!("[tine] flight recorder unavailable: {error}");
                None
            }
        }
    });
    if recorder.is_some() {
        record_fixed_event("runtime.started", Map::new());
        if PREVIOUS_EXIT_UNCLEAN.load(Ordering::Acquire) {
            record_fixed_event("runtime.previous_exit_unclean", Map::new());
        }
    }
}

pub(crate) fn mark_clean_shutdown() {
    record_fixed_event("runtime.clean_shutdown", Map::new());
    if let Some(Some(recorder)) = FLIGHT.get() {
        if let Ok(mut recorder) = recorder.lock() {
            recorder.mark_clean_shutdown();
        }
    }
}

/// A mobile OS reclaiming a BACKGROUNDED app is routine housekeeping, not a
/// crash. Before this, iOS and Android had no orderly end at all — there is no
/// `RunEvent::Exit` when the system reaps a suspended app — so every launch
/// greeted the user with "Tine did not close cleanly last time". GH #426:
/// "It does happen every time in iOS/iPadOS. The OS kills the app when in
/// background so I am having this issue all the time."
///
/// So on mobile the session ends when the app is hidden and restarts when the
/// user returns: a crash they actually witness is still reported, a reap they
/// never see is not.
///
/// Desktop deliberately ignores this. There an occluded or minimised window is
/// still a running session, `RunEvent::Exit` already provides the orderly end
/// (see `run()` in lib.rs), and honouring visibility would hide exactly the
/// background crashes the recorder exists to catch.
/// `visibility_ends_a_session_only_where_the_os_reaps_backgrounded_apps` holds
/// that half; the frontend caller is gated too (`src/sessionActivity.ts`),
/// which is where both platform branches are testable at runtime.
#[tauri::command]
pub(crate) fn diagnostic_session_active(active: bool) {
    if !cfg!(mobile) {
        return;
    }
    set_session_active(active);
}

fn set_session_active(active: bool) {
    let mut fields = Map::new();
    fields.insert("active".into(), json!(active));
    record_fixed_event("runtime.session_active", fields);
    if let Some(Some(recorder)) = FLIGHT.get() {
        if let Ok(mut recorder) = recorder.lock() {
            recorder.set_session_active(active);
        }
    }
}

fn record_fixed_event(event: &'static str, fields: Map<String, Value>) {
    let mut line = Map::new();
    line.insert("schemaVersion".into(), json!(FLIGHT_SCHEMA_VERSION));
    line.insert("elapsedMs".into(), json!(elapsed_ms()));
    line.insert("event".into(), json!(event));
    for (key, value) in fields {
        line.insert(key, value);
    }
    let Ok(encoded) = serde_json::to_string(&line) else {
        return;
    };
    match FLIGHT.get() {
        Some(Some(recorder)) => {
            if let Ok(mut recorder) = recorder.lock() {
                let _ = recorder.append(&encoded);
            }
        }
        Some(None) => {}
        None => {
            if let Ok(mut early) = early_events().lock() {
                if early.len() >= EARLY_EVENT_CAP {
                    early.remove(0);
                }
                early.push(encoded);
            }
        }
    }
}

fn enum_token<T: Serialize>(value: T) -> Value {
    match serde_json::to_value(value) {
        Ok(Value::String(token)) => Value::String(token),
        _ => Value::String("unknown".into()),
    }
}

pub(crate) fn record_storage_transition(
    event: &crate::storage_mode_supervisor::StorageTransitionEvent,
) {
    let mut fields = Map::new();
    fields.insert("operationId".into(), json!(event.operation_id));
    fields.insert(
        "windowKind".into(),
        json!(if event.window == "main" {
            "main"
        } else if event.window.starts_with("graph-") {
            "graph"
        } else {
            "other"
        }),
    );
    fields.insert("kind".into(), enum_token(event.kind));
    fields.insert("phase".into(), enum_token(event.phase));
    fields.insert("elapsedMs".into(), json!(event.elapsed_ms));
    fields.insert("terminal".into(), json!(event.terminal));
    if let Some(outcome) = event.outcome {
        fields.insert("outcome".into(), enum_token(outcome));
    }
    record_fixed_event("storage.transition", fields);
}

pub(crate) fn record_checkpoint_capture_skip(
    reason: tine_core::sync_runtime::SyncCheckpointCaptureSkip,
) {
    let mut fields = Map::new();
    fields.insert(
        "reason".into(),
        json!(match reason {
            tine_core::sync_runtime::SyncCheckpointCaptureSkip::RuntimeNotAttached => {
                "runtime_not_attached"
            }
            tine_core::sync_runtime::SyncCheckpointCaptureSkip::IndexedRuntime => "indexed_runtime",
            tine_core::sync_runtime::SyncCheckpointCaptureSkip::BlockedRuntime => "blocked_runtime",
            tine_core::sync_runtime::SyncCheckpointCaptureSkip::UnsettledRuntime => {
                "unsettled_runtime"
            }
            tine_core::sync_runtime::SyncCheckpointCaptureSkip::DurableFrontierAhead => {
                "durable_frontier_ahead"
            }
            tine_core::sync_runtime::SyncCheckpointCaptureSkip::CaptureFailed => "capture_failed",
        }),
    );
    record_fixed_event("managed.checkpoint_capture_skipped", fields);
}

/// Record only closed vocabulary, identifiers and measurements. In particular,
/// `retry_cause` is intentionally excluded because an OS error can contain a
/// private path or document-derived text.
pub(crate) fn record_history_recovery(
    recovery: &tine_core::sync_runtime::SyncHistoryRecoveryStatus,
) {
    let diagnostics = &recovery.diagnostics;
    let mut fields = Map::new();
    fields.insert("attempt".into(), json!(recovery.attempt));
    fields.insert("reason".into(), enum_token(recovery.reason));
    fields.insert("phase".into(), enum_token(recovery.phase));
    fields.insert(
        "triggeringBatchId".into(),
        json!(recovery.triggering_batch_id),
    );
    fields.insert(
        "triggeringDocumentId".into(),
        json!(recovery.triggering_document_id),
    );
    fields.insert("requestedFloor".into(), json!(recovery.requested_floor));
    fields.insert("actualFloor".into(), json!(recovery.actual_floor));
    fields.insert(
        "retryClass".into(),
        json!(recovery.retry_class.map(enum_token)),
    );
    fields.insert("journalFences".into(), json!(diagnostics.journal_fences));
    fields.insert("acceptedCount".into(), json!(diagnostics.accepted_count));
    fields.insert("pendingCount".into(), json!(diagnostics.pending_count));
    fields.insert("replayedCount".into(), json!(diagnostics.replayed_count));
    fields.insert("waitingMs".into(), json!(diagnostics.waiting_ms));
    fields.insert(
        "reconstructionMs".into(),
        json!(diagnostics.reconstruction_ms),
    );
    if let Some(checkpoint_bytes) = diagnostics.checkpoint_bytes {
        fields.insert("checkpointBytes".into(), json!(checkpoint_bytes));
    }
    fields.insert(
        "publicationEdge".into(),
        enum_token(diagnostics.publication_edge),
    );
    fields.insert(
        "preservationCheck".into(),
        enum_token(diagnostics.preservation_check),
    );
    record_fixed_event("managed.history_recovery", fields);
}

pub(crate) fn record_checkpoint_publication(
    checkpoint: &tine_core::sync_runtime::SyncCheckpointPublicationDiagnostics,
) {
    let mut fields = Map::new();
    fields.insert(
        "measurementSequence".into(),
        json!(checkpoint.measurement_sequence),
    );
    fields.insert(
        "latestAcceptanceUtcMs".into(),
        json!(checkpoint.latest_acceptance_utc_ms),
    );
    fields.insert("eligibleThrough".into(), json!(checkpoint.eligible_through));
    if let Some(age_cutoff_utc_ms) = checkpoint.age_cutoff_utc_ms {
        fields.insert("ageCutoffUtcMs".into(), json!(age_cutoff_utc_ms));
    }
    if let Some(clock_frozen) = checkpoint.clock_frozen {
        fields.insert("clockFrozen".into(), json!(clock_frozen));
    }
    if let Some(clock_reset_utc_ms) = checkpoint.last_clock_reset_utc_ms {
        fields.insert("lastClockResetUtcMs".into(), json!(clock_reset_utc_ms));
    }
    fields.insert("policyRevision".into(), json!(checkpoint.policy_revision));
    fields.insert(
        "minimumTailBytes".into(),
        json!(checkpoint.minimum_tail_bytes),
    );
    fields.insert(
        "liveSizeMultiplier".into(),
        json!(checkpoint.live_size_multiplier),
    );
    fields.insert("documents".into(), json!(checkpoint.documents));
    fields.insert(
        "changedDocuments".into(),
        json!(checkpoint.changed_documents),
    );
    fields.insert(
        "exportedDocuments".into(),
        json!(checkpoint.exported_documents),
    );
    fields.insert("reusedDocuments".into(), json!(checkpoint.reused_documents));
    fields.insert(
        "measurementExports".into(),
        json!(checkpoint.measurement_exports),
    );
    fields.insert(
        "candidateExports".into(),
        json!(checkpoint.candidate_exports),
    );
    fields.insert(
        "verificationImports".into(),
        json!(checkpoint.verification_imports),
    );
    fields.insert(
        "hotManifestReads".into(),
        json!(checkpoint.hot_manifest_reads),
    );
    fields.insert(
        "hotManifestBytes".into(),
        json!(checkpoint.hot_manifest_bytes),
    );
    fields.insert("hotObjectReads".into(), json!(checkpoint.hot_object_reads));
    fields.insert("hotObjectBytes".into(), json!(checkpoint.hot_object_bytes));
    fields.insert(
        "coldManifestReads".into(),
        json!(checkpoint.cold_manifest_reads),
    );
    fields.insert(
        "coldManifestBytes".into(),
        json!(checkpoint.cold_manifest_bytes),
    );
    fields.insert(
        "coldObjectReads".into(),
        json!(checkpoint.cold_object_reads),
    );
    fields.insert(
        "coldObjectBytes".into(),
        json!(checkpoint.cold_object_bytes),
    );
    fields.insert("imagePhaseMs".into(), json!(checkpoint.image_phase_ms));
    fields.insert("payloadPhaseMs".into(), json!(checkpoint.payload_phase_ms));
    fields.insert(
        "publicationPhaseMs".into(),
        json!(checkpoint.publication_phase_ms),
    );
    if let Some(peak_rss_bytes) = checkpoint.peak_rss_bytes {
        fields.insert("peakRssBytes".into(), json!(peak_rss_bytes));
    }
    fields.insert("checkpointBytes".into(), json!(checkpoint.checkpoint_bytes));
    fields.insert(
        "publicationEdge".into(),
        enum_token(checkpoint.publication_edge),
    );
    record_fixed_event("managed.checkpoint_publication", fields);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_direct_save(
    outcome: &'static str,
    total_ms: u64,
    complete_builds: u64,
    exact_updates: u64,
    invalidated: bool,
    captured_entries: Option<u64>,
    captured_bytes: Option<u64>,
) {
    let mut fields = Map::new();
    fields.insert("outcome".into(), json!(outcome));
    fields.insert("totalMs".into(), json!(total_ms));
    fields.insert("guardedIndexBuilds".into(), json!(complete_builds));
    fields.insert("guardedIndexExactUpdates".into(), json!(exact_updates));
    fields.insert("guardedIndexInvalidated".into(), json!(invalidated));
    if let Some(entries) = captured_entries {
        fields.insert("lastBuildEntries".into(), json!(entries));
    }
    if let Some(bytes) = captured_bytes {
        fields.insert("lastBuildBytes".into(), json!(bytes));
    }
    record_fixed_event("direct.save", fields);
}

pub(crate) fn record_managed_save(outcome: &'static str, total_ms: u64) {
    let mut fields = Map::new();
    fields.insert("outcome".into(), json!(outcome));
    fields.insert("totalMs".into(), json!(total_ms));
    record_fixed_event("managed.save", fields);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_watcher_latency(
    mode: &'static str,
    pages: u64,
    event_paths: u64,
    full_diff: bool,
    errors: u64,
    event_to_reconcile_ms: Option<u64>,
    reconcile_ms: u64,
    event_to_emit_ms: Option<u64>,
) {
    let mut fields = Map::new();
    fields.insert("mode".into(), json!(mode));
    fields.insert("pages".into(), json!(pages));
    fields.insert("eventPaths".into(), json!(event_paths));
    fields.insert("fullDiff".into(), json!(full_diff));
    fields.insert("errors".into(), json!(errors));
    fields.insert("eventToReconcileMs".into(), json!(event_to_reconcile_ms));
    fields.insert("reconcileMs".into(), json!(reconcile_ms));
    fields.insert("eventToEmitMs".into(), json!(event_to_emit_ms));
    record_fixed_event("watcher.batch", fields);
}

#[tauri::command]
pub(crate) fn diagnostic_ipc_event(command: String, phase: String, elapsed_ms: u64) {
    if !crate::managed_command_surface::is_known_command(&command)
        || matches!(
            command.as_str(),
            "diagnostic_ipc_event"
                | "diagnostic_frontend_event"
                | "diagnostic_session_active"
                | "diagnostic_report"
                | "save_diagnostic_report"
                | "clear_diagnostics"
                | "debug_info"
                | "debug_log"
        )
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

#[tauri::command]
pub(crate) fn diagnostic_frontend_event(
    kind: String,
    line: Option<u64>,
    column: Option<u64>,
    delay_ms: Option<u64>,
    updater_stage: Option<String>,
    updater_cause: Option<String>,
) {
    if kind == "updater_failure" {
        let Some(stage) = updater_stage.filter(|value| {
            matches!(
                value.as_str(),
                "manifest_fetch"
                    | "manifest_parse"
                    | "target_selection"
                    | "download"
                    | "signature_verification"
                    | "install"
                    | "relaunch"
            )
        }) else {
            return;
        };
        let Some(cause) = updater_cause.filter(|value| {
            matches!(
                value.as_str(),
                "network"
                    | "invalid_manifest"
                    | "unsupported_target"
                    | "invalid_signature"
                    | "install_failed"
                    | "relaunch_failed"
                    | "unknown"
            )
        }) else {
            return;
        };
        let mut fields = Map::new();
        fields.insert("stage".into(), json!(stage));
        fields.insert("cause".into(), json!(cause));
        record_fixed_event("updater.failure", fields);
        return;
    }
    if !matches!(
        kind.as_str(),
        "uncaught_error" | "unhandled_rejection" | "heartbeat_delay"
    ) {
        return;
    }
    let mut fields = Map::new();
    fields.insert("kind".into(), json!(kind));
    fields.insert("line".into(), json!(line));
    fields.insert("column".into(), json!(column));
    fields.insert("delayMs".into(), json!(delay_ms));
    record_fixed_event("frontend.health", fields);
}

#[tauri::command]
pub(crate) fn app_architecture() -> &'static str {
    std::env::consts::ARCH
}

#[tauri::command]
pub(crate) fn debug_log(line: String) {
    if debug_enabled() {
        diag(format!("[ui] {line}"));
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DebugInfo {
    enabled: bool,
    path: String,
    recorder_active: bool,
    previous_exit_unclean: bool,
}

#[tauri::command]
pub(crate) fn debug_info() -> DebugInfo {
    DebugInfo {
        enabled: debug_enabled(),
        path: debug_log_path().display().to_string(),
        recorder_active: matches!(FLIGHT.get(), Some(Some(_))),
        previous_exit_unclean: PREVIOUS_EXIT_UNCLEAN.load(Ordering::Acquire),
    }
}

fn read_event_segment(path: &Path) -> Vec<Value> {
    let Ok(file) = private_read(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(FLIGHT_SEGMENT_MAX_BYTES.saturating_add(4096))
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Vec::new();
    }
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
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

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticReport {
    pub(crate) text: String,
    pub(crate) suggested_file_name: String,
}

fn build_diagnostic_report(
    state: &AppState,
    build_commit: String,
    build_time: String,
) -> DiagnosticReport {
    let mut direct = 0u64;
    let mut managed_writable = 0u64;
    let mut managed_unavailable = 0u64;
    let mut graph_state_unavailable = false;
    match state.graphs.read() {
        Ok(graphs) => {
            for (_, slot) in graphs.entries() {
                match slot.application_page_admission().authority {
                    ApplicationPageAdmissionAuthority::Direct => direct += 1,
                    ApplicationPageAdmissionAuthority::ManagedWritable { .. } => {
                        managed_writable += 1
                    }
                    ApplicationPageAdmissionAuthority::ManagedUnavailable => {
                        managed_unavailable += 1
                    }
                }
            }
        }
        Err(_) => graph_state_unavailable = true,
    }

    let mut sessions = BTreeMap::new();
    if let Some(Some(recorder)) = FLIGHT.get() {
        if let Ok(mut recorder) = recorder.lock() {
            if let Some(file) = recorder.current.as_mut() {
                let _ = file.flush();
            }
            for (name, file) in [
                ("previousOld", "previous-old.jsonl"),
                ("previous", "previous.jsonl"),
                ("currentOld", "current-old.jsonl"),
                ("current", "current.jsonl"),
            ] {
                sessions.insert(name, read_event_segment(&recorder.dir.join(file)));
            }
        }
    }

    let generated_at = unix_ms();
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
            "recorderActive": matches!(FLIGHT.get(), Some(Some(_))),
            "previousExitUnclean": PREVIOUS_EXIT_UNCLEAN.load(Ordering::Acquire),
            "verboseDebugEnabled": debug_enabled(),
            "graphStateUnavailable": graph_state_unavailable,
            "graphBindings": direct + managed_writable + managed_unavailable,
            "directBindings": direct,
            "managedWritableBindings": managed_writable,
            "managedUnavailableBindings": managed_unavailable,
        },
        "activeStorageTransitions": state.storage_supervisor.diagnostic_snapshot(),
        "watcherLatency": crate::watcher::diagnostic_latency_snapshot(),
        "sessions": sessions,
    });
    DiagnosticReport {
        text: serde_json::to_string_pretty(&report).unwrap_or_else(|_| {
            "{\"schemaVersion\":1,\"error\":\"report_serialization_failed\"}".into()
        }),
        suggested_file_name: format!("tine-diagnostics-{generated_at}.json"),
    }
}

#[tauri::command]
pub(crate) fn diagnostic_report(
    state: State<'_, AppState>,
    build_commit: String,
    build_time: String,
) -> DiagnosticReport {
    build_diagnostic_report(&state, build_commit, build_time)
}

#[tauri::command]
pub(crate) async fn save_diagnostic_report(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    build_commit: String,
    build_time: String,
) -> Result<bool, crate::command_error::CommandError> {
    let report = build_diagnostic_report(&state, build_commit, build_time);
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
        .map_err(|error| {
            crate::command_error::CommandError::diagnostic(format!(
                "diagnostic save dialog failed: {error}"
            ))
        })?;
        let Some(chosen) = chosen else {
            return Ok(false);
        };
        let path = chosen.into_path().map_err(|_| {
            crate::command_error::CommandError::prose(
                "diagnostic save destination is not a local file",
            )
        })?;
        tine_core::model::atomic_write(&path, report.text.as_bytes()).map_err(|error| {
            crate::command_error::CommandError::diagnostic(format!(
                "diagnostic report could not be saved: {error}"
            ))
        })?;
        Ok(true)
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, report);
        Err(crate::command_error::CommandError::prose(
            "Save report is available on desktop; use Copy report on this device.",
        ))
    }
}

#[tauri::command]
pub(crate) fn clear_diagnostics() -> Result<(), crate::command_error::CommandError> {
    let Some(Some(recorder)) = FLIGHT.get() else {
        return Ok(());
    };
    recorder
        .lock()
        .map_err(|_| {
            crate::command_error::CommandError::prose("diagnostic recorder is unavailable")
        })?
        .clear()
        .map_err(|error| {
            crate::command_error::CommandError::diagnostic(format!(
                "diagnostic events could not be cleared: {error}"
            ))
        })?;
    record_fixed_event("diagnostics.cleared", Map::new());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "child-process probe for stderr capture"]
    fn diag_disabled_child_probe() {
        diag("diag-disabled-probe");
    }

    #[test]
    fn diag_is_silent_when_debug_is_disabled() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "debug::tests::diag_disabled_child_probe",
                "--nocapture",
            ])
            .env_remove("TINE_DEBUG")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stderr-capture child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "diag leaked to stderr with debugging disabled: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "child-process probe for panic event capture"]
    fn panic_event_child_probe() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        flight_init(dir.clone());
        install_panic_logger();
        let caught = std::thread::Builder::new()
            .name("panic-event-probe".into())
            .spawn(|| std::panic::catch_unwind(|| std::panic::panic_any(7_u8)))
            .unwrap()
            .join()
            .unwrap();
        assert!(caught.is_err());
        let events = std::fs::read_to_string(dir.join("current.jsonl")).unwrap();
        let event = events
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|event| event["event"] == "runtime.panic")
            .expect("panic event");
        assert_eq!(event["thread"], "panic-event-probe");
        assert_eq!(event["message_kind"], "non_string");
        assert!(event["location"].as_str().unwrap().contains("debug.rs:"));
        assert!(!events.contains("7_u8"));
    }

    #[test]
    fn caught_named_thread_panic_records_fixed_shape_location_thread_and_kind() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "debug::tests::panic_event_child_probe",
                "--nocapture",
            ])
            .env_remove("TINE_DEBUG")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "panic-event child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn recorder_retains_previous_unclean_run_and_bounds_each_segment() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        {
            let (mut recorder, previous_unclean) = FlightRecorder::open(dir.clone(), 120).unwrap();
            assert!(!previous_unclean);
            recorder.append(&"a".repeat(80)).unwrap();
            recorder.append(&"b".repeat(80)).unwrap();
            assert!(dir.join("current-old.jsonl").is_file());
            // Deliberately omit mark_clean_shutdown: simulate a killed process.
        }
        let (mut recorder, previous_unclean) = FlightRecorder::open(dir.clone(), 120).unwrap();
        assert!(previous_unclean);
        assert!(dir.join("previous.jsonl").is_file());
        assert!(dir.join("previous-old.jsonl").is_file());
        recorder.mark_clean_shutdown();
        assert!(!dir.join("session-active").exists());
    }

    /// GH #426, the mobile half: iOS/iPadOS reaps a backgrounded app as a
    /// matter of course. The user did nothing wrong and saw nothing happen, so
    /// the next launch must not accuse Tine of crashing.
    #[test]
    fn a_session_the_os_reaps_after_backgrounding_is_not_reported_unclean() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        {
            let (mut recorder, _) = FlightRecorder::open(dir.clone(), 4096).unwrap();
            recorder.set_session_active(false); // the app went to the background
                                                // and the process is never given
                                                // a chance to exit politely.
        }
        let (_, previous_unclean) = FlightRecorder::open(dir.clone(), 4096).unwrap();
        assert!(!previous_unclean);
    }

    /// The two tests above are the pair that kept failing on a first parallel
    /// run and passing on the rerun — four extra `cargo test -p tine`
    /// invocations in one lane alone. Both do the same thing: drop a recorder
    /// and immediately open another on the same directory. That only works if
    /// dropping actually released the single-writer lock, and dropping the
    /// descriptor is NOT that: `flock` belongs to the open file description, so
    /// it survives until the last copy of the description closes. This process
    /// spawns children, and every spawn duplicates the description into a child
    /// that holds it until `exec`.
    ///
    /// Reproduce that window deterministically with an explicit `dup`, which
    /// leaves the kernel in exactly the state a mid-`exec` child does.
    #[test]
    fn a_recorder_releases_its_lock_even_while_a_duplicated_descriptor_lives() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        let duplicate = {
            let (recorder, _) = FlightRecorder::open(dir.clone(), 4096).unwrap();
            recorder.process_lock.try_clone().unwrap()
        };
        let reopened = FlightRecorder::open(dir.clone(), 4096);
        drop(duplicate);
        assert!(
            reopened.is_ok(),
            "the recorder must unlock explicitly, not leave release to the last \
             descriptor close: {:?}",
            reopened.err(),
        );
    }

    /// The other direction, which the fix must not trade away: a session the
    /// user was looking at when it died is still an unclean exit.
    #[test]
    fn a_session_that_dies_after_coming_back_is_still_reported_unclean() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        {
            let (mut recorder, _) = FlightRecorder::open(dir.clone(), 4096).unwrap();
            recorder.set_session_active(false); // backgrounded …
            recorder.set_session_active(true); // … and then reopened by the user.
        }
        let (_, previous_unclean) = FlightRecorder::open(dir.clone(), 4096).unwrap();
        assert!(previous_unclean);
    }

    /// On desktop a minimised or occluded window is still a live session and a
    /// crash behind it is exactly what the recorder exists to catch, so the
    /// command is gated. The gate cannot be exercised at runtime here — the
    /// recorder is a process-global `OnceLock` that no test may install — so it
    /// is pinned the way this file already pins its other structural claims.
    #[test]
    fn visibility_ends_a_session_only_where_the_os_reaps_backgrounded_apps() {
        let source = include_str!("debug.rs");
        let body = source
            .split("pub(crate) fn diagnostic_session_active(active: bool) {")
            .nth(1)
            .expect("diagnostic_session_active is the visibility entry point");
        let gate = body
            .split("set_session_active(active)")
            .next()
            .expect("the gate precedes the effect");
        assert!(
            gate.contains("if !cfg!(mobile) {\n        return;\n    }"),
            "diagnostic_session_active must return early off mobile"
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

    #[test]
    fn fixed_event_shape_contains_no_free_form_message_fields() {
        let source = include_str!("debug.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production diagnostics precede their tests");
        let contract = include_str!("../../docs/contracts/diagnostics.md");
        assert!(!production.contains("fields.insert(\"message\""));
        assert!(!production.contains("fields.insert(\"path\""));
        assert!(!production.contains("fields.insert(\"detail\""));
        assert!(production.contains("verboseDebugLogIncluded\": false"));
        assert!(production.contains("managed.checkpoint_capture_skipped"));
        assert!(production.contains("managed.history_recovery"));
        assert!(production.contains("managed.checkpoint_publication"));
        assert!(production.contains("record_fixed_event(\"managed.save\", fields)"));
        for field in [
            "attempt",
            "reason",
            "phase",
            "triggeringBatchId",
            "triggeringDocumentId",
            "requestedFloor",
            "actualFloor",
            "journalFences",
            "acceptedCount",
            "pendingCount",
            "replayedCount",
            "preservationCheck",
            "policyRevision",
            "minimumTailBytes",
            "liveSizeMultiplier",
            "changedDocuments",
            "exportedDocuments",
            "reusedDocuments",
            "measurementExports",
            "candidateExports",
            "checkpointBytes",
            "publicationEdge",
        ] {
            assert!(
                production.contains(field),
                "missing fixed diagnostic field {field}"
            );
        }
        for reason in [
            "runtime_not_attached",
            "indexed_runtime",
            "blocked_runtime",
            "unsettled_runtime",
            "durable_frontier_ahead",
            "capture_failed",
        ] {
            assert!(production.contains(reason));
            assert!(
                contract.contains(reason),
                "the bounded checkpoint-skip cause and diagnostics contract must change together"
            );
        }
    }

    #[test]
    fn updater_diagnostics_accept_only_fixed_stage_and_cause_tokens() {
        let source = include_str!("debug.rs");
        for token in [
            "manifest_fetch",
            "manifest_parse",
            "target_selection",
            "download",
            "signature_verification",
            "install",
            "relaunch",
            "network",
            "invalid_manifest",
            "unsupported_target",
            "invalid_signature",
            "install_failed",
            "relaunch_failed",
        ] {
            assert!(source.contains(&format!("\"{token}\"")));
        }
        assert!(source.contains("record_fixed_event(\"updater.failure\", fields)"));
        assert!(!source.contains("fields.insert(\"updaterError\""));
    }

    #[test]
    fn user_selected_reports_share_the_atomic_publish_family() {
        let diagnostic = include_str!("debug.rs");
        let verification = include_str!("graph_verification.rs");
        let plain_write = ["std::fs::", "write(path"].concat();
        let atomic_write = ["tine_core::model::", "atomic_write(&path"].concat();

        for (label, source) in [
            ("diagnostic", diagnostic),
            ("graph verification", verification),
        ] {
            assert!(
                !source.contains(&plain_write),
                "{label} report export must not truncate its destination in place"
            );
            assert!(
                source.contains(&atomic_write),
                "{label} report export must use the crash-durable atomic family"
            );
        }
    }
}
