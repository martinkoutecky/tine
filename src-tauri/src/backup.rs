use crate::settings::{settings_path, update_settings};
use crate::state::AppState;
use std::path::PathBuf;
use tauri::{Manager, State};
use tine_core::model::atomic_copy;

// Snapshot the graph's markdown into the OS app-data dir on open, keeping the
// last few. Local-only (outside the graph, so Syncthing never sees it); a safety
// net against a bad write or accidental edit. Best-effort and fully detached so
// it never blocks startup or holds the graph lock during file copies.
const BACKUP_KEEP_DEFAULT: usize = 12;

pub(crate) fn backup_async(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Defer the launch snapshot ~1s so its whole-graph file copy doesn't
        // contend for disk I/O with first-journal paint and the warm-cache parse
        // at open (felt on slow/NFS disks or a throttled laptop). Safe: the
        // snapshot guards this session's edits, and the user hasn't edited yet in
        // the first second — the on-disk files are still intact — so a crash in
        // that window loses nothing the snapshot would have protected.
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let _ = do_backup(&app, ""); // launch snapshot is best-effort
    });
}

/// Take one snapshot of the current graph now (synchronous). Returns the number
/// of files copied (0 = nothing to back up). Reads the keep count from the local
/// app-settings file and prunes old snapshots afterwards. `suffix` tags special
/// snapshots (e.g. "pre-restore") so they get a distinct, collision-proof
/// directory name and are exempt from the keep-count prune.
/// Returns (files copied, complete) — `complete` is false if ANY graph
/// text/config/asset-sidecar copy failed, so the caller (restore) can refuse to
/// proceed without a full rollback snapshot.
fn do_backup(app: &tauri::AppHandle, suffix: &str) -> (usize, bool) {
    // Grab just the paths under the lock, then copy from disk lock-free.
    let (journals, pages, assets, cfg, root) = {
        let state: State<'_, AppState> = app.state();
        let guard = state.graph.read().unwrap();
        match guard.as_ref() {
            Some(g) => (
                g.journals_path(),
                g.pages_path(),
                g.assets_path(),
                g.root.join("logseq").join("config.edn"),
                g.root.clone(),
            ),
            None => return (0, false),
        }
    };
    let Ok(data_dir) = app.path().app_data_dir() else {
        return (0, false);
    };
    let base = data_dir
        .join("backups")
        .join(sanitize_id(&root.display().to_string()));
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
    let mut dest = base.join(&name);
    let mut k = 2;
    loop {
        match std::fs::create_dir(&dest) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                dest = base.join(format!("{name}-{k}"));
                k += 1;
            }
            Err(_) => break, // other error → fall through; copy below is best-effort
        }
    }
    let live_text_n = count_md_recursive(&journals) + count_md_recursive(&pages);
    let (cj, fj) = copy_md_dir(&journals, &dest.join(dir_name(&journals)));
    let (cp, fp) = copy_md_dir(&pages, &dest.join(dir_name(&pages)));
    let (ca, fa) = copy_asset_sidecars_dir(&assets, &dest.join(dir_name(&assets)));
    let mut n = cj + cp + ca;
    let mut failed = fj + fp + fa;
    if cfg.exists() {
        let out = dest.join("logseq");
        if std::fs::create_dir_all(&out).is_ok()
            && std::fs::copy(&cfg, out.join("config.edn")).is_ok()
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
pub(crate) fn set_backup_keep(keep: usize, app: tauri::AppHandle) -> Result<(), String> {
    let keep = keep.clamp(1, 1000);
    update_settings(&app, |json| {
        json["backup_keep"] = serde_json::json!(keep);
    })?;
    // Apply the new (possibly lower) cap to the current graph's snapshots now.
    if let Some(base) = backup_base(&app) {
        prune_backups(&base, keep);
    }
    Ok(())
}

/// The backup directory for the currently-open graph (`<app-data>/backups/<id>`).
fn backup_base(app: &tauri::AppHandle) -> Option<PathBuf> {
    let root = {
        let state: State<'_, AppState> = app.state();
        let guard = state.graph.read().unwrap();
        guard.as_ref().map(|g| g.root.clone())?
    };
    let data_dir = app.path().app_data_dir().ok()?;
    Some(
        data_dir
            .join("backups")
            .join(sanitize_id(&root.display().to_string())),
    )
}

#[tauri::command]
pub(crate) fn list_backups(app: tauri::AppHandle) -> Result<Vec<BackupInfo>, String> {
    let Some(base) = backup_base(&app) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let stamp = match p.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let files = count_md_recursive(&p) + count_asset_sidecars_recursive(&p.join("assets"));
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
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Guard against path traversal — a stamp is only ever `YYYY-MM-DD_HH-MM-SS`.
    if stamp.is_empty()
        || !stamp
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("invalid backup id".into());
    }
    let (journals, pages, assets, cfg_dest) = {
        let guard = state.graph.read().unwrap();
        let g = guard.as_ref().ok_or("no graph loaded")?;
        (
            g.journals_path(),
            g.pages_path(),
            g.assets_path(),
            g.root.join("logseq").join("config.edn"),
        )
    };
    let base = backup_base(&app).ok_or("no app-data dir")?;
    let src = base.join(&stamp);
    if !src.is_dir() {
        return Err("backup not found".into());
    }
    // Safety net: snapshot the current (pre-restore) state first, under a distinct
    // name so it can't collide with (or be pruned by) the launch snapshot the
    // post-restore reload will take. Abort if the snapshot fails while the live
    // graph has content — never run a destructive restore without a way back.
    let (snapshot_n, complete) = do_backup(&app, "pre-restore");
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
    // Restore each dir; a copy failure aborts WITHOUT having deleted anything
    // (copy-in happens before delete-extras), so a failure never loses data.
    restore_md_dir(&src.join(dir_name(&journals)), &journals)
        .map_err(|e| format!("restore journals failed: {e}"))?;
    restore_md_dir(&src.join(dir_name(&pages)), &pages)
        .map_err(|e| format!("restore pages failed: {e}"))?;
    restore_asset_sidecars_dir(&src.join(dir_name(&assets)), &assets)
        .map_err(|e| format!("restore asset sidecars failed: {e}"))?;
    let src_cfg = src.join("logseq").join("config.edn");
    if src_cfg.exists() {
        if let Some(parent) = cfg_dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        atomic_copy(&src_cfg, &cfg_dest).map_err(|e| format!("restore config failed: {e}"))?;
    }
    Ok(())
}

