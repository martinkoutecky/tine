use crate::backup::backup_async;
use crate::settings::{
    approved_external_assets, remember_external_assets_approval, remember_graph,
};
use crate::state::{canonical_graph_root, graph_meta, slot_for_window, AppState, GraphSlot};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tine_core::model::GraphMeta;
use tine_store::{OpenError, OpenOptions, Store};

pub(crate) fn open_error_text(error: OpenError, layout_prefix: bool) -> String {
    let text = error.to_string();
    if layout_prefix {
        format!("unsafe graph layout: {text}")
    } else {
        text
    }
}

/// Reset the warm flag for a new graph load and return the new warm generation
/// (passed to `warm_cache_async`, which only reports done if still current).
pub(crate) fn begin_warm_cache(slot: &GraphSlot) -> u64 {
    slot.begin_startup_warm()
}

/// Resolve the graph root: explicit path, else env var, else the graph named by
/// the launch command line (`cli::launch_request`; desktop only).
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
    #[cfg(desktop)]
    if let crate::cli::LaunchRequest::Open(path) = crate::cli::launch_request_env() {
        return Some(path.display().to_string());
    }
    None
}

/// A remembered path is optional startup state: a moved or deleted graph
/// sends the app to the picker instead of aborting Tauri setup.
pub(crate) fn usable_last_graph_path(path: Option<String>) -> Option<String> {
    path.filter(|root| Store::canonical_root(Path::new(root)).is_ok())
}

