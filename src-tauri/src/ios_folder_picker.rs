use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

tauri::ios_plugin_binding!(init_plugin_tine_ios_folder_picker);

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GraphFolderPickResult {
    status: String,
    path: Option<String>,
}

pub(crate) struct IosFolderPicker<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> IosFolderPicker<R> {
    fn pick_graph_folder(&self) -> Result<GraphFolderPickResult, String> {
        self.0
            .run_mobile_plugin("pickGraphFolder", ())
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub(crate) async fn pick_graph_folder<R: Runtime>(
    _app: AppHandle<R>,
    picker: State<'_, IosFolderPicker<R>>,
) -> Result<GraphFolderPickResult, String> {
    picker.pick_graph_folder()
}

fn init_ios<R: Runtime, C: serde::de::DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<IosFolderPicker<R>, Box<dyn std::error::Error>> {
    let handle = api.register_ios_plugin(init_plugin_tine_ios_folder_picker)?;
    Ok(IosFolderPicker(handle))
}

pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("ios-folder-picker")
        .setup(|app, api| {
            let picker = init_ios(app, api)?;
            app.manage(picker);
            Ok(())
        })
        .build()
}
