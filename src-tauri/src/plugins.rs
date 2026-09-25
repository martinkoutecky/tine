use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Manager;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_WASM_BYTES: usize = 8 * 1024 * 1024;
const MAX_REGISTRY_INDEX_BYTES: usize = 2 * 1024 * 1024;
const MAX_REGISTRY_SIGNATURE_BYTES: usize = 1024;
const REGISTRY_CACHE_KEY: &str = "plugin_registry_cache";
const LEGACY_REGISTRY_INDEX_KEY: &str = "plugin-registry-index";
const LEGACY_REGISTRY_SIGNATURE_KEY: &str = "plugin-registry-signature";
static INSTALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const REGISTRY_PUBLIC_KEY: [u8; 32] = [
    0x6c, 0x25, 0xa1, 0xfd, 0x0c, 0x6d, 0xbc, 0x60, 0xca, 0xb7, 0xa4, 0x8c, 0x23, 0x6a, 0xa9, 0x18,
    0x45, 0x66, 0xa6, 0x57, 0xff, 0x69, 0x72, 0x46, 0xd3, 0x0b, 0xaf, 0xc4, 0x7e, 0x17, 0x6c, 0x00,
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PluginRegistryCacheEnvelope {
    schema_version: u8,
    index_json: String,
    signature: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LegacyPluginRegistryCache {
    index_json: String,
    signature: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum PluginRegistryCacheLoad {
    Absent,
    Envelope {
        envelope: PluginRegistryCacheEnvelope,
    },
    Legacy {
        #[serde(rename = "indexJson")]
        index_json: String,
        signature: String,
    },
    Unsafe {
        reason: String,
    },
}

fn unsafe_cache(reason: impl Into<String>) -> PluginRegistryCacheLoad {
    PluginRegistryCacheLoad::Unsafe {
        reason: reason.into(),
    }
}

fn load_plugin_registry_cache_at(path: &Path) -> PluginRegistryCacheLoad {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return PluginRegistryCacheLoad::Absent;
        }
        Err(error) => {
            return unsafe_cache(format!("registry cache settings are unreadable: {error}"))
        }
    };
    let root: serde_json::Value = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) if value.is_object() => value,
        Ok(_) => return unsafe_cache("registry cache settings root is not an object"),
        Err(error) => {
            return unsafe_cache(format!("registry cache settings are malformed: {error}"))
        }
    };
    if let Some(value) = root.get(REGISTRY_CACHE_KEY) {
        let envelope: PluginRegistryCacheEnvelope = match serde_json::from_value(value.clone()) {
            Ok(envelope) => envelope,
            Err(error) => {
                return unsafe_cache(format!("registry cache envelope is malformed: {error}"))
            }
        };
        if envelope.schema_version != 1
            || envelope.index_json.is_empty()
            || envelope.index_json.len() > MAX_REGISTRY_INDEX_BYTES
            || envelope.signature.trim().is_empty()
            || envelope.signature.len() > MAX_REGISTRY_SIGNATURE_BYTES
        {
            return unsafe_cache("registry cache envelope violates its size or schema contract");
        }
        return PluginRegistryCacheLoad::Envelope { envelope };
    }

    let legacy_index = root.get(LEGACY_REGISTRY_INDEX_KEY);
    let legacy_signature = root.get(LEGACY_REGISTRY_SIGNATURE_KEY);
    match (legacy_index, legacy_signature) {
        (None, None) => PluginRegistryCacheLoad::Absent,
        (Some(index), Some(signature)) => {
            let Some(index_json) = index.as_str() else {
                return unsafe_cache("legacy registry index has the wrong type");
            };
            let Some(signature) = signature.as_str() else {
                return unsafe_cache("legacy registry signature has the wrong type");
            };
            if index_json.is_empty()
                || index_json.len() > MAX_REGISTRY_INDEX_BYTES
                || signature.trim().is_empty()
                || signature.len() > MAX_REGISTRY_SIGNATURE_BYTES
            {
                return unsafe_cache("legacy registry cache violates its size contract");
            }
            PluginRegistryCacheLoad::Legacy {
                index_json: index_json.to_string(),
                signature: signature.to_string(),
            }
        }
        _ => unsafe_cache("legacy registry cache is torn"),
    }
}

