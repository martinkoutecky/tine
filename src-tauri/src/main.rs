// Prevent a console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tine_core::model::{Graph, GraphMeta, PageDto, PageEntry, PageKind, RefGroup};
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
fn load_graph(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<GraphMeta, String> {
    let root = resolve_root(&path).ok_or_else(|| {
        "no graph path provided (set TINE_GRAPH or pass a path)".to_string()
    })?;
    let graph = Graph::open(&root);
    let meta = graph.meta();
    *state.graph.lock().unwrap() = Some(graph);
    warm_cache_async(app);
    Ok(meta)
}

/// Build the search/backlinks cache off the hot path. We let the frontend's
/// first journal load grab the graph lock first, then warm in the background so
/// the first search is instant instead of re-parsing the whole tree.
fn warm_cache_async(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let state: State<'_, AppState> = app.state();
        let guard = state.graph.lock().unwrap();
        if let Some(g) = guard.as_ref() {
            g.warm_cache();
        }
    });
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
fn save_page(page: PageDto, force: Option<bool>, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| {
        let res = if force.unwrap_or(false) { g.force_save_page(&page) } else { g.save_page(&page) };
        res.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "conflict".to_string()
            } else {
                e.to_string()
            }
        })
    })
}

#[tauri::command]
fn get_backlinks(name: String, state: State<'_, AppState>) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.backlinks(&name)))
}

#[tauri::command]
fn get_unlinked_refs(name: String, state: State<'_, AppState>) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.unlinked_refs(&name)))
}

#[tauri::command]
fn delete_page(name: String, kind: PageKind, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.delete_page(&name, kind).map_err(|e| e.to_string()))
}

#[tauri::command]
fn publish_html(state: State<'_, AppState>) -> Result<(String, usize), String> {
    with_graph(&state, |g| g.publish_html().map_err(|e| e.to_string()))
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
fn list_templates(state: State<'_, AppState>) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    with_graph(&state, |g| Ok(g.templates()))
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
fn save_asset(name: String, bytes: Vec<u8>, state: State<'_, AppState>) -> Result<String, String> {
    with_graph(&state, |g| g.save_asset(&name, &bytes).map_err(|e| e.to_string()))
}

#[tauri::command]
fn read_highlights(
    pdf: String,
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    with_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.write_highlights(&pdf, &label, &highlights).map_err(|e| e.to_string())
    })
}

fn main() {
    // WebKitGTK's DMABUF renderer aborts on some GPU/compositor combos
    // ("Could not create default EGL display: EGL_BAD_PARAMETER"). We disable it
    // by default for a reliable launch, but that uses software compositing
    // (slower scrolling). Set TINE_GPU=1 to keep GPU/DMABUF rendering. Linux-only
    // (no effect on the macOS/Windows webviews).
    #[cfg(target_os = "linux")]
    if std::env::var("TINE_GPU").as_deref() != Ok("1")
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState { graph: Mutex::new(None) })
        .setup(|app| {
            // Eagerly open the graph if one was configured at startup.
            if let Some(root) = resolve_root("") {
                let g = Graph::open(&root);
                let meta = g.meta();
                let jdir = g.journals_path();
                let pdir = g.pages_path();
                let count_md = |d: &std::path::Path| {
                    std::fs::read_dir(d)
                        .map(|rd| {
                            rd.flatten()
                                .filter(|e| {
                                    e.path().extension().and_then(|x| x.to_str()) == Some("md")
                                })
                                .count()
                        })
                        .ok()
                };
                eprintln!("[tine] graph root: {}", meta.root);
                eprintln!(
                    "[tine] journals dir: {} (exists={}, .md files={:?})",
                    jdir.display(),
                    jdir.is_dir(),
                    count_md(&jdir)
                );
                eprintln!(
                    "[tine] pages dir: {} (exists={}, .md files={:?})",
                    pdir.display(),
                    pdir.is_dir(),
                    count_md(&pdir)
                );
                eprintln!(
                    "[tine] journals recognized as dates: {} | total page entries: {}",
                    g.journals_desc().len(),
                    g.list_pages().len()
                );
                if let Ok(rd) = std::fs::read_dir(&jdir) {
                    let sample: Vec<String> = rd
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| n.ends_with(".md"))
                        .take(3)
                        .collect();
                    eprintln!("[tine] sample journal files: {sample:?}");
                }
                let state: State<'_, AppState> = app.state();
                *state.graph.lock().unwrap() = Some(g);
            } else {
                eprintln!("[tine] NO graph root resolved — set TINE_GRAPH=/path/to/graph");
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
            get_unlinked_refs,
            delete_page,
            publish_html,
            run_query,
            search,
            quick_switch,
            list_templates,
            resolve_block,
            read_asset,
            import_asset,
            save_asset,
            read_highlights,
            write_highlights
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
