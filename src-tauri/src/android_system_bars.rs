#[cfg(target_os = "android")]
use serde::de::DeserializeOwned;
#[cfg(target_os = "android")]
use serde::Serialize;
#[cfg(target_os = "android")]
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "page.tine.app";

#[cfg(target_os = "android")]
#[derive(Debug, Serialize)]
struct SystemBarAppearance {
    dark: bool,
}

#[cfg(target_os = "android")]
pub(crate) struct AndroidSystemBars<R: Runtime>(PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> AndroidSystemBars<R> {
    fn set_appearance(&self, dark: bool) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<()>("setAppearance", SystemBarAppearance { dark })
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) async fn set_system_bar_appearance<R: Runtime>(
    _app: AppHandle<R>,
    bars: State<'_, AndroidSystemBars<R>>,
    dark: bool,
) -> Result<(), String> {
    bars.set_appearance(dark)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub(crate) async fn set_system_bar_appearance(dark: bool) -> Result<(), String> {
    let _ = dark;
    Ok(())
}

#[cfg(target_os = "android")]
fn init_android<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<AndroidSystemBars<R>, Box<dyn std::error::Error>> {
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "SystemBarsPlugin")?;
    Ok(AndroidSystemBars(handle))
}

#[cfg(target_os = "android")]
pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("android-system-bars")
        .setup(|app, api| {
            let bars = init_android(app, api)?;
            app.manage(bars);
            Ok(())
        })
        .build()
}
