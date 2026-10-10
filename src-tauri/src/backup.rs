use crate::settings::{settings_path, update_settings};
use crate::state::{slot_for_context, GraphContext, GraphSlot};
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Manager;
use tine_store::{is_asset_sidecar, is_graph_text, Area, RestoreFile, Store};

mod collect;
mod naming;
mod restore;
use naming::{highest_counter, snapshot_order};
pub(crate) use restore::restore_backup;

// Snapshot the graph's Markdown/Org into the OS app-data dir on open, keeping the
// last few. Local-only (outside the graph, so Syncthing never sees it); a safety
// net against a bad write or accidental edit. Source validation runs at launch;
// the file copy runs in a detached best-effort worker.
const BACKUP_KEEP_DEFAULT: usize = 12;
static BACKUP_WORK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

/// The process-wide backup permit: one whole-graph copy at a time across
/// every open graph. It is a throttle, not the backup protocol's exclusion;
/// that is the cross-process `lock_namespace` (REVIEW-backup-cas B2). Lock order:
/// this permit, then a namespace's lock.
fn backup_work() -> std::sync::MutexGuard<'static, ()> {
    BACKUP_WORK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Debug)]
pub(crate) struct BackupFailure {
    phase: &'static str,
    kind: ErrorKind,
}

impl BackupFailure {
    fn wire(&self) -> String {
        format!("backup-failed:{}:{:?}", self.phase, self.kind)
    }
}

#[derive(Debug, Default)]
pub(crate) struct BackupOutcome {
    pub(crate) copied: usize,
    pub(crate) failure: Option<BackupFailure>,
    /// The published snapshot's name.
    published: Option<String>,
}

impl BackupOutcome {
    pub(crate) fn success(copied: usize) -> Self {
        Self {
            copied,
            ..Self::default()
        }
    }

    pub(crate) fn failed(copied: usize, phase: &'static str, kind: ErrorKind) -> Self {
        Self {
            copied,
            failure: Some(BackupFailure { phase, kind }),
            published: None,
        }
    }
}

fn launch_failure_token(outcome: &BackupOutcome) -> Option<String> {
    outcome
        .failure
        .as_ref()
        .filter(|failure| failure.phase != "cancelled")
        .map(BackupFailure::wire)
}

pub(crate) fn report_launch_outcome(outcome: &BackupOutcome) {
    if let Some(token) = launch_failure_token(outcome) {
        eprintln!("[tine] {token}");
        crate::debug::diag_private("backup-failed", token);
    }
}
// The event carries only fixed phase/kind and the binding that owns the failure.
// A detached backup must not show feedback in a replacement graph window.
fn report_launch_failure(app: &tauri::AppHandle, slot: &GraphSlot, outcome: &BackupOutcome) {
    use tauri::Emitter;
    report_launch_outcome(outcome);
    if let Some(failure) = launch_failure_token(outcome) {
        if let Err(error) = app.emit(
            "backup-failed",
            serde_json::json!({
                "bindingGeneration": slot.binding_generation, "failure": failure
            }),
        ) {
            crate::debug::diag_private("backup-feedback-failed", error.to_string());
        }
    }
}

#[cfg(test)]
const ASSET_RESTORE_RECOVERY_DIR: &str = ".tine-restore-recovery";

// Master GH #550 policy, driven by the owning warm/cancellation signals.
const LAUNCH_BACKUP_QUIET: std::time::Duration = std::time::Duration::from_secs(5);
const LAUNCH_BACKUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(180);

fn wait_launch_backup(slot: &GraphSlot) -> bool {
    slot.wait_startup_idle(LAUNCH_BACKUP_QUIET, LAUNCH_BACKUP_DEADLINE)
}

pub(crate) fn backup_async(app: tauri::AppHandle, slot: Arc<GraphSlot>) {
    let source = match BackupSource::from_store(&slot.store, &slot.root_key) {
        Ok(source) => source,
        Err((kind, _)) => {
            report_launch_failure(&app, &slot, &BackupOutcome::failed(0, "source", kind));
            return;
        }
    };
    std::thread::spawn(move || {
        if !wait_launch_backup(&slot) {
            return;
        }
        // Bound whole-graph copying process-wide. Revoked bindings check again
        // after obtaining the permit and between directory entries/files.
        let _worker = backup_work();
        if slot.background_cancelled.load(Ordering::Acquire) {
            return;
        }
        let outcome = do_backup_source_cancellable(&app, &slot.store, source, "", &|| {
            slot.background_cancelled.load(Ordering::Acquire)
        });
        if !slot.background_cancelled.load(Ordering::Acquire) {
            report_launch_failure(&app, &slot, &outcome);
        }
    });
}

pub(crate) fn backup_graph_now(
    app: &tauri::AppHandle,
    store: &Store,
    root: &std::path::Path,
    suffix: &str,
) -> BackupOutcome {
    let source = match BackupSource::from_store(store, root) {
        Ok(source) => source,
        Err((kind, _)) => return BackupOutcome::failed(0, "source", kind),
    };
    do_backup_source(app, store, source, suffix)
}

/// Snapshot the graph before a rewrite the user asked for, refusing the rewrite
/// when the snapshot failed so the original files stay recoverable in Backups &
/// recovery. The tagged snapshot is exempt from the keep-count prune.
pub(crate) fn snapshot_before_rewrite(
    app: &tauri::AppHandle,
    slot: &GraphSlot,
    suffix: &str,
) -> Result<(), String> {
    rewrite_snapshot_result(backup_graph_now(app, &slot.store, &slot.root_key, suffix))
}

fn rewrite_snapshot_result(outcome: BackupOutcome) -> Result<(), String> {
    match outcome.failure {
        None => Ok(()),
        Some(failure) => Err(failure.wire()),
    }
}

/// Take one snapshot of the current graph now (synchronous). Returns the number
/// of files copied (0 = nothing to back up). Reads the keep count from the local
/// app-settings file and prunes old snapshots afterwards. `suffix` tags special
/// snapshots (e.g. "pre-restore") so they get a distinct, collision-proof
/// directory name and are exempt from the keep-count prune.
/// The typed outcome records any graph text/config/asset-sidecar copy failure,
/// so the caller (restore) can refuse to proceed without a full rollback snapshot.
#[derive(Clone)]
struct BackupSource {
    root: PathBuf,
    journals_dir: String,
    pages_dir: String,
    assets_dir_name: String,
    hidden: Vec<String>,
    hidden_parse_failed_closed: bool,
}

impl BackupSource {
    /// The live layout to snapshot, or why it can't be read: the error kind
    /// for the `backup-failed:source:<kind>` token (I-9) and a message.
    fn from_store(store: &Store, root: &std::path::Path) -> Result<Self, (ErrorKind, String)> {
        let config = store.config();
        let root = Store::canonical_root(root).map_err(|error| {
            let kind = match &error {
                tine_store::OpenError::NotAFolder(_) => ErrorKind::NotADirectory,
                tine_store::OpenError::Unresolvable { .. } => ErrorKind::NotFound,
                tine_store::OpenError::Io(io) => io.kind,
                tine_store::OpenError::CreateFailed { cause, .. } => cause.kind,
                _ => ErrorKind::InvalidInput,
            };
            (kind, error.to_string())
        })?;
        // Verify the live assets target before using the store's backup layout.
        store.scan_area(Area::Assets, None).map_err(|error| {
            (
                store_error_kind(&error),
                format!("unsafe assets directory: {error:?}"),
            )
        })?;
        let assets_dir_name = config.assets_directory_name.clone();
        Ok(Self {
            root,
            journals_dir: config.journals_dir.clone(),
            pages_dir: config.pages_dir.clone(),
            assets_dir_name,
            hidden: config.hidden.clone(),
            hidden_parse_failed_closed: config.hidden_parse_failed_closed,
        })
    }
}

/// Schema 4 (og-backup-cas, docs/storage-contract.md "Graph backups") lists
/// the same paths as schema 3, but its snapshot directory holds only the
/// manifest, which carries its own checksum: each listed file's bytes live
/// once in the namespace's blob store. Schema 4 lives in its own namespace,
/// `<app-data>/backups/<id>.cas/` (`cas_dir`), which no older build lists,
/// prunes or cleans. Schema 3 (og-B, ADR 0062) keeps a full copy of graph
/// text under `graph/<graph-relative path>` and records the graph-text scope
/// it covered; schema 2 kept only the configured `journals/` and `pages/`
/// roots. Both full-copy schemas sit in the legacy namespace
/// `<app-data>/backups/<id>/`, are master's wire formats and still list,
/// restore and prune.
const SNAPSHOT_SCHEMA: u32 = 4;
const GRAPH_COPY_SNAPSHOT_SCHEMA: u32 = 3;
const LEGACY_SNAPSHOT_SCHEMA: u32 = 2;
/// The schema-4 namespace's parts: blob store, snapshots, lock file.
const BLOB_DIR: &str = "blobs";
const CAS_SNAPSHOTS: &str = "snapshots";
const CAS_LOCK: &str = "lock";
/// The manifest field holding the SHA-256 of the manifest without it.
const MANIFEST_CHECKSUM: &str = "checksum";
/// How long a snapshot before a rewrite or restore, a restore's read and a
/// keep-count change wait for another process's backup work.
const CAS_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// Master's `GRAPH_TEXT_SCOPE_VERSION`: the discovery exclusions this build's
/// `graph_text_eligible` applies (`published-queries/` included).
const GRAPH_TEXT_SCOPE_VERSION: u32 = 2;
/// Marks this build's schema-3/4 snapshots; master ignores the field. Prune
/// counts only snapshots this build wrote (docs/app-identity.md).
const SNAPSHOT_WRITER: &str = "og";
const SNAPSHOT_MANIFEST: &str = "snapshot.json";

/// The schema-4 namespace beside a graph's legacy backup directory
/// `<app-data>/backups/<id>`: `<app-data>/backups/<id>.cas`. A root backup id
/// never contains `.`, so it is no other graph's directory.
fn cas_dir(base: &std::path::Path) -> PathBuf {
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(".cas");
    base.with_file_name(name)
}

/// Take the exclusive OS lock on a schema-4 namespace (REVIEW-backup-cas B2,
/// og-backup-cas D1). Only Tine writes the namespace (assumption A-bk1), and
/// every reservation, blob write or repair, publication, prune, collection
/// and restore read happens under this lock, so a process holding it sees
/// no other writer. `wait: None` tries once; otherwise the wait is a
/// contention bound on this thread. `Ok(None)`: another process holds it.
fn lock_namespace(
    cas: &std::path::Path,
    wait: Option<std::time::Duration>,
) -> std::io::Result<Option<crate::file_lock::FileLock>> {
    std::fs::create_dir_all(cas)?;
    let path = cas.join(CAS_LOCK);
    match wait {
        None => crate::file_lock::try_lock_exclusive(&path),
        Some(wait) => {
            crate::file_lock::lock_exclusive_until(&path, std::time::Instant::now() + wait)
        }
    }
}

/// A launch publishes a new anchor when the newest valid one is this old
/// (og-backup-cas D2), by the clock in either direction.
const ANCHOR_ROTATION_SECS: u64 = 7 * 24 * 60 * 60;

/// Seconds since the Unix epoch; tests shift the clock.
fn now_unix() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    #[cfg(test)]
    let now = now.saturating_add_signed(CLOCK_SHIFT.with(std::cell::Cell::get));
    now
}

/// The largest file a snapshot holds: a backup refuses a bigger one
/// (`FileTooLarge`), so restore can refuse a bigger blob as damaged before
/// reading or hashing it, which bounds restore's memory.
const SNAPSHOT_FILE_MAX_BYTES: u64 = 256 * 1024 * 1024;

#[cfg(test)]
std::thread_local! {
    static PAYLOAD_HASH_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SnapshotFile {
    path: String,
    sha256: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotManifest {
    schema: u32,
    root: String,
    journals_dir: String,
    pages_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph_text_policy: Option<SnapshotGraphTextPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    writer: Option<String>,
    files: Vec<SnapshotFile>,
    complete: bool,
    /// A durably published snapshot the keep-count never counts or prunes
    /// (og-backup-cas D2); covered by the checksum like every field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    anchor: bool,
    /// When the snapshot was taken (Unix seconds); anchor rotation reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_unix: Option<u64>,
}

/// The graph-text scope a schema-3 snapshot covered: restore retires only
/// unlisted live text inside it. A `:hidden` value that failed to parse hides
/// all graph text, so the snapshot holds none and records
/// `hidden_parse_failed_closed: true` (restore then retires none).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SnapshotGraphTextPolicy {
    version: u32,
    hidden: Vec<String>,
    hidden_parse_failed_closed: bool,
}

pub(crate) fn root_backup_id(root: &std::path::Path) -> String {
    let canonical = Store::canonical_root(root).unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    let label = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("graph")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{label}-{}", &digest[..32])
}

/// The SHA-256 of a manifest's JSON without its checksum field (REVIEW B1),
/// over its canonical form: compact, object keys sorted at every level,
/// strings unescaped beyond JSON's minimum (Python's `json.dumps(v,
/// sort_keys=True, separators=(",", ":"), ensure_ascii=False)`).
fn manifest_checksum(manifest: &serde_json::Value) -> String {
    let mut bytes = Vec::new();
    canonical_json(manifest, &mut bytes);
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_json(value: &serde_json::Value, out: &mut Vec<u8>) {
    fn leaf(value: &impl serde::Serialize, out: &mut Vec<u8>) {
        serde_json::to_writer(out, value).expect("a JSON value serializes");
    }
    match value {
        serde_json::Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                leaf(key, out);
                out.push(b':');
                canonical_json(&fields[key], out);
            }
            out.push(b'}');
        }
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                canonical_json(item, out);
            }
            out.push(b']');
        }
        other => leaf(other, out),
    }
}

/// Write the manifest, with its checksum, into an unpublished snapshot
/// directory; the directory's rename publishes it.
fn write_manifest(dir: &std::path::Path, manifest: &SnapshotManifest) -> std::io::Result<()> {
    use std::io::Write;
    let mut value = serde_json::to_value(manifest).map_err(std::io::Error::other)?;
    let checksum = manifest_checksum(&value);
    if let Some(fields) = value.as_object_mut() {
        fields.insert(MANIFEST_CHECKSUM.into(), checksum.into());
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(SNAPSHOT_MANIFEST))?;
    file.write_all(&bytes)?;
    record_backup_op("manifest_write");
    Ok(())
}

#[cfg(test)]
type PauseHook = Option<(&'static str, Box<dyn FnOnce()>)>;

/// What the sync seam makes durable: a file's bytes or a directory's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Synced {
    File,
    Dir,
}

