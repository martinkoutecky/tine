//! Module map: debug startup logging; state graph lock; watcher external changes;
//! graph open/create/warm cache; backup snapshots; settings/session prefs;
//! spellcheck WebKit integration; platform OS bridges; commands thin IPC.

mod backup;
mod commands;
mod debug;
mod android_folder_picker;
mod graph;
mod platform;
mod settings;
mod spellcheck;
mod state;
mod watcher;

use backup::{get_backup_keep, list_backups, restore_backup, set_backup_keep};
use commands::{
    asset_trash_stats, block_ref_counts, block_referrers, delete_page, empty_asset_trash,
    get_backlinks, get_page, get_page_by_path, get_unlinked_refs, graph_source_files, import_asset,
    journal_content_days, journals_desc, list_journal_conflicts, list_orphan_assets, list_pages,
    list_sync_conflicts, list_templates, merge_pages, open_asset, page_aliases, page_icons,
    page_print_html, publish_html, query_facets, quick_switch, read_asset, read_custom_css,
    read_highlights, read_journal_file, read_local_image, rename_file_to_page, rename_page,
    resolve_block, resolve_blocks, resolve_sync_conflict, run_advanced_query, run_query,
    save_asset, save_page, save_pdf_area_image, search, set_default_journal_template,
    set_favorites, set_journal_title_format, set_preferred_format, set_preferred_workflow,
    set_start_of_week, set_timetracking_enabled, sync_conflict_diff, trash_asset,
    trash_journal_file, trash_sync_conflict, write_highlights,
};
use debug::{
    debug_enabled, debug_header, debug_info, debug_init, debug_log, diag, install_panic_logger,
};
use graph::{
    app_platform, begin_warm_cache, create_graph, default_graph_parent, load_graph, resolve_root,
    warm_cache_async, warm_done,
};
use platform::{copy_image_to_clipboard, gpu_env, open_external};
use settings::{
    get_app_bool, get_app_string, get_capture_enter_files, get_link_first_match, get_smooth_scroll,
    load_session, save_session, set_app_bool, set_app_string, set_capture_enter_files,
    set_link_first_match, set_smooth_scroll,
};
use spellcheck::{
    apply_spellcheck, apply_spellcheck_all, list_spellcheck_dictionaries, parse_spellcheck_langs,
};
use state::AppState;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{Emitter, Manager, State};
use tine_core::model::Graph;
use watcher::{get_watch_mode, set_watch_mode, start_watcher};

