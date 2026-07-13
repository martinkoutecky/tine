use crate::settings::{settings_path, update_settings};
use crate::state::{slot_for_context, GraphContext};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;
use tauri::Manager;
use tine_core::model::{atomic_copy, atomic_copy_new, Graph};

// Snapshot the graph's markdown into the OS app-data dir on open, keeping the
// last few. Local-only (outside the graph, so Syncthing never sees it); a safety
// net against a bad write or accidental edit. Best-effort and fully detached so
// it never blocks startup or holds the graph lock during file copies.
const BACKUP_KEEP_DEFAULT: usize = 12;
const ASSET_RESTORE_RECOVERY_DIR: &str = ".tine-restore-recovery";

pub(crate) fn backup_async(app: tauri::AppHandle, graph: &Graph) {
    let source = BackupSource::from_graph(graph);
    std::thread::spawn(move || {
        // Defer the launch snapshot ~1s so its whole-graph file copy doesn't
        // contend for disk I/O with first-journal paint and the warm-cache parse
        // at open (felt on slow/NFS disks or a throttled laptop). Safe: the
        // snapshot guards this session's edits, and the user hasn't edited yet in
        // the first second — the on-disk files are still intact — so a crash in
        // that window loses nothing the snapshot would have protected.
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let _ = do_backup_source(&app, source, ""); // launch snapshot is best-effort
    });
}

pub(crate) fn backup_graph_now(
    app: &tauri::AppHandle,
    graph: &Graph,
    suffix: &str,
) -> (usize, bool) {
    do_backup_source(app, BackupSource::from_graph(graph), suffix)
}

/// Take one snapshot of the current graph now (synchronous). Returns the number
/// of files copied (0 = nothing to back up). Reads the keep count from the local
/// app-settings file and prunes old snapshots afterwards. `suffix` tags special
/// snapshots (e.g. "pre-restore") so they get a distinct, collision-proof
/// directory name and are exempt from the keep-count prune.
/// Returns (files copied, complete) — `complete` is false if ANY graph
/// text/config/asset-sidecar copy failed, so the caller (restore) can refuse to
/// proceed without a full rollback snapshot.
struct BackupSource {
    journals: PathBuf,
    pages: PathBuf,
    assets: PathBuf,
    cfg: PathBuf,
    root: PathBuf,
    journals_dir: String,
    pages_dir: String,
}

impl BackupSource {
    fn from_graph(g: &Graph) -> Self {
        Self {
            journals: g.journals_path(),
            pages: g.pages_path(),
            assets: g.assets_path(),
            cfg: g.root.join("logseq").join("config.edn"),
            root: g.root.clone(),
            journals_dir: g.config.journals_dir.clone(),
            pages_dir: g.config.pages_dir.clone(),
        }
    }
}

const SNAPSHOT_SCHEMA: u32 = 2;
const SNAPSHOT_MANIFEST: &str = "snapshot.json";

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
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
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
    drop(file);
    std::fs::rename(tmp, path)
}

fn read_manifest(dir: &std::path::Path) -> Option<SnapshotManifest> {
    let bytes = std::fs::read(dir.join(SNAPSHOT_MANIFEST)).ok()?;
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes).ok()?;
    (manifest.schema == SNAPSHOT_SCHEMA && manifest.complete).then_some(manifest)
}

