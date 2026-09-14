use crate::backup::backup_async;
use crate::settings::{
    approved_external_assets, remember_external_assets_approval, remember_graph,
};
use crate::state::{
    canonical_graph_root, poke_watcher, slot_for_window, AppState, ApplicationPageAdmission,
    GraphSlot,
};
use crate::storage_mode_supervisor::{
    StableStorageMode, StorageTransitionKind, StorageTransitionOutcome, StorageTransitionPhase,
};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
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

fn startup_graph_path_blocking(app: &tauri::AppHandle) -> Option<String> {
    let settings_path = crate::settings::settings_path(app);
    remembered_startup_graph_path_at(
        resolve_root(""),
        settings_path.as_deref(),
        |phase, terminal, outcome| {
            crate::debug::diag(format!(
                "startup lookup: phase={phase}; terminal={terminal}; outcome={}",
                outcome.unwrap_or("none")
            ));
        },
    )
}

#[tauri::command]
pub(crate) async fn startup_graph_path(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Option<String> {
    let label = window.label().to_string();
    let state = app.state::<AppState>();
    let lookup_id = match state.storage_supervisor.begin_transition(
        &app,
        &label,
        None,
        StorageTransitionKind::Lookup,
    ) {
        Ok(operation_id) => operation_id,
        Err(error) => {
            crate::debug::diag(error);
            return None;
        }
    };
    let worker_app = app.clone();
    let worker_label = label.clone();
    match tauri::async_runtime::spawn_blocking(move || {
        let worker_state = worker_app.state::<AppState>();
        if let Err(error) = worker_state.storage_supervisor.advance_transition(
            &worker_app,
            lookup_id,
            StorageTransitionPhase::LookingUpSelection,
        ) {
            crate::debug::diag(error);
        }
        let result = startup_graph_path_blocking(&worker_app);
        let canonical_target = result
            .as_deref()
            .and_then(|path| canonical_graph_root(path).ok());
        if let Some(root) = canonical_target.clone() {
            if let Err(error) = worker_state
                .storage_supervisor
                .bind_transition_root(lookup_id, root.clone())
            {
                crate::debug::diag(error);
            }
            worker_state
                .storage_supervisor
                .select_window_root(&worker_label, root);
        }
        if let Err(error) = worker_state.storage_supervisor.finish_transition(
            &worker_app,
            lookup_id,
            StorageTransitionOutcome::Succeeded,
            None,
            None,
        ) {
            crate::debug::diag(error);
        }
        result
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            if let Err(error) = state.storage_supervisor.finish_transition(
                &app,
                lookup_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("worker_join_failed".into()),
            ) {
                crate::debug::diag(error);
            }
            None
        }
    }
}

#[tauri::command]
pub(crate) fn capture_target(
    state: State<'_, AppState>,
) -> Result<String, crate::command_error::CommandError> {
    capture_target_for_state(&state)
}

fn capture_target_for_state(
    state: &AppState,
) -> Result<String, crate::command_error::CommandError> {
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
        .ok_or_else(|| crate::command_error::CommandError::prose("no graph window is open"))
}

#[derive(serde::Serialize)]
pub(crate) struct CaptureGraphBindingResult {
    pub(crate) binding_generation: u64,
}

/// Complete the current native show with one selected read lease. Beginning
/// the show already revoked the prior lease; a superseded request cannot bind.
/// The frontend only reads the resulting selection through `capture_graph_binding`.
pub(crate) fn refresh_capture_graph_binding(
    state: &AppState,
    show_generation: u64,
) -> Result<Option<u64>, crate::command_error::CommandError> {
    let target = capture_target_for_state(state)?;
    let slot = slot_for_window(state, &target).map_err(crate::command_error::CommandError::from)?;
    let binding_generation = slot.binding_generation;
    Ok(state
        .complete_capture_show(show_generation, target, binding_generation)
        .then_some(binding_generation))
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
) -> Result<CaptureGraphBindingResult, crate::command_error::CommandError> {
    if window.label() != "capture" {
        return Err(crate::command_error::CommandError::prose(
            "capture graph binding is only available to quick capture",
        ));
    }
    let binding_generation = state
        .capture_graph_binding()
        .ok_or_else(|| {
            crate::command_error::CommandError::prose("no graph bound for quick capture")
        })?
        .binding_generation;
    Ok(CaptureGraphBindingResult { binding_generation })
}

struct LoadedGraph {
    graph: Graph,
    meta: GraphMeta,
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

pub(crate) fn refuse_unclaimed_sparse_archive(
    root: &Path,
) -> Result<(), crate::command_error::CommandError> {
    refuse_unclaimed_sparse_archive_with(root, |shared| match inspect_shared_provider_cold_prefix(
        shared,
    )
    .map_err(crate::command_error::CommandError::graph)?
    {
        SyncSharedProviderColdPrefix::Partial => Ok(false),
        SyncSharedProviderColdPrefix::ReadyForDescriptorInspection => {
            inspect_shared_enrollment_for_cold_discovery(shared)
                .map(|descriptor| descriptor.is_some())
                // A descriptor file can be observed between the provider's
                // create and final write/rename.  Its malformed bytes are an
                // incomplete arrival, not proof of hostile graph state.
                .or(Ok(false))
        }
        SyncSharedProviderColdPrefix::Refused => {
            Err(crate::command_error::CommandError::graph(format!(
                "shared provider namespace has an unsafe filesystem kind [scenario_id={}]",
                ManagedStorageRefusalScenario::UnsafeFilesystemKind
            )))
        }
    })
}

fn refuse_unclaimed_sparse_archive_with(
    root: &Path,
    inspect_shared: impl FnOnce(&Path) -> Result<bool, crate::command_error::CommandError>,
) -> Result<(), crate::command_error::CommandError> {
    match inspect_unclaimed_sparse_archive(root, inspect_shared)? {
        ColdSparseArchive::Absent | ColdSparseArchive::Joinable => Ok(()),
        ColdSparseArchive::Partial => {
            crate::debug::diag(
                "sparse-v2 cold discovery: phase=provider_evidence; outcome=partial_provider_refusal",
            );
            Err(crate::command_error::CommandError::prose(PARTIAL_PROVIDER_REFUSAL))
        }
        ColdSparseArchive::Refused(scenario) => Err(crate::command_error::CommandError::graph(format!(
            "Tine-managed storage data has an unsafe filesystem kind, so this graph could not be opened safely. [scenario_id={scenario}]"
        ))),
    }
}

fn inspect_unclaimed_sparse_archive(
    root: &Path,
    inspect_shared: impl FnOnce(&Path) -> Result<bool, crate::command_error::CommandError>,
) -> Result<ColdSparseArchive, crate::command_error::CommandError> {
    let archive = root.join(".tine-sync/v2");
    let metadata = match std::fs::symlink_metadata(&archive) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ColdSparseArchive::Absent)
        }
        Err(error) => {
            return Err(crate::command_error::CommandError::graph(format!(
                "Couldn't verify Tine-managed storage data before opening this graph: {error}"
            )));
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
            return Err(crate::command_error::CommandError::graph(format!(
                "Couldn't verify Tine-managed storage data before opening this graph: {error}"
            )))
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
        Err(error) => Err(crate::command_error::CommandError::graph(format!(
            "Couldn't validate Tine-managed sync data on this device: {error}"
        ))),
    }
}

