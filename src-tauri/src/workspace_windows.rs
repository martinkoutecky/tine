//! Workspace windows (OG-MULTIWINDOW): secondary native windows that show more
//! panes of the SAME graph over the opener's one JS context.
//!
//! The opener's JS calls `window.open("about:blank")`; the native popup is
//! built here in `on_new_window`. All script runs in the opener's heap, so a
//! popup has no store, save engine or IPC of its own: `ws-*` labels are in no
//! capability file, and every command a popup's UI triggers is invoked by its
//! opener. This module owns only the native lifecycle:
//!
//! - `workspace_window_prepare` arms ONE popup for the calling window and names
//!   its label; `on_new_window` refuses any open it did not arm (plugin or web
//!   content cannot create native windows through this door).
//! - A popup's native close is held and handed to its opener, which disposes
//!   the window's UI (ending an edit in it, so typed text lands in the store)
//!   and then calls `workspace_window_destroy`. A second close request after
//!   [`INSISTED_CLOSE`] closes it without waiting (a stuck opener must not
//!   trap a window on screen).
//! - When an opener reloads or is destroyed, its popups are destroyed with it.
//!
//! Desktop only (Linux, Windows, macOS). On Android and iOS the two commands
//! exist and refuse, nothing else is reachable, and the frontend hides the
//! command there.
#![cfg_attr(mobile, allow(dead_code))]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

/// Popups one opener may hold at a time (I-22). The frontend enforces the same
/// bound (`MAX_WORKSPACE_WINDOWS` in src/windowRealm.ts; a doc-code test pins the two).
pub(crate) const MAX_WORKSPACE_WINDOWS: usize = 8;
/// Window between two close requests after which a popup closes even if its
/// opener has not answered the first.
const INSISTED_CLOSE: Duration = Duration::from_secs(3);
/// The event the opener receives when the user closes one of its popups.
pub(crate) const CLOSE_REQUESTED_EVENT: &str = "workspace-window-close-requested";
/// The event the opener receives when one of its popups' native window is
/// destroyed by any path (its JS disposal is idempotent). Without it a popup
/// destroyed natively (the insisted second close, an OS kill) whose document
/// never fired `pagehide` would linger in JS as a ghost window.
pub(crate) const DESTROYED_EVENT: &str = "workspace-window-destroyed";

/// A saved window rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One monitor's work area in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a popup opens: a size, and a position only when it is visible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub width: f64,
    pub height: f64,
    pub position: Option<(f64, f64)>,
}

const DEFAULT_SIZE: (f64, f64) = (900.0, 760.0);
const MIN_SIZE: (f64, f64) = (360.0, 280.0);
const MAX_SIDE: f64 = 16_384.0;
/// How much of a window's title strip must lie on a monitor to keep its
/// saved position (a sliver past a screen edge is unreachable for a drag).
const VISIBLE_GRIP: f64 = 64.0;

/// Place a saved (or absent) geometry on the current monitors. A malformed
/// rectangle falls back to the default size; a size larger than every monitor
/// is clamped to the largest one; a position whose top strip is not on any
/// monitor (a disconnected display, a changed layout) is dropped so the
/// window manager places it. Never fails. O(monitors).
pub(crate) fn place(saved: Option<Geometry>, monitors: &[Area]) -> Placement {
    let sane = saved.filter(|g| {
        [g.x, g.y, g.width, g.height].iter().all(|v| v.is_finite())
            && g.width >= MIN_SIZE.0
            && g.height >= MIN_SIZE.1
            && g.width <= MAX_SIDE
            && g.height <= MAX_SIDE
            && g.x.abs() <= 4.0 * MAX_SIDE
            && g.y.abs() <= 4.0 * MAX_SIDE
    });
    let (mut width, mut height) = sane.map(|g| (g.width, g.height)).unwrap_or(DEFAULT_SIZE);
    if let Some(widest) = monitors
        .iter()
        .map(|m| (m.width, m.height))
        .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)))
    {
        width = width.min(widest.0.max(MIN_SIZE.0));
        height = height.min(widest.1.max(MIN_SIZE.1));
    }
    let position = sane.and_then(|g| {
        let grip_right = g.x + width.min(VISIBLE_GRIP * 2.0);
        let on_screen = monitors.iter().any(|m| {
            let overlap_x = (g.x + width).min(m.x + m.width) - g.x.max(m.x);
            g.y >= m.y
                && g.y + VISIBLE_GRIP / 2.0 <= m.y + m.height
                && overlap_x >= VISIBLE_GRIP.min(width)
                && grip_right > m.x
        });
        on_screen.then_some((g.x, g.y))
    });
    Placement {
        width,
        height,
        position,
    }
}

