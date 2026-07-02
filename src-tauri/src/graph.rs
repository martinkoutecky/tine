use crate::backup::backup_async;
use crate::state::AppState;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tine_core::model::{Graph, GraphMeta};

/// Reset the warm flag for a new graph load and return the new warm generation
/// (passed to `warm_cache_async`, which only reports done if still current).
pub(crate) fn begin_warm_cache(state: &AppState) -> u64 {
    state.warm_done.store(false, Ordering::Release);
    state.warm_generation.fetch_add(1, Ordering::AcqRel) + 1
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
    std::env::args().nth(1)
}

#[tauri::command]
pub(crate) fn load_graph(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<GraphMeta, String> {
    let root = resolve_root(&path).ok_or_else(|| {
        "no graph path provided (set TINE_GRAPH or pass a path)".to_string()
    })?;
    let graph = Graph::open(&root);
    let meta = graph.meta();
    // Recover any journals mis-saved under their title (see method docs).
    graph.migrate_journal_filenames();
    let warm_generation = begin_warm_cache(&state);
    *state.graph.write().unwrap() = Some(Arc::new(graph));
    // Nudge the watcher so it re-targets the new graph's dirs at once (in inotify
    // mode it's otherwise blocked on the old graph's events).
    if let Some(tx) = state.watch_ctl.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
    backup_async(app.clone());
    warm_cache_async(app, warm_generation);
    Ok(meta)
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

/// Build the search/backlinks cache off the hot path. We let the frontend's
/// first journal load grab the graph lock first, then warm in the background so
/// the first search is instant instead of re-parsing the whole tree. When the
/// warm completes (and this graph is still the current one — generation check),
/// flip `warm_done` and tell the frontend, which has been HOLDING its
/// whole-graph fetches (aliases, ref-count badges) so graph open never does
/// graph-sized work in the foreground.
pub(crate) fn warm_cache_async(app: tauri::AppHandle, warm_generation: u64) {
    std::thread::spawn(move || {
        // Brief delay so the first journal paint (which only needs a few pages)
        // grabs the lock first; then build the whole-graph cache in the
        // background so the first search / query / `g j` agenda doesn't pay for
        // parsing every file synchronously under the lock.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let state: State<'_, AppState> = app.state();
        if state.warm_generation.load(Ordering::Acquire) != warm_generation {
            return; // the graph was switched while we slept — a newer warm owns it
        }
        // Clone the Arc and drop the state lock BEFORE the (long) warm, so a
        // graph switch isn't blocked behind it.
        let graph = state.graph.read().unwrap().as_ref().cloned();
        if let Some(g) = graph {
            g.warm_cache();
            if state.warm_generation.load(Ordering::Acquire) == warm_generation {
                state.warm_done.store(true, Ordering::Release);
                let _ = app.emit("warm-cache-done", ());
            }
        }
    });
}

/// "Have the whole-graph derived caches finished warming for the current graph?"
/// Polled once by the frontend after it subscribes to `warm-cache-done`, closing
/// the boot race where the event fired before the listener mounted.
#[tauri::command]
pub(crate) fn warm_done(state: State<'_, AppState>) -> bool {
    state.warm_done.load(Ordering::Acquire)
}
