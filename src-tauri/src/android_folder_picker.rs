use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use serde::de::DeserializeOwned;
#[cfg(target_os = "android")]
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "dev.tine.app";

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GraphFolderPickResult {
    status: String,
    path: Option<String>,
}

#[cfg(target_os = "android")]
pub(crate) struct AndroidFolderPicker<R: Runtime>(PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> AndroidFolderPicker<R> {
    fn pick_graph_folder(&self) -> Result<GraphFolderPickResult, String> {
        self.0
            .run_mobile_plugin("pickGraphFolder", ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) async fn pick_graph_folder<R: Runtime>(
    _app: AppHandle<R>,
    picker: State<'_, AndroidFolderPicker<R>>,
) -> Result<GraphFolderPickResult, String> {
    picker.pick_graph_folder()
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub(crate) async fn pick_graph_folder() -> Result<GraphFolderPickResult, String> {
    Err("Android folder picker is unsupported on this platform".to_string())
}

#[cfg(target_os = "android")]
fn init_android<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<AndroidFolderPicker<R>, Box<dyn std::error::Error>> {
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "GraphFolderPickerPlugin")?;
    Ok(AndroidFolderPicker(handle))
}

#[cfg(target_os = "android")]
pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("android-folder-picker")
        .setup(|app, api| {
            let picker = init_android(app, api)?;
            app.manage(picker);
            Ok(())
        })
        .build()
}
