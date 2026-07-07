use crate::debug::diag;
#[cfg(desktop)]
use crate::platform::opener_command;
use crate::state::{refresh_graph, with_graph, AppState};
use std::sync::Arc;
use tauri::State;
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
pub(crate) fn list_pages(state: State<'_, AppState>) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.list_pages()))
}

#[tauri::command]
pub(crate) fn journals_desc(
    limit: usize,
    offset: usize,
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
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
pub(crate) fn get_backlinks(
    name: String,
    state: State<'_, AppState>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.backlinks(&name)))
}

#[tauri::command]
pub(crate) fn get_unlinked_refs(
    name: String,
    state: State<'_, AppState>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.unlinked_refs(&name)))
}

/// `block uuid → # of referrers` over the whole graph (drives the per-block
/// reference-count badge). Small map (only referenced uuids); fetched once per
/// graph generation by the frontend.
#[tauri::command]
pub(crate) fn block_ref_counts(
    state: State<'_, AppState>,
) -> Result<Arc<std::collections::HashMap<String, usize>>, String> {
    with_graph(&state, |g| Ok(g.block_ref_counts()))
}

/// The blocks that reference block `uuid`, grouped by page (the badge's referrers
/// panel). Lazy: called only when a badge is clicked open.
#[tauri::command]
pub(crate) fn block_referrers(
    uuid: String,
    state: State<'_, AppState>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.block_referrers(&uuid)))
}

#[tauri::command]
pub(crate) fn delete_page(
    name: String,
    kind: PageKind,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.delete_page(&name, kind).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn rename_page(
    old: String,
    new: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.rename_page(&old, &new).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn publish_html(state: State<'_, AppState>) -> Result<(String, usize), String> {
    with_graph(&state, |g| g.publish_html().map_err(|e| e.to_string()))
}

/// Render one page to a self-contained HTML document (assets inlined, no sidebar)
/// for the print-to-PDF export, with the dialog's options. `Err("no-page")` if the
/// page doesn't exist.
#[tauri::command]
pub(crate) fn page_print_html(
    name: String,
    opts: tine_core::publish::PrintOpts,
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.run_query(&query)))
}

#[tauri::command]
pub(crate) fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: State<'_, AppState>,
) -> Result<tine_core::query::AdvancedResult, String> {
    with_graph(&state, |g| {
        Ok(g.run_advanced_query(&query, current_page.as_deref()))
    })
}

#[tauri::command]
pub(crate) fn query_facets(
    state: State<'_, AppState>,
) -> Result<Vec<(String, Vec<String>)>, String> {
    with_graph(&state, |g| Ok(g.property_facets()))
}

#[tauri::command]
pub(crate) fn page_aliases(state: State<'_, AppState>) -> Result<Vec<(String, String)>, String> {
    with_graph(&state, |g| Ok(g.page_aliases()))
}

#[tauri::command]
pub(crate) fn page_icons(
    names: Vec<String>,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    with_graph(&state, |g| Ok(g.page_icons(&names)))
}

#[tauri::command]
pub(crate) fn set_favorites(names: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_favorites(&names).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_preferred_workflow(
    workflow: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_preferred_workflow(&workflow)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_timetracking_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_timetracking_enabled(enabled)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state);
    Ok(())
}

#[tauri::command]
pub(crate) fn set_default_journal_template(
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_default_journal_template(name.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_start_of_week(n: u32, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_start_of_week(n).map_err(|e| e.to_string())
    })
}

/// Set the graph's `:preferred-format` for new pages/journals ("md" or "org").
#[tauri::command]
pub(crate) fn set_preferred_format(
    format: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let fmt = if format.eq_ignore_ascii_case("org") {
        tine_core::model::Format::Org
    } else {
        tine_core::model::Format::Md
    };
    with_graph(&state, |g| {
        g.set_preferred_format(fmt).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state); // so new pages/journals use the new extension immediately
    Ok(())
}

/// Set the graph's `:journal/page-title-format` (journal display-title format,
/// e.g. "MMM do, yyyy"). Display-only — does not rename journal files.
#[tauri::command]
pub(crate) fn set_journal_title_format(
    format: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_journal_page_title_format(&format)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state); // pick up the new format + migrate any title-named journals
    Ok(())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: State<'_, AppState>) -> Result<String, String> {
    with_graph(&state, |g| Ok(g.custom_css()))
}

#[tauri::command]
pub(crate) fn search(
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.search(&query, limit)))
}

#[tauri::command]
pub(crate) fn quick_switch(
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.quick_switch(&query, limit)))
}

#[tauri::command]
pub(crate) fn list_templates(
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    with_graph(&state, |g| Ok(g.templates()))
}