fn hash_snapshot_file(path: &std::path::Path) -> std::io::Result<String> {
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

fn do_backup_source(app: &tauri::AppHandle, source: BackupSource, suffix: &str) -> (usize, bool) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return (0, false);
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
    let _ = std::fs::create_dir_all(&base);
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
            Err(_) => break, // other error → fall through; copy below is best-effort
        }
    }
    let live_text_n = count_md_recursive(&source.journals) + count_md_recursive(&source.pages);
    let (cj, fj) = copy_md_dir(&source.journals, &dest.join("journals"));
    let (cp, fp) = copy_md_dir(&source.pages, &dest.join("pages"));
    let (ca, fa) = copy_asset_sidecars_dir(&source.assets, &dest.join(dir_name(&source.assets)));
    let mut n = cj + cp + ca;
    let mut failed = fj + fp + fa;
    if source.cfg.exists() {
        let out = dest.join("logseq");
        if std::fs::create_dir_all(&out).is_ok()
            && std::fs::copy(&source.cfg, out.join("config.edn")).is_ok()
        {
            n += 1;
        } else {
            failed += 1;
        }
    }
    let complete = failed == 0 && cj + cp == live_text_n;
    if n == 0 {
        let _ = std::fs::remove_dir_all(&dest);
        return (0, complete);
    }
    if complete {
        let Ok(files) = snapshot_inventory(&dest) else {
            return (n, false);
        };
        if files.len() != n {
            return (n, false);
        }
        let manifest = SnapshotManifest {
            schema: SNAPSHOT_SCHEMA,
            root: std::fs::canonicalize(&source.root)
                .unwrap_or(source.root.clone())
                .display()
                .to_string(),
            journals_dir: source.journals_dir,
            pages_dir: source.pages_dir,
            files,
            complete: true,
        };
        if write_manifest(&dest, &manifest).is_err() || std::fs::rename(&dest, &final_dest).is_err()
        {
            return (n, false);
        }
    }
    prune_backups(&base, backup_keep(app));
    (n, complete)
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
    if let Some(base) = backup_base(&app, &slot.graph) {
        prune_backups(&base, keep);
    }
    Ok(())
}

/// The backup directory for the currently-open graph (`<app-data>/backups/<id>`).
fn backup_base(app: &tauri::AppHandle, graph: &Graph) -> Option<PathBuf> {
    let root = graph.root.clone();
    let data_dir = app.path().app_data_dir().ok()?;
    Some(data_dir.join("backups").join(root_backup_id(&root)))
}

#[tauri::command]
pub(crate) fn list_backups(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<Vec<BackupInfo>, String> {
    let slot = slot_for_context(&state)?;
    let Some(base) = backup_base(&app, &slot.graph) else {
        return Ok(Vec::new());
    };
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
            let current_root = std::fs::canonicalize(&slot.graph.root)
                .unwrap_or_else(|_| slot.graph.root.clone())
                .display()
                .to_string();
            if manifest.root != current_root || !verify_snapshot(&p, &manifest) {
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
    Ok(out)
}

fn count_md_recursive(dir: &std::path::Path) -> usize {
    let mut n = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if is_graph_text(&p) {
                    n += 1;
                } else if is_visible_real_dir(&e).unwrap_or(false) {
                    stack.push(p);
                }
            }
        }
    }
    n
}

fn count_asset_sidecars_recursive(dir: &std::path::Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += count_asset_sidecars_recursive(&p);
            } else if is_asset_sidecar(&p) {
                n += 1;
            }
        }
    }
    n
}

