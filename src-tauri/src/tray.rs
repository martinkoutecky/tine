//! Desktop system tray (GH #625): a tray icon, "minimize to tray" and "start
//! minimized to tray". Three device settings, all OFF by default, stored in
//! `tine-settings.json` through the ordinary `get_app_bool`/`set_app_bool`
//! door (like `native_window_frame`); nothing is graph data.
//!
//! **Question answered.** [`behaviour`] -- given the stored preferences and
//! whether a tray icon actually exists, what do minimize and cold start do?
//! It is the only answerer: minimize-to-tray and start-minimized are both
//! "off for this run" whenever the icon could not be created, so a desktop
//! without a StatusNotifier host never ends up with an invisible app.
//!
//! The windows. Only `main` ever hides. Closing is unchanged (it quits through
//! the frontend's flush handler), workspace windows and other graph windows are
//! never touched by minimize-to-tray, and Quit closes every graph window
//! through that same handler instead of exiting the process directly.
//!
//! The tray never changes the application's activation policy: minimize to
//! tray hides the window only, on every platform (Martin/coordinator decision,
//! 2026-10-06; macOS keeps its Dock icon, menu bar and Cmd-Tab entry).
//!
//! A tray that disappears while Tine runs (the StatusNotifier host quits or
//! crashes) must not strand a hidden window: [`desktop::host_lost`] shows
//! `main` and turns both behaviours off until the icon is created again.
//!
//! Mobile has no tray: the module compiles everywhere (the status command and
//! the stable keys); the decision functions and every tauri tray call are
//! `desktop` only.

use serde::Serialize;

/// Device-settings keys (tine-settings.json), read natively at startup and by
/// the Settings controls through `get_app_bool` / `set_app_bool`.
#[cfg(any(desktop, test))]
pub(crate) const SHOW_KEY: &str = "tray_show";
#[cfg(any(desktop, test))]
pub(crate) const MINIMIZE_KEY: &str = "tray_minimize";
#[cfg(any(desktop, test))]
pub(crate) const START_MINIMIZED_KEY: &str = "tray_start_minimized";

#[cfg(desktop)]
const TRAY_ID: &str = "tine-tray";
#[cfg(desktop)]
const MENU_OPEN: &str = "tray-open";
#[cfg(desktop)]
const MENU_CAPTURE: &str = "tray-capture";
#[cfg(desktop)]
const MENU_QUIT: &str = "tray-quit";
/// How long after Quit the windows are surfaced if the process is still alive:
/// a flush prompt or a "still saving" notice in a hidden window would
/// otherwise be invisible. A flush that finishes sooner never flashes a window.
#[cfg(desktop)]
const QUIT_SURFACE_AFTER_MS: u64 = 1_500;
/// On Windows the click on the notification icon moves focus off `main` before
/// the click event arrives, so "main was focused" means "lost focus just now".
#[cfg(desktop)]
const CLICK_FOCUS_GRACE_MS: u64 = 500;
/// How long a start-minimized launch waits for a tray host that is not up yet
/// (autostart can run before the panel) before it shows the window.
#[cfg(target_os = "linux")]
const HOST_WAIT_MS: u64 = 8_000;
#[cfg(target_os = "linux")]
const HOST_POLL_MS: u64 = 500;

#[cfg(any(desktop, test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Prefs {
    pub(crate) show: bool,
    pub(crate) minimize: bool,
    pub(crate) start_minimized: bool,
}

/// What the tray settings mean for this run.
#[cfg(any(desktop, test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Behaviour {
    pub(crate) minimize_to_tray: bool,
    pub(crate) start_hidden: bool,
}

/// The one answerer. `tray_present` is "an icon exists right now", not "the
/// user asked for one": a request the platform could not honour changes
/// nothing about how the windows behave.
#[cfg(any(desktop, test))]
pub(crate) fn behaviour(prefs: Prefs, tray_present: bool) -> Behaviour {
    let live = prefs.show && tray_present;
    Behaviour {
        minimize_to_tray: live && prefs.minimize,
        start_hidden: live && prefs.start_minimized,
    }
}

/// Whether a cold start leaves `main` hidden. An explicit request to open
/// something (a graph path or a `tine:` link) always shows the window: the
/// user asked to see it. A bare launch, an autostart entry and a cold
/// `--capture` start hidden.
#[cfg(desktop)]
pub(crate) fn starts_hidden(behaviour: Behaviour, launch: &crate::cli::LaunchRequest) -> bool {
    use crate::cli::LaunchRequest;
    behaviour.start_hidden && matches!(launch, LaunchRequest::Focus | LaunchRequest::Capture)
}

