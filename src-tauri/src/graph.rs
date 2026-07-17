use crate::backup::{backup_async, backup_graph_now};
use crate::settings::{
    approved_external_assets, remember_external_assets_approval, remember_graph,
};
use crate::state::{canonical_graph_root, poke_watcher, slot_for_window, AppState, GraphSlot};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tine_core::model::{Graph, GraphMeta};

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

#[tauri::command]
pub(crate) fn startup_graph_path(app: tauri::AppHandle) -> Option<String> {
    resolve_root("").or_else(|| crate::settings::last_graph_path(&app))
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
        graph.migrate_journal_filenames();
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
pub(crate) fn load_graph(
    path: String,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<LoadGraphResult, String> {
    load_graph_for_label(path, &app, window.label(), &state)
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
                meta: slot.graph.meta(),
                binding_generation: slot.binding_generation,
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
    let LoadedGraph {
        graph,
        meta,
        launch_backup_done,
    } = open_graph_for_load(&root, approved_assets.as_deref(), |graph| {
        backup_graph_now(app, graph, "")
    })?;
    let slot = Arc::new(GraphSlot::new(graph, root_key));
    let warm_generation = begin_warm_cache(&slot);
    state
        .graphs
        .write()
        .unwrap()
        .bind(window_label.to_string(), slot.clone())?;
    state.note_focused(window_label);
    poke_watcher(&state);
    if !launch_backup_done {
        backup_async(app.clone(), slot.clone());
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
    warm_cache_async(app.clone(), window_label.to_string(), slot, warm_generation);
    Ok(LoadGraphResult::Loaded {
        meta,
        binding_generation,
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
    },
    AlreadyCurrent {
        meta: GraphMeta,
        binding_generation: u64,
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
) {
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
        let completed = slot.graph.warm_cache_cancellable(|| {
            slot.background_cancelled.load(Ordering::Acquire)
                || slot.warm_generation.load(Ordering::Acquire) != warm_generation
        });
        if !completed {
            return;
        }
        let state: State<'_, AppState> = app.state();
        let current = state
            .graphs
            .read()
            .unwrap()
            .slot(&window_label);
        let still_current = current.as_ref().is_some_and(|current| {
            current.binding_generation == slot.binding_generation && current.root_key == slot.root_key
        });
        if still_current && slot.warm_generation.load(Ordering::Acquire) == warm_generation {
            current.unwrap().warm_done.store(true, Ordering::Release);
            let _ = app.emit_to(&window_label, "warm-cache-done", ());
        }
    });
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
}
