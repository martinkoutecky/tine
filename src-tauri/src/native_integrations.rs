//! The mobile host side of Tine's native integrations (share inbox, Spotlight,
//! quick actions, App Intents, launcher shortcuts; ADR 0073). These are HOST
//! features: nothing here is a plugin capability (`PLUGIN_CAPABILITIES`).
//!
//! - `inboxDirectory`: where the native producers publish share inbox items
//!   (iOS: the App Group container; Android: `filesDir/share-inbox`).
//!   share_inbox.rs reads and commits them; native code never touches the
//!   graph.
//! - `inboxChanged` (native → frontend event, `addPluginListener`): an item
//!   arrived while the app runs.
//! - `spotlight` (iOS): apply one spotlight.rs `Update` to Core Spotlight.
//!
//! Routes (quick actions, shortcuts, intents, Spotlight taps) need no command
//! here: the native side hands them to the existing `tine://` URL handler, so
//! they arrive through deep_links.rs `receive_url` like any other Tine link.
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

// LOAD-BEARING although it names nothing: keeps the Swift package's rlib (and
// its static library) in the link. See ios_folder_picker.rs for the full story.
#[cfg(target_os = "ios")]
extern crate tine_ios_native_integrations;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_tine_native_integrations);

/// The JavaScript plugin name; `src/nativeTineLinks.ts` listens on it.
pub(crate) const PLUGIN_NAME: &str = "native-integrations";

pub(crate) struct NativeIntegrations<R: Runtime> {
    handle: PluginHandle<R>,
    inbox: OnceLock<PathBuf>,
}

#[derive(Deserialize)]
struct InboxDirectory {
    path: String,
}

fn plugin(app: &AppHandle) -> Result<tauri::State<'_, NativeIntegrations<tauri::Wry>>, String> {
    app.try_state::<NativeIntegrations<tauri::Wry>>()
        .ok_or_else(|| "native integrations are not registered".to_string())
}

/// The share inbox root, asked of the native side once. Blocks on the
/// plugin: call from the blocking pool.
pub(crate) fn inbox_root(app: &AppHandle) -> Result<PathBuf, String> {
    let state = plugin(app)?;
    if let Some(path) = state.inbox.get() {
        return Ok(path.clone());
    }
    let directory: InboxDirectory = state
        .handle
        .run_mobile_plugin("inboxDirectory", ())
        .map_err(|error| error.to_string())?;
    if directory.path.is_empty() {
        return Err("the share inbox is unavailable".into());
    }
    Ok(state
        .inbox
        .get_or_init(|| PathBuf::from(directory.path))
        .clone())
}

/// Apply one index update to Core Spotlight. Blocks on the plugin.
#[cfg(target_os = "ios")]
pub(crate) fn spotlight(app: &AppHandle, update: &crate::spotlight::Update) -> Result<(), String> {
    plugin(app)?
        .handle
        .run_mobile_plugin::<()>("spotlight", update)
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "ios")]
fn register<R: Runtime, C: serde::de::DeserializeOwned>(
    api: PluginApi<R, C>,
) -> Result<PluginHandle<R>, Box<dyn std::error::Error>> {
    Ok(api.register_ios_plugin(init_plugin_tine_native_integrations)?)
}

#[cfg(target_os = "android")]
fn register<R: Runtime, C: serde::de::DeserializeOwned>(
    api: PluginApi<R, C>,
) -> Result<PluginHandle<R>, Box<dyn std::error::Error>> {
    Ok(api.register_android_plugin("page.tine.app", "NativeIntegrationsPlugin")?)
}

pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new(PLUGIN_NAME)
        .setup(|app, api| {
            app.manage(NativeIntegrations {
                handle: register(api)?,
                inbox: OnceLock::new(),
            });
            Ok(())
        })
        .build()
}
