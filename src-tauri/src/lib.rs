//! Module map: debug startup logging; state graph lock; watcher external changes;
//! graph open/create/warm cache; backup snapshots; settings/session prefs;
//! spellcheck WebKit integration; platform OS bridges; commands thin IPC.

mod android_folder_picker;
mod android_media;
mod android_system_bars;
mod backup;
mod commands;
mod debug;
mod graph;
#[cfg(target_os = "linux")]
mod linux_window_identity;
mod migrate_identifier;
mod media_protocol;
mod platform;
mod settings;
mod spellcheck;
mod state;
mod watcher;

use backup::{get_backup_keep, list_backups, restore_backup, set_backup_keep};
use commands::{
    asset_trash_stats, block_ref_counts, block_referrers, close_graph_window,
    copy_guide_into_graph, delete_page, detect_media_editor, edit_asset_external,
    empty_asset_trash, get_backlinks, get_page, get_page_by_path, get_unlinked_refs,
    graph_source_files, guide_pages, import_asset, import_native_capture, journal_content_days, journals_desc,
    list_journal_conflicts, list_orphan_assets, list_pages, list_sync_conflicts, list_templates,
    merge_pages, open_asset, open_page_file, page_aliases, page_icons, page_print_html, publish_html, query_facets,
    open_pdf, quick_switch, read_asset, read_custom_css, read_highlights, read_journal_file,
    read_local_image, read_text_file, rename_file_to_page, rename_page, resolve_block,
    resolve_blocks, resolve_sync_conflict, run_advanced_query, run_graph_search, run_query, save_asset, save_page,
    save_pdf_area_image, search, set_default_journal_template, set_favorites, set_guide_announced,
    set_journal_title_format, set_preferred_format, set_preferred_workflow, set_start_of_week,
    set_timetracking_enabled, stream_asset_path, sync_conflict_diff, tine_open_devtools, tine_quit, trash_asset,
    trash_journal_file, trash_sync_conflict, write_highlights, write_pdf_view_state,
};
use debug::{
    debug_enabled, debug_header, debug_info, debug_init, debug_log, diag, install_panic_logger,
};
use graph::{
    app_platform, approve_external_assets, capture_target, create_graph, default_graph_parent,
    inspect_graph_access, load_graph, open_graph_window, resolve_root, startup_graph_path, warm_done,
};
use platform::{clipboard_files, copy_image_to_clipboard, gpu_env, open_external};
use settings::{
    forget_known_graph, get_app_bool, get_app_string, get_capture_enter_files,
    get_link_first_match, get_smooth_scroll, list_known_graphs, load_session, save_session,
    set_app_bool, set_app_string, set_capture_enter_files, set_link_first_match, set_smooth_scroll,
};
use spellcheck::{
    apply_spellcheck, apply_spellcheck_all, list_spellcheck_dictionaries, parse_spellcheck_langs,
};
use state::AppState;
#[cfg(desktop)]
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, RwLock};
#[cfg(desktop)]
use tauri::Emitter;
use tauri::Manager;
use watcher::{get_watch_mode, set_watch_mode, start_watcher};

#[cfg(desktop)]
const MAIN_WINDOW_REVEAL_FALLBACK_MS: u64 = 3_000;

/// Xlib's thread mode is process-global and must be selected before the first
/// GTK/Xlib call. Secondary `--capture` launches are short-lived forwarders, but
/// GTK and Tauri can still touch X11 from separate threads during their startup
/// and teardown; without this initialization XCB aborts instead of forwarding.
#[cfg(target_os = "linux")]
fn init_xlib_threads() {
    // SAFETY: this is the first native-window call in `run`, before GTK/Tauri is
    // initialized. Repeated calls across tests or an AppImage re-exec are safe.
    unsafe {
        let _ = x11::xlib::XInitThreads();
    }
}

