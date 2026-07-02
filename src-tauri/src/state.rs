use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use tauri::State;
use tine_core::model::Graph;

// The graph lives behind an RwLock holding an Arc, so read commands clone the
// Arc and release the lock immediately — a long read (search / query / asset
// read) no longer serializes every other command behind it. Only replacing the
// graph (open / switch) takes the write lock.
pub(crate) struct AppState {
    pub(crate) graph: RwLock<Option<Arc<Graph>>>,
    // "The whole-graph derived caches are warm" — set by warm_cache_async when a
    // warm completes for the CURRENT graph, cleared (and the generation bumped) by
    // begin_warm_cache on every graph open/switch so a stale warm thread can't
    // report done for a graph that's been replaced. The frontend defers its
    // whole-graph fetches (aliases, block-ref counts) on this + the
    // `warm-cache-done` event (perf: graph open must not scale with graph size).
    pub(crate) warm_done: AtomicBool,
    pub(crate) warm_generation: AtomicU64,
    // Poke channel to the file-watcher thread: `load_graph` (graph switch) and
    // `set_watch_mode` send `()` so the watcher re-targets / switches mechanism
    // immediately instead of waiting for its next cycle. Set once by
    // `start_watcher`.
    pub(crate) watch_ctl: Mutex<Option<Sender<()>>>,
}

pub(crate) fn with_graph<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Graph) -> Result<T, String>,
) -> Result<T, String> {
    // Clone the Arc under a brief read lock, then run `f` with the lock released
    // so concurrent commands don't block each other.
    let graph = {
        let guard = state.graph.read().unwrap();
        guard.as_ref().ok_or("no graph loaded")?.clone()
    };
    f(&graph)
}

/// Re-open the graph so in-memory config (journal date formats, preferred format)
/// picks up a just-written `config.edn` change — the `journal_format` is built at
/// open and never mutated, so without this a format change leaves the backend
/// parsing journal names with the OLD format (mis-naming new journals). Also runs
/// the journal-filename migration, which now that the format is fresh can rename
/// any title-named journals (saved during the stale window) back to `yyyy_MM_dd`.
pub(crate) fn refresh_graph(state: &State<'_, AppState>) {
    let root = state.graph.read().unwrap().as_ref().map(|g| g.root.clone());
    if let Some(root) = root {
        let graph = Graph::open(&root);
        graph.migrate_journal_filenames();
        *state.graph.write().unwrap() = Some(Arc::new(graph));
    }
}
