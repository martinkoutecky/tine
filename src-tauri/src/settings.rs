use crate::device_io::read_app_text;
use crate::state::{slot_for_context, GraphContext};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;

pub(crate) const NATIVE_FRAME_KEY: &str = "native_window_frame";
static NATIVE_FRAME_ACTIVE: OnceLock<bool> = OnceLock::new();

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

fn app_bool_at(path: &std::path::Path, key: &str, default: bool) -> bool {
    read_app_text(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|value| value.get(key).and_then(serde_json::Value::as_bool))
        .unwrap_or(default)
}

/// Freeze the native-frame preference before Tauri constructs any windows.
/// Tao does not support changing Linux decorations on an existing window, so
/// every window created by this process must use the same startup value.
pub(crate) fn init_native_frame_active() -> bool {
    *NATIVE_FRAME_ACTIVE.get_or_init(|| {
        crate::app_identity::current_app_data_dir()
            .map(|dir| app_bool_at(&dir.join("tine-settings.json"), NATIVE_FRAME_KEY, false))
            .unwrap_or(false)
    })
}

pub(crate) fn native_frame_active() -> bool {
    init_native_frame_active()
}
/// Serializes ALL device-settings (tine-settings.json) writers; every `set_*` below
/// goes through `update_settings`, which routes to the device settings atomic_update
/// (audit M1): the JSON is read-modify-written under this lock + atomically (temp +
/// fsync + rename), so a crash can't truncate it, a concurrent `set_*` can't clobber
/// another's key, and a transient read error aborts instead of resetting all prefs.
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The one door for device-settings writes. A settings file that exists but does
/// not parse as a JSON object (an external edit, a sync tool's mangled copy) is
/// REFUSED, never rebuilt from `{}`: writing the default view back would erase
/// every known graph, plugin state, asset approval and cached registry (I-9), and
/// a non-object root would panic the `json[key] = ..` mutators. The file is left
/// byte-for-byte untouched and the error names it, so the user can repair or
/// remove it. A missing or blank file is a fresh start. Unknown keys (a newer
/// Tine) are carried through because `mutate` edits the parsed object in place.
/// In-scope scenario for the refusal: an external editor or a sync tool left
/// invalid bytes in this device-local file.
pub(crate) fn update_settings_strict_at(
    path: &std::path::Path,
    mutate: impl Fn(&mut serde_json::Value) -> Result<(), String>,
) -> Result<(), String> {
    crate::device_io::atomic_update(path, &SETTINGS_LOCK, |content| {
        let mut json: serde_json::Value = if content.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(content).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "device settings file {} is not valid JSON ({error}); \
                         it was left untouched, repair or remove it",
                        path.display()
                    ),
                )
            })?
        };
        if !json.is_object() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "device settings file {} does not hold a JSON object; \
                     it was left untouched, repair or remove it",
                    path.display()
                ),
            ));
        }
        mutate(&mut json)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        serde_json::to_string_pretty(&json)
            .map(|mut text| {
                text.push('\n');
                text
            })
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    })
    .map_err(|error| error.to_string())
}

/// Merge one or more keys into the device-settings JSON, durably (see
/// [`update_settings_strict_at`] for the refusal rule).
pub(crate) fn update_settings_at(
    path: &std::path::Path,
    mutate: impl Fn(&mut serde_json::Value),
) -> Result<(), String> {
    update_settings_strict_at(path, |json| {
        mutate(json);
        Ok(())
    })
}

