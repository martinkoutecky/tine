//! Restore a verified snapshot into the live graph (split from `backup.rs`
//! along the snapshot/restore seam). Creating, listing and pruning snapshots
//! stays in the parent module; this module only reads a published snapshot,
//! checks it against its manifest, and hands the verified files to
//! `Store::restore`, the graph's one guarded restore write path.

use super::*;

/// Restore a snapshot into the live graph. Schema 3 puts graph text back at
/// its graph-relative path across the whole graph; schema 2 keeps the old
/// configured-root behaviour. Both restore asset `.edn` sidecars and
/// `config.edn`. Takes a fresh safety snapshot of the *current* state first
/// (so a mistaken restore is itself reversible).
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
        .map_err(|(kind, message)| format!("backup-failed:source:{kind:?}: {message}"))?;
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
    // Schema 3 text sits at its graph-relative path, so the recorded pages and
    // journals directories do not steer it (master 336833b13); `hidden` is
    // the graph-text scope it covered. `[""]` hides all: master's fail-closed
    // `:hidden` snapshot holds no text, so it must retire none.
    let scope = match (&manifest.graph_text_policy, manifest.schema) {
        (Some(policy), SNAPSHOT_SCHEMA) if policy.hidden_parse_failed_closed => {
            Some(vec![String::new()])
        }
        (Some(policy), SNAPSHOT_SCHEMA) => Some(policy.hidden.clone()),
        _ => None,
    };
    let areas = if scope.is_some() {
        vec!["graph", source.assets_dir_name.as_str()]
    } else {
        safe_dir(&manifest.journals_dir)?;
        safe_dir(&manifest.pages_dir)?;
        // A schema-2 restore targets the open graph's current layout; a backup
        // taken under a different :pages-directory / :journals-directory is not
        // restored (v0.6.5 wrote into the backup's old directory names).
        if manifest.journals_dir != source.journals_dir || manifest.pages_dir != source.pages_dir {
            return Err(
                "backup was made with a different pages or journals directory setting".into(),
            );
        }
        vec!["journals", "pages", source.assets_dir_name.as_str()]
    };
    // A missing area would read as "no files" and retire the live ones.
    if areas.iter().any(|area| !src.join(area).is_dir()) {
        return Err("backup contents do not match the verified manifest".into());
    }
    let snapshot = snapshot_current(&source);
    let live_n = [Area::Graph, Area::Assets]
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
    let files = open_verified_restore_files(&src, &manifest, &source, scope.is_some())?;
    store
        .restore(tine_store::EditKind::ReplacePage, files, scope.as_deref())
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
    graph_wide: bool,
) -> Result<Vec<RestoreFile>, String> {
    let mut files = Vec::new();
    // Preserve the old area order: graph text (or journals, pages), asset
    // sidecars, config.
    let text: &[(&str, Area)] = if graph_wide {
        &[("graph", Area::Graph)]
    } else {
        &[("journals", Area::Journals), ("pages", Area::Pages)]
    };
    for &(prefix, area) in text.iter().chain(&[
        (source.assets_dir_name.as_str(), Area::Assets),
        ("logseq", Area::Meta),
    ]) {
        for entry in &manifest.files {
            let Some(rel) = entry
                .path
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
            else {
                continue;
            };
            let accepted = match area {
                Area::Journals | Area::Pages | Area::Graph => {
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
                schema: LEGACY_SNAPSHOT_SCHEMA,
                root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                graph_text_policy: None,
                writer: None,
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
            hidden: Vec::new(),
            hidden_parse_failed_closed: false,
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
            schema: LEGACY_SNAPSHOT_SCHEMA,
            root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            graph_text_policy: None,
            writer: None,
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
                schema: LEGACY_SNAPSHOT_SCHEMA,
                root: std::fs::canonicalize(&graph).unwrap().display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                graph_text_policy: None,
                writer: None,
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

    fn write(path: &std::path::Path, bytes: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Graph text outside `pages/` and `journals/` for the og-B tests: a root
    /// page, a nested non-root page, and every excluded shape next to them.
    fn whole_graph(graph: &std::path::Path) {
        write(
            &graph.join("logseq/config.edn"),
            "{:hidden [\"private\"]}\n",
        );
        write(&graph.join("pages/A.md"), "- a\n");
        write(&graph.join("journals/2026_07_01.md"), "- j\n");
        write(&graph.join("Root.md"), "- root\n");
        write(&graph.join("archive/deep/X.org"), "* x\n");
        write(&graph.join("private/S.md"), "- hidden\n");
        write(&graph.join("assets/doc.edn"), "{:a 1}\n");
        write(&graph.join("assets/note.md"), "- asset, not graph text\n");
        write(&graph.join("logseq/bak/B.md"), "- bak\n");
        write(&graph.join(".dot/D.md"), "- dot\n");
        write(
            &graph.join("Root.sync-conflict-20260101-000000-ABCDEFG.md"),
            "- copy\n",
        );
    }

    /// og-B (master ffb4cb3d7): a snapshot holds graph text across the whole
    /// graph at its graph-relative path, and a restore brings it back there,
    /// retiring later text in the recorded scope into recovery.
    #[test]
    fn whole_graph_snapshot_restores_text_outside_pages_and_journals() {
        let root = scratch("whole-graph-snapshot");
        let graph = root.join("graph");
        let base = root.join("backups");
        whole_graph(&graph);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = write_snapshot(&base, &store, source.clone(), "", &|| false);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let stamp = std::fs::read_dir(&base)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .file_name();
        let stamp = stamp.to_str().unwrap().to_owned();
        let manifest = read_manifest(&base.join(&stamp)).unwrap();
        let mut paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            [
                "assets/doc.edn",
                "graph/Root.md",
                "graph/archive/deep/X.org",
                "graph/journals/2026_07_01.md",
                "graph/pages/A.md",
                "logseq/config.edn",
            ]
        );
        assert_eq!(manifest.schema, SNAPSHOT_SCHEMA);
        assert_eq!(manifest.writer.as_deref(), Some(SNAPSHOT_WRITER));
        let policy = manifest.graph_text_policy.as_ref().unwrap();
        assert_eq!(
            (policy.version, policy.hidden.clone()),
            (2, vec!["private".to_owned()])
        );

        write(&graph.join("Root.md"), "- root edited\n");
        std::fs::remove_file(graph.join("archive/deep/X.org")).unwrap();
        write(&graph.join("archive/Later.md"), "- later\n");
        write(&graph.join("private/Later.md"), "- hidden later\n");
        restore_from_backup_source(&stamp, &base, &store, source, |source| {
            write_snapshot(&base, &store, source.clone(), "pre-restore", &|| false)
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(graph.join("Root.md")).unwrap(),
            "- root\n"
        );
        assert_eq!(
            std::fs::read_to_string(graph.join("archive/deep/X.org")).unwrap(),
            "* x\n"
        );
        assert!(!graph.join("archive/Later.md").exists());
        let recovered = std::fs::read_dir(graph.join("logseq/.tine-trash"))
            .unwrap()
            .flatten()
            .any(|dir| dir.path().join("graph/archive/Later.md").is_file());
        assert!(
            recovered,
            "I-2: retired later text must sit in restore recovery"
        );
        for (rel, bytes) in [
            ("private/Later.md", "- hidden later\n"),
            ("private/S.md", "- hidden\n"),
            ("assets/note.md", "- asset, not graph text\n"),
            ("logseq/bak/B.md", "- bak\n"),
            (".dot/D.md", "- dot\n"),
            ("Root.sync-conflict-20260101-000000-ABCDEFG.md", "- copy\n"),
        ] {
            assert_eq!(
                std::fs::read_to_string(graph.join(rel)).unwrap(),
                bytes,
                "{rel}"
            );
        }
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Old-manifest compatibility: a schema-2 snapshot exactly as og wrote it
    /// before og-B (no scope policy, no writer mark, `journals/` + `pages/`)
    /// still lists and restores into the configured roots, and its restore
    /// leaves text outside those roots alone.
    #[test]
    fn schema_2_snapshot_restores_after_the_schema_3_bump() {
        let root = scratch("schema-2-compat");
        let graph = root.join("graph");
        let base = root.join("backups");
        let stamp = "2026-09-01_00-00-00";
        let snapshot = base.join(stamp);
        for dir in ["pages", "journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
            std::fs::create_dir_all(snapshot.join(dir)).unwrap();
        }
        write(&graph.join("pages/Old.md"), "- old\n");
        write(&graph.join("Root.md"), "- root live\n");
        write(&snapshot.join("pages/New.md"), "- new\n");
        write(&snapshot.join("journals/2026_07_01.md"), "- j\n");
        write(&snapshot.join("assets/doc.edn"), "{:a 1}\n");
        let files = snapshot_inventory(&snapshot).unwrap();
        let canonical = std::fs::canonicalize(&graph).unwrap().display().to_string();
        let manifest = serde_json::json!({
            "schema": 2,
            "root": canonical,
            "journals_dir": "journals",
            "pages_dir": "pages",
            "files": files.iter().map(|f| serde_json::json!({"path": f.path, "sha256": f.sha256})).collect::<Vec<_>>(),
            "complete": true,
        });
        std::fs::write(
            snapshot.join(SNAPSHOT_MANIFEST),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert_eq!(list_backups_from_base(&base, &graph).len(), 1);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        restore_from_backup_source(stamp, &base, &store, source, |_| BackupOutcome::success(1))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(graph.join("pages/New.md")).unwrap(),
            "- new\n"
        );
        assert_eq!(
            std::fs::read_to_string(graph.join("journals/2026_07_01.md")).unwrap(),
            "- j\n"
        );
        assert_eq!(
            std::fs::read_to_string(graph.join("assets/doc.edn")).unwrap(),
            "{:a 1}\n"
        );
        assert!(!graph.join("pages/Old.md").exists());
        assert_eq!(
            std::fs::read_to_string(graph.join("Root.md")).unwrap(),
            "- root live\n",
            "a schema-2 snapshot never covered root text, so its restore must not retire it"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A schema-3 snapshot as master writes it (scope policy, no og writer
    /// mark) lists and restores here. Its recorded pages directory does not
    /// steer the restore (master 336833b13), and a fail-closed `:hidden`
    /// policy hides everything, so that restore retires no live text.
    #[test]
    fn master_schema_3_snapshot_restores_at_graph_relative_paths() {
        for failed_closed in [false, true] {
            let root = scratch(&format!("master-schema-3-{failed_closed}"));
            let graph = root.join("graph");
            let base = root.join("backups");
            let stamp = "2026-09-01_00-00-00";
            let snapshot = base.join(stamp);
            for dir in ["pages", "journals", "assets", "logseq"] {
                std::fs::create_dir_all(graph.join(dir)).unwrap();
            }
            std::fs::create_dir_all(snapshot.join("assets")).unwrap();
            write(&graph.join("Live.md"), "- live\n");
            if !failed_closed {
                write(&snapshot.join("graph/Root.md"), "- root\n");
            } else {
                std::fs::create_dir_all(snapshot.join("graph")).unwrap();
            }
            let files = snapshot_inventory(&snapshot).unwrap();
            let canonical = std::fs::canonicalize(&graph).unwrap().display().to_string();
            let manifest = serde_json::json!({
                "schema": 3,
                "root": canonical,
                "journals_dir": "old-journals",
                "pages_dir": "old-pages",
                "graph_text_policy": {"version": 2, "hidden": [], "hidden_parse_failed_closed": failed_closed},
                "files": files.iter().map(|f| serde_json::json!({"path": f.path, "sha256": f.sha256})).collect::<Vec<_>>(),
                "complete": true,
            });
            std::fs::write(
                snapshot.join(SNAPSHOT_MANIFEST),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
            assert_eq!(list_backups_from_base(&base, &graph).len(), 1);
            assert!(
                is_foreign_snapshot(&snapshot),
                "master's snapshots are not ours to prune"
            );
            let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
            let source = BackupSource::from_store(&store, &graph).unwrap();
            restore_from_backup_source(stamp, &base, &store, source, |_| BackupOutcome::success(1))
                .unwrap();
            if failed_closed {
                assert_eq!(
                    std::fs::read_to_string(graph.join("Live.md")).unwrap(),
                    "- live\n"
                );
            } else {
                assert_eq!(
                    std::fs::read_to_string(graph.join("Root.md")).unwrap(),
                    "- root\n"
                );
                assert!(!graph.join("Live.md").exists());
            }
            drop(store);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// og-T2: a snapshot taken while `:hidden` failed to parse (torn or
    /// hand-broken config.edn) holds no graph text and records the
    /// fail-closed scope, so restoring it retires no live text.
    #[test]
    fn failed_closed_hidden_snapshot_records_scope_and_retires_nothing() {
        let root = scratch("failed-closed-hidden");
        let graph = root.join("graph");
        let base = root.join("backups");
        write(&graph.join("logseq/config.edn"), "{:hidden [\"private\"\n");
        write(&graph.join("pages/A.md"), "- a\n");
        for dir in ["journals", "assets"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = write_snapshot(&base, &store, source.clone(), "", &|| false);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let stamp = std::fs::read_dir(&base)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .file_name();
        let stamp = stamp.to_str().unwrap().to_owned();
        let manifest = read_manifest(&base.join(&stamp)).unwrap();
        assert!(
            manifest
                .graph_text_policy
                .as_ref()
                .unwrap()
                .hidden_parse_failed_closed
        );
        assert!(manifest.files.iter().all(|f| !f.path.starts_with("graph/")));
        write(&graph.join("pages/Later.md"), "- later\n");
        restore_from_backup_source(&stamp, &base, &store, source, |_| BackupOutcome::success(1))
            .unwrap();
        for (rel, bytes) in [("pages/A.md", "- a\n"), ("pages/Later.md", "- later\n")] {
            assert_eq!(std::fs::read_to_string(graph.join(rel)).unwrap(), bytes);
        }
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Differential with master ffb4cb3d7: master's own fixture
    /// (`graph_wide_snapshot_preserves_eligible_paths_and_excludes_internal_trees`)
    /// through og's `Area::Graph` selection copies exactly master's three files.
    #[test]
    fn graph_text_selection_matches_masters_fixture() {
        let root = scratch("master-fixture-differential");
        let graph = root.join("graph");
        write(
            &graph.join("logseq/config.edn"),
            "{:hidden [\"private\"]}\n",
        );
        for (rel, bytes) in [
            ("Root.md", "root\n"),
            ("pages/Normal.org", "* normal\n"),
            ("archive/自由/Elsewhere.Markdown", "elsewhere\n"),
            ("assets/ignored.md", "asset\n"),
            ("logseq/.tine-trash/pages/ignored.md", "trash\n"),
            (".hidden/ignored.md", "hidden\n"),
            ("private/ignored.md", "private\n"),
        ] {
            write(&graph.join(rel), bytes);
        }
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let dest = root.join("snapshot/graph");
        let (copied, failed, failure) =
            copy_store_area(&store, Area::Graph, &dest, is_graph_text, &|| false);
        assert_eq!((copied, failed), (3, 0), "{failure:?}");
        let mut got: Vec<String> = snapshot_inventory(&dest)
            .unwrap()
            .into_iter()
            .map(|file| file.path)
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                "Root.md",
                "archive/自由/Elsewhere.Markdown",
                "pages/Normal.org"
            ]
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
