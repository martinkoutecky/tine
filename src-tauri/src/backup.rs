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

mod archive;
mod naming;
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

/// The user-visible launch failure, if any. A cancelled backup and one
/// another Tine process kept `busy` past the lock wait are skipped, not
/// failed: the next launch backs up (the busy one is logged).
fn launch_failure_token(outcome: &BackupOutcome) -> Option<String> {
    outcome
        .failure
        .as_ref()
        .filter(|failure| !matches!(failure.phase, "cancelled" | "busy"))
        .map(BackupFailure::wire)
}

pub(crate) fn report_launch_outcome(outcome: &BackupOutcome) {
    if outcome
        .failure
        .as_ref()
        .is_some_and(|failure| failure.phase == "busy")
    {
        crate::debug::diag_private("backup-busy", "launch backup skipped");
    }
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
/// recovery. The tagged snapshot counts toward the keep count like a launch
/// one (only `pre-restore` snapshots are exempt).
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

/// The live graph layout a snapshot records and a restore checks against.
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

/// Legacy snapshot directories in `<id>/`, which this build lists, restores
/// and prunes but no longer writes (it writes schema-5 zips, `archive.rs`).
/// Schema 3 (og-B, ADR 0062) keeps graph text under `graph/<graph-relative
/// path>` and records the graph-text scope it covered; schema 2 kept only the
/// configured `journals/` and `pages/` roots. Both are master's wire
/// formats, so either build reads the other's.
const SNAPSHOT_SCHEMA: u32 = 3;
const LEGACY_SNAPSHOT_SCHEMA: u32 = 2;
/// Master's `GRAPH_TEXT_SCOPE_VERSION`: the discovery exclusions this build's
/// `graph_text_eligible` applies (`published-queries/` included).
const GRAPH_TEXT_SCOPE_VERSION: u32 = 2;
/// Marks this build's snapshots (schema-3 directories before, zip manifests
/// now); master ignores the field. Prune counts only snapshots this build
/// wrote (docs/app-identity.md).
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
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// A legacy (schema 2/3) snapshot manifest, for test fixtures: this build
/// writes only zips.
#[cfg(test)]
fn write_manifest(dir: &std::path::Path, manifest: &SnapshotManifest) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(SNAPSHOT_MANIFEST), bytes)
}

