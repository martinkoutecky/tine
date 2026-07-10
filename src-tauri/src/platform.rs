use crate::commands::decode_asset_b64;

const MAX_CLIPBOARD_FILES: usize = 32;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardAssetFile {
    path: String,
    name: String,
    size: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardFileList {
    files: Vec<ClipboardAssetFile>,
    skipped: usize,
    truncated: bool,
}

/// Read paths copied by the OS file manager. This runs only on an explicit
/// paste gesture; Tine never monitors the clipboard. Paths are canonicalized and
/// restricted to regular files before they cross into the WebView. Directories
/// are intentionally skipped rather than recursively importing arbitrary trees.
#[cfg(desktop)]
fn read_clipboard_files() -> Result<ClipboardFileList, String> {
    use clipboard_rs::{Clipboard, ClipboardContext};

    let context = ClipboardContext::new().map_err(|e| e.to_string())?;
    let paths = context.get_files().map_err(|e| e.to_string())?;
    Ok(clipboard_file_list(paths))
}

fn clipboard_file_list(paths: Vec<String>) -> ClipboardFileList {
    use std::path::Path;

    let truncated = paths.len() > MAX_CLIPBOARD_FILES;
    let mut files = Vec::new();
    let mut skipped = paths.len().saturating_sub(MAX_CLIPBOARD_FILES);

    for raw in paths.into_iter().take(MAX_CLIPBOARD_FILES) {
        let original = Path::new(&raw);
        let name = original
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned);
        let Ok(path) = std::fs::canonicalize(original) else {
            skipped += 1;
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&path) else {
            skipped += 1;
            continue;
        };
        let Some(name) = name else {
            skipped += 1;
            continue;
        };
        if !metadata.is_file() {
            skipped += 1;
            continue;
        }
        files.push(ClipboardAssetFile {
            path: path.to_string_lossy().into_owned(),
            name,
            size: metadata.len(),
        });
    }

    ClipboardFileList {
        files,
        skipped,
        truncated,
    }
}

#[tauri::command]
pub(crate) async fn clipboard_files() -> Result<ClipboardFileList, String> {
    #[cfg(desktop)]
    return tauri::async_runtime::spawn_blocking(read_clipboard_files)
        .await
        .map_err(|e| e.to_string())?;

    #[cfg(not(desktop))]
    return Ok(ClipboardFileList {
        files: Vec::new(),
        skipped: 0,
        truncated: false,
    });
}

#[cfg(test)]
mod clipboard_file_tests {
    use super::*;

    #[test]
    fn validates_regular_files_and_skips_directories() {
        let root =
            std::env::temp_dir().join(format!("tine-clipboard-files-{}", std::process::id()));
        let dir = root.join("folder");
        let file = root.join("report.pdf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, b"pdf").unwrap();

        let result = clipboard_file_list(vec![
            file.to_string_lossy().into_owned(),
            dir.to_string_lossy().into_owned(),
            root.join("missing.txt").to_string_lossy().into_owned(),
        ]);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].name, "report.pdf");
        assert_eq!(result.files[0].size, 3);
        assert_eq!(result.skipped, 2);
        assert!(!result.truncated);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn caps_the_number_of_clipboard_entries() {
        let result = clipboard_file_list(vec!["missing".into(); MAX_CLIPBOARD_FILES + 4]);
        assert!(result.truncated);
        assert_eq!(result.files.len(), 0);
        assert_eq!(result.skipped, MAX_CLIPBOARD_FILES + 4);
    }
}

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
pub(crate) fn open_external(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
        return Err("unsupported url scheme".into());
    }
    #[cfg(desktop)]
    {
        let _ = &app;
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
    // Mobile (Android/iOS): there is no xdg-open/open/explorer to spawn, so hand
    // the URL to the platform via the opener plugin (an ACTION_VIEW Intent on
    // Android). This is what makes the About/Help/Releases links actually open on
    // Android — before this they fired a command that returned an error the
    // frontend silently swallowed (GH #49).
    #[cfg(not(desktop))]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    }
}

/// Build the OS "open" command, scrubbing the env vars Tine (or its AppImage
/// wrapper) sets for ITS OWN WebKitGTK rendering before it launches a SEPARATE
/// GUI app. `LD_PRELOAD` (the Wayland libwayland-client self-heal),
/// `WEBKIT_DISABLE_*`, and `GDK_BACKEND` are inherited by `spawn()` and can break a
/// launched player's VIDEO output — e.g. VLC opens then exits immediately — while
/// audio-only playback (no GL/window) is unaffected. A clean env is both the fix
/// and good hygiene; no-op on non-Linux.
#[cfg(desktop)]
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

