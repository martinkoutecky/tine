// Android media capture bridge: camera / photo-picker (`capture_photo`) and
// voice-memo recording (`start_recording` / `stop_recording` / `cancel_recording`).
// Each returns the captured bytes as base64 in `data` + a file `ext`; the
// frontend writes them into the graph's assets/ and inserts the media ref.
// Mirrors android_folder_picker.rs. Non-android targets get erroring stubs so the
// desktop build links and the JS calls fail gracefully.
use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use serde::de::DeserializeOwned;
#[cfg(target_os = "android")]
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "page.tine.app";

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct MediaCaptureResult {
    /// "ok" (data+ext set), "cancelled", or "recording" (start ack).
    status: String,
    /// Base64-encoded file bytes (present when status == "ok").
    data: Option<String>,
    /// File extension without the dot, e.g. "jpg" / "png" / "m4a".
    ext: Option<String>,
}

#[cfg(target_os = "android")]
pub(crate) struct AndroidMedia<R: Runtime>(PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> AndroidMedia<R> {
    fn call(&self, method: &str) -> Result<MediaCaptureResult, String> {
        self.0
            .run_mobile_plugin(method, ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "android")]
macro_rules! android_media_command {
    ($name:ident, $method:literal) => {
        #[tauri::command]
        pub(crate) async fn $name<R: Runtime>(
            _app: AppHandle<R>,
            media: State<'_, AndroidMedia<R>>,
        ) -> Result<MediaCaptureResult, String> {
            media.call($method)
        }
    };
}

#[cfg(not(target_os = "android"))]
macro_rules! android_media_command {
    ($name:ident, $method:literal) => {
        #[tauri::command]
        pub(crate) async fn $name() -> Result<MediaCaptureResult, String> {
            Err("Media capture is only supported on Android".to_string())
        }
    };
}

android_media_command!(capture_photo, "capturePhoto");
android_media_command!(start_recording, "startRecording");
android_media_command!(stop_recording, "stopRecording");
android_media_command!(cancel_recording, "cancelRecording");

#[cfg(target_os = "android")]
fn init_android<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<AndroidMedia<R>, Box<dyn std::error::Error>> {
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "MediaCapturePlugin")?;
    Ok(AndroidMedia(handle))
}

#[cfg(target_os = "android")]
pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("android-media")
        .setup(|app, api| {
            let media = init_android(app, api)?;
            app.manage(media);
            Ok(())
        })
        .build()
}