/// The one armed open per opener label: (popup label, placement).
#[derive(Default)]
pub(crate) struct Pending(Mutex<HashMap<String, (String, Placement)>>);

/// Close requests already handed to an opener: popup label -> first request.
#[derive(Default)]
pub(crate) struct CloseRequests(Mutex<HashMap<String, Instant>>);

#[cfg(desktop)]
static NEXT_POPUP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Is `label` a workspace popup of `opener`? Labels are `ws-<opener>-<n>`.
pub(crate) fn is_popup_of(label: &str, opener: &str) -> bool {
    label
        .strip_prefix("ws-")
        .and_then(|rest| rest.strip_prefix(opener))
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The opener label of a workspace popup label.
pub(crate) fn opener_of(label: &str) -> Option<&str> {
    let rest = label.strip_prefix("ws-")?;
    let (opener, n) = rest.rsplit_once('-')?;
    (!opener.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(opener)
}

fn popups_of<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    opener: &str,
) -> Vec<tauri::WebviewWindow<R>> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| is_popup_of(label, opener))
        .map(|(_, window)| window)
        .collect()
}

#[cfg(desktop)]
fn monitor_areas<R: tauri::Runtime>(window: &tauri::Window<R>) -> Vec<Area> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let scale = m.scale_factor().max(0.1);
            let pos = m.position().to_logical::<f64>(scale);
            let size = m.size().to_logical::<f64>(scale);
            Area {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect()
}

/// Arm one `window.open("about:blank")` from the calling window and return the
/// native label its popup will get. Refuses past [`MAX_WORKSPACE_WINDOWS`]
/// popups (I-22 resource bound; no storage effect). Re-arming replaces an
/// unused arm. O(windows + monitors).
#[tauri::command]
pub(crate) fn workspace_window_prepare(
    window: tauri::Window,
    geometry: Option<Geometry>,
) -> Result<String, String> {
    #[cfg(mobile)]
    {
        let _ = (window, geometry);
        Err("workspace windows are desktop-only".into())
    }
    #[cfg(desktop)]
    prepare_desktop(window, geometry)
}