#[cfg(test)]
std::thread_local! {
    static BACKUP_OPS: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Test record of a completed durability step or publication step, in order.
fn record_backup_op(op: &'static str) {
    #[cfg(test)]
    BACKUP_OPS.with(|ops| ops.borrow_mut().push(op));
    #[cfg(not(test))]
    let _ = op;
}

/// A test hook at a named step of publication or pruning.
#[cfg(test)]
enum Hook {
    /// The step fails with this kind.
    Fail(ErrorKind),
    /// The directory sync at this step reports no platform witness.
    Weak,
    /// Run before the step (a pause, a crash by panic, another process).
    Run(Box<dyn FnOnce()>),
}

#[cfg(test)]
std::thread_local! {
    static HOOKS: std::cell::RefCell<Vec<(&'static str, Hook)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The test seam before each named step: runs or injects the first hook set
/// for `op` on this thread. `Ok(true)` asks a directory sync to report a weak
/// witness. Production: always `Ok(false)`.
fn step(op: &'static str) -> std::io::Result<bool> {
    #[cfg(test)]
    {
        let hook = HOOKS.with(|hooks| {
            let mut hooks = hooks.borrow_mut();
            let at = hooks.iter().position(|(name, _)| *name == op)?;
            // `drain`, not `remove`: host_slot_guard_tests resolves callees by name.
            let (_, hook) = hooks.drain(at..=at).next()?;
            Some(hook)
        });
        match hook {
            Some(Hook::Fail(kind)) => return Err(std::io::Error::new(kind, "injected")),
            Some(Hook::Weak) => return Ok(true),
            Some(Hook::Run(run)) => run(),
            None => {}
        }
    }
    let _ = op;
    Ok(false)
}

/// The durability a publication or a keeper certification reached
/// (REVIEW-backup-zip R1). `Weak`: a directory sync the platform does not
/// offer (the shared helper's tolerated set); a real sync error is `Err`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Durability {
    Weak,
    Strong,
}

fn sync_dir(dir: &std::path::Path, op: &'static str) -> std::io::Result<Durability> {
    let weak = step(op)?;
    let witness = tine_store::directory_durability::sync_directory_witness(dir)?;
    record_backup_op(op);
    Ok(
        if weak || witness == tine_store::directory_durability::DirectoryWitness::Unsupported {
            Durability::Weak
        } else {
            Durability::Strong
        },
    )
}

fn sync_file(file: &std::fs::File, op: &'static str) -> std::io::Result<()> {
    step(op)?;
    file.sync_all()?;
    record_backup_op(op);
    Ok(())
}

/// `<id>.zips/`'s parent chain up to app data: `backups/`, then app data
/// (REVIEW-backup-zip B2: the first publication and every keeper
/// certification settle the whole chain).
fn sync_ancestors(zips: &std::path::Path) -> std::io::Result<Durability> {
    let backups = zips.parent().ok_or(ErrorKind::InvalidInput)?;
    let app_data = backups.parent().ok_or(ErrorKind::InvalidInput)?;
    let first = sync_dir(backups, "backups_dir_sync")?;
    Ok(first.min(sync_dir(app_data, "app_data_dir_sync")?))
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

/// Read every included file of `area` into `out` at `prefix` + its
/// area-relative path, through Store, each bounded by the archive's
/// per-entry limit. Returns (read, failed, first failure).
fn read_store_area(
    store: &Store,
    area: Area,
    prefix: &str,
    include: fn(&tine_store::FileId) -> bool,
    cancelled: &dyn Fn() -> bool,
    out: &mut Vec<(String, Vec<u8>)>,
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
    let mut read = 0;
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
                read,
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
        match store.read(&entry.id, Some(archive::MAX_ENTRY_BYTES)) {
            Ok((bytes, _)) => {
                out.push((format!("{prefix}{}", entry.rel), bytes));
                read += 1;
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
    (read, failed, first_failure)
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
    write_snapshot(
        &base,
        store,
        source,
        suffix,
        cancelled,
        backup_keep(app),
        BACKUP_LOCK_WAIT,
    )
}

/// `<app-data>/backups/<id>.zips/`, this build's snapshot namespace, beside
/// the legacy `<id>/` directory (`base`). Builds before the zip format read
/// only `<id>/`, so they never list, prune or clean it (docs/app-identity.md).
fn zip_base(base: &std::path::Path) -> PathBuf {
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(".zips");
    base.with_file_name(name)
}

/// How long a backup or a restore waits for another Tine process's hold
/// on the namespace lock before giving up (launch: skip, logged as busy).
const BACKUP_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The graph's backed-up files, read once into memory: the zip stores and
/// the manifest hashes the same bytes (REVIEW-backup-zip R5).
struct Capture {
    files: Vec<(String, Vec<u8>)>,
    manifest: archive::ZipManifest,
}

/// Read the backed-up file set (what is backed up is unchanged: graph text
/// at its graph-relative path, asset sidecars under the assets directory,
/// `logseq/config.edn`). Any unreadable file fails the capture.
fn capture(
    store: &Store,
    source: BackupSource,
    cancelled: &dyn Fn() -> bool,
) -> Result<Capture, BackupOutcome> {
    let live_text_n = count_store_text(store, Area::Graph)
        .map_err(|kind| BackupOutcome::failed(0, "inventory", kind))?;
    let mut files = Vec::new();
    let (ct, ft, et) =
        read_store_area(store, Area::Graph, "", is_graph_text, cancelled, &mut files);
    let (_, fa, ea) = read_store_area(
        store,
        Area::Assets,
        &format!("{}/", source.assets_dir_name),
        is_asset_sidecar,
        cancelled,
        &mut files,
    );
    let mut failed = ft + fa;
    let mut first_failure = et.or(ea);
    if !cancelled() {
        match store.scan_area(Area::Meta, None) {
            Ok(listing) => {
                if let Some((_, error)) = listing
                    .unreadable
                    .iter()
                    .find(|(_, error)| error.kind != ErrorKind::NotFound)
                {
                    failed += 1;
                    first_failure.get_or_insert(BackupFailure {
                        phase: "config",
                        kind: error.kind,
                    });
                }
                if let Some(config) = listing.files.iter().find(|entry| entry.rel == "config.edn") {
                    match store.read(&config.id, Some(archive::MAX_ENTRY_BYTES)) {
                        Ok((bytes, _)) => files.push(("logseq/config.edn".into(), bytes)),
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
    let n = files.len();
    if cancelled() {
        return Err(BackupOutcome::failed(
            n,
            "cancelled",
            ErrorKind::Interrupted,
        ));
    }
    if failed != 0 {
        return Err(BackupOutcome {
            copied: n,
            failure: first_failure.or(Some(BackupFailure {
                phase: "copy",
                kind: ErrorKind::Other,
            })),
        });
    }
    if ct != live_text_n {
        return Err(BackupOutcome::failed(
            n,
            "inventory",
            ErrorKind::InvalidData,
        ));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let manifest = archive::ZipManifest {
        schema: archive::ZIP_SCHEMA,
        writer: SNAPSHOT_WRITER.into(),
        root: source.root.display().to_string(),
        journals_dir: source.journals_dir,
        pages_dir: source.pages_dir,
        assets_dir: source.assets_dir_name,
        graph_text_policy: SnapshotGraphTextPolicy {
            version: GRAPH_TEXT_SCOPE_VERSION,
            hidden: source.hidden,
            hidden_parse_failed_closed: source.hidden_parse_failed_closed,
        },
        files: files
            .iter()
            .map(|(path, bytes)| archive::ZipFile {
                path: path.clone(),
                sha256: format!("{:x}", Sha256::digest(bytes)),
                len: bytes.len() as u64,
            })
            .collect(),
        complete: true,
    };
    Ok(Capture { files, manifest })
}

/// Take one snapshot of the graph into `<base>.zips/` (docs/storage-contract.md,
/// Graph backups): capture outside the lock, then, holding the namespace
/// lock, skip an unchanged launch, publish one zip and prune. An empty
/// graph writes nothing (`copied == 0`). `suffix` tags special snapshots
/// (`pre-restore`, `pre-journal-rename`); those are always written.
fn write_snapshot(
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    suffix: &str,
    cancelled: &dyn Fn() -> bool,
    keep: usize,
    lock_wait: std::time::Duration,
) -> BackupOutcome {
    let capture = match capture(store, source, cancelled) {
        Ok(capture) => capture,
        Err(outcome) => return outcome,
    };
    let n = capture.files.len();
    if n == 0 {
        return BackupOutcome::success(0);
    }
    let zips = zip_base(base);
    if let Err(error) = std::fs::create_dir_all(&zips) {
        return BackupOutcome::failed(n, "reserve", error.kind());
    }
    // Concurrent honest Tine processes (REVIEW-backup-zip B1, R2, R3): one
    // holder at a time allocates names, publishes, cleans and prunes.
    let lock = match crate::file_lock::lock_exclusive_until(
        &zips.join(naming::LOCK_FILE),
        std::time::Instant::now() + lock_wait,
    ) {
        Ok(Some(lock)) => lock,
        Ok(None) => return BackupOutcome::failed(n, "busy", ErrorKind::WouldBlock),
        Err(error) => return BackupOutcome::failed(n, "lock", error.kind()),
    };
    let outcome = publish_locked(base, &zips, capture, suffix, keep);
    drop(lock);
    outcome
}

/// One recognized snapshot in either namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    name: naming::SnapshotName,
    namespace: naming::Namespace,
    path: PathBuf,
}

impl Record {
    /// Newest-last order across both namespaces (R4).
    fn order(&self, other: &Self) -> std::cmp::Ordering {
        self.name
            .chrono_cmp(&other.name)
            .then(self.namespace.cmp(&other.namespace))
            .then_with(|| self.name.suffix.cmp(&other.name.suffix))
    }
}

/// Both namespaces' recognized snapshots, every recognized name (partials
/// too, for counter reservation), and the unknown entries.
#[derive(Default)]
struct Scan {
    records: Vec<Record>,
    names: Vec<naming::SnapshotName>,
    unknown: Vec<PathBuf>,
}

fn scan(base: &std::path::Path, zips: &std::path::Path) -> std::io::Result<Scan> {
    use naming::{Entry, Namespace};
    let mut scan = Scan::default();
    for (dir, namespace) in [(base, Namespace::Legacy), (zips, Namespace::Zip)] {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
                scan.unknown.push(path);
                continue;
            };
            let classified = match namespace {
                Namespace::Legacy => naming::classify_legacy(&file_name),
                Namespace::Zip => naming::classify_zip(&file_name),
            };
            match classified {
                Entry::Snapshot(name) if namespace == Namespace::Zip || path.is_dir() => {
                    scan.names.push(name.clone());
                    scan.records.push(Record {
                        name,
                        namespace,
                        path,
                    });
                }
                Entry::Partial(name) => scan.names.push(name),
                Entry::Lock => {}
                Entry::Snapshot(_) | Entry::Unknown => scan.unknown.push(path),
            }
        }
    }
    scan.records.sort_by(Record::order);
    Ok(scan)
}

/// Under the lock no cooperating writer owns a partial: every zip partial
/// is abandoned (REVIEW-backup-zip R3). Legacy partials belong to older
/// builds, which clean their own.
fn remove_partials(zips: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(zips) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if let naming::Entry::Partial(_) = naming::classify_zip(&name.to_string_lossy()) {
            if let Err(error) = std::fs::remove_file(entry.path()) {
                crate::debug::diag_private("backup-partial-cleanup-failed", error.to_string());
            }
        }
    }
}

/// Read a whole archive file, bounded by the archive limit (R7: limit + 1,
/// then require the end).
fn read_archive(path: &std::path::Path) -> Result<Vec<u8>, archive::Problem> {
    let io = |error: std::io::Error| archive::Problem::Io(error.kind());
    let file = std::fs::File::open(path).map_err(io)?;
    let mut bytes = Vec::new();
    file.take(archive::MAX_ARCHIVE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > archive::MAX_ARCHIVE_BYTES {
        return Err(archive::Problem::TooLarge("archive"));
    }
    Ok(bytes)
}

/// The listing-time status of one zip: its manifest, read from the central
/// directory and the manifest entry only.
fn zip_listing(path: &std::path::Path) -> Result<archive::ZipManifest, archive::Problem> {
    let file = std::fs::File::open(path).map_err(|error| archive::Problem::Io(error.kind()))?;
    let len = file
        .metadata()
        .map_err(|error| archive::Problem::Io(error.kind()))?
        .len();
    archive::read_listing(&mut archive::FileSource { file, len })
}

fn publish_locked(
    base: &std::path::Path,
    zips: &std::path::Path,
    capture: Capture,
    suffix: &str,
    keep: usize,
) -> BackupOutcome {
    let n = capture.files.len();
    remove_partials(zips);
    let scan = match scan(base, zips) {
        Ok(scan) => scan,
        Err(error) => return BackupOutcome::failed(n, "scan", error.kind()),
    };
    // Item 4 + R5: a launch whose capture equals the newest state record of
    // any suffix writes nothing, but only once that record fully verifies
    // (B3); a damaged, missing or legacy baseline gets a fresh zip.
    if suffix.is_empty() {
        if let Some(newest) = scan.records.last() {
            if newest.namespace == naming::Namespace::Zip {
                let verified =
                    read_archive(&newest.path).and_then(|bytes| archive::verify(&mut &bytes[..]));
                if verified.is_ok_and(|verified| verified.manifest == capture.manifest) {
                    crate::debug::diag_private("backup-unchanged", newest.name.render());
                    return BackupOutcome::success(n);
                }
            }
        }
    }
    let first = !scan
        .records
        .iter()
        .any(|record| record.namespace == naming::Namespace::Zip);
    let Some(name) = naming::allocate(&tine_core::date::utc_backup_stamp(), suffix, &scan.names)
    else {
        return BackupOutcome::failed(n, "naming", ErrorKind::Other);
    };
    let manifest = archive::encode_manifest(&capture.manifest);
    let bytes = match archive::write_archive(&capture.files, &manifest, &name.stamp) {
        Ok(bytes) => bytes,
        Err(problem) => {
            let kind = match problem {
                archive::Problem::TooLarge(_) => ErrorKind::FileTooLarge,
                _ => ErrorKind::InvalidInput,
            };
            return BackupOutcome::failed(n, "archive", kind);
        }
    };
    match publish_zip(zips, &name, &bytes, first) {
        Err((phase, error)) => BackupOutcome::failed(n, phase, error.kind()),
        Ok((_, Durability::Weak)) => {
            // Published, visible and restorable, but without a platform
            // witness: nothing older may be retired on its strength.
            crate::debug::diag_private("backup-weak-durability", name.render());
            BackupOutcome::success(n)
        }
        Ok((path, Durability::Strong)) => {
            prune_locked(base, zips, keep, Some(&path));
            BackupOutcome::success(n)
        }
    }
}

/// Durable before visible (item 3): a `create_new` partial, one file sync,
/// a no-replace rename, then the directory sync, and on the namespace's
/// first zip the ancestors too. Runs under the namespace lock, so the
/// no-replace fallback's check-then-rename has no cooperating racer. A
/// failure before the rename removes the partial (a crash leaves it for the
/// next lock holder); one after it leaves a visible zip and reports
/// `publish-sync`.
fn publish_zip(
    zips: &std::path::Path,
    name: &naming::SnapshotName,
    bytes: &[u8],
    first: bool,
) -> Result<(PathBuf, Durability), (&'static str, std::io::Error)> {
    use std::io::Write;
    let partial = zips.join(format!(
        "{}{}{}",
        naming::PARTIAL_PREFIX,
        name.render(),
        naming::ZIP_EXT
    ));
    let path = zips.join(format!("{}{}", name.render(), naming::ZIP_EXT));
    let written = step("partial_create").and_then(|_| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH);
        }
        let mut file = options.open(&partial)?;
        file.write_all(bytes)?;
        sync_file(&file, "zip_sync")?;
        step("zip_rename")?;
        crate::device_io::move_file_noreplace(&partial, &path)
    });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(("publish", error));
    }
    record_backup_op("zip_rename");
    let durability = sync_dir(zips, "zips_dir_sync")
        .and_then(|durability| {
            Ok(if first {
                durability.min(sync_ancestors(zips)?)
            } else {
                durability
            })
        })
        .map_err(|error| ("publish-sync", error))?;
    Ok((path, durability))
}

/// The keep-count prune, under the namespace lock (item 5, REVIEW B1, R5).
/// Counted: legacy snapshots as before (this build's, not `-pre-restore`,
/// not another Tine's) and healthy zips except `-pre-restore`; oldest beyond
/// `keep` go. Damaged zips count toward nothing and go only once older than
/// the oldest retained healthy zip; unsupported and unknown entries stay.
/// Nothing is deleted unless the newest healthy zip (the keeper) fully
/// verifies and its file and whole directory chain sync strongly first.
/// `fresh`: a zip this lock hold just published with a strong result, whose
/// file and `<id>.zips/` entry need no second sync.
fn prune_locked(
    base: &std::path::Path,
    zips: &std::path::Path,
    keep: usize,
    fresh: Option<&std::path::Path>,
) {
    let scan = match scan(base, zips) {
        Ok(scan) => scan,
        Err(error) => {
            crate::debug::diag_private("backup-prune-skipped", error.to_string());
            return;
        }
    };
    for path in &scan.unknown {
        crate::debug::diag_private("backup-unknown-entry", path.display().to_string());
    }
    let mut counted = Vec::new();
    let mut healthy = Vec::new();
    let mut damaged = Vec::new();
    for record in scan.records {
        match record.namespace {
            naming::Namespace::Legacy => {
                if !record.name.is_pre_restore() && !is_foreign_snapshot(&record.path) {
                    counted.push(record);
                }
            }
            naming::Namespace::Zip => match zip_listing(&record.path) {
                // Another Tine's zip: never counted, never the keeper, never deleted.
                Ok(manifest) if manifest.writer != SNAPSHOT_WRITER => {}
                Ok(_) => {
                    if !record.name.is_pre_restore() {
                        counted.push(record.clone());
                    }
                    healthy.push(record);
                }
                Err(archive::Problem::Damaged(_) | archive::Problem::TooLarge(_)) => {
                    damaged.push(record)
                }
                Err(problem) => crate::debug::diag_private(
                    "backup-unknown-entry",
                    format!("{}: {problem:?}", record.path.display()),
                ),
            },
        }
    }
    let excess = counted.len().saturating_sub(keep);
    let retained = counted.split_off(excess);
    let mut doomed = counted;
    if let Some(oldest) = retained
        .iter()
        .find(|record| record.namespace == naming::Namespace::Zip)
    {
        doomed.extend(
            damaged
                .into_iter()
                .filter(|record| record.order(oldest).is_lt()),
        );
    }
    let keeper = healthy.last();
    doomed.retain(|record| Some(record) != keeper);
    if doomed.is_empty() {
        return;
    }
    let Some(keeper) = keeper else {
        crate::debug::diag_private("backup-prune-no-keeper", zips.display().to_string());
        return;
    };
    match certify_keeper(keeper, zips, fresh == Some(keeper.path.as_path())) {
        Ok(Durability::Strong) => {}
        Ok(Durability::Weak) => {
            crate::debug::diag_private("backup-prune-weak-keeper", keeper.name.render());
            return;
        }
        Err(error) => {
            crate::debug::diag_private("backup-prune-keeper-failed", error.to_string());
            return;
        }
    }
    for record in doomed {
        let removed = step("prune_delete").and_then(|_| match record.namespace {
            naming::Namespace::Legacy => std::fs::remove_dir_all(&record.path),
            naming::Namespace::Zip => std::fs::remove_file(&record.path),
        });
        match removed {
            Ok(()) => record_backup_op("prune_delete"),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => crate::debug::diag_private(
                "backup-prune-delete-failed",
                format!("{}: {error}", record.path.display()),
            ),
        }
    }
}

/// Establish the keeper before any deletion: full verification, then its
/// file, `<id>.zips/`, `backups/` and app data synced (REVIEW B1/B2). A
/// writer that died after its rename may have left it unsynced.
fn certify_keeper(
    keeper: &Record,
    zips: &std::path::Path,
    fresh: bool,
) -> std::io::Result<Durability> {
    step("keeper_verify")?;
    read_archive(&keeper.path)
        .and_then(|bytes| archive::verify(&mut &bytes[..]).map(|_| ()))
        .map_err(|problem| {
            std::io::Error::new(ErrorKind::InvalidData, format!("keeper: {problem:?}"))
        })?;
    let mut durability = Durability::Strong;
    if !fresh {
        let file = std::fs::OpenOptions::new().write(true).open(&keeper.path)?;
        sync_file(&file, "keeper_sync")?;
        durability = sync_dir(zips, "zips_dir_sync")?;
    }
    Ok(durability.min(sync_ancestors(zips)?))
}

/// The Settings keep-count change: prune under the lock, with the same
/// keeper rule. A busy namespace is left for the next launch's prune.
fn prune_now(base: &std::path::Path, keep: usize, lock_wait: std::time::Duration) {
    let zips = zip_base(base);
    if !zips.is_dir() {
        return; // no zip, so no keeper: nothing may be deleted
    }
    match crate::file_lock::lock_exclusive_until(
        &zips.join(naming::LOCK_FILE),
        std::time::Instant::now() + lock_wait,
    ) {
        Ok(Some(_lock)) => prune_locked(base, &zips, keep, None),
        Ok(None) => crate::debug::diag_private("backup-busy", "prune"),
        Err(error) => crate::debug::diag_private("backup-prune-skipped", error.to_string()),
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

/// One row of Settings' Backups list. `id` names the namespace
/// (`zip:<name>` or `dir:<name>`): a zip and a legacy directory with the same
/// name are distinct entries. `status`: `ok`, `damaged` (not restorable) or
/// `unsupported` (written by a Tine this build does not understand).
#[derive(Debug, serde::Serialize)]
pub(crate) struct BackupInfo {
    id: String,
    stamp: String,
    files: usize,
    status: &'static str,
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
            prune_now(&base, keep, BACKUP_LOCK_WAIT);
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

/// Both namespaces, newest first. Legacy snapshots list as before (an
/// unreadable manifest hides it); a zip lists with its status, read from
/// its central directory and manifest entry only (item 7).
fn list_backups_from_base(base: &std::path::Path, root: &std::path::Path) -> Vec<BackupInfo> {
    let current_root = Store::canonical_root(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string();
    let Ok(scan) = scan(base, &zip_base(base)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for record in scan.records.iter().rev() {
        let name = record.name.render();
        let info = match record.namespace {
            naming::Namespace::Legacy => match read_manifest(&record.path) {
                Some(manifest) if manifest.root == current_root => BackupInfo {
                    id: format!("dir:{name}"),
                    files: manifest.files.len(),
                    stamp: name,
                    status: "ok",
                },
                _ => continue,
            },
            naming::Namespace::Zip => {
                let (files, status) = match zip_listing(&record.path) {
                    Ok(manifest) if manifest.root == current_root => (manifest.files.len(), "ok"),
                    Ok(_) | Err(archive::Problem::Io(ErrorKind::NotFound)) => continue,
                    Err(archive::Problem::Unsupported(_)) => (0, "unsupported"),
                    Err(_) => (0, "damaged"),
                };
                BackupInfo {
                    id: format!("zip:{name}"),
                    stamp: name,
                    files,
                    status,
                }
            }
        };
        out.push(info);
    }
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

#[cfg(test)]
mod tests;
