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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum PluginRegistryCacheLoad {
    Absent,
    Envelope {
        envelope: PluginRegistryCacheEnvelope,
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

    // The pre-envelope `plugin-registry-index`/`-signature` pair is a retired,
    // disposable cache shape (master 4b752120f): it is ignored, and the live
    // registry refetch republishes a verified envelope.
    PluginRegistryCacheLoad::Absent
}

fn store_plugin_registry_cache_at(
    path: &Path,
    index_json: String,
    signature: String,
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
        json[REGISTRY_CACHE_KEY] =
            serde_json::to_value(&envelope).map_err(|error| error.to_string())?;
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
    app: tauri::AppHandle,
) -> Result<(), String> {
    let path = crate::settings::settings_path(&app).ok_or("no app-data dir")?;
    store_plugin_registry_cache_at(&path, index_json, signature)
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

/// The two regular files every complete package holds.
const PACKAGE_FILES: &[&str] = &["manifest.json", "plugin.wasm"];
/// Transient root entries; neither prefix can be a valid plugin id
/// (`safe_component` rejects a leading dot), so they never shadow a package.
const INSTALL_PREFIX: &str = ".install-";
const RETIRED_PREFIX: &str = ".retired-";

/// How an install ended: a new immutable version, or byte-identical bytes that
/// were already present (a repeated install is a no-op, not an error).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackagePublishOutcome {
    Published,
    AlreadyPresentExact,
}

/// Serializes this process's package-store mutations (publish, retire,
/// recovery) so concurrent Tauri commands cannot interleave inside one store.
/// Other processes are handled by the no-replace moves themselves.
fn lock_plugin_store() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unique_transient_name(prefix: &str, id: &str, version: &str) -> String {
    let sequence = INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{id}-{version}-{}-{sequence}", std::process::id())
}

/// Remove one store entry without following a symlink out of plugin storage,
/// then make the removal durable. An already-absent entry is success.
fn reclaim_entry(parent: &Path, name: &std::ffi::OsStr) -> std::io::Result<()> {
    let path = parent.join(name);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        std::fs::remove_file(&path)?;
    } else {
        std::fs::remove_dir_all(&path)?;
    }
    tine_store::directory_durability::sync_directory_entry(parent)
}