#[cfg(desktop)]
fn prepare_desktop(window: tauri::Window, geometry: Option<Geometry>) -> Result<String, String> {
    let opener = window.label().to_string();
    if opener != "main" && !opener.starts_with("graph-") {
        return Err("only a graph window can open a workspace window".into());
    }
    let app = window.app_handle();
    if popups_of(app, &opener).len() >= MAX_WORKSPACE_WINDOWS {
        return Err(format!(
            "at most {MAX_WORKSPACE_WINDOWS} workspace windows can be open"
        ));
    }
    let label = format!(
        "ws-{opener}-{}",
        NEXT_POPUP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let placement = place(geometry, &monitor_areas(&window));
    app.state::<Pending>()
        .0
        .lock()
        .map_err(|_| "workspace window state unavailable".to_string())?
        .insert(opener, (label.clone(), placement));
    Ok(label)
}

/// Destroy one of the calling window's popups after its UI was disposed.
/// Idempotent: an already-gone popup is success.
#[tauri::command]
pub(crate) fn workspace_window_destroy(window: tauri::Window, label: String) -> Result<(), String> {
    if !is_popup_of(&label, window.label()) {
        return Err("not a workspace window of this window".into());
    }
    let app = window.app_handle();
    if let Ok(mut requests) = app.state::<CloseRequests>().0.lock() {
        requests.remove(&label);
    }
    match app.get_webview_window(&label) {
        Some(popup) => popup.destroy().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[cfg(desktop)]
/// Attach the popup door to a graph window builder (`main` or `graph-N`).
/// Only an open armed by `workspace_window_prepare` from this same window, for
/// `about:blank`, is created; everything else is denied.
pub(crate) fn attach<'a, R: tauri::Runtime, M: Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
    app: &tauri::AppHandle<R>,
    opener: &str,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    let app = app.clone();
    let opener = opener.to_string();
    builder.on_new_window(move |url, features| {
        let armed = app.try_state::<Pending>().and_then(|pending| {
            pending
                .0
                .lock()
                .ok()
                .and_then(|mut map| map.remove(&opener))
        });
        let Some((label, placement)) = armed else {
            return tauri::webview::NewWindowResponse::Deny;
        };
        if url.as_str() != "about:blank" {
            return tauri::webview::NewWindowResponse::Deny;
        }
        let builder =
            tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(url))
                .window_features(features)
                .title("Tine")
                .decorations(true)
                .min_inner_size(MIN_SIZE.0, MIN_SIZE.1)
                .inner_size(placement.width, placement.height)
                .on_document_title_changed(|window, title| {
                    let _ = window.set_title(&title);
                });
        let builder = match placement.position {
            Some((x, y)) => builder.position(x, y),
            None => builder,
        };
        match builder.build() {
            Ok(window) => tauri::webview::NewWindowResponse::Create { window },
            Err(error) => {
                crate::debug::diag_private("workspace-window-build-failed", error.to_string());
                tauri::webview::NewWindowResponse::Deny
            }
        }
    })
}

/// WebKitGTK refuses a script `window.open` without a recent user gesture by
/// default; a restored session reopens its windows at launch, with no gesture.
#[cfg(target_os = "linux")]
pub(crate) fn allow_script_windows(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|view| {
        use webkit2gtk::{SettingsExt, WebViewExt};
        if let Some(settings) = view.inner().settings() {
            settings.set_javascript_can_open_windows_automatically(true);
        }
    });
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn allow_script_windows(_window: &tauri::WebviewWindow) {}

#[cfg(desktop)]
/// Desktop: take `main` out of Tauri's automatic window creation so `setup`
/// can build it with the popup door attached (a builder hook cannot be added
/// to a window that already exists). Returns main's configuration.
pub(crate) fn take_main_config(
    context: &mut tauri::Context<tauri::Wry>,
) -> Option<tauri::utils::config::WindowConfig> {
    let main = context
        .config_mut()
        .app
        .windows
        .iter_mut()
        .find(|window| window.label == "main")?;
    let config = main.clone();
    main.create = false;
    Some(config)
}

#[cfg(desktop)]
/// Build `main` from its configuration with the popup door attached. Linux
/// also carries the YouTube identity extension, retried without it on
/// failure exactly as the other startup windows are. Never returns an error
/// (Tauri panics on an error from `.setup`, I-22). If `main` cannot be built
/// at all the process exits with a diagnostic, as it did when Tauri built
/// `main` from the configuration itself: a windowless process would hold the
/// single-instance lock, so every later launch would forward to it and show
/// nothing (review F3).
pub(crate) fn create_main(
    app: &tauri::App,
    config: &tauri::utils::config::WindowConfig,
    start_hidden: bool,
) {
    let build = |with_identity: bool| -> tauri::Result<tauri::WebviewWindow> {
        let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), config)?;
        let builder = attach(builder, app.handle(), &config.label);
        // Start-minimized to the tray (GH #625): the frontend's own reveal
        // after first paint stands down; the tray (or a second launch) shows it.
        let builder = if start_hidden {
            builder.initialization_script("globalThis.__TINE_START_HIDDEN__ = true;")
        } else {
            builder
        };
        #[cfg(target_os = "linux")]
        let builder = if with_identity {
            crate::youtube_identity::configure(builder, app.handle())
        } else {
            builder
        };
        #[cfg(not(target_os = "linux"))]
        let _ = with_identity;
        builder.build()
    };
    let built = build(true).or_else(|error| {
        crate::debug::diag_private("youtube-identity-window-failed", error.to_string());
        build(false)
    });
    match built {
        Ok(window) => allow_script_windows(&window),
        Err(error) => {
            crate::debug::diag_private("startup-window-failed", error.to_string());
            eprintln!("[tine] the main window could not be created: {error}");
            // Setup runs before any graph is open: nothing is unsaved.
            std::process::exit(1);
        }
    }
}

/// Destroy every popup of `opener` (its reload or teardown). O(windows).
pub(crate) fn destroy_popups_of<R: tauri::Runtime>(app: &tauri::AppHandle<R>, opener: &str) {
    for popup in popups_of(app, opener) {
        let _ = popup.destroy();
    }
}