/// Restore a snapshot into the live graph, overwriting `journals/`, `pages/`,
/// asset `.edn` sidecars, and `config.edn`. Takes a fresh safety snapshot of the
/// *current* state first (so a mistaken restore is itself reversible).
/// Destructive — the frontend confirms.
#[tauri::command]
pub(crate) fn restore_backup(
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
    let (journals, pages, assets, cfg_dest, source) = {
        let g = &slot.graph;
        (
            g.journals_path(),
            g.pages_path(),
            g.assets_path(),
            g.root.join("logseq").join("config.edn"),
            BackupSource::from_graph(g),
        )
    };
    let base = backup_base(&app, &slot.graph).ok_or("no app-data dir")?;
    let src = base.join(&stamp);
    if !src.is_dir() {
        return Err("backup not found".into());
    }
    let manifest = read_manifest(&src).ok_or("backup is incomplete or unverified")?;
    let current_root =
        std::fs::canonicalize(&slot.graph.root).unwrap_or_else(|_| slot.graph.root.clone());
    if manifest.root != current_root.display().to_string() {
        return Err("backup belongs to a different graph".into());
    }
    if !verify_snapshot(&src, &manifest) {
        return Err("backup contents do not match the verified manifest".into());
    }
    let safe_dir = |raw: &str| -> Result<PathBuf, String> {
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
        Ok(current_root.join(rel))
    };
    let restore_journals = safe_dir(&manifest.journals_dir)?;
    let restore_pages = safe_dir(&manifest.pages_dir)?;
    let validate_live_layout = || -> Result<(), String> {
        for (label, path) in [
            ("journals", &restore_journals),
            ("pages", &restore_pages),
            ("config", &cfg_dest),
        ] {
            ensure_target_within_root(&current_root, path)
                .map_err(|e| format!("unsafe live {label} path: {e}"))?;
        }
        // Assets have a separate, explicitly-approved capability and therefore
        // validate against their own canonical root. For ordinary graphs this is
        // still `<graph>/assets`; for GH #127 it is the approved external target.
        ensure_target_within_root(&assets, &assets)
            .map_err(|e| format!("unsafe live assets path: {e}"))?;
        Ok(())
    };
    validate_live_layout()?;
    let recovery_id = format!(
        "{}-pre-restore-extras-{}-{}",
        backup_stamp(),
        std::process::id(),
        RESTORE_RECOVERY_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let (graph_recovery, asset_recovery) =
        restore_recovery_roots(&current_root, &assets, &recovery_id);
    std::fs::create_dir_all(&graph_recovery)
        .map_err(|e| format!("couldn't create restore recovery area: {e}"))?;
    std::fs::create_dir_all(&asset_recovery)
        .map_err(|e| format!("couldn't create asset restore recovery area: {e}"))?;
    // Safety net: snapshot the current (pre-restore) state first, under a distinct
    // name so it can't collide with (or be pruned by) the launch snapshot the
    // post-restore reload will take. Abort if the snapshot fails while the live
    // graph has content — never run a destructive restore without a way back.
    let (snapshot_n, complete) = do_backup_source(&app, source, "pre-restore");
    let live_n = count_md_recursive(&journals)
        + count_md_recursive(&pages)
        + count_asset_sidecars_recursive(&assets);
    // A destructive restore must be fully reversible: abort unless the pre-restore
    // snapshot captured everything (every file copied, nothing skipped).
    if live_n > 0 && (snapshot_n == 0 || !complete) {
        return Err(
            "couldn't create a complete pre-restore safety snapshot — restore aborted".into(),
        );
    }
    // The safety snapshot can take time. Revalidate after it so a symlink swap
    // cannot redirect the destructive copy/delete phase outside the graph.
    validate_live_layout()?;
    // Restore each dir; copies happen before extras are moved to the dedicated
    // recovery area, so a failure leaves either the original or a recoverable copy.
    restore_md_dir(
        &src.join("journals"),
        &restore_journals,
        &graph_recovery.join("journals"),
        &current_root,
    )
        .map_err(|e| format!("restore journals failed: {e}"))?;
    restore_md_dir(
        &src.join("pages"),
        &restore_pages,
        &graph_recovery.join("pages"),
        &current_root,
    )
        .map_err(|e| format!("restore pages failed: {e}"))?;
    restore_asset_sidecars_dir(
        &src.join(dir_name(&assets)),
        &assets,
        &asset_recovery,
        &assets,
    )
        .map_err(|e| format!("restore asset sidecars failed: {e}"))?;
    let src_cfg = src.join("logseq").join("config.edn");
    if src_cfg.exists() {
        if let Some(parent) = cfg_dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if cfg_dest.exists() {
            move_live_to_recovery(&cfg_dest, &graph_recovery.join("logseq/config.edn"))
                .map_err(|e| format!("recover current config failed: {e}"))?;
        }
        atomic_copy_new(&src_cfg, &cfg_dest)
            .map_err(|e| format!("restore config failed: {e}"))?;
    }
    crate::state::refresh_graph(&state)?;
    Ok(())
}

fn restore_recovery_roots(
    graph_root: &std::path::Path,
    assets_root: &std::path::Path,
    recovery_id: &str,
) -> (PathBuf, PathBuf) {
    // `rename` is the data-safety primitive below: an external writer holding an
    // open handle follows the detached inode instead of recreating/truncating the
    // just-restored live name. Android commonly places app-data and a selected
    // graph on different filesystems, so an app-data recovery tree makes every
    // rename fail with EXDEV. Keep each recovery tree inside the capability it
    // protects, which guarantees same-filesystem detachment without widening
    // filesystem access. Asset recovery is hidden from snapshot/restore scans.
    let graph = graph_root
        .join("logseq")
        .join(".tine-trash")
        .join(recovery_id);
    let assets = assets_root
        .join(ASSET_RESTORE_RECOVERY_DIR)
        .join(recovery_id);
    (graph, assets)
}

fn ensure_target_within_root(root: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    let canonical_root = std::fs::canonicalize(root)?;
    let mut existing = target;
    while std::fs::symlink_metadata(existing).is_err() {
        existing = existing.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no existing ancestor")
        })?;
    }
    let canonical_existing = std::fs::canonicalize(existing)?;
    let expected = existing
        .strip_prefix(root)
        .map(|rel| canonical_root.join(rel))
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "target is outside graph root")
        })?;
    if canonical_existing == expected {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target escapes graph root",
        ))
    }
}

