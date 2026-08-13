use crate::backup::backup_async;
use crate::settings::{
    approved_external_assets, remember_external_assets_approval, remember_graph,
};
use crate::state::{
    canonical_graph_root, poke_watcher, slot_for_window, AppState, ApplicationPageAdmission,
    GraphSlot,
};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager, State};
use tine_core::model::{Graph, GraphMeta};
use tine_core::sync_runtime::{
    inspect_shared_enrollment_for_cold_discovery, inspect_shared_provider_cold_prefix,
    ManagedStorageRefusalScenario, SyncSharedProviderColdPrefix,
};

/// Reset the warm flag for a new graph load and return the new warm generation
/// (passed to `warm_cache_async`, which only reports done if still current).
pub(crate) fn begin_warm_cache(slot: &GraphSlot) -> u64 {
    slot.warm_done.store(false, Ordering::Release);
    slot.warm_generation.fetch_add(1, Ordering::AcqRel) + 1
}

/// Resolve the graph root: explicit path, else env var, else first CLI arg.
pub(crate) fn resolve_root(path: &str) -> Option<String> {
    if !path.is_empty() {
        return Some(path.to_string());
    }
    for var in ["TINE_GRAPH"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    std::env::args().skip(1).find(|arg| !arg.starts_with('-'))
}

pub(crate) const STARTUP_PROGRESS_EVENT: &str = "startup-progress";
const STARTUP_PROGRESS_MAX_ELAPSED_MS: u64 = 86_400_000;

fn bounded_startup_elapsed_ms(elapsed: std::time::Duration) -> u64 {
    u64::try_from(elapsed.as_millis())
        .unwrap_or(u64::MAX)
        .min(STARTUP_PROGRESS_MAX_ELAPSED_MS)
}

/// A bounded, content-free startup receipt.  It is intentionally suitable for
/// both stderr diagnostics and the still-unbound startup webview: neither the
/// graph path nor an underlying I/O error belongs in this event.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct StartupProgressEvent {
    phase: &'static str,
    elapsed_ms: u64,
    terminal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'static str>,
}

#[derive(Clone)]
pub(crate) struct StartupProgressReporter {
    app: tauri::AppHandle,
    label: String,
    started: Instant,
    terminal_emitted: Arc<AtomicBool>,
}