fn open_graph_for_load(
    root: &str,
    approved_assets: Option<&Path>,
) -> Result<LoadedGraph, crate::command_error::CommandError> {
    let graph = Graph::open_checked_with_assets(root, approved_assets).map_err(|e| {
        crate::command_error::CommandError::graph(format!("unsafe graph layout: {e}"))
    })?;
    let meta = graph.meta();
    // Concord invariant 4 (write-shyness). Opening a graph used to RENAME every
    // title-named journal file to its date stem, behind a synchronous launch
    // backup. It is a genuine repair — such a file cannot be parsed back to a
    // date, so its day looks empty — but it is not a repair the user asked for,
    // and in a graph kept in git or behind a sync tool it lands as a tree
    // rewrite the moment Tine is started. The renames are now PROPOSED
    // (`journal_filename_migrations`) and applied only by the explicit
    // `apply_journal_filename_migrations` command, which takes the same
    // pre-migration snapshot first. Opening touches nothing.
    Ok(LoadedGraph { graph, meta })
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
) -> Result<GraphAccessInspection, crate::command_error::CommandError> {
    let root = resolve_root(&path).ok_or_else(|| {
        crate::command_error::CommandError::prose(
            "no graph path provided (set TINE_GRAPH or pass a path)",
        )
    })?;
    let root = canonical_graph_root(&root).map_err(crate::command_error::CommandError::from)?;
    let external =
        Graph::external_assets_target(&root).map_err(crate::command_error::CommandError::from)?;
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
) -> Result<(), crate::command_error::CommandError> {
    let root =
        canonical_graph_root(&graph_root).map_err(crate::command_error::CommandError::from)?;
    let live = Graph::external_assets_target(&root)
        .map_err(crate::command_error::CommandError::from)?
        .ok_or_else(|| {
            crate::command_error::CommandError::prose(
                "graph no longer uses an external assets directory",
            )
        })?;
    let submitted = std::fs::canonicalize(&assets_path).map_err(|error| {
        crate::command_error::CommandError::graph(format!(
            "couldn't resolve external assets path: {error}"
        ))
    })?;
    if submitted != live {
        return Err(crate::command_error::CommandError::graph(format!(
            "external assets directory changed before approval (now {})",
            live.display()
        )));
    }
    remember_external_assets_approval(&app, &root, &live)
}

