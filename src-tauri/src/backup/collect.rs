//! The schema-4 blob collector (og-backup-cas D3): the only code that
//! removes blobs. It runs under the namespace lock, so every unpublished
//! snapshot directory is abandoned and no writer is active (A-bk1).
//!
//! Mark, then sweep. The mark reads every manifest in `snapshots/`
//! independently of restore eligibility and is complete before anything is
//! removed; any listing, entry or manifest read error, an entry this module
//! did not create, or a manifest whose references cannot be read returns
//! `Err` and removes nothing. The sweep lists the whole blob store before it
//! removes an unreferenced blob; a removal that fails stays in the report and
//! the next collection retries it. Collection is opportunistic: a leaked
//! blob waits for the next collection, which follows the next crashed
//! partial, failed snapshot or deletion. Its refusal scenario is the
//! `src-tauri::backup::collect` row of docs/storage-contract.md's I-8 table.

use super::{
    is_digest, manifest_checksum, record_backup_op, BLOB_DIR, CAS_SNAPSHOTS, MANIFEST_CHECKSUM,
    SNAPSHOT_MANIFEST, SNAPSHOT_SCHEMA,
};
use std::collections::HashSet;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(super) struct CollectReport {
    /// Unreferenced blobs and abandoned blob temps removed.
    pub(super) removed: usize,
    /// Removals that failed, retried by the next collection.
    pub(super) failed: Vec<(PathBuf, ErrorKind)>,
    /// Snapshots whose manifest is damaged or missing, and what happened.
    pub(super) damaged: Vec<String>,
}

/// What one published snapshot's manifest contributes to the mark.
enum Mark {
    /// Every blob it lists, and whether it is a valid schema-4 manifest.
    Refs(Vec<String>, bool),
    /// Not JSON: a torn manifest that can never restore and lists nothing.
    Torn,
}

pub(super) fn collect(cas: &Path) -> io::Result<CollectReport> {
    let mut report = CollectReport::default();
    let mut live = HashSet::new();
    let mut doomed = Vec::new();
    for entry in listing(&cas.join(CAS_SNAPSHOTS))? {
        let entry = entry?;
        let name = entry_name(&entry)?;
        if !entry.file_type()?.is_dir() {
            return Err(unexpected(&entry.path()));
        }
        if name.starts_with(".partial-") {
            // Unpublished, and abandoned under the lock: it lists nothing
            // that stays.
            doomed.push(entry.path());
            continue;
        }
        match std::fs::read(entry.path().join(SNAPSHOT_MANIFEST)) {
            Ok(bytes) => match mark(&bytes)? {
                Mark::Refs(refs, valid) => {
                    if !valid {
                        report
                            .damaged
                            .push(format!("{name}: unverifiable manifest, blobs kept"));
                    }
                    live.extend(refs);
                }
                Mark::Torn => {
                    report
                        .damaged
                        .push(format!("{name}: torn manifest, removed"));
                    doomed.push(entry.path());
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {
                report.damaged.push(format!("{name}: no manifest, removed"));
                doomed.push(entry.path());
            }
            Err(error) => return Err(error),
        }
    }
    let mut unreferenced = Vec::new();
    record_backup_op("blob_dir_scan");
    for entry in listing(&cas.join(BLOB_DIR))? {
        let entry = entry?;
        let name = entry_name(&entry)?;
        if !entry.file_type()?.is_file() {
            return Err(unexpected(&entry.path()));
        }
        if is_digest(&name) {
            if !live.contains(&name) {
                unreferenced.push(entry.path());
            }
        } else if name.starts_with(".tmp-") {
            // `put_blob`'s temp, abandoned under the lock.
            unreferenced.push(entry.path());
        } else {
            return Err(unexpected(&entry.path()));
        }
    }
    for dir in doomed {
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            report.failed.push((dir, error.kind()));
        }
    }
    for blob in unreferenced {
        match std::fs::remove_file(&blob) {
            Ok(()) => {
                report.removed += 1;
                record_backup_op("blob_collect");
            }
            Err(error) => report.failed.push((blob, error.kind())),
        }
    }
    Ok(report)
}

/// Classify a manifest's bytes for the mark (og-backup-cas D3 amendment):
/// JSON with a `files: [{sha256: <digest>}]` list keeps those references
/// whatever its schema, checksum or completeness; JSON whose references
/// cannot be read stops the collection.
fn mark(bytes: &[u8]) -> io::Result<Mark> {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(Mark::Torn);
    };
    let fields = value
        .as_object_mut()
        .ok_or_else(|| uninterpretable("manifest is not an object"))?;
    let checksum = fields
        .remove_entry(MANIFEST_CHECKSUM)
        .map(|(_, value)| value);
    let checksum_ok =
        checksum.as_ref().and_then(serde_json::Value::as_str) == Some(&manifest_checksum(&value));
    let valid = checksum_ok
        && value.get("schema").and_then(serde_json::Value::as_u64)
            == Some(u64::from(SNAPSHOT_SCHEMA));
    let refs = value
        .get("files")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| uninterpretable("manifest has no files list"))?
        .iter()
        .map(|file| {
            file.get("sha256")
                .and_then(serde_json::Value::as_str)
                .filter(|sha256| is_digest(sha256))
                .map(str::to_owned)
                .ok_or_else(|| uninterpretable("manifest lists a file without a blob digest"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(Mark::Refs(refs, valid))
}

/// A directory's entries, every item's error kept. Tests inject one.
fn listing(dir: &Path) -> io::Result<Vec<io::Result<std::fs::DirEntry>>> {
    #[allow(unused_mut)]
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect();
    #[cfg(test)]
    if FAIL_LISTING.with(|fail| fail.borrow().as_deref() == Some(dir)) {
        entries.insert(0, Err(io::Error::other("injected listing entry failure")));
    }
    Ok(entries)
}

/// An entry's name; no name this module creates is anything but UTF-8.
fn entry_name(entry: &std::fs::DirEntry) -> io::Result<String> {
    entry
        .file_name()
        .into_string()
        .map_err(|_| unexpected(&entry.path()))
}

fn unexpected(path: &Path) -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        format!("unexpected backup namespace entry {}", path.display()),
    )
}

fn uninterpretable(what: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, what.to_owned())
}

#[cfg(test)]
std::thread_local! {
    /// The directory whose listing yields an error item on this thread.
    pub(super) static FAIL_LISTING: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}