/// Window-event hook, called for every window before lib.rs's own handling.
/// A popup's close request is held and handed to its opener; an opener's
/// destruction takes its popups with it.
pub(crate) fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let label = window.label();
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            let Some(opener) = opener_of(label) else {
                return;
            };
            let app = window.app_handle();
            if app.get_webview_window(opener).is_none() {
                return; // no opener to dispose it: close natively
            }
            let insisted = app
                .state::<CloseRequests>()
                .0
                .lock()
                .map(|mut requests| {
                    let now = Instant::now();
                    match requests.get(label) {
                        Some(first) if now.duration_since(*first) >= INSISTED_CLOSE => true,
                        _ => {
                            requests.entry(label.to_string()).or_insert(now);
                            false
                        }
                    }
                })
                .unwrap_or(true);
            if insisted {
                return;
            }
            api.prevent_close();
            if app.emit_to(opener, CLOSE_REQUESTED_EVENT, label).is_err() {
                let _ = window.destroy();
            }
        }
        tauri::WindowEvent::Destroyed => {
            let app = window.app_handle();
            if let Some(opener) = opener_of(label) {
                if let Ok(mut requests) = app.state::<CloseRequests>().0.lock() {
                    requests.remove(label);
                }
                if app.get_webview_window(opener).is_some() {
                    let _ = app.emit_to(opener, DESTROYED_EVENT, label);
                }
            } else {
                destroy_popups_of(app, label);
            }
        }
        _ => {}
    }
}

/// Page-load hook: an opener that starts a new document (a reload) has lost
/// the JS heap its popups were drawn from, so they are destroyed.
pub(crate) fn page_load(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if payload.event() != tauri::webview::PageLoadEvent::Started {
        return;
    }
    let label = webview.label();
    if opener_of(label).is_none() {
        destroy_popups_of(webview.app_handle(), label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Area = Area {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    fn geometry(x: f64, y: f64, width: f64, height: f64) -> Option<Geometry> {
        Some(Geometry {
            x,
            y,
            width,
            height,
        })
    }

    #[test]
    fn a_visible_saved_rectangle_is_kept() {
        assert_eq!(
            place(geometry(100.0, 80.0, 800.0, 600.0), &[SCREEN]),
            Placement {
                width: 800.0,
                height: 600.0,
                position: Some((100.0, 80.0))
            }
        );
    }

    #[test]
    fn an_off_screen_position_is_dropped_but_the_size_kept() {
        for (x, y) in [
            (5000.0, 80.0),
            (100.0, -900.0),
            (-2000.0, 80.0),
            (100.0, 1070.0),
        ] {
            let placed = place(geometry(x, y, 800.0, 600.0), &[SCREEN]);
            assert_eq!(placed.position, None, "({x}, {y})");
            assert_eq!((placed.width, placed.height), (800.0, 600.0));
        }
    }

    #[test]
    fn a_second_monitor_keeps_its_window_when_present() {
        let right = Area {
            x: 1920.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        };
        let saved = geometry(2200.0, 100.0, 1200.0, 900.0);
        assert_eq!(
            place(saved, &[SCREEN, right]).position,
            Some((2200.0, 100.0))
        );
        assert_eq!(place(saved, &[SCREEN]).position, None);
    }

    #[test]
    fn malformed_geometry_falls_back_to_the_default_size() {
        for saved in [
            geometry(f64::NAN, 0.0, 800.0, 600.0),
            geometry(0.0, 0.0, f64::INFINITY, 600.0),
            geometry(0.0, 0.0, 10.0, 10.0),
            geometry(0.0, 0.0, 1e9, 600.0),
            geometry(1e12, 0.0, 800.0, 600.0),
            None,
        ] {
            let placed = place(saved, &[SCREEN]);
            assert_eq!((placed.width, placed.height), DEFAULT_SIZE, "{saved:?}");
            assert_eq!(placed.position, None, "{saved:?}");
        }
    }

    #[test]
    fn an_oversized_window_is_clamped_to_the_largest_monitor() {
        let placed = place(geometry(0.0, 0.0, 4000.0, 3000.0), &[SCREEN]);
        assert_eq!((placed.width, placed.height), (1920.0, 1080.0));
        // No monitor information: the saved size stands.
        assert_eq!(place(geometry(0.0, 0.0, 4000.0, 3000.0), &[]).width, 4000.0);
    }

    #[test]
    fn popup_labels_name_their_opener() {
        assert!(is_popup_of("ws-main-3", "main"));
        assert!(is_popup_of("ws-graph-2-14", "graph-2"));
        assert!(!is_popup_of("ws-graph-2-14", "graph-21"));
        assert!(!is_popup_of("ws-main-", "main"));
        assert!(!is_popup_of("ws-main-3x", "main"));
        assert!(!is_popup_of("main", "main"));
        assert_eq!(opener_of("ws-main-3"), Some("main"));
        assert_eq!(opener_of("ws-graph-2-14"), Some("graph-2"));
        assert_eq!(opener_of("graph-2"), None);
        assert_eq!(opener_of("main"), None);
        assert_eq!(opener_of("capture"), None);
    }
}