impl StartupProgressReporter {
    pub(crate) fn for_window(app: &tauri::AppHandle, label: &str) -> Self {
        Self {
            app: app.clone(),
            label: label.to_string(),
            started: Instant::now(),
            terminal_emitted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn phase(&self, phase: &'static str) {
        self.emit(phase, false, None);
    }

    /// Emit exactly one terminal receipt even when the owning blocking worker
    /// is interrupted while Tauri is awaiting it.
    pub(crate) fn terminal(&self, phase: &'static str, outcome: &'static str) {
        if self
            .terminal_emitted
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        self.emit(phase, true, Some(outcome));
    }

    fn emit(&self, phase: &'static str, terminal: bool, outcome: Option<&'static str>) {
        let event = StartupProgressEvent {
            phase,
            elapsed_ms: bounded_startup_elapsed_ms(self.started.elapsed()),
            terminal,
            outcome,
        };
        crate::debug::diag(format!(
            "startup progress: phase={}; elapsed_ms={}; terminal={}; outcome={}",
            event.phase,
            event.elapsed_ms,
            event.terminal,
            event.outcome.unwrap_or("none"),
        ));
        let _ = self.app.emit_to(&self.label, STARTUP_PROGRESS_EVENT, event);
    }
}

/// The remembered-graph lookup retains its historical best-effort semantics:
/// unavailable, truncated, and malformed device settings simply yield no
/// remembered graph.  Unlike the old `last_graph_path` composition, every
/// operation is reported with a bounded code so a stuck startup never looks
/// silent.  The supplied path is never included in a diagnostic.
fn remembered_startup_graph_path_at(
    configured: Option<String>,
    settings_path: Option<&Path>,
    mut report: impl FnMut(&'static str, bool, Option<&'static str>),
) -> Option<String> {
    report("lookup.entry", false, None);
    report("lookup.app_data", false, None);

    report("lookup.settings_stat", false, None);
    let settings_exists = settings_path
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some();

    report("lookup.settings_read", false, None);
    let contents = settings_exists
        .then(|| settings_path.and_then(|path| std::fs::read_to_string(path).ok()))
        .flatten();

    report("lookup.settings_parse", false, None);
    let remembered = contents
        .as_deref()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(contents).ok())
        .and_then(|json| {
            json.get("last_graph_path")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        });

    let result = configured.or(remembered);
    report("lookup.complete", true, Some("ok"));
    result
}

fn startup_graph_path_blocking(
    app: &tauri::AppHandle,
    reporter: &StartupProgressReporter,
) -> Option<String> {
    let settings_path = crate::settings::settings_path(app);
    remembered_startup_graph_path_at(
        resolve_root(""),
        settings_path.as_deref(),
        |phase, terminal, outcome| {
            if terminal {
                reporter.terminal(
                    phase,
                    outcome.expect("terminal startup lookup has an outcome"),
                );
            } else {
                reporter.phase(phase);
            }
        },
    )
}

#[tauri::command]
pub(crate) async fn startup_graph_path(
    attempt: u64,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Option<String> {
    let label = window.label().to_string();
    let state = app.state::<AppState>();
    state.begin_startup_recovery_attempt(&label, attempt);
    let reporter = StartupProgressReporter::for_window(&app, &label);
    let worker_app = app.clone();
    let worker_reporter = reporter.clone();
    let worker_label = label.clone();
    match tauri::async_runtime::spawn_blocking(move || {
        let result = startup_graph_path_blocking(&worker_app, &worker_reporter);
        let canonical_target = result
            .as_deref()
            .and_then(|path| canonical_graph_root(path).ok());
        worker_app
            .state::<AppState>()
            .authorize_startup_recovery_target(&worker_label, attempt, canonical_target);
        result
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            reporter.terminal("lookup.complete", "error");
            None
        }
    }
}

#[tauri::command]
pub(crate) fn capture_target(state: State<'_, AppState>) -> Result<String, String> {
    capture_target_for_state(&state)
}

fn capture_target_for_state(state: &AppState) -> Result<String, String> {
    let preferred = state.last_focused.lock().unwrap().clone();
    if let Some(label) =
        preferred.filter(|label| state.graphs.read().unwrap().slot(label).is_some())
    {
        return Ok(label);
    }
    state
        .graphs
        .read()
        .unwrap()
        .entries()
        .into_iter()
        .next()
        .map(|entry| entry.0)
        .ok_or_else(|| "no graph window is open".to_string())
}

#[derive(serde::Serialize)]
pub(crate) struct CaptureGraphBindingResult {
    pub(crate) binding_generation: u64,
}

/// Snapshot the graph selected for a Quick Capture show. Calling this from the
/// native show path revokes the prior capture lease before a focused, persistent
/// capture WebView can issue a query against an older graph. The frontend calls
/// it again to learn the generation it must present with IPC.
pub(crate) fn refresh_capture_graph_binding(state: &AppState) -> Result<u64, String> {
    let target = capture_target_for_state(state)?;
    let slot = slot_for_window(state, &target)?;
    let binding_generation = slot.binding_generation;
    state.bind_capture_graph(target, binding_generation);
    Ok(binding_generation)
}

/// Return the binding selected by the native capture-show path. This is
/// intentionally separate from `GraphRegistry::bind`: the capture surface must
/// never become a second owner/writer for the graph root. Do not choose again
/// here: the frontend must receive the exact target/generation selected for
/// this show, so an old asynchronous activation cannot retarget itself.
#[tauri::command]
pub(crate) fn capture_graph_binding(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<CaptureGraphBindingResult, String> {
    if window.label() != "capture" {
        return Err("capture graph binding is only available to quick capture".into());
    }
    let binding_generation = state
        .capture_graph_binding()
        .ok_or("no graph bound for quick capture")?
        .binding_generation;
    Ok(CaptureGraphBindingResult { binding_generation })
}

struct LoadedGraph {
    graph: Graph,
    meta: GraphMeta,
    launch_backup_done: bool,
}

const PARTIAL_PROVIDER_REFUSAL: &str =
    "Tine-managed storage sync data appears to still be arriving or is incomplete. Tine left this graph unchanged. Let your file-sync provider finish, then Retry.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColdSparseArchive {
    Absent,
    Joinable,
    Partial,
    Refused(ManagedStorageRefusalScenario),
}

fn refuse_unclaimed_sparse_archive(root: &Path) -> Result<(), String> {
    refuse_unclaimed_sparse_archive_with(root, |shared| match inspect_shared_provider_cold_prefix(
        shared,
    )? {
        SyncSharedProviderColdPrefix::Partial => Ok(false),
        SyncSharedProviderColdPrefix::ReadyForDescriptorInspection => {
            inspect_shared_enrollment_for_cold_discovery(shared)
                .map(|descriptor| descriptor.is_some())
                // A descriptor file can be observed between the provider's
                // create and final write/rename.  Its malformed bytes are an
                // incomplete arrival, not proof of hostile graph state.
                .or(Ok(false))
        }
        SyncSharedProviderColdPrefix::Refused => Err(format!(
            "shared provider namespace has an unsafe filesystem kind [scenario_id={}]",
            ManagedStorageRefusalScenario::UnsafeFilesystemKind
        )),
    })
}

fn refuse_unclaimed_sparse_archive_with(
    root: &Path,
    inspect_shared: impl FnOnce(&Path) -> Result<bool, String>,
) -> Result<(), String> {
    match inspect_unclaimed_sparse_archive(root, inspect_shared)? {
        ColdSparseArchive::Absent | ColdSparseArchive::Joinable => Ok(()),
        ColdSparseArchive::Partial => {
            crate::debug::diag(
                "sparse-v2 cold discovery: phase=provider_evidence; outcome=partial_provider_refusal",
            );
            Err(PARTIAL_PROVIDER_REFUSAL.into())
        }
        ColdSparseArchive::Refused(scenario) => Err(format!(
            "Tine-managed storage data has an unsafe filesystem kind, so this graph could not be opened safely. [scenario_id={scenario}]"
        )),
    }
}

fn inspect_unclaimed_sparse_archive(
    root: &Path,
    inspect_shared: impl FnOnce(&Path) -> Result<bool, String>,
) -> Result<ColdSparseArchive, String> {
    let archive = root.join(".tine-sync/v2");
    let metadata = match std::fs::symlink_metadata(&archive) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ColdSparseArchive::Absent)
        }
        Err(error) => {
            return Err(format!(
                "Couldn't verify Tine-managed storage data before opening this graph: {error}"
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(ColdSparseArchive::Refused(
            ManagedStorageRefusalScenario::UnsafeFilesystemKind,
        ));
    }
    let shared = archive.join("shared");
    let shared_metadata = match std::fs::symlink_metadata(&shared) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ColdSparseArchive::Partial)
        }
        Err(error) => {
            return Err(format!(
                "Couldn't verify Tine-managed storage data before opening this graph: {error}"
            ))
        }
    };
    if shared_metadata.file_type().is_symlink() || !shared_metadata.is_dir() {
        return Ok(ColdSparseArchive::Refused(
            ManagedStorageRefusalScenario::UnsafeFilesystemKind,
        ));
    }
    // The canonical shared directory is the only path that carries discovery
    // authority.  Temporary or future sibling entries under v2 are inert.
    match inspect_shared(&shared) {
        Ok(true) => Ok(ColdSparseArchive::Joinable),
        Ok(false) => Ok(ColdSparseArchive::Partial),
        // Preserve a local filesystem/validation failure.  Collapsing this to
        // Refused (and then to a generic recovery message) previously made a
        // legitimate Android shared-storage incompatibility look exactly like
        // provider bytes that were still arriving.
        Err(error) => Err(format!(
            "Couldn't validate Tine-managed sync data on this device: {error}"
        )),
    }
}

fn open_graph_for_load(
    root: &str,
    approved_assets: Option<&Path>,
    take_launch_backup: impl FnOnce(&Graph) -> (usize, bool),
) -> Result<LoadedGraph, String> {
    let graph = Graph::open_checked_with_assets(root, approved_assets)
        .map_err(|e| format!("unsafe graph layout: {e}"))?;
    let meta = graph.meta();
    let needs_migration = graph.has_journal_filename_migrations();
    let (backup_n, backup_complete) = if needs_migration {
        take_launch_backup(&graph)
    } else {
        (0, false)
    };
    let launch_backup_done = backup_n > 0 && backup_complete;
    if needs_migration && launch_backup_done {
        // Recover any journals mis-saved under their title (see method docs),
        // but only after the launch snapshot has captured the original names.
        graph
            .migrate_journal_filenames_checked()
            .map_err(|error| format!("journal filename migration failed: {error}"))?;
    }
    Ok(LoadedGraph {
        graph,
        meta,
        launch_backup_done,
    })
}

