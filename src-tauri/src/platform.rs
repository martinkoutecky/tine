const MAX_CLIPBOARD_FILES: usize = 32;
use crate::state::{slot_for_context, GraphContext};

/// Resolve through the same Store containment gate used by asset openers.
/// Only a graph-relative asset name crosses IPC; never trust an image src path.
#[tauri::command]
pub(crate) async fn copy_asset_file_to_clipboard(
    name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    #[cfg(desktop)]
    {
        let path = {
            let slot = slot_for_context(&state)?;
            tine_graph_features::assets::path_for_os_handoff(&slot.store, &name)
                .map_err(crate::commands::feature_asset_access_error)?
        };
        return tauri::async_runtime::spawn_blocking(move || copy_asset_file(&path))
            .await
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(desktop))]
    {
        let _ = (name, state);
        Err("file clipboard is desktop-only".into())
    }
}

#[cfg(desktop)]
fn copy_asset_file(path: &std::path::Path) -> Result<(), String> {
    use clipboard_rs::common::RustImage;
    use clipboard_rs::ClipboardContent;
    #[cfg(not(target_os = "windows"))]
    use clipboard_rs::{Clipboard, ClipboardContext};
    // X11 serves selections from this context. Keep it alive after the command
    // returns, and serialize writes across graph windows.
    let mut contents = Vec::new();
    // The original file is always offered, including unsupported image formats
    // and files above the bitmap budget. Failure to decode only omits pixels.
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() <= 64 * 1024 * 1024 {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes))
            .map_err(|e| e.to_string())?;
        if bytes.len() <= 64 * 1024 * 1024 {
            if cfg!(not(target_os = "windows")) && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                // Linux/macOS accept encoded PNG directly. Preserve it rather
                // than re-encoding, but validate IHDR and the decoded/RGBA
                // budget first, just like the decoding route below.
                if validate_clipboard_png(&bytes).is_ok() {
                    let format = if cfg!(target_os = "macos") {
                        "public.png"
                    } else {
                        "image/png"
                    };
                    contents.push(ClipboardContent::Other(format.into(), bytes));
                }
            } else if let Ok(image) = decode_clipboard_image(&bytes) {
                contents.push(ClipboardContent::Image(
                    clipboard_rs::RustImageData::from_dynamic_image(image),
                ));
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        // clipboard-rs's Files encoder does not percent-escape URI paths.
        // Supply these two flavours ourselves for spaces, #, Unicode and %.
        let uri = tauri::Url::from_file_path(path).map_err(|_| "invalid asset file URL")?;
        contents.push(ClipboardContent::Other(
            "text/uri-list".into(),
            format!("{uri}\r\n").into_bytes(),
        ));
        contents.push(ClipboardContent::Other(
            "x-special/gnome-copied-files".into(),
            format!("copy\n{uri}").into_bytes(),
        ));
    }
    #[cfg(target_os = "windows")]
    {
        // clipboard-rs::set(Image, Files) nests clipboard open guards: the
        // inner image guard closes the clipboard before Files is written.
        // Use one native guard and NoClear writes for all flavours instead.
        let mut bitmap = None;
        let mut png = None;
        if let Some(ClipboardContent::Image(image)) = contents.pop() {
            bitmap = image.to_bitmap().ok();
            png = image.to_png().ok();
        }
        let _clipboard = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
        clipboard_win::empty().map_err(|e| e.to_string())?;
        if let Some(bitmap) = bitmap {
            let _ = clipboard_win::raw::set_bitmap_with(
                bitmap.get_bytes(),
                clipboard_win::options::NoClear,
            );
        }
        if let (Some(png), Some(format)) = (png, clipboard_win::register_format("PNG")) {
            let _ = clipboard_win::raw::set_without_clear(format.get(), png.get_bytes());
        }
        return clipboard_win::raw::set_file_list_with(
            &[path.to_string_lossy()],
            clipboard_win::options::NoClear,
        )
        .map_err(|e| e.to_string());
    }
    #[cfg(target_os = "macos")]
    contents.push(ClipboardContent::Files(vec![path
        .to_string_lossy()
        .into_owned()]));
    #[cfg(not(target_os = "windows"))]
    {
        static CONTEXT: std::sync::Mutex<Option<ClipboardContext>> = std::sync::Mutex::new(None);
        let mut context = CONTEXT.lock().map_err(|e| e.to_string())?;
        if context.is_none() {
            *context = Some(ClipboardContext::new().map_err(|e| e.to_string())?);
        }
        context
            .as_ref()
            .unwrap()
            .set(contents)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub(crate) async fn read_android_clipboard_image(
    app: tauri::AppHandle,
) -> Result<tauri::ipc::Response, String> {
    #[cfg(target_os = "android")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            crate::android_clipboard::read_png(&app)
                .map(|bytes| tauri::ipc::Response::new(bytes.unwrap_or_default()))
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(tauri::ipc::Response::new(Vec::<u8>::new()))
    }
}

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
    #[cfg(target_os = "linux")]
    if let Ok(bytes) = context.get_buffer("text/uri-list") {
        return Ok(clipboard_file_list(file_uri_paths(&bytes)));
    }
    let paths = context.get_files().map_err(|e| e.to_string())?;
    Ok(clipboard_file_list(paths))
}

