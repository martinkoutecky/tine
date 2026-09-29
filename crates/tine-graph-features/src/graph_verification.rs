//! Exact-byte fingerprint of a graph's Markdown and Org sources, so two devices
//! that should hold the same graph can be compared file by file.
//!
//! **Question answered.** [`verify_graph_bytes`] — what is the SHA-256 and byte
//! length of every user-visible graph text file? The file set is the Store's
//! `Area::Graph` listing (the same answer backup uses); nothing is parsed, so a
//! file the parser rejects is still hashed.
//!
//! **Cost.** O(files) listing plus O(total bytes) streamed in 64 KiB chunks;
//! memory O(files). Read-only: no write, lock or edit kind. The report is a
//! transient value; only a user-chosen Save writes it. Unit cost: none (no
//! persisted record, index or transport record).
//!
//! **Failure modes (threat: external editor or sync delivery racing the scan).**
//! A file whose length or modification time moved while it was read, an
//! unreadable file, or a file set that changed between the first and the final
//! listing is recorded in `errors` and makes the manifest incomplete. An
//! incomplete manifest never confirms a match. Cancellation returns
//! [`Cancelled`] and produces no manifest.
//!
//! The manifest format (`tine-graph-bytes`, schema 1) is byte-compatible with
//! the reports Tine 0.6.984+ writes, so devices on either build can compare.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};
use tine_store::{Area, Store};

const TOOL: &str = "tine-graph-bytes";
const ALGORITHM: &str = "sha256";
const SCHEMA_VERSION: u32 = 1;

/// Digest of one exact graph source file; `path` is graph-relative, `/`-separated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDigest {
    /// Graph-relative path.
    pub path: String,
    /// Exact byte length.
    pub length: u64,
    /// Lowercase hex SHA-256 of the bytes.
    pub digest: String,
}

/// One reason the manifest is incomplete.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationError {
    /// The file concerned, when one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A short description.
    pub detail: String,
}

/// The comparable record of one graph's source bytes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    schema_version: u32,
    tool: String,
    algorithm: String,
    /// True only when every listed file was hashed stably and the file set did
    /// not change during the run.
    pub complete: bool,
    /// Creation time, milliseconds since the Unix epoch.
    pub generated_at_unix_ms: u128,
    /// Digests, sorted by path.
    pub files: Vec<SourceDigest>,
    /// Digest over the ordered file list; present only when `complete`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_digest: Option<String>,
    /// Why the manifest is incomplete; empty when it is complete.
    pub errors: Vec<VerificationError>,
}

/// The verification was cancelled by its caller.
#[derive(Debug, Eq, PartialEq)]
pub struct Cancelled;

impl Manifest {
    /// Parse and check a report's identity fields (tool, algorithm, schema).
    pub fn from_report(text: &str) -> Result<Self, String> {
        let manifest: Manifest = serde_json::from_str(text)
            .map_err(|error| format!("graph verification report is invalid: {error}"))?;
        if manifest.schema_version != SCHEMA_VERSION
            || manifest.tool != TOOL
            || manifest.algorithm != ALGORITHM
        {
            return Err("graph verification report has an unsupported format".into());
        }
        Ok(manifest)
    }

    /// Pretty JSON, the exact text a user copies or saves.
    pub fn to_report(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Total bytes over the hashed files.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.length).sum()
    }
}

fn aggregate_digest(files: &[SourceDigest]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tine-graph-bytes-v1\0");
    for file in files {
        hasher.update((file.path.len() as u64).to_be_bytes());
        hasher.update(file.path.as_bytes());
        hasher.update(file.length.to_be_bytes());
        hasher.update(file.digest.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn list_paths(store: &Store, errors: &mut Vec<VerificationError>, label: &str) -> Vec<String> {
    let mut paths = match store.scan_area(Area::Graph, None) {
        Ok(listing) => {
            for (name, error) in &listing.unreadable {
                errors.push(VerificationError {
                    path: (!name.is_empty()).then(|| name.clone()),
                    detail: format!("{label} could not list an entry: {error}"),
                });
            }
            listing.files.into_iter().map(|entry| entry.rel).collect()
        }
        Err(error) => {
            errors.push(VerificationError {
                path: None,
                detail: format!("{label} failed: {}", crate::store_error(error)),
            });
            Vec::new()
        }
    };
    paths.sort();
    paths
}

/// Hash one listed file. `Ok(Err(..))` is a per-file problem; `Err(Cancelled)`
/// aborts the run.
fn digest_file(
    store: &Store,
    rel: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Result<SourceDigest, String>, Cancelled> {
    let attempt = (|| -> Result<Option<SourceDigest>, String> {
        let id = store
            .file_id(Area::Graph, rel)
            .map_err(|e| crate::store_error(e).to_string())?;
        let (mut file, length) = store
            .open_read(&id)
            .map_err(|e| crate::store_error(e).to_string())?;
        let before = file.metadata().and_then(|m| m.modified()).ok();
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut read_total = 0_u64;
        loop {
            if cancelled() {
                return Ok(None);
            }
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            read_total += n as u64;
        }
        let after = file.metadata().map_err(|e| e.to_string())?;
        if read_total != length || after.len() != length || after.modified().ok() != before {
            return Err("changed while it was being verified".into());
        }
        Ok(Some(SourceDigest {
            path: rel.to_owned(),
            length,
            digest: format!("{:x}", hasher.finalize()),
        }))
    })();
    match attempt {
        Ok(Some(digest)) => Ok(Ok(digest)),
        Ok(None) => Err(Cancelled),
        Err(detail) => Ok(Err(detail)),
    }
}

/// Fingerprint every graph text file. `progress(done, total)` is called before
/// hashing starts (`0, total`) and after each file; `cancelled` is polled
/// between chunks. Cost O(total bytes), streamed.
pub fn verify_graph_bytes(
    store: &Store,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Manifest, Cancelled> {
    let mut errors = Vec::new();
    let paths = list_paths(store, &mut errors, "source inventory");
    let total = paths.len();
    progress(0, total);
    let mut files = Vec::with_capacity(total);
    for (index, rel) in paths.iter().enumerate() {
        if cancelled() {
            return Err(Cancelled);
        }
        match digest_file(store, rel, cancelled)? {
            Ok(digest) => files.push(digest),
            Err(detail) => errors.push(VerificationError {
                path: Some(rel.clone()),
                detail,
            }),
        }
        progress(index + 1, total);
    }
    let mut after_errors = Vec::new();
    let after = list_paths(store, &mut after_errors, "final source inventory");
    errors.extend(after_errors);
    if after != paths {
        errors.push(VerificationError {
            path: None,
            detail: "source inventory changed while verification was running".into(),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let complete = errors.is_empty() && files.len() == total;
    Ok(Manifest {
        schema_version: SCHEMA_VERSION,
        tool: TOOL.into(),
        algorithm: ALGORITHM.into(),
        complete,
        generated_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        aggregate_digest: complete.then(|| aggregate_digest(&files)),
        files,
        errors,
    })
}