/// Atomically detach the current live name into the restore recovery tree. A
/// writer with an open handle continues writing the recovered inode; a writer
/// that recreates the live name is left untouched. The recovery roots are kept
/// on the live graph/assets filesystems; if an unexpected nested mount still
/// makes `rename` cross-device, preserve a copy but abort the restore without
/// removing the live file rather than risk a copy-then-delete race.
fn move_live_to_recovery(
    live: &std::path::Path,
    recovery: &std::path::Path,
) -> std::io::Result<()> {
    if let Some(parent) = recovery.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(live, recovery) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            atomic_copy(live, recovery)?;
            Err(std::io::Error::new(
                rename_err.kind(),
                format!(
                    "live file copied to recovery but could not be atomically detached: {rename_err}"
                ),
            ))
        }
    }
}

/// Restore graph text files in `dest` from `src`. Each file is copied through
/// the shared atomic helper, so a failure or power-loss mid-copy can never leave
/// a live note truncated/half-written. Copies happen FIRST; only after they all
/// succeed do we move `dest` graph text files not in the backup to a recovery
/// area. A copy error returns early leaving a superset of files. Other files are
/// left untouched.
static RESTORE_RECOVERY_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn restore_md_dir(
    src: &std::path::Path,
    dest: &std::path::Path,
    recovery: &std::path::Path,
    graph_root: &std::path::Path,
) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    ensure_target_within_root(graph_root, dest)?;
    std::fs::create_dir_all(dest)?;
    let mut restored: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    restore_md_copy(
        src,
        dest,
        std::path::Path::new(""),
        &mut restored,
        recovery,
        graph_root,
    )?;
    delete_unrestored_md(
        dest,
        std::path::Path::new(""),
        &restored,
        recovery,
        graph_root,
    )?;
    Ok(())
}

fn restore_md_copy(
    src: &std::path::Path,
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &mut std::collections::HashSet<std::path::PathBuf>,
    recovery: &std::path::Path,
    graph_root: &std::path::Path,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        let rel_child = rel.join(e.file_name());
        if is_graph_text(&p) {
            let target = dest.join(&rel_child);
            ensure_target_within_root(graph_root, &target)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if target.exists() {
                move_live_to_recovery(&target, &recovery.join(&rel_child))?;
            }
            atomic_copy_new(&p, &target)?;
            restored.insert(rel_child);
        } else if is_visible_real_dir(&e)? {
            ensure_target_within_root(graph_root, &dest.join(&rel_child))?;
            std::fs::create_dir_all(dest.join(&rel_child))?;
            restore_md_copy(&p, dest, &rel_child, restored, recovery, graph_root)?;
        }
    }
    Ok(())
}

