#[cfg(desktop)]
use crate::debug::diag;
#[cfg(desktop)]
use crate::platform::{open_page_source, opener_command, reveal_page_source};
use crate::state::{refresh_graph, slot_for_context, with_graph, GraphContext};
use std::sync::Arc;
use tine_core::model::{PageDto, PageEntry, PageKind, RefGroup};

/// Write a PNG image to the OS clipboard. The lightbox encodes the shown image to
/// PNG and sends the bytes. On Linux we prefer `wl-copy`/`xclip` (see above) and
/// fall back to the Tauri clipboard plugin; elsewhere the plugin is reliable.
/// Decode a base64 asset payload. The frontend sends bytes as one base64 string
/// rather than a JSON number[] (which inflated the IPC payload ~4-5x and forced a
/// per-element parse + a giant throwaway array on the webview thread).
pub(crate) fn decode_asset_b64(b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("bad base64 asset payload: {e}"))
}

#[tauri::command]
pub(crate) fn list_pages(state: GraphContext<'_>) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.list_pages()))
}

#[tauri::command]
pub(crate) fn journals_desc(
    limit: usize,
    offset: usize,
    state: GraphContext<'_>,
) -> Result<Vec<PageDto>, String> {
    with_graph(&state, |g| {
        let entries = g.journals_desc();
        let mut out = Vec::new();
        for e in entries.into_iter().skip(offset).take(limit) {
            match g.load_page(&e) {
                Ok(dto) => out.push(dto),
                // A journal deleted from disk between the cache listing and this
                // load just drops out of the feed — don't fail the whole batch (and
                // don't serve a stale ghost; load_page already evicted it).
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.to_string()),
            }
        }
        Ok(out)
    })
}

#[tauri::command]
pub(crate) fn get_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, String> {
    with_graph(&state, |g| {
        g.load_named(&name, kind).map_err(|e| e.to_string())
    })
}

/// One raw source file of the open graph, for the in-app lsdoc↔mldoc diff panel.
#[derive(serde::Serialize)]
pub(crate) struct GraphSourceFile {
    /// graph-root-relative, forward-slashed path (stable id shown in the report)
    rel: String,
    /// the file's raw UTF-8 text (fed to both parsers exactly as on disk)
    text: String,
    /// "md" | "org" — selects the parser grammar
    format: String,
    bytes: u64,
}

/// Raw text of every Markdown/Org file in the open graph (`pages/`, plus
/// `journals/` when `include_journals`), for the "Help improve Tine" diff panel.
/// Mirrors `lsdoc/tools/graph-check.mjs`'s file scan: skips files over 8 MB, tags
/// format by extension, returns graph-root-relative paths sorted for stable
/// output. Read-only and local — the panel makes no network calls.
#[tauri::command]
pub(crate) fn graph_source_files(
    include_journals: bool,
    state: GraphContext<'_>,
) -> Result<Vec<GraphSourceFile>, String> {
    const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
    with_graph(&state, |g| {
        let mut out: Vec<GraphSourceFile> = Vec::new();
        let mut roots = vec![g.pages_path()];
        if include_journals {
            roots.push(g.journals_path());
        }
        for root in roots {
            collect_graph_text(g, &root, MAX_FILE_BYTES, &mut out);
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    })
}

fn collect_graph_text(
    g: &tine_core::model::Graph,
    dir: &std::path::Path,
    max_bytes: u64,
    out: &mut Vec<GraphSourceFile>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_graph_text(g, &p, max_bytes, out);
            continue;
        }
        let format = match p.extension().and_then(|x| x.to_str()) {
            Some("md") => "md",
            Some("org") => "org",
            _ => continue,
        };
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if meta.len() > max_bytes {
            continue; // oversized files skipped, like graph-check
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue; // non-UTF-8 / unreadable file skipped
        };
        out.push(GraphSourceFile {
            rel: g.rel_path(&p),
            text,
            format: format.to_string(),
            bytes: meta.len(),
        });
    }
}

