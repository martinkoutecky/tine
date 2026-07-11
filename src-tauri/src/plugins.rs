use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri::Manager;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_WASM_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct InstalledPlugin {
    manifest_json: String,
    sha256: String,
    selected: bool,
    enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PluginState {
    version: String,
    enabled: bool,
}

fn safe_component(value: &str, dotted: bool) -> bool {
    if value.is_empty() || value.len() > 64 || value.starts_with('.') || value.ends_with('.') {
        return false;
    }
    value.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || byte == b'-'
            || (dotted && byte == b'.')
    }) && (!dotted || value.contains('.'))
}

fn safe_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 || value.starts_with('.') || value.ends_with('.') {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        && value.split('.').take(3).count() == 3
}

fn manifest_identity(manifest_json: &str) -> Result<(String, String), String> {
    if manifest_json.len() > MAX_MANIFEST_BYTES {
        return Err("plugin manifest is too large".to_string());
    }
    let manifest: serde_json::Value =
        serde_json::from_str(manifest_json).map_err(|_| "plugin manifest is invalid JSON")?;
    let id = manifest
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|value| safe_component(value, true))
        .ok_or("plugin id is invalid")?;
    let version = manifest
        .get("version")
        .and_then(|value| value.as_str())
        .filter(|value| safe_version(value))
        .ok_or("plugin version is invalid")?;
    Ok((id.to_string(), version.to_string()))
}

fn plugins_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("plugins"))
        .map_err(|_| "no app-data dir".to_string())
}

fn package_dir(root: &Path, id: &str, version: &str) -> Result<PathBuf, String> {
    if !safe_component(id, true) || !safe_version(version) {
        return Err("plugin identity is invalid".to_string());
    }
    Ok(root.join(id).join(version))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn plugin_states(app: &tauri::AppHandle) -> std::collections::HashMap<String, PluginState> {
    crate::settings::settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json.get("plugin_states").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Persist an immutable plugin version. Installation never executes the guest and
/// leaves it disabled; enabling is a separate explicit action after the frontend
/// has validated the complete manifest and WebAssembly ABI.
#[tauri::command]
pub(crate) fn install_plugin(
    manifest_json: String,
    wasm_b64: String,
    app: tauri::AppHandle,
) -> Result<InstalledPlugin, String> {
    let (id, version) = manifest_identity(&manifest_json)?;
    let wasm = base64::engine::general_purpose::STANDARD
        .decode(wasm_b64)
        .map_err(|_| "plugin entry is not valid base64")?;
    if wasm.len() > MAX_WASM_BYTES {
        return Err("plugin entry is too large".to_string());
    }
    if !wasm.starts_with(b"\0asm\x01\0\0\0") {
        return Err("plugin entry is not WebAssembly".to_string());
    }
    let digest = sha256(&wasm);
    let root = plugins_dir(&app)?;
    let target = package_dir(&root, &id, &version)?;
    if target.exists() {
        let existing = std::fs::read(target.join("plugin.wasm")).map_err(|e| e.to_string())?;
        let existing_manifest =
            std::fs::read_to_string(target.join("manifest.json")).map_err(|e| e.to_string())?;
        if sha256(&existing) != digest || existing_manifest != manifest_json {
            return Err(
                "that immutable plugin version is already installed with different bytes"
                    .to_string(),
            );
        }
    } else {
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let temp = root.join(format!(
            ".install-{}-{}-{}",
            id,
            version,
            std::process::id()
        ));
        if temp.exists() {
            std::fs::remove_dir_all(&temp).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&temp).map_err(|e| e.to_string())?;
        let result = (|| -> Result<(), String> {
            std::fs::write(temp.join("manifest.json"), manifest_json.as_bytes())
                .map_err(|e| e.to_string())?;
            std::fs::write(temp.join("plugin.wasm"), &wasm).map_err(|e| e.to_string())?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&temp, &target).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&temp);
        }
        result?;
    }
    Ok(InstalledPlugin {
        manifest_json,
        sha256: digest,
        selected: false,
        enabled: false,
    })
}

#[tauri::command]
pub(crate) fn list_installed_plugins(app: tauri::AppHandle) -> Vec<InstalledPlugin> {
    let Ok(root) = plugins_dir(&app) else {
        return Vec::new();
    };
    let states = plugin_states(&app);
    let mut installed = Vec::new();
    let Ok(ids) = std::fs::read_dir(root) else {
        return installed;
    };
    for id_entry in ids.flatten().filter(|entry| entry.path().is_dir()) {
        let Some(id) = id_entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !safe_component(&id, true) {
            continue;
        }
        let Ok(versions) = std::fs::read_dir(id_entry.path()) else {
            continue;
        };
        for version_entry in versions.flatten().filter(|entry| entry.path().is_dir()) {
            let Some(version) = version_entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !safe_version(&version) {
                continue;
            }
            let Ok(manifest_json) =
                std::fs::read_to_string(version_entry.path().join("manifest.json"))
            else {
                continue;
            };
            if manifest_identity(&manifest_json).ok().as_ref()
                != Some(&(id.clone(), version.clone()))
            {
                continue;
            }
            let Ok(wasm) = std::fs::read(version_entry.path().join("plugin.wasm")) else {
                continue;
            };
            let state = states.get(&id);
            let selected = state.is_some_and(|item| item.version == version);
            installed.push(InstalledPlugin {
                manifest_json,
                sha256: sha256(&wasm),
                selected,
                enabled: selected && state.is_some_and(|item| item.enabled),
            });
        }
    }
    installed.sort_by(|a, b| a.manifest_json.cmp(&b.manifest_json));
    installed
}

#[tauri::command]
pub(crate) fn read_plugin_entry(
    id: String,
    version: String,
    app: tauri::AppHandle,
) -> Result<tauri::ipc::Response, String> {
    let path = package_dir(&plugins_dir(&app)?, &id, &version)?.join("plugin.wasm");
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_WASM_BYTES || !bytes.starts_with(b"\0asm\x01\0\0\0") {
        return Err("installed plugin entry is invalid".to_string());
    }
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub(crate) fn set_plugin_enabled(
    id: String,
    version: String,
    enabled: bool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let target = package_dir(&plugins_dir(&app)?, &id, &version)?;
    if !target.join("manifest.json").is_file() || !target.join("plugin.wasm").is_file() {
        return Err("plugin version is not installed".to_string());
    }
    crate::settings::update_settings(&app, |json| {
        json["plugin_states"][&id] = serde_json::json!({ "version": version, "enabled": enabled });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_identity_cannot_escape_its_storage_root() {
        assert!(safe_component("dev.tine.example", true));
        assert!(!safe_component("../example", true));
        assert!(!safe_component("Example.Plugin", true));
        assert!(safe_version("0.1.0-beta.1"));
        assert!(!safe_version("../../outside"));
        assert!(package_dir(Path::new("/plugins"), "dev.tine.example", "0.1.0").is_ok());
    }

    #[test]
    fn manifest_identity_rejects_untrusted_paths_and_oversized_input() {
        let good = r#"{"id":"dev.tine.example","version":"0.1.0"}"#;
        assert_eq!(
            manifest_identity(good).unwrap(),
            ("dev.tine.example".to_string(), "0.1.0".to_string())
        );
        assert!(manifest_identity(r#"{"id":"../bad","version":"0.1.0"}"#).is_err());
        assert!(manifest_identity(&"x".repeat(MAX_MANIFEST_BYTES + 1)).is_err());
    }
}
