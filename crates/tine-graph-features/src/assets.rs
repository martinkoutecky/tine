//! Asset imports, reads, orphan discovery, and recoverable trash. Read/open
//! requests take an `assets/`-relative name, including nested paths, and use
//! Store containment checks; callers need not resolve symlinks or graph paths.
//! Bad names, escaped paths, absent files, and I/O failures are errors. Reads
//! cost O(path components + file bytes); open validation/handoff costs O(path
//! components). Orphan discovery costs O(entries + referenced blocks); writes
//! use transactions. Imports and explicit trash still take top-level names.

use std::io;
use std::time::UNIX_EPOCH;
use tine_core::model::{AssetInfo, TrashStats};
use tine_store::{Area, Content, StepResult, Store, StoreError, TrashKind};

use crate::{store_error, tx_error};

const COMPOUND_EXTS: &[&str] = &[".drawio.svg", ".excalidraw.svg", ".excalidraw.png"];

/// Keep the legacy trash error display path while the store owns its layout.
/// Cost O(error text); the original I/O failure remains visible.
pub fn error_for_user(store: &Store, error: io::Error) -> String {
    error.to_string().replace(
        "logseq/.tine-trash/assets",
        &store.asset_trash_location_for_user().display().to_string(),
    )
}

/// How a [`trash_asset`] that did not fail ended (GH #623). `Referenced` is a
/// normal outcome, not an error: the published graph still uses the file, so
/// it was kept. Callers match this value, never message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashOutcome {
    /// The file moved to the recoverable trash.
    Trashed,
    /// Another page still references the file; it was left in `assets/`.
    Referenced,
}

/// Typed source of the `io::Error` that the store refusal becomes inside the
/// transaction mapping; `trash_asset` turns it into [`TrashOutcome::Referenced`].
#[derive(Debug)]
pub(crate) struct AssetReferenced;
impl std::fmt::Display for AssetReferenced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("asset is still referenced")
    }
}
impl std::error::Error for AssetReferenced {}

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

fn validate_read_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !(name.as_bytes().len() >= 2
            && name.as_bytes()[0].is_ascii_alphabetic()
            && name.as_bytes()[1] == b':')
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && std::path::Path::new(name)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

/// An asset request failed before or during store access.
#[derive(Debug)]
pub enum AssetAccessError {
    BadName,
    Store(StoreError),
    StreamSymlink,
}

fn named_asset(store: &Store, name: &str) -> Result<tine_store::FileId, AssetAccessError> {
    if !validate_read_name(name) {
        return Err(AssetAccessError::BadName);
    }
    store
        .file_id(Area::Assets, name)
        .map_err(AssetAccessError::Store)
}

/// Read one asset into bytes, optionally bounded by `max_bytes`. An in-area
/// final symlink may resolve; an escape is refused. Cost O(path components + bytes).
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

/// Open/validate an asset for range-aware media without reading its bytes.
/// Refuses a final symlink even inside assets. Cost O(path components).
pub fn validate_stream_asset(store: &Store, name: &str) -> Result<(), AssetAccessError> {
    let id = named_asset(store, name)?;
    store
        .open_read(&id)
        .map(|_| ())
        .map_err(|error| match error {
            StoreError::StreamSymlink(_) => AssetAccessError::StreamSymlink,
            other => AssetAccessError::Store(other),
        })
}

/// Return the canonical path of an existing regular asset for an OS opener.
/// An in-area symlink may resolve. Cost O(path components).
pub fn path_for_os_handoff(
    store: &Store,
    name: &str,
) -> Result<std::path::PathBuf, AssetAccessError> {
    let id = named_asset(store, name)?;
    store
        .path_for_os_handoff(&id, true)
        .map_err(AssetAccessError::Store)
}

/// Probe child used to locate the assets root through the Store's containment
/// gate; it need not (and normally does not) exist.
const ASSETS_ROOT_PROBE: &str = ".tine-assets-root-probe";

/// Return the canonical path of an existing asset file OR directory for an OS
/// opener; the empty name is the assets root (GH #367, OG's `[p](./assets/)`).
/// Same name validation and Store containment as `path_for_os_handoff` (the
/// live `assets/` must be the approved root; the resolved target may not
/// escape it through a symlink), which keeps its regular-file gate for edit
/// handoffs. Refuses a name that exists as neither file nor directory
/// (deleted or replaced by sync or an external editor after the click; I-8 row
/// "`tine-store::store` read, scan and handoff"). Cost O(path components).
pub fn path_for_os_open(store: &Store, name: &str) -> Result<std::path::PathBuf, AssetAccessError> {
    let root = name.is_empty();
    let id = named_asset(store, if root { ASSETS_ROOT_PROBE } else { name })?;
    let path = store
        .path_for_os_handoff(&id, false)
        .map_err(AssetAccessError::Store)?;
    let target = if root {
        path.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or(path)
    } else {
        path
    };
    if target.is_file() || target.is_dir() {
        Ok(target)
    } else {
        Err(AssetAccessError::Store(StoreError::InvalidTarget(format!(
            "assets/{name}"
        ))))
    }
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
    let mut tx = store.transaction(None);
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

/// List top-level unreferenced media. A failed inventory or graph read is an
/// error, including unreadable entries: a partial scan cannot establish absence
/// (disk/permission failure). Cost O(asset entries + B); no writes.
pub fn orphan_assets(store: &Store) -> io::Result<Vec<AssetInfo>> {
    let listing = store.scan_area(Area::Assets, None).map_err(store_error)?;
    if let Some((name, error)) = listing.unreadable.into_iter().next() {
        return Err(io::Error::new(
            error.kind,
            format!("assets/{name}: {}", error.message),
        ));
    }
    let graph = store.whole_graph().map_err(|error| match error {
        tine_store::LoadError::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
        tine_store::LoadError::Failed { reason } => io::Error::other(reason),
    })?;
    let referenced = graph.referenced_assets();
    Ok(listing
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
        .collect())
}

/// Move one top-level asset into recoverable trash. Reads its current revision
/// and retries a concurrent external write at most four times. The transaction
/// rechecks the latest published asset references under its writer lock: an
/// asset the graph still references stays put and reports
/// [`TrashOutcome::Referenced`]. Unreadable graph entries refuse trash; an
/// external arrival not yet published can still race. Cost O(B + file bytes)
/// per attempt; a missing asset reports the v0.6.5 `no such asset` error.
pub fn trash_asset(store: &Store, name: &str) -> io::Result<TrashOutcome> {
    validate_name(name)?;
    let id = store.file_id(Area::Assets, name).map_err(store_error)?;
    let moved = crate::retry_on_conflict("asset changed repeatedly during trash", || {
        let rev = match store.read(&id, None) {
            Ok((_, rev)) => rev,
            Err(StoreError::NotFound) => {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such asset"))
            }
            Err(error) => return Err(store_error(error)),
        };
        let mut tx = store.transaction(None);
        tx.trash_orphan_asset(&id, rev);
        Ok(crate::commit_retry(tx.commit())?.then_some(()))
    });
    match moved {
        Ok(()) => Ok(TrashOutcome::Trashed),
        Err(error)
            if error
                .get_ref()
                .is_some_and(|source| source.is::<AssetReferenced>()) =>
        {
            Ok(TrashOutcome::Referenced)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