#[tauri::command]
pub(crate) fn save_page(
    page: PageDto,
    base_rev: Option<String>,
    force: Option<bool>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        let res = if force.unwrap_or(false) {
            g.force_save_page(&page)
        } else {
            g.save_page(&page, base_rev.as_deref())
        };
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
pub(crate) fn guide_pages() -> Vec<tine_core::onboarding::GuidePage> {
    tine_core::onboarding::bundled_guide_pages()
}

#[tauri::command]
pub(crate) fn copy_guide_into_graph(
    title: String,
    state: GraphContext<'_>,
) -> Result<tine_core::onboarding::GuideCopyResult, String> {
    let result: Result<tine_core::onboarding::GuideCopyResult, String> = with_graph(&state, |g| {
        tine_core::onboarding::copy_guide_into_graph(g, &title).map_err(|e| e.to_string())
    });
    result
}

#[tauri::command]
pub(crate) fn get_backlinks(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.backlinks(&name)))
}

#[tauri::command]
pub(crate) fn get_unlinked_refs(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.unlinked_refs(&name)))
}

/// `block uuid → # of referrers` over the whole graph (drives the per-block
/// reference-count badge). Small map (only referenced uuids); fetched once per
/// graph generation by the frontend.
#[tauri::command]
pub(crate) fn block_ref_counts(
    state: GraphContext<'_>,
) -> Result<Arc<std::collections::HashMap<String, usize>>, String> {
    with_graph(&state, |g| Ok(g.block_ref_counts()))
}

/// The blocks that reference block `uuid`, grouped by page (the badge's referrers
/// panel). Lazy: called only when a badge is clicked open.
#[tauri::command]
pub(crate) fn block_referrers(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.block_referrers(&uuid)))
}

#[tauri::command]
pub(crate) fn delete_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.delete_page(&name, kind).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn rename_page(old: String, new: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.rename_page(&old, &new).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn publish_html(state: GraphContext<'_>) -> Result<(String, usize), String> {
    with_graph(&state, |g| g.publish_html().map_err(|e| e.to_string()))
}

/// Render one page to a self-contained HTML document (assets inlined, no sidebar)
/// for the print-to-PDF export, with the dialog's options. `Err("no-page")` if the
/// page doesn't exist.
#[tauri::command]
pub(crate) fn page_print_html(
    name: String,
    opts: tine_core::publish::PrintOpts,
    state: GraphContext<'_>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        g.page_print_html(&name, opts)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no-page".to_string())
    })
}

#[tauri::command]
pub(crate) fn run_query(
    query: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.run_query(&query)))
}

