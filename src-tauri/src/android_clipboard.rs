//! Android image clipboard (GH #654).
//!
//! `tauri-plugin-clipboard-manager` 2.x implements `write_image` as
//! "Unsupported on this platform" on mobile, so the image copy button failed on
//! Android no matter what the page sent. Android's own mechanism for an image on
//! the clipboard is a `content://` URI in a `ClipData`: the PNG is staged as a
//! file in the app cache here, and `ClipboardImagePlugin.kt` publishes it
//! through the app's existing `FileProvider`. The pasting app (this WebView
//! included) reads the image through that URI.
//!
//! The staging half is plain filesystem work and is tested on every platform;
//! only the plugin hand-off is Android-only.
use std::path::{Path, PathBuf};

/// Prefix of the staged files. `ClipboardImagePlugin.kt` accepts only a file of
/// this shape directly inside the app cache.
pub(crate) const STAGED_PREFIX: &str = "tine_clip_";
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Write `png` into `cache_dir` as a fresh `tine_clip_*.png` and retire the files
/// earlier copies staged: the new clip replaces the old one, so at most one
/// staged image is ever live and repeated copies cannot grow the cache.
pub(crate) fn stage_clipboard_png(cache_dir: &Path, png: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if !png.starts_with(&PNG_SIGNATURE) {
        return Err("the clipboard image is not a PNG".into());
    }
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(STAGED_PREFIX) && name.ends_with(".png") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = cache_dir.join(format!(
        "{STAGED_PREFIX}{stamp}_{}.png",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(png)
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            let _ = std::fs::remove_file(&path);
            e.to_string()
        })?;
    Ok(path)
}

#[cfg(target_os = "android")]
mod bridge {
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use tauri::{
        plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
        AppHandle, Manager, Runtime,
    };

    const PLUGIN_IDENTIFIER: &str = "page.tine.app";

    #[derive(Serialize)]
    struct CopyImage {
        path: String,
    }

    pub(crate) struct AndroidClipboard<R: Runtime>(PluginHandle<R>);

    /// Stage `png` in the app cache and have the Kotlin plugin put it on the
    /// system clipboard. Blocks on the plugin: call from the blocking pool.
    pub(crate) fn copy_png<R: Runtime>(app: &AppHandle<R>, png: &[u8]) -> Result<(), String> {
        let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
        let staged = super::stage_clipboard_png(&cache, png)?;
        let clipboard = app.state::<AndroidClipboard<R>>();
        let result = clipboard
            .0
            .run_mobile_plugin::<()>(
                "copyImage",
                CopyImage {
                    path: staged.to_string_lossy().into_owned(),
                },
            )
            .map_err(|e| e.to_string());
        if result.is_err() {
            let _ = std::fs::remove_file(&staged);
        }
        result
    }

    fn init_android<R: Runtime, C: DeserializeOwned>(
        _app: &AppHandle<R>,
        api: PluginApi<R, C>,
    ) -> Result<AndroidClipboard<R>, Box<dyn std::error::Error>> {
        let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "ClipboardImagePlugin")?;
        Ok(AndroidClipboard(handle))
    }

    pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
        Builder::new("android-clipboard")
            .setup(|app, api| {
                let clipboard = init_android(app, api)?;
                app.manage(clipboard);
                Ok(())
            })
            .build()
    }
}

#[cfg(target_os = "android")]
pub(crate) use bridge::{copy_png, init};

#[cfg(test)]
mod tests {
    use super::*;

    fn png(extra: &[u8]) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(extra);
        bytes
    }

    #[test]
    fn staged_image_is_the_exact_png_in_a_clip_named_file() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = png(b"pixels");
        let path = stage_clipboard_png(dir.path(), &bytes).unwrap();
        assert_eq!(path.parent(), Some(dir.path()));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(STAGED_PREFIX) && name.ends_with(".png"), "{name}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_new_copy_retires_the_previous_staged_image_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let first = stage_clipboard_png(dir.path(), &png(b"one")).unwrap();
        let photo = dir.path().join("tine_photo_1.jpg");
        std::fs::write(&photo, b"jpeg").unwrap();
        let second = stage_clipboard_png(dir.path(), &png(b"two")).unwrap();
        assert_ne!(first, second);
        assert!(!first.exists(), "stale staged image survives");
        assert!(second.exists());
        assert!(photo.exists(), "a capture token owned by media capture was deleted");
    }

    #[test]
    fn bytes_that_are_not_a_png_are_refused_without_touching_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let kept = stage_clipboard_png(dir.path(), &png(b"keep")).unwrap();
        assert!(stage_clipboard_png(dir.path(), b"GIF89a").is_err());
        assert!(kept.exists(), "a refused copy must leave the live clip alone");
    }
}
