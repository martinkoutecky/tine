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

mod restore;
pub(crate) use restore::restore_backup;

// Snapshot the graph's Markdown/Org into the OS app-data dir on open, keeping the
// last few. Local-only (outside the graph, so Syncthing never sees it); a safety
// net against a bad write or accidental edit. Source validation runs at launch;
// the file copy runs in a detached best-effort worker.
const BACKUP_KEEP_DEFAULT: usize = 12;
static BACKUP_WORK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

/// The process-wide backup permit. Every snapshot write and every prune (with
/// its blob collection) runs under it, so a prune never sees a snapshot whose
/// blobs are written but whose manifest is not yet published.
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

#[derive(Debug)]
pub(crate) struct BackupOutcome {
    pub(crate) copied: usize,
    pub(crate) failure: Option<BackupFailure>,
}

impl BackupOutcome {
    pub(crate) fn success(copied: usize) -> Self {
        Self {
            copied,
            failure: None,
        }
    }

    pub(crate) fn failed(copied: usize, phase: &'static str, kind: ErrorKind) -> Self {
        Self {
            copied,
            failure: Some(BackupFailure { phase, kind }),
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
/// manifest: each listed file's bytes live once in the backup base's blob
/// store, `blobs/<sha256>`. Schema 3 (og-B, ADR 0062) keeps a full copy of
/// graph text under `graph/<graph-relative path>` and records the graph-text
/// scope it covered; schema 2 kept only the configured `journals/` and
/// `pages/` roots. Both full-copy schemas are master's wire formats and still
/// list, restore and prune.
const SNAPSHOT_SCHEMA: u32 = 4;
const GRAPH_COPY_SNAPSHOT_SCHEMA: u32 = 3;
const LEGACY_SNAPSHOT_SCHEMA: u32 = 2;
/// The blob store inside a backup base; never a snapshot.
const BLOB_DIR: &str = "blobs";
/// Master's `GRAPH_TEXT_SCOPE_VERSION`: the discovery exclusions this build's
/// `graph_text_eligible` applies (`published-queries/` included).
const GRAPH_TEXT_SCOPE_VERSION: u32 = 2;
/// Marks this build's schema-3/4 snapshots; master ignores the field. Prune
/// counts only snapshots this build wrote (docs/app-identity.md).
const SNAPSHOT_WRITER: &str = "og";
const SNAPSHOT_MANIFEST: &str = "snapshot.json";

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

/// Write the manifest into an unpublished snapshot directory; the
/// directory's rename publishes it. `durable` makes it reach disk first.
fn write_manifest(
    dir: &std::path::Path,
    manifest: &SnapshotManifest,
    durable: bool,
) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(manifest).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(SNAPSHOT_MANIFEST))?;
    file.write_all(&bytes)?;
    record_backup_op("manifest_write");
    if durable {
        file.sync_all()?;
        record_backup_op("manifest_sync");
        tine_store::directory_durability::sync_directory_entry(dir)?;
        record_backup_op("manifest_dir_sync");
    }
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    static BACKUP_OPS: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record_backup_op(op: &'static str) {
    #[cfg(test)]
    BACKUP_OPS.with(|ops| ops.borrow_mut().push(op));
    #[cfg(not(test))]
    let _ = op;
}

/// Store `bytes`, whose SHA-256 is `sha256`, in the blob store unless an
/// identical blob is already there. Called with `BACKUP_WORK` held, so no
/// prune runs while a snapshot's blobs are still unreferenced.
///
/// Blobs are written without a sync (verification, not fsync, keeps a
/// snapshot honest), so power loss can leave a blob name holding torn bytes.
/// Every later snapshot of that unchanged content would reuse it, so a blob
/// is reused only when its bytes equal the content, and a damaged one is
/// replaced. The temp-and-rename keeps a crash from leaving a blob name with
/// a partial write.
fn put_blob(blobs: &std::path::Path, sha256: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = blobs.join(sha256);
    match std::fs::read(&path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => record_backup_op("blob_repair"),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let tmp = blobs.join(format!(".tmp-{sha256}"));
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    record_backup_op("blob_write");
    written
}

/// Publish an unpublished snapshot directory under its final name. A
/// `durable` snapshot (one taken before a rewrite or restore the user asked
/// for, which the next launch cannot re-take) syncs every blob it lists and
/// its manifest before the rename and the base directory after it. A launch
/// snapshot syncs nothing: restore verifies every blob before any graph
/// write, a torn launch snapshot is refused, and the next launch repairs a
/// torn blob of content the graph still holds.
fn publish_snapshot(
    partial: &std::path::Path,
    final_dest: &std::path::Path,
    manifest: &SnapshotManifest,
    durable: bool,
) -> std::io::Result<()> {
    let base = final_dest.parent().expect("snapshot has parent");
    if durable {
        let blobs = base.join(BLOB_DIR);
        for file in &manifest.files {
            // Windows flushes a file only through a handle that may write.
            std::fs::OpenOptions::new()
                .write(true)
                .open(blobs.join(&file.sha256))?
                .sync_all()?;
            record_backup_op("blob_sync");
        }
        tine_store::directory_durability::sync_directory_entry(&blobs)?;
        record_backup_op("blob_dir_sync");
    }
    write_manifest(partial, manifest, durable)?;
    crate::device_io::move_file_noreplace(partial, final_dest)?;
    record_backup_op("publish_rename");
    if durable {
        tine_store::directory_durability::sync_directory_entry(base)?;
        record_backup_op("publication_dir_sync");
    }
    Ok(())
}

fn read_manifest(dir: &std::path::Path) -> Option<SnapshotManifest> {
    let bytes = std::fs::read(dir.join(SNAPSHOT_MANIFEST)).ok()?;
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes).ok()?;
    let supported = manifest.schema == LEGACY_SNAPSHOT_SCHEMA
        || (matches!(
            manifest.schema,
            GRAPH_COPY_SNAPSHOT_SCHEMA | SNAPSHOT_SCHEMA
        ) && manifest
            .graph_text_policy
            .as_ref()
            .is_some_and(|policy| policy.version == GRAPH_TEXT_SCOPE_VERSION));
    (supported && manifest.complete).then_some(manifest)
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

/// Where a listed file's bytes live: inside a full-copy snapshot, or in the
/// backup base's blob store for schema 4. `None` when a schema-4 entry's hash
/// is not a SHA-256 hex digest, so no manifest names a path outside the
/// store.
fn payload_path(
    snapshot: &std::path::Path,
    manifest: &SnapshotManifest,
    file: &SnapshotFile,
) -> Option<PathBuf> {
    if manifest.schema != SNAPSHOT_SCHEMA {
        return Some(snapshot.join(&file.path));
    }
    let digest = file.sha256.len() == 64
        && file
            .sha256
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    digest.then(|| {
        snapshot
            .parent()
            .map(|base| base.join(BLOB_DIR).join(&file.sha256))
    })?
}

/// A full-copy snapshot holds exactly its manifest's files; a schema-4
/// snapshot's every listed blob exists and hashes to its name.
fn verify_snapshot(dir: &std::path::Path, manifest: &SnapshotManifest) -> bool {
    if manifest.schema != SNAPSHOT_SCHEMA {
        return snapshot_inventory(dir)
            .map(|files| files == manifest.files)
            .unwrap_or(false);
    }
    manifest.files.iter().all(|file| {
        payload_path(dir, manifest, file)
            .is_some_and(|blob| hash_snapshot_file(&blob).is_ok_and(|hash| hash == file.sha256))
    })
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
        match store.read(&entry.id, None) {
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

fn cleanup_partial_backups(base: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(".partial-") {
            let path = entry.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
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
    let outcome = write_snapshot(&base, store, source, suffix, cancelled);
    if outcome.failure.is_none() && outcome.copied > 0 {
        prune_backups(&base, backup_keep(app));
    }
    outcome
}

/// Put one snapshot's files in `base`'s blob store and publish its manifest;
/// the caller prunes. A suffixed snapshot precedes a rewrite or restore the
/// user asked for, so it is published durably (`publish_snapshot`).
fn write_snapshot(
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    suffix: &str,
    cancelled: &dyn Fn() -> bool,
) -> BackupOutcome {
    let stamp = tine_core::date::utc_backup_stamp();
    let name = if suffix.is_empty() {
        stamp
    } else {
        format!("{stamp}-{suffix}")
    };
    // Reserve a UNIQUE name. The stamp is second-granularity, so two snapshots
    // in the same second (e.g. a launch snapshot and a pre-restore snapshot)
    // would otherwise collide. `create_dir` (non-recursive) fails atomically
    // if the partial name is taken, and a published snapshot of that name
    // bumps the counter too (the caller holds `BACKUP_WORK`, so nothing
    // publishes in between).
    if let Err(error) = std::fs::create_dir_all(base) {
        return BackupOutcome::failed(0, "reserve", error.kind());
    }
    cleanup_partial_backups(base);
    let mut final_dest = base.join(&name);
    let mut dest = base.join(format!(".partial-{name}"));
    let mut k = 2;
    loop {
        let reserved = if std::fs::symlink_metadata(&final_dest).is_ok() {
            Err(ErrorKind::AlreadyExists.into())
        } else {
            std::fs::create_dir(&dest)
        };
        match reserved {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                final_dest = base.join(format!("{name}-{k}"));
                dest = base.join(format!(".partial-{name}-{k}"));
                k += 1;
            }
            Err(error) => return BackupOutcome::failed(0, "reserve", error.kind()),
        }
    }
    let mut partial = PartialBackup {
        path: dest.clone(),
        committed: false,
    };
    let blobs = base.join(BLOB_DIR);
    if let Err(error) = std::fs::create_dir_all(&blobs) {
        return BackupOutcome::failed(0, "reserve", error.kind());
    }
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
                    match store.read(&config.id, None) {
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
        };
    }
    if ct != live_text_n {
        return BackupOutcome::failed(n, "inventory", ErrorKind::InvalidData);
    }
    if n == 0 {
        return BackupOutcome::success(0);
    }
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
    };
    if let Err(error) = publish_snapshot(&dest, &final_dest, &manifest, !suffix.is_empty()) {
        return BackupOutcome::failed(n, "publish", error.kind());
    }
    partial.committed = true;
    BackupOutcome::success(n)
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

fn list_backups_from_base(base: &std::path::Path, root: &std::path::Path) -> Vec<BackupInfo> {
    let current_root = Store::canonical_root(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let Some(manifest) = read_manifest(&p) else {
                continue;
            };
            if manifest.root != current_root {
                continue;
            }
            let stamp = match p.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let files = manifest.files.len();
            out.push(BackupInfo { stamp, files });
        }
    }
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp)); // newest first
    out
}

/// A snapshot the keep-count must leave alone: another Tine wrote it.
fn is_foreign_snapshot(dir: &std::path::Path) -> bool {
    let Some(manifest) = std::fs::read(dir.join(SNAPSHOT_MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return false;
    };
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

/// Prune outside a snapshot write (a lowered keep-count), under the permit.
fn prune_now(base: &std::path::Path, keep: usize) {
    let _worker = backup_work();
    prune_backups(base, keep);
}

/// Apply the keep-count, then collect unreferenced blobs. The caller holds
/// `BACKUP_WORK`.
fn prune_backups(base: &std::path::Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(base) else {
        return;
    };
    // Only the routine launch snapshots are subject to the keep-count. Tagged
    // snapshots (e.g. "...-pre-restore") are deliberate safety points and are
    // never auto-pruned.
    let mut dirs: Vec<std::path::PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name() != Some(std::ffi::OsStr::new(BLOB_DIR))
                && !p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.starts_with(".partial-"))
                    .unwrap_or(true)
                && !p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.contains("-pre-restore"))
                    .unwrap_or(false)
        })
        .collect();
    // A snapshot another Tine sharing this app-data dir wrote (master's schema
    // 3, which carries no og writer mark; docs/app-identity.md) is listed and
    // restorable here but is not ours to count or delete.
    dirs.retain(|dir| !is_foreign_snapshot(dir));
    dirs.sort(); // timestamp-named → chronological
    if dirs.len() > keep {
        for d in &dirs[..dirs.len() - keep] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    collect_blobs(base);
}