fn delete_unrestored_md(
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &std::collections::HashSet<std::path::PathBuf>,
    recovery: &std::path::Path,
    graph_root: &std::path::Path,
) -> std::io::Result<()> {
    let dir = dest.join(rel);
    if !dir.is_dir() {
        return Ok(());
    }
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        let p = e.path();
        let rel_child = rel.join(e.file_name());
        if is_graph_text(&p) {
            if !restored.contains(&rel_child) {
                ensure_target_within_root(graph_root, &p)?;
                move_live_to_recovery(&p, &recovery.join(&rel_child))?;
            }
        } else if is_visible_real_dir(&e)? {
            delete_unrestored_md(dest, &rel_child, restored, recovery, graph_root)?;
        }
    }
    Ok(())
}

fn restore_asset_sidecars_dir(
    src: &std::path::Path,
    dest: &std::path::Path,
    recovery: &std::path::Path,
    graph_root: &std::path::Path,
) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    ensure_target_within_root(graph_root, dest)?;
    std::fs::create_dir_all(dest)?;
    let mut restored: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    restore_asset_sidecars_copy(
        src,
        dest,
        std::path::Path::new(""),
        &mut restored,
        recovery,
        graph_root,
    )?;
    delete_unrestored_asset_sidecars(
        dest,
        std::path::Path::new(""),
        &restored,
        recovery,
        graph_root,
    )?;
    Ok(())
}

fn restore_asset_sidecars_copy(
    src: &std::path::Path,
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &mut std::collections::HashSet<std::path::PathBuf>,
    recovery: &std::path::Path,
    graph_root: &std::path::Path,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let ft = e.file_type()?;
        let rel_child = rel.join(e.file_name());
        let p = e.path();
        if ft.is_dir() && !is_asset_restore_recovery_entry(&e) {
            ensure_target_within_root(graph_root, &dest.join(&rel_child))?;
            std::fs::create_dir_all(dest.join(&rel_child))?;
            restore_asset_sidecars_copy(&p, dest, &rel_child, restored, recovery, graph_root)?;
        } else if ft.is_file() && is_asset_sidecar(&p) {
            let target = dest.join(&rel_child);
            ensure_target_within_root(graph_root, &target)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if target.exists() {
                move_live_to_recovery(&target, &recovery.join(&rel_child))?;
            }
            atomic_copy_new(&p, &target)?;
            restored.insert(rel_child);
        }
    }
    Ok(())
}

fn delete_unrestored_asset_sidecars(
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &std::collections::HashSet<std::path::PathBuf>,
    recovery: &std::path::Path,
    graph_root: &std::path::Path,
) -> std::io::Result<()> {
    let dir = dest.join(rel);
    if !dir.is_dir() {
        return Ok(());
    }
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        let ft = e.file_type()?;
        let rel_child = rel.join(e.file_name());
        let p = e.path();
        if ft.is_dir() && !is_asset_restore_recovery_entry(&e) {
            delete_unrestored_asset_sidecars(dest, &rel_child, restored, recovery, graph_root)?;
        } else if ft.is_file() && is_asset_sidecar(&p) && !restored.contains(&rel_child) {
            ensure_target_within_root(graph_root, &p)?;
            move_live_to_recovery(&p, &recovery.join(&rel_child))?;
        }
    }
    Ok(())
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

fn is_asset_restore_recovery_entry(e: &std::fs::DirEntry) -> bool {
    e.file_name() == std::ffi::OsStr::new(ASSET_RESTORE_RECOVERY_DIR)
}

fn is_visible_real_dir(e: &std::fs::DirEntry) -> std::io::Result<bool> {
    let hidden = e
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(true);
    if hidden {
        return Ok(false);
    }
    e.file_type().map(|ft| ft.is_dir())
}

