//! Module map: debug startup logging; state graph lock; watcher external changes;
//! graph open/create/warm cache; backup snapshots; settings/session prefs;
//! spellcheck WebKit integration; platform OS bridges; commands thin IPC.

mod android_folder_picker;
mod android_media;
mod android_system_bars;
mod app_identity;
mod backup;
#[cfg(desktop)]
mod cli;
mod command_surface;
mod commands;
#[path = "commands/concord.rs"]
mod concord;
mod concord_ledger;
mod debug;
mod device_io;
mod drafts;
#[cfg(test)]
mod edit_kind_guard_tests;
#[cfg(desktop)]
mod experiment_config_seed;
mod flight;
mod flight_store;
mod graph;
#[cfg(target_os = "linux")]
mod linux_window_identity;
#[cfg(test)]
mod load_wait_guard_tests;
mod media_protocol;
mod native_mouse_history;
mod pdf_crop_rollback;
mod platform;
mod plugins;
#[path = "commands/query_export.rs"]
mod query_export;
#[path = "commands/query_ir.rs"]
mod query_ir;
mod settings;
mod spellcheck;
mod state;
mod watcher;

use backup::{get_backup_keep, list_backups, restore_backup, set_backup_keep};
use commands::{
    apply_journal_filename_migrations, asset_trash_stats, block_ref_counts, block_referrers,
    capture_quick_switch, close_graph_window, copy_guide_into_graph, delete_page,
    detect_media_editor, edit_asset_external, empty_asset_trash, export_query_subtrees,
    get_backlink_filter_context, get_backlinks, get_page, get_page_by_path, get_unlinked_refs,
    graph_source_files, guide_pages, import_asset, import_native_capture, journal_content_days,
    journal_feed_page, list_journal_conflicts, list_journal_filename_migrations,
    list_orphan_assets, list_templates, load_workspaces, merge_pages, open_asset, open_page_file,
    open_pdf, page_icons, page_inventory, page_print_html, preview_block, publish_html,
    query_facets, quick_switch, read_asset, read_custom_css, read_highlights, read_journal_file,
    read_local_image, read_text_file, rename_file_to_page, rename_page, resolve_block,
    resolve_blocks, resolve_page, run_graph_search, save_asset, save_pages, save_pdf_area_image,
    save_workspaces, search, set_default_journal_template, set_doc_mode_enter_for_new_block,
    set_guide_announced, set_journal_title_format, set_logical_outdenting, set_preferred_format,
    set_preferred_workflow, set_show_brackets, set_start_of_week, set_timetracking_enabled,
    stream_asset_path, tine_open_devtools, tine_quit, trash_asset, trash_journal_file,
    write_highlights, write_pdf_view_state,
};
use concord::{
    conflict_inventory, list_sync_conflicts, live_conflict_diff, resolve_live_conflict,
    resolve_sync_conflict, resolve_vcs_marker_conflict, sync_conflict_diff, trash_sync_conflict,
    vcs_marker_conflict_diff,
};
use debug::{
    debug_enabled, debug_header, debug_info, debug_init, debug_log, diag, diag_private,
    install_panic_logger,
};
use graph::{
    app_platform, approve_external_assets, capture_graph_binding, capture_target, create_graph,
    default_graph_parent, inspect_graph_access, load_graph, local_clock, open_graph_window,
    resolve_root, startup_graph_path, warm_done,
};
use pdf_crop_rollback::rollback_pdf_area_image;
use platform::{clipboard_files, copy_image_to_clipboard, gpu_env, open_external};
use plugins::{
    install_plugin, list_installed_plugins, load_plugin_registry_cache, read_plugin_entry,
    set_plugin_enabled, store_plugin_registry_cache, uninstall_plugin, verify_plugin_registry,
};
use query_export::{publish_live, publish_query, publish_query_plan, sheet_export_inputs};
use query_ir::{
    query_explain_empty, query_og_expressible, query_parse, query_print, query_registry, query_run,
};
use settings::{
    forget_known_graph, get_app_bool, get_app_string, get_capture_enter_files,
    get_link_first_match, get_smooth_scroll, list_known_graphs, load_session, reveal_known_graph,
    save_session, set_app_bool, set_app_string, set_capture_enter_files, set_default_home,
    set_favorites, set_link_first_match, set_smooth_scroll,
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
use watcher::{get_watch_mode, rescan_graph_now, set_watch_mode, watcher_latency_recent};

#[cfg(desktop)]
const MAIN_WINDOW_REVEAL_FALLBACK_MS: u64 = 3_000;

/// Test-only policy used by the native WebKit drawer scenario.  Keeping this
/// narrow (only `main`) prevents a phone-sized E2E process from changing capture
/// or later graph windows.
fn force_mobile_drawers_e2e() -> bool {
    std::env::var("TINE_E2E_FORCE_MOBILE_DRAWERS").as_deref() == Ok("1")
}

fn apply_mobile_drawer_e2e_window_policy(
    windows: &mut [tauri::utils::config::WindowConfig],
    force: bool,
) -> bool {
    if !force {
        return false;
    }
    if let Some(main) = windows.iter_mut().find(|window| window.label == "main") {
        main.width = 390.0;
        main.height = 844.0;
        main.min_width = Some(390.0);
        true
    } else {
        false
    }
}

// Wry normally supplies these arguments itself. Once a window config provides
// `additional_browser_args`, it replaces that default rather than extending it.
#[cfg(any(target_os = "windows", test))]
const WRY_WINDOWS_DEFAULT_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

#[cfg(any(target_os = "windows", test))]
fn merged_windows_webdriver_args(
    configured: Option<&str>,
    automation_enabled: bool,
    inherited: Option<&str>,
) -> Option<String> {
    let inherited = inherited.map(str::trim).filter(|value| !value.is_empty())?;
    if !automation_enabled {
        return None;
    }
    Some(format!(
        "{} {inherited}",
        configured.unwrap_or(WRY_WINDOWS_DEFAULT_BROWSER_ARGS)
    ))
}

#[cfg(any(target_os = "windows", test))]
fn apply_windows_webdriver_window_policy(
    windows: &mut [tauri::utils::config::WindowConfig],
    automation_enabled: bool,
    inherited: Option<&str>,
) -> usize {
    let mut changed = 0;
    for window in windows {
        if let Some(arguments) = merged_windows_webdriver_args(
            window.additional_browser_args.as_deref(),
            automation_enabled,
            inherited,
        ) {
            window.additional_browser_args = Some(arguments);
            changed += 1;
        }
    }
    changed
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_webdriver_args_from_env(configured: Option<&str>) -> Option<String> {
    merged_windows_webdriver_args(
        configured,
        std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true"),
        std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
            .ok()
            .as_deref(),
    )
}

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
        // Retarget the read-only capture lease before the window can receive
        // the fallback focus. Until its frontend obtains this generation, old
        // WebView requests fail stale instead of querying a previously selected
        // graph after a hidden-window reopen.
        let state = app.state::<AppState>();
        if graph::refresh_capture_graph_binding(&state).is_err() {
            state.clear_capture_graph();
        }
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
/// Dispatch a desktop command before GUI initialization. A returned exit code
/// means the command already printed its outcome and no app should start.
#[cfg(desktop)]
pub fn cli_dispatch() -> Option<i32> {
    cli::dispatch()
}

pub fn run() {
    #[cfg(target_os = "linux")]
    init_xlib_threads();

    // Bring up debug logging FIRST (TINE_DEBUG=1 / --debug), so every later
    // milestone — and any panic — is captured to the log file from the very start.
    debug_init();
    flight::flight_init();
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
            diag_private(
                "wayland-preload",
                format!("Wayland AppImage: re-exec with LD_PRELOAD={preload}"),
            );
            let err = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("LD_PRELOAD", preload)
                .env("TINE_WL_PRELOADED", "1")
                .exec();
            diag_private(
                "wayland-preload-failed",
                format!("Wayland libwayland-client preload re-exec failed ({err}); continuing"),
            );
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

    // TEMPORARY (docs/app-identity.md): an experiment build with no graph of its
    // own starts from the released Tine's config. Before the Builder, because
    // WebKitGTK creates the app-data dir while the Builder is assembled.
    #[cfg(desktop)]
    experiment_config_seed::seed_from_release_once();

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
    let force_mobile_drawers = force_mobile_drawers_e2e();
    let mut context = tauri::generate_context!();
    #[cfg(target_os = "windows")]
    {
        let automation_enabled = std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true");
        let inherited = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").ok();
        apply_windows_webdriver_window_policy(
            &mut context.config_mut().app.windows,
            automation_enabled,
            inherited.as_deref(),
        );
    }
    let deny_main_window_state_restore = apply_mobile_drawer_e2e_window_policy(
        &mut context.config_mut().app.windows,
        force_mobile_drawers,
    );
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        if let Some(main) = context
            .config_mut()
            .app
            .windows
            .iter_mut()
            .find(|window| window.label == "main")
        {
            main.decorations = native_frame_active;
        }
    }

    let builder = tauri::Builder::default()
        .register_uri_scheme_protocol("tine-media", |ctx, request| {
            media_protocol::respond(ctx, request)
        });

    // The frontend's platform identity. It cannot be derived from the WebView's
    // user agent: iPadOS 13+ serves a desktop-class `Macintosh; Intel Mac OS X`
    // UA from a stock WKWebView, so UA sniffing reported an iPad as a Mac
    // desktop and every mobile affordance stayed hidden (GH #446). The build
    // knows the truth, so hand it over before frontend code runs -- the same
    // idiom as `__TINE_NATIVE_FRAME__`, and synchronous for the same reason:
    // an async `app_platform` round-trip would flash desktop-only chrome.
    let builder = builder.append_invoke_initialization_script(format!(
        "globalThis.__TINE_PLATFORM__ = {:?};",
        crate::graph::app_platform()
    ));

    // The backend's zone offset at launch, so the frontend's first "today" is
    // already the backend's (GH #607); `local_clock` keeps it current.
    let (offset_minutes, unix_ms) = tine_core::date::JournalDate::local_utc_offset_now();
    let builder = builder.append_invoke_initialization_script(format!(
        "globalThis.__TINE_LOCAL_CLOCK__ = {{ offset_minutes: {offset_minutes}, unix_ms: {unix_ms} }};"
    ));

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
            if argv.iter().any(|a| a == "--capture" || a == "capture") {
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
                .with_denylist(if deny_main_window_state_restore {
                    &["capture", "main"]
                } else {
                    &["capture"]
                })
                .build(),
        );

    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_folder_picker::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_media::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_system_bars::init());
    // Mobile has no xdg-open/open/explorer, so `open_external` routes URL opens
    // through this plugin's platform Intent instead (GH #49). Windows uses it
    // for ShellExecute, because `explorer <url>` opens a File Explorer window
    // instead of the browser (GH #215). `app.opener()` reads this plugin's
    // state, so both arms need it registered. Linux/macOS keep their
    // env-scrubbed spawn and do not compile the plugin at all.
    #[cfg(any(mobile, target_os = "windows"))]
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
                    // Auxiliary windows (Quick Capture in particular) never own
                    // a graph slot. Letting one replace `last_focused` makes a
                    // later capture fall back to HashMap iteration and can route
                    // it to the wrong graph in a multi-window session.
                    if let Ok(slot) = state::slot_for_window(&state, label) {
                        if state.note_focused(label) {
                            let _ =
                                settings::remember_graph(app, &slot.root_key.display().to_string());
                        }
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    state.graphs.write().unwrap().remove(label);
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
            last_focused: Mutex::new(None),
            capture_graph: Mutex::new(None),
            #[cfg(desktop)]
            next_window: AtomicU64::new(1),
        })
        .setup(|app| {
            // After the single-instance plugin: a forwarded second launch has
            // already exited and cannot rotate the primary's diagnostics.
            // Tauri's app-data path is the sandbox-private home on mobile too.
            if let Ok(dir) = app.path().app_data_dir() {
                flight::persist_init(dir.join("diagnostics"));
            }
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
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            if let Some(window) = app.get_webview_window("main") {
                native_mouse_history::install(&window);
            }
            #[cfg(desktop)]
            schedule_main_window_reveal_fallback(app.handle());
            // Eagerly open the graph if one was configured at startup.
            let startup_root = resolve_root("")
                .or_else(|| graph::usable_last_graph_path(settings::last_graph_path(app.handle())));
            if let Some(root) = startup_root {
                let state = app.state::<AppState>();
                graph::load_graph_for_label(root, app.handle(), "main", &state)?;
                let slot = state::slot_for_window(&state, "main")?;
                // These diagnostics build a whole-graph inventory on the cold-cache critical
                // path to first paint, before warm_cache_async. The format! args
                // are evaluated regardless of whether diag() ends up writing, so
                // gate the whole block on debug to keep it off the 99% hot launch.
                if debug_enabled() {
                    let meta = state::graph_meta(&slot);
                    let config = slot.store.config();
                    let journals = slot.store.scan_area(tine_store::Area::Journals, None).ok();
                    let pages = slot.store.scan_area(tine_store::Area::Pages, None).ok();
                    let inventory = matches!(slot.store.is_graph_ready(), Ok(true))
                        .then(|| slot.store.whole_graph().ok().map(|view| view.inventory()))
                        .flatten();
                    let physical: Vec<_> = inventory
                        .iter()
                        .flat_map(|items| &items.0)
                        .flat_map(|entry| match &entry.target {
                            tine_store::Resolved::Existing { id, others } => std::iter::once(id)
                                .chain(others.iter())
                                .map(|id| (entry, id))
                                .collect::<Vec<_>>(),
                            _ => Vec::new(),
                        })
                        .collect();
                    diag_private("graph-root", format!("graph root: {}", meta.root));
                    diag_private(
                        "graph-inventory",
                        format!(
                            "journals dir: {} (.md files={:?})",
                            config.journals_dir,
                            journals.as_ref().map(|listing| listing
                                .files
                                .iter()
                                .filter(|entry| entry.rel.ends_with(".md"))
                                .count())
                        ),
                    );
                    diag_private(
                        "graph-inventory",
                        format!(
                            "pages dir: {} (.md files={:?})",
                            config.pages_dir,
                            pages.as_ref().map(|listing| listing
                                .files
                                .iter()
                                .filter(|entry| entry.rel.ends_with(".md"))
                                .count())
                        ),
                    );
                    diag_private(
                        "graph-inventory",
                        format!(
                            "journals recognized as dates: {} | total page entries: {}",
                            physical
                                .iter()
                                .filter(|(entry, _)| entry.is_journal && entry.day.is_some())
                                .count(),
                            physical.len()
                        ),
                    );
                    let sample: Vec<_> = physical
                        .iter()
                        .filter(|(entry, id)| entry.is_journal && id.as_str().ends_with(".md"))
                        .filter_map(|(_, id)| id.as_str().rsplit('/').next())
                        .take(3)
                        .collect();
                    diag_private(
                        "graph-inventory",
                        format!("sample journal files: {sample:?}"),
                    );
                }
            } else {
                diag("NO graph root resolved — set TINE_GRAPH=/path/to/graph");
            }
            // Watch for external changes (reads whichever graph is current).
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
            if std::env::args().any(|a| a == "--capture" || a == "capture") {
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
            capture_graph_binding,
            capture_frontend_ready,
            create_graph,
            app_platform,
            local_clock,
            default_graph_parent,
            android_folder_picker::pick_graph_folder,
            android_media::capture_photo,
            android_media::start_recording,
            android_media::stop_recording,
            android_media::cancel_recording,
            android_system_bars::set_system_bar_appearance,
            page_inventory,
            journal_feed_page,
            get_page,
            graph_source_files,
            save_pages,
            resolve_page,
            guide_pages,
            copy_guide_into_graph,
            get_backlink_filter_context,
            get_backlinks,
            get_unlinked_refs,
            warm_done,
            block_ref_counts,
            block_referrers,
            delete_page,
            rename_page,
            publish_html,
            publish_query_plan,
            publish_query,
            publish_live,
            sheet_export_inputs,
            page_print_html,
            export_query_subtrees,
            run_graph_search,
            query_facets,
            query_parse,
            query_print,
            query_og_expressible,
            query_registry,
            query_run,
            query_explain_empty,
            page_icons,
            set_favorites,
            set_default_home,
            set_preferred_workflow,
            set_timetracking_enabled,
            set_show_brackets,
            set_doc_mode_enter_for_new_block,
            set_logical_outdenting,
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
            list_journal_filename_migrations,
            apply_journal_filename_migrations,
            list_sync_conflicts,
            sync_conflict_diff,
            resolve_sync_conflict,
            trash_sync_conflict,
            conflict_inventory,
            vcs_marker_conflict_diff,
            resolve_vcs_marker_conflict,
            live_conflict_diff,
            resolve_live_conflict,
            trash_journal_file,
            read_journal_file,
            get_page_by_path,
            merge_pages,
            rename_file_to_page,
            search,
            quick_switch,
            capture_quick_switch,
            list_templates,
            journal_content_days,
            resolve_block,
            resolve_blocks,
            preview_block,
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
            rollback_pdf_area_image,
            get_backup_keep,
            set_backup_keep,
            get_capture_enter_files,
            set_capture_enter_files,
            get_link_first_match,
            set_link_first_match,
            get_watch_mode,
            set_watch_mode,
            rescan_graph_now,
            watcher_latency_recent,
            list_backups,
            restore_backup,
            load_session,
            drafts::load_drafts,
            drafts::store_draft,
            drafts::retire_draft,
            save_session,
            load_workspaces,
            save_workspaces,
            list_known_graphs,
            forget_known_graph,
            reveal_known_graph,
            install_plugin,
            uninstall_plugin,
            list_installed_plugins,
            read_plugin_entry,
            set_plugin_enabled,
            verify_plugin_registry,
            load_plugin_registry_cache,
            store_plugin_registry_cache,
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
            flight::app_architecture,
            flight::clear_diagnostics,
            flight::diagnostic_frontend_event,
            flight::diagnostic_ipc_event,
            flight::diagnostic_report,
            flight::diagnostic_session_active,
            flight::save_diagnostic_report,
            tine_quit,
            close_graph_window,
            tine_open_devtools
        ])
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                // Queued Concord base-ledger updates get one bounded drain
                // (`EXIT_DRAIN_BUDGET`); the ledger is never a save authority.
                concord_ledger::drain_all_for_exit(&app.state::<AppState>());
                // `App::run` never returns, so the orderly end of a run is
                // here: clear the unclean-exit marker (master d9763603).
                flight::mark_clean_shutdown();
                // tao delivers this callback for WM_ENDSESSION (Windows sign-out,
                // restart, shutdown), but on that path its message loop neither
                // receives WM_QUIT nor switches to an exiting ControlFlow.
                // Returning would leave Tine alive until Windows names it on the
                // "app is preventing shutdown" screen and force-terminates it
                // (GH #455). Nothing else remains to flush: page saves are
                // already durable when they report success, so terminate.
                #[cfg(target_os = "windows")]
                std::process::exit(0);
            }
        });
}

