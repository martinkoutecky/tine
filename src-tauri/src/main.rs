// Prevent a console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use logseq_core::model::{Graph, GraphMeta, PageDto, PageEntry, PageKind, RefGroup};
use std::sync::Mutex;
use tauri::{Manager, State};

struct AppState {
    graph: Mutex<Option<Graph>>,
}

/// Resolve the graph root: explicit path, else env var, else first CLI arg.
fn resolve_root(path: &str) -> Option<String> {
    if !path.is_empty() {
        return Some(path.to_string());
    }
    for var in ["TINE_GRAPH", "LOGSEQ_CLAUDE_GRAPH"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    std::env::args().nth(1)
}

#[tauri::command]
fn load_graph(path: String, state: State<'_, AppState>) -> Result<GraphMeta, String> {
    let root = resolve_root(&path).ok_or_else(|| {
        "no graph path provided (set LOGSEQ_CLAUDE_GRAPH or pass a path)".to_string()
    })?;
    let graph = Graph::open(&root);
    let meta = graph.meta();
    *state.graph.lock().unwrap() = Some(graph);
    Ok(meta)
}

fn with_graph<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Graph) -> Result<T, String>,
) -> Result<T, String> {
    let guard = state.graph.lock().unwrap();
    let graph = guard.as_ref().ok_or("no graph loaded")?;
    f(graph)
}

#[tauri::command]
fn list_pages(state: State<'_, AppState>) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.list_pages()))
}

#[tauri::command]
fn journals_desc(
    limit: usize,
    offset: usize,
    state: State<'_, AppState>,
) -> Result<Vec<PageDto>, String> {
    with_graph(&state, |g| {
        let entries = g.journals_desc();
        let mut out = Vec::new();
        for e in entries.into_iter().skip(offset).take(limit) {
            out.push(g.load_page(&e).map_err(|err| err.to_string())?);
        }
        Ok(out)
    })
}

#[tauri::command]
fn get_page(
    name: String,
    kind: PageKind,
    state: State<'_, AppState>,
) -> Result<Option<PageDto>, String> {
    with_graph(&state, |g| g.load_named(&name, kind).map_err(|e| e.to_string()))
}

#[tauri::command]
fn save_page(page: PageDto, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.save_page(&page).map_err(|e| e.to_string()))
}

#[tauri::command]
fn get_backlinks(name: String, state: State<'_, AppState>) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.backlinks(&name)))
}

#[tauri::command]
fn run_query(query: String, state: State<'_, AppState>) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.run_query(&query)))
}

#[tauri::command]
fn search(query: String, limit: usize, state: State<'_, AppState>) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.search(&query, limit)))
}

#[tauri::command]
fn quick_switch(
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.quick_switch(&query, limit)))
}

#[tauri::command]
fn resolve_block(uuid: String, state: State<'_, AppState>) -> Result<Option<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.resolve_block(&uuid)))
}

#[tauri::command]
fn read_asset(name: String, state: State<'_, AppState>) -> Result<Vec<u8>, String> {
    with_graph(&state, |g| g.read_asset(&name).map_err(|e| e.to_string()))
}

#[tauri::command]
fn import_asset(path: String, state: State<'_, AppState>) -> Result<String, String> {
    with_graph(&state, |g| {
        g.import_asset(std::path::Path::new(&path)).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn read_highlights(
    pdf: String,
    state: State<'_, AppState>,
) -> Result<Vec<logseq_core::pdf::Highlight>, String> {
    with_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<logseq_core::pdf::Highlight>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.write_highlights(&pdf, &label, &highlights).map_err(|e| e.to_string())
    })
}

fn main() {
    // WebKitGTK's DMABUF renderer aborts on many GPU/compositor combos
    // (KDE/Wayland, some Mesa/NVIDIA): "Could not create default EGL display:
    // EGL_BAD_PARAMETER". Force the stable path unless the user overrides it.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .manage(AppState { graph: Mutex::new(None) })
        .setup(|app| {
            // Eagerly open the graph if one was configured at startup.
            if let Some(root) = resolve_root("") {
                let state: State<'_, AppState> = app.state();
                *state.graph.lock().unwrap() = Some(Graph::open(&root));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_graph,
            list_pages,
            journals_desc,
            get_page,
            save_page,
            get_backlinks,
            run_query,
            search,
            quick_switch,
            resolve_block,
            read_asset,
            import_asset,
            read_highlights,
            write_highlights
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
