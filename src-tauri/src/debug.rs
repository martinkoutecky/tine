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
                eprintln!("[tine] debug-log-opened");
                Some(Mutex::new(f))
            }
            Err(e) => {
                let _ = (path, e);
                eprintln!("[tine] debug-log-open-failed");
                None
            }
        }
    });
}

/// Emit a fixed, source-owned event name to stderr and the opt-in debug file.
/// Callers must pass a literal; detail belongs in [`diag_private`].
pub(crate) fn diag(event: &'static str) {
    eprintln!("[tine] {event}");
    write_debug(event);
}

/// Emit only the fixed event to stderr, with private detail in the opt-in file.
pub(crate) fn diag_private(event: &'static str, detail: impl AsRef<str>) {
    eprintln!("[tine] {event}");
    write_debug(detail.as_ref());
}

fn write_debug(msg: &str) {
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
    diag_private(
        "startup",
        format!(
            "Tine {} starting — {}/{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
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
        diag_private("startup-env", format!("env {k}={}", env_of(k)));
    }
}

/// Keep panic payloads and backtraces out of stderr; report the Rust source
/// location there and retain full details in the opt-in file.
pub(crate) fn install_panic_logger() {
    if debug_enabled() && std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }
    std::panic::set_hook(Box::new(move |info| {
        crate::flight::record_panic(info);
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown:0:0".to_owned());
        eprintln!("tine: panic at {location} (details in debug log when enabled)");
        if debug_enabled() {
            write_debug(&format!("PANIC: {info}"));
            write_debug(&format!(
                "backtrace:\n{}",
                std::backtrace::Backtrace::force_capture()
            ));
        }
    }));
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn panic_hook_child() {
        if std::env::var_os("TINE_PANIC_HOOK_CHILD").is_none() {
            return;
        }
        super::debug_init();
        super::install_panic_logger();
        let _ = std::panic::catch_unwind(|| panic!("PRIVATE_PANIC_PAYLOAD_123"));
    }

    #[test]
    fn panic_hook_reports_location_without_payload() {
        for enabled in [false, true] {
            let log_path = std::env::temp_dir().join(format!(
                "tine-panic-hook-test-{}-{enabled}.log",
                std::process::id()
            ));
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "debug::tests::panic_hook_child", "--nocapture"])
                .env("TINE_PANIC_HOOK_CHILD", "1")
                .env_remove("TINE_DEBUG")
                .env_remove("TINE_DEBUG_LOG");
            if enabled {
                child
                    .env("TINE_DEBUG", "1")
                    .env("TINE_DEBUG_LOG", &log_path);
            }
            let output = child.output().unwrap();
            assert!(output.status.success());
            let stderr = String::from_utf8(output.stderr).unwrap();
            let line = stderr
                .lines()
                .find(|line| line.starts_with("tine: panic at "))
                .expect("I-5: panic hook must put Rust source location on stderr; exemplar src-tauri/src/debug.rs");
            assert!(
                line.contains("debug.rs:") && line.ends_with(" (details in debug log when enabled)"),
                "I-5: panic location must name Rust source file:line:column; exemplar src-tauri/src/debug.rs: {line}"
            );
            let location = line
                .strip_prefix("tine: panic at ")
                .unwrap()
                .split(" (details in debug log when enabled)")
                .next()
                .unwrap();
            let mut parts = location.rsplit(':');
            assert!(parts.next().unwrap().parse::<u32>().is_ok());
            assert!(parts.next().unwrap().parse::<u32>().is_ok());
            assert!(
                !stderr.contains("PRIVATE_PANIC_PAYLOAD_123") && !stderr.contains("backtrace:"),
                "I-5: panic payloads and backtraces stay out of stderr; exemplar src-tauri/src/debug.rs"
            );
            if enabled {
                let log = std::fs::read_to_string(&log_path).unwrap();
                assert!(log.contains("PRIVATE_PANIC_PAYLOAD_123") && log.contains("backtrace:"));
                std::fs::remove_file(log_path).unwrap();
            }
        }
    }
}

/// Frontend → backend bridge so the webview's own milestones / errors land in the
/// same file (e.g. "frontend booted", a window.onerror). No-op unless debugging.
#[tauri::command]
pub(crate) fn debug_log(line: String) {
    if debug_enabled() {
        diag_private("ui-debug", line);
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
