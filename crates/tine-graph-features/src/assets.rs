//! Asset imports, orphan discovery, and recoverable trash. Reads touch the
//! named file or O(entries + B) for orphan discovery; writes use transactions.

use std::io;
use std::time::UNIX_EPOCH;
use tine_core::model::AssetInfo;
use tine_store::{Area, Content, StepResult, Store, StoreError};

use crate::{is_conflict, store_error, tx_error};

const COMPOUND_EXTS: &[&str] = &[".drawio.svg", ".excalidraw.svg", ".excalidraw.png"];

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
