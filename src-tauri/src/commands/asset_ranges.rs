//! Read-only PDF ranges through the same asset containment gate as read_asset.
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use tine_store::Store;

use super::{feature_asset_access_error, slot_for_context, GraphContext};

const RANGE_MAX_BYTES: u64 = 1_048_576;

#[derive(serde::Serialize)]
pub(crate) struct AssetReadInfo {
    length: u64,
    version: String,
}

fn info(meta: &Metadata, max_bytes: u64) -> Result<AssetReadInfo, String> {
    if !meta.is_file() {
        return Err("asset is not a regular file".into());
    }
    if meta.len() > max_bytes {
        return Err("asset-too-large".into());
    }
    let mut version = format!("{}:{:?}:{:?}", meta.len(), meta.modified(), meta.created());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        version.push_str(&format!(
            ":{}:{}:{}:{}",
            meta.dev(),
            meta.ino(),
            meta.ctime(),
            meta.ctime_nsec()
        ));
    }
    Ok(AssetReadInfo {
        length: meta.len(),
        version,
    })
}

fn open(store: &Store, name: &str, max_bytes: u64) -> Result<(File, AssetReadInfo), String> {
    // Preserves nested names and in-area final symlinks accepted by read_asset.
    // No raw path from the caller reaches File::open.
    let path = tine_graph_features::assets::path_for_os_handoff(store, name)
        .map_err(feature_asset_access_error)?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    Ok((file, info(&metadata, max_bytes)?))
}

fn read_range(
    store: &Store,
    name: &str,
    max_bytes: u64,
    begin: u64,
    end: u64,
    version: &str,
) -> Result<Vec<u8>, String> {
    if end <= begin || end - begin > RANGE_MAX_BYTES {
        return Err("invalid asset byte range".into());
    }
    let (mut file, before) = open(store, name, max_bytes)?;
    if before.version != version {
        return Err("PDF asset changed while open; close and reopen it".into());
    }
    if end > before.length {
        return Err("invalid asset byte range".into());
    }
    file.seek(SeekFrom::Start(begin))
        .map_err(|error| error.to_string())?;
    let mut bytes = vec![0; (end - begin) as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    let after = info(
        &file.metadata().map_err(|error| error.to_string())?,
        max_bytes,
    )?;
    if before.version != after.version {
        return Err("PDF asset changed during a byte range read".into());
    }
    Ok(bytes)
}

/// Metadata only: no graph load wait, hashing or content read.
#[tauri::command]
pub(crate) async fn stat_asset(
    name: String,
    max_bytes: u64,
    state: GraphContext<'_>,
) -> Result<AssetReadInfo, String> {
    let slot = slot_for_context(&state)?;
    crate::state::off_ui(move || open(&slot.store, &name, max_bytes).map(|(_, metadata)| metadata))
        .await
}

/// One bounded binary IPC response, tied by GraphContext to the captured binding.
#[tauri::command]
pub(crate) async fn read_asset_range(
    name: String,
    max_bytes: u64,
    begin: u64,
    end: u64,
    version: String,
    state: GraphContext<'_>,
) -> Result<tauri::ipc::Response, String> {
    let slot = slot_for_context(&state)?;
    crate::state::off_ui(move || {
        read_range(&slot.store, &name, max_bytes, begin, end, &version)
            .map(tauri::ipc::Response::new)
    })
    .await
}

#[cfg(test)]
#[path = "asset_ranges_tests.rs"]
mod tests;
