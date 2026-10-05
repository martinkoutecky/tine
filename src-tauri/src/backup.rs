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
        let _worker = BACKUP_WORK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap();
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

/// Schema 3 (og-B, ADR 0062) keeps graph text under `graph/<graph-relative
/// path>` and records the graph-text scope it covered; schema 2 kept only the
/// configured `journals/` and `pages/` roots and still lists and restores.
/// Both are master's wire formats, so either build reads the other's.
const SNAPSHOT_SCHEMA: u32 = 3;
const LEGACY_SNAPSHOT_SCHEMA: u32 = 2;
/// Master's `GRAPH_TEXT_SCOPE_VERSION`: the discovery exclusions this build's
/// `graph_text_eligible` applies (`published-queries/` included).
const GRAPH_TEXT_SCOPE_VERSION: u32 = 2;
/// Marks this build's schema-3 snapshots; master ignores the field. Prune
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

fn write_manifest(dir: &std::path::Path, manifest: &SnapshotManifest) -> std::io::Result<()> {
    let path = dir.join(SNAPSHOT_MANIFEST);
    let tmp = dir.join(".snapshot.json.tmp");
    let bytes = serde_json::to_vec_pretty(manifest).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    use std::io::Write;
    file.write_all(&bytes)?;
    file.sync_all()?;
    record_backup_op("manifest_sync");
    drop(file);
    crate::device_io::move_file_noreplace(&tmp, &path)?;
    tine_store::directory_durability::sync_directory_entry(dir)
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

fn write_payload(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    record_backup_op("payload_sync");
    Ok(())
}

/// Sync every payload directory, children before parents. An explicit stack:
/// a snapshot mirrors the graph's depth, which costs heap, not stack (I-22).
fn sync_payload_dirs(dir: &std::path::Path) -> std::io::Result<()> {
    let mut pending = vec![(dir.to_path_buf(), false)];
    while let Some((dir, children_synced)) = pending.pop() {
        if children_synced {
            tine_store::directory_durability::sync_directory_entry(&dir)?;
            record_backup_op("payload_dir_sync");
            continue;
        }
        pending.push((dir.clone(), true));
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push((entry.path(), false));
            }
        }
    }
    Ok(())
}

fn publish_snapshot(
    partial: &std::path::Path,
    final_dest: &std::path::Path,
    manifest: &SnapshotManifest,
) -> std::io::Result<()> {
    sync_payload_dirs(partial)?;
    write_manifest(partial, manifest)?;
    crate::device_io::move_file_noreplace(partial, final_dest)?;
    record_backup_op("publish_rename");
    tine_store::directory_durability::sync_directory_entry(
        final_dest.parent().expect("snapshot has parent"),
    )?;
    record_backup_op("publication_dir_sync");
    Ok(())
}