/// The frontend normally reveals the main window after its themed App has
/// painted. Keep a native fail-safe so a parser/session/frontend failure cannot
/// strand the process as an invisible application.
#[cfg(desktop)]
fn schedule_main_window_reveal_fallback(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(
            MAIN_WINDOW_REVEAL_FALLBACK_MS,
        ));
        let main_thread_app = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(window) = main_thread_app.get_webview_window("main") else {
                return;
            };
            if !window.is_visible().unwrap_or(false) {
                let _ = window.show();
            }
        });
    });
}

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
        // Do not activate until the frontend acknowledges that its textarea and
        // capture-shown listener exist. Activating a newly mapped window first
        // lets a fast typist send keys into an unready WebView; Plasma can also
        // reject that too-early focus request before the surface is paint-ready.
        let _ = app.emit("capture-shown", ());

        // The frontend-ready acknowledgement is the preferred path, but it is
        // deliberately not the only one: WebKitGTK may throttle a hidden
        // auxiliary WebView enough to miss/delay its event listener. Start the
        // same bounded activation sequence after the newly mapped window has
        // had one paint turn. Visibility checks make this harmless if the user
        // closes capture again in the meantime.
        let focus_app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(
                CAPTURE_INITIAL_FOCUS_DELAY_MS,
            ));
            let main_thread_app = focus_app.clone();
            let _ = focus_app.run_on_main_thread(move || {
                if main_thread_app
                    .get_webview_window("capture")
                    .and_then(|window| window.is_visible().ok())
                    .unwrap_or(false)
                {
                    activate_capture_window(&main_thread_app);
                }
            });
        });
    }
}

#[cfg(desktop)]
const CAPTURE_INITIAL_FOCUS_DELAY_MS: u64 = 120;
#[cfg(desktop)]
const CAPTURE_FOCUS_RETRY_DELAYS_MS: [u64; 5] = [40, 120, 260, 520, 900];

/// Activate Quick Capture only after its frontend has mounted the editor. The
/// bounded retries are intentional: KWin/Plasma may reject a focus request made
/// in the same turn as mapping a frameless window because it is not ready for
/// painting yet. Every retry follows an explicit `tine --capture` user action.
#[cfg(desktop)]
fn activate_capture_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("capture") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = app.emit_to("capture", "capture-focus-editor", ());

    let app = app.clone();
    std::thread::spawn(move || {
        let mut elapsed = 0;
        for at in CAPTURE_FOCUS_RETRY_DELAYS_MS {
            std::thread::sleep(std::time::Duration::from_millis(at - elapsed));
            elapsed = at;
            let focus_app = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(window) = focus_app.get_webview_window("capture") else {
                    return;
                };
                if window.is_visible().unwrap_or(false) {
                    let _ = window.set_focus();
                    let _ = focus_app.emit_to("capture", "capture-focus-editor", ());
                }
            });
        }
    });
}

#[tauri::command]
fn capture_frontend_ready(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(desktop)]
    {
        if window.label() != "capture" {
            return Err("capture activation is only available to the capture window".into());
        }
        if !window.is_visible().map_err(|error| error.to_string())? {
            return Err("capture window is hidden".into());
        }
        activate_capture_window(&app);
        Ok(())
    }

    #[cfg(not(desktop))]
    {
        let _ = (window, app);
        Err("quick capture is only available on desktop".into())
    }
}

#[cfg(desktop)]
fn focus_last_graph_window(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let label = state.last_focused.lock().unwrap().clone().or_else(|| {
        state
            .graphs
            .read()
            .unwrap()
            .entries()
            .into_iter()
            .next()
            .map(|e| e.0)
    });
    if let Some(window) = label.and_then(|label| app.get_webview_window(&label)) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(desktop)]
fn forwarded_graph_path(argv: &[String], cwd: &str) -> Option<String> {
    let raw = argv.iter().skip(1).find(|arg| !arg.starts_with('-'))?;
    let path = std::path::Path::new(raw);
    Some(
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::path::Path::new(cwd).join(path)
        }
        .display()
        .to_string(),
    )
}

#[cfg(all(test, desktop))]
mod multi_window_tests {
    use super::*;