#[tauri::command]
pub(crate) async fn run_graph_search(
    source: String,
    page_limit: usize,
    block_limit: usize,
    lane: Option<String>,
    explain: bool,
    state: GraphContext<'_>,
) -> Result<tine_core::query_plan::QueryExecution, String> {
    let graph = Arc::clone(&slot_for_context(&state)?.graph);
    tauri::async_runtime::spawn_blocking(move || match lane.as_deref() {
        Some(lane) => graph.run_graph_search_latest(lane, &source, page_limit, block_limit, explain),
        None => graph.run_graph_search(&source, page_limit, block_limit, explain),
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::AdvancedResult, String> {
    with_graph(&state, |g| {
        Ok(g.run_advanced_query(&query, current_page.as_deref()))
    })
}

#[tauri::command]
pub(crate) fn query_facets(state: GraphContext<'_>) -> Result<Vec<(String, Vec<String>)>, String> {
    with_graph(&state, |g| Ok(g.property_facets()))
}

#[tauri::command]
pub(crate) fn page_aliases(state: GraphContext<'_>) -> Result<Vec<(String, String)>, String> {
    with_graph(&state, |g| Ok(g.page_aliases()))
}

#[tauri::command]
pub(crate) fn page_icons(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<std::collections::HashMap<String, String>, String> {
    with_graph(&state, |g| Ok(g.page_icons(&names)))
}

#[tauri::command]
pub(crate) fn set_favorites(names: Vec<String>, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_favorites(&names).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_preferred_workflow(
    workflow: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_preferred_workflow(&workflow)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_timetracking_enabled(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_timetracking_enabled(enabled)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_guide_announced(announced: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_guide_announced(announced).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_default_journal_template(
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_default_journal_template(name.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_start_of_week(n: u32, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_start_of_week(n).map_err(|e| e.to_string())
    })
}

/// Set the graph's `:preferred-format` for new pages/journals ("md" or "org").
#[tauri::command]
pub(crate) fn set_preferred_format(format: String, state: GraphContext<'_>) -> Result<(), String> {
    let fmt = if format.eq_ignore_ascii_case("org") {
        tine_core::model::Format::Org
    } else {
        tine_core::model::Format::Md
    };
    with_graph(&state, |g| {
        g.set_preferred_format(fmt).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?; // so new pages/journals use the new extension immediately
    Ok(())
}

/// Set the graph's `:journal/page-title-format` (journal display-title format,
/// e.g. "MMM do, yyyy"). Display-only — does not rename journal files.
#[tauri::command]
pub(crate) fn set_journal_title_format(
    format: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_journal_page_title_format(&format)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?; // pick up the new format + migrate any title-named journals
    Ok(())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: GraphContext<'_>) -> Result<String, String> {
    with_graph(&state, |g| Ok(g.custom_css()))
}

#[tauri::command]
pub(crate) async fn search(
    query: String,
    limit: usize,
    lane: Option<String>,
    state: GraphContext<'_>,
) -> Result<Vec<RefGroup>, String> {
    let graph = Arc::clone(&slot_for_context(&state)?.graph);
    tauri::async_runtime::spawn_blocking(move || match lane.as_deref() {
        Some(lane) => graph.search_latest(lane, &query, limit),
        None => graph.search(&query, limit),
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn quick_switch(
    query: String,
    limit: usize,
    state: GraphContext<'_>,
) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.quick_switch(&query, limit)))
}

#[tauri::command]
pub(crate) fn list_templates(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    with_graph(&state, |g| Ok(g.templates()))
}

#[tauri::command]
pub(crate) fn journal_content_days(state: GraphContext<'_>) -> Result<Vec<i64>, String> {
    with_graph(&state, |g| Ok(g.journal_content_days()))
}

#[tauri::command]
pub(crate) fn resolve_block(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Option<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.resolve_block(&uuid)))
}

#[tauri::command]
pub(crate) fn resolve_blocks(
    uuids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<Option<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.resolve_blocks(&uuids)))
}

#[tauri::command]
pub(crate) fn read_asset(
    name: String,
    max_bytes: Option<u64>,
    state: GraphContext<'_>,
) -> Result<tauri::ipc::Response, String> {
    // Return RAW bytes (not a JSON number[]), so a multi-MB PDF/image isn't
    // serialized element-by-element and re-parsed on the JS side — the frontend
    // receives an ArrayBuffer directly.
    with_graph(&state, |g| {
        max_bytes
            .map_or_else(
                || g.read_asset(&name),
                |limit| g.read_asset_limited(&name, limit),
            )
            .map(tauri::ipc::Response::new)
            .map_err(|e| e.to_string())
    })
}

/// Validate one graph media file and return its top-level asset name for the
/// range-aware `tine-media:` protocol. The protocol revalidates against the
/// requesting window's current graph on every request.
#[tauri::command]
pub(crate) fn stream_asset_path(
    name: String,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    slot.graph
        .stream_asset_path(&name)
        .map_err(|e| e.to_string())?;
    Ok(format!("{}/{}", slot.binding_generation, name))
}

/// Quit the app cleanly. On Linux, first SIGKILL WebKitGTK's helper subprocesses so
/// they don't run their buggy GL-driver atexit teardown and dump a SIGABRT core on
/// exit (GH #28). The JS close handler calls this only AFTER `flushAll()`/
/// `flushSession()` have resolved, so tearing the web process down hard loses no
/// edits. Then hand off to Tauri's normal exit (the main process still tears down
/// the way it always has — no dump there). On non-Linux this is just `app.exit(0)`.
#[tauri::command]
pub(crate) fn tine_quit(app: tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    crate::platform::kill_webkit_children();
    app.exit(0);
}

/// Close only the calling graph window. The final graph window still performs
/// the process-wide WebKit cleanup before exit; the hidden capture window never
/// keeps the process alive by itself.
#[tauri::command]
pub(crate) fn close_graph_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    if state.graphs.read().unwrap().len() <= 1 {
        #[cfg(target_os = "linux")]
        crate::platform::kill_webkit_children();
        app.exit(0);
        return Ok(());
    }
    window.destroy().map_err(|e| e.to_string())
}

/// Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
/// debugging (GH #31). `open_devtools`/`close_devtools` are compiled in because
/// we enable tauri's `devtools` feature unconditionally (see Cargo.toml) — so
/// this works in shipped release builds, not just debug.
#[tauri::command]
pub(crate) fn tine_open_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        // #31 follow-up: on X11/XWayland, open the inspector as its OWN window
        // instead of docked into the app. Docked, WebKitGTK puts the window's resize
        // grip at the top of the inspector pane. Do not force this on native Wayland:
        // Fedora 44 / WebKitGTK 2.52 renders the detached inspector black, while its
        // docked inspector is correctly scaled and usable. Query the actual GDK
        // display rather than session environment variables because an AppImage in a
        // Wayland session deliberately runs GTK through XWayland.
        // WebKit creates/attaches the inspector asynchronously, so an immediate
        // is_attached()+detach() races and usually does nothing. Arm a one-shot hook
        // BEFORE opening instead. The attach signal is the event boundary; its idle
        // continuation runs after WebKit's default attach handler has finished, then
        // detaches. There is deliberately no guessed timeout. Disconnecting first
        // also lets the user attach the already-open inspector manually afterward.
        #[cfg(target_os = "linux")]
        {
            let _ = window.with_webview(|wv| {
                use std::{cell::RefCell, rc::Rc};
                use gtk::{gdk::prelude::DisplayExtManual, prelude::WidgetExt};
                use webkit2gtk::{glib, glib::prelude::ObjectExt, WebInspectorExt, WebViewExt};
                if wv.inner().display().backend().is_wayland() {
                    return;
                }
                if let Some(inspector) = wv.inner().inspector() {
                    let handler_slot = Rc::new(RefCell::new(None));
                    let callback_slot = Rc::clone(&handler_slot);
                    let handler_id = inspector.connect_attach(move |inspector| {
                        if let Some(handler_id) = callback_slot.borrow_mut().take() {
                            inspector.disconnect(handler_id);
                        }
                        let inspector = inspector.clone();
                        glib::idle_add_local_once(move || {
                            if inspector.is_attached() {
                                inspector.detach();
                            }
                        });
                        false
                    });
                    *handler_slot.borrow_mut() = Some(handler_id);
                }
            });
        }
        // Tauri queues UI-thread messages in order: with_webview installs the
        // hook above before this open request is dispatched.
        window.open_devtools();
    }
}

#[tauri::command]
pub(crate) fn read_local_image(
    path: String,
    app: tauri::AppHandle,
) -> Result<tauri::ipc::Response, String> {
    // Read an image from an ABSOLUTE path OUTSIDE the graph, for raw-HTML `<img>`
    // srcs the user has explicitly opted into (Settings → "Load local-file images").
    // OFF by default; gated here too (defense in depth — the frontend also checks),
    // restricted to image extensions + a size cap so an allowed note can't slurp an
    // arbitrary file. Returns RAW bytes like `read_asset`. See ADR 0019.
    if !crate::settings::get_app_bool("allow_local_file_images".into(), false, app) {
        return Err("local-file images are disabled".into());
    }
    let p = std::path::Path::new(&path);
    let ext_ok = matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "avif" | "apng")
    );
    if !ext_ok {
        return Err("not an image file".into());
    }
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err("image too large".into());
    }
    std::fs::read(p)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn import_asset(
    path: String,
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        g.import_asset(std::path::Path::new(&path), name.as_deref())
            .map_err(|e| e.to_string())
    })
}