fn dir_name(p: &std::path::Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("dir")
        .to_string()
}
/// Copy every graph text file from `src` to `dest`. Returns (copied, failed) so
/// the caller can tell a complete snapshot from a partial one.
fn copy_md_dir(src: &std::path::Path, dest: &std::path::Path) -> (usize, usize) {
    // Materialize the dest dir up front, even when src has no .md files — so the
    // snapshot records "this dir existed and was empty". Otherwise restore can't
    // tell an empty-at-backup dir from a missing one, and leaves destination .md
    // extras in place (mixing current files into the restored snapshot).
    let _ = std::fs::create_dir_all(dest);
    match std::fs::read_dir(src) {
        Ok(_) => {}
        // A genuinely-absent source dir (e.g. a graph with no pages/) is not a
        // failure — there's nothing to snapshot. But a dir we CAN'T read
        // (permission / I/O) MUST count as failed, so a pre-restore safety
        // snapshot isn't falsely reported complete and a destructive restore can
        // refuse to proceed without a trustworthy rollback.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (0, 0),
        Err(_) => return (0, 1),
    }
    let (mut copied, mut failed) = (0usize, 0usize);
    let mut stack = vec![(src.to_path_buf(), std::path::PathBuf::new())];
    while let Some((dir, rel)) = stack.pop() {
        let target_dir = dest.join(&rel);
        let _ = std::fs::create_dir_all(&target_dir);
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                failed += 1;
                continue;
            }
        };
        for entry in rd {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };
            let p = entry.path();
            let rel_child = rel.join(entry.file_name());
            if is_graph_text(&p) {
                let target = dest.join(&rel_child);
                let copied_ok = target
                    .parent()
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .is_ok()
                    && std::fs::copy(&p, &target).is_ok();
                if copied_ok {
                    copied += 1;
                } else {
                    failed += 1;
                }
            } else {
                match is_visible_real_dir(&entry) {
                    Ok(true) => stack.push((p, rel_child)),
                    Ok(false) => {}
                    Err(_) => failed += 1,
                }
            }
        }
    }
    (copied, failed)
}

