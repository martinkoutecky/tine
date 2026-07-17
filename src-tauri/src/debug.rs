use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Startup debug logging  (enable with TINE_DEBUG=1  or  the --debug flag)
// ---------------------------------------------------------------------------
// A "bad startup" report usually means the window never appeared — so stderr,
// which a desktop-launched app discards, tells the user nothing. When debug mode
// is on we ALSO append timestamped milestones to a findable log file (default
// `<tmp>/tine-debug.log`, override with TINE_DEBUG_LOG), install a panic hook
// that captures a backtrace, and let the frontend forward its console errors
// here (the `debug_log` command). One file then tells the whole startup story,
// so diagnosing a remote user takes a single round-trip: "run this, send me that
// file." See README → Troubleshooting.
static DEBUG_LOG: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
static DEBUG_START: OnceLock<std::time::Instant> = OnceLock::new();

pub(crate) fn debug_enabled() -> bool {
    matches!(std::env::var("TINE_DEBUG"), Ok(v) if !v.is_empty() && v != "0")
        || std::env::args().any(|a| a == "--debug")
}

fn debug_log_path() -> PathBuf {
    std::env::var_os("TINE_DEBUG_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("tine-debug.log"))
}

/// Open (truncating) the debug log once, so each run is a clean trace. No-op when
/// debug mode is off. Safe to call repeatedly.
pub(crate) fn debug_init() {
    DEBUG_START.get_or_init(std::time::Instant::now);
    DEBUG_LOG.get_or_init(|| {
        if !debug_enabled() {
            return None;
        }
        let path = debug_log_path();
        match std::fs::File::create(&path) {
            Ok(f) => {
                eprintln!("[tine] DEBUG logging to {}", path.display());
                Some(Mutex::new(f))
            }
            Err(e) => {
                eprintln!("[tine] could not open debug log {}: {e}", path.display());
                None
            }
        }
    });
}

/// Emit one diagnostic line to stderr AND, when debug mode is on, the log file
/// (prefixed with a +Nms offset from process start).
pub(crate) fn diag(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    eprintln!("[tine] {msg}");
    if let Some(Some(lock)) = DEBUG_LOG.get() {
        let ms = DEBUG_START
            .get()
            .map(|s| s.elapsed().as_millis())
            .unwrap_or(0);
        if let Ok(mut f) = lock.lock() {
            let _ = writeln!(f, "[+{ms:>7}ms] {msg}");
            let _ = f.flush();
        }
    }
}

/// Log the environment that most often explains a broken launch (renderer,
/// session type, AppImage, graph override, preload).
pub(crate) fn debug_header() {
    if !debug_enabled() {
        return;
    }
    diag(format!(
        "Tine {} starting — {}/{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let env_of = |k: &str| std::env::var(k).unwrap_or_else(|_| "<unset>".into());
    for k in [
        "TINE_GRAPH",
        "TINE_GPU",
        "WEBVIEW2_USER_DATA_FOLDER",
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
        "XDG_SESSION_TYPE",
        "WAYLAND_DISPLAY",
        "APPIMAGE",
        "LD_PRELOAD",
        "GDK_BACKEND",
    ] {
        diag(format!("env {k}={}", env_of(k)));
    }
}

/// Install a panic hook that records the panic + a backtrace into the debug log
/// (RUST_BACKTRACE forced on), then chains to the default hook. Debug mode only.
pub(crate) fn install_panic_logger() {
    if !debug_enabled() {
        return;
    }
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        diag(format!("PANIC: {info}"));
        diag(format!(
            "backtrace:\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        default(info);
    }));
}

/// Frontend → backend bridge so the webview's own milestones / errors land in the
/// same file (e.g. "frontend booted", a window.onerror). No-op unless debugging.
#[tauri::command]
pub(crate) fn debug_log(line: String) {
    if debug_enabled() {
        diag(format!("[ui] {line}"));
    }
}

#[derive(serde::Serialize)]
pub(crate) struct DebugInfo {
    enabled: bool,
    path: String,
}

/// Lets the frontend learn whether debug mode is on (so it can wire up its error
/// forwarding) and where the log lives (to surface the path to the user).
#[tauri::command]
pub(crate) fn debug_info() -> DebugInfo {
    DebugInfo {
        enabled: debug_enabled(),
        path: debug_log_path().display().to_string(),
    }
}