/// Read a dropped delimited-text file for the CSV/TSV → grid drop path.
/// Deliberately NARROW: this is the only webview-reachable read of a
/// caller-chosen path (everything else is gated to the graph/assets dirs),
/// so it refuses anything that isn't the drop feature's file types — it must
/// not grow into a general file-read primitive.
#[tauri::command]
pub(crate) fn read_text_file(path: String) -> Result<String, String> {
    fn delimited_ext(p: &std::path::Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("csv") || e.eq_ignore_ascii_case("tsv"))
            .unwrap_or(false)
    }
    let p = std::path::Path::new(&path);
    if !delimited_ext(p) {
        return Err("unsupported file type".into());
    }
    // Re-check on the RESOLVED path too — a symlink named x.csv pointing at an
    // arbitrary file must not pass the extension gate (review finding).
    let resolved = std::fs::canonicalize(p).map_err(|e| e.to_string())?;
    if !delimited_ext(&resolved) {
        return Err("unsupported file type".into());
    }
    let meta = std::fs::metadata(&resolved).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err("text file too large".into());
    }
    std::fs::read_to_string(&resolved).map_err(|e| e.to_string())
}

/// Open a graph asset (by its `assets/`-relative name) in the OS default app,
/// e.g. a video/audio file in the system player. Path-gated to the assets dir
/// (canonicalized) so a crafted name can't open a file outside the graph.
#[tauri::command]
pub(crate) fn open_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    let target = with_graph(&state, |g| g.asset_file_for_read(&name).map_err(|e| e.to_string()))?;
    #[cfg(desktop)]
    {
        #[cfg(target_os = "linux")]
        let prog = "xdg-open";
        #[cfg(target_os = "macos")]
        let prog = "open";
        #[cfg(target_os = "windows")]
        let prog = "explorer";
        diag(format!(
            "open_asset: {name} -> {} ({prog})",
            target.display()
        ));
        opener_command(prog)
            .arg(&target)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    // Mobile: opening an asset in an external app uses a platform intent; stub for now (M1).
    #[cfg(not(desktop))]
    {
        let _ = (&name, &target);
        Err("open asset externally is not supported on this platform".into())
    }
}

