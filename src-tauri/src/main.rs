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
    if let Ok(p) = std::env::var("LOGSEQ_CLAUDE_GRAPH") {
        if !p.is_empty() {
            return Some(p);
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

fn main() {
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
            resolve_block
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
