//! Asset imports, orphan discovery, and recoverable trash. Reads touch the
//! named file or O(entries + B) for orphan discovery; writes use transactions.

use std::io;
use std::time::UNIX_EPOCH;
use tine_core::model::{AssetInfo, TrashStats};
use tine_store::{Area, Content, StepResult, Store, StoreError, TrashKind};

use crate::{is_conflict, store_error, tx_error};

const COMPOUND_EXTS: &[&str] = &[".drawio.svg", ".excalidraw.svg", ".excalidraw.png"];

/// Keep the legacy trash error display path while the store owns its layout.
/// Cost O(error text); the original I/O failure remains visible.
pub fn error_for_user(store: &Store, error: io::Error) -> String {
    error.to_string().replace(
        "logseq/.tine-trash/assets",
        &store.asset_trash_location_for_user().display().to_string(),
    )
}

fn split_name(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    for ext in COMPOUND_EXTS {
        if lower.ends_with(ext) {
            return name.split_at(name.len() - ext.len());
        }
    }
    match name.rfind('.') {
        Some(index) => name.split_at(index),
        None => (name, ""),
    }
}

fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad asset name",
        ))
    } else {
        Ok(())
    }
}

/// A top-level asset request failed before or during store access.
#[derive(Debug)]
pub enum AssetAccessError {
    BadName,
    Store(StoreError),
    StreamSymlink,
}

fn named_asset(store: &Store, name: &str) -> Result<tine_store::FileId, AssetAccessError> {
    if validate_name(name).is_err() {
        return Err(AssetAccessError::BadName);
    }
    store
        .file_id(Area::Assets, name)
        .map_err(AssetAccessError::Store)
}

/// Read one top-level asset into bytes. Cost O(file bytes).
pub fn read_asset(
    store: &Store,
    name: &str,
    max_bytes: Option<u64>,
) -> Result<Vec<u8>, AssetAccessError> {
    let id = named_asset(store, name)?;
    store
        .read(&id, max_bytes)
        .map(|(bytes, _)| bytes)
        .map_err(AssetAccessError::Store)
}

/// Validate a top-level asset for the range-aware media protocol. Cost O(path components).
pub fn validate_stream_asset(store: &Store, name: &str) -> Result<(), AssetAccessError> {
    let id = named_asset(store, name)?;
    store
        .open_read(&id)
        .map(|_| ())
        .map_err(|error| match error {
            StoreError::InvalidTarget(reason) if reason.starts_with("symlink:") => {
                AssetAccessError::StreamSymlink
            }
            other => AssetAccessError::Store(other),
        })
}

/// Return an existing top-level asset path for an OS opener. Cost O(path components).
pub fn path_for_os_handoff(
    store: &Store,
    name: &str,
) -> Result<std::path::PathBuf, AssetAccessError> {
    let id = named_asset(store, name)?;
    store
        .path_for_os_handoff(&id, true)
        .map_err(AssetAccessError::Store)
}

/// A device import failed during filename selection or streaming.
/// Choose and validate an import name from an explicit name or the device
/// source's final component. No path is opened. Cost O(name bytes).
pub fn choose_import_name(
    source_filename: Option<&str>,
    name: Option<&str>,
) -> Result<String, String> {
    let chosen = name
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| source_filename.map(str::to_owned))
        .ok_or_else(|| "bad source filename".to_string())?;
    validate_name(&chosen).map_err(|_| "bad asset name".to_string())?;
    Ok(chosen)
}

/// Summarize all recoverable trash categories. Cost O(trash entries).
pub fn asset_trash_stats(store: &Store) -> Result<TrashStats, StoreError> {
    let mut stats = TrashStats::default();
    for (kind, count, bytes) in store.trash_stats()? {
        match kind {
            TrashKind::Asset => {
                stats.count = count;
                stats.bytes = bytes;
            }
            TrashKind::Page => stats.pages = count,
            TrashKind::Journal => stats.journals = count,
            TrashKind::Conflict => stats.conflicts = count,
            TrashKind::Legacy => stats.other = count,
        }
    }
    Ok(stats)
}

fn create_unique(store: &Store, name: &str, content: Content) -> io::Result<String> {
    validate_name(name)?;
    let (stem, ext) = split_name(name);
    let mut tx = store.transaction();
    tx.create_unique(Area::Assets, stem, ext, content);
    let steps = tx_error(tx.commit())?;
    match &steps[0] {
        StepResult::Written { file, .. } => Ok(file
            .as_str()
            .strip_prefix("assets/")
            .unwrap_or(file.as_str())
            .to_owned()),
        _ => unreachable!("create_unique result"),
    }
}

/// Save bytes under a unique asset name. Cost O(bytes + collision candidates).
pub fn save_asset(store: &Store, name: &str, bytes: &[u8]) -> io::Result<String> {
    create_unique(store, name, Content::Bytes(bytes.to_vec()))
}

/// Import an already opened source stream under a unique name. The caller may
/// pass `u64::MAX` for the v0.6.5 unlimited import. Cost O(source bytes +
/// collision candidates); failures leave no committed asset.
pub fn import_asset(store: &Store, name: &str, source: Content) -> io::Result<String> {
    create_unique(store, name, source)
}

/// Native capture import with a caller-supplied byte cap. Cost O(source bytes +
/// collision candidates); the stream is rewound by the transaction.
pub fn import_asset_file(store: &Store, name: &str, source: Content) -> io::Result<String> {
    create_unique(store, name, source)
}

/// List top-level unreferenced media; unreadable scan entries are skipped as in
/// v0.6.5. Cost O(asset entries + B), including the referenced-asset walk.
pub fn orphan_assets(store: &Store) -> Vec<AssetInfo> {
    let Ok(listing) = store.scan_area(Area::Assets, None) else {
        return Vec::new();
    };
    let Ok(graph) = store.whole_graph() else {
        return Vec::new();
    };
    let referenced = graph.referenced_assets();
    listing
        .files
        .into_iter()
        .filter_map(|entry| {
            let name = entry.rel;
            if name.contains('/')
                || name.starts_with('.')
                || name.ends_with(".edn")
                || referenced.contains(&name)
            {
                return None;
            }
            let meta = entry.meta?;
            Some(AssetInfo {
                name,
                size: meta.len,
                modified: meta
                    .mtime
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            })
        })
        .collect()
}

/// Move one top-level asset into recoverable trash. Reads its current revision
/// and retries a concurrent external write at most four times. Cost O(file bytes)
/// per attempt; a missing asset reports the v0.6.5 `no such asset` error.
pub fn trash_asset(store: &Store, name: &str) -> io::Result<()> {
    validate_name(name)?;
    let id = store.file_id(Area::Assets, name).map_err(store_error)?;
    for _ in 0..4 {
        let rev = match store.read(&id, None) {
            Ok((_, rev)) => rev,
            Err(StoreError::NotFound) => {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such asset"))
            }
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.transaction();
        tx.trash(&id, rev);
        let outcome = tx.commit();
        if is_conflict(&outcome) {
            continue;
        }
        tx_error(outcome).map_err(|error| {
            if error.kind() == io::ErrorKind::NotADirectory {
                let message = error.to_string();
                let cause = message.rsplit(": ").next().unwrap_or(&message);
                io::Error::new(
                    error.kind(),
                    format!("could not create trash directory logseq/.tine-trash/assets: {cause}"),
                )
            } else {
                error
            }
        })?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "asset changed repeatedly during trash",
    ))
}