#[cfg(test)]
mod mobile_drawer_policy_tests {
    use super::{
        apply_mobile_drawer_e2e_window_policy, apply_windows_webdriver_window_policy,
        merged_windows_webdriver_args, WRY_WINDOWS_DEFAULT_BROWSER_ARGS,
    };

    #[test]
    fn production_policy_does_not_mutate_the_real_window_config() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let before = context.config_mut().app.windows.clone();
        assert!(!apply_mobile_drawer_e2e_window_policy(
            &mut context.config_mut().app.windows,
            false,
        ));
        assert_eq!(context.config_mut().app.windows, before);
    }

    #[test]
    fn forced_policy_changes_only_main_in_the_real_window_config() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let capture = context
            .config_mut()
            .app
            .windows
            .iter()
            .find(|window| window.label == "capture")
            .expect("configured capture window")
            .clone();
        let mut neighbor = capture.clone();
        neighbor.label = "neighbor".into();
        neighbor.width = 777.0;
        context.config_mut().app.windows.push(neighbor.clone());

        assert!(apply_mobile_drawer_e2e_window_policy(
            &mut context.config_mut().app.windows,
            true,
        ));
        let windows = &context.config_mut().app.windows;
        let main = windows
            .iter()
            .find(|window| window.label == "main")
            .unwrap();
        assert_eq!(
            (main.width, main.height, main.min_width),
            (390.0, 844.0, Some(390.0))
        );
        assert_eq!(
            windows.iter().find(|window| window.label == "capture"),
            Some(&capture)
        );
        assert_eq!(
            windows.iter().find(|window| window.label == "neighbor"),
            Some(&neighbor)
        );
    }

    #[test]
    fn webdriver_arguments_are_inert_without_explicit_automation() {
        assert_eq!(
            merged_windows_webdriver_args(None, false, Some("--remote-debugging-port=9222")),
            None
        );
        assert_eq!(merged_windows_webdriver_args(None, true, Some("  ")), None);
    }

    #[test]
    fn webdriver_arguments_preserve_wry_defaults_and_configured_values() {
        assert_eq!(
            merged_windows_webdriver_args(None, true, Some("--remote-debugging-port=9222")),
            Some(format!(
                "{WRY_WINDOWS_DEFAULT_BROWSER_ARGS} --remote-debugging-port=9222"
            ))
        );
        assert_eq!(
            merged_windows_webdriver_args(
                Some("--disable-gpu"),
                true,
                Some("--remote-debugging-port=9222")
            ),
            Some("--disable-gpu --remote-debugging-port=9222".into())
        );
    }

    #[test]
    fn webdriver_policy_updates_every_configured_window_only_in_automation() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let windows = &mut context.config_mut().app.windows;
        assert!(windows.len() >= 2);
        let window_count = windows.len();
        assert_eq!(
            apply_windows_webdriver_window_policy(
                windows,
                true,
                Some("--remote-debugging-port=9222")
            ),
            window_count
        );
        assert!(windows.iter().all(|window| window
            .additional_browser_args
            .as_deref()
            .is_some_and(|args| args.contains("--remote-debugging-port=9222"))));
    }
}