/// Open or reveal the exact source file recorded on a loaded page. Rust resolves
/// and canonicalizes the graph-relative identity; the WebView never supplies an
/// arbitrary absolute path.
#[tauri::command]
pub(crate) fn open_page_file(
    name: String,
    kind: PageKind,
    path: Option<String>,
    reveal: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let target = with_graph(&state, |graph| {
        graph
            .page_source_file(&name, kind, path.as_deref())
            .map_err(|error| error.to_string())
    })?;
    #[cfg(desktop)]
    {
        if reveal {
            reveal_page_source(&target)
        } else {
            open_page_source(&target)
        }
    }
    #[cfg(not(desktop))]
    {
        let _ = (target, reveal);
        Err("page file actions are available on desktop only".into())
    }
}

/// Open a graph asset in a SPECIFIC external editor (drawio/Excalidraw/…) so a
/// diagram can be edited in place. `command` is the user-configured command
/// template for that editor (from Settings → Files); empty falls back to the OS
/// opener, exactly like `open_asset`. The template is tokenised on whitespace:
/// token[0] is the program, a `{}` inside any token is replaced by the asset
/// path, and if no argument contains `{}` the path is appended as the final arg.
/// Spawned as an argv (no shell → no injection) through `opener_command`, which
/// scrubs the WebKitGTK/AppImage env and detaches the child (so a Flatpak drawio
/// doesn't inherit Tine's bundled `LD_LIBRARY_PATH`). Path-gated to `assets/`.
/// Double quotes group a program/argument containing whitespace; backslashes are
/// literal so Windows paths such as `"C:\Program Files\draw.io\draw.io.exe" {}`
/// survive unchanged.
#[tauri::command]
pub(crate) fn edit_asset_external(
    name: String,
    command: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let target = with_graph(&state, |g| g.asset_file_for_read(&name).map_err(|e| e.to_string()))?;
    #[cfg(desktop)]
    {
        let target_str = target.to_string_lossy().to_string();
        let trimmed = command.trim();
        if trimmed.is_empty() {
            // No editor configured → same OS opener as open_asset.
            #[cfg(target_os = "linux")]
            let prog = "xdg-open";
            #[cfg(target_os = "macos")]
            let prog = "open";
            #[cfg(target_os = "windows")]
            let prog = "explorer";
            diag(format!(
                "edit_asset_external: {name} -> {target_str} (opener {prog})"
            ));
            opener_command(prog)
                .arg(&target)
                .spawn()
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        let (prog, args) = build_editor_argv(trimmed, &target_str)?;
        diag(format!("edit_asset_external: {name} -> {prog} {args:?}"));
        opener_command(&prog)
            .args(&args)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = (&name, &command, &target);
        Err("editing an asset externally is not supported on this platform".into())
    }
}

/// Best-effort autodetect of an installed external editor's launch command, by
/// PROBING known install locations on disk — never executing anything (so a
/// Flatpak wrapper can't leak its bundled env into the probe). Returns a command
/// template suitable for `edit_asset_external`, or an empty string if not found
/// (the caller then leaves the setting empty = OS opener). Currently knows
/// `drawio`; other ids return empty.
#[tauri::command]
pub(crate) fn detect_media_editor(id: String) -> Result<String, String> {
    #[cfg(desktop)]
    {
        if id == "drawio" {
            return Ok(detect_drawio());
        }
        Ok(String::new())
    }
    #[cfg(not(desktop))]
    {
        let _ = id;
        Ok(String::new())
    }
}

/// Probe common drawio install sites without executing. Order: Flatpak exported
/// launcher (checked as a FILE, per the reporter's note — not via `flatpak run`,
/// which would inherit our env), then snap, then a `drawio` on PATH, then the
/// platform app bundle. Returns a command template or "".
#[cfg(desktop)]
fn detect_drawio() -> String {
    #[cfg(target_os = "linux")]
    {
        // Flatpak: the exported bin is a plain wrapper file we can stat.
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let flatpak_bins = [
            home.as_ref()
                .map(|h| h.join(".local/share/flatpak/exports/bin/com.jgraph.drawio.desktop")),
            Some(std::path::PathBuf::from(
                "/var/lib/flatpak/exports/bin/com.jgraph.drawio.desktop",
            )),
        ];
        for b in flatpak_bins.into_iter().flatten() {
            if b.exists() {
                return "flatpak run com.jgraph.drawio.desktop {}".to_string();
            }
        }
        if std::path::Path::new("/snap/bin/drawio").exists() {
            return "/snap/bin/drawio {}".to_string();
        }
        if let Some(p) = which_on_path("drawio") {
            return format!("{} {{}}", p.display());
        }
        String::new()
    }
    #[cfg(target_os = "macos")]
    {
        if std::path::Path::new("/Applications/draw.io.app").exists() {
            return "open -a draw.io {}".to_string();
        }
        String::new()
    }
    #[cfg(target_os = "windows")]
    {
        detect_drawio_windows()
    }
}

#[cfg(any(target_os = "windows", test))]
fn detect_drawio_windows() -> String {
    detect_drawio_windows_with(
        |name: &'static str| std::env::var_os(name),
        |path| path.is_file(),
    )
}

/// Windows installers can be per-user (`LOCALAPPDATA`) or per-machine
/// (`ProgramFiles`, including 32-bit installs). Keep the environment/filesystem
/// inputs injectable so this platform-specific discovery policy is covered by
/// host tests without mutating the process environment.
#[cfg(any(target_os = "windows", test))]
fn detect_drawio_windows_with<V, F>(mut var: V, mut is_file: F) -> String
where
    // Every probed environment name below is a string literal. Expressing that
    // lifetime avoids passing the generic `std::env::var_os` function item
    // through a higher-ranked `FnMut(&str)` bound, which MSVC rejects as "not
    // general enough" even though host builds accept it.
    V: FnMut(&'static str) -> Option<std::ffi::OsString>,
    F: FnMut(&std::path::Path) -> bool,
{
    let locations = [
        ("LOCALAPPDATA", Some("Programs")),
        ("ProgramFiles", None),
        ("ProgramFiles(x86)", None),
    ];
    for (variable, extra) in locations {
        let Some(root) = var(variable) else {
            continue;
        };
        let mut exe = std::path::PathBuf::from(root);
        if let Some(component) = extra {
            exe.push(component);
        }
        exe.push("draw.io");
        exe.push("draw.io.exe");
        if is_file(&exe) {
            // Windows executable paths commonly contain spaces. The tokenizer
            // below strips these grouping quotes before direct argv spawning.
            return format!("\"{}\" {{}}", exe.display());
        }
    }
    String::new()
}

/// Find an executable by name on `$PATH` (stat only, no exec). Linux/macOS.
#[cfg(all(desktop, unix))]
fn which_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| cand.is_file())
}

