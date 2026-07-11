use crate::state::{slot_for_context, GraphContext};
use std::path::PathBuf;
use tauri::Manager;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) struct KnownGraph {
    pub(crate) path: String,
    pub(crate) name: String,
}

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
    mutate: impl Fn(&mut serde_json::Value),
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

/// Stable installation identity for managed sync. It lives in device-local app
/// settings, never in the provider-shared graph: two installations writing the
/// same device stream would violate the one-writer invariant. Loro peer ids are
/// session-scoped separately; this UUID names the installation directory only.
pub(crate) fn managed_sync_device_id(app: &tauri::AppHandle) -> Result<uuid::Uuid, String> {
    let chosen = std::sync::Mutex::new(None);
    update_settings(app, |json| {
        let id = json
            .get("managed_sync_device_id")
            .and_then(|value| value.as_str())
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .unwrap_or_else(uuid::Uuid::new_v4);
        json["managed_sync_device_id"] = serde_json::Value::String(id.to_string());
        *chosen.lock().unwrap() = Some(id);
    })?;
    chosen
        .into_inner()
        .map_err(|_| "managed sync device-id lock poisoned".to_string())?
        .ok_or_else(|| "managed sync device id was not written".to_string())
}

fn graph_display_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn parse_known_graphs(json: &serde_json::Value) -> Vec<KnownGraph> {
    json.get("known_graphs")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn remember_graph_json(json: &mut serde_json::Value, path: &str) {
    let mut graphs = parse_known_graphs(json);
    graphs.retain(|graph| graph.path != path);
    graphs.insert(
        0,
        KnownGraph {
            path: path.to_string(),
            name: graph_display_name(path),
        },
    );
    json["known_graphs"] = serde_json::to_value(graphs).unwrap_or_default();
    json["last_graph_path"] = serde_json::Value::String(path.to_string());
}

fn forget_graph_json(json: &mut serde_json::Value, path: &str) {
    let mut graphs = parse_known_graphs(json);
    graphs.retain(|graph| graph.path != path);
    json["known_graphs"] = serde_json::to_value(graphs).unwrap_or_default();
}

pub(crate) fn remember_graph(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    update_settings(app, |json| remember_graph_json(json, path))
}

#[tauri::command]
pub(crate) fn list_known_graphs(app: tauri::AppHandle) -> Vec<KnownGraph> {
    settings_path(&app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|json| parse_known_graphs(&json))
        .unwrap_or_default()
}

#[tauri::command]
pub(crate) fn forget_known_graph(path: String, app: tauri::AppHandle) -> Result<(), String> {
    update_settings(&app, |json| forget_graph_json(json, &path))
}

pub(crate) fn last_graph_path(app: &tauri::AppHandle) -> Option<String> {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|json| {
            json.get("last_graph_path")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
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
        json[&key] = serde_json::Value::String(value.clone());
    })
}

/// Path to the persisted UI session (open tabs / active tab / zoom). This is
/// app-level window state, not graph content, so it lives next to the settings
/// file in the app-data dir. (WebKitGTK's localStorage is not durably persisted
/// for this app, so the frontend can't rely on it — it round-trips here.)
fn legacy_session_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("tine-session.json"))
}

fn session_id(root: &std::path::Path) -> String {
    // Stable FNV-1a over the canonical path. The readable basename is cosmetic;
    // the hash prevents two same-named graphs in different folders colliding.
    let text = root.to_string_lossy();
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    format!("{name}-{hash:016x}.json")
}

fn session_path(app: &tauri::AppHandle, root: &std::path::Path) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("sessions").join(session_id(root)))
}

static SESSION_MIGRATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tauri::command]
pub(crate) fn load_session(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<Option<String>, String> {
    let slot = slot_for_context(&state)?;
    let path = session_path(&app, &slot.root_key).ok_or("no app-data dir")?;
    if !path.exists() {
        let _migration = SESSION_MIGRATION_LOCK.lock().unwrap();
        if !path.exists() {
            if let Some(legacy) = legacy_session_path(&app).filter(|p| p.exists()) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(legacy, &path).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(std::fs::read_to_string(path).ok())
}

#[tauri::command]
pub(crate) fn save_session(
    data: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Unique temp name per write so two concurrent saves (a burst of tab actions)
    // can't clobber each other's temp file before the rename.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let slot = slot_for_context(&state)?;
    let p = session_path(&app, &slot.root_key).ok_or("no app-data dir")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_graphs_are_deduplicated_mru_and_removable() {
        let mut json = serde_json::json!({});
        remember_graph_json(&mut json, "/graphs/alpha");
        remember_graph_json(&mut json, "/other/beta");
        remember_graph_json(&mut json, "/graphs/alpha");
        assert_eq!(
            parse_known_graphs(&json),
            vec![
                KnownGraph {
                    path: "/graphs/alpha".into(),
                    name: "alpha".into()
                },
                KnownGraph {
                    path: "/other/beta".into(),
                    name: "beta".into()
                },
            ]
        );
        assert_eq!(json["last_graph_path"], "/graphs/alpha");
        forget_graph_json(&mut json, "/graphs/alpha");
        assert_eq!(parse_known_graphs(&json).len(), 1);
        assert_eq!(json["last_graph_path"], "/graphs/alpha");
    }

    #[test]
    fn session_ids_separate_same_named_graphs() {
        assert_ne!(
            session_id(std::path::Path::new("/one/graph")),
            session_id(std::path::Path::new("/two/graph"))
        );
    }
}