/// A cold start that asked for a hidden window but found no tray host yet
/// (an autostart entry can run before the panel does) waits for one instead of
/// giving up at once. Only a host that is merely late is worth waiting for: a
/// missing library or a launch that opens something shows the window now.
#[cfg(desktop)]
pub(crate) fn waits_for_host(
    prefs: Prefs,
    launch: &crate::cli::LaunchRequest,
    library_present: bool,
    tray_present: bool,
) -> bool {
    use crate::cli::LaunchRequest;
    prefs.show
        && prefs.start_minimized
        && library_present
        && !tray_present
        && matches!(launch, LaunchRequest::Focus | LaunchRequest::Capture)
}

/// Whether a second launch (or any "bring the app back" request) must reveal
/// `main`: only when this feature hid it. A window the user minimized
/// themselves, or left visible, is never touched, so with every tray setting
/// off a second launch behaves exactly as it did before the tray existed.
#[cfg(any(desktop, test))]
pub(crate) fn should_reveal(
    start_hidden_pending: bool,
    hidden_by_tray: bool,
    visible: bool,
) -> bool {
    start_hidden_pending || (hidden_by_tray && !visible)
}

#[cfg(any(desktop, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClickAction {
    Present,
    Hide,
}

/// A left click on the icon: hide `main` only when it is already visible and
/// focused, otherwise bring it to the front (unminimize, show, focus).
#[cfg(any(desktop, test))]
pub(crate) fn left_click_action(visible: bool, minimized: bool, focused: bool) -> ClickAction {
    if visible && !minimized && focused {
        ClickAction::Hide
    } else {
        ClickAction::Present
    }
}

/// What the Settings controls need to know after a change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TrayStatus {
    /// This build can show a tray icon at all (desktop).
    pub(crate) supported: bool,
    /// An icon exists right now.
    pub(crate) active: bool,
    /// The icon was requested and could not be created (fixed wording).
    pub(crate) problem: Option<String>,
}

#[cfg(any(desktop, test))]
pub(crate) const NO_TRAY_HOST: &str = "No system tray was found on this desktop, so the tray \
options are off for now. Tine stays in the window list.";

#[cfg(any(desktop, test))]
pub(crate) fn read_prefs(app: &tauri::AppHandle) -> Prefs {
    Prefs {
        show: crate::settings::device_bool(app, SHOW_KEY, false),
        minimize: crate::settings::device_bool(app, MINIMIZE_KEY, false),
        start_minimized: crate::settings::device_bool(app, START_MINIMIZED_KEY, false),
    }
}

/// Create/remove the tray icon to match the stored preferences and report the
/// outcome. Mobile has no tray.
#[tauri::command]
pub(crate) async fn tray_apply(app: tauri::AppHandle) -> Result<TrayStatus, String> {
    #[cfg(desktop)]
    {
        crate::state::off_ui(move || Ok(desktop::apply(&app))).await
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(TrayStatus::default())
    }
}

/// Where the Linux tray icon file must be written for the desktop's
/// StatusNotifier host to read it. tray-icon writes the PNG to disk and hands
/// the PATH to libayatana-appindicator, so the host (outside any sandbox) opens
/// that path itself. Inside a Flatpak, `$XDG_RUNTIME_DIR` is the sandbox's own
/// directory and the host cannot read it; the one subdirectory shared with the
/// host at the same path is `$XDG_RUNTIME_DIR/app/$FLATPAK_ID`. `None` outside
/// a Flatpak keeps tray-icon's default (`$XDG_RUNTIME_DIR/tray-icon`).
#[cfg(target_os = "linux")]
pub(crate) fn flatpak_icon_dir(
    flatpak_id: Option<&str>,
    runtime_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let id = flatpak_id.filter(|id| !id.is_empty())?;
    let runtime = runtime_dir.filter(|dir| dir.is_absolute())?;
    Some(runtime.join("app").join(id))
}

#[cfg(desktop)]
pub(crate) use desktop::{
    init, main_start_hidden_pending, reveal_main_if_hidden, window_event, TrayState,
};

#[cfg(desktop)]
mod desktop {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::menu::{Menu, MenuEvent, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, Manager};

