//! Configure YouTube client identity before native WebViews start navigating.
//! Linux embeds a WebKit WebProcess extension; O(1) setup writes one temporary
//! binary, never graph data. Extraction failure logs and retains ordinary app
//! startup. The four other shipped targets await native platform verification.

#[cfg(target_os = "linux")]
pub(crate) fn prepare(
    context: &mut tauri::Context<tauri::Wry>,
) -> Vec<tauri::utils::config::WindowConfig> {
    context
        .config_mut()
        .app
        .windows
        .iter_mut()
        .filter(|window| window.create)
        .map(|window| {
            let config = window.clone();
            window.create = false;
            config
        })
        .collect()
}

/// Extract the WebProcess extension and register it for every window built
/// afterwards. Never returns an error: a failure logs and costs only the
/// YouTube identity.
#[cfg(target_os = "linux")]
pub(crate) fn install_extension(app: &mut tauri::App) {
    use tauri::Manager;
    let extract = || -> std::io::Result<tempfile::TempDir> {
        let dir = tempfile::Builder::new().prefix("tine-youtube-").tempdir()?;
        crate::device_io::atomic_write(
            &dir.path().join("libtine_youtube.so"),
            include_bytes!(concat!(env!("OUT_DIR"), "/libtine_youtube.so")),
        )?;
        Ok(dir)
    };
    match extract() {
        Ok(dir) => {
            app.manage(ExtensionDirectory(std::sync::Mutex::new(Some(dir))));
        }
        Err(error) => crate::debug::diag_private("youtube-identity-unavailable", error.to_string()),
    }
}

/// Build the startup windows other than `main` (which `workspace_windows`
/// builds). Call after `main`: a WebDriver session attaches to the first
/// window the process creates, and every native journey assumes that is `main`.
#[cfg(target_os = "linux")]
pub(crate) fn create_windows(app: &mut tauri::App, windows: &[tauri::utils::config::WindowConfig]) {
    // Never returns an error: Tauri panics on an error from `.setup` (I-22).
    // A window the extension cannot accompany is retried without it, so
    // YouTube identity is the only thing a failure here can cost.
    for config in windows {
        let build = |with_identity: bool| -> tauri::Result<()> {
            let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), config)?;
            let builder = if with_identity {
                configure(builder, app.handle())
            } else {
                builder
            };
            builder.build().map(|_| ())
        };
        if let Err(error) = build(true) {
            crate::debug::diag_private("youtube-identity-window-failed", error.to_string());
            if let Err(error) = build(false) {
                crate::debug::diag_private("startup-window-failed", error.to_string());
            }
        }
    }
}

#[cfg(target_os = "linux")]
struct ExtensionDirectory(std::sync::Mutex<Option<tempfile::TempDir>>);

#[cfg(target_os = "linux")]
pub(crate) fn configure<'a>(
    builder: tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle>,
    app: &tauri::AppHandle,
) -> tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle> {
    use tauri::Manager;
    if let Some(state) = app.try_state::<ExtensionDirectory>() {
        if let Some(dir) = state
            .0
            .lock()
            .expect("YouTube extension directory lock")
            .as_ref()
        {
            return builder.extensions_path(dir.path());
        }
    }
    builder
}

#[cfg(target_os = "linux")]
pub(crate) fn cleanup(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(state) = app.try_state::<ExtensionDirectory>() {
        state
            .0
            .lock()
            .expect("YouTube extension directory lock")
            .take();
    }
}

// Explicitly enumerate shipped platforms: these are pending, not Unsupported.
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "android",
    target_os = "ios"
))]
pub(crate) fn cleanup(_app: &tauri::AppHandle) {}