#[tauri::command]
pub(crate) fn startup_graph_path(app: tauri::AppHandle) -> Option<String> {
    resolve_root("").or_else(|| usable_last_graph_path(crate::settings::last_graph_path(&app)))
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

/// Complete this native show's read lease. A newer show returns None and
/// retains its own selection; an unbound cold launch errors and stays pending.
/// Cost O(open windows); the frontend reads the frozen lease, never reselects.
pub(crate) fn refresh_capture_graph_binding(
    state: &AppState,
    show_generation: u64,
) -> Result<Option<u64>, String> {
    let target = capture_target_for_state(state)?;
    let slot = slot_for_window(state, &target)?;
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
    store: Store,
    meta: GraphMeta,
}

/// One launch-owned result, consumed under `graph_load` before binding a slot.
/// Failed opens remain command errors, never setup errors. Dropping an unused
/// result drops its Store and cancels its worker; no thread can bind a window.
#[derive(Default)]
pub(crate) struct StartupGraph(
    std::sync::Mutex<
        Option<(
            std::path::PathBuf,
            std::thread::JoinHandle<Result<LoadedGraph, String>>,
        )>,
    >,
);

impl StartupGraph {
    fn begin(
        &self,
        root: std::path::PathBuf,
        approved: Option<std::path::PathBuf>,
        watch: tine_store::WatchMode,
    ) {
        let requested = root.clone();
        let worker = std::thread::Builder::new()
            .name("tine-startup-open".into())
            .spawn(move || {
                open_graph_for_load(&requested.display().to_string(), approved.as_deref(), watch)
            });
        match worker {
            Ok(worker) => *self.0.lock().unwrap() = Some((root, worker)),
            Err(_) => crate::debug::diag("startup-open worker unavailable; load command will open"),
        }
    }

    fn take(&self, root: &Path) -> Option<Result<LoadedGraph, String>> {
        let (requested, worker) = self.0.lock().unwrap().take()?;
        if requested != root {
            return None;
        }
        Some(
            worker
                .join()
                .unwrap_or_else(|_| Err("startup graph open panicked".into())),
        )
    }
}

/// Start read-only Store opening/warming while JS boots; the webview still owns
/// activation and error presentation through its ordinary load command (I-22).
pub(crate) fn prepare_startup_graph(app: &tauri::AppHandle) {
    let Some(root) = startup_graph_path(app.clone()) else {
        return;
    };
    let Ok(root) = canonical_graph_root(&root) else {
        return;
    };
    app.state::<StartupGraph>().begin(
        root.clone(),
        approved_external_assets(app, &root),
        crate::watcher::watch_mode(app),
    );
}

/// Open a graph without writing to it. Title-named journal files are proposed
/// for renaming in Settings, never renamed here (master e6f9b6e1ceae): a
/// rename at open lands as an unrequested change in a synced or git-kept graph.
fn open_graph_for_load(
    root: &str,
    approved_assets: Option<&Path>,
    watch: tine_store::WatchMode,
) -> Result<LoadedGraph, String> {
    let (store, meta, _) = Store::open(
        Path::new(root),
        OpenOptions {
            approved_external_assets: approved_assets.map(Path::to_path_buf),
            watch,
        },
    )
    .map_err(|error| open_error_text(error, true))?;
    Ok(LoadedGraph { store, meta })
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
    let inspection = Store::inspect(&root).map_err(|error| open_error_text(error, false))?;
    let approved = inspection.external_assets.is_none()
        || approved_external_assets(&app, &root)
            .is_some_and(|path| inspection.approves_external_assets(&path).unwrap_or(false));
    Ok(GraphAccessInspection {
        graph_root: root.display().to_string(),
        external_assets_path: inspection
            .external_assets
            .map(|path| path.display().to_string()),
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
    let inspection = Store::inspect(&root).map_err(|error| open_error_text(error, false))?;
    let live = inspection
        .external_assets
        .clone()
        .ok_or_else(|| "graph no longer uses an external assets directory".to_string())?;
    let matches = inspection
        .approves_external_assets(Path::new(&assets_path))
        .map_err(|error| format!("couldn't resolve external assets path: {error}"))?;
    if !matches {
        return Err(format!(
            "external assets directory changed before approval (now {})",
            live.display()
        ));
    }
    remember_external_assets_approval(&app, &root, &live)
}

#[tauri::command]
pub(crate) fn load_graph(
    path: String,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<LoadGraphResult, String> {
    let result = load_graph_for_label(path, &app, window.label(), &state)?;
    #[cfg(desktop)]
    if let LoadGraphResult::Loaded {
        binding_generation, ..
    }
    | LoadGraphResult::AlreadyCurrent {
        binding_generation, ..
    } = &result
    {
        crate::complete_pending_capture_show(&app, window.label().to_string(), *binding_generation);
    }
    Ok(result)
}

pub(crate) fn load_graph_for_label(
    path: String,
    app: &tauri::AppHandle,
    window_label: &str,
    state: &State<'_, AppState>,
) -> Result<LoadGraphResult, String> {
    let root = resolve_root(&path)
        .ok_or_else(|| "no graph path provided (set TINE_GRAPH or pass a path)".to_string())?;
    let root_key = canonical_graph_root(&root)?;
    let _load = state.graph_load.lock().unwrap();
    if let Some(owner) = state.graphs.read().unwrap().owner(&root_key) {
        if owner == window_label {
            let slot = slot_for_window(&state, &owner)?;
            return Ok(LoadGraphResult::AlreadyCurrent {
                meta: graph_meta(&slot),
                binding_generation: slot.binding_generation,
                config_problem: config_problem(&slot.store),
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
        return Ok(LoadGraphResult::FocusedExisting {
            window_label: owner,
        });
    }
    let root = root_key.display().to_string();
    let approved_assets = approved_external_assets(app, &root_key);
    let LoadedGraph { store, meta } = match app.state::<StartupGraph>().take(&root_key) {
        Some(result) => result,
        None => open_graph_for_load(
            &root,
            approved_assets.as_deref(),
            crate::watcher::watch_mode(app),
        ),
    }?;
    let slot = Arc::new(GraphSlot::new(store, root_key));
    let warm_generation = begin_warm_cache(&slot);
    state
        .graphs
        .write()
        .unwrap()
        .bind(window_label.to_string(), slot.clone())?;
    state.note_focused(window_label);
    crate::concord_ledger::attach(app.path().app_data_dir().ok(), &slot);
    crate::watcher::start_slot_events(app.clone(), window_label.to_string(), &slot);
    backup_async(app.clone(), slot.clone());
    // The graph is bound above; a settings write that fails (disk full, EIO)
    // must not return early, or `open_graph_window` skips its cleanup and the
    // graph stays owned by a window that never existed (master graph.rs:629).
    if remember_graph(app, &meta.root).is_err() {
        crate::debug::diag("remembering the opened graph in settings failed");
    }
    if let Some(window) = app.get_webview_window(window_label) {
        let name = Path::new(&meta.root)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Graph");
        let _ = window.set_title(&format!("Tine — {name}"));
    }
    let binding_generation = slot.binding_generation;
    let config_problem = config_problem(&slot.store);
    warm_cache_async(app.clone(), window_label.to_string(), slot, warm_generation);
    Ok(LoadGraphResult::Loaded {
        meta,
        binding_generation,
        config_problem,
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
pub(crate) struct ConfigProblem {
    kind: &'static str,
    message: String,
}
fn config_problem(store: &Store) -> Option<ConfigProblem> {
    store.config().problem.map(|error| ConfigProblem {
        kind: "config-read",
        message: error.to_string(),
    })
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LoadGraphResult {
    Loaded {
        meta: GraphMeta,
        binding_generation: u64,
        config_problem: Option<ConfigProblem>,
    },
    AlreadyCurrent {
        meta: GraphMeta,
        binding_generation: u64,
        config_problem: Option<ConfigProblem>,
    },
    FocusedExisting {
        window_label: String,
    },
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
    let root =
        tine_graph_features::guide::create_demo_graph(Path::new(dir)).map_err(
            |error| match error {
                OpenError::NotAFolder(_) => format!("{dir} is not a folder"),
                OpenError::CreateFailed { path, cause }
                    if path.parent() == Some(Path::new(dir))
                        && path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with("tine-demo")
                        }) =>
                {
                    format!("couldn't create folder: {}", cause.message)
                }
                OpenError::CreateFailed { cause, .. } | OpenError::Io(cause) => {
                    format!("couldn't create the demo graph: {}", cause.message)
                }
                other => format!("couldn't create the demo graph: {other}"),
            },
        )?;
    Ok(root.display().to_string())
}

/// This process's local clock: UTC offset in minutes and the instant sampled.
/// The backend's zone rules are the app's calendar authority (GH #607).
#[derive(serde::Serialize)]
pub(crate) struct LocalClock {
    offset_minutes: i32,
    unix_ms: i64,
}

#[tauri::command]
pub(crate) fn local_clock() -> LocalClock {
    let (offset_minutes, unix_ms) = tine_core::date::JournalDate::local_utc_offset_now();
    LocalClock {
        offset_minutes,
        unix_ms,
    }
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

/// Wait for Store::open's background parse off the hot path. Let the frontend's
/// first journal load a head start before this warm task waits for the
/// whole-graph cache. When the
/// warm completes (and this graph is still the current one — generation check),
/// flip `warm_done` and tell the frontend, which has been HOLDING its
/// whole-graph fetches (aliases, ref-count badges) so graph open never does
/// graph-sized work in the foreground.
pub(crate) fn warm_cache_async(
    app: tauri::AppHandle,
    window_label: String,
    slot: Arc<GraphSlot>,
    warm_generation: u64,
) {
    std::thread::spawn(move || {
        // Brief delay to reduce contention with the first journal paint.
        // Store::open owns the initial parse; whole_graph waits for its result.
        std::thread::sleep(std::time::Duration::from_millis(250));
        if slot.background_cancelled.load(Ordering::Acquire)
            || slot.warm_generation.load(Ordering::Acquire) != warm_generation
        {
            return; // the graph was switched while we slept — a newer warm owns it
        }
        // Serialize these post-open readiness waits process-wide. Store::open
        // may already have started a parse worker for each open slot. The lock
        // guards no data, so a warm that panicked while holding it must not
        // poison every later graph's warm.
        static WARM_WORK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let _worker = WARM_WORK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cancelled = || {
            slot.background_cancelled.load(Ordering::Acquire)
                || slot.warm_generation.load(Ordering::Acquire) != warm_generation
        };
        if cancelled() {
            return;
        }
        settle_launch_warm(
            || {
                // A failed whole-graph read still settles: the waiting reads
                // then take their ordinary (error-reporting) route.
                let _ = slot.store.whole_graph();
            },
            cancelled,
            || {
                let state: State<'_, AppState> = app.state();
                let current = state.graphs.read().unwrap().slot(&window_label);
                let still_current = current.as_ref().is_some_and(|current| {
                    current.binding_generation == slot.binding_generation
                        && current.root_key == slot.root_key
                });
                if still_current && current.unwrap().finish_startup_warm(warm_generation) {
                    let _ = app.emit_to(&window_label, "warm-cache-done", ());
                }
            },
        );
    });
}

/// Run a launch warm's `work` and then send its completion signal (`finish`)
/// exactly once, however the work ended: normally, with a failed read, or by
/// panicking. The frontend's alias and block-ref-count fetches wait for
/// `warm-cache-done` and nothing else ends that wait, so a silent end left them
/// empty for the session (master 39b88bd69, GH #543). Only a cancelled warm
/// (`cancelled()` true: graph switched or closed, a newer warm owns the window)
/// stays silent. O(1) beyond `work`.
fn settle_launch_warm(work: impl FnOnce(), cancelled: impl Fn() -> bool, finish: impl FnOnce()) {
    struct Settle<C: Fn() -> bool, F: FnOnce()> {
        cancelled: C,
        finish: Option<F>,
    }
    impl<C: Fn() -> bool, F: FnOnce()> Drop for Settle<C, F> {
        fn drop(&mut self) {
            if !(self.cancelled)() {
                if let Some(finish) = self.finish.take() {
                    finish();
                }
            }
        }
    }
    let _settle = Settle {
        cancelled,
        finish: Some(finish),
    };
    work();
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

    #[test]
    fn background_startup_result_is_owned_once_and_open_errors_reach_load() {
        let dir = scratch("startup-owned-result");
        let startup = StartupGraph::default();
        startup.begin(dir.clone(), None, Default::default());
        // Wait for completion before the webview requests its result: warming
        // starts at setup rather than at the first load command.
        while !startup.0.lock().unwrap().as_ref().unwrap().1.is_finished() {
            std::thread::yield_now();
        }
        let loaded = startup.take(&dir).unwrap().unwrap();
        assert_eq!(loaded.meta.root, dir.display().to_string());
        assert!(
            startup.take(&dir).is_none(),
            "launch result has exactly one owner"
        );
        drop(loaded);
        let missing = dir.join("missing");
        startup.begin(missing.clone(), None, Default::default());
        assert!(
            startup.take(&missing).unwrap().is_err(),
            "I-22: open errors reach the load command, never setup"
        );
        startup.begin(dir.clone(), None, Default::default());
        assert!(
            startup.take(&missing).is_none(),
            "a changed launch target cannot consume the old graph"
        );
        assert!(startup.take(&dir).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Master 39b88bd69 (GH #543): a launch warm that ended without being
    /// cancelled always sends its completion signal exactly once -- on success,
    /// after a failed read, and after a panic; a cancelled warm stays silent.
    #[test]
    fn a_launch_warm_that_ends_uncancelled_always_signals_completion() {
        let signalled = std::cell::Cell::new(0);
        settle_launch_warm(|| {}, || false, || signalled.set(signalled.get() + 1));
        assert_eq!(
            signalled.get(),
            1,
            "ended: not exactly one completion signal"
        );

        let signalled = std::sync::atomic::AtomicBool::new(false);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            settle_launch_warm(
                || panic!("warm panicked"),
                || false,
                || signalled.store(true, Ordering::Release),
            )
        }));
        assert!(panicked.is_err());
        assert!(
            signalled.load(Ordering::Acquire),
            "panicked: no completion signal"
        );

        let signalled = std::cell::Cell::new(false);
        settle_launch_warm(|| {}, || true, || signalled.set(true));
        assert!(
            !signalled.get(),
            "cancelled: a newer warm owns the window's signal"
        );
    }
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn moved_last_graph_reaches_a_canonical_root_error() {
        let missing =
            std::env::temp_dir().join(format!("tine-moved-last-graph-{}", std::process::id()));
        assert!(Store::canonical_root(&missing).is_err());
        assert_eq!(
            usable_last_graph_path(Some(missing.display().to_string())),
            None
        );
        let present = Path::new(env!("CARGO_MANIFEST_DIR")).display().to_string();
        assert_eq!(usable_last_graph_path(Some(present.clone())), Some(present));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tine-graph-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("journals")).unwrap();
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        dir
    }

    #[test]
    fn graph_load_proposes_journal_renames_instead_of_performing_them() {
        let dir = scratch("propose-journal-rename");
        std::fs::create_dir_all(dir.join("logseq")).unwrap();
        std::fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        let title_named = dir.join("journals").join("Thursday, 25-06-2026.org");
        std::fs::write(&title_named, "* original title-named journal\n").unwrap();

        let loaded = open_graph_for_load(dir.to_str().unwrap(), None, Default::default()).unwrap();

        assert_eq!(
            std::fs::read_to_string(&title_named).unwrap(),
            "* original title-named journal\n",
            "opening a graph must not rename journal files"
        );
        assert!(!dir.join("journals").join("2026_06_25.org").exists());
        assert_eq!(
            tine_graph_features::journals::journal_filename_migrations(&loaded.store).unwrap(),
            vec![tine_graph_features::journals::JournalFilenameMigration {
                from: "Thursday, 25-06-2026.org".into(),
                to: "2026_06_25.org".into(),
            }],
            "the rename is proposed instead"
        );
        drop(loaded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Master e6f9b6e1ceae: journal files are renamed only when the user applies
    /// the Settings proposal, after a snapshot. A second caller (graph open, a
    /// journal-format change) renames the user's files unasked. Exemplar:
    /// `commands.rs::apply_journal_filename_migrations`.
    #[test]
    fn only_the_applied_proposal_renames_journal_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut callers = Vec::new();
        for dir in ["src-tauri/src", "crates/tine-graph-features/src"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                let path = entry.unwrap().path();
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                // A call, not the definition or this census's own string literals.
                let calls = text.lines().filter(|line| {
                    line.contains("migrate_journal_filenames(")
                        && !line.contains("fn migrate_journal_filenames(")
                        && !line.contains('"')
                });
                if calls.count() > 0 {
                    callers.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
        }
        assert_eq!(
            callers,
            ["commands.rs"],
            "only apply_journal_filename_migrations may rename journal files (master e6f9b6e1ceae)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_error_adapter_keeps_legacy_layout_text() {
        let dir = scratch("layout-error-text");
        let outside = scratch("layout-outside");
        std::fs::remove_dir(dir.join("pages")).unwrap();
        std::os::unix::fs::symlink(outside.join("pages"), dir.join("pages")).unwrap();
        let new = open_graph_for_load(dir.to_str().unwrap(), None, Default::default())
            .err()
            .unwrap();
        assert_eq!(
            new,
            "unsafe graph layout: pages directory escapes graph root: \"pages\""
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(outside);
    }
}
