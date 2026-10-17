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

/// A snapshot's result: `failure`, or the durability its zip reached
/// (REVIEW-backup-zip D1). `Weak` is published and restorable but not
/// settled: the platform offered no directory-sync witness.
#[derive(Debug)]
pub(crate) struct BackupOutcome {
    pub(crate) copied: usize,
    pub(crate) failure: Option<BackupFailure>,
    pub(crate) durability: Durability,
}

impl BackupOutcome {
    pub(crate) fn success(copied: usize, durability: Durability) -> Self {
        Self {
            copied,
            failure: None,
            durability,
        }
    }

    pub(crate) fn failed(copied: usize, phase: &'static str, kind: ErrorKind) -> Self {
        Self {
            copied,
            failure: Some(BackupFailure { phase, kind }),
            durability: Durability::Weak,
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
/// one (only `pre-restore` snapshots are exempt). A weak one proceeds
/// ([`proceed_on_safety_snapshot`]).
pub(crate) fn snapshot_before_rewrite(
    app: &tauri::AppHandle,
    slot: &GraphSlot,
    suffix: &str,
) -> Result<(), String> {
    rewrite_snapshot_result(backup_graph_now(app, &slot.store, &slot.root_key, suffix))
}

fn rewrite_snapshot_result(outcome: BackupOutcome) -> Result<(), String> {
    match outcome.failure {
        None => {
            proceed_on_safety_snapshot(&outcome);
            Ok(())
        }
        Some(failure) => Err(failure.wire()),
    }
}

/// A safety snapshot (pre-restore, pre-journal-*) that published but is
/// weak lets its operation proceed, logged (D1): the guarantee is
/// conditional on a platform directory-sync witness, and refusing where
/// none exists would remove restore entirely there. A sync error refuses.
fn proceed_on_safety_snapshot(outcome: &BackupOutcome) {
    if outcome.durability == Durability::Weak {
        crate::debug::diag("backup-safety-weak");
    }
}

/// The live graph layout a snapshot records and a restore checks against.
/// `assets_dir_name` names the assets area inside a legacy snapshot
/// directory only; a zip always holds it at `assets/` (D5).
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
pub(crate) enum Durability {
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

/// The whole directory chain of a zip in `<id>.zips/`: that directory,
/// `backups/`, then app data. Every publication and every settle runs it;
/// nothing infers it from an earlier zip (REVIEW-backup-zip B2, D1).
fn sync_chain(zips: &std::path::Path) -> std::io::Result<Durability> {
    let backups = zips.parent().ok_or(ErrorKind::InvalidInput)?;
    let app_data = backups.parent().ok_or(ErrorKind::InvalidInput)?;
    let zips = sync_dir(zips, "zips_dir_sync")?;
    let backups = sync_dir(backups, "backups_dir_sync")?;
    Ok(zips
        .min(backups)
        .min(sync_dir(app_data, "app_data_dir_sync")?))
}

/// Settle an already-published zip: its file, then [`sync_chain`]. No
/// bytes are written.
fn settle(
    path: &std::path::Path,
    zips: &std::path::Path,
    file_op: &'static str,
) -> std::io::Result<Durability> {
    // Write access only because Windows flushes no read-only handle.
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    sync_file(&file, file_op)?;
    sync_chain(zips)
}

/// A whole file, refusing one longer than `limit` without retaining more
/// than `limit + 1` bytes (D3).
fn read_bounded(path: &std::path::Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(ErrorKind::FileTooLarge.into());
    }
    Ok(bytes)
}

fn read_manifest(dir: &std::path::Path) -> Option<SnapshotManifest> {
    let bytes = read_bounded(&dir.join(SNAPSHOT_MANIFEST), archive::MAX_MANIFEST_BYTES).ok()?;
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

/// What a capture may still hold (D3): the archive's payload limit (the
/// manifest is not counted) and its entry count less the manifest's. Spent
/// while reading, so a graph over the limit is refused before the excess
/// is retained.
struct Budget {
    bytes: u64,
    entries: u64,
}

impl Budget {
    fn archive() -> Self {
        Self {
            bytes: archive::MAX_TOTAL_BYTES,
            entries: archive::MAX_ENTRIES - 1,
        }
    }

    /// One file through Store, within the per-entry limit and what is left.
    /// Over either: `FileTooLarge`, having read at most one byte past it.
    fn read(&mut self, store: &Store, id: &tine_store::FileId) -> Result<Vec<u8>, ErrorKind> {
        if self.entries == 0 {
            return Err(ErrorKind::FileTooLarge);
        }
        let limit = archive::MAX_ENTRY_BYTES.min(self.bytes);
        let (mut bytes, _) = store
            .read(id, Some(limit))
            .map_err(|error| store_error_kind(&error))?;
        // The capture holds every file until its zip is written: keep only
        // the bytes, not the read's growth headroom (up to 2x; og-merge
        // review M1).
        bytes.shrink_to_fit();
        self.bytes -= bytes.len() as u64;
        self.entries -= 1;
        Ok(bytes)
    }
}

/// Read every included file of `area` into `out` at `prefix` + its
/// area-relative path, through Store, within `budget`. Returns how many it
/// read, or the first failure, named by the area's phase (an unreadable or
/// over-limit file fails the whole capture, so reading stops there).
fn read_store_area(
    store: &Store,
    area: Area,
    prefix: &str,
    include: fn(&tine_store::FileId) -> bool,
    cancelled: &dyn Fn() -> bool,
    budget: &mut Budget,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<usize, BackupFailure> {
    let phase = match area {
        Area::Journals => "journals",
        Area::Pages => "pages",
        Area::Assets => "assets",
        Area::Meta => "config",
        Area::Trash => "trash",
        Area::Graph => "graph",
    };
    let fail = |kind| BackupFailure { phase, kind };
    if cancelled() {
        return Err(fail(ErrorKind::Interrupted));
    }
    let listing = store
        .scan_area(area, None)
        .map_err(|error| fail(store_error_kind(&error)))?;
    if let Some((_, error)) = listing
        .unreadable
        .iter()
        .find(|(_, error)| error.kind != ErrorKind::NotFound)
    {
        return Err(fail(error.kind));
    }
    let mut read = 0;
    for entry in listing.files {
        if cancelled() {
            return Err(fail(ErrorKind::Interrupted));
        }
        if !include(&entry.id) {
            continue;
        }
        let bytes = budget.read(store, &entry.id).map_err(fail)?;
        out.push((format!("{prefix}{}", entry.rel), bytes));
        read += 1;
    }
    Ok(read)
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
/// at its graph-relative path, asset sidecars under `assets/` whatever the
/// assets target is called (D5), `logseq/config.edn`) within `budget`.
/// Any unreadable or over-limit file fails the capture.
fn capture(
    store: &Store,
    source: BackupSource,
    cancelled: &dyn Fn() -> bool,
    mut budget: Budget,
) -> Result<Capture, BackupOutcome> {
    let live_text_n = count_store_text(store, Area::Graph)
        .map_err(|kind| BackupOutcome::failed(0, "inventory", kind))?;
    let mut files = Vec::new();
    let mut read = |area, prefix, include| {
        read_store_area(
            store,
            area,
            prefix,
            include,
            cancelled,
            &mut budget,
            &mut files,
        )
    };
    let read = read(Area::Graph, "", is_graph_text).and_then(|text_n| {
        read(Area::Assets, "assets/", is_asset_sidecar)?;
        read(Area::Meta, "logseq/", |id| {
            id.as_str() == "logseq/config.edn"
        })?;
        Ok(text_n)
    });
    let n = files.len();
    if cancelled() {
        return Err(BackupOutcome::failed(
            n,
            "cancelled",
            ErrorKind::Interrupted,
        ));
    }
    match read {
        Err(failure) => {
            return Err(BackupOutcome {
                copied: n,
                failure: Some(failure),
                durability: Durability::Weak,
            })
        }
        Ok(text_n) if text_n != live_text_n => {
            return Err(BackupOutcome::failed(
                n,
                "inventory",
                ErrorKind::InvalidData,
            ))
        }
        Ok(_) => {}
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let manifest = archive::ZipManifest {
        schema: archive::ZIP_SCHEMA,
        writer: SNAPSHOT_WRITER.into(),
        root: source.root.display().to_string(),
        journals_dir: source.journals_dir,
        pages_dir: source.pages_dir,
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
/// lock, settle an unchanged launch's baseline or publish one zip, then
/// apply the keep count. An empty graph writes nothing (`copied == 0`). `suffix` tags special snapshots
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
    let capture = match capture(store, source, cancelled, Budget::archive()) {
        Ok(capture) => capture,
        Err(outcome) => return outcome,
    };
    let n = capture.files.len();
    if n == 0 {
        return BackupOutcome::success(0, Durability::Strong);
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
/// builds, which clean their own. Callers scan first, so a partial's name
/// stays reserved for this hold's allocation (D4).
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
    read_bounded(path, archive::MAX_ARCHIVE_BYTES).map_err(|error| match error.kind() {
        ErrorKind::FileTooLarge => archive::Problem::TooLarge("archive"),
        kind => archive::Problem::Io(kind),
    })
}

/// A full verification of the zip at `path`, read positionally from the
/// file one entry at a time and keeping no payload (og-merge review M1:
/// the skip and every retention pass would otherwise hold the archive and
/// all of its payloads). Its manifest on success.
fn verify_zip(path: &std::path::Path) -> Result<archive::ZipManifest, archive::Problem> {
    let file = std::fs::File::open(path).map_err(|error| archive::Problem::Io(error.kind()))?;
    let len = file
        .metadata()
        .map_err(|error| archive::Problem::Io(error.kind()))?
        .len();
    archive::check(&mut archive::FileSource { file, len })
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
    // D4: every recognized name, partials included, is reserved before a
    // partial is cleaned.
    let scan = match scan(base, zips) {
        Ok(scan) => scan,
        Err(error) => return BackupOutcome::failed(n, "scan", error.kind()),
    };
    remove_partials(zips);
    // Item 4 + R5: a launch whose capture equals the newest state record of
    // any suffix writes nothing, but only once that record fully verifies
    // (B3); a damaged, missing or legacy baseline gets a fresh zip. The
    // baseline is settled again, as a publication would be (D2: no file
    // written, syncs of clean files only).
    let baseline = scan
        .records
        .last()
        .filter(|newest| suffix.is_empty() && newest.namespace == naming::Namespace::Zip)
        .filter(|newest| {
            verify_zip(&newest.path).is_ok_and(|manifest| manifest == capture.manifest)
        });
    let (path, durability) = match baseline {
        Some(newest) => {
            // Nothing is written: the retention pass below runs without it.
            drop(capture);
            crate::debug::diag_private("backup-unchanged", newest.name.render());
            match settle(&newest.path, zips, "baseline_sync") {
                Ok(durability) => (newest.path.clone(), durability),
                Err(error) => return BackupOutcome::failed(n, "publish-sync", error.kind()),
            }
        }
        None => {
            let Some(name) =
                naming::allocate(&tine_core::date::utc_backup_stamp(), suffix, &scan.names)
            else {
                return BackupOutcome::failed(n, "naming", ErrorKind::Other);
            };
            let manifest = archive::encode_manifest(&capture.manifest);
            drop(scan);
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
            drop(capture);
            match publish_zip(zips, &name, &bytes) {
                Ok(published) => published,
                Err((phase, error)) => return BackupOutcome::failed(n, phase, error.kind()),
            }
        }
    };
    match durability {
        // Published, visible and restorable, but without a platform
        // witness: nothing older may be retired on its strength.
        Durability::Weak => {
            crate::debug::diag_private("backup-weak-durability", path.display().to_string())
        }
        Durability::Strong => maintain_locked(base, zips, keep, Some(&path)),
    }
    BackupOutcome::success(n, durability)
}

/// Visible after the rename, settled after the supported sync recipe
/// (item 3, D1): a `create_new` partial, one file sync, a no-replace
/// rename, then the whole directory chain. Runs under the namespace lock,
/// so the no-replace fallback's check-then-rename has no cooperating racer.
/// A failure before the rename removes the partial (a crash leaves it for
/// the next lock holder); one after it leaves a visible zip and reports
/// `publish-sync`.
fn publish_zip(
    zips: &std::path::Path,
    name: &naming::SnapshotName,
    bytes: &[u8],
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
    let durability = sync_chain(zips).map_err(|error| ("publish-sync", error))?;
    Ok((path, durability))
}

/// One snapshot the keep count may retire or keep (D2).
struct Candidate {
    record: Record,
    /// Holds a retention slot once it verifies (not `-pre-restore`).
    counted: bool,
    /// Its listing already shows damage.
    damaged: bool,
}

/// The keep count, the one retention function every locked path runs
/// (D2: changed and unchanged launches, safety snapshots, Settings; item
/// 5, REVIEW B1, R5).
///
/// Counted: this build's legacy snapshots and zips except `-pre-restore`
/// (another Tine's, unsupported and unknown entries are never touched;
/// `-pre-restore` is never deleted, damaged or not). Listing is cheap and
/// decides whether anything could go: more counted snapshots than `keep`,
/// or a damaged one. Only then are snapshots fully verified, newest first,
/// until `keep` healthy counted ones and the keeper (the newest healthy
/// zip) are found: those hold the slots; every older counted snapshot goes,
/// and so does a damaged one older than the oldest retained zip. Nothing is
/// deleted until the keeper's file and directory chain are settled
/// (`fresh`: this hold just settled it strongly). A failed deletion is
/// logged and retried by the next locked path.
fn maintain_locked(
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
    let mut candidates = Vec::new();
    for record in scan.records.into_iter().rev() {
        let counted = !record.name.is_pre_restore();
        let damaged = match record.namespace {
            naming::Namespace::Legacy if !counted || is_foreign_snapshot(&record.path) => continue,
            naming::Namespace::Legacy => false,
            naming::Namespace::Zip => match zip_listing(&record.path) {
                // Another Tine's zip: never counted, never the keeper, never deleted.
                Ok(manifest) if manifest.writer != SNAPSHOT_WRITER => continue,
                Ok(_) => false,
                Err(archive::Problem::Damaged(_) | archive::Problem::TooLarge(_)) if counted => {
                    true
                }
                Err(archive::Problem::Damaged(_) | archive::Problem::TooLarge(_)) => continue,
                Err(problem) => {
                    crate::debug::diag_private(
                        "backup-unknown-entry",
                        format!("{}: {problem:?}", record.path.display()),
                    );
                    continue;
                }
            },
        };
        candidates.push(Candidate {
            record,
            counted,
            damaged,
        });
    }
    let listed_counted = candidates
        .iter()
        .filter(|candidate| candidate.counted && !candidate.damaged)
        .count();
    if listed_counted <= keep && !candidates.iter().any(|candidate| candidate.damaged) {
        return;
    }
    let mut retained = 0;
    let mut oldest_retained_zip: Option<&Record> = None;
    let mut keeper: Option<&Record> = None;
    let mut doomed = Vec::new();
    let mut damaged = Vec::new();
    for candidate in &candidates {
        let record = &candidate.record;
        if candidate.damaged {
            damaged.push(record);
            continue;
        }
        if retained >= keep && keeper.is_some() {
            if candidate.counted {
                doomed.push(record);
            }
            continue;
        }
        let zip = record.namespace == naming::Namespace::Zip;
        let healthy = match record.namespace {
            naming::Namespace::Zip => verify_zip(&record.path).is_ok(),
            naming::Namespace::Legacy => read_manifest(&record.path)
                .is_some_and(|manifest| verify_snapshot(&record.path, &manifest)),
        };
        if healthy && zip && keeper.is_none() {
            keeper = Some(record);
        }
        if !candidate.counted {
            continue;
        }
        if !healthy {
            damaged.push(record);
        } else if retained < keep {
            retained += 1;
            if zip {
                oldest_retained_zip = Some(record);
            }
        } else {
            doomed.push(record);
        }
    }
    if let Some(oldest) = oldest_retained_zip {
        doomed.extend(
            damaged
                .into_iter()
                .filter(|record| record.order(oldest).is_lt()),
        );
    }
    doomed.retain(|record| Some(*record) != keeper);
    if doomed.is_empty() {
        return;
    }
    let Some(keeper) = keeper else {
        crate::debug::diag_private("backup-prune-no-keeper", zips.display().to_string());
        return;
    };
    let certified = if fresh == Some(keeper.path.as_path()) {
        Ok(Durability::Strong)
    } else {
        settle(&keeper.path, zips, "keeper_sync")
    };
    match certified {
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
        Ok(Some(_lock)) => maintain_locked(base, &zips, keep, None),
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
    let Some(manifest) = read_bounded(&dir.join(SNAPSHOT_MANIFEST), archive::MAX_MANIFEST_BYTES)
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