pub(crate) fn update_settings(
    app: &tauri::AppHandle,
    mutate: impl Fn(&mut serde_json::Value),
) -> Result<(), String> {
    let p = settings_path(app).ok_or("no app-data dir")?;
    update_settings_at(&p, mutate)
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

fn external_assets_approvals(
    json: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    json.get("external_assets_approvals")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// The trust grant is deliberately device-local and keyed by BOTH canonical
/// graph root and canonical external target. It never enters graph content, and
/// a retargeted symlink/junction therefore cannot inherit the old grant.
pub(crate) fn approved_external_assets(
    app: &tauri::AppHandle,
    graph_root: &std::path::Path,
) -> Option<PathBuf> {
    let key = graph_root.display().to_string();
    settings_path(app)
        .and_then(|p| read_app_text(&p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|json| {
            external_assets_approvals(&json)
                .get(&key)
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
        })
}

pub(crate) fn remember_external_assets_approval(
    app: &tauri::AppHandle,
    graph_root: &std::path::Path,
    assets_root: &std::path::Path,
) -> Result<(), String> {
    let graph = graph_root.display().to_string();
    let assets = assets_root.display().to_string();
    update_settings(app, |json| {
        remember_external_assets_approval_json(json, &graph, &assets)
    })
}

fn remember_external_assets_approval_json(json: &mut serde_json::Value, graph: &str, assets: &str) {
    let mut approvals = external_assets_approvals(json);
    approvals.insert(
        graph.to_string(),
        serde_json::Value::String(assets.to_string()),
    );
    json["external_assets_approvals"] = serde_json::Value::Object(approvals);
}

pub(crate) fn remember_graph(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    update_settings(app, |json| remember_graph_json(json, path))
}

#[tauri::command]
pub(crate) fn list_known_graphs(app: tauri::AppHandle) -> Vec<KnownGraph> {
    settings_path(&app)
        .and_then(|p| read_app_text(&p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|json| parse_known_graphs(&json))
        .unwrap_or_default()
}

#[tauri::command]
pub(crate) fn forget_known_graph(path: String, app: tauri::AppHandle) -> Result<(), String> {
    update_settings(&app, |json| forget_graph_json(json, &path))?;
    // Best-effort, after the removal itself; never fails it (ADR 0070).
    crate::graph::forget_launch_checkpoint(
        crate::graph::checkpoint_app_data(&app).as_deref(),
        &path,
    );
    Ok(())
}

/// Reveal a remembered graph root in the desktop file manager. Only paths
/// already in the known-graph list are accepted; absent paths and mobile
/// platforms return an error. Cost: O(known graphs) plus one OS handoff.
/// The file-manager handoff waits for `dbus-send`'s reply, so it runs on the
/// blocking pool, off the UI thread (GH #623, I-21).
#[tauri::command]
pub(crate) async fn reveal_known_graph(path: String, app: tauri::AppHandle) -> Result<(), String> {
    if !list_known_graphs(app)
        .iter()
        .any(|known| known.path == path)
    {
        return Err("that graph is not in the known-graph list".into());
    }
    #[cfg(desktop)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            crate::platform::reveal_page_source(std::path::Path::new(&path))
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(desktop))]
    {
        Err("showing a graph folder is available on desktop only".into())
    }
}

pub(crate) fn last_graph_path(app: &tauri::AppHandle) -> Option<String> {
    settings_path(app)
        .and_then(|p| read_app_text(&p).ok())
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
        .and_then(|p| read_app_text(&p).ok())
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

/// Legacy `[[`/`#` autocomplete preference (app-level, in tine-settings.json):
/// true → Enter links the first match; false (default, OG) → Enter creates a new
/// page/tag unless an exact match exists. Read-only: it is the migration source
/// for the three-mode `linkAutocompletePolicy` string key, which is the only writer.
fn link_first_match(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .and_then(|p| read_app_text(&p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("link_first_match").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

#[tauri::command]
pub(crate) fn get_link_first_match(app: tauri::AppHandle) -> bool {
    link_first_match(&app)
}

/// Smooth-scrolling preference (app-level, in tine-settings.json). Experimental,
/// default false. Read at startup by the frontend to (re-)install Lenis. Device-
/// local because it's a feel preference, not graph data.
fn smooth_scroll(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .and_then(|p| read_app_text(&p).ok())
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

/// Device-local boolean preference, O(settings bytes). The query crossing key
/// is graph-scoped: it reads/writes the current bound graph's master-compatible
/// notices record instead of tine-settings.json. Other keys are device-wide.
#[tauri::command]
pub(crate) fn get_app_bool(
    key: String,
    default: bool,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> bool {
    if key != "queryCrossingNoticeDismissed" {
        return device_bool(&app, &key, default);
    }
    slot_for_context(&state)
        .ok()
        .zip(app.path().app_data_dir().ok())
        .is_some_and(|(slot, dir)| master_crossing_notice_dismissed(&dir, &slot.root_key))
}

pub(crate) fn device_bool(app: &tauri::AppHandle, key: &str, default: bool) -> bool {
    settings_path(app)
        .map(|path| app_bool_at(&path, key, default))
        .unwrap_or(default)
}

fn notices_id(root: &std::path::Path) -> String {
    let id = session_id(root);
    format!("{}-notices.json", id.strip_suffix(".json").unwrap_or(&id))
}

fn parse_notices(text: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .filter(|json| {
            json.is_object()
                && json.get("dismissed").is_none_or(|keys| {
                    keys.as_array()
                        .is_some_and(|keys| keys.iter().all(|key| key.is_string()))
                })
        })
        .unwrap_or_else(|| serde_json::json!({"dismissed": []}))
}

fn load_notices_at(path: &std::path::Path) -> serde_json::Value {
    match read_app_text(path) {
        Ok(text) => parse_notices(&text),
        Err(error) => {
            // Recovery over refusal for disposable notices, as on master.
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::debug::diag_private("notice-read-failed", error.to_string());
            }
            serde_json::json!({"dismissed": []})
        }
    }
}

fn master_crossing_notice_dismissed(dir: &std::path::Path, root: &std::path::Path) -> bool {
    notice_dismissed(dir, root, "query-crossing")
}

/// Whether the graph's notices record lists `notice` as dismissed.
pub(crate) fn notice_dismissed(
    dir: &std::path::Path,
    root: &std::path::Path,
    notice: &str,
) -> bool {
    load_notices_at(&dir.join("sessions").join(notices_id(root)))["dismissed"]
        .as_array()
        .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(notice)))
}

static NOTICES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One device-private, atomic read/modify/write; unknown notices/fields survive.
/// Like master, malformed notice JSON costs one extra notice and is repaired by
/// the next dismissal. I/O errors refuse the write and are reported to the user.
pub(crate) fn set_notice_at(
    dir: &std::path::Path,
    root: &std::path::Path,
    notice: &str,
    value: bool,
) -> Result<(), String> {
    let path = dir.join("sessions").join(notices_id(root));
    crate::device_io::atomic_update(&path, &NOTICES_LOCK, |text| {
        let mut json = parse_notices(text);
        let mut keys = json["dismissed"].as_array().cloned().unwrap_or_default();
        keys.retain(|key| key.as_str().is_some_and(|key| key != notice));
        if value {
            keys.push(serde_json::json!(notice));
        }
        json["dismissed"] = serde_json::json!(keys);
        serde_json::to_string(&json).map_err(std::io::Error::other)
    })
    .map_err(|error| error.to_string())
}

fn set_crossing_notice_at(
    dir: &std::path::Path,
    root: &std::path::Path,
    value: bool,
) -> Result<(), String> {
    set_notice_at(dir, root, "query-crossing", value)
}

#[tauri::command]
pub(crate) fn set_app_bool(
    key: String,
    value: bool,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    if key == "queryCrossingNoticeDismissed" {
        let slot = slot_for_context(&state)?;
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?;
        return set_crossing_notice_at(&dir, &slot.root_key, value);
    }
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
        .and_then(|p| read_app_text(&p).ok())
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
/// file in the app-data dir. The backend owns atomic structured persistence and
/// makes it independent of a particular WebView's localStorage namespace.
fn legacy_session_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("tine-session.json"))
}

pub(crate) fn session_id(root: &std::path::Path) -> String {
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

fn workspaces_path(app: &tauri::AppHandle, root: &std::path::Path) -> Option<PathBuf> {
    let id = session_id(root);
    let stem = id.strip_suffix(".json").unwrap_or(&id);
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("sessions").join(format!("{stem}-workspaces.json")))
}

fn blank_session_json() -> serde_json::Value {
    serde_json::json!({
        "tabs": [{
            "history": [{ "kind": "journals" }],
            "pos": 0,
            "pinned": false
        }],
        "activeIndex": 0
    })
}

fn migrated_workspaces_json(session: Option<&str>) -> String {
    let blob = session
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .unwrap_or_else(blank_session_json);
    serde_json::to_string(&serde_json::json!({
        "version": 1,
        "activeId": "default",
        "workspaces": [{ "id": "default", "name": "", "blob": blob }]
    }))
    .expect("workspace migration JSON is serializable")
}

fn validate_workspaces_json(data: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("workspace registry version must be 1".into());
    }
    let active = value
        .get("activeId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("workspace registry requires an activeId")?;
    let entries = value
        .get("workspaces")
        .and_then(serde_json::Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or("workspace registry requires at least one workspace")?;
    if !entries.iter().any(|entry| {
        entry.get("id").and_then(serde_json::Value::as_str) == Some(active)
            && entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some()
            && entry.get("blob").is_some_and(serde_json::Value::is_object)
    }) {
        return Err("active workspace is missing or invalid".into());
    }
    Ok(())
}

static WORKSPACES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Publication is visible after rename even when the following directory sync fails.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WorkspaceSaveOutcome {
    Durable,
    PublishedUnsynced,
}

fn atomic_write_json_with_sync(
    path: &std::path::Path,
    data: &str,
    sync: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> Result<WorkspaceSaveOutcome, String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("JSON file has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.json");
    let tmp = parent.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(data.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())?;
    Ok(if sync(parent).is_ok() {
        WorkspaceSaveOutcome::Durable
    } else {
        WorkspaceSaveOutcome::PublishedUnsynced
    })
}

fn atomic_write_workspaces(
    path: &std::path::Path,
    data: &str,
) -> Result<WorkspaceSaveOutcome, String> {
    atomic_write_json_with_sync(
        path,
        data,
        tine_store::directory_durability::sync_directory_entry,
    )
}

fn load_workspaces_at(path: &std::path::Path, session: &std::path::Path) -> Result<String, String> {
    let _guard = WORKSPACES_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match read_app_text(path) {
        Ok(data) => {
            validate_workspaces_json(&data)?;
            Ok(data)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let legacy = read_optional_session(session).map_err(|error| error.to_string())?;
            let data = migrated_workspaces_json(legacy.as_deref());
            atomic_write_workspaces(path, &data)?;
            Ok(data)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn save_workspaces_at(path: &std::path::Path, data: &str) -> Result<WorkspaceSaveOutcome, String> {
    validate_workspaces_json(data)?;
    let _guard = WORKSPACES_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    atomic_write_workspaces(path, data)
}

pub(crate) fn load_workspaces(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    let session = session_path(&app, &slot.root_key).ok_or("no app-data dir")?;
    let path = workspaces_path(&app, &slot.root_key).ok_or("no app-data dir")?;
    load_workspaces_at(&path, &session)
}

pub(crate) fn save_workspaces(
    data: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<WorkspaceSaveOutcome, String> {
    let slot = slot_for_context(&state)?;
    let path = workspaces_path(&app, &slot.root_key).ok_or("no app-data dir")?;
    save_workspaces_at(&path, &data)
}

// A missing session is fresh state; a disk/permission failure must not publish
// a blank workspace over state that still exists. Shared by restore and migration.
fn read_optional_session(path: &std::path::Path) -> std::io::Result<Option<String>> {
    match read_app_text(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn session_present(path: &std::path::Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

static SESSION_MIGRATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tauri::command]
pub(crate) fn load_session(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<Option<String>, String> {
    let slot = slot_for_context(&state)?;
    let path = session_path(&app, &slot.root_key).ok_or("no app-data dir")?;
    if !session_present(&path)? {
        let _migration = SESSION_MIGRATION_LOCK.lock().unwrap();
        if !session_present(&path)? {
            if let Some(legacy) = legacy_session_path(&app) {
                if !session_present(&legacy)? {
                    return Ok(None);
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(legacy, &path).map_err(|e| e.to_string())?;
            }
        }
    }
    read_optional_session(&path).map_err(|error| error.to_string())
}

/// Durably replace the bound graph's app-data session JSON with unvalidated
/// `data`: unique temp, file fsync, rename, then directory fsync. No bound graph
/// or any I/O failure returns an error. A post-rename directory-sync error is
/// also returned although the new file is already visible; its power-loss
/// durability is uncertain. O(data bytes) and two syncs, so the write runs on
/// the blocking pool and the command future awaits it: this command fires after
/// every navigation (debounced 150 ms) and the syncs took 2-3 s on a slow
/// Windows disk, which as a synchronous command froze the main thread (GH #623,
/// I-21). The durability contract is unchanged: the promise resolves only
/// after both syncs (or reports the failure).
#[tauri::command]
pub(crate) async fn save_session(
    data: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    let path = session_path(&app, &slot.root_key).ok_or("no app-data dir")?;
    // Taken before the first await, in invocation order.
    let ticket = SESSION_TICKET.fetch_add(1, Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        ordered_session_write(&path, ticket, || atomic_write_session(&path, &data))
    })
    .await
    .map_err(|error| error.to_string())?
}

static SESSION_TICKET: AtomicU64 = AtomicU64::new(1);
type SessionGate = Arc<Mutex<u64>>;
static SESSION_GATES: OnceLock<Mutex<HashMap<PathBuf, SessionGate>>> = OnceLock::new();

/// Run `write` for one session file one writer at a time, and never let an
/// older invocation overwrite a newer one that already published. The
/// synchronous command was ordered by the main thread; two blocking-pool jobs
/// are not, so an older save that lost the race to the lock reports success
/// without writing (the newer bytes it would have replaced are already on
/// disk, or the newer job reports its own failure).
fn ordered_session_write(
    path: &std::path::Path,
    ticket: u64,
    write: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let gate = SESSION_GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(path.to_owned())
        .or_default()
        .clone();
    let mut newest = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if ticket < *newest {
        return Ok(());
    }
    let outcome = write();
    // A published-but-unsynced outcome is an error here yet the file is the
    // newest bytes; an older ticket must still not replace it.
    *newest = ticket;
    outcome
}

fn atomic_write_session(path: &std::path::Path, data: &str) -> Result<(), String> {
    atomic_write_session_with_sync(
        path,
        data,
        tine_store::directory_durability::sync_directory_entry,
    )
}

fn atomic_write_session_with_sync(
    path: &std::path::Path,
    data: &str,
    sync: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> Result<(), String> {
    match atomic_write_json_with_sync(path, data, sync)? {
        WorkspaceSaveOutcome::Durable => Ok(()),
        WorkspaceSaveOutcome::PublishedUnsynced => {
            Err("session directory sync failed after publication".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_session_save_never_replaces_a_newer_one() {
        let path = std::path::Path::new("/ordered/session-a");
        let written = std::cell::RefCell::new(Vec::new());
        let save = |ticket: u64, bytes: &'static str| {
            ordered_session_write(path, ticket, || {
                written.borrow_mut().push(bytes);
                Ok(())
            })
        };
        save(10, "newer").unwrap();
        save(9, "older").unwrap();
        save(11, "newest").unwrap();
        assert_eq!(*written.borrow(), ["newer", "newest"]);
        // Another file has its own order.
        ordered_session_write(std::path::Path::new("/ordered/session-b"), 1, || {
            written.borrow_mut().push("other");
            Ok(())
        })
        .unwrap();
        assert_eq!(written.borrow().last(), Some(&"other"));
    }

    #[test]
    fn master_notice_read_is_graph_keyed_and_rollback_preserves_all_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::path::Path::new("/one/graph");
        let id = session_id(root);
        let path = temp.path().join("sessions").join(format!(
            "{}-notices.json",
            id.strip_suffix(".json").unwrap()
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let master_bytes = br#"{ "dismissed": ["query-crossing", "future-notice"], "future": 1 }"#;
        std::fs::write(&path, master_bytes).unwrap();
        assert!(master_crossing_notice_dismissed(temp.path(), root));
        assert!(!master_crossing_notice_dismissed(
            temp.path(),
            std::path::Path::new("/two/graph")
        ));
        assert_eq!(std::fs::read(&path).unwrap(), master_bytes);
        let rollback: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(rollback["dismissed"][0], "query-crossing");
        std::fs::write(&path, b"{torn").unwrap();
        assert!(!master_crossing_notice_dismissed(temp.path(), root));
        assert_eq!(std::fs::read(&path).unwrap(), b"{torn");
    }

    #[test]
    fn a_second_notice_is_independent_of_the_crossing_notice() {
        let temp = tempfile::tempdir().unwrap();
        let graph = std::path::Path::new("/one/graph");
        let other = std::path::Path::new("/two/graph");
        set_crossing_notice_at(temp.path(), graph, true).unwrap();
        assert!(!notice_dismissed(
            temp.path(),
            graph,
            "windows-defender-hint"
        ));
        set_notice_at(temp.path(), graph, "windows-defender-hint", true).unwrap();
        assert!(notice_dismissed(
            temp.path(),
            graph,
            "windows-defender-hint"
        ));
        // Dismissing one notice never un-dismisses or hides the other, and
        // the dismissal is keyed by graph.
        assert!(master_crossing_notice_dismissed(temp.path(), graph));
        assert!(!notice_dismissed(
            temp.path(),
            other,
            "windows-defender-hint"
        ));
        set_notice_at(temp.path(), graph, "windows-defender-hint", false).unwrap();
        assert!(!notice_dismissed(
            temp.path(),
            graph,
            "windows-defender-hint"
        ));
        assert!(master_crossing_notice_dismissed(temp.path(), graph));
    }

    #[test]
    fn graph_notice_dismissals_roundtrip_without_touching_graph_or_device_settings() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        std::fs::create_dir(&graph).unwrap();
        let page = graph.join("page.md");
        std::fs::write(&page, b"- unchanged\r\n").unwrap();
        let settings = temp.path().join("tine-settings.json");
        std::fs::write(&settings, br#"{"queryCrossingNoticeDismissed":true}"#).unwrap();
        let other = temp.path().join("other");
        let path = temp.path().join("sessions").join(notices_id(&graph));
        set_crossing_notice_at(temp.path(), &graph, true).unwrap();
        assert!(master_crossing_notice_dismissed(temp.path(), &graph));
        assert!(!master_crossing_notice_dismissed(temp.path(), &other));
        // Master consumes this exact payload and filename on rollback.
        assert_eq!(
            load_notices_at(&path),
            serde_json::json!({"dismissed":["query-crossing"]})
        );
        std::fs::write(&path, br#"{"dismissed":["future-notice"],"future":1}"#).unwrap();
        set_crossing_notice_at(temp.path(), &graph, true).unwrap();
        assert_eq!(
            load_notices_at(&path)["dismissed"],
            serde_json::json!(["future-notice", "query-crossing"])
        );
        assert_eq!(load_notices_at(&path)["future"], 1);
        set_crossing_notice_at(temp.path(), &graph, false).unwrap();
        assert!(!master_crossing_notice_dismissed(temp.path(), &graph));
        assert_eq!(
            load_notices_at(&path)["dismissed"],
            serde_json::json!(["future-notice"])
        );
        std::fs::write(&path, b"{torn").unwrap();
        set_crossing_notice_at(temp.path(), &graph, true).unwrap();
        assert!(master_crossing_notice_dismissed(temp.path(), &graph));
        assert_eq!(std::fs::read(page).unwrap(), b"- unchanged\r\n");
        assert_eq!(
            std::fs::read(settings).unwrap(),
            br#"{"queryCrossingNoticeDismissed":true}"#
        );
    }

    #[test]
    fn session_save_syncs_published_file_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions").join("session.json");
        atomic_write_session_with_sync(&path, "new tabs", |parent| {
            assert_eq!(parent, path.parent().unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "new tabs");
            Ok(())
        })
        .unwrap();
        let error = atomic_write_session_with_sync(&path, "newer tabs", |_| {
            Err(std::io::Error::other("directory sync failed"))
        })
        .unwrap_err();
        assert!(error.contains("directory sync failed"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "newer tabs");
    }

    #[test]
    fn workspace_sync_failure_after_rename_reports_publication_and_allows_next_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspaces.json");
        let first =
            r#"{"version":1,"activeId":"a","workspaces":[{"id":"a","name":"First","blob":{}}]}"#;
        let outcome = atomic_write_json_with_sync(&path, first, |_| {
            Err(std::io::Error::other("injected directory sync I/O failure"))
        })
        .unwrap();
        assert_eq!(outcome, WorkspaceSaveOutcome::PublishedUnsynced);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        let second = r#"{"version":1,"activeId":"a","workspaces":[{"id":"a","name":"First","blob":{}},{"id":"b","name":"Second","blob":{}}]}"#;
        assert_eq!(
            save_workspaces_at(&path, second).unwrap(),
            WorkspaceSaveOutcome::Durable
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), second);
    }

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

    #[test]
    fn workspace_migration_preserves_the_session_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("graph.json");
        let registry = temp.path().join("graph-workspaces.json");
        let prior = r#"{"tabs":[{"history":[{"kind":"page","name":"Prior","pageKind":"page"}],"pos":0,"pinned":true}],"activeIndex":0}"#;
        std::fs::write(&session, prior).unwrap();

        let first = load_workspaces_at(&registry, &session).unwrap();
        let first_value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first_value["activeId"], "default");
        assert_eq!(first_value["workspaces"][0]["name"], "");
        assert_eq!(
            first_value["workspaces"][0]["blob"],
            serde_json::from_str::<serde_json::Value>(prior).unwrap()
        );
        assert_eq!(std::fs::read_to_string(&session).unwrap(), prior);

        let bytes = std::fs::read(&registry).unwrap();
        let modified = std::fs::metadata(&registry).unwrap().modified().unwrap();
        let second = load_workspaces_at(&registry, &session).unwrap();
        assert_eq!(second, first);
        assert_eq!(std::fs::read(&registry).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(&registry).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(std::fs::read_to_string(&session).unwrap(), prior);
    }

    #[test]
    fn workspace_registry_full_cycle_keeps_graph_page_bytes_and_mtime_identical() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("tine-test");
        let pages = graph.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        let page = pages.join("byte-identical.md");
        std::fs::write(&page, "- original graph bytes\n").unwrap();
        let before_bytes = std::fs::read(&page).unwrap();
        let before_modified = std::fs::metadata(&page).unwrap().modified().unwrap();

        let registry = temp
            .path()
            .join("app-data/sessions/tine-test-workspaces.json");
        let states = [
            serde_json::json!({"version":1,"activeId":"one","workspaces":[{"id":"one","name":"One","blob":blank_session_json()}]}),
            serde_json::json!({"version":1,"activeId":"two","workspaces":[{"id":"one","name":"One","blob":blank_session_json()},{"id":"two","name":"Two","blob":blank_session_json()}]}),
            serde_json::json!({"version":1,"activeId":"one","workspaces":[{"id":"one","name":"Renamed","blob":blank_session_json()},{"id":"two","name":"Two","blob":blank_session_json()}]}),
            serde_json::json!({"version":1,"activeId":"one","workspaces":[{"id":"one","name":"Renamed","blob":blank_session_json()}]}),
        ];
        for state in states {
            save_workspaces_at(&registry, &state.to_string()).unwrap();
            assert_eq!(std::fs::read(&page).unwrap(), before_bytes);
            assert_eq!(
                std::fs::metadata(&page).unwrap().modified().unwrap(),
                before_modified
            );
        }
    }

    #[test]
    fn external_asset_approvals_are_device_local_and_target_specific() {
        let mut json = serde_json::json!({ "unrelated": true });
        remember_external_assets_approval_json(&mut json, "/graphs/a", "/media/one");
        remember_external_assets_approval_json(&mut json, "/graphs/b", "/media/two");
        remember_external_assets_approval_json(&mut json, "/graphs/a", "/media/retargeted");

        let approvals = external_assets_approvals(&json);
        assert_eq!(approvals["/graphs/a"], "/media/retargeted");
        assert_eq!(approvals["/graphs/b"], "/media/two");
        assert_eq!(json["unrelated"], true);
    }

    #[test]
    fn update_settings_refuses_an_unparseable_file_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tine-settings.json");
        let broken = r#"{"known_graphs":[{"path":"/g","name":"g"}],"plugin_states":{"p":"#;
        std::fs::write(&path, broken).unwrap();
        let error = update_settings_at(&path, |json| {
            remember_graph_json(json, "/graphs/new");
        })
        .unwrap_err();
        assert!(error.contains("tine-settings.json"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn an_oversize_session_is_an_error_not_a_blank_workspace() {
        // I-22: the session read is bounded; a damaged huge file is reported
        // like any unreadable session, never loaded whole and never "missing".
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session.json");
        std::fs::File::create(&session)
            .unwrap()
            .set_len(33 * 1024 * 1024)
            .unwrap();
        let error = read_optional_session(&session).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(read_optional_session(&dir.path().join("none"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn update_settings_refuses_a_non_object_root_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tine-settings.json");
        for root in ["[1,2]", "42", "\"text\"", "null"] {
            std::fs::write(&path, root).unwrap();
            let outcome = std::panic::catch_unwind(|| {
                update_settings_at(&path, |json| {
                    json["capture_enter_files"] = serde_json::Value::Bool(true);
                })
            });
            let result = outcome.expect("a non-object root must never panic the writer");
            assert!(result.is_err(), "{root}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), root);
        }
    }

    #[test]
    fn update_settings_carries_unknown_keys_and_starts_fresh_when_absent_or_blank() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("tine-settings.json");
        update_settings_at(&path, |json| json["a"] = serde_json::json!(1)).unwrap();
        assert_eq!(read_json(&path)["a"], 1);
        std::fs::write(&path, r#"{"future_key":{"nested":[1,2]},"a":1}"#).unwrap();
        update_settings_at(&path, |json| json["b"] = serde_json::json!(2)).unwrap();
        let after = read_json(&path);
        assert_eq!(after["future_key"], serde_json::json!({"nested":[1,2]}));
        assert_eq!(
            (after["a"].clone(), after["b"].clone()),
            (1.into(), 2.into())
        );
        std::fs::write(&path, "  \n").unwrap();
        update_settings_at(&path, |json| json["c"] = serde_json::json!(3)).unwrap();
        assert_eq!(read_json(&path)["c"], 3);
    }

    fn read_json(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn app_bool_reader_preserves_defaults_for_missing_or_invalid_settings() {
        let root = std::env::temp_dir().join(format!(
            "tine-settings-bool-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");

        assert!(app_bool_at(&path, "frame", true));
        std::fs::write(&path, "not json").unwrap();
        assert!(!app_bool_at(&path, "frame", false));
        std::fs::write(&path, r#"{"frame":true,"other":false}"#).unwrap();
        assert!(app_bool_at(&path, "frame", false));
        assert!(app_bool_at(&path, "missing", true));

        let _ = std::fs::remove_dir_all(root);
    }
}

/// Favorites membership (`:favorites`) and, once the graph has one, the
/// arrangement page (`:tine/favorites-page`), in one guarded config write.
#[tauri::command]
pub(crate) fn set_favorites(
    names: Vec<String>,
    page: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::config::set_favorites(&slot.store, &names, page.as_deref())
        .map_err(|e| e.to_string())
}

/// Persist or clear the graph home page through the guarded config transaction.
/// Cost follows config.edn bytes; malformed map and I/O errors are returned.
#[tauri::command]
pub(crate) fn set_default_home(
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::config::set_default_home_page(&slot.store, name.as_deref())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod fail_read_tests {
    use super::*;
    #[test]
    fn fail_read_legacy_session_refuses_blank_workspace_publication() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("session.json");
        assert!(read_optional_session(&session).unwrap().is_none());
        std::fs::create_dir(&session).unwrap();
        assert!(read_optional_session(&session).is_err());
        let registry = temp.path().join("workspaces.json");
        assert!(load_workspaces_at(&registry, &session).is_err());
        assert!(
            !registry.exists(),
            "I-2: read failure cannot publish an empty registry"
        );
    }
}