#[tauri::command]
pub(crate) async fn load_graph(
    path: String,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<LoadGraphResult, crate::command_error::CommandError> {
    let label = window.label().to_string();
    drop((window, state));
    let worker_app = app.clone();
    let worker_label = label.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        load_graph_for_label(path, &worker_app, &worker_label, &state)
    })
    .await
    .map_err(|error| {
        crate::command_error::CommandError::graph(format!("graph-open worker failed: {error}"))
    })??;

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
        return Err(crate::command_error::CommandError::prose(
            "graph window closed while storage was opening",
        ));
    }

    // Only a pending unbound Capture request may consume this publication.
    // A shown Capture keeps its frozen graph lease across unrelated opens.
    #[cfg(desktop)]
    if let LoadGraphResult::Loaded {
        binding_generation, ..
    }
    | LoadGraphResult::AlreadyCurrent {
        binding_generation, ..
    } = &result
    {
        crate::complete_pending_capture_show(&app, label.clone(), *binding_generation);
    }

    // The iOS Simulator probe exercises the same bound-graph helper as the
    // Guide button.  Keeping the hook simulator-only prevents a hidden launch
    // argument from becoming product behavior on physical devices.
    #[cfg(all(target_os = "ios", target_abi = "sim"))]
    if std::env::args().any(|argument| argument == "--tine-ci-copy-guide") {
        let binding_generation = match &result {
            LoadGraphResult::Loaded {
                binding_generation, ..
            }
            | LoadGraphResult::AlreadyCurrent {
                binding_generation, ..
            } => Some(*binding_generation),
            LoadGraphResult::FocusedExisting { .. } => None,
        }
        .ok_or_else(|| {
            crate::command_error::CommandError::prose(
                "iOS Guide-copy probe did not retain the requested graph",
            )
        })?;
        let worker_app = app.clone();
        let worker_label = label.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::commands::copy_guide_into_bound_graph(
                &worker_app,
                &worker_label,
                binding_generation,
                "Tine Guide".to_string(),
            )
        })
        .await
        .map_err(|error| {
            crate::command_error::CommandError::graph(format!(
                "iOS Guide-copy probe worker failed: {error}"
            ))
        })?
        .map_err(|error| {
            crate::command_error::CommandError::graph(format!(
                "iOS Guide-copy probe failed: {error}"
            ))
        })?;
        crate::debug::diag("iOS Guide-copy probe completed".to_string());
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
) -> Result<(Arc<GraphSlot>, u64), crate::command_error::CommandError> {
    let slot = Arc::new(GraphSlot::new(graph, root_key));
    let warm_generation = begin_warm_cache(&slot);
    state
        .graphs
        .write()
        .unwrap()
        .bind(window_label.to_string(), Arc::clone(&slot))
        .map_err(crate::command_error::CommandError::from)?;
    state.note_focused(window_label);
    poke_watcher(state);
    Ok((slot, warm_generation))
}