#[cfg(test)]
mod platform_lifecycle_guard_tests {
    fn lib_source() -> String {
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("read src-tauri/src/lib.rs")
    }

    /// GH #455: on Windows sign-out/shutdown tao runs the `RunEvent::Exit`
    /// callback for WM_ENDSESSION but never leaves its message loop, so the
    /// event loop must terminate the process itself on Windows.
    #[test]
    fn windows_session_end_exit_terminates_the_process() {
        let source = lib_source();
        let run = &source[source.find(".run(|app, event|").expect("the event loop")..];
        let run = &run[..run.find("});").expect("the end of the event loop")];
        assert!(
            run.contains("tauri::RunEvent::Exit")
                && run.contains(
                    "#[cfg(target_os = \"windows\")]\n                std::process::exit(0);"
                ),
            "GH #455: the RunEvent::Exit arm must call std::process::exit(0) on Windows, \
             because WM_ENDSESSION does not break tao's message loop"
        );
    }

    /// Master d9763603: `App::run` never returns, so a clean-shutdown call
    /// placed after it never runs and every relaunch reports an unclean exit.
    /// The orderly end is the `RunEvent::Exit` arm, before Windows' exit(0).
    #[test]
    fn the_exit_arm_clears_the_unclean_exit_marker_before_terminating() {
        let source = lib_source();
        let run = &source[source.find(".run(|app, event|").expect("the event loop")..];
        let run = &run[..run.find("});").expect("the end of the event loop")];
        let clean = run
            .find("flight::mark_clean_shutdown();")
            .expect("RunEvent::Exit must call flight::mark_clean_shutdown()");
        assert!(clean < run.find("std::process::exit(0)").unwrap());
        assert!(source.contains("flight::persist_init(dir.join(\"diagnostics\"))"));
    }