#[tauri::command]
pub(crate) fn journal_content_days(state: State<'_, AppState>) -> Result<Vec<i64>, String> {
    with_graph(&state, |g| Ok(g.journal_content_days()))
}

#[tauri::command]
pub(crate) fn resolve_block(
    uuid: String,
    state: State<'_, AppState>,
) -> Result<Option<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.resolve_block(&uuid)))
}

#[tauri::command]
pub(crate) fn resolve_blocks(
    uuids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Option<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.resolve_blocks(&uuids)))
}

#[tauri::command]
pub(crate) fn read_asset(
    name: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    // Return RAW bytes (not a JSON number[]), so a multi-MB PDF/image isn't
    // serialized element-by-element and re-parsed on the JS side — the frontend
    // receives an ArrayBuffer directly.
    with_graph(&state, |g| {
        g.read_asset(&name)
            .map(tauri::ipc::Response::new)
            .map_err(|e| e.to_string())
    })
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

/// Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
/// debugging (GH #31). `open_devtools`/`close_devtools` are compiled in because
/// we enable tauri's `devtools` feature unconditionally (see Cargo.toml) — so
/// this works in shipped release builds, not just debug.
#[tauri::command]
pub(crate) fn tine_open_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
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
    state: State<'_, AppState>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        g.import_asset(std::path::Path::new(&path), name.as_deref())
            .map_err(|e| e.to_string())
    })
}

/// Open a graph asset (by its `assets/`-relative name) in the OS default app,
/// e.g. a video/audio file in the system player. Path-gated to the assets dir
/// (canonicalized) so a crafted name can't open a file outside the graph.
#[tauri::command]
pub(crate) fn open_asset(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let target = with_graph(&state, |g| {
        let assets = g.assets_path();
        let canon_assets = assets.canonicalize().map_err(|e| e.to_string())?;
        let canon = assets
            .join(&name)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !canon.starts_with(&canon_assets) {
            return Err("asset path escapes assets dir".to_string());
        }
        Ok(canon)
    })?;
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

/// Orphaned `assets/` files (no block references them) for the cleanup UI.
#[tauri::command]
pub(crate) fn list_orphan_assets(
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::model::AssetInfo>, String> {
    with_graph(&state, |g| Ok(g.orphan_assets()))
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_asset(name: String, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.trash_asset(&name).map_err(|e| e.to_string()))
}

/// Count + total bytes in the recoverable asset trash.
#[tauri::command]
pub(crate) fn asset_trash_stats(
    state: State<'_, AppState>,
) -> Result<tine_core::model::TrashStats, String> {
    with_graph(&state, |g| Ok(g.asset_trash_stats()))
}

/// Permanently delete everything in the asset trash; returns files removed.
#[tauri::command]
pub(crate) fn empty_asset_trash(state: State<'_, AppState>) -> Result<u64, String> {
    with_graph(&state, |g| g.empty_asset_trash().map_err(|e| e.to_string()))
}

/// Journal days that resolve to more than one file (e.g. a date-stem file plus a
/// title-named one) — for the user to reconcile.
#[tauri::command]
pub(crate) fn list_journal_conflicts(
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::model::JournalConflict>, String> {
    with_graph(&state, |g| Ok(g.journal_conflicts()))
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) fn list_sync_conflicts(
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    with_graph(&state, |g| Ok(g.list_sync_conflicts()))
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: State<'_, AppState>,
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
    base_rev: Option<String>,
    pre_choice: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.resolve_sync_conflict(
            &winner,
            &conflict,
            &decisions,
            base_rev.as_deref(),
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
pub(crate) fn trash_sync_conflict(
    conflict: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.trash_sync_conflict(&conflict).map_err(|e| e.to_string())
    })
}

/// Move one journal file (by exact filename) to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_journal_file(name: String, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.trash_journal_file(&name).map_err(|e| e.to_string())
    })
}

/// Raw contents of one journal file (by exact filename) — for inspecting a
/// duplicate day's files before reconciling.
#[tauri::command]
pub(crate) fn read_journal_file(
    name: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
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
    state: State<'_, AppState>,
) -> Result<Option<PageDto>, String> {
    with_graph(&state, |g| g.load_by_path(&path).map_err(|e| e.to_string()))
}

/// Reconcile a duplicate-day pair: append the blocks of `src` to `dst`, then trash
/// `src` (both graph-root-relative paths). The merged `dst` is written through the
/// normal round-tripping save path (#21).
#[tauri::command]
pub(crate) fn merge_pages(
    src: String,
    dst: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_graph(&state, |g| {
        g.save_asset(&name, &bytes).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn read_highlights(
    pdf: String,
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    with_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
pub(crate) fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_graph(&state, |g| {
        g.write_pdf_area_image(&pdf, page, &id, stamp, &bytes)
            .map_err(|e| e.to_string())
    })
}