#[derive(serde::Serialize)]
pub(crate) struct GraphAccessInspection {
    graph_root: String,
    external_assets_path: Option<String>,
    approved: bool,
}

/// Inspect graph access before binding it to a window. This is intentionally a
/// separate, read-only command so the frontend can show the resolved external
/// target and obtain informed consent before any graph/asset operation begins.
#[tauri::command]
pub(crate) fn inspect_graph_access(
    path: String,
    app: tauri::AppHandle,
) -> Result<GraphAccessInspection, String> {
    let root = resolve_root(&path)
        .ok_or_else(|| "no graph path provided (set TINE_GRAPH or pass a path)".to_string())?;
    let root = canonical_graph_root(&root)?;
    let external = Graph::external_assets_target(&root).map_err(|error| error.to_string())?;
    let approved_target =
        approved_external_assets(&app, &root).and_then(|path| std::fs::canonicalize(path).ok());
    let approved = external
        .as_ref()
        .is_none_or(|target| approved_target.as_ref() == Some(target));
    Ok(GraphAccessInspection {
        graph_root: root.display().to_string(),
        external_assets_path: external.map(|path| path.display().to_string()),
        approved,
    })
}

/// Persist consent only if the submitted target still exactly matches the
/// graph's live canonical assets target (TOCTOU/retarget guard).
#[tauri::command]
pub(crate) fn approve_external_assets(
    graph_root: String,
    assets_path: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let root = canonical_graph_root(&graph_root)?;
    let live = Graph::external_assets_target(&root)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "graph no longer uses an external assets directory".to_string())?;
    let submitted = std::fs::canonicalize(&assets_path)
        .map_err(|error| format!("couldn't resolve external assets path: {error}"))?;
    if submitted != live {
        return Err(format!(
            "external assets directory changed before approval (now {})",
            live.display()
        ));
    }
    remember_external_assets_approval(&app, &root, &live)
}

#[tauri::command]
pub(crate) async fn load_graph(
    path: String,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<LoadGraphResult, String> {
    let label = window.label().to_string();
    drop((window, state));
    let worker_app = app.clone();
    let worker_label = label.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        load_graph_for_label(path, &worker_app, &worker_label, &state)
    })
    .await
    .map_err(|error| format!("graph-open worker failed: {error}"))??;

    if app.get_webview_window(&label).is_none() {
        let binding_generation = match &result {
            LoadGraphResult::Loaded {
                binding_generation, ..
            }
            | LoadGraphResult::AlreadyCurrent {
                binding_generation, ..
            } => Some(*binding_generation),
            LoadGraphResult::FocusedExisting { .. } => None,
        };
        if let Some(binding_generation) = binding_generation {
            let state = app.state::<AppState>();
            let removed = {
                let mut graphs = state.graphs.write().unwrap();
                let owns_generation = graphs
                    .slot(&label)
                    .is_some_and(|slot| slot.binding_generation == binding_generation);
                owns_generation.then(|| graphs.remove(&label)).flatten()
            };
            if removed.is_some() {
                poke_watcher(&state);
            }
        }
        return Err("graph window closed while storage was opening".into());
    }

    Ok(result)
}

fn graph_load_phase(started: Option<Instant>, previous: &mut Option<Instant>, phase: &str) {
    let (Some(started), Some(prior)) = (started, *previous) else {
        return;
    };
    let now = Instant::now();
    crate::debug::diag(format!(
        "graph load phase: {phase}; phase_ms={}; total_ms={}",
        now.duration_since(prior).as_millis(),
        now.duration_since(started).as_millis()
    ));
    *previous = Some(now);
}

/// The one Direct Files publish lifecycle used by ordinary graph open and the
/// cold managed-storage escape.  Callers must already have made their storage
/// authority decision (and, for a cold return, preserved the managed state).
/// It deliberately includes the normal backup, migration, remembered-graph,
/// title, watcher, and cache scheduling work so a recovery binding is not a
/// half-open graph that only happens to answer `AlreadyCurrent` later.
pub(crate) struct DirectFilesOpen {
    pub(crate) meta: GraphMeta,
    pub(crate) binding_generation: u64,
    pub(crate) application_page_admission: ApplicationPageAdmission,
}

/// Install the ordinary Direct Files authority in the window registry.  This is
/// intentionally the one testable sub-boundary of Direct Files publishing: a
/// caller that reaches it has already made the storage decision, and the slot
/// itself proves every later graph command is routed to Direct Files rather
/// than a sparse-v2 actor.
fn publish_direct_files_slot(
    state: &AppState,
    window_label: &str,
    graph: Graph,
    root_key: PathBuf,
) -> Result<(Arc<GraphSlot>, u64), String> {
    let slot = Arc::new(GraphSlot::new(graph, root_key));
    let warm_generation = begin_warm_cache(&slot);
    state
        .graphs
        .write()
        .unwrap()
        .bind(window_label.to_string(), Arc::clone(&slot))?;
    state.note_focused(window_label);
    poke_watcher(state);
    Ok((slot, warm_generation))
}

fn direct_files_projection_path(app: &tauri::AppHandle, root: &Path) -> Result<PathBuf, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app data directory is unavailable: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(b"tine-direct-projection-path-v1\0");
    digest.update(root.to_string_lossy().as_bytes());
    let key = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(app_data
        .join("direct-files-projections")
        .join(format!("{key}.sqlite")))
}