/// Split a user command template into (program, args) for an editor launch.
/// Double quotes group whitespace but are not passed to the child; backslashes
/// are always literal, which is required for ordinary Windows paths. This is a
/// deliberately small argv tokenizer, not a shell: there is no expansion,
/// interpolation, or escape syntax. Unmatched quotes and an empty program are
/// rejected. `{}` is substituted in arguments; otherwise the target path is
/// appended as the final argument.
#[cfg(any(desktop, test))]
fn build_editor_argv(command: &str, target: &str) -> Result<(String, Vec<String>), String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quoted = false;
    for ch in command.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                token_started = true;
            }
            ch if ch.is_whitespace() && !quoted => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            _ => {
                token.push(ch);
                token_started = true;
            }
        }
    }
    if quoted {
        return Err("unclosed double quote in editor command".to_string());
    }
    if token_started {
        tokens.push(token);
    }

    let (prog, rest) = tokens
        .split_first()
        .ok_or_else(|| "empty editor command".to_string())?;
    if prog.is_empty() {
        return Err("editor command program is empty".to_string());
    }
    let mut args: Vec<String> = Vec::new();
    let mut substituted = false;
    for tok in rest {
        if tok.contains("{}") {
            args.push(tok.replace("{}", target));
            substituted = true;
        } else {
            args.push((*tok).to_string());
        }
    }
    if !substituted {
        args.push(target.to_string());
    }
    Ok((prog.clone(), args))
}