    #[test]
    fn forwarded_path_ignores_flags_and_resolves_against_sender_cwd() {
        let argv = vec![
            "tine".to_string(),
            "--debug".to_string(),
            "graphs/second".to_string(),
        ];
        assert_eq!(
            forwarded_graph_path(&argv, "/home/user").as_deref(),
            Some("/home/user/graphs/second")
        );
    }

    #[test]
    fn capture_only_launch_has_no_graph_path() {
        let argv = vec!["tine".to_string(), "--capture".to_string()];
        assert!(forwarded_graph_path(&argv, "/tmp").is_none());
    }

    #[test]
    fn capture_activation_retries_after_the_frontend_ready_handshake() {
        assert_eq!(CAPTURE_INITIAL_FOCUS_DELAY_MS, 120);
        assert_eq!(CAPTURE_FOCUS_RETRY_DELAYS_MS, [40, 120, 260, 520, 900]);
        assert!(CAPTURE_FOCUS_RETRY_DELAYS_MS
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn graph_window_creation_stays_out_of_synchronous_windows_handlers() {
        // Windows WebView2 can deadlock the process if WebviewWindowBuilder is
        // reached from a synchronous command or event callback. Guard both
        // entry points: Shift-click IPC and single-instance argv forwarding.
        let graph_source = include_str!("graph.rs");
        let lib_source = include_str!("lib.rs");
        assert!(graph_source.contains("pub(crate) async fn open_graph_window("));
        assert!(lib_source.contains("tauri::async_runtime::spawn(async move"));
        assert!(lib_source.contains("open_graph_window(path, command_app.clone(), state).await"));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    init_xlib_threads();

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

    // Migrate the desktop app-data dir left behind by the app-identifier renames
    // (dev.logseqclaude.app / dev.tine.app / page.tine.app -> page.tine.Tine)
    // BEFORE building the webview. WebKitGTK's WebsiteDataManager creates the
    // new-id data dir (and its empty localStorage store) as the Builder is
    // assembled, so this has to happen first — otherwise the migration finds the
    // new dir already populated and backs off, orphaning the user's graph/session
    // (localStorage) + settings + backups. Records a one-shot flag; the frontend
    // toasts about the (possible) prefs reset. Android intentionally keeps
    // page.tine.app and run_early() is a no-op there.
    migrate_identifier::run_early();

    // Wayland resolves the shell/titlebar icon by matching a window app ID to a
    // desktop-entry basename. Packages ship that identity themselves; the raw
    // binary Martin runs is self-contained, so publish its marker-owned entry
    // before Tauri maps the first window.
    #[cfg(target_os = "linux")]
    linux_window_identity::install_desktop_identity();

    // Tao cannot reliably change Linux decorations after a GTK window exists.
    // Read the device preference before Tauri constructs the configured windows,
    // and expose the frozen value to each webview so custom controls never flash.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let native_frame_active = settings::init_native_frame_active();
    let context = tauri::generate_context!();
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let context = {
        let mut context = context;
        if let Some(main) = context
            .config_mut()
            .app
            .windows
            .iter_mut()
            .find(|window| window.label == "main")
        {
            main.decorations = native_frame_active;
        }
        context
    };

    let builder = tauri::Builder::default().register_uri_scheme_protocol(
        "tine-media",
        |ctx, request| media_protocol::respond(ctx, request),
    );

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let builder = builder.append_invoke_initialization_script(format!(
        "globalThis.__TINE_NATIVE_FRAME__ = {native_frame_active};"
    ));

    #[cfg(desktop)]
    let builder = builder
        // MUST be the first plugin. A second launch (e.g. the DE hotkey running
        // `tine --capture`) doesn't start a new process — this fires in the
        // already-running instance with the new argv. `--capture` pops the
        // capture window; a plain re-launch just surfaces the main window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            if argv.iter().any(|a| a == "--capture") {
                show_capture(app);
            } else if let Some(path) = forwarded_graph_path(&argv, &cwd) {
                // WebView2 deadlocks if a WebviewWindow is built directly from
                // a synchronous event handler. Use the async command path so
                // Windows' event loop remains available while Tauri creates it.
                let command_app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = command_app.state::<AppState>();
                    let _ = open_graph_window(path, command_app.clone(), state).await;
                });
            } else {
                focus_last_graph_window(app);
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
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_media::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_system_bars::init());
    // Mobile has no xdg-open/open/explorer, so `open_external` routes URL opens
    // through this plugin's platform Intent instead (GH #49). Desktop keeps its
    // env-scrubbed spawn; the plugin is compiled/registered on mobile only.
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_opener::init());

    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|window, event| {
            let label = window.label();
            if label != "main" && !label.starts_with("graph-") {
                return;
            }
            let app = window.app_handle();
            let state = app.state::<AppState>();
            match event {
                tauri::WindowEvent::Focused(true) => {
                    let changed = state.note_focused(label);
                    if changed {
                        if let Ok(slot) = state::slot_for_window(&state, label) {
                            let _ =
                                settings::remember_graph(app, &slot.root_key.display().to_string());
                        }
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    state.graphs.write().unwrap().remove(label);
                    state::poke_watcher(&state);
                    if state.graphs.read().unwrap().len() == 0 {
                        #[cfg(target_os = "linux")]
                        platform::kill_webkit_children();
                        app.exit(0);
                    }
                }
                _ => {}
            }
        })
        .manage(AppState {
            graphs: RwLock::new(state::GraphRegistry::default()),
            graph_load: Mutex::new(()),
            watch_ctl: Mutex::new(None),
            last_focused: Mutex::new(None),
            #[cfg(desktop)]
            next_window: AtomicU64::new(1),
        })
        .setup(|app| {
            diag("setup() begin");
            #[cfg(target_os = "linux")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    linux_window_identity::apply_to_window(&window);
                }
                if let Some(window) = app.get_webview_window("capture") {
                    linux_window_identity::apply_to_window(&window);
                }
            }
            #[cfg(desktop)]
            schedule_main_window_reveal_fallback(app.handle());
            // Eagerly open the graph if one was configured at startup.
            let startup_root = resolve_root("").or_else(|| settings::last_graph_path(app.handle()));
            if let Some(root) = startup_root {
                let state = app.state::<AppState>();
                graph::load_graph_for_label(root, app.handle(), "main", &state)?;
                let slot = state::slot_for_window(&state, "main")?;
                let g = &slot.graph;
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
            inspect_graph_access,
            approve_external_assets,
            open_graph_window,
            startup_graph_path,
            capture_target,
            capture_frontend_ready,
            create_graph,
            app_platform,
            default_graph_parent,
            android_folder_picker::pick_graph_folder,
            android_media::capture_photo,
            android_media::start_recording,
            android_media::stop_recording,
            android_media::cancel_recording,
            android_system_bars::set_system_bar_appearance,
            list_pages,
            journals_desc,
            get_page,
            graph_source_files,
            save_page,
            guide_pages,
            copy_guide_into_graph,
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
            run_graph_search,
            run_advanced_query,
            query_facets,
            page_aliases,
            page_icons,
            set_favorites,
            set_preferred_workflow,
            set_timetracking_enabled,
            set_guide_announced,
            set_preferred_format,
            set_journal_title_format,
            set_default_journal_template,
            set_start_of_week,
            read_custom_css,
            open_external,
            copy_image_to_clipboard,
            clipboard_files,
            open_asset,
            open_page_file,
            edit_asset_external,
            detect_media_editor,
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
            stream_asset_path,
            read_local_image,
            read_text_file,
            import_asset,
            import_native_capture,
            save_asset,
            read_highlights,
            open_pdf,
            write_highlights,
            write_pdf_view_state,
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
            list_known_graphs,
            forget_known_graph,
            migrate_identifier::take_identifier_migration_notice,
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
            debug_log,
            tine_quit,
            close_graph_window,
            tine_open_devtools
        ])
        .run(context)
        .expect("error while running tauri application");
}