fn direct_files_projection_path(
    app: &tauri::AppHandle,
    root: &Path,
) -> Result<PathBuf, crate::command_error::CommandError> {
    let app_data = app.path().app_data_dir().map_err(|error| {
        crate::command_error::CommandError::graph(format!(
            "app data directory is unavailable: {error}"
        ))
    })?;
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

pub(crate) struct PreparedDirectFilesOpen {
    graph: Graph,
    meta: GraphMeta,
    root_key: PathBuf,
}

/// Perform the expensive, non-authoritative Direct Files work without touching
/// the window registry.  A storage transition may be superseded while this
/// runs; only `publish_prepared_direct_files` is the linearization point.
pub(crate) fn prepare_direct_files_open(
    app: &tauri::AppHandle,
    root_key: PathBuf,
) -> Result<PreparedDirectFilesOpen, crate::command_error::CommandError> {
    let root = root_key.display().to_string();
    let approved_assets = approved_external_assets(app, &root_key);
    // Convergent cross-page moves (packet B2, I-3/I-2). A move writes N+1 files
    // and the process may die between any two of them. The durable record that
    // makes that convergent is retired here, BEFORE a single page is parsed, so
    // no reader ever observes the half-move: the next open either completes it
    // or rolls it back. This is the layer that owns the app-private root —
    // `Graph` is never handed one, which is exactly why recovery cannot live
    // inside it. See `docs/contracts/direct-move-recovery.md` §3.
    match crate::backup::direct_move_recovery_dir(app, &root_key) {
        Some(store_root) => {
            let report = tine_core::direct_move_recovery::recover_all(&store_root, &root_key);
            if !report.is_empty() {
                crate::debug::diag(report.summary());
            }
        }
        None => crate::debug::diag(
            "Direct move recovery store unavailable; a crashed cross-page move cannot converge"
                .to_string(),
        ),
    }
    let LoadedGraph { graph, meta } = open_graph_for_load(&root, approved_assets.as_deref())?;
    attach_direct_files_services(&graph, direct_files_service_paths(app, &root_key));
    Ok(PreparedDirectFilesOpen {
        graph,
        meta,
        root_key,
    })
}

/// The app-private locations of the services a Direct Files `Graph` carries:
/// the disposable SQLite projection and the Concord base ledger. Resolved from
/// the `AppHandle` once, so the attach itself needs no handle and can be driven
/// from a test.
pub(crate) struct DirectFilesServicePaths {
    pub(crate) projection: Result<PathBuf, crate::command_error::CommandError>,
    pub(crate) concord_ledger: Option<PathBuf>,
}

pub(crate) fn direct_files_service_paths(
    app: &tauri::AppHandle,
    root_key: &Path,
) -> DirectFilesServicePaths {
    DirectFilesServicePaths {
        projection: direct_files_projection_path(app, root_key),
        concord_ledger: crate::backup::concord_ledger_dir(app, root_key),
    }
}

/// Attach the Direct Files services to a freshly opened `Graph`. This is the
/// ONE place that does so: the ordinary open and the configuration refresh
/// (`state::reopen_legacy_for_refresh`) both go through it, so a `Graph` that
/// reaches the window registry always carries its projection. A refresh that
/// reopened without attaching left every query `ProjectionUnavailable` until
/// the next graph open (GH draft "Query Engine", 2026-09-11: dismissing the
/// Guide toast killed all queries).
pub(crate) fn attach_direct_files_services(graph: &Graph, paths: DirectFilesServicePaths) {
    match paths.projection {
        Ok(path) => {
            if let Err(error) = graph.attach_direct_projection(path) {
                crate::debug::diag(format!(
                    "Direct Files SQLite projection unavailable; indexed reads are unavailable: {error}"
                ));
            }
        }
        Err(error) => crate::debug::diag(format!(
            "Direct Files SQLite projection path unavailable; indexed reads are unavailable: {error}"
        )),
    }
    // Concord base ledger (ADR 0056): Direct Files only, app-private, outside
    // the sync tree. Attach failure is impossible (the attach performs no I/O);
    // an unavailable app-data dir simply leaves the ledger off — every hook
    // no-ops and conflict diffs stay 2-way.
    match paths.concord_ledger {
        Some(dir) => graph.attach_concord_ledger(dir),
        None => crate::debug::diag(
            "Concord ledger directory unavailable; conflict diffs stay 2-way".to_string(),
        ),
    }
}

/// The short authoritative half of a Direct Files open.  Callers must invoke
/// this from the native storage supervisor's `commit_if_current` closure.
pub(crate) fn publish_prepared_direct_files(
    app: &tauri::AppHandle,
    window_label: &str,
    state: &AppState,
    prepared: PreparedDirectFilesOpen,
) -> Result<DirectFilesOpen, crate::command_error::CommandError> {
    let PreparedDirectFilesOpen {
        graph,
        meta,
        root_key,
    } = prepared;
    let (slot, warm_generation) = publish_direct_files_slot(state, window_label, graph, root_key)?;
    // Opening no longer mutates the tree, so the launch snapshot is never on a
    // rename's critical path: it stays the ordinary background backup.
    backup_async(app.clone(), slot.clone())?;
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
) -> Result<LoadGraphResult, crate::command_error::CommandError> {
    let started = crate::debug::debug_enabled().then(Instant::now);
    let mut previous = started;
    let root = resolve_root(&path).ok_or_else(|| {
        crate::command_error::CommandError::prose(
            "no graph path provided (set TINE_GRAPH or pass a path)",
        )
    })?;
    let root_key = canonical_graph_root(&root).map_err(crate::command_error::CommandError::from)?;
    state
        .storage_supervisor
        .select_window_root(window_label, root_key.clone());
    graph_load_phase(started, &mut previous, "canonical graph root");
    let transition_gate = state.storage_supervisor.transition_lane(&root_key);
    let _load = transition_gate.lock().unwrap();
    graph_load_phase(started, &mut previous, "serialized graph-open lock");
    let lookup_id = state.storage_supervisor.begin_transition(
        app,
        window_label,
        Some(root_key.clone()),
        StorageTransitionKind::Lookup,
    )?;
    state.storage_supervisor.advance_transition(
        app,
        lookup_id,
        StorageTransitionPhase::LookingUpSelection,
    )?;
    if let Some(owner) = state.graphs.read().unwrap().owner(&root_key) {
        if owner == window_label {
            let slot = slot_for_window(&state, &owner)
                .map_err(crate::command_error::CommandError::from)?;
            state.storage_supervisor.finish_transition(
                app,
                lookup_id,
                StorageTransitionOutcome::Succeeded,
                None,
                None,
            )?;
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
        state.storage_supervisor.finish_transition(
            app,
            lookup_id,
            StorageTransitionOutcome::Succeeded,
            None,
            None,
        )?;
        return Ok(LoadGraphResult::FocusedExisting {
            window_label: owner,
        });
    }
    let mut rebuild_managed_after_direct = false;
    let binding_record = match state.sync_runtime.binding_record(app, &root_key) {
        Ok(binding_record) => binding_record,
        // There is no pre-0.7 migration path. If the private opt-in record is
        // readable but not the one current format, preserve the complete
        // private root and rebuild managed state from the Markdown/Org tree.
        Err(_) if state.sync_runtime.unrecognized_binding_file(app, &root_key) => {
            if let Err(error) = state
                .sync_runtime
                .archive_unrecognized_private_state(app, &root_key)
            {
                // The archive helper published the durable Direct/rebuild
                // intent first. Keep the Markdown/Org graph available now;
                // automatic activation below will retry the archive, and a
                // later open will retry again if that activation also fails.
                crate::debug::diag(format!(
                    "pre-0.7 managed-state archive deferred; opening Direct Files: {error}"
                ));
            }
            rebuild_managed_after_direct = true;
            None
        }
        Err(error) => {
            let _ = state.storage_supervisor.finish_transition(
                app,
                lookup_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("private_storage_discovery_failed".into()),
            );
            return Err(error);
        }
    };
    if binding_record.is_none()
        && state
            .sync_runtime
            .blank_slate_rebuild_pending(app, &root_key)?
    {
        // This includes a crash after archival and a prior automatic rebuild
        // attempt that failed while Direct Files remained safely available.
        rebuild_managed_after_direct = true;
    }
    graph_load_phase(started, &mut previous, "private storage discovery");
    let mut lookup_finished = false;
    'managed: {
        let Some(record) = binding_record else {
            break 'managed;
        };
        state.storage_supervisor.finish_transition(
            app,
            lookup_id,
            StorageTransitionOutcome::Succeeded,
            None,
            None,
        )?;
        lookup_finished = true;
        let managed_id = state.storage_supervisor.begin_transition(
            app,
            window_label,
            Some(root_key.clone()),
            StorageTransitionKind::OpenManaged,
        )?;
        state.storage_supervisor.advance_transition(
            app,
            managed_id,
            StorageTransitionPhase::ValidatingTarget,
        )?;
        state.storage_supervisor.advance_transition(
            app,
            managed_id,
            StorageTransitionPhase::OpeningManaged,
        )?;
        let meta = crate::sync_runtime::SyncRuntimeFacade::graph_meta(&record);
        let binding = match state
            .sync_runtime
            .open_record_for_window(app, window_label, &record)
        {
            Ok(binding) => binding,
            Err(error) => {
                let _ = state.storage_supervisor.finish_transition(
                    app,
                    managed_id,
                    StorageTransitionOutcome::Failed,
                    None,
                    Some("managed_open_failed".into()),
                );
                return Err(error);
            }
        };
        graph_load_phase(started, &mut previous, "managed storage recovery");
        if binding.requires_blank_slate_rebuild() {
            // A readable outer binding can still lead to an unrecognized
            // private store. Preserve it, publish a Direct Files source, then
            // reconstruct the one current managed format automatically.
            if let Err(error) = state
                .sync_runtime
                .archive_unrecognized_private_state(app, &root_key)
            {
                crate::debug::diag(format!(
                    "pre-0.7 managed-state archive deferred; opening Direct Files: {error}"
                ));
                state.storage_supervisor.finish_transition(
                    app,
                    managed_id,
                    StorageTransitionOutcome::Cancelled,
                    None,
                    Some("blank_slate_archive_deferred".into()),
                )?;
                rebuild_managed_after_direct = true;
                break 'managed;
            }
            state.storage_supervisor.finish_transition(
                app,
                managed_id,
                StorageTransitionOutcome::Cancelled,
                None,
                Some("blank_slate_rebuild_requested".into()),
            )?;
            rebuild_managed_after_direct = true;
            break 'managed;
        }
        if let Some(detail) = binding.serving_failure_detail() {
            let _ = state.storage_supervisor.finish_transition(
                app,
                managed_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("managed_open_not_serving".into()),
            );
            return Err(crate::command_error::CommandError::graph(format!(
                "Managed storage could not serve this workspace: {detail}"
            )));
        }
        let slot = Arc::new(GraphSlot::from_sparse_v2(
            binding,
            root_key.clone(),
            meta.clone(),
        ));
        crate::sync_runtime::prove_managed_application_ready(&slot, None).map_err(|error| {
            crate::command_error::CommandError::graph({
                let _ = state.storage_supervisor.finish_transition(
                    app,
                    managed_id,
                    StorageTransitionOutcome::Failed,
                    None,
                    Some("managed_readiness_failed".into()),
                );
                format!("Managed storage opened but its pages are not usable: {error}")
            })
        })?;
        graph_load_phase(started, &mut previous, "managed application readiness");
        if let Err(error) = state.storage_supervisor.commit_if_current(managed_id, || {
            state
                .graphs
                .write()
                .unwrap()
                .bind(window_label.to_string(), Arc::clone(&slot))
                .map_err(crate::command_error::CommandError::from)
        }) {
            // A newer operation (especially emergency Direct Files or a graph
            // switch) owns publication now. Completing this stale managed
            // worker is not a user-visible graph-open failure: report the
            // binding that actually won, if one is already installed.
            if !state.storage_supervisor.operation_is_current(managed_id) {
                if let Ok(current) = slot_for_window(state, window_label) {
                    return Ok(LoadGraphResult::AlreadyCurrent {
                        meta: current.graph_meta(),
                        binding_generation: current.binding_generation,
                        application_page_admission: current.application_page_admission(),
                    });
                }
            }
            let _ = state.storage_supervisor.finish_transition(
                app,
                managed_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("managed_publish_failed".into()),
            );
            return Err(error);
        }
        state.storage_supervisor.finish_transition(
            app,
            managed_id,
            StorageTransitionOutcome::Succeeded,
            Some(StableStorageMode::Managed),
            None,
        )?;
        graph_load_phase(started, &mut previous, "managed publish");
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
        return Ok(LoadGraphResult::Loaded {
            meta,
            binding_generation: slot.binding_generation,
            application_page_admission: slot.application_page_admission(),
        });
    }
    let explicit_direct = state
        .sync_runtime
        .direct_selection_is_active(app, &root_key)?;
    if !explicit_direct {
        if let Err(error) = refuse_unclaimed_sparse_archive(&root_key) {
            let _ = state.storage_supervisor.finish_transition(
                app,
                lookup_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("unclaimed_managed_archive".into()),
            );
            return Err(error);
        }
    }
    graph_load_phase(started, &mut previous, "shared storage discovery");
    if !lookup_finished {
        state.storage_supervisor.finish_transition(
            app,
            lookup_id,
            StorageTransitionOutcome::Succeeded,
            None,
            None,
        )?;
    }
    let direct_id = state.storage_supervisor.begin_transition(
        app,
        window_label,
        Some(root_key.clone()),
        StorageTransitionKind::OpenDirect,
    )?;
    state.storage_supervisor.advance_transition(
        app,
        direct_id,
        StorageTransitionPhase::ValidatingTarget,
    )?;
    state.storage_supervisor.advance_transition(
        app,
        direct_id,
        StorageTransitionPhase::OpeningDirect,
    )?;
    let prepared = match prepare_direct_files_open(app, root_key) {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = state.storage_supervisor.finish_transition(
                app,
                direct_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("direct_open_failed".into()),
            );
            return Err(error);
        }
    };
    graph_load_phase(started, &mut previous, "Direct Files prepare");
    let direct = match state.storage_supervisor.commit_if_current(direct_id, || {
        publish_prepared_direct_files(app, window_label, state, prepared)
    }) {
        Ok(direct) => direct,
        Err(error) => {
            let _ = state.storage_supervisor.finish_transition(
                app,
                direct_id,
                StorageTransitionOutcome::Failed,
                None,
                Some("direct_open_failed".into()),
            );
            return Err(error);
        }
    };
    state.storage_supervisor.finish_transition(
        app,
        direct_id,
        StorageTransitionOutcome::Succeeded,
        Some(StableStorageMode::Direct),
        None,
    )?;
    graph_load_phase(started, &mut previous, "Direct Files publish");
    if rebuild_managed_after_direct {
        let source_generation = direct.binding_generation;
        drop(_load);
        return match crate::sync_runtime::activate_sparse_v2_blocking(
            app,
            window_label,
            source_generation,
        ) {
            Ok(_) => {
                let managed = slot_for_window(state, window_label)
                    .map_err(crate::command_error::CommandError::from)?;
                Ok(LoadGraphResult::Loaded {
                    meta: managed.graph_meta(),
                    binding_generation: managed.binding_generation,
                    application_page_admission: managed.application_page_admission(),
                })
            }
            Err(error) => {
                crate::debug::diag(format!(
                    "automatic managed-storage rebuild failed; Direct Files remains active: {error}"
                ));
                Ok(LoadGraphResult::Loaded {
                    meta: direct.meta,
                    binding_generation: direct.binding_generation,
                    application_page_admission: direct.application_page_admission,
                })
            }
        };
    }
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
) -> Result<LoadGraphResult, crate::command_error::CommandError> {
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
                    return Err(crate::command_error::CommandError::graph(format!(
                        "couldn't create graph window: {error}"
                    )));
                }
            }
        }
        Ok(result)
    }
    #[cfg(not(desktop))]
    {
        let _ = (path, app, state);
        Err(crate::command_error::CommandError::prose(
            "multiple graph windows are desktop-only",
        ))
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
pub(crate) fn create_graph(dir: String) -> Result<String, crate::command_error::CommandError> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Err(crate::command_error::CommandError::prose(
            "no folder was chosen",
        ));
    }
    let base = Path::new(dir);
    if !base.is_dir() {
        return Err(crate::command_error::CommandError::graph(format!(
            "{dir} is not a folder"
        )));
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
        std::fs::create_dir(&cand).map_err(|e| {
            crate::command_error::CommandError::graph(format!("couldn't create folder: {e}"))
        })?;
        cand
    };
    tine_core::onboarding::create_demo_graph(&root).map_err(|e| {
        crate::command_error::CommandError::graph(format!("couldn't create the demo graph: {e}"))
    })?;
    Ok(root.display().to_string())
}