/// SIGKILL WebKitGTK's helper subprocesses (`WebKitWebProcess` /
/// `WebKitNetworkProcess` / `WebKitGPUProcess`) that are direct children of THIS
/// process, before we quit.
///
/// Why (GH #28): those aux processes terminate by *returning from `main()`*, so on
/// exit they run the full C runtime teardown (`exit()` → `__run_exit_handlers` →
/// GL/EGL/GBM driver static destructors). On many Mesa/driver combos that teardown
/// double-frees and aborts (SIGABRT), producing a coredump *after* the app has
/// already closed — harmless but alarming, and it happens even on plain Intel iGPUs.
/// SIGKILL is uncatchable and never runs exit handlers, so killing the web process
/// here (rather than letting `destroy()` shut it down gracefully) makes the crash
/// impossible instead of merely hiding the dump. It keeps GPU compositing on for the
/// whole session (unlike `WEBKIT_DISABLE_DMABUF_RENDERER`, the `TINE_GPU=0` opt-out).
///
/// Must run BEFORE the WebView is destroyed and AFTER the JS close handler has
/// flushed pending edits (it has — it only calls this via `destroy`/quit once
/// `flushAll()`/`flushSession()` resolved). We restrict to our own children by PPID
/// so we never touch another app's WebKit processes. Note the kernel caps `comm` at
/// 15 bytes, so "WebKitWebProcess" (16) shows as "WebKitWebProces" — match the
/// "WebKit" prefix, not the full name.
#[cfg(target_os = "linux")]
pub fn kill_webkit_children() {
    let me = std::process::id();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return;
    };
    let mut killed: Vec<i32> = Vec::new();
    for entry in rd.flatten() {
        let fname = entry.file_name();
        let Some(pid) = fname.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue; // non-numeric /proc entry
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue; // process gone / unreadable
        };
        let Some((comm, ppid)) = parse_comm_ppid(&stat) else {
            continue;
        };
        // Kernel caps comm at 15 bytes → "WebKitWebProcess" shows as "WebKitWebProces";
        // match the prefix. Restrict to OUR children so we never kill another app's
        // WebKit processes.
        if comm.starts_with("WebKit") && ppid == me {
            // SAFETY: kill(2) with a pid we just read from /proc; SIGKILL has no
            // handler and cannot corrupt our own state.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            killed.push(pid);
        }
    }
    if !killed.is_empty() {
        crate::debug::diag(format!("kill_webkit_children: SIGKILL {killed:?} (GH #28)"));
    }
}

/// Parse `comm` and `ppid` out of a `/proc/<pid>/stat` line.
/// Format: `<pid> (<comm>) <state> <ppid> ...`. `comm` may contain spaces AND
/// parentheses (e.g. a process named `foo) (bar`), so we slice between the FIRST
/// `(` and the LAST `)` rather than tokenizing. The fields after the last `)` are
/// space-separated: `state` then `ppid`.
#[cfg(target_os = "linux")]
fn parse_comm_ppid(stat: &str) -> Option<(&str, u32)> {
    let lp = stat.find('(')?;
    let rp = stat.rfind(')')?;
    if rp < lp {
        return None;
    }
    let comm = &stat[lp + 1..rp];
    let mut fields = stat[rp + 1..].split_whitespace();
    let _state = fields.next()?;
    let ppid: u32 = fields.next()?.parse().ok()?;
    Some((comm, ppid))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::parse_comm_ppid;

    #[test]
    fn parses_plain_stat() {
        // real-ish line; ppid is the 4th field (after the ')')
        let s = "18394 (WebKitWebProces) S 18380 18380 18380 0 -1 4194560 ...";
        let (comm, ppid) = parse_comm_ppid(s).unwrap();
        assert_eq!(comm, "WebKitWebProces"); // 15-byte truncation of WebKitWebProcess
        assert!(comm.starts_with("WebKit"));
        assert_eq!(ppid, 18380);
    }

    #[test]
    fn handles_comm_with_spaces_and_parens() {
        // comm containing spaces and ')' must not break ppid extraction
        let s = "42 (weird ) name)) R 7 7 0 0 -1 0";
        let (comm, ppid) = parse_comm_ppid(s).unwrap();
        assert_eq!(comm, "weird ) name)");
        assert_eq!(ppid, 7);
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_comm_ppid("no parens here").is_none());
        assert!(parse_comm_ppid("12 (comm)").is_none()); // no ppid field
        assert!(parse_comm_ppid("12 (comm) S notanumber").is_none());
    }
}
