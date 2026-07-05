use crate::commands::decode_asset_b64;

/// On Linux, hand a PNG to the OS clipboard via `wl-copy` (Wayland) or `xclip`/
/// `xsel` (X11). These tools FORK a daemon that serves the selection until it's
/// replaced — which is exactly what an image clipboard needs. `arboard` (what the
/// Tauri plugin uses) tries to do this in-process and frequently drops the image
/// on WebKitGTK, so we prefer the native tools and only fall back to the plugin.
#[cfg(target_os = "linux")]
fn linux_copy_image(bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    // Prefer the tool matching the active session, but try all — a Wayland session
    // under Xwayland may have only xclip, and vice versa.
    let mut order: Vec<(&str, Vec<&str>)> = Vec::new();
    let push = |o: &mut Vec<(&str, Vec<&str>)>, prog: &'static str| {
        let args: Vec<&str> = match prog {
            "wl-copy" => vec!["--type", "image/png"],
            "xclip" => vec!["-selection", "clipboard", "-t", "image/png"],
            "xsel" => vec!["--clipboard", "--input"],
            _ => vec![],
        };
        o.push((prog, args));
    };
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        push(&mut order, "wl-copy");
        push(&mut order, "xclip");
        push(&mut order, "xsel");
    } else {
        push(&mut order, "xclip");
        push(&mut order, "xsel");
        push(&mut order, "wl-copy");
    }
    let mut last_err = String::from("no clipboard tool found (install wl-clipboard or xclip)");
    for (prog, args) in order {
        let child = Command::new(prog)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("{prog}: {e}");
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(bytes) {
                last_err = format!("{prog}: write stdin: {e}");
                continue;
            }
        }
        // wl-copy/xclip fork a server and the foreground process exits promptly;
        // reap it on a thread so we neither block here nor leak a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    Err(last_err)
}

#[tauri::command]
pub(crate) fn copy_image_to_clipboard(
    app: tauri::AppHandle,
    bytes_b64: String,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let bytes = decode_asset_b64(&bytes_b64)?;
    #[cfg(target_os = "linux")]
    if linux_copy_image(&bytes).is_ok() {
        return Ok(());
    }
    let img = tauri::image::Image::from_bytes(&bytes).map_err(|e| e.to_string())?;
    app.clipboard().write_image(&img).map_err(|e| e.to_string())
}

/// What the backend knows about the rendering path, so the UI can warn — loudly
/// — when Tine is painting on the CPU. Speed is the whole pitch, and a silent
/// software-rendering fallback makes scrolling feel sluggish; better to say so.
/// `software_forced` is true only when we *know* GPU compositing is off because
/// an env var disabled it: TINE_GPU=0 (we then set WEBKIT_DISABLE_DMABUF_RENDERER
/// just above in `main`), or the user set WEBKIT_DISABLE_DMABUF_RENDERER /
/// WEBKIT_DISABLE_COMPOSITING_MODE themselves. A *silent* driver/EGL fallback
/// (DMABUF requested but EGL init failed) does NOT show up here — that's detected
/// in the webview from the WebGL renderer string (llvmpipe/swrast ⇒ software).
/// `appimage` lets the message steer AppImage users to the deb/rpm, which use the
/// host graphics stack instead of the bundled one. Env reads are cross-platform
/// (these vars are simply absent on macOS/Windows ⇒ both false).
#[derive(serde::Serialize)]
pub(crate) struct GpuEnv {
    software_forced: bool,
    appimage: bool,
}

#[tauri::command]
pub(crate) fn gpu_env() -> GpuEnv {
    let set = |k: &str| std::env::var_os(k).is_some();
    GpuEnv {
        software_forced: set("WEBKIT_DISABLE_DMABUF_RENDERER")
            || set("WEBKIT_DISABLE_COMPOSITING_MODE")
            || std::env::var("TINE_GPU").as_deref() == Ok("0"),
        appimage: set("APPIMAGE"),
    }
}

/// Open a web/mail URL in the user's default external application. Scheme-gated
/// to http(s)/mailto; the URL is passed as a single argument (no shell), so it
/// can't inject commands.
#[tauri::command]
pub(crate) fn open_external(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
        return Err("unsupported url scheme".into());
    }
    #[cfg(target_os = "linux")]
    let prog = "xdg-open";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    opener_command(prog)
        .arg(&url)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Build the OS "open" command, scrubbing the env vars Tine (or its AppImage
/// wrapper) sets for ITS OWN WebKitGTK rendering before it launches a SEPARATE
/// GUI app. `LD_PRELOAD` (the Wayland libwayland-client self-heal),
/// `WEBKIT_DISABLE_*`, and `GDK_BACKEND` are inherited by `spawn()` and can break a
/// launched player's VIDEO output — e.g. VLC opens then exits immediately — while
/// audio-only playback (no GL/window) is unaffected. A clean env is both the fix
/// and good hygiene; no-op on non-Linux.
pub(crate) fn opener_command(prog: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(prog);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // 1. Scrub the env Tine (or its AppImage wrapper) sets for ITS OWN
        //    WebKitGTK rendering before launching a SEPARATE GUI app. Several of
        //    these break a launched player's VIDEO output — VLC opens then exits
        //    immediately. The WEBKIT_*/GDK_BACKEND/LD_PRELOAD set is what Tine
        //    itself may set; the LD_LIBRARY_PATH/GST_*/GTK_*/GIO_* set is what an
        //    AppImage bundle injects so a child loads bundled libs/plugins that
        //    mismatch the host player. Removing an unset var is a no-op, so this
        //    is safe on the raw binary too.
        for k in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "WEBKIT_DISABLE_DMABUF_RENDERER",
            "WEBKIT_DISABLE_COMPOSITING_MODE",
            "GDK_BACKEND",
            "GST_PLUGIN_SYSTEM_PATH",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GST_PLUGIN_PATH",
            "GST_PLUGIN_PATH_1_0",
            "GIO_MODULE_DIR",
            "GTK_PATH",
            "GTK_EXE_PREFIX",
            "GDK_PIXBUF_MODULE_FILE",
            "GTK_IM_MODULE_FILE",
            "FONTCONFIG_FILE",
            "FONTCONFIG_PATH",
        ] {
            cmd.env_remove(k);
        }
        // 2. Detach: own process group + no inherited stdio. A player whose
        //    lifetime is tied to Tine's process group, or that probes a stdio it
        //    inherited from a GUI parent, can "open then close immediately" even
        //    on the raw binary where no bundle env vars are set. Giving it its
        //    own session-ish group and /dev/null stdio is the robust fix.
        cmd.process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    cmd
}
