// Android media capture bridge: camera / photo-picker (`capture_photo`) and
// voice-memo recording (`start_recording` / `stop_recording` / `cancel_recording`).
// Photos return captured bytes as base64 in `data` + a file `ext`. Voice memos
// return a native-cache `path`; Rust streams that token into the graph before
// the frontend inserts the media ref.
// Mirrors android_folder_picker.rs. Non-android targets get erroring stubs so the
// desktop build links and the JS calls fail gracefully.
#[cfg(target_os = "android")]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use tauri::{
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "page.tine.app";

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct MediaCaptureResult {
    /// "ok" (path + ext set), "cancelled", or "recording" (start ack).
    status: String,
    /// Native app-cache token (present for successful photo or voice capture).
    path: Option<String>,
    /// File extension without the dot, e.g. "jpg" / "png" / "m4a".
    ext: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::MediaCaptureResult;

    #[test]
    fn native_capture_path_survives_mobile_plugin_deserialization() {
        let result: MediaCaptureResult = serde_json::from_str(
            r#"{"status":"ok","path":"/data/user/0/page.tine.app/cache/voice.m4a","ext":"m4a"}"#,
        )
        .expect("voice memo result should deserialize");

        assert_eq!(
            result.path.as_deref(),
            Some("/data/user/0/page.tine.app/cache/voice.m4a")
        );
        assert_eq!(result.ext.as_deref(), Some("m4a"));
    }
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