pub(crate) fn open_and_publish_direct_files(
    app: &tauri::AppHandle,
    window_label: &str,
    state: &AppState,
    root_key: PathBuf,
) -> Result<DirectFilesOpen, String> {
    let root = root_key.display().to_string();
    let approved_assets = approved_external_assets(app, &root_key);
    let LoadedGraph {
        graph,
        meta,
        launch_backup_done,
    } = open_graph_for_load(&root, approved_assets.as_deref(), |graph| {
        crate::backup::backup_graph_now(app, graph, "")
    })?;
    if launch_backup_done {
        graph
            .migrate_journal_filenames_checked()
            .map_err(|error| format!("journal filename migration failed: {error}"))?;
    }
    match direct_files_projection_path(app, &root_key) {
        Ok(path) => {
            if let Err(error) = graph.attach_direct_projection(path) {
                crate::debug::diag(format!(
                    "Direct Files SQLite projection unavailable; parser fallback remains active: {error}"
                ));
            }
        }
        Err(error) => crate::debug::diag(format!(
            "Direct Files SQLite projection path unavailable; parser fallback remains active: {error}"
        )),
    }
    let (slot, warm_generation) = publish_direct_files_slot(state, window_label, graph, root_key)?;
    if !launch_backup_done {
        backup_async(app.clone(), slot.clone())?;
    }
    remember_graph(app, &meta.root)?;
    if let Some(window) = app.get_webview_window(window_label) {
        let name = Path::new(&meta.root)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Graph");
        let _ = window.set_title(&format!("Tine — {name}"));
    }
    let binding_generation = slot.binding_generation;
    let application_page_admission = slot.application_page_admission();
    warm_cache_async(app.clone(), window_label.to_string(), slot, warm_generation)?;
    state.clear_startup_recovery_target(window_label);
    Ok(DirectFilesOpen {
        meta,
        binding_generation,
        application_page_admission,
    })
}

pub(crate) fn load_graph_for_label(
    path: String,
    app: &tauri::AppHandle,
    window_label: &str,
    state: &State<'_, AppState>,
) -> Result<LoadGraphResult, String> {
    let started = crate::debug::debug_enabled().then(Instant::now);
    let mut previous = started;
    let root = resolve_root(&path)
        .ok_or_else(|| "no graph path provided (set TINE_GRAPH or pass a path)".to_string())?;
    let root_key = canonical_graph_root(&root)?;
    graph_load_phase(started, &mut previous, "canonical graph root");
    let _load = state.graph_load.lock().unwrap();
    graph_load_phase(started, &mut previous, "serialized graph-open lock");
    if let Some(owner) = state.graphs.read().unwrap().owner(&root_key) {
        if owner == window_label {
            let slot = slot_for_window(&state, &owner)?;
            state.clear_startup_recovery_target(window_label);
            return Ok(LoadGraphResult::AlreadyCurrent {
                meta: slot.graph_meta(),
                binding_generation: slot.binding_generation,
                application_page_admission: slot.application_page_admission(),
            });
        }
        if let Some(existing) = app.get_webview_window(&owner) {
            let _ = existing.show();
            #[cfg(desktop)]
            let _ = existing.unminimize();
            let _ = existing.set_focus();
            // `FocusedExisting` is an explicit activation request. Update
            // capture routing now instead of depending solely on a subsequent
            // OS focus event, which is not guaranteed on every WM/headless
            // environment.
            if state.note_focused(&owner) {
                if let Ok(slot) = slot_for_window(state, &owner) {
                    let _ = remember_graph(app, &slot.root_key.display().to_string());
                }
            }
        }
        state.clear_startup_recovery_target(window_label);
        return Ok(LoadGraphResult::FocusedExisting {
            window_label: owner,
        });
    }
    let binding_record = state.sync_runtime.binding_record(app, &root_key)?;
    graph_load_phase(started, &mut previous, "private storage discovery");
    if let Some(record) = binding_record {
        let meta = crate::sync_runtime::SyncRuntimeFacade::graph_meta(&record);
        let binding = state
            .sync_runtime
            .open_record_for_window(app, window_label, &record)?;
        graph_load_phase(started, &mut previous, "managed storage recovery");
        let slot = Arc::new(GraphSlot::from_sparse_v2(
            binding,
            root_key.clone(),
            meta.clone(),
        ));
        state
            .graphs
            .write()
            .unwrap()
            .bind(window_label.to_string(), Arc::clone(&slot))?;
        state.note_focused(window_label);
        poke_watcher(state);
        remember_graph(app, &meta.root)?;
        if let Some(window) = app.get_webview_window(window_label) {
            let name = Path::new(&meta.root)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Graph");
            let _ = window.set_title(&format!("Tine — {name}"));
        }
        state.clear_startup_recovery_target(window_label);
        return Ok(LoadGraphResult::Loaded {
            meta,
            binding_generation: slot.binding_generation,
            application_page_admission: slot.application_page_admission(),
        });
    }
    refuse_unclaimed_sparse_archive(&root_key)?;
    graph_load_phase(started, &mut previous, "shared storage discovery");
    let direct = open_and_publish_direct_files(app, window_label, state, root_key)?;
    graph_load_phase(started, &mut previous, "Direct Files open and publish");
    Ok(LoadGraphResult::Loaded {
        meta: direct.meta,
        binding_generation: direct.binding_generation,
        application_page_admission: direct.application_page_admission,
    })
}