#[cfg(test)]
std::thread_local! {
    static BACKUP_OPS: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Runs once when this thread records the named op.
    static PAUSE_AT: std::cell::RefCell<PauseHook> = const { std::cell::RefCell::new(None) };
    /// Every sync this thread completed, in order (og-backup-cas D6).
    static SYNCS: std::cell::RefCell<Vec<(Synced, PathBuf)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Fails this thread's sync once `SYNCS` holds this many.
    static FAIL_SYNC: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Seconds added to this thread's clock.
    static CLOCK_SHIFT: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

fn record_backup_op(op: &'static str) {
    #[cfg(test)]
    {
        BACKUP_OPS.with(|ops| ops.borrow_mut().push(op));
        let hook = PAUSE_AT.with(|pause| {
            let mut pause = pause.borrow_mut();
            pause
                .as_ref()
                .is_some_and(|(at, _)| *at == op)
                .then(|| pause.take())
                .flatten()
        });
        if let Some((_, hook)) = hook {
            hook();
        }
    }
    #[cfg(not(test))]
    let _ = op;
}

/// The one seam every backup sync goes through (og-backup-cas D6). A test
/// observes a sync only after the real call returned Ok, so a publication
/// that skips a call loses its observation; tests make the n-th one fail.
fn sync_durable(kind: Synced, path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_SYNC.with(std::cell::Cell::get) == Some(SYNCS.with(|syncs| syncs.borrow().len())) {
        return Err(std::io::Error::other("injected sync failure"));
    }
    match kind {
        // Windows flushes a file only through a handle that may write.
        Synced::File => std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .sync_all()?,
        Synced::Dir => tine_store::directory_durability::sync_directory_entry(path)?,
    }
    #[cfg(test)]
    SYNCS.with(|syncs| syncs.borrow_mut().push((kind, path.to_path_buf())));
    record_backup_op("sync");
    Ok(())
}

/// Whether the blob at `path` holds exactly `bytes`, reading at most one
/// buffer past their length (REVIEW F1).
fn blob_matches(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() != bytes.len() as u64 {
        return Ok(false);
    }
    let mut buf = [0u8; 64 * 1024];
    let mut offset = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(offset == bytes.len());
        }
        if bytes.get(offset..offset + n) != Some(&buf[..n]) {
            return Ok(false);
        }
        offset += n;
    }
}