fn store_plugin_registry_cache_at(
    path: &Path,
    index_json: String,
    signature: String,
    expected_legacy: Option<LegacyPluginRegistryCache>,
) -> Result<(), String> {
    if index_json.is_empty() || index_json.len() > MAX_REGISTRY_INDEX_BYTES {
        return Err("plugin registry index is empty or too large".to_string());
    }
    let signature = signature.trim().to_string();
    if signature.is_empty() || signature.len() > MAX_REGISTRY_SIGNATURE_BYTES {
        return Err("plugin registry signature is empty or too large".to_string());
    }
    verify_plugin_registry(index_json.clone(), signature.clone())?;
    let envelope = PluginRegistryCacheEnvelope {
        schema_version: 1,
        index_json,
        signature,
    };
    crate::settings::update_settings_strict_at(path, |json| {
        if let Some(expected) = &expected_legacy {
            let actual_index = json
                .get(LEGACY_REGISTRY_INDEX_KEY)
                .and_then(serde_json::Value::as_str);
            let actual_signature = json
                .get(LEGACY_REGISTRY_SIGNATURE_KEY)
                .and_then(serde_json::Value::as_str);
            if json.get(REGISTRY_CACHE_KEY).is_some()
                || actual_index != Some(expected.index_json.as_str())
                || actual_signature != Some(expected.signature.as_str())
            {
                return Err("legacy registry cache changed during migration".to_string());
            }
        }
        json[REGISTRY_CACHE_KEY] =
            serde_json::to_value(&envelope).map_err(|error| error.to_string())?;
        let object = json
            .as_object_mut()
            .ok_or_else(|| "device settings root is not an object".to_string())?;
        object.remove(LEGACY_REGISTRY_INDEX_KEY);
        object.remove(LEGACY_REGISTRY_SIGNATURE_KEY);
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn load_plugin_registry_cache(app: tauri::AppHandle) -> PluginRegistryCacheLoad {
    crate::settings::settings_path(&app).map_or_else(
        || unsafe_cache("no app-data dir"),
        |path| load_plugin_registry_cache_at(&path),
    )
}

#[tauri::command]
pub(crate) fn store_plugin_registry_cache(
    index_json: String,
    signature: String,
    expected_legacy: Option<LegacyPluginRegistryCache>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let path = crate::settings::settings_path(&app).ok_or("no app-data dir")?;
    store_plugin_registry_cache_at(&path, index_json, signature, expected_legacy)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct InstalledPlugin {
    id: String,
    version: String,
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
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let (core, prerelease) = value
        .split_once('-')
        .map_or((value, None), |(core, prerelease)| (core, Some(prerelease)));
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
        })
    {
        return false;
    }
    prerelease.is_none_or(|suffix| {
        !suffix.is_empty()
            && !suffix.starts_with('.')
            && !suffix.ends_with('.')
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    })
}

fn unique_install_dir(root: &Path, id: &str, version: &str) -> Result<PathBuf, String> {
    for _ in 0..128 {
        let sequence = INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = root.join(format!(
            ".install-{id}-{version}-{}-{sequence}",
            std::process::id()
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not allocate a private plugin install directory".to_string())
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

/// Validate one immutable package without following a symlink out of plugin
/// storage. The boolean reports whether this is currently the id's last entry.
fn validate_uninstall_target(
    root: &Path,
    id: &str,
    version: &str,
) -> Result<(PathBuf, PathBuf, bool), String> {
    let target = package_dir(root, id, version)?;
    let id_dir = root.join(id);
    let id_meta = std::fs::symlink_metadata(&id_dir).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "plugin version is not installed".to_string()
        } else {
            error.to_string()
        }
    })?;
    if id_meta.file_type().is_symlink() || !id_meta.is_dir() {
        return Err("installed plugin directory is unsafe".to_string());
    }
    let target_meta = std::fs::symlink_metadata(&target).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "plugin version is not installed".to_string()
        } else {
            error.to_string()
        }
    })?;
    if target_meta.file_type().is_symlink() || !target_meta.is_dir() {
        return Err("installed plugin package is unsafe".to_string());
    }
    let manifest_json = std::fs::read_to_string(target.join("manifest.json"))
        .map_err(|_| "installed plugin manifest is unreadable".to_string())?;
    if manifest_identity(&manifest_json).ok().as_ref()
        != Some(&(id.to_string(), version.to_string()))
    {
        return Err("installed plugin manifest identity does not match its directory".to_string());
    }
    let mut last_version = true;
    for entry in std::fs::read_dir(&id_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.path() != target {
            last_version = false;
            break;
        }
    }
    Ok((id_dir, target, last_version))
}