/// True when `dir` is a real directory holding every [`PACKAGE_FILES`] entry
/// as a regular file (symlinks do not count).
fn package_has_required_shape(dir: &Path) -> std::io::Result<bool> {
    let regular = |path: &Path| match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata.file_type())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    if !regular(dir)?.is_some_and(|kind| kind.is_dir()) {
        return Ok(false);
    }
    for name in PACKAGE_FILES {
        if !regular(&dir.join(name))?.is_some_and(|kind| kind.is_file()) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Reclaim crash residue in one plugin store (master e6f131c09 semantics on
/// og's plain-`std::fs` store). In-scope threat: crash or power loss during an
/// install or uninstall. It removes every `.install-*` staging and
/// `.retired-*` retirement entry, every version directory that lacks a
/// required regular file (a crash inside an older build's `remove_dir_all`
/// uninstall leaves one, and it would otherwise block reinstalling that
/// version), and every id directory left empty. It never follows a symlinked
/// id directory. A missing store is already clean. Cost O(store entries).
fn recover_plugin_store_at(root: &Path) -> Result<(), String> {
    let _guard = lock_plugin_store();
    recover_plugin_store_locked(root).map_err(|error| error.to_string())
}

fn recover_plugin_store_locked(root: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let name = entry?.file_name();
        let text = name.to_string_lossy();
        if text.starts_with(INSTALL_PREFIX) || text.starts_with(RETIRED_PREFIX) {
            reclaim_entry(root, &name)?;
            continue;
        }
        let id_dir = root.join(&name);
        let metadata = std::fs::symlink_metadata(&id_dir)?;
        if text.starts_with('.') || metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        for version in std::fs::read_dir(&id_dir)? {
            let version = version?.file_name();
            if !package_has_required_shape(&id_dir.join(&version))? {
                reclaim_entry(&id_dir, &version)?;
            }
        }
        if std::fs::read_dir(&id_dir)?.next().is_none() {
            match std::fs::remove_dir(&id_dir) {
                Ok(()) => tine_store::directory_durability::sync_directory_entry(root)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

/// Store roots this process has already recovered.
static RECOVERED_PLUGIN_STORES: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Reclaim residue once per process per store root, at first use (master
/// 3f9dcdbdb). Recovering on every command would let one honest instance's
/// plugin listing delete another live instance's in-flight `.install-*`
/// staging (in-scope: honest concurrent instances). A previous process's
/// crash residue is still reclaimed at the next first use.
fn recover_plugin_store_once(root: &Path) -> Result<(), String> {
    let mut recovered = RECOVERED_PLUGIN_STORES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if recovered.iter().any(|known| known == root) {
        return Ok(());
    }
    recover_plugin_store_at(root)?;
    recovered.push(root.to_path_buf());
    Ok(())
}

/// The store root every plugin command uses: `root`, recovered once per process.
fn plugin_store_root(root: PathBuf) -> Result<PathBuf, String> {
    recover_plugin_store_once(&root)?;
    Ok(root)
}

/// `Some(true)` when `target` already holds exactly `files` as regular files,
/// `Some(false)` when it holds anything else, `None` when it is absent.
fn existing_package_exact(target: &Path, files: &[(&str, &[u8])]) -> std::io::Result<Option<bool>> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Ok(Some(false))
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    for (name, bytes) in files {
        let path = target.join(name);
        if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file()) {
            return Ok(Some(false));
        }
        if read_bounded(&path, bytes.len())?.as_deref() != Some(*bytes) {
            return Ok(Some(false));
        }
    }
    Ok(Some(true))
}

const IMMUTABLE_VERSION_COLLISION: &str =
    "that immutable plugin version is already installed with different bytes";

/// Publish one immutable plugin version without ever replacing one (og 15a K09,
/// I-2; no-clobber per master e6f131c09). Each file is fsynced inside a private
/// `.install-*` staging directory, which is moved to `<id>/<version>` with a
/// no-replace move, and the directories that changed are synced. A crash
/// leaves no package or a complete one; staging residue is reclaimed by
/// [`recover_plugin_store_at`]. Identical bytes already present are
/// `AlreadyPresentExact`. Refusal: different bytes under an installed version
/// are refused (in-scope threat: an honest concurrent instance or command
/// installing another build of the same immutable version); the first
/// complete winner is kept. On Windows directory syncs are no-ops and the move
/// is write-through.
fn publish_package(
    root: &Path,
    id: &str,
    version: &str,
    manifest: &[u8],
    wasm: &[u8],
) -> Result<PackagePublishOutcome, String> {
    let files: [(&str, &[u8]); 2] = [("manifest.json", manifest), ("plugin.wasm", wasm)];
    let exact = |target: &Path| match existing_package_exact(target, &files) {
        Ok(Some(true)) => Ok(PackagePublishOutcome::AlreadyPresentExact),
        Ok(_) => Err(IMMUTABLE_VERSION_COLLISION.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let sync = tine_store::directory_durability::sync_directory_entry;
    let target = package_dir(root, id, version)?;
    let _guard = lock_plugin_store();
    if std::fs::symlink_metadata(&target).is_ok() {
        return exact(&target);
    }
    let io = |error: std::io::Error| error.to_string();
    std::fs::create_dir_all(root).map_err(io)?;
    let id_dir = root.join(id);
    match std::fs::create_dir(&id_dir) {
        Ok(()) => sync(root).map_err(io)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    let id_meta = std::fs::symlink_metadata(&id_dir).map_err(io)?;
    if id_meta.file_type().is_symlink() || !id_meta.is_dir() {
        return Err("installed plugin directory is unsafe".to_string());
    }
    let (staging_name, staging) = unique_install_dir(root, id, version)?;
    let staged = files.iter().try_for_each(|(name, bytes)| {
        crate::device_io::atomic_write_new(&staging.join(name), bytes)
    });
    if let Err(error) = staged {
        let _ = reclaim_entry(root, staging_name.as_ref());
        return Err(error.to_string());
    }
    match crate::device_io::move_file_noreplace(&staging, &target) {
        Ok(()) => {
            sync(&id_dir).map_err(io)?; // `<version>` appeared
            sync(root).map_err(io)?; // the staging entry left
            Ok(PackagePublishOutcome::Published)
        }
        Err(error) => {
            let _ = reclaim_entry(root, staging_name.as_ref());
            match std::fs::symlink_metadata(&target) {
                Ok(_) => exact(&target),
                Err(_) => Err(error.to_string()),
            }
        }
    }
}

/// Create a private, never-before-used `.install-*` staging directory.
fn unique_install_dir(root: &Path, id: &str, version: &str) -> Result<(String, PathBuf), String> {
    for _ in 0..128 {
        let name = unique_transient_name(INSTALL_PREFIX, id, version);
        let candidate = root.join(&name);
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok((name, candidate)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not allocate a private plugin install directory".to_string())
}

/// Reads at most `max` bytes of a regular file; a larger file is `Ok(None)`
/// without reading past the limit, and a FIFO or device is refused without
/// waiting on a writer (I-22: the plugin directory is imported content).
fn read_bounded(path: &Path, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    crate::device_io::read_bounded(path, max as u64)
}

fn read_manifest_bounded(path: &Path) -> std::io::Result<Option<String>> {
    match read_bounded(path, MAX_MANIFEST_BYTES) {
        Ok(bytes) => Ok(bytes.and_then(|bytes| String::from_utf8(bytes).ok())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Decodes a sideloaded plugin entry, refusing an over-limit payload from its
/// encoded length before any decode allocation (I-22).
fn decode_plugin_entry(wasm_b64: &str) -> Result<Vec<u8>, String> {
    if wasm_b64.len() > MAX_WASM_BYTES.div_ceil(3) * 4 {
        return Err("plugin entry is too large".to_string());
    }
    let wasm = base64::engine::general_purpose::STANDARD
        .decode(wasm_b64)
        .map_err(|_| "plugin entry is not valid base64")?;
    if wasm.len() > MAX_WASM_BYTES {
        return Err("plugin entry is too large".to_string());
    }
    Ok(wasm)
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

/// App-owned plugin storage, recovered once per process (see
/// [`plugin_store_root`]).
fn plugins_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map(|dir| dir.join("plugins"))
        .map_err(|_| "no app-data dir".to_string())?;
    plugin_store_root(root)
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
    let manifest_json = read_manifest_bounded(&target.join("manifest.json"))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "installed plugin manifest is unreadable".to_string())?;
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
    uninstall_package_with(root, id, version, || Ok(()))
}

/// [`uninstall_package`] with a test hook after the retirement move. The
/// version leaves `<id>/<version>` in one no-replace move to a root
/// `.retired-*` name and only then is deleted, so a crash or power loss
/// mid-uninstall (in-scope) never leaves a half-removed package that could be
/// listed or block reinstalling; the residue is reclaimed by
/// [`recover_plugin_store_at`].
fn uninstall_package_with(
    root: &Path,
    id: &str,
    version: &str,
    after_move: impl FnOnce() -> std::io::Result<()>,
) -> Result<bool, String> {
    let sync = tine_store::directory_durability::sync_directory_entry;
    let _guard = lock_plugin_store();
    let (id_dir, target, _) = validate_uninstall_target(root, id, version)?;
    let mut retired = None;
    for _ in 0..128 {
        let name = unique_transient_name(RETIRED_PREFIX, id, version);
        match crate::device_io::move_file_noreplace(&target, &root.join(&name)) {
            Ok(()) => {
                retired = Some(name);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    let retired = retired.ok_or("could not allocate a private plugin retirement name")?;
    let io = |error: std::io::Error| error.to_string();
    after_move().map_err(io)?;
    sync(&id_dir).map_err(io)?;
    sync(root).map_err(io)?;
    reclaim_entry(root, retired.as_ref()).map_err(io)?;
    if std::fs::read_dir(&id_dir).map_err(io)?.next().is_none() {
        std::fs::remove_dir(&id_dir).map_err(io)?;
        sync(root).map_err(io)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Drop the settings an uninstall makes stale: the selected/enabled state when
/// this version is selected or is the last one, and the per-plugin settings
/// with the last version.
fn clear_uninstalled_plugin_settings(
    json: &mut serde_json::Value,
    id: &str,
    version: &str,
    last_version: bool,
) {
    let selected_version = json
        .get("plugin_states")
        .and_then(|states| states.get(id))
        .and_then(|state| state.get("version"))
        .and_then(|value| value.as_str());
    if last_version || selected_version == Some(version) {
        if let Some(states) = json
            .get_mut("plugin_states")
            .and_then(serde_json::Value::as_object_mut)
        {
            states.remove(id);
        }
    }
    if last_version {
        if let Some(root) = json.as_object_mut() {
            root.remove(&format!("plugin-settings:{id}"));
        }
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

/// The stored per-plugin enabled/selected map. A missing settings file is an
/// empty map; a file that exists but cannot be read or parsed is an ERROR, not
/// "everything defaults to disabled" (I-9): the caller surfaces it, and no writer
/// ever sees a partial view (writes go through `update_settings_strict_at`, which
/// refuses the same file). Entries this build cannot parse (a newer Tine's shape)
/// are skipped in the view and stay untouched in the file.
fn plugin_states_at(path: &Path) -> Result<std::collections::HashMap<String, PluginState>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(format!("device settings could not be read: {error}")),
    };
    if text.trim().is_empty() {
        return Ok(Default::default());
    }
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        format!(
            "device settings file {} is not valid JSON ({error}); plugin states are unavailable until it is repaired or removed",
            path.display()
        )
    })?;
    Ok(json
        .get("plugin_states")
        .and_then(serde_json::Value::as_object)
        .map(|states| {
            states
                .iter()
                .filter_map(|(id, value)| {
                    serde_json::from_value(value.clone())
                        .ok()
                        .map(|state| (id.clone(), state))
                })
                .collect()
        })
        .unwrap_or_default())
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
    let wasm = decode_plugin_entry(&wasm_b64)?;
    if !wasm.starts_with(b"\0asm\x01\0\0\0") {
        return Err("plugin entry is not WebAssembly".to_string());
    }
    let digest = sha256(&wasm);
    let root = plugins_dir(&app)?;
    publish_package(&root, &id, &version, manifest_json.as_bytes(), &wasm)?;
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
        clear_uninstalled_plugin_settings(json, &id, &version, last_version)
    })?;
    uninstall_package(&root, &id, &version)?;
    Ok(())
}

/// List valid packages under app-owned plugin storage, across all versions.
/// Reads and hashes each wasm (at most 8 MiB each) and a 64 KiB manifest;
/// malformed, missing or oversized packages are omitted. Storage/recovery
/// failures propagate; an unreadable or unparseable device
/// settings file is an error, never silently "all disabled". Cost O(installed wasm bytes).
#[tauri::command]
pub(crate) fn list_installed_plugins(
    app: tauri::AppHandle,
) -> Result<Vec<InstalledPlugin>, String> {
    let root = plugins_dir(&app)?;
    let states = match crate::settings::settings_path(&app) {
        Some(path) => plugin_states_at(&path)?,
        None => Default::default(),
    };
    list_installed_plugins_at(&root, &states).map_err(|error| error.to_string())
}

fn list_installed_plugins_at(
    root: &Path,
    states: &std::collections::HashMap<String, PluginState>,
) -> std::io::Result<Vec<InstalledPlugin>> {
    let mut installed = Vec::new();
    let ids = match std::fs::read_dir(root) {
        Ok(ids) => ids,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(installed),
        Err(error) => return Err(error),
    };
    for id_entry in ids {
        let id_entry = id_entry?;
        if !id_entry.metadata()?.is_dir() {
            continue;
        }
        let Some(id) = id_entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !safe_component(&id, true) {
            continue;
        }
        let versions = std::fs::read_dir(id_entry.path())?;
        for version_entry in versions {
            let version_entry = version_entry?;
            if !version_entry.metadata()?.is_dir() {
                continue;
            }
            let Some(version) = version_entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !safe_version(&version) {
                continue;
            }
            let Some(manifest_json) =
                read_manifest_bounded(&version_entry.path().join("manifest.json"))?
            else {
                continue;
            };
            if manifest_identity(&manifest_json).ok().as_ref()
                != Some(&(id.clone(), version.clone()))
            {
                continue;
            }
            let wasm = match read_bounded(&version_entry.path().join("plugin.wasm"), MAX_WASM_BYTES)
            {
                Ok(wasm) => wasm,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let Some(wasm) = wasm else {
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
    Ok(installed)
}

/// Read one installed wasm from app-owned plugin storage. Unsafe id/version,
/// missing, oversized (>8 MiB), unreadable or invalid wasm returns a string
/// error; this checks neither enabled state nor the package hash.
#[tauri::command]
pub(crate) fn read_plugin_entry(
    id: String,
    version: String,
    app: tauri::AppHandle,
) -> Result<tauri::ipc::Response, String> {
    let path = package_dir(&plugins_dir(&app)?, &id, &version)?.join("plugin.wasm");
    let bytes = read_bounded(&path, MAX_WASM_BYTES)
        .map_err(|e| e.to_string())?
        .ok_or("installed plugin entry is invalid")?;
    if !bytes.starts_with(b"\0asm\x01\0\0\0") {
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
#[path = "plugin_store_tests.rs"]
mod store_tests;

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
    fn plugin_install_publishes_a_complete_package_durably() {
        // og 15a K09 (I-2): the install path fsyncs each file and every
        // directory that gains or loses an entry, as `atomic_file` does
        // (directory syncs are no-ops on Windows), and publishes with a
        // no-replace move. Power loss cannot be simulated, so the fsync shape
        // is pinned at the source and the result checked behaviourally.
        let source = include_str!("plugins.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let install = &production[production.find("pub(crate) fn install_plugin(").unwrap()..];
        let install = &install[..install.find("\n}\n").unwrap()];
        assert!(
            install.contains("publish_package(") && !install.contains("std::fs::write("),
            "I-2: plugin install must publish through publish_package (fsynced files + directories)"
        );
        let publish = &production[production.find("fn publish_package(").unwrap()..];
        let publish = &publish[..publish.find("\n}\n").unwrap()];
        assert_eq!(
            publish
                .matches("crate::device_io::atomic_write_new(")
                .count(),
            1,
            "both package files go through the one fsyncing create-new writer"
        );
        assert!(
            publish.contains("sync_directory_entry")
                && publish.contains("sync(&id_dir)")
                && publish.contains("crate::device_io::move_file_noreplace(")
        );

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        let target = package_dir(&root, "dev.tine.demo", "1.0.0").unwrap();
        let manifest = test_manifest("dev.tine.demo", "1.0.0");
        assert_eq!(
            publish_package(
                &root,
                "dev.tine.demo",
                "1.0.0",
                manifest.as_bytes(),
                b"\0asm\x01\0\0\0",
            )
            .unwrap(),
            PackagePublishOutcome::Published
        );
        assert_eq!(
            std::fs::read_to_string(target.join("manifest.json")).unwrap(),
            manifest
        );
        assert_eq!(
            std::fs::read(target.join("plugin.wasm")).unwrap(),
            b"\0asm\x01\0\0\0"
        );
        let names = |dir: &Path| -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            names(&root),
            vec!["dev.tine.demo"],
            "no staging directory left behind"
        );
        assert_eq!(
            names(&target),
            vec!["manifest.json", "plugin.wasm"],
            "no temp file left behind"
        );
        assert!(
            publish_package(&root, "dev.tine.demo", "1.0.0", b"{}", b"x").is_err(),
            "an installed version is not replaced"
        );
        assert_eq!(names(&root), vec!["dev.tine.demo"]);
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
    fn storage_admission_is_distinct_from_manifest_syntax() {
        // Native path admission is not the frontend manifest schema: retain the
        // existing storage policy while packageIdentity.ts owns manifest syntax.
        assert!(safe_component("dev.tine.demo-", true));
        assert!(!safe_version("1.2.3-."));
        assert!(!safe_version("1.2.3-beta."));
        assert!(safe_version("1.2.3-beta.1"));
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
        )
        .unwrap();

        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(persisted["unrelated"]["keep"], true);
        assert_eq!(
            persisted[REGISTRY_CACHE_KEY],
            envelope(SIGNED_CONTROL_INDEX, SIGNED_CONTROL_SIGNATURE)
        );
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

        std::fs::write(&path, r#"{"plugin-registry-index":"index"}"#).unwrap();
        assert_eq!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Absent,
            "the retired settings-cache shape is disposable and must trigger a refetch"
        );
        std::fs::write(
            &path,
            r#"{"plugin-registry-index":"index","plugin-registry-signature":"sig"}"#,
        )
        .unwrap();
        assert_eq!(
            load_plugin_registry_cache_at(&path),
            PluginRegistryCacheLoad::Absent,
            "a complete retired pair is not migrated; the live registry refetches it"
        );
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
    fn invalid_or_unpublishable_registry_cache_never_replaces_last_good_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tine-settings.json");
        std::fs::write(&path, "{\n  \"keep\": true\n}\n").unwrap();
        let before = std::fs::read(&path).unwrap();

        assert!(store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            "invalid".to_string(),
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(store_plugin_registry_cache_at(
            &path,
            "x".repeat(MAX_REGISTRY_INDEX_BYTES + 1),
            SIGNED_CONTROL_SIGNATURE.to_string(),
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);

        std::fs::write(&path, "not-json\n").unwrap();
        let malformed = std::fs::read(&path).unwrap();
        assert!(store_plugin_registry_cache_at(
            &path,
            SIGNED_CONTROL_INDEX.to_string(),
            SIGNED_CONTROL_SIGNATURE.to_string(),
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

    #[test]
    fn oversized_plugin_entry_is_refused_before_decoding() {
        // og 15b K09 (I-22): the encoded length bounds the decode. A payload
        // past the limit whose bytes are not even base64 must be refused as
        // too large, proving no decode ran; one at the limit still decodes.
        let limit_b64 = MAX_WASM_BYTES.div_ceil(3) * 4;
        assert_eq!(
            decode_plugin_entry(&"!".repeat(limit_b64 + 4)).unwrap_err(),
            "plugin entry is too large"
        );
        let at_limit = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_WASM_BYTES]);
        assert_eq!(
            decode_plugin_entry(&at_limit).unwrap().len(),
            MAX_WASM_BYTES
        );
    }

    #[test]
    fn installed_package_reads_stop_at_the_size_limit() {
        // og 15b K09 (I-22): listing and entry reads never read an installed
        // file past its limit; an oversized plugin.wasm is not listed at all
        // (it was listed, and hashed whole, before), a package at the limit is.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let big = write_test_package(root, "dev.tine.big", "0.1.0");
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        wasm.resize(MAX_WASM_BYTES + 1, 0);
        std::fs::write(big.join("plugin.wasm"), &wasm).unwrap();
        let ok = write_test_package(root, "dev.tine.ok", "0.1.0");
        wasm.truncate(MAX_WASM_BYTES);
        std::fs::write(ok.join("plugin.wasm"), &wasm).unwrap();

        let listed = list_installed_plugins_at(root, &Default::default()).unwrap();
        let ids: Vec<_> = listed.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["dev.tine.ok"]);

        assert_eq!(
            read_bounded(&big.join("plugin.wasm"), MAX_WASM_BYTES).unwrap(),
            None
        );
        assert_eq!(
            read_bounded(&ok.join("plugin.wasm"), MAX_WASM_BYTES)
                .unwrap()
                .map(|b| b.len()),
            Some(MAX_WASM_BYTES)
        );
        std::fs::write(ok.join("manifest.json"), "x".repeat(MAX_MANIFEST_BYTES + 1)).unwrap();
        assert_eq!(
            read_manifest_bounded(&ok.join("manifest.json")).unwrap(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_where_a_plugin_manifest_belongs_is_refused_without_waiting() {
        // I-22 (imported plugin directory): a FIFO named manifest.json must not
        // block the listing on a writer that never comes.
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("manifest.json");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = fifo.clone();
        std::thread::spawn(move || tx.send(read_manifest_bounded(&reader).is_err()).unwrap());
        let refused = rx.recv_timeout(std::time::Duration::from_secs(2));
        if refused.is_err() {
            // Release the stuck reader so the test process can exit.
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
        }
        assert_eq!(refused, Ok(true), "FIFO manifest read waited for a writer");
    }
}

#[cfg(test)]
mod fail_read_tests {
    use super::*;
    #[test]
    fn fail_read_plugin_inventory_cannot_be_an_empty_success() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        std::fs::write(&root, "directory unavailable").unwrap();
        assert!(
            format!(
                "{:?}",
                list_installed_plugins_at(&root, &Default::default())
            )
            .contains("Err"),
            "I-2: a failed plugin inventory read cannot mean no plugins"
        );
    }
}
