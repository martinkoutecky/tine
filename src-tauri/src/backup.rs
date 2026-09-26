use crate::settings::{settings_path, update_settings};
use crate::state::{slot_for_context, GraphContext, GraphSlot};
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Manager;
use tine_store::{Area, RestoreFile, Store};

// Snapshot the graph's markdown into the OS app-data dir on open, keeping the
// last few. Local-only (outside the graph, so Syncthing never sees it); a safety
// net against a bad write or accidental edit. Best-effort and fully detached so
// it never blocks startup or holds the graph lock during file copies.
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
        crate::debug::diag(token);
    }
}
#[cfg(test)]
const ASSET_RESTORE_RECOVERY_DIR: &str = ".tine-restore-recovery";

pub(crate) fn backup_async(app: tauri::AppHandle, slot: Arc<GraphSlot>) {
    let Ok(source) = BackupSource::from_store(&slot.store, &slot.root_key) else {
        report_launch_outcome(&BackupOutcome::failed(0, "source", ErrorKind::Other));
        return;
    };
    std::thread::spawn(move || {
        // Defer the launch snapshot ~1s so its whole-graph file copy doesn't
        // contend for disk I/O with first-journal paint and the warm-cache parse
        // at open (felt on slow/NFS disks or a throttled laptop). Safe: the
        // snapshot guards this session's edits, and the user hasn't edited yet in
        // the first second — the on-disk files are still intact — so a crash in
        // that window loses nothing the snapshot would have protected.
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if slot.background_cancelled.load(Ordering::Acquire) {
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
            report_launch_outcome(&outcome);
        }
    });
}

pub(crate) fn backup_graph_now(
    app: &tauri::AppHandle,
    store: &Store,
    root: &std::path::Path,
    suffix: &str,
) -> BackupOutcome {
    let Ok(source) = BackupSource::from_store(store, root) else {
        return BackupOutcome::failed(0, "source", ErrorKind::Other);
    };
    do_backup_source(app, store, source, suffix)
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
}

impl BackupSource {
    fn from_store(store: &Store, root: &std::path::Path) -> Result<Self, String> {
        let config = store.config();
        let root = Store::canonical_root(root).map_err(|error| error.to_string())?;
        let probe = store
            .file_id(Area::Assets, "__tine_backup_probe__")
            .map_err(|error| format!("unsafe assets directory: {error:?}"))?;
        let assets_dir_name = store
            .path_for_os_handoff(&probe, false)
            .map_err(|error| format!("unsafe assets directory: {error:?}"))?
            .parent()
            .ok_or("unsafe assets directory")?
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dir")
            .to_owned();
        Ok(Self {
            root,
            journals_dir: config.journals_dir.clone(),
            pages_dir: config.pages_dir.clone(),
            assets_dir_name,
        })
    }
}

const SNAPSHOT_SCHEMA: u32 = 2;
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
    files: Vec<SnapshotFile>,
    complete: bool,
}

fn root_backup_id(root: &std::path::Path) -> String {
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
    sync_dir(dir)
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

fn sync_dir(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // FlushFileBuffers does not support directory handles. The two
        // namespace publications use MoveFileExW(MOVEFILE_WRITE_THROUGH).
        let _ = path;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(path)?.sync_all()
    }
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

fn sync_payload_dirs(dir: &std::path::Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_payload_dirs(&entry.path())?;
        }
    }
    sync_dir(dir)?;
    record_backup_op("payload_dir_sync");
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
    sync_dir(final_dest.parent().expect("snapshot has parent"))?;
    record_backup_op("publication_dir_sync");
    Ok(())
}