/// Delete every blob that no snapshot directory's manifest lists. Every
/// manifest counts, whatever its schema, writer or name (`.partial-*` and
/// pre-restore ones included), read loosely as `files[].sha256`. Any doubt
/// keeps the blobs: an unreadable backup base, entry or manifest (other than
/// a missing one) stops the collection. A manifest that is not valid JSON
/// names nothing: that snapshot is torn and never restores. A crash between
/// a manifest's deletion and this collection only leaves unreferenced blobs,
/// which the next prune collects.
fn collect_blobs(base: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let mut live = std::collections::HashSet::new();
    for entry in entries {
        let Ok((kind, entry)) = entry.and_then(|entry| Ok((entry.file_type()?, entry))) else {
            return;
        };
        if entry.file_name() == BLOB_DIR || !kind.is_dir() {
            continue;
        }
        let bytes = match std::fs::read(entry.path().join(SNAPSHOT_MANIFEST)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => return,
        };
        let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let files = manifest.get("files").and_then(serde_json::Value::as_array);
        for file in files.into_iter().flatten() {
            if let Some(sha256) = file.get("sha256").and_then(serde_json::Value::as_str) {
                live.insert(sha256.to_owned());
            }
        }
    }
    let Ok(blobs) = std::fs::read_dir(base.join(BLOB_DIR)) else {
        return;
    };
    for blob in blobs.flatten() {
        if !blob
            .file_name()
            .to_str()
            .is_some_and(|name| live.contains(name))
        {
            let _ = std::fs::remove_file(blob.path());
            record_backup_op("blob_collect");
        }
    }
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
        let before: std::collections::BTreeSet<_> = std::fs::read_dir(base)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
        let outcome = write_snapshot(base, &store, source, suffix, &|| false);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
        store.close();
        let published = std::fs::read_dir(base)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| !before.contains(path) && path.file_name().unwrap() != BLOB_DIR)
            .unwrap();
        (ops, published)
    }

    /// Restated for the content-addressed design (relaxation ledger: the old
    /// test asserted per-file payload and directory fsyncs before every
    /// publication). A launch snapshot publishes its manifest only after
    /// every blob write and syncs nothing; restore verifies every listed
    /// blob. A snapshot taken before a user-requested rewrite or restore
    /// syncs every blob and its manifest before the rename, as before.
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
        assert!(
            !ops.iter().any(|op| op.ends_with("sync")),
            "a launch snapshot syncs nothing: {ops:?}"
        );
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

        let (ops, _) = snapshot_now(&base, &graph, "pre-restore");
        assert_eq!(ops.iter().filter(|op| **op == "blob_sync").count(), 5);
        assert!(last(&ops, "blob_sync") < position(&ops, "blob_dir_sync"));
        assert!(position(&ops, "blob_dir_sync") < position(&ops, "manifest_sync"));
        assert!(position(&ops, "manifest_sync") < position(&ops, "manifest_dir_sync"));
        assert!(position(&ops, "manifest_dir_sync") < position(&ops, "publish_rename"));
        assert!(
            position(&ops, "publish_rename") < position(&ops, "publication_dir_sync"),
            "I-1/I-2: a pre-rewrite snapshot is durable before the rewrite runs; exemplar backup.rs publish_snapshot"
        );
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
        std::fs::write(base.join(BLOB_DIR).join(&page.sha256), b"").unwrap();
        std::fs::write(base.join(BLOB_DIR).join(&config.sha256), [0u8; 3]).unwrap();
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
        prune_backups(&base, 1);
        assert!(!first.exists() && second.exists());
        assert!(base.join(BLOB_DIR).join(&shared).is_file());
        assert!(!base.join(BLOB_DIR).join(&old_b).exists());
        assert!(verify_snapshot(&second, &read_manifest(&second).unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A prune (with its blob collection) that races a snapshot waits for the
    /// backup permit, so it never deletes blobs the snapshot wrote before
    /// publishing its manifest. Driven by the snapshot's cancellation hook,
    /// which runs between files. A `.partial-*` manifest also keeps its blobs.
    #[test]
    fn a_prune_racing_a_snapshot_never_deletes_its_blobs() {
        use std::sync::atomic::AtomicUsize;
        let root = scratch("backup-gc-race");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph(&graph);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let (entered, release) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        let calls = AtomicUsize::new(0);
        let (pruned_tx, pruned_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let snapshot = scope.spawn(|| {
                let _worker = backup_work();
                write_snapshot(&base, &store, source, "", &|| {
                    // Pause after the first blob is written, manifest unwritten.
                    if calls.fetch_add(1, Ordering::SeqCst) == 2 {
                        entered.wait();
                        release.wait();
                    }
                    false
                })
            });
            entered.wait();
            let written = std::fs::read_dir(base.join(BLOB_DIR)).unwrap().count();
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
            let blobs_now = std::fs::read_dir(base.join(BLOB_DIR)).unwrap().count();
            release.wait();
            assert!(snapshot.join().unwrap().failure.is_none());
            prune.join().unwrap();
            assert!(!pruned_early, "the prune waits for the snapshot");
            assert_eq!(
                blobs_now, written,
                "no blob of the unpublished snapshot is collected"
            );
        });
        let published = std::fs::read_dir(&base)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.file_name().unwrap() != BLOB_DIR)
            .unwrap();
        assert!(verify_snapshot(
            &published,
            &read_manifest(&published).unwrap()
        ));

        // An in-progress snapshot's manifest names references too.
        let partial = base.join(".partial-in-progress");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(base.join(BLOB_DIR).join("ab"), b"x").unwrap();
        std::fs::write(
            partial.join(SNAPSHOT_MANIFEST),
            r#"{"files":[{"path":"graph/x.md","sha256":"ab"}]}"#,
        )
        .unwrap();
        collect_blobs(&base);
        assert!(base.join(BLOB_DIR).join("ab").is_file());
        store.close();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Launch-backup cost on a real graph (dossier og-backup-cas): files
    /// created and bytes written under the backup base, sync calls, wall
    /// time, for a first backup, an unchanged second one, and one after
    /// three page edits. Runs only on a scratch COPY of a graph:
    /// `TINE_BACKUP_CORPUS=<copy> cargo test -p tine launch_backup_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement; needs TINE_BACKUP_CORPUS (a scratch copy of a graph)"]
    fn launch_backup_cost_on_corpus() {
        let graph =
            PathBuf::from(std::env::var_os("TINE_BACKUP_CORPUS").expect("TINE_BACKUP_CORPUS"));
        let base = graph.with_extension("backup-cost");
        let _ = std::fs::remove_dir_all(&base);
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
        let run = |label: &str| {
            let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
            let source = BackupSource::from_store(&store, &graph).unwrap();
            let before = files_under(&base);
            BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
            let started = std::time::Instant::now();
            let outcome = write_snapshot(&base, &store, source, "", &|| false);
            prune_backups(&base, BACKUP_KEEP_DEFAULT);
            let elapsed = started.elapsed();
            assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
            let after = files_under(&base);
            let created: Vec<_> = after
                .keys()
                .filter(|path| !before.contains_key(*path))
                .collect();
            let bytes: u64 = created.iter().map(|path| after[*path]).sum();
            let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
            let syncs = ops.iter().filter(|op| op.ends_with("sync")).count();
            eprintln!(
                "BACKUP-COST {label}: graph_files={} created_files={} bytes_written={bytes} syncs={syncs} wall_ms={:.1}",
                outcome.copied,
                created.len(),
                elapsed.as_secs_f64() * 1000.0
            );
            store.close();
        };
        run("first");
        run("unchanged");
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
        run("three-edits");
        let _ = std::fs::remove_dir_all(&base);
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
        let outcome = write_snapshot(&root.join("backups"), &store, source, "", &|| false);
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
        let outcome = write_snapshot(&root.join("backups"), &store, source, "", &|| false);
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

        let crashed = root.join(".partial-crashed");
        std::fs::create_dir_all(&crashed).unwrap();
        std::fs::write(crashed.join("half.md"), b"partial").unwrap();
        cleanup_partial_backups(&root);
        assert!(!crashed.exists());
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
        };
        write_manifest(&root, &manifest, false).unwrap();
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
            },
            false,
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
