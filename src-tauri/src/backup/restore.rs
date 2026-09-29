//! Restore a verified snapshot into the live graph (split from `backup.rs`
//! along the snapshot/restore seam). Creating, listing and pruning snapshots
//! stays in the parent module; this module only reads a published snapshot,
//! checks it against its manifest, and hands the verified files to
//! `Store::restore`, the graph's one guarded restore write path.

use super::*;

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
                        Area::Assets => is_asset_sidecar(&entry.id),
                        _ => is_graph_text(&entry.id),
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
        .restore(tine_store::EditKind::ReplacePage, files)
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
                Area::Journals | Area::Pages => {
                    is_graph_text(&tine_store::FileId::from(format!("{prefix}/{rel}")))
                }
                Area::Assets => {
                    is_asset_sidecar(&tine_store::FileId::from(format!("{prefix}/{rel}")))
                }
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
    fn restore_wire_error_names_recovery_paths_and_kind() {
        let root = scratch("restore-wire-error");
        let store = Store::open(&root, Default::default()).unwrap().0;
        let recovery = root.join("logseq/.tine-trash/restore-1");
        let error = tine_store::RestoreFailed {
            phase: "copy pages".into(),
            cause: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "copy failed").into(),
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

    #[cfg(unix)]
    #[test]
    fn uncapturable_page_names_fail_the_safety_snapshot_before_restore() {
        use std::os::unix::ffi::OsStringExt;
        let root = scratch("backup-uncapturable-names");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        let non_utf = std::ffi::OsString::from_vec(b"lost-\xff.md".to_vec());
        let non_utf_path = root.join("pages").join(non_utf);
        let invalid_id_path = root.join("pages/invalid\\name.md");
        let invalid_dir_path = root.join("pages/invalid\\directory");
        std::fs::write(&non_utf_path, b"- keep A\n").unwrap();
        std::fs::write(&invalid_id_path, b"- keep B\n").unwrap();
        std::fs::create_dir_all(&invalid_dir_path).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let (copied, failed, failure) = copy_store_area(
            &store,
            Area::Pages,
            &root.join("backup-out"),
            is_graph_text,
            &|| false,
        );
        assert_eq!((copied, failed), (0, 3));
        assert!(require_safety_snapshot(BackupOutcome { copied, failure }, 2).is_err());
        assert_eq!(std::fs::read(&non_utf_path).unwrap(), b"- keep A\n");
        assert_eq!(std::fs::read(&invalid_id_path).unwrap(), b"- keep B\n");
        store.close();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
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
}
