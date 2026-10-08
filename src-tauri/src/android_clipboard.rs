//! Android image clipboard (GH #654).
//!
//! `tauri-plugin-clipboard-manager` 2.x implements `write_image` as
//! "Unsupported on this platform" on mobile, so the image copy button failed on
//! Android no matter what the page sent. Android's own mechanism for an image on
//! the clipboard is a `content://` URI in a `ClipData`: `ClipboardImagePlugin.kt`
//! stages the PNG as a file in the app cache and publishes it through the app's
//! existing `FileProvider`. The pasting app (this WebView included) reads the
//! image through that URI. This module is only the hand-off; the file is written
//! on the Kotlin side so no persisted-format writer site is added here.
use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

const PLUGIN_IDENTIFIER: &str = "page.tine.app";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CopyImage<'a> {
    bytes_b64: &'a str,
}

pub(crate) struct AndroidClipboard<R: Runtime>(PluginHandle<R>);

/// Put the base64 PNG on the system clipboard through the Kotlin plugin. Blocks
/// on the plugin: call from the blocking pool.
pub(crate) fn copy_png<R: Runtime>(app: &AppHandle<R>, bytes_b64: &str) -> Result<(), String> {
    app.state::<AndroidClipboard<R>>()
        .0
        .run_mobile_plugin::<()>("copyImage", CopyImage { bytes_b64 })
        .map_err(|e| e.to_string())
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