#[cfg(test)]
mod editor_argv_tests {
    use super::{build_editor_argv, detect_drawio_windows, detect_drawio_windows_with};
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn appends_path_when_no_placeholder() {
        let (p, a) = build_editor_argv("drawio", "/g/assets/x.drawio.svg").unwrap();
        assert_eq!(p, "drawio");
        assert_eq!(a, vec!["/g/assets/x.drawio.svg"]);
    }

    #[test]
    fn substitutes_a_placeholder_token() {
        let (p, a) =
            build_editor_argv("flatpak run com.jgraph.drawio.desktop {}", "/g/x.svg").unwrap();
        assert_eq!(p, "flatpak");
        assert_eq!(a, vec!["run", "com.jgraph.drawio.desktop", "/g/x.svg"]);
    }

    #[test]
    fn substitutes_inside_a_token() {
        let (p, a) = build_editor_argv("app --file={}", "/g/x.svg").unwrap();
        assert_eq!(p, "app");
        assert_eq!(a, vec!["--file=/g/x.svg"]);
    }

    #[test]
    fn quoted_windows_program_path_is_one_argv_token() {
        let (p, a) = build_editor_argv(
            r#""C:\Program Files\draw.io\draw.io.exe" {}"#,
            r#"C:\graph\assets\x.drawio.svg"#,
        )
        .unwrap();
        assert_eq!(p, r#"C:\Program Files\draw.io\draw.io.exe"#);
        assert_eq!(a, vec![r#"C:\graph\assets\x.drawio.svg"#]);
    }

    #[test]
    fn quoted_argument_with_spaces_is_one_argv_token() {
        let (p, a) = build_editor_argv(
            r#"drawio --profile "C:\Users\Me\Drawio Profile" {}"#,
            r#"C:\graph\assets\x.drawio.svg"#,
        )
        .unwrap();
        assert_eq!(p, "drawio");
        assert_eq!(
            a,
            vec![
                r#"--profile"#,
                r#"C:\Users\Me\Drawio Profile"#,
                r#"C:\graph\assets\x.drawio.svg"#,
            ]
        );
    }

    #[test]
    fn malformed_or_empty_commands_are_rejected() {
        assert_eq!(
            build_editor_argv("   ", "/g/x.svg").unwrap_err(),
            "empty editor command"
        );
        assert_eq!(
            build_editor_argv(r#""C:\Program Files\draw.io\draw.io.exe {}"#, "/g/x.svg")
                .unwrap_err(),
            "unclosed double quote in editor command"
        );
        assert_eq!(
            build_editor_argv(r#""" {}"#, "/g/x.svg").unwrap_err(),
            "editor command program is empty"
        );
    }

    #[test]
    fn windows_autodetect_checks_per_machine_install_locations() {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            let root = PathBuf::from(format!("/{variable}"));
            let expected = root.join("draw.io").join("draw.io.exe");
            let command = detect_drawio_windows_with(
                |key| (key == variable).then(|| OsString::from(&root)),
                |path| path == expected,
            );
            assert_eq!(command, format!("\"{}\" {{}}", expected.display()));
        }
    }

    #[test]
    fn windows_autodetect_keeps_per_user_install_first() {
        let local = PathBuf::from("/Local App Data");
        let machine = PathBuf::from("/Program Files");
        let expected = local.join("Programs").join("draw.io").join("draw.io.exe");
        let command = detect_drawio_windows_with(
            |key| match key {
                "LOCALAPPDATA" => Some(OsString::from(&local)),
                "ProgramFiles" => Some(OsString::from(&machine)),
                _ => None,
            },
            |path| path == expected || path == machine.join("draw.io").join("draw.io.exe"),
        );
        assert_eq!(command, format!("\"{}\" {{}}", expected.display()));
    }

    #[test]
    fn windows_autodetect_returns_empty_when_no_candidate_is_a_file() {
        let command = detect_drawio_windows_with(|_| Some(OsString::from("/missing")), |_| false);
        assert!(command.is_empty());
    }

    #[test]
    fn windows_autodetect_real_callbacks_compile_and_run() {
        // This wrapper is the exact Windows call site. Keeping it compiled in
        // host tests catches callback lifetime regressions even before the
        // Windows CI runner builds the cfg(target_os = "windows") branch.
        let _ = detect_drawio_windows();
    }
}

/// Orphaned `assets/` files (no block references them) for the cleanup UI.
#[tauri::command]
pub(crate) fn list_orphan_assets(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::AssetInfo>, String> {
    with_graph(&state, |g| Ok(g.orphan_assets()))
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| g.trash_asset(&name).map_err(|e| e.to_string()))
}

/// Count + total bytes in the recoverable asset trash.
#[tauri::command]
pub(crate) fn asset_trash_stats(
    state: GraphContext<'_>,
) -> Result<tine_core::model::TrashStats, String> {
    with_graph(&state, |g| Ok(g.asset_trash_stats()))
}

/// Permanently delete everything in the asset trash; returns files removed.
#[tauri::command]
pub(crate) fn empty_asset_trash(state: GraphContext<'_>) -> Result<u64, String> {
    with_graph(&state, |g| g.empty_asset_trash().map_err(|e| e.to_string()))
}

/// Journal days that resolve to more than one file (e.g. a date-stem file plus a
/// title-named one) — for the user to reconcile.
#[tauri::command]
pub(crate) fn list_journal_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::JournalConflict>, String> {
    with_graph(&state, |g| Ok(g.journal_conflicts()))
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    with_graph(&state, |g| Ok(g.list_sync_conflicts()))
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    with_graph(&state, |g| {
        g.sync_conflict_diff(&winner, &conflict)
            .map_err(|e| e.to_string())
    })
}

/// Resolve a sync-conflict copy: merge it into its winner per the user's per-row
/// `decisions` (row id → "mine"/"theirs"/"both") via the normal save path, then
/// trash the conflict copy. `base_rev` guards against the winner changing under
/// the merge; returns "conflict" if it did. `pre_choice`: "mine"/"theirs"/"union".
#[tauri::command]
pub(crate) fn resolve_sync_conflict(
    winner: String,
    conflict: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    conflict_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.resolve_sync_conflict(
            &winner,
            &conflict,
            &decisions,
            &base_rev,
            &conflict_rev,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "conflict".to_string()
            } else {
                e.to_string()
            }
        })
    })
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) fn trash_sync_conflict(conflict: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.trash_sync_conflict(&conflict).map_err(|e| e.to_string())
    })
}

