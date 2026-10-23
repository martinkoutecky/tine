//! Device-only reads run wholly on the blocking pool, including settings,
//! canonicalization and bound-graph checks. Graph bytes still use tine-store.
use crate::state::AppState;
use tauri::Manager;

pub(super) fn read_local_image(
    path: &str,
    app: &tauri::AppHandle,
) -> Result<tauri::ipc::Response, String> {
    if !crate::settings::device_bool(app, "allow_local_file_images", false) {
        return Err("local-file images are disabled".into());
    }
    let p = std::path::Path::new(path);
    let ext_ok = matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "avif" | "apng")
    );
    if !ext_ok {
        return Err("not an image file".into());
    }
    let p = super::refuse_bound_graph_path(p, &app.state::<AppState>())?;
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    crate::device_io::read_regular_file_bounded(&p, MAX_BYTES).map(tauri::ipc::Response::new)
}