#[cfg(target_os = "linux")]
fn file_uri_paths(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| tauri::Url::parse(line).ok()?.to_file_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[cfg(desktop)]
#[cfg(test)]
mod asset_copy_tests {
    use super::*;

    fn oversized_png(original: &[u8]) -> Vec<u8> {
        // Rewrite IHDR and its CRC without allocating the described pixels.
        let mut bytes = original.to_vec();
        bytes[16..20].copy_from_slice(&16384_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&16384_u32.to_be_bytes());
        let mut crc = !0_u32;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320_u32 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        bytes
    }

    #[test]
    fn png_header_and_rgba_budget_are_checked_without_decoding_pixels() {
        let image = image::GrayImage::new(2, 3);
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let original = png.into_inner();
        assert!(validate_clipboard_png(&original).is_ok());
        let oversized = oversized_png(&original);
        assert!(validate_clipboard_png(&oversized)
            .unwrap_err()
            .contains("limit"));
        assert!(decode_clipboard_image(&oversized)
            .unwrap_err()
            .contains("limit"));
        assert!(validate_clipboard_png(b"\x89PNG\r\n\x1a\n").is_err());
        let mut bad_crc = original;
        bad_crc[16] ^= 1;
        assert!(validate_clipboard_png(&bad_crc).is_err());
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    #[ignore = "requires the native desktop clipboard"]
    fn native_file_copy_keeps_file_and_bitmap_together() {
        use clipboard_rs::{Clipboard, ClipboardContext};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image #%.png");
        image::RgbaImage::new(2, 3).save(&path).unwrap();
        copy_asset_file(&path).unwrap();
        let clipboard = ClipboardContext::new().unwrap();
        assert_eq!(clipboard.get_files().unwrap(), vec![path.to_string_lossy()]);
        assert!(clipboard.get_image().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn file_urls_decode_escaped_filenames_and_refuse_remote_schemes() {
        assert_eq!(
            file_uri_paths(b"# comment\r\nfile:///tmp/a%20%23%25.png\r\nhttps://example.com/a\n"),
            vec!["/tmp/a #%.png"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires X11 and xclip; run under xvfb-run"]
    fn native_file_copy_offers_original_uri_and_bounded_bitmap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image #%.png");
        image::RgbaImage::new(2, 3).save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        copy_asset_file(&path).unwrap();
        let read = |flavour: &str| {
            let output = std::process::Command::new("xclip")
                .args(["-o", "-selection", "clipboard", "-t", flavour])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        assert_eq!(
            file_uri_paths(&read("text/uri-list")),
            vec![path.to_string_lossy()]
        );
        assert!(read("x-special/gnome-copied-files").starts_with(b"copy\nfile://"));
        assert_eq!(
            image::load_from_memory(&read("image/png")).unwrap().width(),
            2
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(read("image/png"), original);
        // A tiny encoded PNG can describe an RGBA buffer above the budget.
        // Rewrite IHDR dimensions and CRC without allocating those pixels.
        let oversized = oversized_png(&original);
        assert!(decode_clipboard_image(&oversized)
            .unwrap_err()
            .contains("limit"));
        for refused in [oversized, b"\x89PNG\r\n\x1a\n".to_vec()] {
            std::fs::write(&path, &refused).unwrap();
            copy_asset_file(&path).unwrap();
            assert_eq!(
                file_uri_paths(&read("text/uri-list")),
                vec![path.to_string_lossy()]
            );
            assert!(!String::from_utf8_lossy(&read("TARGETS")).contains("image/png"));
            assert_eq!(std::fs::read(&path).unwrap(), refused);
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(65 * 1024 * 1024)
            .unwrap();
        copy_asset_file(&path).unwrap();
        assert_eq!(
            file_uri_paths(&read("text/uri-list")),
            vec![path.to_string_lossy()]
        );
        assert!(!String::from_utf8_lossy(&read("TARGETS")).contains("image/png"));
        assert_eq!(
            read_clipboard_files().unwrap().files[0].size,
            65 * 1024 * 1024
        );
    }
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

/// How long the foreground clipboard tool gets to report whether it took the
/// image. `wl-copy`/`xclip`/`xsel` hand the selection to a forked server and the
/// foreground process exits at once, so a healthy tool answers well inside this;
/// a tool that is still running afterwards is serving the selection from the
/// foreground and counts as success.
#[cfg(target_os = "linux")]
const CLIPBOARD_TOOL_VERDICT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Reap `child` on a detached thread so a process we stop caring about never
/// becomes a zombie (I-21: every exit path reaps its child).
#[cfg(desktop)]
fn reap_in_background(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Spawn a fire-and-forget helper (browser or file-manager opener) and reap it
/// when it exits. The caller learns only whether it started.
#[cfg(desktop)]
pub(crate) fn spawn_reaped(command: &mut std::process::Command) -> Result<(), String> {
    command
        .spawn()
        .map(reap_in_background)
        .map_err(|error| error.to_string())
}

/// Stop a child we are abandoning and collect it. `kill` may fail because it
/// already exited; `wait` then still reaps it.
#[cfg(target_os = "linux")]
fn kill_and_reap(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Feed `bytes` to each clipboard tool in turn. A tool counts as having taken
/// the image only when it exits successfully, or is still serving it after
/// `verdict`; a tool that fails to start, cannot be written to, or exits with a
/// failure status falls through to the next one, and every child is reaped on
/// every path. The error names the last tool's failure.
#[cfg(target_os = "linux")]
fn copy_with_tools(
    tools: &[(String, Vec<String>)],
    bytes: &[u8],
    verdict: std::time::Duration,
) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut last_err = String::from("no clipboard tool found (install wl-clipboard or xclip)");
    for (prog, args) in tools {
        let spawned = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("{prog}: {e}");
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let written = stdin.write_all(bytes);
            drop(stdin); // EOF: the tool now owns the whole image
            if let Err(e) = written {
                kill_and_reap(&mut child);
                last_err = format!("{prog}: write stdin: {e}");
                continue;
            }
        }
        let deadline = std::time::Instant::now() + verdict;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    last_err = format!("{prog}: exited with {status}");
                    break;
                }
                Ok(None) if std::time::Instant::now() >= deadline => {
                    // Still running: it serves the selection from the foreground.
                    reap_in_background(child);
                    return Ok(());
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(e) => {
                    kill_and_reap(&mut child);
                    last_err = format!("{prog}: wait: {e}");
                    break;
                }
            }
        }
    }
    Err(last_err)
}

/// On Linux, hand a PNG to the OS clipboard via `wl-copy` (Wayland) or `xclip`/
/// `xsel` (X11). These tools FORK a daemon that serves the selection until it's
/// replaced — which is exactly what an image clipboard needs. `arboard` (what the
/// Tauri plugin uses) tries to do this in-process and frequently drops the image
/// on WebKitGTK, so we prefer the native tools and only fall back to the plugin.
#[cfg(target_os = "linux")]
fn linux_copy_image(bytes: &[u8]) -> Result<(), String> {
    // Prefer the tool matching the active session, but try all — a Wayland session
    // under Xwayland may have only xclip, and vice versa.
    let tool = |prog: &str| -> (String, Vec<String>) {
        let args: &[&str] = match prog {
            "wl-copy" => &["--type", "image/png"],
            "xclip" => &["-selection", "clipboard", "-t", "image/png"],
            "xsel" => &["--clipboard", "--input"],
            _ => &[],
        };
        (
            prog.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
        )
    };
    let names: [&str; 3] = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        ["wl-copy", "xclip", "xsel"]
    } else {
        ["xclip", "xsel", "wl-copy"]
    };
    let order: Vec<_> = names.iter().map(|name| tool(name)).collect();
    copy_with_tools(&order, bytes, CLIPBOARD_TOOL_VERDICT)
}

/// Starting the clipboard tool (PATH search, exec, writing the image to its
/// stdin) runs on the blocking pool, off the UI thread (GH #623, I-21).
#[tauri::command]
pub(crate) async fn copy_image_to_clipboard(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let bytes = clipboard_request_bytes(request.body())?;
    tauri::async_runtime::spawn_blocking(move || {
        // The clipboard plugin's mobile `write_image` is "Unsupported on this
        // platform", so Android publishes the image through its own plugin
        // (GH #654).
        #[cfg(target_os = "android")]
        {
            use base64::Engine;
            let png = clipboard_png(&bytes)?;
            let bytes_b64 = base64::engine::general_purpose::STANDARD.encode(png);
            return crate::android_clipboard::copy_png(&app, &bytes_b64);
        }
        #[cfg(not(target_os = "android"))]
        {
            use tauri_plugin_clipboard_manager::ClipboardExt;
            #[cfg(target_os = "linux")]
            if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && linux_copy_image(&bytes).is_ok() {
                return Ok(());
            }
            let image = decode_clipboard_image(&bytes)?;
            #[cfg(target_os = "linux")]
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                && linux_copy_image(&encode_clipboard_png(&image)?).is_ok()
            {
                return Ok(());
            }
            // Reuse decoded pixels if the Linux tools failed too.
            let rgba = image.into_rgba8();
            let (width, height) = rgba.dimensions();
            let img = tauri::image::Image::new_owned(rgba.into_raw(), width, height);
            app.clipboard().write_image(&img).map_err(|e| e.to_string())
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Tauri 2.12.3 never supplies Raw on Android; desktop's postMessage fallback
/// also serializes Uint8Array as a JSON array. Bound every form before making
/// an owned byte buffer. Tauri has already materialized the request itself.
fn clipboard_request_bytes(body: &tauri::ipc::InvokeBody) -> Result<Vec<u8>, String> {
    const MAX_BYTES: usize = 64 * 1024 * 1024;
    let check_len = |len: usize| {
        if len == 0 || len > MAX_BYTES {
            Err("image is empty or exceeds 64 MiB clipboard limit".to_string())
        } else {
            Ok(())
        }
    };
    match body {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            check_len(bytes.len())?;
            Ok(bytes.clone())
        }
        tauri::ipc::InvokeBody::Json(value) => {
            if let Some(b64) = value.get("bytesB64").and_then(serde_json::Value::as_str) {
                // Shared asset gate checks encoded and decoded lengths before
                // base64 allocation, including the padding boundary.
                let bytes = crate::commands::decode_asset_b64(b64)?;
                check_len(bytes.len())?;
                Ok(bytes)
            } else if let Some(values) = value.as_array() {
                check_len(values.len())?;
                // Reject malformed values before allocating the output too.
                if !values.iter().all(|v| v.as_u64().is_some_and(|n| n <= 255)) {
                    return Err("expected image byte array".into());
                }
                Ok(values.iter().map(|v| v.as_u64().unwrap() as u8).collect())
            } else {
                Err("expected raw image bytes or base64 image payload".into())
            }
        }
    }
}

fn decode_clipboard_image(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    check_clipboard_image_budget(&decoder)?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn check_clipboard_image_budget(decoder: &impl image::ImageDecoder) -> Result<(), String> {
    // Unlike ImageReader::decode, into_decoder doesn't reserve the output
    // buffer. Bound both the decoded buffer and the eventual RGBA conversion
    // before allocating either (a small JPEG can describe enormous dimensions).
    let (width, height) = decoder.dimensions();
    image::Limits::default()
        .reserve(
            decoder
                .total_bytes()
                .max((u64::from(width) * u64::from(height)).saturating_mul(4)),
        )
        .map_err(|e| e.to_string())
}

#[cfg(desktop)]
fn validate_clipboard_png(bytes: &[u8]) -> Result<(), String> {
    // The PNG decoder validates IHDR (including CRC, colour/depth and nonzero
    // dimensions) without allocating the pixel buffer or re-encoding bytes.
    let decoder =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png)
            .into_decoder()
            .map_err(|e| e.to_string())?;
    check_clipboard_image_budget(&decoder)
}

#[cfg(any(target_os = "android", all(test, target_os = "linux")))]
fn clipboard_png(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    Ok(std::borrow::Cow::Owned(encode_clipboard_png(
        &decode_clipboard_image(bytes)?,
    )?))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn encode_clipboard_png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod image_copy_tests {
    use super::*;

    #[test]
    fn clipboard_accepts_android_json_and_desktop_transport_fallback() {
        use tauri::ipc::InvokeBody::{Json, Raw};
        let bytes = vec![137, 80, 78, 71];
        // Actual ipc-protocol.js postMessage payloads (Android string and
        // desktop Uint8Array serialized by process-ipc-message-fn.js).
        let android = serde_json::from_str(r#"{"bytesB64":"iVBORw=="}"#).unwrap();
        let fallback = serde_json::from_str("[137,80,78,71]").unwrap();
        for body in [Raw(bytes.clone()), Json(android), Json(fallback)] {
            assert_eq!(clipboard_request_bytes(&body).unwrap(), bytes);
        }
        for value in [
            "null",
            "{}",
            "[]",
            "[256]",
            "[-1]",
            "[1.5]",
            "[null]",
            r#"{"bytesB64":""}"#,
            r#"{"bytesB64":"invalid"}"#,
        ] {
            assert!(clipboard_request_bytes(&Json(serde_json::from_str(value).unwrap())).is_err());
        }
        assert!(clipboard_request_bytes(&Raw(Vec::new())).is_err());
        assert!(clipboard_request_bytes(&Raw(vec![0; 64 * 1024 * 1024 + 1])).is_err());
        // Maximum encoded length, but padding implies one byte over the bound.
        let b64 = "A".repeat((64_usize * 1024 * 1024).div_ceil(3) * 4);
        assert!(
            clipboard_request_bytes(&Json(serde_json::json!({"bytesB64": b64})))
                .unwrap_err()
                .contains("64 MiB")
        );
    }

    #[test]
    fn clipboard_decode_preserves_pixels_and_rejects_invalid_images() {
        let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([12, 34, 56, 78]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(
            decode_clipboard_image(png.get_ref()).unwrap().into_rgba8(),
            image
        );
        assert!(decode_clipboard_image(b"invalid").is_err());
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(matches!(
            clipboard_png(png.get_ref()).unwrap(),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn clipboard_jpeg_preserves_browser_orientation() {
        let image = image::RgbImage::from_fn(16, 24, |x, y| {
            image::Rgb([(x * 13) as u8, (y * 9) as u8, ((x + y) * 5) as u8])
        });
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let jpeg = jpeg.into_inner();
        // JPEG APP1: Exif TIFF, one orientation entry (6 = 90 degrees CW).
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&jpeg[2..]);
        let decoded = decode_clipboard_image(&oriented).unwrap().into_rgba8();
        let original = decode_clipboard_image(&jpeg).unwrap().into_rgba8();
        assert_eq!(decoded, image::imageops::rotate90(&original));
        assert_ne!(decoded, image::imageops::rotate270(&original));
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let png = clipboard_png(&oriented).unwrap();
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(decode_clipboard_image(&png).unwrap().into_rgba8(), decoded);
        }
    }

    #[test]
    fn clipboard_rejects_huge_decoded_jpeg_before_pixel_allocation() {
        let image = image::RgbImage::from_pixel(2, 3, image::Rgb([12, 34, 56]));
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let mut jpeg = jpeg.into_inner();
        let sof = jpeg.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
        // The baseline JPEG SOF describes a 16384-square RGB image (768 MiB),
        // despite the tiny encoded payload. RGBA would require 1 GiB.
        jpeg[sof + 5..sof + 7].copy_from_slice(&16384_u16.to_be_bytes());
        jpeg[sof + 7..sof + 9].copy_from_slice(&16384_u16.to_be_bytes());
        assert!(decode_clipboard_image(&jpeg)
            .unwrap_err()
            .to_lowercase()
            .contains("limit"));
    }
}

/// What the backend knows about the rendering path, so the UI can warn — loudly
/// — when Tine is painting on the CPU. Speed is the whole pitch, and a silent
/// software-rendering fallback makes scrolling feel sluggish; better to say so.
/// `software_forced` is true only when we *know* GPU compositing is off because
/// an env var disabled it: the Linux startup policy sets
/// WEBKIT_DISABLE_DMABUF_RENDERER for TINE_GPU=0 or an NVIDIA driver older than 610 (GH #777),
/// or the user set WEBKIT_DISABLE_DMABUF_RENDERER /
/// WEBKIT_DISABLE_COMPOSITING_MODE themselves. A *silent* driver/EGL fallback
/// (DMABUF requested but EGL init failed) does NOT show up here — that's detected
/// in the webview from the WebGL renderer string (llvmpipe/swrast ⇒ software).
/// `appimage` lets the message steer AppImage users to the deb/rpm, which use the
/// host graphics stack instead of the bundled one. `flatpak` is the packaging
/// answer for the updater: a Flatpak sandbox's `/app` is read-only, so the
/// in-place update install could only fail and the updater offers a download
/// instead (see `in_flatpak`). Env reads are cross-platform (these vars are
/// simply absent on macOS/Windows ⇒ all false).
#[derive(serde::Serialize)]
pub(crate) struct GpuEnv {
    software_forced: bool,
    appimage: bool,
    flatpak: bool,
    packaged: Option<bool>,
    /// A portable Windows instance (GH #719): it updates by download, since the
    /// updater's installer would install a second, per-user copy.
    portable: bool,
}

/// Package identity is a runtime property, shared by Store and sideloaded MSIX.
/// None preserves a failed probe so only the installer can fail closed.
#[cfg(target_os = "windows")]
pub(crate) fn packaged() -> Option<bool> {
    use windows_sys::Win32::Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER};
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut length = 0;
    // SAFETY: length is writable; a null buffer requests only the required size.
    let result = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    match result as u32 {
        ERROR_INSUFFICIENT_BUFFER => Some(true),
        APPMODEL_ERROR_NO_PACKAGE => Some(false),
        _ => None,
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
pub(crate) fn packaged() -> Option<bool> {
    Some(false)
}

/// Whether the process runs inside a Flatpak sandbox: Flatpak sets `FLATPAK_ID`
/// and mounts `/.flatpak-info` in every sandbox, and either one is enough. The
/// decision is separated from the environment it reads so a test can drive both
/// signals.
pub(crate) fn in_flatpak(flatpak_id_set: bool, flatpak_info_present: bool) -> bool {
    flatpak_id_set || flatpak_info_present
}

#[tauri::command]
pub(crate) fn gpu_env() -> GpuEnv {
    let set = |k: &str| std::env::var_os(k).is_some();
    GpuEnv {
        software_forced: set("WEBKIT_DISABLE_DMABUF_RENDERER")
            || set("WEBKIT_DISABLE_COMPOSITING_MODE")
            || std::env::var("TINE_GPU").as_deref() == Ok("0"),
        appimage: set("APPIMAGE"),
        packaged: packaged(),
        flatpak: in_flatpak(
            set("FLATPAK_ID"),
            std::path::Path::new("/.flatpak-info").exists(),
        ),
        portable: crate::portable::active(),
    }
}

/// Turn a `file:` URL into the local path it names, or `None` when it names
/// something that is not a plain local file.
///
/// Delegated to the WHATWG URL parser (`tauri::Url`, the re-exported `url`
/// crate) rather than string surgery, because the shapes users actually write
/// are not uniform (GH #444): Logseq's `file://D:\test.txt` puts the drive in
/// the authority position, Obsidian's `<file:///D:\test.txt>` uses backslashes
/// after a triple slash, and either can carry percent escapes. The parser
/// already resolves all three, and rejects a remote authority
/// (`file://host/share`) everywhere except Windows, where it is a real UNC path.
pub(crate) fn file_url_to_path(url: &str) -> Option<std::path::PathBuf> {
    let parsed = tauri::Url::parse(url).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }
    let path = parsed.to_file_path().ok()?;
    (path.as_os_str().len() > 0).then_some(path)
}

/// The OS path of `relative` (a `/`-separated graph-relative path that a file
/// link resolved to as OG does, GH #457, src/render/relativeFileLink.ts) inside
/// the graph at `root`. Refusal scenario (in scope: malformed or imported
/// Markdown, a synced note): a link in a note is untrusted content, so a path
/// that leaves the graph folder, lexically (`..` above the root, an absolute or
/// drive component) or through a symlink, is refused rather than handed to the
/// OS opener. That is an intentional difference from OG, which opens absolute
/// paths and lets its path join climb out of the graph. A missing target
/// is reported instead of opening an unrelated window. Read-only; Cost
/// O(path components) plus two canonicalizations.
pub(crate) fn graph_relative_target(
    root: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, String> {
    const OUTSIDE: &str = "that link points outside the graph folder";
    if relative.starts_with(['/', '\\']) {
        return Err(OUTSIDE.into());
    }
    let mut clean = std::path::PathBuf::new();
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if !clean.pop() {
                    return Err(OUTSIDE.into());
                }
            }
            part => {
                let component = std::path::Path::new(part);
                if part.contains(':') || component.has_root() {
                    return Err(OUTSIDE.into());
                }
                clean.push(component);
            }
        }
    }
    let graph = root
        .canonicalize()
        .map_err(|error| format!("the graph folder could not be read: {error}"))?;
    let target = root
        .join(&clean)
        .canonicalize()
        .map_err(|_| format!("{} could not be found", clean.display()))?;
    if !target.starts_with(&graph) {
        return Err(OUTSIDE.into());
    }
    Ok(target)
}

/// Where a clicked link is headed, decided before anything is spawned so the
/// routing decision is testable on its own — the spawn that follows is not.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExternalOpen {
    /// A `file:` link naming a local path: the OS default application opens it.
    LocalPath(std::path::PathBuf),
    /// http(s) or mailto: the URL opener handles it.
    Url,
    /// Another app's scheme (`zotero:`, `obsidian:`, `shortcuts:` …) that the
    /// user confirmed for this click (GH #503): the URL opener hands it to the
    /// OS, which picks the registered app.
    CustomScheme,
}

/// Schemes no outbound link ever opens, confirmed or not: they run code or
/// carry inline content in whatever receives them (I-22). `file:` and `tine:`
/// have their own routes and are not custom schemes either.
const NEVER_OPENED_SCHEMES: &[&str] = &["javascript", "vbscript", "data", "blob", "about"];

/// The RFC 3986 scheme of `url`, lowercased, or `None` when it has none. A
/// single letter is a Windows drive (`C:\notes`), not a scheme.
pub(crate) fn url_scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (scheme.len() > 1
        && first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then(|| scheme.to_ascii_lowercase())
}

/// OG parity for `file:` (og-1.0.0 `electron/window.cljs` `open-default-app!`):
/// Logseq sends http(s)/mailto to `shell.openExternal` and hands EVERY other
/// scheme, `file:` included, to the OS default application with no confirmation
/// and no extension filtering. Tine matches that for `file:`: a graph is the
/// user's own content, and a link that silently does nothing (GH #444, where
/// this gate rejected `file:` outright and the frontend then discarded the
/// rejection) is worse for the honest case than the parity behaviour is for the
/// hostile one.
///
/// Any other scheme (GH #503: `zotero://`, `obsidian://`, `shortcuts://` …) OG
/// also hands to the OS silently. Tine opens it only when `confirmed_scheme`
/// names that exact scheme, which the frontend supplies after the user
/// confirmed a prompt naming the scheme and the full target. Refusal scenario
/// (in scope: malformed or imported Markdown, a synced note from another
/// device): a link in a note is untrusted content, and an unconfirmed click
/// must not launch an arbitrary registered handler. `javascript:`/`data:`-like
/// schemes are refused even when confirmed (I-22).
pub(crate) fn external_open_plan(
    url: &str,
    confirmed_scheme: Option<&str>,
) -> Result<ExternalOpen, String> {
    if url.starts_with("file:") {
        let path = file_url_to_path(url)
            .ok_or_else(|| "that file link does not name a local file".to_string())?;
        return Ok(ExternalOpen::LocalPath(path));
    }
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
        return Ok(ExternalOpen::Url);
    }
    let Some(scheme) = url_scheme(url) else {
        return Err("unsupported url scheme".into());
    };
    if NEVER_OPENED_SCHEMES.contains(&scheme.as_str()) || scheme == "tine" {
        return Err(format!("{scheme}: links are never opened"));
    }
    if confirmed_scheme.map(str::to_ascii_lowercase).as_deref() != Some(scheme.as_str()) {
        return Err(format!("opening a {scheme}: link needs your confirmation"));
    }
    Ok(ExternalOpen::CustomScheme)
}

/// Open a web/mail URL, a local `file:` link, or a user-confirmed custom-scheme
/// link (GH #503) in the user's default external application. The URL is passed as a single argument (no shell), so it can't
/// inject commands.
/// Starting the opener (PATH search, exec) runs on the blocking pool, off the
/// UI thread (GH #623, I-21).
#[tauri::command]
pub(crate) async fn open_external(
    app: tauri::AppHandle,
    url: String,
    confirmed_scheme: Option<String>,
) -> Result<(), String> {
    if url.starts_with("tine:") {
        crate::deep_links::receive_url(&app, url);
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        if let ExternalOpen::LocalPath(path) =
            external_open_plan(&url, confirmed_scheme.as_deref())?
        {
            // Every OS opener accepts a path that is not there — `explorer.exe` even
            // opens an unrelated window for one — so without this check a stale link
            // reproduces the reported "clicking does nothing" from the other side.
            if !path.exists() {
                return Err(format!("{} could not be found", path.display()));
            }
            #[cfg(desktop)]
            return open_page_source(&path);
            #[cfg(not(desktop))]
            {
                let _ = (path, &app);
                return Err("opening local files is available on desktop only".into());
            }
        }
        // Linux/macOS: spawn the desktop-session URL opener directly (Linux needs
        // the env-scrubbed browser policy; see opener_command_env).
        #[cfg(all(desktop, not(target_os = "windows")))]
        {
            let _ = &app;
            #[cfg(target_os = "linux")]
            let mut command = opener_command_env("xdg-open", OpenerEnvPolicy::Browser);
            #[cfg(target_os = "macos")]
            let mut command = opener_command("open");
            spawn_reaped(command.arg(&url))
        }
        // Windows: do NOT spawn `explorer <url>`. explorer.exe treats an http(s)/
        // mailto argument as a shell item and frequently opens a File Explorer
        // window instead of handing the URL to the default browser/mail client
        // (GH #215). Route the open through the opener plugin, which calls
        // ShellExecute — the canonical Windows "open this URL" API and the same
        // path mobile already uses successfully.
        #[cfg(all(desktop, target_os = "windows"))]
        {
            use tauri_plugin_opener::OpenerExt;
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| e.to_string())
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
    })
    .await
    .map_err(|error| error.to_string())?
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
    #[cfg(target_os = "linux")]
    return opener_command_env(prog, OpenerEnvPolicy::Media);

    #[cfg(not(target_os = "linux"))]
    opener_command_env(prog)
}

#[cfg(all(desktop, target_os = "linux"))]
enum OpenerEnvPolicy {
    Media,
    Browser,
}

#[cfg(desktop)]
fn opener_command_env(
    prog: &str,
    #[cfg(target_os = "linux")] policy: OpenerEnvPolicy,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(prog);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let appdir = std::env::var_os("APPDIR");
        let env = match policy {
            // Start media players from a small desktop-session allowlist.
            // AppImage runtimes inject more variables than a blacklist can safely
            // anticipate; one leaked GStreamer/GIO/GTK loader path is enough to
            // make a host VLC process load incompatible bundled libraries.
            OpenerEnvPolicy::Media => sanitized_opener_env(std::env::vars_os(), appdir),
            // Browser scheme resolution needs the full desktop session env, so
            // remove only bundle and loader state that can poison host programs.
            OpenerEnvPolicy::Browser => sanitized_browser_env(std::env::vars_os(), appdir),
        };
        cmd.env_clear().envs(env);
        // Create a new session, not merely a new process group. This prevents an
        // external player from inheriting Tine's controlling-terminal/session
        // lifetime. `setsid` is async-signal-safe and this closure runs after
        // fork and before exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    cmd
}

#[cfg(desktop)]
pub(crate) fn open_page_source(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut command = opener_command("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = opener_command("open");
    #[cfg(target_os = "windows")]
    let mut command = opener_command("explorer");
    spawn_reaped(command.arg(path))
}

#[cfg(desktop)]
pub(crate) fn reveal_page_source(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return spawn_reaped(opener_command("open").arg("-R").arg(path));

    #[cfg(target_os = "windows")]
    return spawn_reaped(opener_command("explorer").arg("/select,").arg(path));

    #[cfg(target_os = "linux")]
    {
        // Freedesktop's file-manager interface selects the exact file. If the
        // desktop has no implementation, opening the containing folder is still
        // a useful and portable fallback.
        if let Ok(uri) = tauri::Url::from_file_path(path) {
            let status = opener_command("dbus-send")
                .args([
                    "--session",
                    "--print-reply",
                    "--dest=org.freedesktop.FileManager1",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    &format!("array:string:{uri}"),
                    "string:",
                ])
                .status();
            if status.is_ok_and(|status| status.success()) {
                return Ok(());
            }
        }
        let parent = path
            .parent()
            .ok_or_else(|| "page source has no parent directory".to_string())?;
        spawn_reaped(opener_command("xdg-open").arg(parent))
    }
}

#[cfg(target_os = "linux")]
fn sanitized_opener_env(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    appdir: Option<std::ffi::OsString>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    const KEEP: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "DESKTOP_SESSION",
        // xdg-open uses these to select kde-open{version}. Dropping them makes
        // Plasma fall back to the obsolete kfmclient path (or do nothing), so
        // opener isolation must retain desktop identity while still removing
        // loader/plugin paths from the AppImage runtime.
        "KDE_FULL_SESSION",
        "KDE_SESSION_VERSION",
        "DBUS_SESSION_BUS_ADDRESS",
        "XAUTHORITY",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_DIRS",
        "TMPDIR",
        "TMP",
        "TEMP",
    ];
    let appdir = appdir.map(std::path::PathBuf::from);
    vars.into_iter()
        .filter_map(|(key, value)| {
            let key_text = key.to_string_lossy();
            if !KEEP.contains(&key_text.as_ref()) && !key_text.starts_with("LC_") {
                return None;
            }
            let value = if matches!(key_text.as_ref(), "PATH" | "XDG_DATA_DIRS") {
                if let Some(appdir) = &appdir {
                    let clean = std::env::split_paths(&value)
                        .filter(|part| !part.starts_with(appdir))
                        .collect::<Vec<_>>();
                    std::env::join_paths(clean).ok()?
                } else {
                    value
                }
            } else {
                value
            };
            Some((key, value))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn sanitized_browser_env(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    appdir: Option<std::ffi::OsString>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    const DROP: &[&str] = &[
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "GTK_PATH",
        "GTK_EXE_PREFIX",
        "GTK_DATA_PREFIX",
        "GTK_IM_MODULE_FILE",
        "GIO_MODULE_DIR",
        "GIO_EXTRA_MODULES",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SCANNER",
        "GST_REGISTRY",
        "GST_REGISTRY_1_0",
        "GDK_PIXBUF_MODULE_FILE",
        "GDK_PIXBUF_MODULEDIR",
        "QT_PLUGIN_PATH",
        "QT_QPA_PLATFORM_PLUGIN_PATH",
        "PYTHONHOME",
        "PYTHONPATH",
        "PERLLIB",
        "PERL5LIB",
        "FONTCONFIG_PATH",
        "FONTCONFIG_FILE",
        "GDK_BACKEND",
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
        "APPDIR",
        "APPIMAGE",
        "ARGV0",
        "OWD",
    ];
    let appdir = appdir.map(std::path::PathBuf::from);
    vars.into_iter()
        .filter_map(|(key, value)| {
            let key_text = key.to_string_lossy();
            if DROP.contains(&key_text.as_ref()) {
                return None;
            }
            let value = if matches!(key_text.as_ref(), "PATH" | "XDG_DATA_DIRS") {
                if let Some(appdir) = &appdir {
                    let clean = std::env::split_paths(&value)
                        .filter(|part| !part.starts_with(appdir))
                        .collect::<Vec<_>>();
                    std::env::join_paths(clean).ok()?
                } else {
                    value
                }
            } else {
                value
            };
            Some((key, value))
        })
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod opener_tests {
    use super::{sanitized_browser_env, sanitized_opener_env};
    use std::ffi::OsString;

    #[test]
    fn keeps_session_identity_but_drops_bundle_loader_state() {
        let vars = vec![
            ("PATH", "/tmp/.mount_tine/bin:/usr/bin"),
            ("XDG_DATA_DIRS", "/tmp/.mount_tine/share:/usr/share"),
            ("HOME", "/home/test"),
            ("DISPLAY", ":0"),
            ("XDG_SESSION_TYPE", "wayland"),
            ("KDE_FULL_SESSION", "true"),
            ("KDE_SESSION_VERSION", "6"),
            ("LC_ALL", "C.UTF-8"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_tine/lib"),
            ("GST_PLUGIN_PATH", "/tmp/.mount_tine/gstreamer"),
            ("APPIMAGE", "/tmp/Tine.AppImage"),
        ]
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)));

        let clean = sanitized_opener_env(vars, Some(OsString::from("/tmp/.mount_tine")));
        let clean = clean
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(clean.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(
            clean.get("XDG_DATA_DIRS").map(String::as_str),
            Some("/usr/share")
        );
        assert_eq!(clean.get("HOME").map(String::as_str), Some("/home/test"));
        assert_eq!(clean.get("DISPLAY").map(String::as_str), Some(":0"));
        assert_eq!(
            clean.get("XDG_SESSION_TYPE").map(String::as_str),
            Some("wayland")
        );
        assert_eq!(
            clean.get("KDE_FULL_SESSION").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            clean.get("KDE_SESSION_VERSION").map(String::as_str),
            Some("6")
        );
        assert_eq!(clean.get("LC_ALL").map(String::as_str), Some("C.UTF-8"));
        assert!(!clean.contains_key("LD_LIBRARY_PATH"));
        assert!(!clean.contains_key("GST_PLUGIN_PATH"));
        assert!(!clean.contains_key("APPIMAGE"));
    }

    #[test]
    fn browser_keeps_desktop_state_but_drops_bundle_loader_state() {
        let vars = vec![
            ("PATH", "/tmp/.mount_tine/bin:/usr/bin"),
            ("XDG_CONFIG_DIRS", "/etc/xdg"),
            ("XDG_SESSION_DESKTOP", "KDE"),
            ("GTK_USE_PORTAL", "1"),
            ("BROWSER", "firefox"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_tine/lib"),
            ("GIO_MODULE_DIR", "/tmp/.mount_tine/gio"),
            ("GST_PLUGIN_SYSTEM_PATH", "/tmp/.mount_tine/gstreamer"),
            ("APPDIR", "/tmp/.mount_tine"),
            ("APPIMAGE", "/tmp/Tine.AppImage"),
        ]
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)));

        let clean = sanitized_browser_env(vars, Some(OsString::from("/tmp/.mount_tine")));
        let clean = clean
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(clean.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(
            clean.get("XDG_CONFIG_DIRS").map(String::as_str),
            Some("/etc/xdg")
        );
        assert_eq!(
            clean.get("XDG_SESSION_DESKTOP").map(String::as_str),
            Some("KDE")
        );
        assert_eq!(clean.get("GTK_USE_PORTAL").map(String::as_str), Some("1"));
        assert_eq!(clean.get("BROWSER").map(String::as_str), Some("firefox"));
        assert!(!clean.contains_key("LD_LIBRARY_PATH"));
        assert!(!clean.contains_key("GIO_MODULE_DIR"));
        assert!(!clean.contains_key("GST_PLUGIN_SYSTEM_PATH"));
        assert!(!clean.contains_key("APPDIR"));
        assert!(!clean.contains_key("APPIMAGE"));
    }
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
        crate::debug::diag_private(
            "webkit-children-killed",
            format!("kill_webkit_children: SIGKILL {killed:?} (GH #28)"),
        );
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
    use super::in_flatpak;

    /// A Flatpak must be told apart from a native build by either sandbox
    /// signal, so the updater never attempts an in-place install under `/app`.
    #[test]
    fn a_flatpak_sandbox_is_recognized_by_either_signal() {
        assert!(in_flatpak(true, true));
        assert!(in_flatpak(true, false));
        assert!(in_flatpak(false, true));
        assert!(!in_flatpak(false, false));
    }
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

#[cfg(test)]
mod file_url_tests {
    use super::{external_open_plan, file_url_to_path, url_scheme, ExternalOpen};

    /// The three shapes GH #444 reported, plus the ordinary POSIX one. On
    /// Windows all four resolve to a real local path; elsewhere the drive-letter
    /// forms still parse to a path, they just are not meaningful there — the
    /// point of the assertion is that the parser does NOT reject them and does
    /// NOT hand back a percent-escaped or backslash-mangled string.
    #[test]
    fn accepts_the_link_shapes_users_actually_write() {
        // Logseq: the drive letter sits in the authority position.
        let logseq = file_url_to_path("file://D:\\test.txt").expect("logseq drive form");
        assert!(logseq.to_string_lossy().contains("D:"));
        assert!(logseq.to_string_lossy().ends_with("test.txt"));

        // Obsidian: triple slash, then a backslash separator.
        let obsidian = file_url_to_path("file:///D:\\test.txt").expect("obsidian drive form");
        assert_eq!(obsidian, logseq);

        // The canonical forward-slash spelling agrees with both.
        assert_eq!(file_url_to_path("file:///D:/test.txt"), Some(logseq));

        // Percent escapes are decoded, so a spaced filename opens.
        let spaced = file_url_to_path("file:///tmp/a%20b.txt").expect("percent-decoded");
        assert_eq!(
            spaced.file_name().and_then(|name| name.to_str()),
            Some("a b.txt")
        );
    }

    /// GH #444's two reported shapes, at the decision the command actually
    /// makes. Before the fix this gate answered `unsupported url scheme` for
    /// both — the link was rendered, clickable, and inert.
    #[test]
    fn a_file_link_is_routed_to_the_os_default_application() {
        for url in ["file://D:\\test.txt", "file:///D:\\test.txt"] {
            match external_open_plan(url, None) {
                Ok(ExternalOpen::LocalPath(path)) => {
                    assert!(path.to_string_lossy().ends_with("test.txt"), "{url}");
                }
                other => panic!("{url} should open a local path, got {other:?}"),
            }
        }
        // A directory is the same route; OG hands it to the file manager too.
        assert!(matches!(
            external_open_plan("file:///home/user/notes/", None),
            Ok(ExternalOpen::LocalPath(_))
        ));
    }

    #[test]
    fn web_and_mail_links_keep_their_own_route_and_everything_else_stays_refused() {
        assert_eq!(
            external_open_plan("https://example.com/a", None),
            Ok(ExternalOpen::Url)
        );
        assert_eq!(
            external_open_plan("http://example.com/a", None),
            Ok(ExternalOpen::Url)
        );
        assert_eq!(
            external_open_plan("mailto:a@example.com", None),
            Ok(ExternalOpen::Url)
        );
        // Stricter than OG, which hands every unknown scheme to the OS: without
        // the user's confirmation nothing else opens.
        for url in [
            "javascript:alert(1)",
            "smb://host/share",
            "data:text/html,x",
            "zotero://select/items/ABC",
        ] {
            assert!(
                external_open_plan(url, None).is_err(),
                "{url} must stay refused"
            );
        }
    }

    /// GH #503: another app's link opens once the user confirmed its scheme;
    /// the confirmation is for that scheme only, and code-carrying schemes are
    /// never opened, confirmed or not (I-22).
    #[test]
    fn a_confirmed_custom_scheme_opens_and_nothing_else_does() {
        for (url, scheme) in [
            ("shortcuts://run-shortcut?name=abc", "shortcuts"),
            ("zotero://select/items/ABC", "zotero"),
            ("obsidian://open?vault=v&file=f", "obsidian"),
            ("Zotero://select/items/ABC", "ZOTERO"),
            ("tel:+420123456", "tel"),
        ] {
            assert_eq!(
                external_open_plan(url, Some(scheme)),
                Ok(ExternalOpen::CustomScheme),
                "{url}"
            );
        }
        // A confirmation of one scheme does not open another.
        assert!(external_open_plan("zotero://x", Some("obsidian")).is_err());
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "vbscript:x",
            "data:text/html,x",
            "blob:https://x/y",
            "about:blank",
            "tine://today",
        ] {
            let scheme = url_scheme(url).expect("a scheme");
            assert!(
                external_open_plan(url, Some(&scheme)).is_err(),
                "{url} must never open"
            );
        }
        // A Windows drive path or a bare word is not a scheme.
        assert_eq!(url_scheme("C:\\notes\\a.txt"), None);
        assert_eq!(url_scheme("notes/a.txt"), None);
        assert_eq!(url_scheme("-x:y"), None);
        // http(s), mailto and file keep their routes whatever is confirmed.
        assert_eq!(
            external_open_plan("https://example.com", Some("zotero")),
            Ok(ExternalOpen::Url)
        );
    }

    #[test]
    fn refuses_what_is_not_a_local_file() {
        assert_eq!(file_url_to_path("https://example.com/a.txt"), None);
        assert_eq!(file_url_to_path("not a url"), None);
        // A remote authority is not a local path on this platform.
        #[cfg(not(target_os = "windows"))]
        assert_eq!(file_url_to_path("file://example.com/share/a.txt"), None);
    }
}

#[cfg(test)]
mod clipboard_tool_tests {
    // Script tools need a POSIX shell; an inner cfg keeps the module visible to
    // the production-source scan (og-enforcement) as test code.
    #![cfg(target_os = "linux")]
    use super::*;
    use std::time::{Duration, Instant};

    /// A child process writes the script, so this test process never holds a
    /// write descriptor that a parallel test's fork could inherit; executing a
    /// file another process still has open for writing fails with ETXTBSY.
    fn script(dir: &std::path::Path, name: &str, body: &str) -> (String, Vec<String>) {
        let path = dir.join(name);
        let written = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"",
                "sh",
            ])
            .arg(format!("#!/bin/sh\n{body}\n"))
            .arg(&path)
            .status()
            .unwrap();
        assert!(
            written.success(),
            "could not write test script {}",
            path.display()
        );
        (path.to_string_lossy().into_owned(), Vec::new())
    }
    fn pid_of(dir: &std::path::Path, file: &str) -> u32 {
        std::fs::read_to_string(dir.join(file))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
    /// The process still has a /proc entry: running, or a zombie nobody waited for.
    fn lingers(pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    const QUICK: Duration = Duration::from_millis(400);

    #[test]
    fn a_tool_that_exits_with_failure_is_not_a_copy_and_the_next_tool_is_tried() {
        let dir = tempfile::tempdir().unwrap();
        let tools = vec![
            script(dir.path(), "bad", "cat >/dev/null; exit 3"),
            script(dir.path(), "good", "cat >/dev/null; exit 0"),
        ];
        assert!(copy_with_tools(&tools, b"png", QUICK).is_ok());
    }

    #[test]
    fn every_tool_failing_reports_the_failure_instead_of_success() {
        let dir = tempfile::tempdir().unwrap();
        let tools = vec![
            script(dir.path(), "bad1", "cat >/dev/null; exit 3"),
            script(dir.path(), "bad2", "cat >/dev/null; exit 4"),
        ];
        let error = copy_with_tools(&tools, b"png", QUICK).unwrap_err();
        assert!(error.contains("bad2") && error.contains("exit"), "{error}");
    }

    #[test]
    fn a_missing_tool_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let tools = vec![
            ("/nonexistent/tine-clip-tool".to_string(), Vec::new()),
            script(dir.path(), "good", "cat >/dev/null"),
        ];
        assert!(copy_with_tools(&tools, b"png", QUICK).is_ok());
    }

    #[test]
    fn a_tool_that_stops_reading_is_killed_and_reaped_before_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let tools = vec![
            // Records its pid and exits without reading: the 4 MiB write hits EPIPE.
            script(
                dir.path(),
                "deaf",
                &format!("echo $$ > {}", pidfile.display()),
            ),
            script(dir.path(), "good", "cat >/dev/null"),
        ];
        assert!(copy_with_tools(&tools, &vec![0u8; 4 << 20], QUICK).is_ok());
        let pid = pid_of(dir.path(), "pid");
        assert!(
            !lingers(pid),
            "the abandoned tool {pid} was left as a zombie"
        );
    }

    #[test]
    fn a_tool_still_serving_in_the_foreground_counts_as_success_and_is_reaped_after_it_exits() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let tools = vec![script(
            dir.path(),
            "server",
            &format!(
                "echo $$ > {}; cat >/dev/null; exec sleep 30",
                pidfile.display()
            ),
        )];
        let started = Instant::now();
        assert!(copy_with_tools(&tools, b"png", Duration::from_millis(200)).is_ok());
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid = pid_of(dir.path(), "pid");
        assert!(lingers(pid), "the serving tool must keep running");
        // The selection owner is replaced: it exits, and the reaper collects it.
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while lingers(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !lingers(pid),
            "an exited serving tool {pid} was never reaped"
        );
    }

    #[test]
    fn spawn_reaped_collects_a_finished_opener() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let (prog, _) = script(
            dir.path(),
            "opener",
            &format!("echo $$ > {}", pidfile.display()),
        );
        spawn_reaped(&mut std::process::Command::new(prog)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pidfile.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = pid_of(dir.path(), "pid");
        let deadline = Instant::now() + Duration::from_secs(5);
        while lingers(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!lingers(pid), "opener {pid} stayed a zombie");
        assert!(spawn_reaped(&mut std::process::Command::new("/nonexistent/tine-opener")).is_err());
    }
}

#[cfg(test)]
mod reveal_platform_arms {
    /// The body of `fn name` in `source`, from its signature to the next
    /// top-level item. Test-only source slicing; O(source bytes).
    fn body<'a>(source: &'a str, name: &str) -> &'a str {
        let start = source
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("fn {name} not found"));
        let rest = &source[start..];
        let end = rest.find("\n}\n").map_or(rest.len(), |end| end + 3);
        &rest[..end]
    }

    /// I-16 (GH #447): "show the graph folder" names all five shipped targets.
    /// tauri-build defines `desktop` as "target_os is neither ios nor android"
    /// (tauri-build 2.6 `cfg_alias("desktop", !mobile)`), so Linux, Windows and
    /// macOS each need their own reveal arm, and iOS/Android must reach the
    /// `not(desktop)` refusal that the frontend's `canRevealInFileManager`
    /// hides. Dropping any arm, or the refusal, fails here.
    #[test]
    fn reveal_names_linux_windows_macos_and_refuses_ios_android() {
        let platform = include_str!("platform.rs");
        let reveal = body(platform, "reveal_page_source");
        for os in ["linux", "windows", "macos"] {
            assert_eq!(
                reveal
                    .matches(&format!("#[cfg(target_os = \"{os}\")]"))
                    .count(),
                1,
                "I-16: reveal_page_source needs exactly one {os} arm; exemplar platform.rs"
            );
        }
        let settings = include_str!("settings.rs");
        let command = body(settings, "reveal_known_graph");
        assert!(command.contains("#[cfg(desktop)]"));
        assert!(
            command.contains("#[cfg(not(desktop))]"),
            "I-16: iOS/Android must get reveal_known_graph's explicit refusal"
        );
    }
}

#[cfg(test)]
mod graph_relative_target_tests {
    use super::graph_relative_target;

    /// GH #457: a resolved file link opens the file inside the graph, and a
    /// path that leaves the graph folder, lexically or through a symlink, is
    /// refused (a note's link is untrusted content).
    #[test]
    fn opens_inside_the_graph_and_refuses_what_leaves_it() {
        let base = std::env::temp_dir().join(format!("tine-457-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let graph = base.join("graph");
        std::fs::create_dir_all(graph.join("files")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(graph.join("files/a b.txt"), "x").unwrap();
        std::fs::write(base.join("outside/secret.txt"), "x").unwrap();

        let opened = graph_relative_target(&graph, "files/a b.txt").expect("inside");
        assert_eq!(opened, graph.join("files/a b.txt").canonicalize().unwrap());
        assert_eq!(
            graph_relative_target(&graph, "pages/../files/a b.txt").unwrap(),
            opened
        );
        assert_eq!(
            graph_relative_target(&graph, "").unwrap(),
            graph.canonicalize().unwrap()
        );
        for escaping in [
            "../outside/secret.txt",
            "files/../../outside/secret.txt",
            "/etc/passwd",
            "C:/Windows/win.ini",
        ] {
            assert!(
                graph_relative_target(&graph, escaping)
                    .unwrap_err()
                    .contains("outside the graph folder"),
                "{escaping} must be refused"
            );
        }
        assert!(graph_relative_target(&graph, "files/missing.txt")
            .unwrap_err()
            .contains("could not be found"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("outside"), graph.join("link")).unwrap();
            assert!(graph_relative_target(&graph, "link/secret.txt")
                .unwrap_err()
                .contains("outside the graph folder"));
        }
        std::fs::remove_dir_all(&base).unwrap();
    }
}