fn copy_asset_sidecars_dir(src: &std::path::Path, dest: &std::path::Path) -> (usize, usize) {
    let _ = std::fs::create_dir_all(dest);
    let rd = match std::fs::read_dir(src) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (0, 0),
        Err(_) => return (0, 1),
    };
    let (mut copied, mut failed) = (0usize, 0usize);
    for entry in rd {
        let Ok(entry) = entry else {
            failed += 1;
            continue;
        };
        let p = entry.path();
        let Ok(ft) = entry.file_type() else {
            failed += 1;
            continue;
        };
        let target = dest.join(entry.file_name());
        if ft.is_dir() && !is_asset_restore_recovery_entry(&entry) {
            let (c, f) = copy_asset_sidecars_dir(&p, &target);
            copied += c;
            failed += f;
        } else if ft.is_file() && is_asset_sidecar(&p) {
            if std::fs::create_dir_all(dest).is_ok() && std::fs::copy(&p, &target).is_ok() {
                copied += 1;
            } else {
                failed += 1;
            }
        }
    }
    (copied, failed)
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
    fn restore_recovery_roots_live_on_the_filesystems_they_detach_from() {
        let root = scratch("restore-recovery-roots");
        let graph = root.join("mounted-graph");
        let assets = root.join("mounted-assets");
        let (graph_recovery, asset_recovery) =
            restore_recovery_roots(&graph, &assets, "restore-1");

        assert!(graph_recovery.starts_with(graph.join("logseq/.tine-trash")));
        assert!(asset_recovery.starts_with(assets.join(".tine-restore-recovery")));
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
    fn copy_asset_sidecars_dir_copies_only_edn_recursively() {
        let root = scratch("copy-sidecars");
        let src = root.join("assets");
        let dst = root.join("backup").join("assets");
        std::fs::create_dir_all(src.join("nested")).unwrap();
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

        assert_eq!(copy_asset_sidecars_dir(&src, &dst), (2, 0));
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
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_asset_sidecars_dir_restores_sidecars_and_leaves_binary_assets() {
        let root = scratch("restore-sidecars");
        let src = root.join("backup").join("assets");
        let dest = root.join("graph").join("assets");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::create_dir_all(dest.join("nested")).unwrap();
        std::fs::write(src.join("doc.edn"), "new\n").unwrap();
        std::fs::write(src.join("nested").join("hl.edn"), "nested new\n").unwrap();
        std::fs::write(dest.join("doc.edn"), "old\n").unwrap();
        std::fs::write(dest.join("stale.edn"), "stale\n").unwrap();
        std::fs::write(dest.join("image.png"), b"keep").unwrap();
        std::fs::write(dest.join("nested").join("stale.edn"), "stale\n").unwrap();
        std::fs::write(dest.join("nested").join("image.png"), b"keep").unwrap();

        let recovery = dest
            .join(ASSET_RESTORE_RECOVERY_DIR)
            .join("restore-sidecars");
        restore_asset_sidecars_dir(&src, &dest, &recovery, &root).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("doc.edn")).unwrap(),
            "new\n"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("nested").join("hl.edn")).unwrap(),
            "nested new\n"
        );
        assert!(!dest.join("stale.edn").exists());
        assert!(!dest.join("nested").join("stale.edn").exists());
        assert_eq!(
            std::fs::read_to_string(recovery.join("stale.edn")).unwrap(),
            "stale\n"
        );
        assert_eq!(std::fs::read(dest.join("image.png")).unwrap(), b"keep");
        assert_eq!(
            std::fs::read(dest.join("nested").join("image.png")).unwrap(),
            b"keep"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn graph_text_backup_and_restore_include_nested_pages() {
        let root = scratch("nested-md");
        let graph = root.join("graph");
        let pages = graph.join("pages");
        let journals = graph.join("journals");
        let backup = root.join("backup");
        std::fs::create_dir_all(pages.join("client-a")).unwrap();
        std::fs::create_dir_all(&journals).unwrap();
        std::fs::write(pages.join("Top.md"), b"top\n").unwrap();
        std::fs::write(pages.join("client-a").join("Deep.md"), b"deep\n").unwrap();
        std::fs::write(journals.join("2026_07_09.md"), b"journal\n").unwrap();

        let live_pages = count_md_recursive(&pages);
        let live_journals = count_md_recursive(&journals);
        let (copied_pages, failed_pages) = copy_md_dir(&pages, &backup.join("pages"));
        let (copied_journals, failed_journals) = copy_md_dir(&journals, &backup.join("journals"));
        let copied = copied_pages + copied_journals;
        let failed = failed_pages + failed_journals;
        let complete = failed == 0 && copied == live_pages + live_journals;

        assert_eq!(live_pages, 2);
        assert_eq!(live_journals, 1);
        assert!(complete);
        assert_eq!(
            std::fs::read(backup.join("pages").join("Top.md")).unwrap(),
            b"top\n"
        );
        assert_eq!(
            std::fs::read(backup.join("pages").join("client-a").join("Deep.md")).unwrap(),
            b"deep\n"
        );
        assert_eq!(
            std::fs::read(backup.join("journals").join("2026_07_09.md")).unwrap(),
            b"journal\n"
        );

        std::fs::write(pages.join("client-a").join("Deep.md"), b"corrupt\n").unwrap();
        std::fs::write(pages.join("client-a").join("Stale.md"), b"stale\n").unwrap();
        std::fs::write(pages.join("client-a").join("notes.txt"), b"keep\n").unwrap();
        let recovery = root
            .join("graph")
            .join("logseq")
            .join(".tine-trash")
            .join("restore-pages");
        restore_md_dir(&backup.join("pages"), &pages, &recovery, &root).unwrap();
        assert_eq!(
            std::fs::read(pages.join("client-a").join("Deep.md")).unwrap(),
            b"deep\n"
        );
        assert!(!pages.join("client-a").join("Stale.md").exists());
        assert_eq!(
            std::fs::read_to_string(recovery.join("client-a").join("Stale.md")).unwrap(),
            "stale\n"
        );
        assert_eq!(
            std::fs::read(pages.join("client-a").join("notes.txt")).unwrap(),
            b"keep\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