/// Remove exactly one validated immutable package without ever following a
/// symlink out of plugin storage. Returns true when no versions of this plugin
/// remain and the now-empty id directory was removed too.
fn uninstall_package(root: &Path, id: &str, version: &str) -> Result<bool, String> {
    let (id_dir, target, _) = validate_uninstall_target(root, id, version)?;
    std::fs::remove_dir_all(&target).map_err(|error| error.to_string())?;
    let mut remaining = std::fs::read_dir(&id_dir).map_err(|error| error.to_string())?;
    if remaining.next().is_none() {
        std::fs::remove_dir(&id_dir).map_err(|error| error.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[tauri::command]
pub(crate) fn verify_plugin_registry(
    index_json: String,
    signature_b64: String,
) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    if index_json.len() > MAX_REGISTRY_INDEX_BYTES {
        return Err("plugin registry index is too large".to_string());
    }
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| "plugin registry signature is invalid base64")?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| "plugin registry signature has the wrong length")?;
    let key = VerifyingKey::from_bytes(&REGISTRY_PUBLIC_KEY)
        .map_err(|_| "embedded plugin registry key is invalid")?;
    key.verify(index_json.as_bytes(), &signature)
        .map_err(|_| "plugin registry signature did not verify".to_string())
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
        let temp = unique_install_dir(&root, &id, &version)?;
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
        id,
        version,
        manifest_json,
        sha256: digest,
        selected: false,
        enabled: false,
    })
}

/// Uninstall removes only the app-local immutable package. It clears a selected
/// version before deleting bytes so a crash cannot leave startup pointing at a
/// half-removed plugin. Per-plugin settings are retained while another version
/// remains and removed with the last version. Graph files are never in scope.
#[tauri::command]
pub(crate) fn uninstall_plugin(
    id: String,
    version: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let root = plugins_dir(&app)?;
    let (_, _, last_version) = validate_uninstall_target(&root, &id, &version)?;
    crate::settings::update_settings(&app, |json| {
        let selected_version = json
            .get("plugin_states")
            .and_then(|states| states.get(&id))
            .and_then(|state| state.get("version"))
            .and_then(|value| value.as_str());
        if last_version || selected_version == Some(version.as_str()) {
            if let Some(states) = json
                .get_mut("plugin_states")
                .and_then(serde_json::Value::as_object_mut)
            {
                states.remove(&id);
            }
        }
        if last_version {
            if let Some(root) = json.as_object_mut() {
                root.remove(&format!("plugin-settings:{id}"));
            }
        }
    })?;
    uninstall_package(&root, &id, &version)?;
    Ok(())
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
                id: id.clone(),
                version: version.clone(),
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
    let root = plugins_dir(&app)?;
    let settings = crate::settings::settings_path(&app).ok_or("no app-data dir")?;
    set_plugin_enabled_at(&root, &settings, &id, &version, enabled)
}