    #[derive(Default)]
    pub(crate) struct TrayState {
        behaviour: Mutex<Behaviour>,
        /// `main` was created hidden by start-minimized and nobody has
        /// presented it yet. The frontend reveal and the native fallback
        /// both stand down while this is set.
        start_hidden_pending: AtomicBool,
        /// `main` was hidden by the tray (minimize to tray, or a left click)
        /// and has not been shown since: only then does a second launch
        /// reveal it.
        hidden_by_tray: AtomicBool,
        main_blurred_at: Mutex<Option<Instant>>,
        problem: Mutex<Option<String>>,
    }

    impl TrayState {
        fn behaviour(&self) -> Behaviour {
            *self.behaviour.lock().unwrap_or_else(|e| e.into_inner())
        }
        fn set_behaviour(&self, value: Behaviour) {
            *self.behaviour.lock().unwrap_or_else(|e| e.into_inner()) = value;
        }
        fn problem(&self) -> Option<String> {
            self.problem
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
        fn set_problem(&self, value: Option<String>) {
            *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = value;
        }
    }

    fn state(app: &AppHandle) -> tauri::State<'_, TrayState> {
        app.state::<TrayState>()
    }

    pub(crate) fn tray_present(app: &AppHandle) -> bool {
        app.tray_by_id(TRAY_ID).is_some()
    }

    /// Setup-time entry: create the icon when it is requested, and decide
    /// whether `main` is created hidden. Returns that decision for
    /// `workspace_windows::create_main`. Never fails: a tray that cannot be
    /// created is a diagnostic and a visible window.
    pub(crate) fn init(app: &tauri::App) -> bool {
        let handle = app.handle();
        let prefs = read_prefs(handle);
        if prefs.show {
            create_icon_noting_problem(handle);
        }
        let launch = crate::cli::launch_request_env();
        let waiting = wait_wanted(prefs, &launch, tray_present(handle));
        let behaviour = if waiting {
            // Provisionally hidden: the host is late, not missing.
            Behaviour {
                minimize_to_tray: false,
                start_hidden: true,
            }
        } else {
            behaviour(prefs, tray_present(handle))
        };
        state(handle).set_behaviour(behaviour);
        let hidden = starts_hidden(behaviour, &launch);
        if waiting {
            wait_for_host(handle.clone());
        }
        state(handle)
            .start_hidden_pending
            .store(hidden, Ordering::SeqCst);
        if hidden {
            crate::debug::diag("tray-start-minimized");
        }
        hidden
    }

    /// True while `main` is hidden by start-minimized and not yet presented.
    pub(crate) fn main_start_hidden_pending(app: &AppHandle) -> bool {
        state(app).start_hidden_pending.load(Ordering::SeqCst)
    }

    #[cfg(target_os = "linux")]
    fn wait_wanted(prefs: Prefs, launch: &crate::cli::LaunchRequest, present: bool) -> bool {
        waits_for_host(
            prefs,
            launch,
            linux_host::library_present().is_ok(),
            present,
        )
    }
    #[cfg(not(target_os = "linux"))]
    fn wait_wanted(_: Prefs, _: &crate::cli::LaunchRequest, _: bool) -> bool {
        false
    }