    /// GH #446: the frontend's platform identity comes from the build
    /// (`graph::app_platform`), injected before any frontend code runs, never
    /// from the WebView user agent (iPadOS reports a Mac UA).
    #[test]
    fn frontend_platform_identity_is_injected_from_the_build() {
        let source = lib_source();
        assert!(
            source.contains(
                "\"globalThis.__TINE_PLATFORM__ = {:?};\",\n        crate::graph::app_platform()"
            ),
            "GH #446: lib.rs must inject __TINE_PLATFORM__ from crate::graph::app_platform()"
        );
    }

    /// GH #607: the frontend's calendar is the backend's. The launch offset is
    /// injected before frontend code runs, from the same zone source as
    /// `JournalDate::today`; `local_clock` keeps it current.
    #[test]
    fn frontend_clock_correction_is_injected_from_the_backend_zone() {
        let source = lib_source();
        assert!(
            source.contains("JournalDate::local_utc_offset_now();")
                && source.contains("globalThis.__TINE_LOCAL_CLOCK__ = "),
            "GH #607: lib.rs must inject __TINE_LOCAL_CLOCK__ from JournalDate::local_utc_offset_now()"
        );
    }

    /// GH #572: apps get the WebKit that shipped with macOS (a Safari update
    /// does not change it), and Tine needs the Safari 15.4 engine, which first
    /// shipped in macOS 12.3. The bundle declares that floor so an older Mac is
    /// refused at install/launch instead of running a half-working app.
    #[test]
    fn macos_bundle_declares_the_webkit_floor() {
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tauri.macos.conf.json"
            ))
            .expect("read src-tauri/tauri.macos.conf.json"),
        )
        .expect("tauri.macos.conf.json is JSON");
        assert_eq!(
            config["bundle"]["macOS"]["minimumSystemVersion"], "12.3",
            "GH #572: macOS bundles must require 12.3 (the Safari 15.4 engine)"
        );
    }

    /// GH #241: the updater ships on every desktop target (Windows, Linux,
    /// macOS) and on no mobile target (Android, iOS). Windows uses native-tls
    /// (Schannel) plus Reqwest's system-proxy reader; Linux/macOS keep rustls.
    /// This pins the exact target sections so a cfg edit cannot silently drop
    /// a platform's updater or its transport.
    #[test]
    fn updater_transport_is_declared_for_every_desktop_target() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read src-tauri/Cargo.toml");
        let mut section = "";
        let mut updater_sections = Vec::new();
        let mut windows_lines = Vec::new();
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = line;
                continue;
            }
            if line.starts_with("tauri-plugin-updater") {
                updater_sections.push((section, line));
            }
            if section == "[target.'cfg(windows)'.dependencies]" && !line.starts_with('#') {
                windows_lines.push(line);
            }
        }
        assert_eq!(
            updater_sections,
            vec![
                (
                    "[target.'cfg(windows)'.dependencies]",
                    "tauri-plugin-updater = { version = \"2\", default-features = false, features = [\"native-tls\", \"zip\"] }",
                ),
                (
                    "[target.'cfg(all(not(target_os = \"android\"), not(target_os = \"ios\"), not(target_os = \"windows\")))'.dependencies]",
                    "tauri-plugin-updater = \"2\"",
                ),
            ],
            "GH #241: the updater must be declared once for Windows (native-tls) and once for \
             Linux/macOS (rustls), and never for Android/iOS"
        );
        assert!(
            windows_lines.contains(
                &"reqwest = { version = \"0.13\", default-features = false, features = [\"system-proxy\"] }"
            ),
            "GH #241: the Windows updater must follow the system proxy via reqwest's system-proxy feature"
        );
    }
}