fn set_plugin_enabled_at(
    plugins_root: &Path,
    settings_path: &Path,
    id: &str,
    version: &str,
    enabled: bool,
) -> Result<(), String> {
    let target = package_dir(plugins_root, id, version)?;
    if !target.join("manifest.json").is_file() || !target.join("plugin.wasm").is_file() {
        return Err("plugin version is not installed".to_string());
    }
    crate::settings::update_settings_strict_at(settings_path, |json| {
        json["plugin_states"][id] = serde_json::json!({ "version": version, "enabled": enabled });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNED_CONTROL_INDEX: &str = "{\n  \"schemaVersion\": 1,\n  \"generatedAt\": \"2026-07-12T00:00:00Z\",\n  \"plugins\": [],\n  \"themes\": [],\n  \"revocations\": []\n}\n";
    const SIGNED_CONTROL_SIGNATURE: &str =
        "2g6EPs5ssf7fkuBH5kYfDNaCEnoTX8PznGPsZ6yzz+xVMggocK5cyYHyE3tnnFGeyuMIBLx6ixPaHWN0FvNdAw==";

    fn test_manifest(id: &str, version: &str) -> String {
        format!(r#"{{"id":"{id}","version":"{version}"}}"#)
    }

    fn write_test_package(root: &Path, id: &str, version: &str) -> PathBuf {
        let package = root.join(id).join(version);
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("manifest.json"), test_manifest(id, version)).unwrap();
        std::fs::write(package.join("plugin.wasm"), b"\0asm\x01\0\0\0").unwrap();
        package
    }

    #[test]
    fn plugin_identity_cannot_escape_its_storage_root() {
        assert!(safe_component("dev.tine.example", true));
        assert!(!safe_component("../example", true));
        assert!(!safe_component("Example.Plugin", true));
        assert!(safe_version("0.1.0-beta.1"));
        assert!(!safe_version("0.1.0.4"));
        assert!(!safe_version("01.1.0"));
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

    #[test]
    fn registry_public_key_has_the_expected_identity() {
        assert_eq!(REGISTRY_PUBLIC_KEY.len(), 32);
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&REGISTRY_PUBLIC_KEY).is_ok());
        verify_plugin_registry(
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
        )
        .unwrap();
        assert!(verify_plugin_registry(
            format!("{SIGNED_CONTROL_INDEX} "),
            SIGNED_CONTROL_SIGNATURE.to_string()
        )
        .is_err());
    }

    fn envelope(index_json: &str, signature: &str) -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 1,
            "indexJson": index_json,
            "signature": signature,
        })
    }

    #[test]
    fn registry_cache_store_is_one_atomic_envelope_and_preserves_unrelated_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tine-settings.json");
        let old = serde_json::json!({
            "unrelated": { "keep": true },
            REGISTRY_CACHE_KEY: envelope("old-index", "old-signature"),
        });
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&old).unwrap()),
        )
        .unwrap();

        store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
            None,
        )
        .unwrap();

        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(persisted["unrelated"]["keep"], true);
        assert_eq!(
            persisted[REGISTRY_CACHE_KEY],
            envelope(SIGNED_CONTROL_INDEX, SIGNED_CONTROL_SIGNATURE)
        );
        assert!(persisted.get(LEGACY_REGISTRY_INDEX_KEY).is_none());
        assert!(persisted.get(LEGACY_REGISTRY_SIGNATURE_KEY).is_none());
        assert!(matches!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Envelope { .. }
        ));
    }

    #[test]
    fn registry_cache_load_distinguishes_absent_torn_and_malformed_states() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tine-settings.json");
        assert_eq!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Absent
        );
        std::fs::write(&path, "{}\n").unwrap();
        assert_eq!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Absent
        );

        std::fs::write(
            &path,
            format!(r#"{{"{LEGACY_REGISTRY_INDEX_KEY}":"index"}}"#),
        )
        .unwrap();
        assert!(matches!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Unsafe { .. }
        ));
        std::fs::write(&path, format!(r#"{{"{REGISTRY_CACHE_KEY}":{{"schemaVersion":1,"indexJson":"x","signature":"y","extra":true}}}}"#)).unwrap();
        assert!(matches!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Unsafe { .. }
        ));
        std::fs::write(&path, "not-json\n").unwrap();
        assert!(matches!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Unsafe { .. }
        ));
    }

    #[test]
    fn guarded_legacy_migration_removes_both_keys_or_publishes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tine-settings.json");
        let legacy = LegacyPluginRegistryCache {
            index_json: SIGNED_CONTROL_INDEX.to_string(),
            signature: SIGNED_CONTROL_SIGNATURE.to_string(),
        };
        let initial = serde_json::json!({
            "keep": 7,
            LEGACY_REGISTRY_INDEX_KEY: legacy.index_json,
            LEGACY_REGISTRY_SIGNATURE_KEY: legacy.signature,
        });
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&initial).unwrap()),
        )
        .unwrap();

        let mismatched = LegacyPluginRegistryCache {
            index_json: "other".to_string(),
            signature: SIGNED_CONTROL_SIGNATURE.to_string(),
        };
        let before = std::fs::read(&path).unwrap();
        assert!(store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
            Some(mismatched),
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);

        store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
            Some(legacy),
        )
        .unwrap();
        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(persisted["keep"], 7);
        assert!(persisted.get(LEGACY_REGISTRY_INDEX_KEY).is_none());
        assert!(persisted.get(LEGACY_REGISTRY_SIGNATURE_KEY).is_none());
        assert_eq!(
            persisted[REGISTRY_CACHE_KEY],
            envelope(SIGNED_CONTROL_INDEX, SIGNED_CONTROL_SIGNATURE)
        );
    }

    #[test]
    fn invalid_or_unpublishable_registry_cache_never_replaces_last_good_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tine-settings.json");
        std::fs::write(&path, "{\n  \"keep\": true\n}\n").unwrap();
        let before = std::fs::read(&path).unwrap();

        assert!(store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            "invalid".to_string(),
            None,
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(store_plugin_registry_cache_at(
            &path,
            "x".repeat(MAX_REGISTRY_INDEX_BYTES + 1),
            SIGNED_CONTROL_SIGNATURE.to_string(),
            None,
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);

        std::fs::write(&path, "not-json\n").unwrap();
        let malformed = std::fs::read(&path).unwrap();
        assert!(store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
            None,
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), malformed);
    }

    #[cfg(unix)]
    #[test]
    fn registry_cache_publication_failure_preserves_last_good_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tine-settings.json");
        let old = serde_json::json!({ REGISTRY_CACHE_KEY: envelope("old-index", "old-signature") });
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&old).unwrap()),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let original_mode = std::fs::metadata(temp.path()).unwrap().permissions().mode();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
            None,
        );
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(original_mode))
            .unwrap();

        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn concurrent_registry_cache_readers_observe_complete_old_or_new_envelopes() {
        use std::sync::Arc;
        let temp = tempfile::tempdir().unwrap();
        let path = Arc::new(temp.path().join("tine-settings.json"));
        let old = serde_json::json!({ REGISTRY_CACHE_KEY: envelope("old-index", "old-signature") });
        std::fs::write(
            path.as_ref(),
            format!("{}\n", serde_json::to_string_pretty(&old).unwrap()),
        )
        .unwrap();

        let writer_path = Arc::clone(&path);
        let writer = std::thread::spawn(move || {
            store_plugin_registry_cache_at(
                writer_path.as_ref(),
                SIGNED_CONTROL_INDEX.to_string(),
                SIGNED_CONTROL_SIGNATURE.to_string(),
                None,
            )
            .unwrap();
        });
        for _ in 0..500 {
            let text = std::fs::read_to_string(path.as_ref()).unwrap();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let cache = &value[REGISTRY_CACHE_KEY];
            let pair = (
                cache["indexJson"].as_str().unwrap(),
                cache["signature"].as_str().unwrap(),
            );
            assert!(
                pair == ("old-index", "old-signature")
                    || pair == (SIGNED_CONTROL_INDEX, SIGNED_CONTROL_SIGNATURE)
            );
        }
        writer.join().unwrap();
    }

    #[test]
    fn revoked_plugin_state_is_durably_disabled_without_opening_guest_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let plugins = temp.path().join("plugins");
        write_test_package(&plugins, "page.tine.revoked", "1.0.0");
        let settings = temp.path().join("tine-settings.json");
        std::fs::write(
            &settings,
            r#"{"plugin_states":{"page.tine.revoked":{"version":"1.0.0","enabled":true}}}"#,
        )
        .unwrap();

        set_plugin_enabled_at(&plugins, &settings, "page.tine.revoked", "1.0.0", false).unwrap();

        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        assert_eq!(
            persisted["plugin_states"]["page.tine.revoked"]["enabled"],
            false
        );
        assert_eq!(
            persisted["plugin_states"]["page.tine.revoked"]["version"],
            "1.0.0"
        );
    }

    #[test]
    fn uninstall_removes_only_the_requested_version() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        let first = write_test_package(&root, "dev.tine.example", "0.1.0");
        let second = write_test_package(&root, "dev.tine.example", "0.2.0");
        let other = write_test_package(&root, "dev.tine.other", "1.0.0");

        assert!(!uninstall_package(&root, "dev.tine.example", "0.1.0").unwrap());
        assert!(!first.exists());
        assert!(second.exists());
        assert!(other.exists());
        assert!(uninstall_package(&root, "dev.tine.example", "0.2.0").unwrap());
        assert!(!root.join("dev.tine.example").exists());
        assert!(other.exists());
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_refuses_symlinked_plugin_directories() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        let outside = temp.path().join("outside");
        write_test_package(&outside, "dev.tine.example", "0.1.0");
        std::fs::create_dir_all(&root).unwrap();
        symlink(
            outside.join("dev.tine.example"),
            root.join("dev.tine.example"),
        )
        .unwrap();

        assert!(uninstall_package(&root, "dev.tine.example", "0.1.0").is_err());
        assert!(outside.join("dev.tine.example/0.1.0").exists());
    }
}