/// Compose and durably publish the recovery record for one Direct cross-page
/// move, BEFORE the frontend writes the first page (packet B2; I-3, I-2).
///
/// `destination` and `sources` are the POST-move DTOs — the exact DTOs the
/// choreography is about to save, in the order it will save them. The record
/// binds every participant's path, page identity, base revision, preimage bytes
/// and proposed postimage bytes, so recovery can complete the move OR roll it
/// back from the record alone.
///
/// Returns the record identifier, or `null` when no record was composed. `null`
/// is NOT a failure and never refuses the move: a degenerate move (every
/// participant resolves to one file) needs no record because the ordinary
/// base-revision guard already makes a single save safe, and an unavailable
/// app-private root leaves the move exactly as convergent as it was before B2
/// rather than making a page unmovable on a box whose data home is unwritable
/// (`docs/contracts/direct-move-recovery.md` §4 — preferring degraded recovery
/// to a refusal with no in-scope threat scenario).
#[tauri::command]
pub(crate) async fn begin_direct_cross_page_move(
    destination: tine_core::model::PageDto,
    sources: Vec<tine_core::model::PageDto>,
    state: crate::state::GraphContext<'_>,
) -> Result<Option<String>, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let app_state = app.state::<AppState>();
        let slot =
            crate::state::slot_for_bound_window(&app_state, &label, Some(binding_generation))
                .map_err(crate::command_error::CommandError::from)?;
        let graph = slot
            .legacy_graph()
            .map_err(crate::command_error::CommandError::from)?;
        let Some(store_root) = crate::backup::direct_move_recovery_dir(&app, &graph.root) else {
            crate::debug::diag(
                "Direct move recovery store unavailable; this cross-page move is unbracketed"
                    .to_string(),
            );
            return Ok(None);
        };
        let prepared = match graph.prepare_direct_cross_page_move(&destination, &sources) {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return Ok(None),
            Err(error) => {
                // Composition failed (an unreadable file, or the serializer's own
                // corruption firewall). The save about to run will report the real
                // error itself; do not turn a diagnostic into a refusal.
                crate::debug::diag(format!(
                    "Direct move record not composed; move proceeds unbracketed: {}",
                    error.kind()
                ));
                return Ok(None);
            }
        };
        let store = tine_core::direct_move_recovery::RecoveryStore::new(store_root);
        let move_id = prepared.record.move_id.clone();
        match store.commit_record(&prepared.record, &prepared.images) {
            Ok(()) => Ok(Some(move_id)),
            Err(error) => {
                crate::debug::diag(format!(
                    "Direct move record not published; move proceeds unbracketed: {}",
                    error.kind()
                ));
                Ok(None)
            }
        }
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