static BLOB_TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Store `bytes`, whose SHA-256 is `sha256`, in the blob store unless an
/// identical blob is already there. The caller holds the namespace lock.
///
/// Blobs are written without a sync (verification, not fsync, keeps a
/// snapshot honest), so power loss can leave a blob name holding torn bytes.
/// Every later snapshot of that unchanged content would reuse it, so a blob
/// is reused only when its bytes equal the content, and a damaged one is
/// replaced. Each write goes to its own `create_new` temp and is renamed into
/// place, so a crash never leaves a blob name with a partial write.
fn put_blob(blobs: &std::path::Path, sha256: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let path = blobs.join(sha256);
    match blob_matches(&path, bytes) {
        Ok(true) => return Ok(()),
        Ok(false) => record_backup_op("blob_repair"),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let (tmp, mut file) = loop {
        let tmp = blobs.join(format!(
            ".tmp-{sha256}-{}-{}",
            std::process::id(),
            BLOB_TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => break (tmp, file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let written = file.write_all(bytes).map(|()| drop(file)).and_then(|()| {
        record_backup_op("blob_temp");
        std::fs::rename(&tmp, &path)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    record_backup_op("blob_write");
    written
}

/// Publish an unpublished snapshot directory under its final name. A
/// `durable` snapshot (an anchor, or one taken before a rewrite or restore
/// the user asked for) syncs, every time and in this order (og-backup-cas
/// D4): every blob it lists, `blobs/`, the manifest, the unpublished
/// directory, then after the rename `snapshots/`, `<id>.cas/`, `backups/`
/// and the app-data directory. Existence is no durability witness: a launch
/// may have written those entries without a sync. A failed sync fails the
/// snapshot. A routine launch snapshot syncs nothing: restore verifies every
/// blob before any graph write, a torn one is refused, and the anchor
/// remains (docs/storage-contract.md "Graph backups").
fn publish_snapshot(
    partial: &std::path::Path,
    final_dest: &std::path::Path,
    manifest: &SnapshotManifest,
    durable: bool,
) -> std::io::Result<()> {
    let snapshots = final_dest.parent().expect("snapshot has parent");
    let cas = snapshots.parent().expect("namespace has parent");
    if durable {
        let blobs = cas.join(BLOB_DIR);
        let listed: std::collections::BTreeSet<&str> = manifest
            .files
            .iter()
            .map(|file| file.sha256.as_str())
            .collect();
        for sha256 in listed {
            sync_durable(Synced::File, &blobs.join(sha256))?;
        }
        sync_durable(Synced::Dir, &blobs)?;
    }
    write_manifest(partial, manifest)?;
    if durable {
        sync_durable(Synced::File, &partial.join(SNAPSHOT_MANIFEST))?;
        sync_durable(Synced::Dir, partial)?;
    }
    crate::device_io::move_file_noreplace(partial, final_dest)?;
    record_backup_op("publish_rename");
    if durable {
        let backups = cas.parent().expect("backups dir");
        let app_data = backups.parent().expect("app-data dir");
        for dir in [snapshots, cas, backups, app_data] {
            sync_durable(Synced::Dir, dir)?;
        }
    }
    Ok(())
}

/// A manifest's JSON without its checksum field, and whether that checksum
/// matched; `None` when it is unreadable or not a JSON object.
fn manifest_json(dir: &std::path::Path) -> Option<(serde_json::Value, bool)> {
    let bytes = std::fs::read(dir.join(SNAPSHOT_MANIFEST)).ok()?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let checksum = value
        .as_object_mut()?
        .remove_entry(MANIFEST_CHECKSUM)
        .map(|(_, checksum)| checksum);
    let checksum_ok =
        checksum.as_ref().and_then(serde_json::Value::as_str) == Some(&manifest_checksum(&value));
    Some((value, checksum_ok))
}

/// A listable, restorable manifest: a supported schema, complete, and for
/// schema 4 a checksum that matches (REVIEW B1: a bit flip in a path or the
/// scope can leave valid JSON). Listing and restore both read through this.
fn read_manifest(dir: &std::path::Path) -> Option<SnapshotManifest> {
    let (value, checksum_ok) = manifest_json(dir)?;
    let manifest: SnapshotManifest = serde_json::from_value(value).ok()?;
    let supported = manifest.schema == LEGACY_SNAPSHOT_SCHEMA
        || (matches!(
            manifest.schema,
            GRAPH_COPY_SNAPSHOT_SCHEMA | SNAPSHOT_SCHEMA
        ) && manifest
            .graph_text_policy
            .as_ref()
            .is_some_and(|policy| policy.version == GRAPH_TEXT_SCOPE_VERSION));
    let checked = manifest.schema != SNAPSHOT_SCHEMA || checksum_ok;
    (supported && checked && manifest.complete).then_some(manifest)
}

fn hash_snapshot_file(path: &std::path::Path) -> std::io::Result<String> {
    #[cfg(test)]
    PAYLOAD_HASH_READS.with(|reads| reads.set(reads.get() + 1));
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn snapshot_inventory(dir: &std::path::Path) -> std::io::Result<Vec<SnapshotFile>> {
    let mut files = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), PathBuf::new())];
    while let Some((current, rel)) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let rel_child = rel.join(entry.file_name());
            if file_type.is_dir() {
                stack.push((entry.path(), rel_child));
            } else if file_type.is_file()
                && rel_child != std::path::Path::new(SNAPSHOT_MANIFEST)
                && rel_child != std::path::Path::new(".snapshot.json.tmp")
            {
                let path = rel_child
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                files.push(SnapshotFile {
                    path,
                    sha256: hash_snapshot_file(&entry.path())?,
                });
            } else if !file_type.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "snapshot contains a non-regular entry",
                ));
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// A SHA-256 hex digest, the only blob name a manifest may use.
fn is_digest(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where a listed file's bytes live: inside a full-copy snapshot, or in the
/// namespace's blob store for schema 4. `None` when a schema-4 entry's hash
/// is not a digest, so no manifest names a path outside the store.
fn payload_path(
    snapshot: &std::path::Path,
    cas: &std::path::Path,
    manifest: &SnapshotManifest,
    file: &SnapshotFile,
) -> Option<PathBuf> {
    if manifest.schema != SNAPSHOT_SCHEMA {
        return Some(snapshot.join(&file.path));
    }
    is_digest(&file.sha256).then(|| cas.join(BLOB_DIR).join(&file.sha256))
}

/// Every listed file's bytes, each read once and checked against its hash,
/// or `None` for a damaged snapshot. A full copy must also hold exactly its
/// manifest's files. Restore consumes these bytes and never reads the
/// snapshot again (REVIEW N1); the caller holds the namespace lock.
fn load_verified_payloads(
    snapshot: &std::path::Path,
    cas: &std::path::Path,
    manifest: &SnapshotManifest,
) -> Option<Vec<Vec<u8>>> {
    if manifest.schema != SNAPSHOT_SCHEMA && snapshot_inventory(snapshot).ok()? != manifest.files {
        return None;
    }
    manifest
        .files
        .iter()
        .map(|file| {
            let mut input =
                std::fs::File::open(payload_path(snapshot, cas, manifest, file)?).ok()?;
            // No backup writes a file over the cap, so a bigger one is damage,
            // refused before it is read or hashed.
            if input.metadata().ok()?.len() > SNAPSHOT_FILE_MAX_BYTES {
                return None;
            }
            #[cfg(test)]
            PAYLOAD_HASH_READS.with(|reads| reads.set(reads.get() + 1));
            let mut bytes = Vec::new();
            (&mut input)
                .take(SNAPSHOT_FILE_MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > SNAPSHOT_FILE_MAX_BYTES {
                return None;
            }
            (format!("{:x}", Sha256::digest(&bytes)) == file.sha256).then_some(bytes)
        })
        .collect()
}

/// Whether every blob a schema-4 manifest lists hashes to its name,
/// streaming each once: an anchor's check before older anchors go (D2).
fn blobs_verify(cas: &std::path::Path, manifest: &SnapshotManifest) -> bool {
    let listed: std::collections::BTreeSet<&str> = manifest
        .files
        .iter()
        .map(|file| file.sha256.as_str())
        .collect();
    listed.into_iter().all(|sha256| {
        is_digest(sha256)
            && hash_snapshot_file(&cas.join(BLOB_DIR).join(sha256)).is_ok_and(|hash| hash == sha256)
    })
}

/// Test helper: whether a snapshot verifies, for one in either namespace.
#[cfg(test)]
fn verify_snapshot(dir: &std::path::Path, manifest: &SnapshotManifest) -> bool {
    let cas = dir.parent().and_then(std::path::Path::parent).unwrap();
    load_verified_payloads(dir, cas, manifest).is_some()
}

/// Test helper: the published schema-4 snapshots of `base`, oldest first.
#[cfg(test)]
fn cas_snapshots(base: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs: Vec<(String, PathBuf)> = std::fs::read_dir(cas_dir(base).join(CAS_SNAPSHOTS))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .filter(|(name, _)| !name.starts_with(".partial-"))
        .collect();
    dirs.sort_by(|(a, _), (b, _)| snapshot_order(a).cmp(&snapshot_order(b)));
    dirs.into_iter().map(|(_, dir)| dir).collect()
}

/// A private, unnamed copy of verified bytes for `Store::restore`, so the
/// restore copies exactly what it verified (REVIEW N1).
fn stage_restore_bytes(dir: &std::path::Path, bytes: &[u8]) -> std::io::Result<std::fs::File> {
    use std::io::{Seek, Write};
    let mut file = tempfile::tempfile_in(dir)?;
    file.write_all(bytes)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    Ok(file)
}

fn do_backup_source(
    app: &tauri::AppHandle,
    store: &Store,
    source: BackupSource,
    suffix: &str,
) -> BackupOutcome {
    let _worker = backup_work();
    do_backup_source_cancellable(app, store, source, suffix, &|| false)
}

/// Read one store area's included files, put each in the blob store and list
/// it in `files` as `<prefix>/<area-relative path>`.
fn copy_store_area(
    store: &Store,
    area: Area,
    blobs: &std::path::Path,
    prefix: &str,
    include: fn(&tine_store::FileId) -> bool,
    files: &mut Vec<SnapshotFile>,
    cancelled: &dyn Fn() -> bool,
) -> (usize, usize, Option<BackupFailure>) {
    let phase = match area {
        Area::Journals => "journals",
        Area::Pages => "pages",
        Area::Assets => "assets",
        Area::Meta => "config",
        Area::Trash => "trash",
        Area::Graph => "graph",
    };
    if cancelled() {
        return (
            0,
            1,
            Some(BackupFailure {
                phase,
                kind: ErrorKind::Interrupted,
            }),
        );
    }
    let listing = match store.scan_area(area, None) {
        Ok(listing) => listing,
        Err(error) => {
            return (
                0,
                1,
                Some(BackupFailure {
                    phase,
                    kind: store_error_kind(&error),
                }),
            )
        }
    };
    let mut copied = 0;
    let mut first_failure = listing
        .unreadable
        .iter()
        .find(|(_, error)| error.kind != ErrorKind::NotFound)
        .map(|(_, error)| BackupFailure {
            phase,
            kind: error.kind,
        });
    let mut failed = listing
        .unreadable
        .iter()
        .filter(|(_, error)| error.kind != ErrorKind::NotFound)
        .count();
    for entry in listing.files {
        if cancelled() {
            return (
                copied,
                failed + 1,
                Some(BackupFailure {
                    phase,
                    kind: ErrorKind::Interrupted,
                }),
            );
        }
        if !include(&entry.id) {
            continue;
        }
        match store.read(&entry.id, Some(SNAPSHOT_FILE_MAX_BYTES)) {
            Ok((bytes, _)) => {
                let sha256 = format!("{:x}", Sha256::digest(&bytes));
                match put_blob(blobs, &sha256, &bytes) {
                    Ok(()) => {
                        copied += 1;
                        files.push(SnapshotFile {
                            path: format!("{prefix}/{}", entry.rel),
                            sha256,
                        });
                    }
                    Err(error) => {
                        failed += 1;
                        first_failure.get_or_insert(BackupFailure {
                            phase,
                            kind: error.kind(),
                        });
                    }
                }
            }
            Err(error) => {
                failed += 1;
                first_failure.get_or_insert(BackupFailure {
                    phase,
                    kind: store_error_kind(&error),
                });
            }
        }
    }
    (copied, failed, first_failure)
}

fn store_error_kind(error: &tine_store::StoreError) -> ErrorKind {
    match error {
        tine_store::StoreError::Io(error) => error.kind(),
        tine_store::StoreError::NotFound => ErrorKind::NotFound,
        tine_store::StoreError::Undecodable | tine_store::StoreError::Unparseable(_) => {
            ErrorKind::InvalidData
        }
        tine_store::StoreError::InvalidTarget(_)
        | tine_store::StoreError::PageSource(_)
        | tine_store::StoreError::StreamSymlink(_) => ErrorKind::InvalidInput,
        tine_store::StoreError::TooLarge { .. } => ErrorKind::FileTooLarge,
        tine_store::StoreError::Closed => ErrorKind::BrokenPipe,
    }
}

/// The live graph-text count a snapshot must match, or the failure that
/// prevented counting: the scan's own error, or the first unreadable entry
/// (I-9: the cause reaches the backup token, not a fixed `Other`).
fn count_store_text(store: &Store, area: Area) -> Result<usize, ErrorKind> {
    let listing = store
        .scan_area(area, None)
        .map_err(|error| store_error_kind(&error))?;
    if let Some((_, error)) = listing
        .unreadable
        .iter()
        .find(|(_, error)| error.kind != ErrorKind::NotFound)
    {
        return Err(error.kind);
    }
    Ok(listing
        .files
        .iter()
        .filter(|entry| is_graph_text(&entry.id))
        .count())
}

struct PartialBackup {
    path: PathBuf,
    committed: bool,
}

impl Drop for PartialBackup {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Whether a crashed snapshot left an unpublished directory in a
/// namespace's `snapshots/`. An unreadable listing counts as one, so the
/// collection that follows reports it. The collector removes them.
fn has_partials(snapshots: &std::path::Path) -> bool {
    match std::fs::read_dir(snapshots) {
        Ok(entries) => entries.into_iter().any(|entry| {
            entry.map_or(true, |entry| {
                entry.file_name().to_string_lossy().starts_with(".partial-")
            })
        }),
        Err(error) => error.kind() != ErrorKind::NotFound,
    }
}

fn do_backup_source_cancellable(
    app: &tauri::AppHandle,
    store: &Store,
    source: BackupSource,
    suffix: &str,
    cancelled: &dyn Fn() -> bool,
) -> BackupOutcome {
    if cancelled() {
        return BackupOutcome::failed(0, "cancelled", ErrorKind::Interrupted);
    }
    let Ok(data_dir) = app.path().app_data_dir() else {
        return BackupOutcome::failed(0, "app-data", ErrorKind::NotFound);
    };
    let base = data_dir.join("backups").join(root_backup_id(&source.root));
    backup_locked(&base, store, source, suffix, backup_keep(app), cancelled)
}

/// Take one snapshot under the namespace lock, then apply the keep-count.
/// A launch snapshot (no suffix) that finds the lock held is skipped, not
/// refused: another Tine process is backing up this graph now. A suffixed
/// snapshot waits, bounded, and fails on timeout, which refuses the rewrite
/// or restore it precedes (concurrent honest instances; refusal row
/// docs/storage-contract.md, `src-tauri::backup` namespace lock).
///
/// A launch with no valid anchor, or whose newest is `ANCHOR_ROTATION_SECS`
/// old, publishes its snapshot as the new anchor, durably, verifies it, and
/// only then deletes the older anchors (og-backup-cas D2). The collector
/// runs only after a crashed partial, a failed snapshot or a deletion, never
/// on a launch that removed nothing (REVIEW F2).
fn backup_locked(
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    suffix: &str,
    keep: usize,
    cancelled: &dyn Fn() -> bool,
) -> BackupOutcome {
    let cas = cas_dir(base);
    let _lock = match lock_namespace(&cas, (!suffix.is_empty()).then_some(CAS_LOCK_WAIT)) {
        Ok(Some(lock)) => lock,
        Ok(None) if suffix.is_empty() => {
            crate::debug::diag_private(
                "backup-skipped",
                "launch backup skipped: another Tine process holds this graph's backup lock",
            );
            return BackupOutcome::success(0);
        }
        Ok(None) => return BackupOutcome::failed(0, "lock", ErrorKind::TimedOut),
        Err(error) => return BackupOutcome::failed(0, "lock", error.kind()),
    };
    let crashed = has_partials(&cas.join(CAS_SNAPSHOTS));
    let anchors = if suffix.is_empty() {
        valid_anchors(&cas)
    } else {
        Vec::new()
    };
    let newest = anchors.iter().filter_map(|(_, created)| *created).max();
    let rotate = suffix.is_empty()
        && newest.is_none_or(|newest| now_unix().abs_diff(newest) >= ANCHOR_ROTATION_SECS);
    let mut outcome = write_snapshot(base, store, source, suffix, rotate, cancelled);
    let mut removed = 0;
    if outcome.failure.is_none() && outcome.copied > 0 {
        if let (true, Some(name)) = (rotate, &outcome.published) {
            match settle_anchor(&cas, name, &anchors) {
                Ok(older) => removed += older,
                Err(kind) => outcome = BackupOutcome::failed(outcome.copied, "anchor", kind),
            }
        }
        removed += prune_backups(base, keep);
    }
    if crashed || removed > 0 || outcome.failure.is_some() {
        report_collection(collect::collect(&cas));
    }
    outcome
}

/// The namespace's valid anchors: a checksummed manifest marked `anchor`,
/// with its creation time. An unreadable listing reads as none, which only
/// makes the launch publish one.
fn valid_anchors(cas: &std::path::Path) -> Vec<(String, Option<u64>)> {
    std::fs::read_dir(cas.join(CAS_SNAPSHOTS))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with(".partial-") {
                return None;
            }
            let manifest = read_manifest(&entry.path())?;
            manifest.anchor.then_some((name, manifest.created_unix))
        })
        .collect()
}

/// Verify a just-published anchor by hashing every blob it lists, then
/// delete the older anchors; returns how many went. A failed check removes
/// the new anchor instead, so every older one stays (refusal scenario:
/// docs/storage-contract.md I-8 row `src-tauri::backup::settle_anchor`).
fn settle_anchor(
    cas: &std::path::Path,
    name: &str,
    older: &[(String, Option<u64>)],
) -> Result<usize, ErrorKind> {
    let snapshots = cas.join(CAS_SNAPSHOTS);
    let dir = snapshots.join(name);
    record_backup_op("anchor_published");
    if !read_manifest(&dir).is_some_and(|manifest| blobs_verify(cas, &manifest)) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(ErrorKind::InvalidData);
    }
    record_backup_op("anchor_verified");
    for (old, _) in older {
        let _ = std::fs::remove_dir_all(snapshots.join(old));
    }
    Ok(older.len())
}

/// Log what a collection could not do; it never fails the snapshot.
fn report_collection(result: std::io::Result<collect::CollectReport>) {
    match result {
        Ok(report) if report.failed.is_empty() && report.damaged.is_empty() => {}
        Ok(report) => crate::debug::diag_private(
            "backup-collect",
            format!(
                "backup collection: {} removed, failed {:?}, damaged {:?}",
                report.removed, report.failed, report.damaged
            ),
        ),
        Err(error) => {
            record_backup_op("collect_failed");
            crate::debug::diag_private(
                "backup-collect-failed",
                format!("backup collection stopped before deleting anything: {error}"),
            );
        }
    }
}

/// Put one snapshot's files in the blob store of `base`'s schema-4
/// namespace and publish its manifest there; the caller holds the namespace
/// lock and prunes. An anchor, and a suffixed snapshot (one before a rewrite
/// or restore the user asked for), are published durably (`publish_snapshot`).
fn write_snapshot(
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    suffix: &str,
    anchor: bool,
    cancelled: &dyn Fn() -> bool,
) -> BackupOutcome {
    let stamp = tine_core::date::utc_backup_stamp();
    let name = if suffix.is_empty() {
        stamp
    } else {
        format!("{stamp}-{suffix}")
    };
    let cas = cas_dir(base);
    let snapshots = cas.join(CAS_SNAPSHOTS);
    let blobs = cas.join(BLOB_DIR);
    // Reserve a UNIQUE name. The stamp is second-granularity, so two snapshots
    // in the same second (e.g. a launch snapshot and a pre-restore snapshot)
    // would otherwise collide. The counter starts above every counter this
    // second already has, published or partial, in either namespace, so a
    // freed lower name is never reused below a surviving one (which would
    // order the new snapshot as older). `create_dir` (non-recursive) fails
    // atomically if the partial name is taken anyway.
    let highest = std::fs::create_dir_all(&snapshots)
        .and_then(|()| std::fs::create_dir_all(&blobs))
        .and_then(|()| Ok(highest_counter(&snapshots, &name)?.max(highest_counter(base, &name)?)));
    let mut k = match highest {
        Ok(highest) => highest + 1,
        Err(error) => return BackupOutcome::failed(0, "reserve", error.kind()),
    };
    let mut published = if k == 1 {
        name.clone()
    } else {
        format!("{name}-{k}")
    };
    let (final_dest, dest) = loop {
        let final_dest = snapshots.join(&published);
        let dest = snapshots.join(format!(".partial-{published}"));
        let reserved = if std::fs::symlink_metadata(&final_dest).is_ok()
            || std::fs::symlink_metadata(base.join(&published)).is_ok()
        {
            Err(ErrorKind::AlreadyExists.into())
        } else {
            std::fs::create_dir(&dest)
        };
        match reserved {
            Ok(()) => break (final_dest, dest),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                k = k.max(1) + 1;
                published = format!("{name}-{k}");
            }
            Err(error) => return BackupOutcome::failed(0, "reserve", error.kind()),
        }
    };
    let mut partial = PartialBackup {
        path: dest.clone(),
        committed: false,
    };
    let live_text_n = match count_store_text(store, Area::Graph) {
        Ok(count) => count,
        Err(kind) => return BackupOutcome::failed(0, "inventory", kind),
    };
    let mut files = Vec::new();
    // Graph text anywhere in the graph-text scope, at its graph-relative path.
    let (ct, ft, et) = copy_store_area(
        store,
        Area::Graph,
        &blobs,
        "graph",
        is_graph_text,
        &mut files,
        cancelled,
    );
    let (ca, fa, ea) = copy_store_area(
        store,
        Area::Assets,
        &blobs,
        &source.assets_dir_name,
        is_asset_sidecar,
        &mut files,
        cancelled,
    );
    let mut n = ct + ca;
    let mut failed = ft + fa;
    let mut first_failure = et.or(ea);
    if !cancelled() {
        match store.scan_area(Area::Meta, None) {
            Ok(listing) => {
                failed += listing
                    .unreadable
                    .iter()
                    .filter(|(_, error)| error.kind != std::io::ErrorKind::NotFound)
                    .count();
                if first_failure.is_none() {
                    first_failure = listing
                        .unreadable
                        .iter()
                        .find(|(_, error)| error.kind != ErrorKind::NotFound)
                        .map(|(_, error)| BackupFailure {
                            phase: "config",
                            kind: error.kind,
                        });
                }
                if let Some(config) = listing.files.iter().find(|entry| entry.rel == "config.edn") {
                    match store.read(&config.id, Some(SNAPSHOT_FILE_MAX_BYTES)) {
                        Ok((bytes, _)) => {
                            let sha256 = format!("{:x}", Sha256::digest(&bytes));
                            match put_blob(&blobs, &sha256, &bytes) {
                                Ok(()) => {
                                    n += 1;
                                    files.push(SnapshotFile {
                                        path: "logseq/config.edn".into(),
                                        sha256,
                                    });
                                }
                                Err(error) => {
                                    failed += 1;
                                    first_failure.get_or_insert(BackupFailure {
                                        phase: "config",
                                        kind: error.kind(),
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            failed += 1;
                            first_failure.get_or_insert(BackupFailure {
                                phase: "config",
                                kind: store_error_kind(&error),
                            });
                        }
                    }
                }
            }
            Err(error) => {
                failed += 1;
                first_failure.get_or_insert(BackupFailure {
                    phase: "config",
                    kind: store_error_kind(&error),
                });
            }
        }
    }
    if cancelled() {
        return BackupOutcome::failed(n, "cancelled", ErrorKind::Interrupted);
    }
    if failed != 0 {
        return BackupOutcome {
            copied: n,
            failure: first_failure.or(Some(BackupFailure {
                phase: "copy",
                kind: ErrorKind::Other,
            })),
            published: None,
        };
    }
    if ct != live_text_n {
        return BackupOutcome::failed(n, "inventory", ErrorKind::InvalidData);
    }
    if n == 0 {
        return BackupOutcome::success(0);
    }
    record_backup_op("blobs_written");
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = SnapshotManifest {
        schema: SNAPSHOT_SCHEMA,
        root: source.root.display().to_string(),
        journals_dir: source.journals_dir,
        pages_dir: source.pages_dir,
        graph_text_policy: Some(SnapshotGraphTextPolicy {
            version: GRAPH_TEXT_SCOPE_VERSION,
            hidden: source.hidden,
            hidden_parse_failed_closed: source.hidden_parse_failed_closed,
        }),
        writer: Some(SNAPSHOT_WRITER.into()),
        files,
        complete: true,
        anchor,
        created_unix: Some(now_unix()),
    };
    if let Err(error) =
        publish_snapshot(&dest, &final_dest, &manifest, anchor || !suffix.is_empty())
    {
        return BackupOutcome::failed(n, "publish", error.kind());
    }
    partial.committed = true;
    BackupOutcome {
        copied: n,
        failure: None,
        published: Some(published),
    }
}

fn backup_keep(app: &tauri::AppHandle) -> usize {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("backup_keep").and_then(|x| x.as_u64()))
        .map(|n| (n as usize).max(1))
        .unwrap_or(BACKUP_KEEP_DEFAULT)
}

#[derive(serde::Serialize)]
pub(crate) struct BackupInfo {
    stamp: String,
    files: usize,
}

#[tauri::command]
pub(crate) fn get_backup_keep(app: tauri::AppHandle) -> usize {
    backup_keep(&app)
}

#[tauri::command]
pub(crate) async fn set_backup_keep(
    keep: usize,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let keep = keep.clamp(1, 1000);
    // Resolved first, as before the write: a stale binding still writes the
    // setting (device-wide) but prunes nothing, exactly like the old order.
    let slot = slot_for_context(&state);
    // Settings fsync and snapshot pruning (R3): off the main thread.
    crate::state::off_ui(move || {
        update_settings(&app, |json| {
            json["backup_keep"] = serde_json::json!(keep);
        })?;
        // Apply the new (possibly lower) cap to the current graph's snapshots now.
        let slot = slot?;
        if let Some(base) = backup_base(&app, &slot.root_key) {
            prune_now(&base, keep);
        }
        Ok(())
    })
    .await
}

/// The backup directory for the currently-open graph (`<app-data>/backups/<id>`).
fn backup_base(app: &tauri::AppHandle, root: &std::path::Path) -> Option<PathBuf> {
    backup_base_for_root(app, root)
}

fn backup_base_for_root(app: &tauri::AppHandle, root: &std::path::Path) -> Option<PathBuf> {
    let data_dir = app.path().app_data_dir().ok()?;
    Some(data_dir.join("backups").join(root_backup_id(root)))
}

#[tauri::command]
pub(crate) async fn list_backups(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<Vec<BackupInfo>, String> {
    let root = slot_for_context(&state)?.root_key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(base) = backup_base_for_root(&app, &root) else {
            return Vec::new();
        };
        list_backups_from_base(&base, &root)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Every snapshot directory in both namespaces, schema 4 first; a name in
/// both (only an older build's same-second snapshot) is the schema-4 one.
fn snapshot_dirs(base: &std::path::Path) -> Vec<(String, PathBuf)> {
    let mut seen = std::collections::BTreeSet::new();
    [cas_dir(base).join(CAS_SNAPSHOTS), base.to_path_buf()]
        .iter()
        .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .filter(|(name, _)| !name.starts_with(".partial-") && seen.insert(name.clone()))
        .collect()
}

/// The snapshots a restore can select, newest first. Listing reads only
/// manifests: a listed snapshot's blobs are verified when it is restored.
fn list_backups_from_base(base: &std::path::Path, root: &std::path::Path) -> Vec<BackupInfo> {
    let current_root = Store::canonical_root(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string();
    let mut out: Vec<BackupInfo> = snapshot_dirs(base)
        .into_iter()
        .filter_map(|(stamp, dir)| {
            let manifest = read_manifest(&dir)?;
            (manifest.root == current_root).then(|| BackupInfo {
                stamp,
                files: manifest.files.len(),
            })
        })
        .collect();
    out.sort_by(|a, b| snapshot_order(&b.stamp).cmp(&snapshot_order(&a.stamp))); // newest first
    out
}

/// A snapshot the keep-count leaves alone: another Tine wrote it, or it is
/// a valid anchor (og-backup-cas D2). A damaged anchor counts as routine.
fn keep_count_exempt(dir: &std::path::Path) -> bool {
    let Some((manifest, checksum_ok)) = manifest_json(dir) else {
        return false;
    };
    if checksum_ok && manifest.get("anchor") == Some(&serde_json::Value::Bool(true)) {
        return true;
    }
    match manifest.get("schema").and_then(serde_json::Value::as_u64) {
        None => false,
        Some(schema) if schema == u64::from(LEGACY_SNAPSHOT_SCHEMA) => false,
        Some(schema)
            if schema == u64::from(GRAPH_COPY_SNAPSHOT_SCHEMA)
                || schema == u64::from(SNAPSHOT_SCHEMA) =>
        {
            manifest.get("writer").and_then(serde_json::Value::as_str) != Some(SNAPSHOT_WRITER)
        }
        Some(_) => true,
    }
}

/// Prune outside a snapshot write (a lowered keep-count), under the
/// namespace lock; when another process holds it past the wait, the next
/// launch applies the keep-count instead.
fn prune_now(base: &std::path::Path, keep: usize) {
    let cas = cas_dir(base);
    let Ok(Some(_lock)) = lock_namespace(&cas, Some(CAS_LOCK_WAIT)) else {
        return;
    };
    let crashed = has_partials(&cas.join(CAS_SNAPSHOTS));
    if prune_backups(base, keep) > 0 || crashed {
        report_collection(collect::collect(&cas));
    }
}

/// Apply the keep-count across both namespaces, newest `keep` by name; the
/// caller holds the namespace lock. Only routine snapshots this build wrote
/// count: tagged snapshots (e.g. "...-pre-restore") are deliberate safety
/// points and are never auto-pruned, anchors are never pruned, and another
/// Tine's snapshots are not ours to delete. Returns how many schema-4
/// snapshots it deleted; their blobs wait for the collector (a crash
/// mid-delete leaves a directory without a manifest, which it removes).
fn prune_backups(base: &std::path::Path, keep: usize) -> usize {
    let snapshots = cas_dir(base).join(CAS_SNAPSHOTS);
    let mut dirs: Vec<(String, PathBuf)> = snapshot_dirs(base)
        .into_iter()
        .filter(|(name, dir)| {
            !name.contains("-pre-restore")
                // A snapshot another Tine sharing this app-data dir wrote
                // (master's schema 3, which carries no og writer mark;
                // docs/app-identity.md) is listed and restorable here but is
                // not ours to count or delete.
                && !keep_count_exempt(dir)
        })
        .collect();
    dirs.sort_by(|(a, _), (b, _)| snapshot_order(a).cmp(&snapshot_order(b)));
    let doomed = &dirs[..dirs.len().saturating_sub(keep)];
    for (_, dir) in doomed {
        let _ = std::fs::remove_dir_all(dir);
    }
    doomed
        .iter()
        .filter(|(_, dir)| dir.parent() == Some(snapshots.as_path()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tine-tauri-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A released Tine sharing this app-data dir (docs/app-identity.md) writes
    /// schema-3 snapshots without this build's writer mark. They list and
    /// restore here, but the launch keep-count must never delete them: they
    /// are that Tine's backups. This build's own schema-2 and marked schema-3
    /// snapshots are the ones the keep-count counts.
    #[test]
    fn prune_never_deletes_another_tines_snapshots() {
        let base = scratch("backup-prune-foreign");
        let snapshot = |name: &str, schema: u32, writer: &str| {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(SNAPSHOT_MANIFEST),
                format!(r#"{{"schema":{schema},"root":"/g","journals_dir":"journals","pages_dir":"pages",{writer}"files":[],"complete":true}}"#),
            )
            .unwrap();
        };
        let ours = format!(r#""writer":"{SNAPSHOT_WRITER}","#);
        snapshot("2026-09-01_00-00-00", 3, "");
        snapshot("2026-09-02_00-00-00", LEGACY_SNAPSHOT_SCHEMA, "");
        snapshot("2026-09-03_00-00-00", 3, r#""writer":"master","#);
        snapshot("2026-09-04_00-00-00", SNAPSHOT_SCHEMA, &ours);
        snapshot("2026-09-05_00-00-00", SNAPSHOT_SCHEMA, &ours);

        prune_backups(&base, 2);

        let mut left: Vec<String> = std::fs::read_dir(&base)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "2026-09-01_00-00-00",
                "2026-09-03_00-00-00",
                "2026-09-04_00-00-00",
                "2026-09-05_00-00-00"
            ]
        );
        let _ = std::fs::remove_dir_all(base);
    }

    /// Read one area into a fresh blob store the way `write_snapshot` does.
    fn copy_area(
        store: &Store,
        area: Area,
        blobs: &std::path::Path,
        include: fn(&tine_store::FileId) -> bool,
        cancelled: &dyn Fn() -> bool,
    ) -> (usize, usize, Option<BackupFailure>, Vec<SnapshotFile>) {
        let mut files = Vec::new();
        let (copied, failed, failure) =
            copy_store_area(store, area, blobs, "area", include, &mut files, cancelled);
        (copied, failed, failure, files)
    }

    /// The bytes a listed file's blob holds.
    fn blob_bytes(blobs: &std::path::Path, files: &[SnapshotFile], path: &str) -> Vec<u8> {
        let file = files.iter().find(|file| file.path == path).unwrap();
        std::fs::read(blobs.join(&file.sha256)).unwrap()
    }

    /// A small graph: two pages, a journal, config and an asset sidecar.
    fn small_graph(graph: &std::path::Path) {
        for (rel, bytes) in [
            ("pages/A.md", "- a\n"),
            ("pages/B.md", "- b\n"),
            ("journals/2026_10_10.md", "- j\n"),
            ("logseq/config.edn", "{}\n"),
            ("assets/doc.edn", "{:a 1}\n"),
        ] {
            std::fs::create_dir_all(graph.join(rel).parent().unwrap()).unwrap();
            std::fs::write(graph.join(rel), bytes).unwrap();
        }
    }

    fn snapshot_now(
        base: &std::path::Path,
        graph: &std::path::Path,
        suffix: &str,
    ) -> (Vec<&'static str>, PathBuf) {
        // Snapshot names have one-second resolution; a same-second name gets
        // a counter, so the newest is the last published one.
        let (store, _, _) = Store::open(graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, graph).unwrap();
        BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
        take_syncs();
        let outcome = write_snapshot(base, &store, source, suffix, false, &|| false);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
        store.close();
        let published = cas_dir(base)
            .join(CAS_SNAPSHOTS)
            .join(outcome.published.unwrap());
        (ops, published)
    }

    /// The syncs this thread completed since the last call (og-backup-cas D6).
    fn take_syncs() -> Vec<(Synced, PathBuf)> {
        SYNCS.with(|syncs| std::mem::take(&mut *syncs.borrow_mut()))
    }

    /// The exact syncs a durable publication of `published` makes, in order
    /// (og-backup-cas D4/D6): each listed blob once, `blobs/`, the manifest,
    /// the unpublished directory, then `snapshots/`, `<id>.cas/`, `backups/`
    /// and the app-data directory, leaf to root.
    fn durable_chain(
        base: &std::path::Path,
        published: &std::path::Path,
        manifest: &SnapshotManifest,
    ) -> Vec<(Synced, PathBuf)> {
        let cas = cas_dir(base);
        let snapshots = cas.join(CAS_SNAPSHOTS);
        let name = published.file_name().unwrap().to_str().unwrap();
        let partial = snapshots.join(format!(".partial-{name}"));
        let listed: std::collections::BTreeSet<&str> = manifest
            .files
            .iter()
            .map(|file| file.sha256.as_str())
            .collect();
        let mut chain: Vec<_> = listed
            .into_iter()
            .map(|sha256| (Synced::File, cas.join(BLOB_DIR).join(sha256)))
            .collect();
        chain.push((Synced::Dir, cas.join(BLOB_DIR)));
        chain.push((Synced::File, partial.join(SNAPSHOT_MANIFEST)));
        chain.push((Synced::Dir, partial));
        let backups = base.parent().unwrap().to_path_buf();
        let app_data = backups.parent().unwrap().to_path_buf();
        for dir in [snapshots, cas, backups, app_data] {
            chain.push((Synced::Dir, dir));
        }
        chain
    }

    /// One launch backup through `backup_locked`: its outcome, published
    /// snapshot (if any), ops and syncs.
    fn launch(
        base: &std::path::Path,
        graph: &std::path::Path,
        keep: usize,
    ) -> (
        BackupOutcome,
        Option<PathBuf>,
        Vec<&'static str>,
        Vec<(Synced, PathBuf)>,
    ) {
        let (store, _, _) = Store::open(graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, graph).unwrap();
        BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
        take_syncs();
        let outcome = backup_locked(base, &store, source, "", keep, &|| false);
        store.close();
        let published = outcome
            .published
            .as_ref()
            .map(|name| cas_dir(base).join(CAS_SNAPSHOTS).join(name));
        let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
        (outcome, published, ops, take_syncs())
    }

    fn sha256_of(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Restated for the content-addressed design (relaxation ledger: the old
    /// test asserted per-file payload and directory fsyncs before every
    /// publication). A routine launch snapshot publishes its manifest only
    /// after every blob write and syncs nothing; restore verifies every
    /// listed blob. Durable publication: `a_durable_snapshot_syncs_*`.
    #[test]
    fn publication_follows_every_blob_write_and_restore_verifies_every_blob() {
        let root = scratch("backup-publication-order");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let (ops, launch) = snapshot_now(&base, &graph, "");
        let position = |ops: &[&str], name| ops.iter().position(|op| *op == name).unwrap();
        let last = |ops: &[&str], name| ops.iter().rposition(|op| *op == name).unwrap();
        assert_eq!(ops.iter().filter(|op| **op == "blob_write").count(), 5);
        assert!(last(&ops, "blob_write") < position(&ops, "manifest_write"));
        assert!(
            position(&ops, "manifest_write") < position(&ops, "publish_rename"),
            "I-1/I-2: backup publication follows every blob write; exemplar backup.rs publish_snapshot"
        );
        assert_eq!(take_syncs(), [], "a routine launch snapshot syncs nothing");
        let manifest = read_manifest(&launch).unwrap();
        assert_eq!(
            std::fs::read_dir(&launch).unwrap().count(),
            1,
            "a schema-4 snapshot directory holds only its manifest"
        );
        PAYLOAD_HASH_READS.with(|reads| reads.set(0));
        assert!(verify_snapshot(&launch, &manifest));
        assert_eq!(
            PAYLOAD_HASH_READS.with(|reads| reads.get()),
            manifest.files.len(),
            "restore verifies every listed blob"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D4/D6, same process: a snapshot before a rewrite or
    /// restore syncs every blob it lists, even ones an earlier launch wrote
    /// without a sync, then the manifest, then every directory from its own
    /// to the app-data directory, each through the sync seam, in order, with
    /// the rename after the unpublished directory's sync. The directory
    /// syncs reach the store's directory helper.
    #[test]
    fn a_durable_snapshot_syncs_its_whole_chain_after_a_launch_in_this_process() {
        let root = scratch("backup-durable-chain");
        let graph = root.join("graph");
        let base = root.join("app-data").join("backups").join("graph-id");
        small_graph(&graph);
        snapshot_now(&base, &graph, "");
        tine_store::directory_durability::take_synced_directories();
        let (ops, published) = snapshot_now(&base, &graph, "pre-restore");
        let manifest = read_manifest(&published).unwrap();
        let chain = durable_chain(&base, &published, &manifest);
        assert_eq!(
            take_syncs(),
            chain,
            "I-1/I-2: a durable snapshot syncs blobs, blobs/, manifest, then each directory leaf to root; exemplar backup.rs publish_snapshot"
        );
        let synced_before_rename = ops
            .iter()
            .take_while(|op| **op != "publish_rename")
            .filter(|op| **op == "sync")
            .count();
        assert_eq!(synced_before_rename, chain.len() - 4, "{ops:?}");
        let dirs = tine_store::directory_durability::take_synced_directories();
        for (kind, path) in &chain {
            if *kind == Synced::Dir {
                assert!(
                    dirs.contains(path),
                    "{} reached the directory helper",
                    path.display()
                );
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D4, other process: a durable snapshot syncs the blobs
    /// and directories another process's routine launch wrote, every time.
    #[test]
    fn a_durable_snapshot_syncs_its_whole_chain_after_a_launch_in_another_process() {
        let root = scratch("backup-durable-chain-process");
        let graph = root.join("graph");
        let base = root.join("app-data").join("backups").join("graph-id");
        small_graph(&graph);
        let (anchor, ..) = launch(&base, &graph, 12);
        assert!(anchor.failure.is_none());
        std::fs::write(graph.join("pages/A.md"), "- a from the other process\n").unwrap();
        let child = ChildBackup::spawn(&graph, &base, ":never");
        assert_eq!(child.finish(), "done None");
        let routine = cas_snapshots(&base).pop().unwrap();
        assert!(
            !read_manifest(&routine).unwrap().anchor,
            "the other process's launch is routine"
        );
        let (published_name, syncs) = {
            let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
            let source = BackupSource::from_store(&store, &graph).unwrap();
            take_syncs();
            let outcome = backup_locked(&base, &store, source, "pre-restore", 12, &|| false);
            store.close();
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
            (outcome.published.unwrap(), take_syncs())
        };
        let published = cas_dir(&base).join(CAS_SNAPSHOTS).join(published_name);
        let manifest = read_manifest(&published).unwrap();
        assert!(manifest
            .files
            .iter()
            .any(|file| file.sha256 == sha256_of(b"- a from the other process\n")));
        assert_eq!(syncs, durable_chain(&base, &published, &manifest));
        let _ = std::fs::remove_dir_all(root);
    }

    /// The cost the content-addressed store exists for: an unchanged launch
    /// writes one manifest and no blob, and edits write only their own blobs.
    #[test]
    fn unchanged_launch_writes_no_blob_and_edits_write_only_theirs() {
        let root = scratch("backup-unchanged-launch");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let count = |ops: &[&str], name| ops.iter().filter(|op| **op == name).count();
        let (ops, _) = snapshot_now(&base, &graph, "");
        assert_eq!(count(&ops, "blob_write"), 5);
        let (ops, _) = snapshot_now(&base, &graph, "");
        assert_eq!(
            (count(&ops, "blob_write"), count(&ops, "manifest_write")),
            (0, 1),
            "an unchanged graph writes one manifest"
        );
        std::fs::write(graph.join("pages/A.md"), "- a edited\n").unwrap();
        std::fs::write(graph.join("journals/2026_10_10.md"), "- j edited\n").unwrap();
        let (ops, latest) = snapshot_now(&base, &graph, "");
        assert_eq!(count(&ops, "blob_write"), 2);
        assert!(verify_snapshot(&latest, &read_manifest(&latest).unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Power loss after an unsynced blob write can leave the blob's name with
    /// torn bytes (zero length, or zeroed at full length). The next snapshot
    /// of that content repairs it instead of reusing it, so the torn blob
    /// does not damage every later snapshot, and the older snapshot that
    /// lists it verifies again.
    #[test]
    fn a_torn_blob_is_repaired_not_reused() {
        let root = scratch("backup-torn-blob");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let (_, first) = snapshot_now(&base, &graph, "");
        let manifest = read_manifest(&first).unwrap();
        let page = manifest
            .files
            .iter()
            .find(|file| file.path == "graph/pages/A.md")
            .unwrap();
        let config = manifest
            .files
            .iter()
            .find(|file| file.path == "logseq/config.edn")
            .unwrap();
        let blobs = cas_dir(&base).join(BLOB_DIR);
        std::fs::write(blobs.join(&page.sha256), b"").unwrap();
        std::fs::write(blobs.join(&config.sha256), [0u8; 3]).unwrap();
        assert!(!verify_snapshot(&first, &manifest));
        let (ops, latest) = snapshot_now(&base, &graph, "");
        assert_eq!(
            ops.iter().filter(|op| **op == "blob_repair").count(),
            2,
            "{ops:?}"
        );
        assert!(verify_snapshot(&latest, &read_manifest(&latest).unwrap()));
        assert!(verify_snapshot(&first, &manifest));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A blob two snapshots share survives pruning one of them; a blob only
    /// the pruned snapshot listed is collected; the blob store itself is
    /// never counted or pruned as a snapshot.
    #[test]
    fn a_shared_blob_survives_pruning_one_of_its_snapshots() {
        let root = scratch("backup-shared-blob");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let (_, first) = snapshot_now(&base, &graph, "");
        let old = read_manifest(&first).unwrap();
        std::fs::write(graph.join("pages/B.md"), "- b edited\n").unwrap();
        let (_, second) = snapshot_now(&base, &graph, "");
        let hash = |manifest: &SnapshotManifest, path: &str| {
            manifest
                .files
                .iter()
                .find(|file| file.path == path)
                .unwrap()
                .sha256
                .clone()
        };
        let (shared, old_b) = (
            hash(&old, "graph/pages/A.md"),
            hash(&old, "graph/pages/B.md"),
        );
        prune_now(&base, 1);
        assert!(!first.exists() && second.exists());
        let blobs = cas_dir(&base).join(BLOB_DIR);
        assert!(blobs.join(&shared).is_file());
        assert!(!blobs.join(&old_b).exists());
        assert!(verify_snapshot(&second, &read_manifest(&second).unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A prune that races a snapshot in another thread waits for the
    /// namespace lock, so it never deletes blobs the snapshot wrote before
    /// publishing its manifest. Driven by the snapshot's cancellation hook,
    /// which runs between files.
    #[test]
    fn a_prune_racing_a_snapshot_never_deletes_its_blobs() {
        use std::sync::atomic::AtomicUsize;
        let root = scratch("backup-gc-race");
        let graph = root.join("graph");
        let base = root.join("backups");
        let cas = cas_dir(&base);
        small_graph(&graph);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let (entered, release) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        let calls = AtomicUsize::new(0);
        let (pruned_tx, pruned_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let snapshot = scope.spawn(|| {
                backup_locked(&base, &store, source, "", 12, &|| {
                    // Pause after the first blob is written, manifest unwritten.
                    if calls.fetch_add(1, Ordering::SeqCst) == 2 {
                        entered.wait();
                        release.wait();
                    }
                    false
                })
            });
            entered.wait();
            let written = std::fs::read_dir(cas.join(BLOB_DIR)).unwrap().count();
            assert!(written >= 1, "the snapshot has written a blob");
            let base = &base;
            let prune = scope.spawn(move || {
                prune_now(base, 1);
                let _ = pruned_tx.send(());
            });
            // Observe, release, then assert, so a failure cannot hang the test.
            let pruned_early = pruned_rx
                .recv_timeout(std::time::Duration::from_millis(200))
                .is_ok();
            let blobs_now = std::fs::read_dir(cas.join(BLOB_DIR)).unwrap().count();
            release.wait();
            assert!(snapshot.join().unwrap().failure.is_none());
            prune.join().unwrap();
            assert!(!pruned_early, "the prune waits for the snapshot");
            assert_eq!(
                blobs_now, written,
                "no blob of the unpublished snapshot is collected"
            );
        });
        let published = cas_snapshots(&base);
        assert_eq!(published.len(), 1);
        assert!(verify_snapshot(
            &published[0],
            &read_manifest(&published[0]).unwrap()
        ));
        store.close();
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D2: the first backup is an anchor, published with the
    /// full durable chain; later routine launches sync nothing; neither the
    /// launch keep-count nor a keep-count change ever prunes the anchor.
    #[test]
    fn the_first_backup_is_a_durable_anchor_the_keep_count_never_prunes() {
        let root = scratch("backup-anchor-first");
        let graph = root.join("graph");
        let base = root.join("app-data").join("backups").join("graph-id");
        small_graph(&graph);
        let (outcome, anchor, _, syncs) = launch(&base, &graph, 1);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let anchor = anchor.unwrap();
        let manifest = read_manifest(&anchor).unwrap();
        assert!(manifest.anchor && manifest.created_unix.is_some());
        assert_eq!(
            syncs,
            durable_chain(&base, &anchor, &manifest),
            "the first backup pays the full sync"
        );
        for edit in 0..3 {
            std::fs::write(graph.join("pages/A.md"), format!("- a {edit}\n")).unwrap();
            let (outcome, latest, _, syncs) = launch(&base, &graph, 1);
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
            let latest = latest.unwrap();
            assert!(!read_manifest(&latest).unwrap().anchor);
            assert_eq!(syncs, [], "a routine launch syncs nothing");
            let left = cas_snapshots(&base);
            assert_eq!(left[0], anchor, "the anchor is never pruned");
            assert_eq!(left.len(), 2, "keep 1 plus the anchor");
            assert_eq!(left.last(), Some(&latest));
        }
        prune_now(&base, 1);
        assert_eq!(cas_snapshots(&base).len(), 2);
        assert_eq!(
            cas_snapshots(&base)[0],
            anchor,
            "a keep-count change keeps the anchor"
        );
        assert!(verify_snapshot(&anchor, &manifest));
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D2: an anchor 7 days old by its recorded creation time,
    /// in either clock direction, is replaced at the next launch: the new
    /// anchor is published durably and verified, then the old one goes.
    #[test]
    fn a_week_old_anchor_rotates_after_its_successor_is_durable() {
        let root = scratch("backup-anchor-rotate");
        let graph = root.join("graph");
        let base = root.join("app-data").join("backups").join("graph-id");
        small_graph(&graph);
        let (_, first, ..) = launch(&base, &graph, 12);
        assert!(read_manifest(&first.unwrap()).unwrap().anchor);
        let (_, routine, _, syncs) = launch(&base, &graph, 12);
        assert_eq!(syncs, [], "a fresh anchor does not rotate");
        let routine = routine.unwrap();
        for (shift, label) in [(8 * 86_400, "clock forward"), (0, "clock back")] {
            CLOCK_SHIFT.with(|clock| clock.set(shift));
            let before = cas_snapshots(&base);
            let (outcome, rotated, ops, syncs) = launch(&base, &graph, 12);
            assert!(outcome.failure.is_none(), "{label}: {:?}", outcome.failure);
            let rotated = rotated.unwrap();
            let manifest = read_manifest(&rotated).unwrap();
            assert!(manifest.anchor, "{label}");
            assert_eq!(syncs, durable_chain(&base, &rotated, &manifest), "{label}");
            assert!(ops.contains(&"anchor_verified"), "{label}: {ops:?}");
            let anchors: Vec<_> = cas_snapshots(&base)
                .into_iter()
                .filter(|dir| read_manifest(dir).unwrap().anchor)
                .collect();
            assert_eq!(
                anchors,
                [rotated.clone()],
                "{label}: exactly the newest anchor"
            );
            assert!(!before.contains(&rotated), "{label}: a new anchor");
            assert!(before.iter().any(|dir| dir == &routine) && routine.is_dir());
        }
        CLOCK_SHIFT.with(|clock| clock.set(0));
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D2: a new anchor that fails verification is removed and
    /// the old anchor stays; the next launch rotates.
    #[test]
    fn a_new_anchor_that_fails_verification_never_replaces_the_old_one() {
        let root = scratch("backup-anchor-verify");
        let graph = root.join("graph");
        let base = root.join("app-data").join("backups").join("graph-id");
        small_graph(&graph);
        let (_, old, ..) = launch(&base, &graph, 12);
        let old = old.unwrap();
        let edited = b"- a edited for the new anchor\n";
        std::fs::write(graph.join("pages/A.md"), edited).unwrap();
        CLOCK_SHIFT.with(|clock| clock.set(8 * 86_400));
        let blob = cas_dir(&base).join(BLOB_DIR).join(sha256_of(edited));
        let torn = blob.clone();
        PAUSE_AT.with(|hook| {
            *hook.borrow_mut() = Some((
                "anchor_published",
                Box::new(move || std::fs::write(&torn, b"bit rot").unwrap()),
            ))
        });
        let (outcome, ..) = launch(&base, &graph, 12);
        assert_eq!(
            outcome.failure.as_ref().map(|failure| failure.phase),
            Some("anchor")
        );
        assert_eq!(
            cas_snapshots(&base),
            [old.clone()],
            "the failed anchor is gone, the old one stays"
        );
        assert!(verify_snapshot(&old, &read_manifest(&old).unwrap()));
        let (outcome, rotated, ..) = launch(&base, &graph, 12);
        CLOCK_SHIFT.with(|clock| clock.set(0));
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        assert_eq!(cas_snapshots(&base), [rotated.unwrap()]);
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D2 across processes: an anchor another process published
    /// is never pruned by this process's keep-count.
    #[test]
    fn another_processs_anchor_survives_this_processs_keep_count() {
        let root = scratch("backup-anchor-process");
        let graph = root.join("graph");
        let base = root.join("app-data").join("backups").join("graph-id");
        small_graph(&graph);
        let child = ChildBackup::spawn(&graph, &base, ":never");
        assert_eq!(child.finish(), "done None");
        let anchor = cas_snapshots(&base).pop().unwrap();
        assert!(read_manifest(&anchor).unwrap().anchor);
        for edit in 0..3 {
            std::fs::write(graph.join("pages/B.md"), format!("- b {edit}\n")).unwrap();
            let (outcome, ..) = launch(&base, &graph, 1);
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        }
        let left = cas_snapshots(&base);
        assert_eq!((left.len(), &left[0]), (2, &anchor));
        assert!(verify_snapshot(&anchor, &read_manifest(&anchor).unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// REVIEW N5: same-second counters order numerically, `-10` after `-9`,
    /// for the keep-count and the listing alike, across both namespaces.
    /// og-backup-cas D6: a file `fsync` cannot be observed in-process, so
    /// the backup module keeps exactly one file-sync and one directory-sync
    /// call, both inside `sync_durable`, the seam every durable step goes
    /// through and the tests record. A second site would be unrecorded.
    #[test]
    fn every_backup_sync_goes_through_the_one_seam() {
        let production =
            |source: &'static str| source.split("\n#[cfg(test)]\nmod tests {").next().unwrap();
        let sources = [
            production(include_str!("backup.rs")),
            production(include_str!("backup/collect.rs")),
            production(include_str!("backup/naming.rs")),
            production(include_str!("backup/restore.rs")),
        ];
        let seam = sources[0]
            .split("\nfn sync_durable(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("sync_durable exists");
        for (call, count_in_seam) in [
            (".sync_all(", 1),
            (".sync_data(", 0),
            ("sync_directory_entry(", 1),
            ("sync_private_directory(", 0),
        ] {
            let total: usize = sources
                .iter()
                .map(|source| source.matches(call).count())
                .sum();
            assert_eq!(
                (total, seam.matches(call).count()),
                (count_in_seam, count_in_seam),
                "I-1/D6: `{call}` in the backup module must be exactly the sync_durable seam's; exemplar backup.rs sync_durable"
            );
        }
    }

    /// A same-second name is reserved above every counter that second
    /// already has, published or partial: after a prune freed `<s>` and
    /// `<s>-2` and left `<s>-3`, the next snapshot is `<s>-4` and sorts
    /// newest (reusing `<s>` would order it oldest, and the keep-count would
    /// then delete the newest snapshot).
    #[test]
    fn a_same_second_name_never_reuses_a_freed_lower_counter() {
        let root = scratch("backup-counter-reuse");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let snapshots = cas_dir(&base).join(CAS_SNAPSHOTS);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let snapshot = || {
            let source = BackupSource::from_store(&store, &graph).unwrap();
            let outcome = write_snapshot(&base, &store, source, "", false, &|| false);
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
            outcome.published.unwrap()
        };
        // Each attempt takes milliseconds; one that straddles a second
        // boundary gets a fresh stamp and is retried.
        for _attempt in 0..10 {
            let _ = std::fs::remove_dir_all(cas_dir(&base));
            let stamp = snapshot();
            std::fs::rename(snapshots.join(&stamp), snapshots.join(format!("{stamp}-3"))).unwrap();
            let next = snapshot();
            if !next.starts_with(&stamp) {
                continue;
            }
            assert_eq!(next, format!("{stamp}-4"));
            assert_eq!(
                cas_snapshots(&base).last(),
                Some(&snapshots.join(&next)),
                "sorts newest"
            );
            std::fs::create_dir(snapshots.join(format!(".partial-{stamp}-6"))).unwrap();
            let after = snapshot();
            if !after.starts_with(&stamp) {
                continue;
            }
            assert_eq!(after, format!("{stamp}-7"), "a partial's counter counts");
            store.close();
            let _ = std::fs::remove_dir_all(root);
            return;
        }
        panic!("ten attempts each crossed a second boundary");
    }

    #[test]
    fn same_second_counters_order_numerically() {
        let root = scratch("backup-counter-order");
        let graph = root.join("graph");
        let base = root.join("backups");
        std::fs::create_dir_all(&graph).unwrap();
        let canonical = std::fs::canonicalize(&graph).unwrap().display().to_string();
        let manifest = |schema| SnapshotManifest {
            schema,
            root: canonical.clone(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            graph_text_policy: Some(SnapshotGraphTextPolicy {
                version: GRAPH_TEXT_SCOPE_VERSION,
                hidden: Vec::new(),
                hidden_parse_failed_closed: false,
            }),
            writer: Some(SNAPSHOT_WRITER.into()),
            files: Vec::new(),
            complete: true,
            anchor: false,
            created_unix: None,
        };
        let snapshots = cas_dir(&base).join(CAS_SNAPSHOTS);
        for name in [
            "2026-10-10_01-00-00-10",
            "2026-10-10_01-00-00",
            "2026-10-10_01-00-00-9",
        ] {
            std::fs::create_dir_all(snapshots.join(name)).unwrap();
            write_manifest(&snapshots.join(name), &manifest(SNAPSHOT_SCHEMA)).unwrap();
        }
        let legacy = base.join("2026-10-09_23-00-00-11");
        std::fs::create_dir_all(&legacy).unwrap();
        write_manifest(&legacy, &manifest(GRAPH_COPY_SNAPSHOT_SCHEMA)).unwrap();
        let listed = || {
            list_backups_from_base(&base, &graph)
                .into_iter()
                .map(|info| info.stamp)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            listed(),
            [
                "2026-10-10_01-00-00-10",
                "2026-10-10_01-00-00-9",
                "2026-10-10_01-00-00",
                "2026-10-09_23-00-00-11"
            ]
        );
        prune_backups(&base, 1);
        assert_eq!(listed(), ["2026-10-10_01-00-00-10"]);
        let _ = std::fs::remove_dir_all(root);
    }

    /// REVIEW F2 as amended by og-backup-cas D3: a launch that deleted
    /// nothing never enumerates the blob store. One whose keep-count deleted
    /// a snapshot, or that found a crashed partial, runs the collector, which
    /// removes exactly the blobs and temps no manifest lists.
    #[test]
    fn launches_that_delete_nothing_never_enumerate_the_blob_store() {
        let root = scratch("backup-no-blob-scan");
        let graph = root.join("graph");
        let base = root.join("backups");
        let cas = cas_dir(&base);
        small_graph(&graph);
        for edit in 0..3 {
            std::fs::write(graph.join("pages/A.md"), format!("- a {edit}\n")).unwrap();
            let (outcome, _, ops, _) = launch(&base, &graph, 2);
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
            assert!(!ops.contains(&"blob_dir_scan"), "launch {edit}: {ops:?}");
        }
        std::fs::write(graph.join("pages/A.md"), "- a 3\n").unwrap();
        let (_, _, ops, _) = launch(&base, &graph, 2);
        assert!(
            ops.contains(&"blob_dir_scan"),
            "a deletion runs the collector: {ops:?}"
        );
        assert_eq!(
            ops.iter().filter(|op| **op == "blob_collect").count(),
            1,
            "only the pruned snapshot's own page blob: {ops:?}"
        );
        assert!(!cas.join(BLOB_DIR).join(sha256_of(b"- a 1\n")).exists());
        // Anchor + two routine snapshots: 4 shared blobs and 3 page versions.
        assert_eq!(
            std::fs::read_dir(cas.join(BLOB_DIR)).unwrap().count(),
            4 + 3
        );

        // A crash left a partial and a temp: the next launch collects both.
        std::fs::create_dir_all(cas.join(CAS_SNAPSHOTS).join(".partial-crashed")).unwrap();
        std::fs::write(cas.join(BLOB_DIR).join(".tmp-crashed"), b"half").unwrap();
        let (_, _, ops, _) = launch(&base, &graph, 12);
        assert!(ops.contains(&"blob_dir_scan"), "{ops:?}");
        assert!(!cas.join(BLOB_DIR).join(".tmp-crashed").exists());
        assert!(!cas.join(CAS_SNAPSHOTS).join(".partial-crashed").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D6 (REVIEW N6): any sync of a durable publication that
    /// fails fails that snapshot (so the rewrite or restore is refused);
    /// before the rename nothing is published.
    #[test]
    fn a_failed_sync_fails_a_snapshot_taken_before_a_rewrite() {
        let probe = scratch("backup-sync-fault-probe");
        small_graph(&probe.join("graph"));
        let (_, published) =
            snapshot_now(&probe.join("backups"), &probe.join("graph"), "pre-restore");
        let steps = durable_chain(
            &probe.join("backups"),
            &published,
            &read_manifest(&published).unwrap(),
        )
        .len();
        let _ = std::fs::remove_dir_all(probe);
        for step in 0..steps {
            let root = scratch(&format!("backup-sync-fault-{step}"));
            let graph = root.join("graph");
            let base = root.join("backups");
            small_graph(&graph);
            let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
            let source = BackupSource::from_store(&store, &graph).unwrap();
            take_syncs();
            FAIL_SYNC.with(|fail| fail.set(Some(step)));
            let outcome = write_snapshot(&base, &store, source, "pre-restore", false, &|| false);
            FAIL_SYNC.with(|fail| fail.set(None));
            store.close();
            assert_eq!(
                take_syncs().len(),
                step,
                "step {step}: the failed sync is not observed"
            );
            assert_eq!(
                outcome.failure.as_ref().map(|failure| failure.phase),
                Some("publish"),
                "step {step}"
            );
            assert!(rewrite_snapshot_result(outcome).is_err(), "step {step}");
            let entries = std::fs::read_dir(cas_dir(&base).join(CAS_SNAPSHOTS))
                .unwrap()
                .count();
            let published = usize::from(step >= steps - 4);
            assert_eq!(
                entries, published,
                "step {step}: no partial, published only after the rename"
            );
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// REVIEW B1: the checksum covers every manifest field in a canonical
    /// form. Pinned, so a change of form is a deliberate format change.
    #[test]
    fn the_manifest_checksum_is_pinned() {
        let root = scratch("backup-checksum-pin");
        let manifest = SnapshotManifest {
            schema: SNAPSHOT_SCHEMA,
            root: "/g".into(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            graph_text_policy: Some(SnapshotGraphTextPolicy {
                version: GRAPH_TEXT_SCOPE_VERSION,
                hidden: vec!["private".into()],
                hidden_parse_failed_closed: false,
            }),
            writer: Some(SNAPSHOT_WRITER.into()),
            files: vec![SnapshotFile {
                path: "graph/pages/A.md".into(),
                sha256: "0".repeat(64),
            }],
            complete: true,
            anchor: false,
            created_unix: None,
        };
        write_manifest(&root, &manifest).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(SNAPSHOT_MANIFEST)).unwrap()).unwrap();
        // Independently: Python `json.dumps(m, sort_keys=True, separators=(",", ":"))`.
        assert_eq!(
            value[MANIFEST_CHECKSUM],
            "ad39ef957dde5983d530aace08391907d2ef25673b7659fcb0f83dc63fd3f059"
        );
        assert!(read_manifest(&root).is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The checksum's canonical form is an explicit recursive key sort, not
    /// `serde_json`'s map order (a `preserve_order` feature anywhere in the
    /// build would change that). Pinned encoding vector with a Unicode
    /// string, escapes and a numeric unknown field, cross-checked with Python
    /// `json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
    #[test]
    fn the_manifest_checksum_canonical_form_is_pinned() {
        let value = serde_json::json!({
            "zeta": 1,
            "files": [{"path": "graph/pages/Č.md", "sha256": "a".repeat(64)}],
            "name": "Příliš žluťoučký kůň 🐎 \"quoted\" \\ tab\t ctrl\u{1}",
            "unknown_number": 12345,
            "nested": {"b": -7, "a": [true, null, "\u{2028}"]},
        });
        let mut bytes = Vec::new();
        canonical_json(&value, &mut bytes);
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.starts_with(r#"{"files":[{"path":"graph/pages/Č.md","#),
            "{text}"
        );
        assert!(
            text.ends_with(
                "\"nested\":{\"a\":[true,null,\"\u{2028}\"],\"b\":-7},\"unknown_number\":12345,\"zeta\":1}"
            ),
            "{text}"
        );
        assert_eq!(
            manifest_checksum(&value),
            "fc6ea8639cd4f2786ad10c743d3112ebc85fa741e72200c5d9325b439071ac20"
        );
    }

    /// A namespace for the collector tests: one routine snapshot and one
    /// planted unreferenced blob a fail-open collector would delete.
    fn collector_fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = scratch(tag);
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let (_, snapshot) = snapshot_now(&base, &graph, "");
        let stray = cas_dir(&base).join(BLOB_DIR).join("f".repeat(64));
        std::fs::write(&stray, b"unreferenced").unwrap();
        (root, base, snapshot, stray)
    }

    /// The collector removed nothing: the snapshot still verifies and the
    /// unreferenced blob is still there.
    fn nothing_collected(snapshot: &std::path::Path, stray: &std::path::Path) -> bool {
        stray.is_file() && verify_snapshot(snapshot, &read_manifest(snapshot).unwrap())
    }

    /// og-backup-cas D3 (REVIEW-backup-cas-2 R2-B2): an error item while
    /// listing `snapshots/` or `blobs/` stops the collection before any
    /// removal.
    #[test]
    fn the_collector_stops_on_a_listing_entry_error() {
        let (root, base, snapshot, stray) = collector_fixture("collect-entry-error");
        let cas = cas_dir(&base);
        for dir in [CAS_SNAPSHOTS, BLOB_DIR] {
            collect::FAIL_LISTING.with(|fail| *fail.borrow_mut() = Some(cas.join(dir)));
            let result = collect::collect(&cas);
            collect::FAIL_LISTING.with(|fail| *fail.borrow_mut() = None);
            assert!(result.is_err(), "{dir}");
            assert!(nothing_collected(&snapshot, &stray), "{dir}");
        }
        assert_eq!(
            collect::collect(&cas).unwrap().removed,
            1,
            "the fixture collects when listing works"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D3: a manifest that cannot be read (here: permission)
    /// stops the collection; its blobs and every other survive.
    #[cfg(unix)]
    #[test]
    fn the_collector_stops_on_a_manifest_read_error() {
        use std::os::unix::fs::PermissionsExt;
        let (root, base, snapshot, stray) = collector_fixture("collect-read-error");
        let manifest = snapshot.join(SNAPSHOT_MANIFEST);
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = collect::collect(&cas_dir(&base));
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
        assert!(nothing_collected(&snapshot, &stray));
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D3: the mark is independent of restore eligibility. A
    /// manifest whose checksum fails, or of an unfamiliar schema, still keeps
    /// every blob it lists (and is reported); valid JSON whose references
    /// cannot be read, or an entry the collector did not create, stops it.
    #[test]
    fn the_collector_keeps_unverifiable_refs_and_stops_on_uninterpretable_ones() {
        let (root, base, snapshot, stray) = collector_fixture("collect-formats");
        let cas = cas_dir(&base);
        let manifest = snapshot.join(SNAPSHOT_MANIFEST);
        let original = std::fs::read_to_string(&manifest).unwrap();
        let flipped = original.replace("graph/pages/A.md", "graph/pages/X.md");
        assert_ne!(flipped, original);
        std::fs::write(&manifest, &flipped).unwrap();
        assert!(read_manifest(&snapshot).is_none(), "the checksum fails");
        let report = collect::collect(&cas).unwrap();
        assert_eq!((report.removed, report.damaged.len()), (1, 1), "{report:?}");
        assert!(!stray.exists());
        let unfamiliar: serde_json::Value = serde_json::from_str(&original).unwrap();
        let unfamiliar =
            serde_json::json!({"schema": 99, "files": unfamiliar["files"].clone(), "extra": 1});
        std::fs::write(&manifest, unfamiliar.to_string()).unwrap();
        std::fs::write(&stray, b"unreferenced").unwrap();
        let report = collect::collect(&cas).unwrap();
        assert_eq!((report.removed, report.damaged.len()), (1, 1), "{report:?}");
        std::fs::write(&manifest, &original).unwrap();
        assert!(
            verify_snapshot(&snapshot, &read_manifest(&snapshot).unwrap()),
            "every listed blob survived"
        );

        std::fs::write(&stray, b"unreferenced").unwrap();
        for malformed in [
            r#"{"files":"none"}"#,
            r#"{"schema":4}"#,
            r#"{"files":[{"sha256":"not-a-digest"}]}"#,
            r#"{"files":[{"path":"graph/x.md"}]}"#,
            r#"[1,2]"#,
        ] {
            std::fs::write(&manifest, malformed).unwrap();
            assert!(collect::collect(&cas).is_err(), "{malformed}");
            assert!(stray.is_file(), "{malformed}: nothing removed");
        }
        std::fs::write(&manifest, &original).unwrap();
        for (unexpected, dir) in [("loose-file", false), ("not-a-digest", true)] {
            let path = if dir {
                cas.join(BLOB_DIR).join(unexpected)
            } else {
                cas.join(CAS_SNAPSHOTS).join(unexpected)
            };
            std::fs::write(&path, b"?").unwrap();
            assert!(collect::collect(&cas).is_err(), "{unexpected}");
            assert!(nothing_collected(&snapshot, &stray), "{unexpected}");
            std::fs::remove_file(&path).unwrap();
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D3: under the lock, a torn manifest (not JSON), a
    /// published directory without a manifest and any unpublished directory
    /// are abandoned: the collector removes them, reports the damage, and
    /// collects the blobs and temps nothing else lists.
    #[test]
    fn the_collector_removes_torn_manifestless_and_abandoned_snapshots() {
        let (root, base, snapshot, stray) = collector_fixture("collect-torn");
        let cas = cas_dir(&base);
        let snapshots = cas.join(CAS_SNAPSHOTS);
        let torn = snapshots.join("2026-01-01_00-00-00");
        std::fs::create_dir_all(&torn).unwrap();
        std::fs::write(torn.join(SNAPSHOT_MANIFEST), r#"{"files":[{"sha"#).unwrap();
        let empty = snapshots.join("2026-01-02_00-00-00");
        std::fs::create_dir_all(&empty).unwrap();
        let partial = snapshots.join(".partial-2026-01-03_00-00-00");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(
            partial.join(SNAPSHOT_MANIFEST),
            &std::fs::read(snapshot.join(SNAPSHOT_MANIFEST)).unwrap(),
        )
        .unwrap();
        let temp = cas.join(BLOB_DIR).join(".tmp-abandoned");
        std::fs::write(&temp, b"half").unwrap();
        let report = collect::collect(&cas).unwrap();
        assert_eq!(report.removed, 2, "{report:?}");
        assert_eq!(report.damaged.len(), 2, "{report:?}");
        assert!(report.failed.is_empty(), "{report:?}");
        for gone in [&torn, &empty, &partial, &temp, &stray] {
            assert!(!gone.exists(), "{}", gone.display());
        }
        assert!(verify_snapshot(
            &snapshot,
            &read_manifest(&snapshot).unwrap()
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    /// R2-B2's fixture (og-backup-cas D3): an unsearchable `snapshots/` fails
    /// the launch's snapshot, and the collection that follows removes nothing
    /// and logs that it stopped.
    #[cfg(unix)]
    #[test]
    fn an_unsearchable_snapshots_dir_fails_the_snapshot_and_collects_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (root, base, snapshot, stray) = collector_fixture("collect-unsearchable");
        let graph = root.join("graph");
        std::fs::write(graph.join("pages/A.md"), "- a edited\n").unwrap();
        let snapshots = cas_dir(&base).join(CAS_SNAPSHOTS);
        std::fs::set_permissions(&snapshots, std::fs::Permissions::from_mode(0o400)).unwrap();
        let (outcome, _, ops, _) = launch(&base, &graph, 12);
        std::fs::set_permissions(&snapshots, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(outcome.failure.is_some(), "the snapshot fails");
        assert!(
            ops.contains(&"collect_failed"),
            "the collection's error is logged: {ops:?}"
        );
        assert!(nothing_collected(&snapshot, &stray));
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D3: a removal that fails stays in the report, is not
    /// fatal, and the next deletion-prune's collection removes that blob.
    #[cfg(unix)]
    #[test]
    fn a_failed_blob_removal_is_retried_by_the_next_deletion() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("collect-unlink-retry");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        for edit in 0..3 {
            std::fs::write(graph.join("pages/A.md"), format!("- a {edit}\n")).unwrap();
            assert!(launch(&base, &graph, 12).0.failure.is_none());
        }
        let blobs = cas_dir(&base).join(BLOB_DIR);
        let leaked = blobs.join(sha256_of(b"- a 1\n"));
        std::fs::set_permissions(&blobs, std::fs::Permissions::from_mode(0o555)).unwrap();
        BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
        prune_now(&base, 1);
        std::fs::set_permissions(&blobs, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(leaked.is_file(), "the removal failed");
        assert_eq!(cas_snapshots(&base).len(), 2, "the prune itself went ahead");
        assert!(!BACKUP_OPS.with(|ops| ops.borrow().contains(&"collect_failed")));
        std::fs::write(graph.join("pages/A.md"), "- a 3\n").unwrap();
        let (outcome, _, ops, _) = launch(&base, &graph, 1);
        assert!(outcome.failure.is_none());
        assert!(ops.contains(&"blob_dir_scan"));
        assert!(
            !leaked.exists(),
            "the next deletion's collection retried it"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Restore staging bound: a blob over the cap is damage (no backup writes
    /// one) and is refused before it is read or hashed.
    #[test]
    fn an_oversized_blob_is_refused_before_it_is_read() {
        let root = scratch("backup-oversized-blob");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let (_, snapshot) = snapshot_now(&base, &graph, "");
        let manifest = read_manifest(&snapshot).unwrap();
        let first = cas_dir(&base)
            .join(BLOB_DIR)
            .join(&manifest.files[0].sha256);
        std::fs::File::options()
            .write(true)
            .open(&first)
            .unwrap()
            .set_len(SNAPSHOT_FILE_MAX_BYTES + 1)
            .unwrap();
        PAYLOAD_HASH_READS.with(|reads| reads.set(0));
        assert!(!verify_snapshot(&snapshot, &manifest));
        assert_eq!(
            PAYLOAD_HASH_READS.with(|reads| reads.get()),
            0,
            "refused before it is read"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The other half of the staging bound: a backup never lists a file over
    /// the cap; it fails with `FileTooLarge` instead.
    #[test]
    fn a_file_over_the_cap_fails_the_backup() {
        let root = scratch("backup-oversized-file");
        let graph = root.join("graph");
        small_graph(&graph);
        std::fs::File::create(graph.join("assets/huge.edn"))
            .unwrap()
            .set_len(SNAPSHOT_FILE_MAX_BYTES + 1)
            .unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = write_snapshot(&root.join("backups"), &store, source, "", false, &|| false);
        store.close();
        let failure = outcome.failure.expect("refused");
        assert_eq!(
            (failure.phase, failure.kind),
            ("assets", ErrorKind::FileTooLarge)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    const CHILD_ENV: &str = "TINE_BACKUP_CAS_CHILD";

    /// The second OS process of the cross-process tests below: one backup
    /// (`<suffix>:<op>` in `TINE_BACKUP_CAS_CHILD`) of `TINE_BACKUP_CAS_GRAPH`
    /// into `TINE_BACKUP_CAS_BASE`, pausing at the op until its parent
    /// writes a line. A no-op in an ordinary run.
    #[test]
    fn cas_child_process() {
        let Ok(mode) = std::env::var(CHILD_ENV) else {
            return;
        };
        let (suffix, pause) = mode.split_once(':').unwrap();
        let var = |name| PathBuf::from(std::env::var_os(name).unwrap());
        let (graph, base) = (var("TINE_BACKUP_CAS_GRAPH"), var("TINE_BACKUP_CAS_BASE"));
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let pause: &'static str = Box::leak(pause.to_owned().into_boxed_str());
        PAUSE_AT.with(|hook| {
            *hook.borrow_mut() = Some((
                pause,
                Box::new(|| {
                    use std::io::Write;
                    println!("CHILD:paused");
                    std::io::stdout().flush().unwrap();
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line).unwrap();
                }),
            ));
        });
        let outcome = backup_locked(&base, &store, source, suffix, BACKUP_KEEP_DEFAULT, &|| {
            false
        });
        println!(
            "CHILD:done {:?}",
            outcome.failure.as_ref().map(BackupFailure::wire)
        );
        store.close();
    }

    /// A backup in a second OS process: this test binary re-executed to run
    /// only `cas_child_process`.
    struct ChildBackup {
        process: std::process::Child,
        out: std::io::BufReader<std::process::ChildStdout>,
    }

    impl ChildBackup {
        fn spawn(graph: &std::path::Path, base: &std::path::Path, mode: &str) -> Self {
            let mut process = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "backup::tests::cas_child_process",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_ENV, mode)
                .env("TINE_BACKUP_CAS_GRAPH", graph)
                .env("TINE_BACKUP_CAS_BASE", base)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let out = std::io::BufReader::new(process.stdout.take().unwrap());
            Self { process, out }
        }

        /// The child's next `CHILD:` line that starts with `prefix`.
        fn line(&mut self, prefix: &str) -> String {
            use std::io::BufRead;
            loop {
                let mut line = String::new();
                assert!(
                    self.out.read_line(&mut line).unwrap() > 0,
                    "the child ended before {prefix}"
                );
                // libtest prints `test <name> ... ` before the test's first line.
                if let Some((_, rest)) = line.trim_end().split_once("CHILD:") {
                    if rest.starts_with(prefix) {
                        return rest.to_owned();
                    }
                }
            }
        }

        fn resume(&mut self) {
            use std::io::Write;
            writeln!(self.process.stdin.as_mut().unwrap(), "go").unwrap();
        }

        /// The child's outcome; it must have run its one test and passed.
        fn finish(mut self) -> String {
            let done = self.line("done");
            let mut rest = String::new();
            self.out.read_to_string(&mut rest).unwrap();
            assert!(self.process.wait().unwrap().success(), "{rest}");
            assert!(rest.contains("1 passed"), "the child ran its test: {rest}");
            done
        }
    }

    /// REVIEW B2, two OS processes: a prune never deletes the blobs of
    /// another process's snapshot, before its manifest is written or after
    /// (written, not yet published). The writer holds the namespace lock;
    /// the prune waits for it. The writer reuses a blob that only the
    /// snapshot the prune deletes lists.
    #[test]
    fn a_prune_in_another_process_never_deletes_a_writers_blobs() {
        for pause in ["blobs_written", "manifest_write"] {
            let root = scratch(&format!("cas-process-prune-{pause}"));
            let graph = root.join("graph");
            let base = root.join("backups");
            small_graph(&graph);
            snapshot_now(&base, &graph, "");
            std::fs::write(graph.join("pages/B.md"), "- b edited\n").unwrap();
            snapshot_now(&base, &graph, "");
            std::fs::write(graph.join("pages/B.md"), "- b\n").unwrap();
            let mut child = ChildBackup::spawn(&graph, &base, &format!(":{pause}"));
            child.line("paused");
            assert!(
                lock_namespace(&cas_dir(&base), None).unwrap().is_none(),
                "the writer holds the namespace lock"
            );
            let (pruned_tx, pruned_rx) = std::sync::mpsc::channel();
            let prune_base = base.clone();
            let prune = std::thread::spawn(move || {
                prune_now(&prune_base, 1);
                let _ = pruned_tx.send(());
            });
            let pruned_early = pruned_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .is_ok();
            child.resume();
            let done = child.finish();
            prune.join().unwrap();
            assert!(!pruned_early, "{pause}: the prune waits for the writer");
            assert_eq!(done, "done None", "{pause}");
            let left = cas_snapshots(&base);
            let writer = left.last().unwrap();
            assert!(
                read_manifest(writer).unwrap().anchor,
                "{pause}: the first launch anchors"
            );
            assert_eq!(left.len(), 2, "{pause}: keep 1 plus the writer's anchor");
            for snapshot in &left {
                assert!(
                    verify_snapshot(snapshot, &read_manifest(snapshot).unwrap()),
                    "{pause}: every blob of the writer's snapshot survived the prune"
                );
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// REVIEW B2, two OS processes: a launch backup that finds the namespace
    /// lock held skips (not a refusal) and never cleans up the live writer's
    /// partial snapshot.
    #[test]
    fn a_launch_in_another_process_skips_while_a_writer_holds_the_lock() {
        let root = scratch("cas-process-skip");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let mut child = ChildBackup::spawn(&graph, &base, ":blobs_written");
        child.line("paused");
        let partial = std::fs::read_dir(cas_dir(&base).join(CAS_SNAPSHOTS))
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".partial-")
            })
            .expect("the writer's partial");
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = backup_locked(&base, &store, source, "", 12, &|| false);
        store.close();
        let skipped = (outcome.failure.is_none(), outcome.copied, outcome.published);
        let partial_kept = partial.is_dir();
        child.resume();
        let done = child.finish();
        assert_eq!(skipped, (true, 0, None), "the launch skipped");
        assert!(partial_kept, "the live partial is untouched");
        assert_eq!(done, "done None");
        let left = cas_snapshots(&base);
        assert_eq!(left.len(), 1);
        assert!(verify_snapshot(&left[0], &read_manifest(&left[0]).unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// REVIEW B2, two OS processes: two snapshots of the same new content
    /// never share a temp or race a blob's rename. The second waits for the
    /// namespace lock; both verify and no temp remains.
    #[test]
    fn a_same_content_blob_write_in_two_processes_is_serialised() {
        let root = scratch("cas-process-same-blob");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let mut child = ChildBackup::spawn(&graph, &base, "pre-restore:blob_temp");
        child.line("paused");
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let (parent_graph, parent_base) = (graph.clone(), base.clone());
        let parent = std::thread::spawn(move || {
            let (store, _, _) =
                Store::open(&parent_graph, tine_store::OpenOptions::default()).unwrap();
            let source = BackupSource::from_store(&store, &parent_graph).unwrap();
            let outcome = backup_locked(&parent_base, &store, source, "pre-rewrite", 12, &|| false);
            store.close();
            let _ = done_tx.send(());
            outcome
        });
        let parent_early = done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok();
        child.resume();
        let done = child.finish();
        let outcome = parent.join().unwrap();
        assert!(!parent_early, "the second writer waits for the lock");
        assert_eq!(done, "done None");
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let left = cas_snapshots(&base);
        assert_eq!(left.len(), 2);
        for snapshot in &left {
            assert!(verify_snapshot(snapshot, &read_manifest(snapshot).unwrap()));
        }
        let temps = std::fs::read_dir(cas_dir(&base).join(BLOB_DIR))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp-"))
            .count();
        assert_eq!(temps, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Launch-backup cost on a real graph (dossier og-backup-cas), through
    /// the launch path (`backup_locked`): files created and bytes written in
    /// the namespace, syncs (seam observations), wall time, for a first
    /// backup (the anchor), an unchanged second one, one after three page
    /// edits, an anchor rotation (clock 8 days on), and a launch whose
    /// keep-count deletes a snapshot (collection). Created files count
    /// surviving new paths, not every write (temps, repairs). Runs only on a
    /// scratch COPY of a graph:
    /// `TINE_BACKUP_CORPUS=<copy> cargo test -p tine launch_backup_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement; needs TINE_BACKUP_CORPUS (a scratch copy of a graph)"]
    fn launch_backup_cost_on_corpus() {
        let graph =
            PathBuf::from(std::env::var_os("TINE_BACKUP_CORPUS").expect("TINE_BACKUP_CORPUS"));
        let base = graph
            .with_extension("backup-cost-app")
            .join("backups")
            .join("graph-id");
        let app_data = base.parent().unwrap().parent().unwrap().to_path_buf();
        let cas = cas_dir(&base);
        let _ = std::fs::remove_dir_all(&app_data);
        let files_under = |dir: &std::path::Path| {
            let mut out = std::collections::BTreeMap::new();
            let mut stack = vec![dir.to_path_buf()];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let meta = entry.metadata().unwrap();
                    if meta.is_dir() {
                        stack.push(entry.path());
                    } else {
                        out.insert(entry.path(), meta.len());
                    }
                }
            }
            out
        };
        let run = |label: &str, keep: usize| {
            let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
            let source = BackupSource::from_store(&store, &graph).unwrap();
            let before = files_under(&cas);
            BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
            take_syncs();
            let started = std::time::Instant::now();
            let outcome = backup_locked(&base, &store, source, "", keep, &|| false);
            let elapsed = started.elapsed();
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
            let after = files_under(&cas);
            let created: Vec<_> = after
                .keys()
                .filter(|path| !before.contains_key(*path))
                .collect();
            let bytes: u64 = created.iter().map(|path| after[*path]).sum();
            let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
            let syncs = take_syncs().len();
            let blob_writes = ops.iter().filter(|op| **op == "blob_write").count();
            let scans = ops.iter().filter(|op| **op == "blob_dir_scan").count();
            eprintln!(
                "BACKUP-COST {label}: graph_files={} created_files={} bytes_written={bytes} syncs={syncs} blob_writes={blob_writes} blob_scans={scans} wall_ms={:.1}",
                outcome.copied,
                created.len(),
                elapsed.as_secs_f64() * 1000.0
            );
            store.close();
            (created.len(), blob_writes, syncs, scans)
        };
        let (_, _, first_syncs, _) = run("first-anchor", BACKUP_KEEP_DEFAULT);
        assert!(first_syncs > 0, "the first backup is a durable anchor");
        assert_eq!(
            run("unchanged", BACKUP_KEEP_DEFAULT),
            (1, 0, 0, 0),
            "one manifest, no blob, no sync"
        );
        let mut pages: Vec<PathBuf> = std::fs::read_dir(graph.join("pages"))
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        pages.sort();
        for page in &pages[..3] {
            let mut text = std::fs::read(page).unwrap();
            text.extend_from_slice(b"\n- backup cost probe edit\n");
            std::fs::write(page, text).unwrap();
        }
        assert_eq!(
            run("three-edits", BACKUP_KEEP_DEFAULT),
            (4, 3, 0, 0),
            "three blobs and a manifest"
        );
        CLOCK_SHIFT.with(|clock| clock.set(8 * 86_400));
        let (_, _, rotation_syncs, _) = run("anchor-rotation", BACKUP_KEEP_DEFAULT);
        assert!(
            rotation_syncs > 8,
            "a rotation pays the full durable chain once"
        );
        // Same clock, so the fresh anchor does not rotate again.
        let (_, _, prune_syncs, scans) = run("prune-keep-1", 1);
        CLOCK_SHIFT.with(|clock| clock.set(0));
        assert_eq!(
            (prune_syncs, scans),
            (0, 1),
            "a deletion runs the collector"
        );
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn launch_backup_copy_failure_reaches_diagnostic_adapter() {
        let root = scratch("launch-backup-copy-error");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/note.md"), b"- keep\n").unwrap();
        let blobs = root.join("blocked-destination");
        std::fs::write(&blobs, b"already a file").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let (copied, failed, failure, _) =
            copy_area(&store, Area::Pages, &blobs, is_graph_text, &|| false);
        assert_eq!((copied, failed), (0, 1));
        let failure = failure.unwrap();
        let token = launch_failure_token(&BackupOutcome {
            copied,
            failure: Some(failure.clone()),
            published: None,
        })
        .unwrap();
        assert_eq!(token, format!("backup-failed:pages:{:?}", failure.kind),
            "I-9: forced copy failure must reach the launch diagnostic adapter; exemplar backup.rs backup_async");
        store.close();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_root_ids_do_not_conflate_punctuation() {
        let root = scratch("backup-root-id");
        let dash = root.join("a-b");
        let underscore = root.join("a_b");
        std::fs::create_dir_all(&dash).unwrap();
        std::fs::create_dir_all(&underscore).unwrap();
        assert_ne!(root_backup_id(&dash), root_backup_id(&underscore));
        assert_eq!(root_backup_id(&dash), root_backup_id(&dash));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_backup_reads_graph_files_through_store() {
        let root = scratch("store-backup-read");
        std::fs::create_dir_all(root.join("pages/nested")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(root.join("pages/nested/Note.md"), b"- note\n").unwrap();
        std::fs::write(root.join("pages/Ignore.txt"), b"skip").unwrap();
        let (store, _, _) = Store::open(&root, tine_store::OpenOptions::default()).unwrap();
        let blobs = root.join("backup-out");
        std::fs::create_dir_all(&blobs).unwrap();
        let (copied, failed, failure, files) =
            copy_area(&store, Area::Pages, &blobs, is_graph_text, &|| false);
        assert_eq!((copied, failed), (1, 0));
        assert!(failure.is_none());
        assert_eq!(
            blob_bytes(&blobs, &files, "area/nested/Note.md"),
            b"- note\n"
        );
        assert_eq!(files.len(), 1, "Ignore.txt is not graph text");
        store.close();
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn backup_source_refuses_retargeted_external_assets() {
        let root = scratch("retargeted-backup-assets");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        let first = root.with_extension("assets-first");
        let second = root.with_extension("assets-second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::os::unix::fs::symlink(&first, root.join("assets")).unwrap();
        let (store, _, _) = Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: Some(first.clone()),
                watch: Default::default(),
                launch_checkpoint: None,
            },
        )
        .unwrap();
        assert!(BackupSource::from_store(&store, &root).is_ok());
        std::fs::remove_file(root.join("assets")).unwrap();
        std::os::unix::fs::symlink(&second, root.join("assets")).unwrap();
        assert!(BackupSource::from_store(&store, &root).is_err());
        store.close();
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    /// REG-OG-C5-L06-B1 (I-9): a graph inventory that cannot be read names
    /// its cause in the backup token. Before, every inventory failure became
    /// `backup-failed:inventory:Other`, hiding a permission or disk error.
    #[cfg(unix)]
    #[test]
    fn inventory_failure_keeps_its_error_kind() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("inventory-error-kind");
        let graph = root.join("graph");
        for dir in ["pages/locked", "journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        std::fs::write(graph.join("pages/locked/a.md"), b"- a\n").unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let locked = graph.join("pages/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let outcome = write_snapshot(&root.join("backups"), &store, source, "", false, &|| false);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        let failure = outcome
            .failure
            .expect("an unreadable page directory fails the backup");
        assert_eq!(
            (failure.phase, failure.kind),
            ("inventory", ErrorKind::PermissionDenied),
            "I-9: the inventory failure's cause must reach the backup token; exemplar backup.rs count_store_text"
        );

        // A store closed under the backup reports that, not `Other`.
        let source = BackupSource::from_store(&store, &graph).unwrap();
        store.close();
        let outcome = write_snapshot(&root.join("backups"), &store, source, "", false, &|| false);
        let failure = outcome.failure.expect("a closed store fails the backup");
        assert_eq!(
            (failure.phase, failure.kind),
            ("inventory", ErrorKind::BrokenPipe)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_and_abandoned_partial_backups_are_removed() {
        let root = scratch("partial-backup-cleanup");
        let failed = root.join(".partial-failed");
        std::fs::create_dir_all(&failed).unwrap();
        std::fs::write(failed.join("half.md"), b"partial").unwrap();
        {
            let _guard = PartialBackup {
                path: failed.clone(),
                committed: false,
            };
        }
        assert!(!failed.exists());

        assert!(!has_partials(&root));
        let crashed = root.join(".partial-crashed");
        std::fs::create_dir_all(&crashed).unwrap();
        assert!(has_partials(&root), "a crashed partial triggers collection");
        assert!(!has_partials(&root.join("missing")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancellable_copy_stops_before_traversing_the_tree() {
        let root = scratch("backup-cancel");
        let graph = root.join("graph");
        let src = graph.join("pages");
        let blobs = root.join("blobs");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&blobs).unwrap();
        for dir in ["journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        std::fs::write(src.join("note.md"), b"secret").unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let (copied, failed, failure, files) =
            copy_area(&store, Area::Pages, &blobs, is_graph_text, &|| true);
        assert_eq!((copied, failed), (0, 1));
        assert_eq!(failure.unwrap().kind, ErrorKind::Interrupted);
        assert!(files.is_empty());
        assert_eq!(std::fs::read_dir(&blobs).unwrap().count(), 0);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_complete_v2_manifests_are_readable() {
        let root = scratch("backup-manifest");
        let manifest = SnapshotManifest {
            schema: LEGACY_SNAPSHOT_SCHEMA,
            root: root.display().to_string(),
            journals_dir: "diary".into(),
            pages_dir: "archive/pages".into(),
            graph_text_policy: None,
            writer: None,
            files: Vec::new(),
            complete: true,
            anchor: false,
            created_unix: None,
        };
        write_manifest(&root, &manifest).unwrap();
        let read = read_manifest(&root).unwrap();
        assert_eq!(read.pages_dir, "archive/pages");
        assert!(verify_snapshot(&root, &read));
        std::fs::write(root.join("journals.md"), "- changed\n").unwrap();
        assert!(!verify_snapshot(&root, &read));
        std::fs::remove_file(root.join("journals.md")).unwrap();
        std::fs::write(
            root.join(SNAPSHOT_MANIFEST),
            r#"{"schema":2,"root":"x","journals_dir":"journals","pages_dir":"pages","files":[],"complete":false}"#,
        )
        .unwrap();
        assert!(read_manifest(&root).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_listing_never_hashes_snapshot_payloads() {
        let root = scratch("manifest-only-listing");
        let graph = root.join("graph");
        let base = root.join("backups");
        let snapshot = base.join("2026-07-22_12-00-00");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        std::fs::create_dir_all(snapshot.join("pages")).unwrap();
        std::fs::write(snapshot.join("pages/note.md"), b"tampered payload").unwrap();
        write_manifest(
            &snapshot,
            &SnapshotManifest {
                schema: LEGACY_SNAPSHOT_SCHEMA,
                root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                graph_text_policy: None,
                writer: None,
                files: vec![SnapshotFile {
                    path: "pages/note.md".into(),
                    sha256: "manifest metadata only".into(),
                }],
                complete: true,
                anchor: false,
                created_unix: None,
            },
        )
        .unwrap();

        PAYLOAD_HASH_READS.with(|reads| reads.set(0));
        let listed = list_backups_from_base(&base, &graph);

        assert_eq!(
            PAYLOAD_HASH_READS.with(|reads| reads.get()),
            0,
            "listing must not read or hash snapshot payloads"
        );
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].files, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn copy_asset_sidecars_dir_copies_only_edn_recursively() {
        let root = scratch("copy-sidecars");
        let graph = root.join("graph");
        let src = graph.join("assets");
        let blobs = root.join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::create_dir_all(src.join("nested")).unwrap();
        for dir in ["pages", "journals", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        std::fs::write(src.join("doc.edn"), "{:a 1}\n").unwrap();
        std::fs::write(src.join("nested").join("hl.edn"), "{:b 2}\n").unwrap();
        std::fs::write(src.join("image.png"), b"png").unwrap();
        std::fs::write(src.join("nested").join("image.png"), b"png").unwrap();
        std::fs::create_dir_all(src.join(ASSET_RESTORE_RECOVERY_DIR)).unwrap();
        std::fs::write(
            src.join(ASSET_RESTORE_RECOVERY_DIR).join("old.edn"),
            "{:old true}\n",
        )
        .unwrap();

        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let (copied, failed, failure, files) =
            copy_area(&store, Area::Assets, &blobs, is_asset_sidecar, &|| false);
        assert_eq!((copied, failed), (2, 0));
        assert!(failure.is_none());
        assert_eq!(blob_bytes(&blobs, &files, "area/doc.edn"), b"{:a 1}\n");
        assert_eq!(
            blob_bytes(&blobs, &files, "area/nested/hl.edn"),
            b"{:b 2}\n"
        );
        let mut paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            ["area/doc.edn", "area/nested/hl.edn"],
            "no images and no restore recovery"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn graph_text_backup_includes_nested_pages() {
        let root = scratch("nested-md-backup");
        let graph = root.join("graph");
        let pages = graph.join("pages");
        let journals = graph.join("journals");
        let blobs = root.join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::create_dir_all(pages.join("client-a")).unwrap();
        for dir in ["assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        std::fs::create_dir_all(&journals).unwrap();
        std::fs::write(pages.join("Top.md"), b"top\n").unwrap();
        std::fs::write(pages.join("client-a/Deep.md"), b"deep\n").unwrap();
        std::fs::write(journals.join("2026_07_09.md"), b"journal\n").unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let live_pages = count_store_text(&store, Area::Pages).unwrap();
        let live_journals = count_store_text(&store, Area::Journals).unwrap();
        let (copied_pages, failed_pages, _, pages_files) =
            copy_area(&store, Area::Pages, &blobs, is_graph_text, &|| false);
        let (copied_journals, failed_journals, _, journal_files) =
            copy_area(&store, Area::Journals, &blobs, is_graph_text, &|| false);
        let copied = copied_pages + copied_journals;
        let failed = failed_pages + failed_journals;
        let complete = failed == 0 && copied == live_pages + live_journals;
        assert_eq!(live_pages, 2);
        assert_eq!(live_journals, 1);
        assert!(complete);
        assert_eq!(blob_bytes(&blobs, &pages_files, "area/Top.md"), b"top\n");
        assert_eq!(
            blob_bytes(&blobs, &pages_files, "area/client-a/Deep.md"),
            b"deep\n"
        );
        assert_eq!(
            blob_bytes(&blobs, &journal_files, "area/2026_07_09.md"),
            b"journal\n"
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod fail_read_tests {
    use super::*;
    #[test]
    fn fail_read_backup_refusal_keeps_phase_and_io_kind() {
        let error = rewrite_snapshot_result(BackupOutcome::failed(
            0,
            "pages",
            ErrorKind::PermissionDenied,
        ))
        .unwrap_err();
        assert_eq!(error, "backup-failed:pages:PermissionDenied");
    }
}

#[cfg(test)]
mod launch_schedule_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn launch_backup_does_not_copy_while_startup_is_still_running() {
        let root = tempfile::tempdir().unwrap();
        let slot = Arc::new(GraphSlot::new(
            Store::open(root.path(), Default::default()).unwrap().0,
            root.path().to_path_buf(),
        ));
        let (sent, received) = std::sync::mpsc::channel();
        let worker_slot = slot.clone();
        let worker = std::thread::spawn(move || {
            sent.send(wait_launch_backup(&worker_slot)).unwrap();
        });
        let early = received.recv_timeout(Duration::from_millis(1200));
        // End the worker after observing the result, even on the old schedule.
        slot.cancel_background();
        worker.join().unwrap();
        assert!(early.is_err(), "I-20: launch backup must wait for the owning warm completion signal; exemplar src-tauri/src/backup.rs");
    }
}

#[cfg(test)]
mod idle_signal_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn warm_completion_quiet_deadline_and_revocation_own_the_backup_wait() {
        let root = tempfile::tempdir().unwrap();
        let slot = Arc::new(GraphSlot::new(
            Store::open(root.path(), Default::default()).unwrap().0,
            root.path().to_path_buf(),
        ));
        let old = slot.begin_startup_warm();
        let current = slot.begin_startup_warm();
        slot.finish_startup_warm(old);
        assert!(
            !slot.warm_done.load(Ordering::Acquire),
            "an old warm cannot release the current backup"
        );
        let (sent, received) = std::sync::mpsc::channel();
        let waiting = slot.clone();
        let worker = std::thread::spawn(move || {
            sent.send(waiting.wait_startup_idle(Duration::from_millis(80), Duration::from_secs(5)))
                .unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(20)).is_err());
        slot.finish_startup_warm(current);
        assert!(
            received.recv_timeout(Duration::from_millis(20)).is_err(),
            "completion must retain a quiet turn"
        );
        assert!(received.recv_timeout(Duration::from_secs(2)).unwrap());
        worker.join().unwrap();
        // A missed warm signal still preserves the safety net at the deadline.
        slot.begin_startup_warm();
        assert!(slot.wait_startup_idle(Duration::from_secs(5), Duration::from_millis(1)));
        let waiting = slot.clone();
        let worker = std::thread::spawn(move || {
            waiting.wait_startup_idle(Duration::from_secs(5), Duration::from_secs(180))
        });
        slot.cancel_background();
        assert!(!worker.join().unwrap());
    }
}