#[tauri::command]
pub(crate) async fn open_graph_window(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<LoadGraphResult, String> {
    #[cfg(desktop)]
    {
        let id = state.next_window.fetch_add(1, Ordering::Relaxed);
        let label = format!("graph-{id}");
        let result = load_graph_for_label(path, &app, &label, &state)?;
        if let LoadGraphResult::Loaded { ref meta, .. } = result {
            let name = Path::new(&meta.root)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Graph");
            let builder = tauri::WebviewWindowBuilder::new(
                &app,
                &label,
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title(format!("Tine — {name}"))
            .inner_size(1200.0, 820.0)
            .min_inner_size(640.0, 480.0)
            .initialization_script(format!(
                "window.__GRAPH_PATH__ = {};",
                serde_json::to_string(&meta.root).unwrap_or_else(|_| "\"\"".to_string())
            ));
            #[cfg(target_os = "macos")]
            let builder = builder
                .decorations(true)
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            let builder = builder.decorations(crate::settings::native_frame_active());
            #[cfg(target_os = "windows")]
            let builder = if let Some(arguments) = crate::windows_webdriver_args_from_env(None) {
                builder.additional_browser_args(&arguments)
            } else {
                builder
            };
            let built = builder.build();
            match built {
                Ok(window) => {
                    #[cfg(target_os = "linux")]
                    crate::linux_window_identity::apply_to_window(&window);
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    crate::native_mouse_history::install(&window);
                    let _ = window.set_focus();
                }
                Err(error) => {
                    state.graphs.write().unwrap().remove(&label);
                    poke_watcher(&state);
                    return Err(format!("couldn't create graph window: {error}"));
                }
            }
        }
        Ok(result)
    }
    #[cfg(not(desktop))]
    {
        let _ = (path, app, state);
        Err("multiple graph windows are desktop-only".to_string())
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LoadGraphResult {
    Loaded {
        meta: GraphMeta,
        binding_generation: u64,
        application_page_admission: ApplicationPageAdmission,
    },
    AlreadyCurrent {
        meta: GraphMeta,
        binding_generation: u64,
        application_page_admission: ApplicationPageAdmission,
    },
    FocusedExisting {
        window_label: String,
    },
}

fn dir_is_empty(p: &Path) -> bool {
    std::fs::read_dir(p)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

/// Create a brand-new demo graph (the onboarding "Create a new graph" path) and
/// return its root path for the frontend to open. Scaffolds in `dir` if that
/// folder is empty; otherwise creates a fresh `tine-demo` subfolder so we never
/// write into a user's existing files. Does NOT load the graph — the frontend
/// calls `load_graph` with the returned path (matching the "open existing" flow).
#[tauri::command]
pub(crate) fn create_graph(dir: String) -> Result<String, String> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Err("no folder was chosen".into());
    }
    let base = Path::new(dir);
    if !base.is_dir() {
        return Err(format!("{dir} is not a folder"));
    }
    let root = if dir_is_empty(base) {
        base.to_path_buf()
    } else {
        let mut cand = base.join("tine-demo");
        let mut n = 2;
        while cand.exists() {
            cand = base.join(format!("tine-demo-{n}"));
            n += 1;
        }
        std::fs::create_dir(&cand).map_err(|e| format!("couldn't create folder: {e}"))?;
        cand
    };
    tine_core::onboarding::create_demo_graph(&root)
        .map_err(|e| format!("couldn't create the demo graph: {e}"))?;
    Ok(root.display().to_string())
}

#[tauri::command]
pub(crate) fn app_platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "desktop"
    }
}

#[tauri::command]
pub(crate) fn default_graph_parent(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("couldn't resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't create app data dir: {e}"))?;
    Ok(dir.display().to_string())
}

/// Build the search/backlinks cache off the hot path. We let the frontend's
/// first journal load grab the graph lock first, then warm in the background so
/// the first search is instant instead of re-parsing the whole tree. When the
/// warm completes (and this graph is still the current one — generation check),
/// flip `warm_done` and tell the frontend, which has been HOLDING its
/// whole-graph fetches (aliases, ref-count badges) so graph open never does
/// graph-sized work in the foreground.
pub(crate) fn warm_cache_async(
    app: tauri::AppHandle,
    window_label: String,
    slot: Arc<GraphSlot>,
    warm_generation: u64,
) -> Result<(), String> {
    let graph = slot.legacy_graph_cloned()?;
    std::thread::spawn(move || {
        // Brief delay so the first journal paint (which only needs a few pages)
        // grabs the lock first; then build the whole-graph cache in the
        // background so the first search / query / `g j` agenda doesn't pay for
        // parsing every file synchronously under the lock.
        std::thread::sleep(std::time::Duration::from_millis(250));
        if slot.background_cancelled.load(Ordering::Acquire)
            || slot.warm_generation.load(Ordering::Acquire) != warm_generation
        {
            return; // the graph was switched while we slept — a newer warm owns it
        }
        // At most one process-wide graph warm parses files at a time. Rapid
        // switches may leave short-lived sleepers, but cannot amplify disk/CPU
        // work; revoked slots stop between page parses.
        static WARM_WORK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let _worker = WARM_WORK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();
        if slot.background_cancelled.load(Ordering::Acquire)
            || slot.warm_generation.load(Ordering::Acquire) != warm_generation
        {
            return;
        }
        let completed = graph.warm_cache_cancellable(|| {
            slot.background_cancelled.load(Ordering::Acquire)
                || slot.warm_generation.load(Ordering::Acquire) != warm_generation
        });
        if !completed {
            return;
        }
        let state: State<'_, AppState> = app.state();
        let current = state.graphs.read().unwrap().slot(&window_label);
        let still_current = current.as_ref().is_some_and(|current| {
            current.binding_generation == slot.binding_generation
                && current.root_key == slot.root_key
        });
        if still_current && slot.warm_generation.load(Ordering::Acquire) == warm_generation {
            current.unwrap().warm_done.store(true, Ordering::Release);
            let _ = app.emit_to(&window_label, "warm-cache-done", ());
        }
    });
    Ok(())
}