/// Restore graph text files in `dest` from `src`. Each file is copied through
/// the shared atomic helper, so a failure or power-loss mid-copy can never leave
/// a live note truncated/half-written. Copies happen FIRST; only after they all
/// succeed do we delete `dest` graph text files not in the backup. A copy error
/// returns early leaving a superset of files (no data lost). Other files are left
/// untouched.
fn restore_md_dir(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dest)?;
    let mut restored: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    restore_md_copy(src, dest, std::path::Path::new(""), &mut restored)?;
    delete_unrestored_md(dest, std::path::Path::new(""), &restored)?;
    Ok(())
}

fn restore_md_copy(
    src: &std::path::Path,
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &mut std::collections::HashSet<std::path::PathBuf>,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        let rel_child = rel.join(e.file_name());
        if is_graph_text(&p) {
            let target = dest.join(&rel_child);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            atomic_copy(&p, &target)?;
            restored.insert(rel_child);
        } else if is_visible_real_dir(&e)? {
            std::fs::create_dir_all(dest.join(&rel_child))?;
            restore_md_copy(&p, dest, &rel_child, restored)?;
        }
    }
    Ok(())
}

fn delete_unrestored_md(
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &std::collections::HashSet<std::path::PathBuf>,
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
                let _ = std::fs::remove_file(&p);
            }
        } else if is_visible_real_dir(&e)? {
            delete_unrestored_md(dest, &rel_child, restored)?;
        }
    }
    Ok(())
}

fn restore_asset_sidecars_dir(
    src: &std::path::Path,
    dest: &std::path::Path,
) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dest)?;
    let mut restored: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    restore_asset_sidecars_copy(src, dest, std::path::Path::new(""), &mut restored)?;
    delete_unrestored_asset_sidecars(dest, std::path::Path::new(""), &restored)?;
    Ok(())
}

fn restore_asset_sidecars_copy(
    src: &std::path::Path,
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &mut std::collections::HashSet<std::path::PathBuf>,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let ft = e.file_type()?;
        let rel_child = rel.join(e.file_name());
        let p = e.path();
        if ft.is_dir() {
            std::fs::create_dir_all(dest.join(&rel_child))?;
            restore_asset_sidecars_copy(&p, dest, &rel_child, restored)?;
        } else if ft.is_file() && is_asset_sidecar(&p) {
            let target = dest.join(&rel_child);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            atomic_copy(&p, &target)?;
            restored.insert(rel_child);
        }
    }
    Ok(())
}

fn delete_unrestored_asset_sidecars(
    dest: &std::path::Path,
    rel: &std::path::Path,
    restored: &std::collections::HashSet<std::path::PathBuf>,
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
        if ft.is_dir() {
            delete_unrestored_asset_sidecars(dest, &rel_child, restored)?;
        } else if ft.is_file() && is_asset_sidecar(&p) && !restored.contains(&rel_child) {
            let _ = std::fs::remove_file(&p);
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
fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
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
        if ft.is_dir() {
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
    fn copy_asset_sidecars_dir_copies_only_edn_recursively() {
        let root = scratch("copy-sidecars");
        let src = root.join("assets");
        let dst = root.join("backup").join("assets");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("doc.edn"), "{:a 1}\n").unwrap();
        std::fs::write(src.join("nested").join("hl.edn"), "{:b 2}\n").unwrap();
        std::fs::write(src.join("image.png"), b"png").unwrap();
        std::fs::write(src.join("nested").join("image.png"), b"png").unwrap();

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

        restore_asset_sidecars_dir(&src, &dest).unwrap();
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
        restore_md_dir(&backup.join("pages"), &pages).unwrap();
        assert_eq!(
            std::fs::read(pages.join("client-a").join("Deep.md")).unwrap(),
            b"deep\n"
        );
        assert!(!pages.join("client-a").join("Stale.md").exists());
        assert_eq!(
            std::fs::read(pages.join("client-a").join("notes.txt")).unwrap(),
            b"keep\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