/// Move one journal file (by exact filename) to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_journal_file(name: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.trash_journal_file(&name).map_err(|e| e.to_string())
    })
}

/// Raw contents of one journal file (by exact filename) — for inspecting a
/// duplicate day's files before reconciling.
#[tauri::command]
pub(crate) fn read_journal_file(name: String, state: GraphContext<'_>) -> Result<String, String> {
    with_graph(&state, |g| {
        g.read_journal_file(&name).map_err(|e| e.to_string())
    })
}

/// Load a page from a SPECIFIC file by its graph-root-relative path — lets the UI
/// navigate to a duplicate-day stray that shares a (kind,name) with the canonical
/// file and so is unreachable by name (#21).
#[tauri::command]
pub(crate) fn get_page_by_path(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, String> {
    with_graph(&state, |g| g.load_by_path(&path).map_err(|e| e.to_string()))
}

/// Reconcile a duplicate-day pair: append the blocks of `src` to `dst`, then trash
/// `src` (both graph-root-relative paths). The merged `dst` is written through the
/// normal round-tripping save path (#21).
#[tauri::command]
pub(crate) fn merge_pages(src: String, dst: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.merge_pages(&src, &dst).map_err(|e| e.to_string())
    })
}

/// Rescue a duplicate-day stray by moving it to a uniquely-named page
/// (`pages/<new_name>`), so it stops colliding and becomes normally navigable (#21).
#[tauri::command]
pub(crate) fn rename_file_to_page(
    path: String,
    new_name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.rename_file_to_page(&path, &new_name)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn save_asset(
    name: String,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_graph(&state, |g| {
        g.save_asset(&name, &bytes).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn read_highlights(
    pdf: String,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    with_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
pub(crate) fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.write_highlights(&pdf, &label, &highlights, &base_ids)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn save_pdf_area_image(
    pdf: String,
    page: i64,
    id: String,
    stamp: i64,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_graph(&state, |g| {
        g.write_pdf_area_image(&pdf, page, &id, stamp, &bytes)
            .map_err(|e| e.to_string())
    })
}