    /// Poll for a StatusNotifier host while `main` is held hidden. A host
    /// that shows up gets the icon; none within [`HOST_WAIT_MS`] shows the
    /// window, because an invisible app is worse than a visible one.
    #[cfg(target_os = "linux")]
    fn wait_for_host(app: AppHandle) {
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(HOST_WAIT_MS);
            loop {
                if !read_prefs(&app).show {
                    break;
                }
                if linux_host::check().is_ok() {
                    let (tx, rx) = mpsc::channel();
                    let on_main = app.clone();
                    let queued = app.run_on_main_thread(move || {
                        create_icon_noting_problem(&on_main);
                        let _ = tx.send(());
                    });
                    if queued.is_ok() {
                        let _ = rx.recv_timeout(Duration::from_secs(10));
                    }
                    if tray_present(&app) {
                        state(&app).set_behaviour(behaviour(read_prefs(&app), true));
                        return;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(HOST_POLL_MS));
            }
            let on_main = app.clone();
            let _ = app.run_on_main_thread(move || {
                state(&on_main).set_behaviour(Behaviour::default());
                if main_start_hidden_pending(&on_main) {
                    crate::debug::diag("tray-host-wait-expired");
                    present_main(&on_main);
                }
            });
        });
    }
    #[cfg(not(target_os = "linux"))]
    fn wait_for_host(_: AppHandle) {}

    fn create_icon_noting_problem(app: &AppHandle) {
        match create_icon(app) {
            Ok(()) => state(app).set_problem(None),
            Err(reason) => {
                crate::debug::diag_private("tray-unavailable", &reason);
                state(app).set_problem(Some(NO_TRAY_HOST.to_string()));
            }
        }
    }

    pub(crate) fn apply(app: &AppHandle) -> TrayStatus {
        let prefs = read_prefs(app);
        if prefs.show && !tray_present(app) {
            let (tx, rx) = mpsc::channel();
            let for_main = app.clone();
            let queued = app.run_on_main_thread(move || {
                create_icon_noting_problem(&for_main);
                let _ = tx.send(());
            });
            if queued.is_err() || rx.recv_timeout(Duration::from_secs(10)).is_err() {
                crate::debug::diag("tray-create-timeout");
                state(app).set_problem(Some(NO_TRAY_HOST.to_string()));
            }
        } else if !prefs.show && tray_present(app) {
            let for_main = app.clone();
            let (tx, rx) = mpsc::channel();
            let queued = app.run_on_main_thread(move || {
                // Never strand a window that only the icon could bring back.
                if stranded(&for_main) {
                    present_main(&for_main);
                }
                let _ = for_main.remove_tray_by_id(TRAY_ID);
                let _ = tx.send(());
            });
            if queued.is_ok() {
                let _ = rx.recv_timeout(Duration::from_secs(10));
            }
            state(app).set_problem(None);
        } else if !prefs.show {
            state(app).set_problem(None);
        }
        let present = tray_present(app);
        let behaviour = behaviour(prefs, present);
        state(app).set_behaviour(behaviour);
        TrayStatus {
            supported: true,
            active: present,
            problem: if prefs.show && !present {
                state(app)
                    .problem()
                    .or_else(|| Some(NO_TRAY_HOST.to_string()))
            } else {
                None
            },
        }
    }

    /// `main` is hidden and only the tray could bring it back.
    fn stranded(app: &AppHandle) -> bool {
        let visible = app
            .get_webview_window("main")
            .is_none_or(|window| window.is_visible().unwrap_or(true));
        should_reveal(
            main_start_hidden_pending(app),
            state(app).hidden_by_tray.load(Ordering::SeqCst),
            visible,
        )
    }

    /// Show, unminimize and focus `main` (or the first graph window if `main`
    /// is gone). Only the one window is touched: workspace windows and other
    /// graph windows stay as they are.
    pub(crate) fn present_main(app: &AppHandle) {
        state(app)
            .start_hidden_pending
            .store(false, Ordering::SeqCst);
        state(app).hidden_by_tray.store(false, Ordering::SeqCst);
        let window = app.get_webview_window("main").or_else(|| {
            let mut graphs: Vec<_> = app
                .webview_windows()
                .into_iter()
                .filter(|(label, _)| label.starts_with("graph-"))
                .collect();
            graphs.sort_by(|a, b| a.0.cmp(&b.0));
            graphs.into_iter().next().map(|(_, window)| window)
        });
        let Some(window) = window else {
            return;
        };
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }

    /// A second launch shows `main` only when the tray (or a start-minimized
    /// launch) hid it; anything else is left to the ordinary
    /// focus-last-window path, exactly as before the tray existed.
    pub(crate) fn reveal_main_if_hidden(app: &AppHandle) {
        if stranded(app) {
            present_main(app);
        }
    }

    fn hide_main(app: &AppHandle) {
        if let Some(window) = app.get_webview_window("main") {
            state(app).hidden_by_tray.store(true, Ordering::SeqCst);
            let _ = window.hide();
        }
    }

    /// The tray host left while Tine was running (the panel quit or crashed):
    /// nothing can bring a hidden window back any more. Show `main`, drop the
    /// icon and run without minimize-to-tray / start-hidden until an explicit
    /// settings apply finds a host again.
    #[cfg(target_os = "linux")]
    pub(crate) fn host_lost(app: &AppHandle) {
        if !tray_present(app) {
            return;
        }
        crate::debug::diag("tray-host-lost");
        if stranded(app) {
            present_main(app);
        }
        let _ = app.remove_tray_by_id(TRAY_ID);
        let tray = state(app);
        tray.set_behaviour(Behaviour::default());
        tray.set_problem(Some(NO_TRAY_HOST.to_string()));
    }

    fn left_click(app: &AppHandle) {
        let Some(window) = app.get_webview_window("main") else {
            present_main(app);
            return;
        };
        let recently_blurred = state(app)
            .main_blurred_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|at| at.elapsed() <= Duration::from_millis(CLICK_FOCUS_GRACE_MS));
        let focused = window.is_focused().unwrap_or(false) || recently_blurred;
        match left_click_action(
            window.is_visible().unwrap_or(false),
            window.is_minimized().unwrap_or(false),
            focused,
        ) {
            ClickAction::Hide => hide_main(app),
            ClickAction::Present => present_main(app),
        }
    }

    fn menu_event(app: &AppHandle, event: MenuEvent) {
        match event.id().as_ref() {
            MENU_OPEN => present_main(app),
            MENU_CAPTURE => crate::show_capture(app),
            MENU_QUIT => quit(app),
            _ => {}
        }
    }

    /// Quit exactly as closing the windows does: every graph window gets an
    /// ordinary close request, so the frontend's flush handler runs (session,
    /// pending saves, workspace windows) and the last window exits the
    /// process. No path here exits the process itself. A close that needs the
    /// user (a discard prompt, "still saving") would be invisible in a hidden
    /// window, so the windows are surfaced if the app is still running after
    /// a moment.
    pub(crate) fn quit(app: &AppHandle) {
        crate::debug::diag("tray-quit");
        for (label, window) in app.webview_windows() {
            if label == "main" || label.starts_with("graph-") {
                let _ = window.close();
            }
        }
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(QUIT_SURFACE_AFTER_MS));
            let surface = app.clone();
            let _ = app.run_on_main_thread(move || {
                for (label, window) in surface.webview_windows() {
                    if label == "main" || label.starts_with("graph-") {
                        let _ = window.unminimize();
                        let _ = window.show();
                    }
                }
            });
        });
    }

    fn create_icon(app: &AppHandle) -> Result<(), String> {
        if tray_present(app) {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        linux_host::check()?;
        let icon = app
            .default_window_icon()
            .cloned()
            .ok_or_else(|| "no application icon".to_string())?;
        let open = MenuItem::with_id(app, MENU_OPEN, "Open Tine", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let capture = MenuItem::with_id(app, MENU_CAPTURE, "Quick Capture", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)
            .map_err(|e| e.to_string())?;
        let menu = Menu::with_items(app, &[&open, &capture, &quit]).map_err(|e| e.to_string())?;
        let title = app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| "Tine".to_string());
        // Linux: `linux_host::check` has already proved the AppIndicator
        // library loads, which is what would otherwise make the backend panic.
        let builder = TrayIconBuilder::with_id(TRAY_ID)
            .icon(icon)
            .tooltip(title)
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(menu_event)
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    left_click(tray.app_handle());
                }
            });
        // Inside a Flatpak the icon file must live where the host can read it.
        #[cfg(target_os = "linux")]
        let builder = match super::flatpak_icon_dir(
            std::env::var("FLATPAK_ID").ok().as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .as_deref(),
        ) {
            Some(dir) => builder.temp_dir_path(dir),
            None => builder,
        };
        builder.build(app).map_err(|error| error.to_string())?;
        #[cfg(target_os = "linux")]
        linux_host::watch(app);
        Ok(())
    }

    /// Window-event hook (called for every window before lib.rs's own
    /// handling). Only `main` matters: it hides when minimized (minimize to
    /// tray) and remembers when it last lost focus (the tray click grace).
    pub(crate) fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
        if window.label() != "main" {
            return;
        }
        let app = window.app_handle();
        let Some(tray) = app.try_state::<TrayState>() else {
            return;
        };
        match event {
            tauri::WindowEvent::Focused(true) => {
                // Any path that shows and focuses main (a second launch, a
                // link, the tray) leaves it presented.
                tray.start_hidden_pending.store(false, Ordering::SeqCst);
                tray.hidden_by_tray.store(false, Ordering::SeqCst);
            }
            tauri::WindowEvent::Focused(false) => {
                *tray
                    .main_blurred_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                // macOS reports no resize for a miniaturize; check once it
                // has settled.
                #[cfg(target_os = "macos")]
                if tray.behaviour().minimize_to_tray {
                    let app = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(500));
                        let again = app.clone();
                        let _ = app.run_on_main_thread(move || hide_if_minimized(&again));
                    });
                }
            }
            tauri::WindowEvent::Resized(_) => {
                // Linux (iconified state change) and Windows (WM_SIZE with
                // SIZE_MINIMIZED) both deliver a resize when main is minimized.
                if tray.behaviour().minimize_to_tray {
                    hide_if_minimized(app);
                }
            }
            _ => {}
        }
    }

    fn hide_if_minimized(app: &AppHandle) {
        let behaviour = state(app).behaviour();
        if !behaviour.minimize_to_tray || !tray_present(app) {
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        if window.is_minimized().unwrap_or(false) {
            crate::debug::diag("tray-minimize-to-tray");
            hide_main(app);
        }
    }

    /// Linux has no tray of its own: the icon is shown by a StatusNotifier
    /// host (KDE, XFCE, GNOME with the AppIndicator extension, waybar...).
    /// libayatana-appindicator "succeeds" without a host and the icon simply
    /// never appears, which would leave a minimized window unreachable. So the
    /// host is checked on the session bus first, and the library is probed so
    /// that its absence is an error and not a panic.
    #[cfg(target_os = "linux")]
    mod linux_host {
        use gtk::gio;
        use gtk::glib::{ToVariant, VariantTy};
        use std::sync::atomic::{AtomicBool, Ordering};
        use tauri::AppHandle;

        const WATCHER: &str = "org.kde.StatusNotifierWatcher";

        fn session() -> Result<gio::DBusConnection, String> {
            gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                .map_err(|e| format!("session bus unavailable: {e}"))
        }

        fn watcher_owned(connection: &gio::DBusConnection) -> Result<bool, String> {
            let owned = connection
                .call_sync(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "NameHasOwner",
                    Some(&(WATCHER,).to_variant()),
                    Some(VariantTy::new("(b)").map_err(|e| e.to_string())?),
                    gio::DBusCallFlags::NONE,
                    1_500,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| format!("StatusNotifierWatcher lookup failed: {e}"))?;
            Ok(owned.child_value(0).get::<bool>().unwrap_or(false))
        }

        pub(super) fn check() -> Result<(), String> {
            library_present()?;
            let connection = session()?;
            if !watcher_owned(&connection)? {
                return Err("no StatusNotifierWatcher on the session bus".to_string());
            }
            // A watcher without a registered host shows nothing either. A
            // watcher that does not answer the property is given the benefit
            // of the doubt.
            let registered = connection.call_sync(
                Some(WATCHER),
                "/StatusNotifierWatcher",
                "org.freedesktop.DBus.Properties",
                "Get",
                Some(&(WATCHER, "IsStatusNotifierHostRegistered").to_variant()),
                Some(VariantTy::new("(v)").map_err(|e| e.to_string())?),
                gio::DBusCallFlags::NONE,
                1_500,
                gio::Cancellable::NONE,
            );
            if let Ok(reply) = registered {
                let inner = reply.child_value(0).as_variant();
                if inner.and_then(|v| v.get::<bool>()) == Some(false) {
                    return Err("no StatusNotifier host is registered".to_string());
                }
            }
            Ok(())
        }

        /// Watch the session bus for the StatusNotifierWatcher losing its owner
        /// (NameOwnerChanged with an empty new owner): the panel that hosted the
        /// icon quit or crashed. Subscribes once per process, from the GTK main
        /// thread (the callback runs there), and re-checks right after
        /// subscribing so a loss between `check` and now is not missed.
        pub(super) fn watch(app: &AppHandle) {
            static WATCHING: AtomicBool = AtomicBool::new(false);
            if WATCHING.swap(true, Ordering::SeqCst) {
                return;
            }
            let Ok(connection) = session() else {
                return;
            };
            let handle = app.clone();
            let _ = connection.signal_subscribe(
                Some("org.freedesktop.DBus"),
                Some("org.freedesktop.DBus"),
                Some("NameOwnerChanged"),
                Some("/org/freedesktop/DBus"),
                Some(WATCHER),
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, parameters| {
                    let new_owner = parameters
                        .child_value(2)
                        .get::<String>()
                        .unwrap_or_default();
                    if new_owner.is_empty() {
                        super::host_lost(&handle);
                    }
                },
            );
            if matches!(watcher_owned(&connection), Ok(false)) {
                super::host_lost(app);
            }
        }

        /// The libraries libappindicator-sys loads, in its order.
        pub(super) fn library_present() -> Result<(), String> {
            for name in [c"libayatana-appindicator3.so.1", c"libappindicator3.so.1"] {
                // SAFETY: dlopen of a shared library by soname; the handle is
                // deliberately kept (the tray library loads the same one).
                let handle =
                    unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
                if !handle.is_null() {
                    return Ok(());
                }
            }
            Err("libayatana-appindicator3 is not installed".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Prefs = Prefs {
        show: true,
        minimize: true,
        start_minimized: true,
    };

    #[test]
    fn everything_is_off_by_default() {
        assert_eq!(
            Prefs::default(),
            Prefs {
                show: false,
                minimize: false,
                start_minimized: false
            }
        );
        assert_eq!(behaviour(Prefs::default(), false), Behaviour::default());
        assert_eq!(behaviour(Prefs::default(), true), Behaviour::default());
    }

    #[test]
    fn minimize_and_start_hidden_need_the_icon_setting() {
        let without_icon_setting = Prefs { show: false, ..ALL };
        assert_eq!(behaviour(without_icon_setting, true), Behaviour::default());
        assert_eq!(
            behaviour(ALL, true),
            Behaviour {
                minimize_to_tray: true,
                start_hidden: true
            }
        );
        assert_eq!(
            behaviour(
                Prefs {
                    start_minimized: false,
                    ..ALL
                },
                true
            ),
            Behaviour {
                minimize_to_tray: true,
                start_hidden: false
            }
        );
        assert_eq!(
            behaviour(
                Prefs {
                    minimize: false,
                    ..ALL
                },
                true
            ),
            Behaviour {
                minimize_to_tray: false,
                start_hidden: true
            }
        );
    }

    /// Never an invisible app: when the icon could not be created, every
    /// request leaves the window visible and minimize behaves as today.
    #[test]
    fn a_tray_that_could_not_be_created_shows_the_window() {
        assert_eq!(behaviour(ALL, false), Behaviour::default());
    }

    #[cfg(desktop)]
    #[test]
    fn only_a_bare_or_capture_launch_starts_hidden() {
        use crate::cli::LaunchRequest;
        let hidden = behaviour(ALL, true);
        assert!(starts_hidden(hidden, &LaunchRequest::Focus));
        assert!(starts_hidden(hidden, &LaunchRequest::Capture));
        assert!(!starts_hidden(
            hidden,
            &LaunchRequest::Open("/tmp/graph".into())
        ));
        assert!(!starts_hidden(
            hidden,
            &LaunchRequest::Link("tine://x".into())
        ));
        // No tray: never hidden, whatever was launched.
        let shown = behaviour(ALL, false);
        assert!(!starts_hidden(shown, &LaunchRequest::Focus));
        assert!(!starts_hidden(shown, &LaunchRequest::Capture));
    }

    #[test]
    fn left_click_hides_only_a_visible_focused_window() {
        assert_eq!(left_click_action(true, false, true), ClickAction::Hide);
        assert_eq!(left_click_action(true, false, false), ClickAction::Present);
        assert_eq!(left_click_action(true, true, true), ClickAction::Present);
        assert_eq!(left_click_action(false, false, false), ClickAction::Present);
        assert_eq!(left_click_action(false, false, true), ClickAction::Present);
    }

    #[test]
    fn the_tray_menu_and_settings_keys_are_stable() {
        assert_eq!(SHOW_KEY, "tray_show");
        assert_eq!(MINIMIZE_KEY, "tray_minimize");
        assert_eq!(START_MINIMIZED_KEY, "tray_start_minimized");
        let source = include_str!("tray.rs");
        for label in ["\"Open Tine\"", "\"Quick Capture\"", "\"Quit\""] {
            assert!(
                source.contains(label),
                "tray menu item {label} is part of the contract"
            );
        }
    }

    #[test]
    fn a_second_launch_reveals_only_what_the_tray_hid() {
        // Every setting off: nothing is pending or tray-hidden, so a window the
        // user minimized (not visible per the WM, or iconified) is left alone.
        assert!(!should_reveal(false, false, false));
        assert!(!should_reveal(false, false, true));
        // The tray hid it and it is still hidden.
        assert!(should_reveal(false, true, false));
        // The tray hid it, but something else already showed it.
        assert!(!should_reveal(false, true, true));
        // Start-minimized and nobody has presented it yet.
        assert!(should_reveal(true, false, false));
    }

    #[cfg(desktop)]
    #[test]
    fn only_a_late_host_is_waited_for() {
        use crate::cli::LaunchRequest;
        let wait = |prefs, launch: &LaunchRequest, lib, present| {
            waits_for_host(prefs, launch, lib, present)
        };
        assert!(wait(ALL, &LaunchRequest::Focus, true, false));
        assert!(wait(ALL, &LaunchRequest::Capture, true, false));
        // Library missing: no host could ever appear.
        assert!(!wait(ALL, &LaunchRequest::Focus, false, false));
        // Already present, or the window is not meant to be hidden anyway.
        assert!(!wait(ALL, &LaunchRequest::Focus, true, true));
        assert!(!wait(
            Prefs {
                start_minimized: false,
                ..ALL
            },
            &LaunchRequest::Focus,
            true,
            false
        ));
        assert!(!wait(
            Prefs { show: false, ..ALL },
            &LaunchRequest::Focus,
            true,
            false
        ));
        assert!(!wait(
            ALL,
            &LaunchRequest::Open("/tmp/g".into()),
            true,
            false
        ));
    }

    /// Martin/coordinator, 2026-10-06: minimize to tray hides the window only.
    /// The activation policy (macOS Dock) is never changed, so a graph window
    /// keeps its Dock icon, menu bar and Cmd-Tab entry while main is hidden.
    #[test]
    fn the_tray_never_changes_the_activation_policy() {
        let source = include_str!("tray.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "set_dock_visibility",
            "set_activation_policy",
            "ActivationPolicy",
        ] {
            assert!(
                !code.contains(forbidden),
                "tray code must not call {forbidden}: hide the window only"
            );
        }
    }

    /// The tray icon is a file the HOST reads by path, so inside a Flatpak it
    /// must go where the sandbox shares a directory with the host.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_flatpak_tray_icon_is_written_where_the_host_can_read_it() {
        use std::path::Path;
        let run = Path::new("/run/user/1000");
        assert_eq!(
            flatpak_icon_dir(Some("org.example.App"), Some(run)),
            Some(Path::new("/run/user/1000/app/org.example.App").to_path_buf())
        );
        // Not a Flatpak: tray-icon's own default stays.
        assert_eq!(flatpak_icon_dir(None, Some(run)), None);
        assert_eq!(flatpak_icon_dir(Some(""), Some(run)), None);
        // No usable runtime dir: no guess.
        assert_eq!(flatpak_icon_dir(Some("org.example.App"), None), None);
        assert_eq!(
            flatpak_icon_dir(Some("org.example.App"), Some(Path::new("relative"))),
            None
        );
    }

    /// The builder must actually use the decision (the helper alone proves nothing).
    #[cfg(target_os = "linux")]
    #[test]
    fn the_linux_tray_builder_uses_the_flatpak_icon_dir() {
        let source = include_str!("tray.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        assert!(code.contains("super::flatpak_icon_dir("));
        assert!(code.contains(".temp_dir_path(dir)"));
    }

    /// A tray that disappears at runtime must not strand a hidden window: the
    /// watcher subscription reaches `host_lost`, which shows main.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_linux_tray_watches_for_its_host_leaving() {
        let source = include_str!("tray.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        assert!(code.contains("\"NameOwnerChanged\""));
        assert!(code.contains("super::host_lost(&handle)"));
        assert!(code.contains("linux_host::watch(app)"));
    }

    /// Quit must not skip the flush: it only issues ordinary close requests
    /// (the frontend's onCloseRequested handler flushes and then exits the
    /// process). Architectural fact enforced by source: no exit call inside
    /// the desktop tray code.
    #[test]
    fn tray_quit_closes_windows_and_never_exits_the_process() {
        let source = include_str!("tray.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["app.exit(", ".exit(0)", "std::process::exit", "tine_quit"] {
            assert!(!code.contains(forbidden), "tray code must not call {forbidden}: quitting goes through the windows' close handler");
        }
        assert!(code.contains("window.close()"));
    }

    /// AGENTS.md section 2: a platform cfg list must name every shipped target
    /// or it silently selects a fallback. The tray's platform split is
    /// `desktop` (Linux, Windows, macOS: tray) versus mobile (Android, iOS:
    /// nothing); the only per-OS arms are the Linux host probe/watch/wait and
    /// the macOS minimize check, each with an explicit counterpart, and no
    /// list names a subset.
    #[test]
    fn tray_cfgs_never_name_a_partial_platform_list() {
        let source = include_str!("tray.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("target_os = \"android\"") && !code.contains("target_os = \"ios\""));
        assert!(
            !code.contains("cfg(any(target_os"),
            "name platforms through `desktop`, not an os list"
        );
    }
}