/// Show + focus the always-on-top quick-capture mini window (created hidden at
/// startup). Each show resets it to the small base size and anchors it near the
/// top of the screen so the frontend can grow it downward (multiple blocks, an
/// autocomplete popup, the date picker) without running off the bottom edge.
/// No-op if the window is missing.
#[cfg(desktop)]
fn show_capture(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.set_size(tauri::LogicalSize::new(600.0, 92.0));
        if let Ok(Some(mon)) = w.current_monitor() {
            let size = mon.size().to_logical::<f64>(mon.scale_factor());
            let x = ((size.width - 600.0) / 2.0).max(0.0);
            let y = size.height * 0.18;
            let _ = w.set_position(tauri::LogicalPosition::new(x, y));
        } else {
            let _ = w.center();
        }
        let _ = w.show();
        let _ = w.set_focus();
        // Tell the webview it was (re)shown so it re-fits its window to the
        // current content. The frontend can't rely on the focus event alone:
        // this window is created `focus: false` and frameless, so a WM may not
        // deliver a focus-gained event on show — leaving the window stuck at a
        // previously-grown size.
        let _ = app.emit("capture-shown", ());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Bring up debug logging FIRST (TINE_DEBUG=1 / --debug), so every later
    // milestone — and any panic — is captured to the log file from the very start.
    debug_init();
    install_panic_logger();
    debug_header();
    diag("main() entered");

    // AppImages bundle their own libwayland-client.so; on a Wayland session it can
    // mismatch the host compositor and abort WebKitGTK's EGL init ("Could not
    // create default EGL display: EGL_BAD_PARAMETER"). Self-heal by re-exec'ing
    // ONCE with the host's libwayland-client preloaded — so users never have to
    // set LD_PRELOAD by hand. Guarded to the actual problem case: only inside an
    // AppImage (`APPIMAGE` set), only on Wayland, only once (`TINE_WL_PRELOADED`),
    // only if a host lib is found. No effect on the raw binary / deb / rpm.
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("TINE_WL_PRELOADED").is_none()
        && !std::env::var("LD_PRELOAD")
            .unwrap_or_default()
            .contains("libwayland-client")
    {
        // Common host locations across distros (Debian/Ubuntu, Fedora/openSUSE, Arch).
        const CANDIDATES: &[&str] = &[
            "/usr/lib/x86_64-linux-gnu/libwayland-client.so.0",
            "/usr/lib64/libwayland-client.so.0",
            "/usr/lib/libwayland-client.so.0",
            "/lib/x86_64-linux-gnu/libwayland-client.so.0",
        ];
        if let (Some(lib), Ok(exe)) = (
            CANDIDATES.iter().find(|p| std::path::Path::new(p).exists()),
            std::env::current_exe(),
        ) {
            use std::os::unix::process::CommandExt;
            let existing = std::env::var("LD_PRELOAD").unwrap_or_default();
            let preload = if existing.is_empty() {
                lib.to_string()
            } else {
                format!("{lib}:{existing}")
            };
            // `exec` only returns on failure; on success it replaces this process
            // (same PID, env + bundled LD_LIBRARY_PATH inherited, host lib preloaded).
            diag(format!(
                "Wayland AppImage: re-exec with LD_PRELOAD={preload}"
            ));
            let err = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("LD_PRELOAD", preload)
                .env("TINE_WL_PRELOADED", "1")
                .exec();
            diag(format!(
                "Wayland libwayland-client preload re-exec failed ({err}); continuing"
            ));
        }
    }

    // GPU/DMABUF rendering is ON by default (smoother scrolling — that's the point
    // of Tine). On the rare GPU/compositor combo where WebKitGTK's DMABUF renderer
    // aborts ("Could not create default EGL display: EGL_BAD_PARAMETER"), set
    // TINE_GPU=0 to fall back to software compositing. Linux-only.
    #[cfg(target_os = "linux")]
    if std::env::var("TINE_GPU").as_deref() == Ok("0")
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        diag("TINE_GPU=0 → set WEBKIT_DISABLE_DMABUF_RENDERER=1 (software compositing)");
    }

    let builder = tauri::Builder::default();

    #[cfg(desktop)]
    let builder = builder
        // MUST be the first plugin. A second launch (e.g. the DE hotkey running
        // `tine --capture`) doesn't start a new process — this fires in the
        // already-running instance with the new argv. `--capture` pops the
        // capture window; a plain re-launch just surfaces the main window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|a| a == "--capture") {
                show_capture(app);
            } else if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // In-app self-update. The updater reads `plugins.updater` from
        // tauri.conf.json (endpoints + minisign pubkey); process powers the
        // frontend's post-install `relaunch()`. Inert until a signed release with
        // a `latest.json` exists — the frontend catches any check() error and
        // falls back to opening the releases page (see src/update.ts).
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Remember window size/position/maximized across launches (Wayland
        // compositors don't restore this per-app). Exclude FULLSCREEN so it
        // doesn't conflict with focus mode, which is intentionally not persisted.
        // The capture window is centered on demand, so keep it off the denylist.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .with_denylist(&["capture"])
                .build(),
        );

    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_folder_picker::init());

    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        // The hidden capture window would otherwise keep the process alive after
        // the main window is closed (Tauri exits only when ALL windows are gone).
        // So when `main` is destroyed, exit explicitly. The JS close handler has
        // already flushed pending edits by the time it calls destroy().
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Destroyed = event {
                    window.app_handle().exit(0);
                }
            }
        })
        .manage(AppState {
            graph: RwLock::new(None),
            warm_done: AtomicBool::new(false),
            warm_generation: AtomicU64::new(0),
            watch_ctl: Mutex::new(None),
        })
        .setup(|app| {
            diag("setup() begin");
            // Eagerly open the graph if one was configured at startup.
            if let Some(root) = resolve_root("") {
                let g = Graph::open(&root);
                // These diagnostics enumerate dirs AND force a whole-graph cache
                // build (journals_desc()/list_pages()) — on the cold-cache critical
                // path to first paint, before warm_cache_async. The format! args
                // are evaluated regardless of whether diag() ends up writing, so
                // gate the whole block on debug to keep it off the 99% hot launch.
                if debug_enabled() {
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
                    diag(format!("graph root: {}", meta.root));
                    diag(format!(
                        "journals dir: {} (exists={}, .md files={:?})",
                        jdir.display(),
                        jdir.is_dir(),
                        count_md(&jdir)
                    ));
                    diag(format!(
                        "pages dir: {} (exists={}, .md files={:?})",
                        pdir.display(),
                        pdir.is_dir(),
                        count_md(&pdir)
                    ));
                    diag(format!(
                        "journals recognized as dates: {} | total page entries: {}",
                        g.journals_desc().len(),
                        g.list_pages().len()
                    ));
                    if let Ok(rd) = std::fs::read_dir(&jdir) {
                        let sample: Vec<String> = rd
                            .flatten()
                            .filter_map(|e| e.file_name().into_string().ok())
                            .filter(|n| n.ends_with(".md"))
                            .take(3)
                            .collect();
                        diag(format!("sample journal files: {sample:?}"));
                    }
                }
                let state: State<'_, AppState> = app.state();
                let warm_generation = begin_warm_cache(&state);
                *state.graph.write().unwrap() = Some(Arc::new(g));
                // Warm the whole-graph cache in the background. Without this the
                // startup (TINE_GRAPH / argv) path never warms — only the
                // `load_graph` command did — so the user's first `g j` (whose
                // agenda query touches the whole graph) paid to parse every file
                // synchronously. First nav slow, second fine; this fixes it.
                warm_cache_async(app.handle().clone(), warm_generation);
            } else {
                diag("NO graph root resolved — set TINE_GRAPH=/path/to/graph");
            }
            // Watch for external changes (reads whichever graph is current).
            start_watcher(app.handle().clone());
            diag("setup() done — watcher started, handing off to webview");
            // Spell checking (WebKitGTK): apply the persisted prefs to every window.
            // Default ON (matches Logseq); languages empty ⇒ OS locale; listing
            // several ⇒ bilingual. The frontend re-applies after its own init too.
            {
                let h = app.handle();
                let enabled = get_app_bool("spellcheck_enabled".to_string(), true, h.clone());
                let langs = parse_spellcheck_langs(&get_app_string(
                    "spellcheck_languages".to_string(),
                    String::new(),
                    h.clone(),
                ));
                apply_spellcheck_all(h, enabled, &langs);
            }
            // Cold start via `tine --capture` (app wasn't already running): pop
            // the capture window once we're up (the main window loads too).
            // Desktop-only: the capture window and `--capture` argv don't exist on mobile.
            #[cfg(desktop)]
            if std::env::args().any(|a| a == "--capture") {
                show_capture(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_graph,
            create_graph,
            app_platform,
            default_graph_parent,
            android_folder_picker::pick_graph_folder,
            list_pages,
            journals_desc,
            get_page,
            graph_source_files,
            save_page,
            get_backlinks,
            get_unlinked_refs,
            warm_done,
            block_ref_counts,
            block_referrers,
            delete_page,
            rename_page,
            publish_html,
            page_print_html,
            run_query,
            run_advanced_query,
            query_facets,
            page_aliases,
            page_icons,
            set_favorites,
            set_preferred_workflow,
            set_timetracking_enabled,
            set_preferred_format,
            set_journal_title_format,
            set_default_journal_template,
            set_start_of_week,
            read_custom_css,
            open_external,
            copy_image_to_clipboard,
            open_asset,
            list_orphan_assets,
            trash_asset,
            asset_trash_stats,
            empty_asset_trash,
            list_journal_conflicts,
            list_sync_conflicts,
            sync_conflict_diff,
            resolve_sync_conflict,
            trash_sync_conflict,
            trash_journal_file,
            read_journal_file,
            get_page_by_path,
            merge_pages,
            rename_file_to_page,
            search,
            quick_switch,
            list_templates,
            journal_content_days,
            resolve_block,
            resolve_blocks,
            read_asset,
            read_local_image,
            import_asset,
            save_asset,
            read_highlights,
            write_highlights,
            save_pdf_area_image,
            get_backup_keep,
            set_backup_keep,
            get_capture_enter_files,
            set_capture_enter_files,
            get_link_first_match,
            set_link_first_match,
            get_watch_mode,
            set_watch_mode,
            list_backups,
            restore_backup,
            load_session,
            save_session,
            gpu_env,
            get_smooth_scroll,
            set_smooth_scroll,
            get_app_bool,
            set_app_bool,
            get_app_string,
            set_app_string,
            apply_spellcheck,
            list_spellcheck_dictionaries,
            debug_info,
            debug_log
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
