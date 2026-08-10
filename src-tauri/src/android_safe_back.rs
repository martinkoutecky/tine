//! Android's permanent Back owner. Tauri's stock AppPlugin reasonably falls
//! back to WebView history or activity finish when JavaScript has no listener;
//! that fallback is unsafe while a managed-storage shutdown is unverified.

#[cfg(target_os = "android")]
use serde::de::DeserializeOwned;
#[cfg(target_os = "android")]
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Runtime,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "page.tine.app";

/// Retain the registered mobile handle for the life of the Tauri plugin. The
/// Kotlin side owns both listener accounting and dispatch; Rust only registers
/// that narrow native owner and intentionally exposes no guest capability.
#[cfg(target_os = "android")]
pub(crate) struct AndroidSafeBack<R: Runtime>(PluginHandle<R>);

#[cfg(target_os = "android")]
fn init_android<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<AndroidSafeBack<R>, Box<dyn std::error::Error>> {
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "SafeBackPlugin")?;
    Ok(AndroidSafeBack(handle))
}

#[cfg(target_os = "android")]
pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("safe-back")
        .setup(|app, api| {
            app.manage(init_android(app, api)?);
            Ok(())
        })
        .build()
}