/// Retire a move record once every participant is durably terminal.
///
/// Retires ONLY when the whole move has landed. A record whose participants are
/// not all terminal is deliberately left in place: either a page save is still
/// conflicted — in which case the live conflict UI owns the decision and
/// recovery must not silently overwrite the user's page — or the process is
/// about to die, and the next open converges it. Returns whether the record was
/// retired.
#[tauri::command]
pub(crate) async fn finish_direct_cross_page_move(
    move_id: String,
    state: crate::state::GraphContext<'_>,
) -> Result<bool, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let app_state = app.state::<AppState>();
        let slot =
            crate::state::slot_for_bound_window(&app_state, &label, Some(binding_generation))
                .map_err(crate::command_error::CommandError::from)?;
        let graph = slot
            .legacy_graph()
            .map_err(crate::command_error::CommandError::from)?;
        let Some(store_root) = crate::backup::direct_move_recovery_dir(&app, &graph.root) else {
            return Ok(false);
        };
        Ok(tine_core::direct_move_recovery::retire_if_terminal(
            &store_root,
            &graph.root,
            &move_id,
        ))
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
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
pub(crate) fn default_graph_parent(
    app: tauri::AppHandle,
) -> Result<String, crate::command_error::CommandError> {
    let dir = app.path().app_data_dir().map_err(|e| {
        crate::command_error::CommandError::graph(format!("couldn't resolve app data dir: {e}"))
    })?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        crate::command_error::CommandError::graph(format!("couldn't create app data dir: {e}"))
    })?;
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
) -> Result<(), crate::command_error::CommandError> {
    let graph = slot
        .legacy_graph_cloned()
        .map_err(crate::command_error::CommandError::from)?;
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
) -> Result<bool, crate::command_error::CommandError> {
    Ok(slot_for_window(&state, window.label())
        .map_err(crate::command_error::CommandError::from)?
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

    fn assert_unsafe_provider_refusal(error: &crate::command_error::CommandError) {
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
            storage_supervisor: crate::storage_mode_supervisor::StorageModeSupervisor::default(),
            watch_ctl: std::sync::Mutex::new(None),
            last_focused: std::sync::Mutex::new(None),
            capture_graph: std::sync::Mutex::new(Default::default()),
            sync_runtime: crate::sync_runtime::SyncRuntimeFacade::default(),
            #[cfg(desktop)]
            next_window: std::sync::atomic::AtomicU64::new(1),
        }
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
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A `Graph` that reaches the window registry carries the Direct Files
    /// projection. Both producers of such a graph — the open and the
    /// configuration refresh — must attach through the ONE shared function;
    /// a second direct `attach_direct_projection(` call site in production is
    /// how the refresh path drifted into attaching nothing.
    #[test]
    fn every_direct_files_graph_is_attached_through_one_function() {
        /// Byte offsets of every `needle` that is NOT inside a `#[cfg(test)]`
        /// module. A module is open from its `#[cfg(test)]\nmod` header until
        /// the next closing brace at column 0; the files here use several such
        /// modules, not one trailing `mod tests`.
        fn production_sites(source: &str, needle: &str) -> Vec<usize> {
            // Spelled without a literal closing brace: `typedErrorRatchet.test.ts`
            // brace-counts this file to skip its test modules, and a closing brace inside
            // a string literal would end this module early for it.
            let close = format!("\n{}\n", char::from(0x7du8));
            let mut test_ranges = Vec::new();
            let mut from = 0;
            while let Some(found) = source[from..].find("\n#[cfg(test)]\nmod ") {
                let start = from + found;
                let end = source[start..]
                    .find(close.as_str())
                    .map(|at| start + at + close.len())
                    .unwrap_or(source.len());
                test_ranges.push(start..end);
                from = end;
            }
            source
                .match_indices(needle)
                .map(|(at, _)| at)
                .filter(|at| !test_ranges.iter().any(|range| range.contains(at)))
                .collect()
        }
        let graph = include_str!("graph.rs");
        assert_eq!(
            production_sites(graph, "attach_direct_projection(").len(),
            1,
            "graph.rs attaches the projection in `attach_direct_files_services` only"
        );
        for (name, source) in [
            ("state.rs", include_str!("state.rs")),
            ("commands.rs", include_str!("commands.rs")),
            ("backup.rs", include_str!("backup.rs")),
            ("watcher.rs", include_str!("watcher.rs")),
        ] {
            assert_eq!(
                production_sites(source, "attach_direct_projection(").len(),
                0,
                "{name} must attach through `attach_direct_files_services`"
            );
        }
        fn production(source: &str) -> &str {
            source
                .split("\n#[cfg(test)]\nmod tests")
                .next()
                .expect("production source")
        }
        let open = &graph[graph
            .find("pub(crate) fn prepare_direct_files_open")
            .expect("open path")..];
        assert!(
            open.find("attach_direct_files_services(") < open.find("Ok(PreparedDirectFilesOpen"),
            "the open path attaches before it publishes"
        );
        let state = production(include_str!("state.rs"));
        let refresh = &state[state
            .find("pub(crate) fn reopen_legacy_for_refresh")
            .expect("refresh core")..];
        let detach = refresh
            .find("detach_direct_projection(")
            .expect("old worker retired");
        let reopen = refresh
            .find("Graph::open_checked_with_assets(")
            .expect("refresh reopens");
        let attach = refresh
            .find("attach_direct_files_services(")
            .expect("refresh attaches");
        let slot = refresh
            .find("GraphSlot::refreshed(")
            .expect("refresh publishes");
        assert!(
            detach < reopen && reopen < attach && attach < slot,
            "refresh retires the old projection worker, reopens, attaches, then builds the slot"
        );
        let refresh_entry = &state[state
            .find("pub(crate) fn refresh_graph_for_label")
            .expect("refresh entry")..];
        assert!(
            refresh_entry.find("reopen_legacy_for_refresh(")
                < refresh_entry.find("warm_cache_async("),
            "the refreshed slot is warmed, or the projection never receives its payload"
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
            refuse_unclaimed_sparse_archive_with(&dir, |_| Err(
                crate::command_error::CommandError::prose("malformed")
            ))
            .unwrap_err(),
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
    fn graph_load_proposes_journal_renames_instead_of_performing_them() {
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
        let before = std::fs::read(dir.join("journals").join("Thursday, 25-06-2026.org")).unwrap();

        let loaded = open_graph_for_load(dir.to_str().unwrap(), None).unwrap();

        // Concord invariant 4: opening PROPOSES the rename and performs none.
        assert_eq!(
            loaded.graph.journal_filename_migrations(),
            vec![tine_core::model::JournalFilenameMigration {
                from: "journals/Thursday, 25-06-2026.org".into(),
                to: "journals/2026_06_25.org".into(),
            }],
            "the rename is offered"
        );
        assert!(
            dir.join("journals")
                .join("Thursday, 25-06-2026.org")
                .exists(),
            "opening a graph must not rename a journal file"
        );
        assert_eq!(
            std::fs::read(dir.join("journals").join("Thursday, 25-06-2026.org")).unwrap(),
            before,
            "opening a graph must not rewrite a journal file either"
        );
        assert!(!dir.join("journals").join("2026_06_25.org").exists());

        // The repair is still one call away, and it is the SAME selection.
        assert_eq!(loaded.graph.migrate_journal_filenames_checked().unwrap(), 1);
        assert!(dir.join("journals").join("2026_06_25.org").exists());
        assert!(loaded.graph.journal_filename_migrations().is_empty());

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
                < load.find("let prepared = match prepare_direct_files_open"),
            "partial provider evidence must be refused before a Direct Files binding is installed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_reopen_proves_pages_before_registry_publication() {
        let source = include_str!("graph.rs");
        let start = source
            .find("pub(crate) fn load_graph_for_label")
            .expect("graph load function");
        let managed = &source[start..];
        let readiness = managed
            .find("prove_managed_application_ready(&slot, None)")
            .expect("managed page readiness proof");
        let publication = managed
            .find("commit_if_current(managed_id")
            .expect("managed registry publication");
        assert!(readiness < publication);
    }

    #[test]
    fn managed_reopen_reports_nonserving_binding_before_page_readiness() {
        let source = include_str!("graph.rs");
        let start = source
            .find("pub(crate) fn load_graph_for_label")
            .expect("graph load function");
        let managed = &source[start..];
        let refusal = managed
            .find("serving_failure_detail()")
            .expect("typed managed-open refusal");
        let readiness = managed
            .find("prove_managed_application_ready(&slot, None)")
            .expect("managed page readiness proof");
        assert!(refusal < readiness);
    }

    #[test]
    fn pre_07_unknown_state_archives_then_rebuilds_without_manual_reactivation() {
        let source = include_str!("graph.rs");
        let start = source
            .find("pub(crate) fn load_graph_for_label")
            .expect("graph load function");
        let body = &source[start
            ..source[start..]
                .find("#[tauri::command]")
                .map(|offset| start + offset)
                .expect("end of graph load function")];
        let classification = body
            .find("unrecognized_binding_file")
            .expect("unrecognized private binding classification");
        let archive = body[classification..]
            .find("archive_unrecognized_private_state")
            .map(|offset| classification + offset)
            .expect("unrecognized private state archive");
        assert!(body[archive..].contains("blank_slate_archive_deferred"));
        assert!(body[archive..].contains("opening Direct Files"));
        let mut archive_paths = body;
        let mut checked_archive_paths = 0;
        while let Some(archive_offset) = archive_paths.find("archive_unrecognized_private_state") {
            let after_archive = &archive_paths[archive_offset..];
            let retry_offset = after_archive
                .find("rebuild_managed_after_direct = true")
                .expect("archive path must schedule automatic rebuild");
            assert!(
                !after_archive[..retry_offset].contains("return Err(error)"),
                "archive failure must fall through to Direct Files"
            );
            checked_archive_paths += 1;
            archive_paths = &after_archive[retry_offset + 1..];
        }
        assert_eq!(checked_archive_paths, 2);
        let durable_retry = body[archive..]
            .find("blank_slate_rebuild_pending")
            .map(|offset| archive + offset)
            .expect("durable automatic-rebuild retry intent");
        let direct = body[durable_retry..]
            .find("Direct Files publish")
            .map(|offset| durable_retry + offset)
            .expect("safe Markdown/Org source publication");
        let unlock = body[direct..]
            .find("drop(_load)")
            .map(|offset| direct + offset)
            .expect("graph-open lane release before activation");
        let rebuild = body[unlock..]
            .find("activate_sparse_v2_blocking")
            .map(|offset| unlock + offset)
            .expect("automatic managed rebuild");
        assert!(
            classification < archive
                && archive < durable_retry
                && durable_retry < direct
                && direct < unlock
                && unlock < rebuild
        );
        assert!(body[rebuild..].contains("Direct Files remains active"));
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
        let loaded = open_graph_for_load(dir.to_str().unwrap(), None).unwrap();
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
        let loaded = open_graph_for_load(no_v1.to_str().unwrap(), None).unwrap();
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
        let loaded = open_graph_for_load(v1_file.to_str().unwrap(), None).unwrap();
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
        let loaded = open_graph_for_load(dir.to_str().unwrap(), None).unwrap();
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