/// "Have the whole-graph derived caches finished warming for the current graph?"
/// Polled once by the frontend after it subscribes to `warm-cache-done`, closing
/// the boot race where the event fired before the listener mounted.
#[tauri::command]
pub(crate) fn warm_done(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    Ok(slot_for_window(&state, window.label())?
        .warm_done
        .load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tine-graph-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("journals")).unwrap();
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        dir
    }

    fn assert_unsafe_provider_refusal(error: &str) {
        assert!(
            error.contains("unsafe filesystem kind"),
            "the refusal must name the user-actionable class: {error}"
        );
        assert!(
            error.contains(ManagedStorageRefusalScenario::UnsafeFilesystemKind.as_str()),
            "the refusal must preserve its stable scenario ID: {error}"
        );
    }

    fn tree_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn collect(root: &Path, relative: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
            for entry in std::fs::read_dir(root.join(relative)).unwrap() {
                let entry = entry.unwrap();
                let child = relative.join(entry.file_name());
                let kind = entry.file_type().unwrap();
                assert!(!kind.is_symlink(), "fixture must not contain symlinks");
                if kind.is_dir() {
                    collect(root, &child, files);
                } else {
                    assert!(kind.is_file(), "fixture must contain only regular files");
                    files.push((child.clone(), std::fs::read(root.join(child)).unwrap()));
                }
            }
        }

        let mut files = Vec::new();
        collect(root, Path::new(""), &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    fn direct_test_state() -> AppState {
        AppState {
            graphs: std::sync::RwLock::new(crate::state::GraphRegistry::default()),
            graph_load: std::sync::Mutex::new(()),
            watch_ctl: std::sync::Mutex::new(None),
            last_focused: std::sync::Mutex::new(None),
            capture_graph: std::sync::Mutex::new(None),
            startup_recovery: std::sync::Mutex::new(std::collections::HashMap::new()),
            sync_runtime: crate::sync_runtime::SyncRuntimeFacade::default(),
            #[cfg(desktop)]
            next_window: std::sync::atomic::AtomicU64::new(1),
        }
    }

    fn copy_graph_text_dir(src: &Path, dest: &Path) -> (usize, bool) {
        let _ = std::fs::create_dir_all(dest);
        let mut copied = 0usize;
        let mut failed = false;
        let Ok(rd) = std::fs::read_dir(src) else {
            return (0, false);
        };
        for entry in rd {
            let Ok(entry) = entry else {
                failed = true;
                continue;
            };
            let p = entry.path();
            if !matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("md") | Some("org")
            ) {
                continue;
            }
            if std::fs::copy(&p, dest.join(entry.file_name())).is_ok() {
                copied += 1;
            } else {
                failed = true;
            }
        }
        (copied, !failed)
    }

    #[test]
    fn remembered_startup_lookup_has_one_bounded_terminal_receipt_without_paths() {
        let dir = scratch("startup-lookup-diagnostics");
        let settings = dir.join("tine-settings.json");
        let remembered = dir.join("remembered-graph");
        std::fs::write(
            &settings,
            serde_json::json!({ "last_graph_path": remembered }).to_string(),
        )
        .unwrap();
        let mut receipts = Vec::new();
        let result =
            remembered_startup_graph_path_at(None, Some(&settings), |phase, terminal, outcome| {
                receipts.push((phase, terminal, outcome));
            });
        assert_eq!(result, Some(remembered.display().to_string()));
        assert_eq!(
            receipts
                .iter()
                .map(|(phase, ..)| *phase)
                .collect::<Vec<_>>(),
            vec![
                "lookup.entry",
                "lookup.app_data",
                "lookup.settings_stat",
                "lookup.settings_read",
                "lookup.settings_parse",
                "lookup.complete",
            ]
        );
        assert_eq!(
            receipts.iter().filter(|(_, terminal, _)| *terminal).count(),
            1
        );
        assert_eq!(
            receipts.last(),
            Some(&("lookup.complete", true, Some("ok")))
        );
        let encoded = serde_json::to_string(&StartupProgressEvent {
            phase: "lookup.complete",
            elapsed_ms: 1,
            terminal: true,
            outcome: Some("ok"),
        })
        .unwrap();
        assert!(!encoded.contains(&remembered.display().to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_progress_elapsed_is_clamped_to_the_frontend_contract_before_serialization() {
        assert_eq!(
            bounded_startup_elapsed_ms(std::time::Duration::from_millis(
                STARTUP_PROGRESS_MAX_ELAPSED_MS - 1
            )),
            STARTUP_PROGRESS_MAX_ELAPSED_MS - 1
        );
        let elapsed_ms = bounded_startup_elapsed_ms(std::time::Duration::from_millis(
            STARTUP_PROGRESS_MAX_ELAPSED_MS + 1,
        ));
        assert_eq!(elapsed_ms, STARTUP_PROGRESS_MAX_ELAPSED_MS);
        let encoded = serde_json::to_value(StartupProgressEvent {
            phase: "lookup.complete",
            elapsed_ms,
            terminal: true,
            outcome: Some("ok"),
        })
        .unwrap();
        assert_eq!(
            encoded["elapsed_ms"],
            serde_json::Value::from(STARTUP_PROGRESS_MAX_ELAPSED_MS)
        );
    }

    #[test]
    fn startup_lookup_keeps_all_settings_io_inside_spawn_blocking() {
        let source = include_str!("graph.rs");
        let start = source
            .find("pub(crate) async fn startup_graph_path")
            .expect("startup lookup command");
        let command = &source[start
            ..source[start..]
                .find("#[tauri::command]")
                .map(|end| start + end)
                .unwrap_or(source.len())];
        let blocking = command
            .find("tauri::async_runtime::spawn_blocking")
            .expect("startup lookup worker");
        assert!(
            !command[..blocking].contains("settings_path"),
            "invoke dispatch must not synchronously read device settings"
        );
        assert!(command[blocking..].contains("startup_graph_path_blocking"));
    }

    #[test]
    fn unclaimed_sparse_archive_refuses_legacy_graph_open() {
        let dir = scratch("unclaimed-sparse");
        assert_eq!(refuse_unclaimed_sparse_archive(&dir), Ok(()));
        std::fs::create_dir_all(dir.join(".tine-sync/v2")).unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive(&dir).unwrap_err(),
            PARTIAL_PROVIDER_REFUSAL
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn honest_cold_provider_prefixes_are_retryable_before_the_descriptor_arrives() {
        for (tag, prefix) in [
            ("outbox-absent", 0_u8),
            ("enrollment-absent", 1),
            ("enrollment-empty", 2),
        ] {
            let dir = scratch(&format!("cold-provider-prefix-{tag}"));
            let page = dir.join("pages/representative.md");
            let recovery = dir.join(".tine-sync/recovery/returned/receipt");
            let shared = dir.join(".tine-sync/v2/shared");
            std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
            std::fs::write(&page, b"- unchanged Direct Files bytes\n").unwrap();
            std::fs::write(&recovery, b"unchanged recovery bytes\n").unwrap();
            match prefix {
                // Syncthing may create the sibling provider tree before outbox.
                0 => std::fs::create_dir_all(shared.join("inbox")).unwrap(),
                // The provider may deliver another recognized outbox namespace
                // before it delivers enrollment.
                1 => std::fs::create_dir_all(shared.join("outbox/objects")).unwrap(),
                // The enrollment directory itself may arrive before its one
                // canonical descriptor file.
                2 => {
                    std::fs::create_dir_all(shared.join("outbox/objects")).unwrap();
                    std::fs::create_dir_all(shared.join("outbox/enrollment")).unwrap();
                }
                _ => unreachable!(),
            }
            let page_before = std::fs::read(&page).unwrap();
            let recovery_before = std::fs::read(&recovery).unwrap();

            assert_eq!(
                refuse_unclaimed_sparse_archive(&dir).unwrap_err(),
                PARTIAL_PROVIDER_REFUSAL,
                "{tag} is an honest provider prefix, not corrupt managed data"
            );
            assert_eq!(std::fs::read(&page).unwrap(), page_before);
            assert_eq!(std::fs::read(&recovery).unwrap(), recovery_before);
            assert!(
                !dir.join(".tine-sync/v1").exists(),
                "a retryable cold refusal must not activate or create v1"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn cold_provider_incomplete_descriptor_is_retryable() {
        let dir = scratch("cold-provider-invalid-enrollment");
        let enrollment = dir.join(".tine-sync/v2/shared/outbox/enrollment");
        std::fs::create_dir_all(&enrollment).unwrap();
        std::fs::write(enrollment.join("shared-enrollment-v1.json"), b"{").unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive(&dir).unwrap_err(),
            PARTIAL_PROVIDER_REFUSAL,
            "a descriptor observed before its final write is an incomplete provider arrival"
        );
        std::fs::write(
            enrollment.join("shared-enrollment-v1.sync-conflict.json"),
            b"{",
        )
        .unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive(&dir).unwrap_err(),
            PARTIAL_PROVIDER_REFUSAL,
            "a sibling provider artifact must not turn an incomplete canonical descriptor into a permanent refusal"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cold_provider_unsafe_canonical_kinds_refuse_but_unrelated_entries_retry() {
        let non_directory = scratch("cold-provider-outbox-file");
        let shared = non_directory.join(".tine-sync/v2/shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("outbox"), b"not a directory").unwrap();
        assert_unsafe_provider_refusal(
            &refuse_unclaimed_sparse_archive(&non_directory).unwrap_err(),
        );
        let _ = std::fs::remove_dir_all(non_directory);

        let unknown = scratch("cold-provider-unknown-outbox-entry");
        std::fs::create_dir_all(unknown.join(".tine-sync/v2/shared/outbox/unknown")).unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive(&unknown).unwrap_err(),
            PARTIAL_PROVIDER_REFUSAL
        );
        let _ = std::fs::remove_dir_all(unknown);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let symlinked = scratch("cold-provider-outbox-symlink");
            let shared = symlinked.join(".tine-sync/v2/shared");
            std::fs::create_dir_all(&shared).unwrap();
            let target = symlinked.join("provider-outbox-target");
            std::fs::create_dir_all(&target).unwrap();
            symlink(&target, shared.join("outbox")).unwrap();
            assert_unsafe_provider_refusal(
                &refuse_unclaimed_sparse_archive(&symlinked).unwrap_err(),
            );
            let _ = std::fs::remove_dir_all(symlinked);
        }
    }

    #[test]
    fn cold_shared_discovery_uses_the_real_v2_shared_namespace() {
        let dir = scratch("cold-shared-layout");
        let v2 = dir.join(".tine-sync/v2");
        let shared = v2.join("shared");
        std::fs::create_dir_all(shared.join("outbox/enrollment")).unwrap();
        std::fs::write(
            shared.join("outbox/enrollment/shared-enrollment-v1.json"),
            b"test descriptor bytes",
        )
        .unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive_with(&dir, |path| {
                assert_eq!(path, shared);
                Ok(true)
            }),
            Ok(())
        );

        std::fs::write(v2.join("unknown"), b"retain").unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive_with(&dir, |_| Ok(true)),
            Ok(()),
            "unrelated v2 provider artifacts do not override the canonical shared namespace"
        );
        std::fs::remove_file(v2.join("unknown")).unwrap();
        assert_eq!(
            refuse_unclaimed_sparse_archive_with(&dir, |_| Ok(false)).unwrap_err(),
            PARTIAL_PROVIDER_REFUSAL
        );
        assert_eq!(
            refuse_unclaimed_sparse_archive_with(&dir, |_| Err("malformed".into())).unwrap_err(),
            "Couldn't validate Tine-managed sync data on this device: malformed"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::remove_dir_all(&shared).unwrap();
            let target = dir.join("provider-target");
            std::fs::create_dir_all(&target).unwrap();
            symlink(&target, &shared).unwrap();
            assert_unsafe_provider_refusal(
                &refuse_unclaimed_sparse_archive_with(&dir, |_| Ok(true)).unwrap_err(),
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn graph_load_snapshots_original_journal_filename_before_migration() {
        let dir = scratch("pre-migrate-backup");
        std::fs::create_dir_all(dir.join("logseq")).unwrap();
        std::fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("journals").join("Thursday, 25-06-2026.org"),
            "* original title-named journal\n",
        )
        .unwrap();
        let backup = dir.join("backup");

        let loaded = open_graph_for_load(dir.to_str().unwrap(), None, |g| {
            copy_graph_text_dir(&g.journals_path(), &backup.join("journals"))
        })
        .unwrap();

        assert!(loaded.launch_backup_done, "pre-migration backup ran");
        assert!(
            backup
                .join("journals")
                .join("Thursday, 25-06-2026.org")
                .exists(),
            "backup must contain the original pre-migration filename"
        );
        assert!(
            dir.join("journals").join("2026_06_25.org").exists(),
            "load still migrates the journal filename"
        );
        assert!(
            !dir.join("journals")
                .join("Thursday, 25-06-2026.org")
                .exists(),
            "live graph was renamed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_provider_arrival_refuses_before_direct_open_without_mutating_the_graph() {
        let dir = scratch("partial-provider-arrival");
        let page = dir.join("pages/representative.md");
        let recovery = dir.join(".tine-sync/recovery/v2-returned-from-desktop/receipt");
        let partial_shared = dir.join(".tine-sync/v2/shared");
        std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&partial_shared).unwrap();
        std::fs::write(&page, "- representative Direct Files bytes\n").unwrap();
        std::fs::write(&recovery, "preserved recovery evidence\n").unwrap();
        let page_before = std::fs::read(&page).unwrap();
        let recovery_before = std::fs::read(&recovery).unwrap();

        assert_eq!(
            refuse_unclaimed_sparse_archive(&dir).unwrap_err(),
            "Tine-managed storage sync data appears to still be arriving or is incomplete. Tine left this graph unchanged. Let your file-sync provider finish, then Retry."
        );
        assert_eq!(std::fs::read(&page).unwrap(), page_before);
        assert_eq!(std::fs::read(&recovery).unwrap(), recovery_before);
        assert!(
            !dir.join(".tine-sync/v1").exists(),
            "a partial v2 refusal must not invoke legacy activation"
        );

        let source = include_str!("graph.rs");
        let load = &source[source
            .find("pub(crate) fn load_graph_for_label")
            .expect("ordinary graph-load decision")..];
        assert!(
            load.find("refuse_unclaimed_sparse_archive")
                < load.find("open_and_publish_direct_files"),
            "partial provider evidence must be refused before a Direct Files binding is installed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_direct_publish_treats_v1_and_return_recovery_as_inert() {
        let dir = scratch("direct-with-inert-legacy-and-recovery");
        let page = dir.join("pages/representative.md");
        let recovery = dir.join(".tine-sync/recovery/v2-returned-from-desktop/receipt");
        let v1 = dir.join(".tine-sync/v1");
        std::fs::write(&page, "- exact Direct Files bytes\n").unwrap();
        std::fs::create_dir_all(v1.join("genesis")).unwrap();
        std::fs::write(v1.join("genesis/inert-v1-sentinel"), b"legacy v1 bytes\n").unwrap();
        std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
        std::fs::write(&recovery, "preserved provider recovery evidence\n").unwrap();
        let page_before = std::fs::read(&page).unwrap();
        let v1_before = tree_bytes(&v1);
        let recovery_before = std::fs::read(&recovery).unwrap();
        let graph_before = tree_bytes(&dir);

        Graph::open_checked(&dir)
            .expect("inert legacy-v1 bytes must not reject a checked Direct Files open");
        let root_key = std::fs::canonicalize(&dir).unwrap();
        let loaded = open_graph_for_load(dir.to_str().unwrap(), None, |_| (0, false)).unwrap();
        assert_eq!(loaded.meta.root, dir.display().to_string());
        let state = direct_test_state();
        let (slot, warm_generation) =
            publish_direct_files_slot(&state, "ordinary", loaded.graph, root_key.clone()).unwrap();
        let installed = state
            .graphs
            .read()
            .unwrap()
            .slot("ordinary")
            .expect("ordinary publish must install a graph slot");
        assert!(Arc::ptr_eq(&installed, &slot));
        assert_eq!(installed.root_key, root_key);
        assert_eq!(installed.binding_generation, slot.binding_generation);
        assert_eq!(
            installed.warm_generation.load(Ordering::Acquire),
            warm_generation,
            "the installed Direct slot owns the scheduled warm generation"
        );
        assert!(
            !installed.is_sparse_v2() && installed.legacy_graph().is_ok(),
            "ordinary publish must install a Direct Files registry binding"
        );
        assert_eq!(std::fs::read(&page).unwrap(), page_before);
        assert_eq!(std::fs::read(&recovery).unwrap(), recovery_before);
        assert_eq!(tree_bytes(&v1), v1_before);
        assert_eq!(tree_bytes(&dir), graph_before);

        let no_v1 = scratch("direct-publish-does-not-create-v1");
        std::fs::write(no_v1.join("pages/representative.md"), b"- direct only\n").unwrap();
        let no_v1_before = tree_bytes(&no_v1);
        let no_v1_root = std::fs::canonicalize(&no_v1).unwrap();
        let loaded = open_graph_for_load(no_v1.to_str().unwrap(), None, |_| (0, false)).unwrap();
        publish_direct_files_slot(&state, "direct-only", loaded.graph, no_v1_root).unwrap();
        assert!(
            !no_v1.join(".tine-sync/v1").exists(),
            "ordinary Direct Files publishing must not create or activate v1"
        );
        assert_eq!(tree_bytes(&no_v1), no_v1_before);

        // A pre-release v1 prototype could also have stopped before it had
        // created a directory-shaped store.  The retired child is never
        // traversed, so this malformed-but-inert shape must be just as harmless
        // to the checked Direct Files path as a representative directory tree.
        let v1_file = scratch("direct-publish-with-inert-v1-file");
        let v1_file_path = v1_file.join(".tine-sync/v1");
        std::fs::write(
            v1_file.join("pages/representative.md"),
            b"- direct with old v1 file\n",
        )
        .unwrap();
        std::fs::create_dir_all(v1_file_path.parent().unwrap()).unwrap();
        std::fs::write(&v1_file_path, b"incomplete retired v1 bytes\n").unwrap();
        let v1_file_before = tree_bytes(&v1_file);
        Graph::open_checked(&v1_file)
            .expect("an inert non-directory v1 child must not reject Direct Files");
        let v1_file_root = std::fs::canonicalize(&v1_file).unwrap();
        let loaded = open_graph_for_load(v1_file.to_str().unwrap(), None, |_| (0, false)).unwrap();
        let (slot, _) =
            publish_direct_files_slot(&state, "inert-v1-file", loaded.graph, v1_file_root).unwrap();
        assert!(
            !slot.is_sparse_v2() && slot.legacy_graph().is_ok(),
            "the malformed legacy child still installs an ordinary Direct Files slot"
        );
        assert_eq!(
            tree_bytes(&v1_file),
            v1_file_before,
            "Direct Files must neither inspect nor rewrite inert malformed v1 bytes"
        );

        let _ = std::fs::remove_dir_all(&v1_file);
        let _ = std::fs::remove_dir_all(&no_v1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "manual large-graph startup benchmark"]
    fn direct_large_graph_open_manual_benchmark() {
        let page_count = std::env::var("TINE_DIRECT_OPEN_BENCH_PAGES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(13_000);
        let asset_count = std::env::var("TINE_DIRECT_OPEN_BENCH_ASSETS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(12_884);
        let dir = scratch("direct-large-open-bench");
        let assets = dir.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        for index in 0..page_count {
            std::fs::write(
                dir.join("pages").join(format!("page-{index:05}.md")),
                format!("- page {index}\n"),
            )
            .unwrap();
        }
        for index in 0..asset_count {
            let bucket = assets.join(format!("bucket-{:03}", index % 281));
            std::fs::create_dir_all(&bucket).unwrap();
            let file = std::fs::File::create(bucket.join(format!("asset-{index:05}.bin"))).unwrap();
            file.set_len(2 * 1024 * 1024).unwrap();
        }

        let started = std::time::Instant::now();
        let loaded = open_graph_for_load(dir.to_str().unwrap(), None, |_| (0, false)).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(loaded.meta.root, dir.display().to_string());
        eprintln!(
            "direct open: pages={page_count}, assets={asset_count}, apparent_asset_gib={:.1}, elapsed={elapsed:?}",
            asset_count as f64 * 2.0 / 1024.0
        );

        if std::env::var_os("TINE_DIRECT_OPEN_BENCH_KEEP").is_some() {
            eprintln!("retained benchmark graph at {}", dir.display());
        } else {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
