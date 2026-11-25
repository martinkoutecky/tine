//! PROBE ONLY (tine#1043, never merged): log the main window's geometry at
//! each step of the window-state save/restore cycle. Inert unless
//! `TINE_PROBE_1043` names a file; each moment appends one JSON line there.
//!
//! Order on the main thread (setup builds main): Tauri runs every plugin's
//! `on_window_ready` synchronously inside `build()`, in registration order, so
//! the `built` plugin (registered before window-state) sees the window before
//! the restore and the `restored` plugin (registered after) sees it right after.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static FILE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static SHOWN: AtomicBool = AtomicBool::new(false);
static FIRST_RESIZE_AFTER_SHOW: AtomicBool = AtomicBool::new(false);
static EVENTS: AtomicU32 = AtomicU32::new(0);

fn file() -> Option<&'static std::path::PathBuf> {
    FILE.get_or_init(|| std::env::var_os("TINE_PROBE_1043").map(Into::into))
        .as_ref()
}

pub(crate) fn enabled() -> bool {
    START.get_or_init(Instant::now);
    file().is_some()
}

pub(crate) fn log<R: tauri::Runtime>(label: &str, window: &tauri::Window<R>) {
    let Some(path) = file() else { return };
    let inner = window.inner_size().ok();
    let outer = window.outer_size().ok();
    let position = window.outer_position().ok();
    let monitor = window.current_monitor().ok().flatten();
    let line = serde_json::json!({
        "label": label,
        "t_ms": START.get_or_init(Instant::now).elapsed().as_millis() as u64,
        "window": window.label(),
        "inner_size": inner.map(|s| [s.width, s.height]),
        "outer_size": outer.map(|s| [s.width, s.height]),
        "outer_position": position.map(|p| [p.x, p.y]),
        "scale_factor": window.scale_factor().ok(),
        "is_decorated": window.is_decorated().ok(),
        "is_maximized": window.is_maximized().ok(),
        "is_visible": window.is_visible().ok(),
        "monitor": monitor.map(|m| serde_json::json!({
            "size": [m.size().width, m.size().height],
            "position": [m.position().x, m.position().y],
            "scale_factor": m.scale_factor(),
        })),
        "win32": win32(window),
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(windows)]
fn win32<R: tauri::Runtime>(window: &tauri::Window<R>) -> serde_json::Value {
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetSystemMetrics, GetWindowLongW, GetWindowRect, GWL_STYLE,
        SM_CXPADDEDBORDER, SM_CYCAPTION, SM_CYSIZEFRAME,
    };
    let Ok(hwnd) = window.hwnd() else {
        return serde_json::Value::Null;
    };
    let hwnd = hwnd.0 as HWND;
    let mut window_rect = RECT::default();
    let mut client_rect = RECT::default();
    // SAFETY: plain Win32 queries on a live HWND owned by this process.
    let (ok_w, ok_c, style, caption, frame, padded) = unsafe {
        (
            GetWindowRect(hwnd, &mut window_rect),
            GetClientRect(hwnd, &mut client_rect),
            GetWindowLongW(hwnd, GWL_STYLE) as u32,
            GetSystemMetrics(SM_CYCAPTION),
            GetSystemMetrics(SM_CYSIZEFRAME),
            GetSystemMetrics(SM_CXPADDEDBORDER),
        )
    };
    let rect = |r: RECT| [r.left, r.top, r.right, r.bottom];
    serde_json::json!({
        "window_rect": (ok_w != 0).then(|| rect(window_rect)),
        "client_rect": (ok_c != 0).then(|| rect(client_rect)),
        "style": format!("{style:#010x}"),
        "sm_cycaption": caption,
        "sm_cysizeframe": frame,
        "sm_cxpaddedborder": padded,
    })
}

#[cfg(not(windows))]
fn win32<R: tauri::Runtime>(_window: &tauri::Window<R>) -> serde_json::Value {
    serde_json::Value::Null
}

/// A plugin that logs `label` when the main window is ready.
pub(crate) fn plugin<R: tauri::Runtime>(
    name: &'static str,
    label: &'static str,
) -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new(name)
        .on_window_ready(move |window| {
            if window.label() == "main" {
                log(label, &window);
            }
        })
        .build()
}

/// Log `shown` once main becomes visible (the frontend shows it after first
/// paint; there is no native "shown" event to hook).
pub(crate) fn watch_shown(app: &tauri::AppHandle) {
    if !enabled() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let end = Instant::now() + std::time::Duration::from_secs(180);
        while Instant::now() < end {
            if let Some(window) = tauri::Manager::get_webview_window(&app, "main") {
                if window.is_visible().unwrap_or(false) {
                    SHOWN.store(true, Ordering::SeqCst);
                    log("shown", &window.as_ref().window());
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
}

pub(crate) fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" || !enabled() {
        return;
    }
    let label = match event {
        tauri::WindowEvent::Resized(_) => {
            if SHOWN.load(Ordering::SeqCst) && !FIRST_RESIZE_AFTER_SHOW.swap(true, Ordering::SeqCst)
            {
                "first-resize-after-show"
            } else if SHOWN.load(Ordering::SeqCst) {
                "resized-after-show"
            } else {
                "resized-before-show"
            }
        }
        tauri::WindowEvent::ScaleFactorChanged { .. } => "scale-factor-changed",
        tauri::WindowEvent::CloseRequested { .. } => "close-requested",
        _ => return,
    };
    // A bounded log: a user dragging the edge must not fill the disk.
    if label != "close-requested" && EVENTS.fetch_add(1, Ordering::SeqCst) >= 40 {
        return;
    }
    log(label, window);
}