fn read_manifest(dir: &std::path::Path) -> Option<SnapshotManifest> {
    let bytes = std::fs::read(dir.join(SNAPSHOT_MANIFEST)).ok()?;
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes).ok()?;
    (manifest.schema == SNAPSHOT_SCHEMA && manifest.complete).then_some(manifest)
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
    include: fn(&std::path::Path) -> bool,
    cancelled: &dyn Fn() -> bool,
) -> (usize, usize, Option<BackupFailure>) {
    let phase = match area {
        Area::Journals => "journals",
        Area::Pages => "pages",
        Area::Assets => "assets",
        Area::Meta => "config",
        Area::Trash => "trash",
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
        Err(_) => {
            return (
                0,
                1,
                Some(BackupFailure {
                    phase,
                    kind: ErrorKind::Other,
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
        if !include(std::path::Path::new(&entry.rel)) {
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
        tine_store::StoreError::InvalidTarget(_) => ErrorKind::InvalidInput,
        tine_store::StoreError::TooLarge { .. } => ErrorKind::FileTooLarge,
        tine_store::StoreError::Closed => ErrorKind::BrokenPipe,
    }
}

fn count_store_text(store: &Store, area: Area) -> Option<usize> {
    let listing = store.scan_area(area, None).ok()?;
    if listing
        .unreadable
        .iter()
        .any(|(_, error)| error.kind != std::io::ErrorKind::NotFound)
    {
        return None;
    }
    Some(
        listing
            .files
            .iter()
            .filter(|entry| is_graph_text(std::path::Path::new(&entry.rel)))
            .count(),
    )
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
    let stamp = backup_stamp();
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
    let Some(live_text_n) = count_store_text(store, Area::Journals)
        .and_then(|journals| count_store_text(store, Area::Pages).map(|pages| journals + pages))
    else {
        return BackupOutcome::failed(0, "inventory", ErrorKind::Other);
    };
    let (cj, fj, ej) = copy_store_area(
        store,
        Area::Journals,
        &dest.join("journals"),
        is_graph_text,
        cancelled,
    );
    let (cp, fp, ep) = copy_store_area(
        store,
        Area::Pages,
        &dest.join("pages"),
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
    let mut n = cj + cp + ca;
    let mut failed = fj + fp + fa;
    let mut first_failure = ej.or(ep).or(ea);
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
            Err(_) => {
                failed += 1;
                first_failure.get_or_insert(BackupFailure {
                    phase: "config",
                    kind: ErrorKind::Other,
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
    if cj + cp != live_text_n {
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
        files,
        complete: true,
    };
    if let Err(error) = publish_snapshot(&dest, &final_dest, &manifest) {
        return BackupOutcome::failed(n, "publish", error.kind());
    }
    partial.committed = true;
    prune_backups(&base, backup_keep(app));
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

/// Restore a snapshot into the live graph, overwriting `journals/`, `pages/`,
/// asset `.edn` sidecars, and `config.edn`. Takes a fresh safety snapshot of the
/// *current* state first (so a mistaken restore is itself reversible).
/// Destructive — the frontend confirms.
#[tauri::command]
pub(crate) async fn restore_backup(
    stamp: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    // Guard against path traversal — a stamp is only ever `YYYY-MM-DD_HH-MM-SS`.
    if stamp.is_empty()
        || !stamp
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("invalid backup id".into());
    }
    let slot = slot_for_context(&state)?;
    let source = BackupSource::from_store(&slot.store, &slot.root_key)
        .map_err(|message| format!("backup-failed:source:Other: {message}"))?;
    let restore_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let base = backup_base_for_root(&restore_app, &source.root).ok_or("no app-data dir")?;
        restore_from_backup_source(&stamp, &base, &slot.store, source, |source| {
            do_backup_source(&restore_app, &slot.store, source.clone(), "pre-restore")
        })
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(())
}

fn restore_from_backup_source(
    stamp: &str,
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    snapshot_current: impl FnOnce(&BackupSource) -> BackupOutcome,
) -> Result<(), String> {
    let src = base.join(stamp);
    if !src.is_dir() {
        return Err("backup not found".into());
    }
    let manifest = read_manifest(&src).ok_or("backup is incomplete or unverified")?;
    if manifest.root != source.root.display().to_string() {
        return Err("backup belongs to a different graph".into());
    }
    if !verify_snapshot(&src, &manifest) {
        return Err("backup contents do not match the verified manifest".into());
    }
    let safe_dir = |raw: &str| -> Result<(), String> {
        let rel = std::path::Path::new(raw);
        if raw.is_empty()
            || raw.contains('\\')
            || rel.is_absolute()
            || rel
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("backup contains an unsafe graph directory".into());
        }
        Ok(())
    };
    safe_dir(&manifest.journals_dir)?;
    safe_dir(&manifest.pages_dir)?;
    // Restore targets the open graph's current layout; a backup taken under a
    // different :pages-directory / :journals-directory is not restored (v0.6.5
    // wrote into the backup's old directory names).
    if manifest.journals_dir != source.journals_dir || manifest.pages_dir != source.pages_dir {
        return Err("backup was made with a different pages or journals directory setting".into());
    }
    if ["journals", "pages", &source.assets_dir_name]
        .into_iter()
        .any(|area| !src.join(area).is_dir())
    {
        return Err("backup contents do not match the verified manifest".into());
    }
    let snapshot = snapshot_current(&source);
    let live_n = [Area::Journals, Area::Pages, Area::Assets]
        .into_iter()
        .map(|area| {
            store.scan_area(area, None).ok().map(|listing| {
                listing
                    .files
                    .iter()
                    .filter(|entry| match area {
                        Area::Assets => is_asset_sidecar(std::path::Path::new(&entry.rel)),
                        _ => is_graph_text(std::path::Path::new(&entry.rel)),
                    })
                    .count()
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            format!(
                "{}: couldn't create a complete pre-restore safety snapshot — restore aborted",
                BackupFailure {
                    phase: "live-inventory",
                    kind: ErrorKind::Other
                }
                .wire()
            )
        })?
        .into_iter()
        .sum::<usize>();
    require_safety_snapshot(snapshot, live_n)?;
    let files = open_verified_restore_files(&src, &manifest, &source)?;
    store
        .restore(files)
        .map_err(|error| format_restore_failure(&error))?;
    Ok(())
}

fn require_safety_snapshot(snapshot: BackupOutcome, live_n: usize) -> Result<(), String> {
    if live_n > 0 && (snapshot.copied == 0 || snapshot.failure.is_some()) {
        let token = snapshot.failure.map_or_else(
            || {
                BackupFailure {
                    phase: "safety-snapshot",
                    kind: ErrorKind::InvalidData,
                }
                .wire()
            },
            |failure| failure.wire(),
        );
        return Err(format!(
            "{token}: couldn't create a complete pre-restore safety snapshot — restore aborted"
        ));
    }
    Ok(())
}

fn format_restore_failure(error: &tine_store::RestoreFailed) -> String {
    let recovery = error
        .done
        .recovery
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let kept = error
        .done
        .kept_external
        .iter()
        .map(|file| file.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "restore-failed:{:?}: {}: {}; recovery: {}; kept live: {}",
        error.cause.kind, error.phase, error.cause.message, recovery, kept
    )
}

fn open_verified_restore_files(
    snapshot: &std::path::Path,
    manifest: &SnapshotManifest,
    source: &BackupSource,
) -> Result<Vec<RestoreFile>, String> {
    let mut files = Vec::new();
    // Preserve the old area order: journals, pages, asset sidecars, config.
    for (prefix, area) in [
        ("journals", Area::Journals),
        ("pages", Area::Pages),
        (source.assets_dir_name.as_str(), Area::Assets),
        ("logseq", Area::Meta),
    ] {
        for entry in &manifest.files {
            let Some(rel) = entry
                .path
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
            else {
                continue;
            };
            let accepted = match area {
                Area::Journals | Area::Pages => is_graph_text(std::path::Path::new(rel)),
                Area::Assets => is_asset_sidecar(std::path::Path::new(rel)),
                Area::Meta => rel == "config.edn",
                Area::Trash => false,
            };
            if !accepted {
                continue;
            }
            let path = std::path::Path::new(rel);
            if rel.is_empty()
                || rel.contains('\\')
                || path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("backup contents do not match the verified manifest".into());
            }
            let mut file = std::fs::File::open(snapshot.join(&entry.path))
                .map_err(|_| "backup contents do not match the verified manifest")?;
            let meta = file
                .metadata()
                .map_err(|_| "backup contents do not match the verified manifest")?;
            if !meta.is_file() {
                return Err("backup contents do not match the verified manifest".into());
            }
            let mut hasher = Sha256::new();
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = file
                    .read(&mut buf)
                    .map_err(|_| "backup contents do not match the verified manifest")?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            if format!("{:x}", hasher.finalize()) != entry.sha256 {
                return Err("backup contents do not match the verified manifest".into());
            }
            files.push(RestoreFile {
                area,
                rel: rel.into(),
                source: file,
                len: meta.len(),
            });
        }
    }
    Ok(files)
}

/// Page/journal text files Tine snapshots + restores: Markdown and Org. Asset
/// `.edn` sidecars are handled separately under `assets`; binary asset bytes stay
/// excluded from snapshots by design.
fn is_graph_text(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|x| x.to_str()),
        Some("md") | Some("org")
    )
}

fn is_asset_sidecar(p: &std::path::Path) -> bool {
    matches!(p.extension().and_then(|x| x.to_str()), Some("edn"))
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
    dirs.sort(); // timestamp-named → chronological
    if dirs.len() > keep {
        for d in &dirs[..dirs.len() - keep] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
/// UTC `YYYY-MM-DD_HH-MM-SS` from the system clock (Hinnant civil-from-days).
fn backup_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
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
            schema: SNAPSHOT_SCHEMA,
            root: "test".into(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
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
    fn restore_wire_error_names_recovery_paths_and_kind() {
        let root = scratch("restore-wire-error");
        let store = Store::open(&root, Default::default()).unwrap().0;
        let recovery = root.join("logseq/.tine-trash/restore-1");
        let error = tine_store::RestoreFailed {
            phase: "copy pages".into(),
            cause: tine_store::IoError {
                kind: std::io::ErrorKind::PermissionDenied,
                message: "copy failed".into(),
            },
            done: tine_store::RestoreReport {
                restored: 1,
                recovery: vec![recovery.clone()],
                kept_external: Vec::new(),
                graph_rev: store.whole_graph().unwrap().rev(),
            },
        };
        let wire = format_restore_failure(&error);
        assert!(
            wire.starts_with("restore-failed:PermissionDenied:"),
            "I-9: restore wire keeps family; exemplar backup.rs restore_from_backup_source: {wire}"
        );
        assert!(wire.contains(&recovery.display().to_string()), "I-9: restore wire names recovery location; exemplar backup.rs restore_from_backup_source: {wire}");
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_failure_wire_has_fixed_phase_and_kind() {
        let wire = require_safety_snapshot(
            BackupOutcome::failed(2, "pages", ErrorKind::PermissionDenied),
            3,
        )
        .unwrap_err();
        assert!(wire.starts_with("backup-failed:pages:PermissionDenied:"),
            "I-9: pre-restore backup errors need a fixed family token; exemplar backup.rs require_safety_snapshot: {wire}");
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
            schema: SNAPSHOT_SCHEMA,
            root: root.display().to_string(),
            journals_dir: "diary".into(),
            pages_dir: "archive/pages".into(),
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
                schema: SNAPSHOT_SCHEMA,
                root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
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
    fn restore_verifies_a_selected_snapshot_before_mutating_the_graph() {
        let root = scratch("restore-verification-before-mutation");
        let graph = root.join("graph");
        let base = root.join("backups");
        let stamp = "2026-07-22_12-00-00";
        let snapshot = base.join(stamp);
        let live_page = graph.join("pages/note.md");
        std::fs::create_dir_all(live_page.parent().unwrap()).unwrap();
        std::fs::create_dir_all(snapshot.join("pages")).unwrap();
        std::fs::write(&live_page, b"live graph data").unwrap();
        std::fs::write(snapshot.join("pages/note.md"), b"tampered payload").unwrap();
        write_manifest(
            &snapshot,
            &SnapshotManifest {
                schema: SNAPSHOT_SCHEMA,
                root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                files: vec![SnapshotFile {
                    path: "pages/note.md".into(),
                    sha256: "does not match the payload".into(),
                }],
                complete: true,
            },
        )
        .unwrap();
        let source = BackupSource {
            root: graph.clone(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            assets_dir_name: "assets".into(),
        };
        for dir in ["journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();

        PAYLOAD_HASH_READS.with(|reads| reads.set(0));
        let result = restore_from_backup_source(stamp, &base, &store, source, |_| {
            std::fs::write(&live_page, b"mutated graph data").unwrap();
            BackupOutcome::success(1)
        });

        assert_eq!(
            PAYLOAD_HASH_READS.with(|reads| reads.get()),
            1,
            "restoring must verify the selected snapshot payload"
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&live_page).unwrap(), b"live graph data");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn verified_snapshot_files_reach_store_restore() {
        let root = scratch("verified-restore-handoff");
        let graph = root.join("graph");
        let snapshot = root.join("backups/2026-07-22_12-00-00");
        for dir in ["pages", "journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
            std::fs::create_dir_all(snapshot.join(dir)).unwrap();
        }
        std::fs::write(graph.join("pages/Old.md"), b"old").unwrap();
        std::fs::write(snapshot.join("pages/New.md"), b"new").unwrap();
        let manifest = SnapshotManifest {
            schema: SNAPSHOT_SCHEMA,
            root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            files: snapshot_inventory(&snapshot).unwrap(),
            complete: true,
        };
        write_manifest(&snapshot, &manifest).unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        restore_from_backup_source(
            "2026-07-22_12-00-00",
            &root.join("backups"),
            &store,
            source,
            |_| BackupOutcome::success(1),
        )
        .unwrap();
        assert_eq!(std::fs::read(graph.join("pages/New.md")).unwrap(), b"new");
        assert!(!graph.join("pages/Old.md").exists());
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_empty_snapshot_area_cannot_retire_live_files() {
        let root = scratch("missing-empty-snapshot-area");
        let graph = root.join("graph");
        let snapshot = root.join("backups/2026-07-22_12-00-00");
        for dir in ["pages", "journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        for dir in ["pages", "assets"] {
            std::fs::create_dir_all(snapshot.join(dir)).unwrap();
        }
        std::fs::write(graph.join("journals/Old.md"), b"old").unwrap();
        write_manifest(
            &snapshot,
            &SnapshotManifest {
                schema: SNAPSHOT_SCHEMA,
                root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                files: Vec::new(),
                complete: true,
            },
        )
        .unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let result = restore_from_backup_source(
            "2026-07-22_12-00-00",
            &root.join("backups"),
            &store,
            source,
            |_| panic!("missing snapshot area must be rejected before the safety snapshot"),
        );
        assert_eq!(
            result.unwrap_err(),
            "backup contents do not match the verified manifest"
        );
        assert_eq!(
            std::fs::read(graph.join("journals/Old.md")).unwrap(),
            b"old"
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
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
