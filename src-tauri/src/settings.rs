use std::path::PathBuf;
use tauri::Manager;

// --- local app settings (outside the graph): currently just the backup keep
// count. A tiny JSON file in the OS app-data dir. ---
pub(crate) fn settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("tine-settings.json"))
}
/// Serializes ALL device-settings (tine-settings.json) writers; every `set_*` below
/// goes through `update_settings`, which routes to the shared `tine_core` atomic_update
/// (audit M1): the JSON is read-modify-written under this lock + atomically (temp +
/// fsync + rename), so a crash can't truncate it, a concurrent `set_*` can't clobber
/// another's key, and a transient read error aborts instead of resetting all prefs.
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Merge one or more keys into the device-settings JSON, durably. `mutate` edits the
/// parsed object (an unparseable existing file is treated as `{}`, the prior behavior).
pub(crate) fn update_settings(
    app: &tauri::AppHandle,
    mutate: impl FnOnce(&mut serde_json::Value),
) -> Result<(), String> {
    let p = settings_path(app).ok_or("no app-data dir")?;
    tine_core::model::atomic_update(&p, &SETTINGS_LOCK, |content| {
        let mut json: serde_json::Value =
            serde_json::from_str(content).unwrap_or_else(|_| serde_json::json!({}));
        mutate(&mut json);
        serde_json::to_string_pretty(&json)
            .map(|mut s| {
                s.push('\n');
                s
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    })
    .map_err(|e| e.to_string())
}

/// Quick-capture Enter behaviour (app-level, in tine-settings.json): true → a
/// plain Enter files the capture; false (default) → Enter makes a new block and
/// Cmd/Ctrl+Enter files.
fn capture_enter_files(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("capture_enter_files").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

#[tauri::command]
pub(crate) fn get_capture_enter_files(app: tauri::AppHandle) -> bool {
    capture_enter_files(&app)
}

#[tauri::command]
pub(crate) fn set_capture_enter_files(value: bool, app: tauri::AppHandle) -> Result<(), String> {
    update_settings(&app, |json| {
        json["capture_enter_files"] = serde_json::Value::Bool(value);
    })
}

/// `[[`/`#` autocomplete default action (app-level, in tine-settings.json):
/// true → Enter links the first match; false (default, OG) → Enter creates a new
/// page/tag unless an exact match exists. A workflow preference, device-local.
fn link_first_match(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("link_first_match").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

#[tauri::command]
pub(crate) fn get_link_first_match(app: tauri::AppHandle) -> bool {
    link_first_match(&app)
}

#[tauri::command]
pub(crate) fn set_link_first_match(value: bool, app: tauri::AppHandle) -> Result<(), String> {
    update_settings(&app, |json| {
        json["link_first_match"] = serde_json::Value::Bool(value);
    })
}

/// Smooth-scrolling preference (app-level, in tine-settings.json). Experimental,
/// default false. Read at startup by the frontend to (re-)install Lenis. Device-
/// local because it's a feel preference, not graph data.
fn smooth_scroll(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("smooth_scroll").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

#[tauri::command]
pub(crate) fn get_smooth_scroll(app: tauri::AppHandle) -> bool {
    smooth_scroll(&app)
}

#[tauri::command]
pub(crate) fn set_smooth_scroll(value: bool, app: tauri::AppHandle) -> Result<(), String> {
    update_settings(&app, |json| {
        json["smooth_scroll"] = serde_json::Value::Bool(value);
    })
}

/// Generic device-local boolean preference (tine-settings.json). For simple
/// behavior toggles that don't each warrant bespoke read/get/set code — the caller
/// supplies the key and the default. (Used by the copy-behavior options.)
#[tauri::command]
pub(crate) fn get_app_bool(key: String, default: bool, app: tauri::AppHandle) -> bool {
    settings_path(&app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(&key).and_then(|x| x.as_bool()))
        .unwrap_or(default)
}

#[tauri::command]
pub(crate) fn set_app_bool(key: String, value: bool, app: tauri::AppHandle) -> Result<(), String> {
    update_settings(&app, |json| {
        json[&key] = serde_json::Value::Bool(value);
    })
}

/// Generic device-local STRING preference (tine-settings.json) — the string twin of
/// `get_app_bool`. Used for the asset-filename format template (a personal naming
/// preference, read once at startup and applied in the frontend tokenizer).
#[tauri::command]
pub(crate) fn get_app_string(key: String, default: String, app: tauri::AppHandle) -> String {
    settings_path(&app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(&key).and_then(|x| x.as_str().map(str::to_string)))
        .unwrap_or(default)
}

#[tauri::command]
pub(crate) fn set_app_string(
    key: String,
    value: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    update_settings(&app, |json| {
        json[&key] = serde_json::Value::String(value);
    })
}

/// Path to the persisted UI session (open tabs / active tab / zoom). This is
/// app-level window state, not graph content, so it lives next to the settings
/// file in the app-data dir. (WebKitGTK's localStorage is not durably persisted
/// for this app, so the frontend can't rely on it — it round-trips here.)
fn session_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("tine-session.json"))
}

#[tauri::command]
pub(crate) fn load_session(app: tauri::AppHandle) -> Option<String> {
    std::fs::read_to_string(session_path(&app)?).ok()
}

#[tauri::command]
pub(crate) fn save_session(data: String, app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Unique temp name per write so two concurrent saves (a burst of tab actions)
    // can't clobber each other's temp file before the rename.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let p = session_path(&app).ok_or("no app-data dir")?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = p.with_extension(format!("json.tmp{seq}"));
    // Write to a temp file then atomically rename, so a crash mid-write can never
    // leave a truncated session that fails to parse.
    std::fs::write(&tmp, data.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(())
}
