use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

// LOAD-BEARING, and it looks like dead code. Do not remove it.
//
// `tine-ios-folder-picker-native` exports no Rust items at all — it exists only
// so its build script compiles the Swift package and emits
// `cargo:rustc-link-lib=static=tine-ios-folder-picker-native`. Those directives
// reach the final link only for rlibs rustc actually loads, and nothing else in
// this crate ever names it: `ios_plugin_binding!` below expands to
// `swift_rs::swift!(fn ...)`, a bare `extern "C"` declaration that references no
// crate (tauri-2.11.2 src/lib.rs:61-65). So without this line the dependency is
// pruned, the Swift archive is never bundled into `libtine_lib.a`, and Xcode
// fails with `Undefined symbols: "_init_plugin_tine_ios_folder_picker"`.
//
// `tauri-plugin-opener` uses the identical Swift binding pattern and does NOT
// need this only because `tauri_plugin_opener::init()` is called in `lib.rs`,
// which pulls its rlib in as a side effect.
extern crate tine_ios_folder_picker_native;

tauri::ios_plugin_binding!(init_plugin_tine_ios_folder_picker);

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GraphFolderPickResult {
    status: String,
    path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepareGraphFolderPayload<'a> {
    path: &'a str,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PrepareGraphFolderResult {
    status: String,
    location: Option<String>,
    // Swift may rebase a remembered graph after iOS moves the app container.
    // Keep the effective path in the bridge response; omitting this field makes
    // serde silently discard it and sends the frontend back to the stale UUID.
    path: Option<String>,
}

pub(crate) struct IosFolderPicker<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> IosFolderPicker<R> {
    fn pick_graph_folder(&self) -> Result<GraphFolderPickResult, String> {
        self.0
            .run_mobile_plugin("pickGraphFolder", ())
            .map_err(|e| e.to_string())
    }

    fn prepare_graph_folder(&self, path: &str) -> Result<PrepareGraphFolderResult, String> {
        self.0
            .run_mobile_plugin("prepareGraphFolder", PrepareGraphFolderPayload { path })
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

#[tauri::command]
pub(crate) async fn prepare_graph_folder<R: Runtime>(
    _app: AppHandle<R>,
    picker: State<'_, IosFolderPicker<R>>,
    path: String,
) -> Result<PrepareGraphFolderResult, String> {
    picker.prepare_graph_folder(&path)
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
