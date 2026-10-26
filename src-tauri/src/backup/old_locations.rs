//! On-demand discovery and removal of snapshots at earlier graph locations.
use super::*;

/// On demand only. An archive's recorded root, rather than a basename guess,
/// authorizes each old namespace; sealed bytes remain untouched.
pub(super) fn old_backup_roots(
    backups: &std::path::Path,
    root: &std::path::Path,
) -> Vec<(String, String)> {
    let current = root_backup_id(root);
    let label = current
        .rsplit_once('-')
        .map(|(label, _)| label)
        .unwrap_or("");
    let Ok(entries) = std::fs::read_dir(backups) else {
        return vec![];
    };
    let keys: std::collections::BTreeSet<_> = entries
        .flatten()
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            let key = name.strip_suffix(".zips").unwrap_or(&name);
            let (old_label, hash) = key.rsplit_once('-')?;
            (key != current
                && old_label == label
                && hash.len() == 32
                && hash.bytes().all(|c| c.is_ascii_hexdigit()))
            .then(|| key.to_owned())
        })
        .collect();
    keys.into_iter()
        .filter_map(|key| {
            let base = backups.join(&key);
            let scanned = scan(&base, &zip_base(&base)).ok()?;
            let latest = scanned.records.last()?;
            let old = match latest.namespace {
                naming::Namespace::Zip => zip_listing(&latest.path).ok()?.root,
                naming::Namespace::Legacy => read_manifest(&latest.path)?.root,
            };
            let old_path = std::path::Path::new(&old);
            (old_path.is_absolute()
                && root_backup_id(old_path) == key
                && Store::canonical_root(old_path).is_err())
            .then_some((key, old))
        })
        .collect()
}

/// Revalidate the selected old namespace at every destructive action. The
/// wire id is not a path and cannot nominate a live graph or another label.
pub(super) fn old_backup_selection<'a>(
    backups: &std::path::Path,
    root: &std::path::Path,
    id: &'a str,
) -> Result<(PathBuf, String, &'a str), String> {
    let rest = id.strip_prefix("old:").ok_or("invalid old backup id")?;
    let (key, snapshot) = rest.split_once(':').ok_or("invalid old backup id")?;
    let (_, old) = old_backup_roots(backups, root)
        .into_iter()
        .find(|(candidate, _)| candidate == key)
        .ok_or("old backup location is no longer available")?;
    Ok((backups.join(key), old, snapshot))
}

#[tauri::command]
pub(crate) async fn delete_old_backups(
    id: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let base = backup_base_for_root(&app, &slot.root_key).ok_or("no app-data dir")?;
        let (base, origin, _) = old_backup_selection(base.parent().unwrap(), &slot.root_key, &id)?;
        delete_old_snapshots(&base, &origin)
    })
    .await
}

pub(super) fn delete_old_snapshots(base: &std::path::Path, origin: &str) -> Result<(), String> {
    restore::under_namespace_lock(base, || {
        if Store::canonical_root(std::path::Path::new(&origin)).is_ok() {
            return Err("old backup location is available again; reopen Backups".into());
        }
        let scanned = scan(&base, &zip_base(&base)).map_err(|e| e.to_string())?;
        for record in scanned.records {
            match record.namespace {
                naming::Namespace::Zip => std::fs::remove_file(record.path),
                naming::Namespace::Legacy => std::fs::remove_dir_all(record.path),
            }
            .map_err(|e| e.to_string())?;
        }
        // Keep the lock inode: waiters may already have it open.
        crate::device_io::sync_private_directory(&zip_base(&base)).map_err(|e| e.to_string())?;
        if base.exists() {
            crate::device_io::sync_private_directory(&base).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}