fn read_manifest(dir: &std::path::Path) -> Option<SnapshotManifest> {
    let bytes = std::fs::read(dir.join(SNAPSHOT_MANIFEST)).ok()?;
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes).ok()?;
    let supported = manifest.schema == LEGACY_SNAPSHOT_SCHEMA
        || (manifest.schema == SNAPSHOT_SCHEMA
            && manifest
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

fn verify_snapshot(dir: &std::path::Path, manifest: &SnapshotManifest) -> bool {
    snapshot_inventory(dir)
        .map(|files| files == manifest.files)
        .unwrap_or(false)
}

fn do_backup_source(
    app: &tauri::AppHandle,
    store: &Store,
    source: BackupSource,
    suffix: &str,
) -> BackupOutcome {
    let _worker = BACKUP_WORK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap();
    do_backup_source_cancellable(app, store, source, suffix, &|| false)
}

fn copy_store_area(
    store: &Store,
    area: Area,
    dest: &std::path::Path,
    include: fn(&tine_store::FileId) -> bool,
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
    if let Err(error) = std::fs::create_dir_all(dest) {
        return (
            0,
            1,
            Some(BackupFailure {
                phase,
                kind: error.kind(),
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
        let target = dest.join(&entry.rel);
        match store.read(&entry.id, None) {
            Ok((bytes, _)) => {
                let result = target
                    .parent()
                    .ok_or_else(|| std::io::Error::from(ErrorKind::InvalidInput))
                    .and_then(std::fs::create_dir_all)
                    .and_then(|_| write_payload(&target, &bytes));
                match result {
                    Ok(()) => copied += 1,
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

/// Copy one snapshot into `base` and publish it; the caller prunes.
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
    // Reserve a UNIQUE destination directory. The stamp is second-granularity, so
    // two snapshots in the same second (e.g. a launch snapshot racing a pre-restore
    // snapshot) would otherwise share one directory — and copy_md_dir, which copies
    // in but never removes files absent from the live graph, would mix both
    // snapshots' files, leaving a later restore with stale notes/sidecars. `create_dir`
    // (non-recursive) fails atomically if the name is taken, so we bump a counter
    // until we win an unused name.
    if let Err(error) = std::fs::create_dir_all(&base) {
        return BackupOutcome::failed(0, "reserve", error.kind());
    }
    cleanup_partial_backups(&base);
    let mut final_dest = base.join(&name);
    let mut dest = base.join(format!(".partial-{name}"));
    let mut k = 2;
    loop {
        match std::fs::create_dir(&dest) {
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
    let live_text_n = match count_store_text(store, Area::Graph) {
        Ok(count) => count,
        Err(kind) => return BackupOutcome::failed(0, "inventory", kind),
    };
    // Graph text anywhere in the graph-text scope, at its graph-relative path.
    let (ct, ft, et) = copy_store_area(
        store,
        Area::Graph,
        &dest.join("graph"),
        is_graph_text,
        cancelled,
    );
    let (ca, fa, ea) = copy_store_area(
        store,
        Area::Assets,
        &dest.join(&source.assets_dir_name),
        is_asset_sidecar,
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
                            let result =
                                std::fs::create_dir_all(dest.join("logseq")).and_then(|_| {
                                    write_payload(&dest.join("logseq/config.edn"), &bytes)
                                });
                            match result {
                                Ok(()) => n += 1,
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
    let files = match snapshot_inventory(&dest) {
        Ok(files) => files,
        Err(error) => return BackupOutcome::failed(n, "inventory", error.kind()),
    };
    if files.len() != n {
        return BackupOutcome::failed(n, "inventory", ErrorKind::InvalidData);
    }
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
    if let Err(error) = publish_snapshot(&dest, &final_dest, &manifest) {
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
pub(crate) fn set_backup_keep(
    keep: usize,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let keep = keep.clamp(1, 1000);
    update_settings(&app, |json| {
        json["backup_keep"] = serde_json::json!(keep);
    })?;
    // Apply the new (possibly lower) cap to the current graph's snapshots now.
    let slot = slot_for_context(&state)?;
    if let Some(base) = backup_base(&app, &slot.root_key) {
        prune_backups(&base, keep);
    }
    Ok(())
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
        Some(schema) if schema == u64::from(SNAPSHOT_SCHEMA) => {
            manifest.get("writer").and_then(serde_json::Value::as_str) != Some(SNAPSHOT_WRITER)
        }
        Some(_) => true,
    }
}

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

    /// I-22: a snapshot mirrors the graph's directory depth. Linux caps a
    /// path near 2000 one-letter levels, Windows long paths near 16,000, so
    /// the Linux-maximal tree runs on one ninth of a 2 MiB worker stack.
    #[test]
    fn deep_payload_directories_sync_without_recursion() {
        let root = scratch("backup-deep-payload");
        let mut dir = root.clone();
        while dir.as_os_str().len() < 3990 {
            dir.push("d");
        }
        std::fs::create_dir_all(&dir).unwrap();
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024 / 9)
            .spawn(move || sync_payload_dirs(&root))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn backup_payload_and_directories_sync_before_publication() {
        let root = scratch("backup-publication-order");
        let partial = root.join(".partial-1");
        std::fs::create_dir_all(partial.join("pages/nested")).unwrap();
        BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
        write_payload(&partial.join("pages/nested/a.md"), b"- durable\n").unwrap();
        let final_dest = root.join("complete-1");
        let manifest = SnapshotManifest {
            schema: LEGACY_SNAPSHOT_SCHEMA,
            root: "test".into(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            graph_text_policy: None,
            writer: None,
            files: vec![],
            complete: true,
        };
        publish_snapshot(&partial, &final_dest, &manifest).unwrap();
        let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
        let position = |name| ops.iter().position(|op| *op == name).unwrap();
        assert!(position("payload_sync") < position("payload_dir_sync"));
        assert!(position("payload_dir_sync") < position("manifest_sync"));
        assert!(position("manifest_sync") < position("publish_rename"));
        assert!(position("publish_rename") < position("publication_dir_sync"),
            "I-1/I-2: backup publication follows fsynced payload and directory; exemplar backup.rs publish_snapshot");
        assert_eq!(
            std::fs::read(final_dest.join("pages/nested/a.md")).unwrap(),
            b"- durable\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_backup_copy_failure_reaches_diagnostic_adapter() {
        let root = scratch("launch-backup-copy-error");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/note.md"), b"- keep\n").unwrap();
        let dest = root.join("blocked-destination");
        std::fs::write(&dest, b"already a file").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let (copied, failed, failure) =
            copy_store_area(&store, Area::Pages, &dest, is_graph_text, &|| false);
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
        let dest = root.join("backup-out");
        let (copied, failed, failure) =
            copy_store_area(&store, Area::Pages, &dest, is_graph_text, &|| false);
        assert_eq!((copied, failed), (1, 0));
        assert!(failure.is_none());
        assert_eq!(
            std::fs::read(dest.join("nested/Note.md")).unwrap(),
            b"- note\n"
        );
        assert!(!dest.join("Ignore.txt").exists());
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
        let dest = root.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        for dir in ["journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        std::fs::write(src.join("note.md"), b"secret").unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let (copied, failed, failure) =
            copy_store_area(&store, Area::Pages, &dest, is_graph_text, &|| true);
        assert_eq!((copied, failed), (0, 1));
        assert_eq!(failure.unwrap().kind, ErrorKind::Interrupted);
        assert!(!dest.exists());
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
        let dst = root.join("backup").join("assets");
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
        let (copied, failed, failure) =
            copy_store_area(&store, Area::Assets, &dst, is_asset_sidecar, &|| false);
        assert_eq!((copied, failed), (2, 0));
        assert!(failure.is_none());
        assert_eq!(
            std::fs::read_to_string(dst.join("doc.edn")).unwrap(),
            "{:a 1}\n"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nested").join("hl.edn")).unwrap(),
            "{:b 2}\n"
        );
        assert!(!dst.join("image.png").exists());
        assert!(!dst.join("nested").join("image.png").exists());
        assert!(!dst.join(ASSET_RESTORE_RECOVERY_DIR).exists());
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn graph_text_backup_includes_nested_pages() {
        let root = scratch("nested-md-backup");
        let graph = root.join("graph");
        let pages = graph.join("pages");
        let journals = graph.join("journals");
        let backup = root.join("backup");
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
        let (copied_pages, failed_pages, _) = copy_store_area(
            &store,
            Area::Pages,
            &backup.join("pages"),
            is_graph_text,
            &|| false,
        );
        let (copied_journals, failed_journals, _) = copy_store_area(
            &store,
            Area::Journals,
            &backup.join("journals"),
            is_graph_text,
            &|| false,
        );
        let copied = copied_pages + copied_journals;
        let failed = failed_pages + failed_journals;
        let complete = failed == 0 && copied == live_pages + live_journals;
        assert_eq!(live_pages, 2);
        assert_eq!(live_journals, 1);
        assert!(complete);
        assert_eq!(
            std::fs::read(backup.join("pages/Top.md")).unwrap(),
            b"top\n"
        );
        assert_eq!(
            std::fs::read(backup.join("pages/client-a/Deep.md")).unwrap(),
            b"deep\n"
        );
        assert_eq!(
            std::fs::read(backup.join("journals/2026_07_09.md")).unwrap(),
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
