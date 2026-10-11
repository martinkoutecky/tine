//! Restore a verified snapshot into the live graph (split from `backup.rs`
//! along the snapshot/restore seam). Creating, listing and pruning snapshots
//! stays in the parent module; this module only reads a published snapshot,
//! checks it against its manifest, and hands the verified files to
//! `Store::restore`, the graph's one guarded restore write path.

use super::*;

/// Restore a snapshot into the live graph. A schema-5 zip and a schema-3
/// directory put graph text back at its graph-relative path across the
/// whole graph; schema 2 keeps the old configured-root behaviour. All
/// restore asset `.edn` sidecars and `config.edn`. Takes a fresh safety snapshot of the *current* state first
/// (so a mistaken restore is itself reversible). With a page host, the
/// restore runs between its stop and a fresh host (`restore_hosted`);
/// `consumed_last_id` is the last host answer the window consumed (0 with
/// no host). Destructive — the frontend confirms.
#[tauri::command]
pub(crate) async fn restore_backup(
    id: String,
    consumed_last_id: u64,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<RestoreReply, String> {
    let slot = slot_for_context(&state)?;
    let source = BackupSource::from_store(&slot.store, &slot.root_key)
        .map_err(|(kind, message)| format!("backup-failed:source:{kind:?}: {message}"))?;
    let restore_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let base = backup_base_for_root(&restore_app, &source.root).ok_or("no app-data dir")?;
        restore_hosted(&slot.host, consumed_last_id, || {
            restore_from_backup_source(&id, &base, &slot.store, source, |source| {
                do_backup_source(&restore_app, &slot.store, source.clone(), "pre-restore")
            })
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// A restore that ran (plan v3 §5, S8): `error` when it failed part-way,
/// and the session of the host relaunched after it, on success and on a
/// partial failure alike (None with no host). The window rebinds to that
/// session either way.
#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct RestoreReply {
    error: Option<String>,
    reloaded: Option<tine_store::Reloaded>,
}

/// A backup restore around the binding's page host (STEP3 §7, R7; plan v3
/// S8). The binding's host lock is held throughout, so a retirement, an
/// adoption, another restore and every hosted writer wait for this one's
/// outcome. A running host stops in restore mode: every drained edit saved
/// and published, every draft and custody debt retired. A stop that cannot
/// complete restores nothing and the host keeps running, naming the pages
/// it could not save (today's restore likewise aborts when its flush
/// fails). Otherwise `restore` runs and one fresh host launches on the tree
/// as it now is, on success and on failure alike. With no host `restore` is
/// the plain call.
pub(super) fn restore_hosted(
    slot: &std::sync::RwLock<crate::state::PageHostSlot>,
    consumed_last_id: u64,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<RestoreReply, String> {
    use crate::state::PageHostSlot;
    let mut slot = slot.write().unwrap_or_else(|e| e.into_inner());
    if matches!(*slot, PageHostSlot::Revoked) {
        // Adopted by another window's binding (REVIEW-3b-P1 B1): never the
        // no-host arm, whose restore would bypass the adopted host.
        return Err(crate::state::STALE_BINDING.into());
    }
    let PageHostSlot::Running(host) = std::mem::take(&mut *slot) else {
        restore()?;
        return Ok(RestoreReply {
            error: None,
            reloaded: None,
        });
    };
    let stopped = match host.stop_saved(consumed_last_id, tine_store::StopMode::Restore) {
        Ok(stopped) => stopped,
        Err((host, pages)) => {
            *slot = PageHostSlot::Running(*host);
            let pages: Vec<_> = pages.iter().map(tine_store::PageId::as_str).collect();
            return Err(format!(
                "restore-aborted: unsaved pages: {}",
                pages.join(", ")
            ));
        }
    };
    let error = restore().err();
    let host = stopped.relaunch().map_err(|relaunch| {
        let relaunch = format!("page-host-relaunch-failed: {relaunch}");
        error
            .iter()
            .fold(relaunch, |relaunch, error| format!("{error}; {relaunch}"))
    })?;
    let reloaded = host.window_reloaded();
    *slot = PageHostSlot::Running(host);
    Ok(RestoreReply {
        error,
        reloaded: Some(reloaded),
    })
}

/// Every refusal of a snapshot whose own bytes are missing, torn, oversized
/// or unverifiable, its manifest checksum included (power loss, torn write,
/// disk error). Nothing in the graph has changed. Refusal row:
/// docs/storage-contract.md, Graph backups.
const DAMAGED: &str = "this backup is damaged; pick another";

/// A restore that waited its bound for another Tine process's backup work
/// on this graph (concurrent honest instances). Nothing in the graph has
/// changed. Refusal row: docs/storage-contract.md, Graph backups.
const BUSY: &str =
    "backups are busy in another Tine window or process; try the restore again in a moment";

/// The verified files to restore and the graph-text scope they cover.
type Verified = (Vec<RestoreFile>, Option<Vec<String>>);

/// Restore the snapshot `id` (`zip:<name>` or `dir:<name>`, from
/// `list_backups`). Every byte is read and verified before the pre-restore
/// safety snapshot, and `Store::restore` writes exactly those bytes; a
/// refusal before then writes nothing anywhere.
fn restore_from_backup_source(
    id: &str,
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    snapshot_current: impl FnOnce(&BackupSource) -> BackupOutcome,
) -> Result<(), String> {
    let (namespace, name) = id.split_once(':').ok_or("invalid backup id")?;
    let name = naming::SnapshotName::parse(name).ok_or("invalid backup id")?;
    let (files, scope) = match namespace {
        "zip" => verified_zip(base, &name, &source)?,
        "dir" => verified_dir(base, &name, &source)?,
        _ => return Err("invalid backup id".into()),
    };
    let snapshot = snapshot_current(&source);
    // Config is live content too (og-backup-cas D5).
    let live_n = [Area::Graph, Area::Assets, Area::Meta]
        .into_iter()
        .map(|area| {
            store.scan_area(area, None).ok().map(|listing| {
                listing
                    .files
                    .iter()
                    .filter(|entry| match area {
                        Area::Assets => is_asset_sidecar(&entry.id),
                        Area::Meta => entry.rel == "config.edn",
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
    store
        .restore(tine_store::EditKind::ReplacePage, files, scope.as_deref())
        .map_err(|error| format_restore_failure(&error))?;
    Ok(())
}

/// Run `read` holding the namespace lock (bounded wait), so no prune or
/// partial cleanup removes the snapshot mid-read (REVIEW-backup-zip B1).
fn under_namespace_lock<T>(
    base: &std::path::Path,
    read: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let zips = zip_base(base);
    let lock_failed = |error: std::io::Error| format!("backup-failed:lock:{:?}", error.kind());
    std::fs::create_dir_all(&zips).map_err(lock_failed)?;
    let _lock = match crate::file_lock::lock_exclusive_until(
        &zips.join(naming::LOCK_FILE),
        std::time::Instant::now() + BACKUP_LOCK_WAIT,
    ) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Err(BUSY.into()),
        Err(error) => return Err(lock_failed(error)),
    };
    read()
}

/// A schema-5 zip: read whole under the lock (bounded), then fully verify
/// outside it, then map each entry to its graph area (REVIEW R8).
fn verified_zip(
    base: &std::path::Path,
    name: &naming::SnapshotName,
    source: &BackupSource,
) -> Result<Verified, String> {
    let path = zip_base(base).join(format!("{}{}", name.render(), naming::ZIP_EXT));
    let bytes = under_namespace_lock(base, || match read_archive(&path) {
        Ok(bytes) => Ok(bytes),
        Err(archive::Problem::Io(ErrorKind::NotFound)) => Err("backup not found".into()),
        Err(archive::Problem::TooLarge(_)) => Err(DAMAGED.into()),
        Err(problem) => Err(format!("backup-failed:read:{problem:?}")),
    })?;
    let verified = match archive::verify(&mut &bytes[..]) {
        Ok(verified) => verified,
        Err(archive::Problem::Unsupported(_)) => {
            return Err("this backup was made by a newer Tine; it cannot be restored here".into())
        }
        Err(_) => return Err(DAMAGED.into()),
    };
    drop(bytes);
    let manifest = verified.manifest;
    if manifest.root != source.root.display().to_string() {
        return Err("backup belongs to a different graph".into());
    }
    // Its sidecars live under the assets directory it recorded; restoring
    // them elsewhere, or none, would retire the live ones.
    if manifest.assets_dir != source.assets_dir_name {
        return Err("backup was made with a different assets directory setting".into());
    }
    let policy = &manifest.graph_text_policy;
    let scope = if policy.hidden_parse_failed_closed {
        vec![String::new()]
    } else {
        policy.hidden.clone()
    };
    // Names that differ only in case (or, on Apple, in Unicode normalization)
    // would be one file on a folding volume.
    if !archive::paths_are_safe(
        manifest.files.iter().map(|file| file.path.as_str()),
        cfg!(windows),
        archive::DESTINATION_FOLD,
    ) {
        return Err(
            "this backup holds names that differ only in letter case; restore it by hand".into(),
        );
    }
    let assets = format!("{}/", manifest.assets_dir);
    let files = manifest
        .files
        .iter()
        .zip(verified.files)
        .map(|(entry, bytes)| {
            let id = tine_store::FileId::from(entry.path.clone());
            let (area, rel) = if entry.path == "logseq/config.edn" {
                (Area::Meta, "config.edn")
            } else if let Some(rel) = entry
                .path
                .strip_prefix(&assets)
                .filter(|_| is_asset_sidecar(&id))
            {
                (Area::Assets, rel)
            } else if is_graph_text(&id) {
                (Area::Graph, entry.path.as_str())
            } else {
                return Err(DAMAGED.to_owned());
            };
            Ok(RestoreFile {
                area,
                rel: rel.into(),
                bytes,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((files, Some(scope)))
}

/// A legacy schema-2/3 snapshot directory: its manifest, its inventory,
/// and every file read bounded and hashed, all under the lock.
fn verified_dir(
    base: &std::path::Path,
    name: &naming::SnapshotName,
    source: &BackupSource,
) -> Result<Verified, String> {
    let src = base.join(name.render());
    under_namespace_lock(base, || {
        if !src.is_dir() {
            return Err("backup not found".into());
        }
        let manifest = read_manifest(&src).ok_or("backup is incomplete or unverified")?;
        if manifest.root != source.root.display().to_string() {
            return Err("backup belongs to a different graph".into());
        }
        if !verify_snapshot(&src, &manifest) {
            return Err(DAMAGED.into());
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
        // Schema 3 text sits at its graph-relative path, so the recorded
        // pages and journals directories do not steer it (master 336833b13);
        // `hidden` is the graph-text scope it covered. `[""]` hides all:
        // master's fail-closed `:hidden` snapshot holds no text, so it must
        // retire none.
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
            // A schema-2 restore targets the open graph's current layout; a
            // backup taken under a different :pages-directory /
            // :journals-directory is not restored (v0.6.5 wrote into the
            // backup's old directory names).
            if manifest.journals_dir != source.journals_dir
                || manifest.pages_dir != source.pages_dir
            {
                return Err(
                    "backup was made with a different pages or journals directory setting".into(),
                );
            }
            vec!["journals", "pages", source.assets_dir_name.as_str()]
        };
        // A missing area would read as "no files" and retire the live ones.
        if areas.iter().any(|area| !src.join(area).is_dir()) {
            return Err(DAMAGED.into());
        }
        let files = read_verified_dir_files(&src, &manifest, source, scope.is_some())?;
        Ok((files, scope))
    })
}

fn require_safety_snapshot(snapshot: BackupOutcome, live_n: usize) -> Result<(), String> {
    // Any safety-snapshot failure refuses, whatever the live count
    // (og-backup-cas D5); so does an empty snapshot of a nonempty graph.
    if snapshot.failure.is_some() || (live_n > 0 && snapshot.copied == 0) {
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

/// A legacy snapshot's restorable files, each read whole (bounded by the
/// zip format's per-entry and aggregate limits) and checked against its
/// manifest hash, in the old area order: graph text (or journals, pages),
/// asset sidecars, config.
fn read_verified_dir_files(
    snapshot: &std::path::Path,
    manifest: &SnapshotManifest,
    source: &BackupSource,
    graph_wide: bool,
) -> Result<Vec<RestoreFile>, String> {
    let mut files = Vec::new();
    let mut total = 0u64;
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
                return Err(DAMAGED.into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(snapshot.join(&entry.path))
                .and_then(|file| {
                    file.take(archive::MAX_ENTRY_BYTES + 1)
                        .read_to_end(&mut bytes)
                })
                .map_err(|_| DAMAGED)?;
            total += bytes.len() as u64;
            if bytes.len() as u64 > archive::MAX_ENTRY_BYTES
                || total > archive::MAX_TOTAL_BYTES
                || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256
            {
                return Err(DAMAGED.into());
            }
            files.push(RestoreFile {
                area,
                rel: rel.into(),
                bytes,
            });
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R7 (STEP3 §7 steps 4–6), plan v3 S8: a restore with a running host
    /// holds the binding's host gate across stop, file restore and relaunch
    /// (a hosted writer or another restore waits for its outcome), then
    /// binds one fresh host and replies with its session, after a failed
    /// restore too; with no host the restore is the plain call.
    #[test]
    fn a_restore_holds_the_host_gate_until_one_fresh_host_runs() {
        use crate::state::PageHostSlot;
        use std::sync::{Arc, RwLock};
        let root = scratch("restore-host");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/a.md"), "- a\n").unwrap();
        let app_data = scratch("restore-host-app");
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let gated = |slot: &RwLock<PageHostSlot>| slot.try_read().is_err();
        let off = RwLock::new(PageHostSlot::Off);
        let mut ran = false;
        let reply = restore_hosted(&off, 0, || {
            ran = true;
            Ok(())
        })
        .unwrap();
        assert!(ran && matches!(*off.read().unwrap(), PageHostSlot::Off));
        assert_eq!(reply.reloaded, None, "no host: no session to rebind to");
        let host = tine_store::PageHost::start_for_tests(&store, &app_data).unwrap();
        let slot = RwLock::new(PageHostSlot::Running(host));
        let mut sessions = std::collections::BTreeSet::new();
        for outcome in [Ok(()), Err("restore-failed:Other: copy".to_owned())] {
            let reply = restore_hosted(&slot, 0, || {
                assert!(gated(&slot), "S8: writers wait while the restore runs");
                outcome.clone()
            })
            .unwrap();
            assert_eq!(reply.error, outcome.err());
            let reloaded = serde_json::to_value(reply.reloaded.unwrap()).unwrap();
            assert!(sessions.insert(reloaded["session"].as_u64().unwrap()));
            assert!(
                slot.read().unwrap().running().is_some(),
                "R7: one fresh host after the restore, on failure too"
            );
        }
        // S8: a second restore waits for the first one's outcome.
        let (entered, release) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        let overlapped = std::sync::atomic::AtomicBool::new(false);
        let inside = std::sync::atomic::AtomicBool::new(false);
        let run = |first: bool| {
            restore_hosted(&slot, 0, || {
                if inside.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if first {
                    entered.wait();
                    release.wait();
                }
                inside.store(false, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .unwrap()
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(|| run(true));
            entered.wait();
            let second = scope.spawn(|| run(false));
            std::thread::sleep(std::time::Duration::from_millis(200));
            release.wait();
            first.join().unwrap();
            second.join().unwrap();
        });
        assert!(
            !overlapped.load(std::sync::atomic::Ordering::SeqCst),
            "S8: two restores of one binding never overlap"
        );
        *slot.write().unwrap() = PageHostSlot::Off;
        store.close();
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(app_data).unwrap();
    }

    /// Plan v3 §3 step 6, S8: a window closed while its restore runs. The
    /// retirement waits for the restore's outcome, then stops the host the
    /// restore relaunched, and only then closes the Store.
    #[test]
    fn a_retirement_waits_for_the_restore_it_raced() {
        use crate::state::{GraphRegistry, GraphSlot, PageHostSlot};
        use std::sync::{Arc, Barrier};
        let root = scratch("restore-retire");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        let app_data = scratch("restore-retire-app");
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let mut slot = GraphSlot::new(store.clone(), Store::canonical_root(&root).unwrap());
        let host = tine_store::PageHost::start_for_tests(&store, &app_data).unwrap();
        *slot.host.get_mut().unwrap() = PageHostSlot::Running(host);
        let slot = Arc::new(slot);
        let mut registry = GraphRegistry::default();
        registry.bind("graph-1".into(), slot.clone()).unwrap();
        let (entered, release) = (Barrier::new(2), Barrier::new(2));
        let closed = || matches!(store.is_graph_ready(), Err(tine_store::LoadError::Closed));
        std::thread::scope(|scope| {
            let restore = scope.spawn(|| {
                restore_hosted(&slot.host, 0, || {
                    entered.wait();
                    release.wait();
                    Ok(())
                })
            });
            entered.wait();
            assert!(
                registry.remove("graph-1").is_none(),
                "retiring, not dropped"
            );
            let retirement = registry.retirement.clone();
            assert!(retirement
                .wait_idle(std::time::Duration::from_millis(200))
                .is_err());
            assert!(!closed(), "the Store stays open under the restore");
            release.wait();
            assert!(restore.join().unwrap().unwrap().reloaded.is_some());
            let idle = retirement.wait_idle(std::time::Duration::from_secs(20));
            assert_eq!(idle, Ok(()));
        });
        assert!(matches!(*slot.host_slot().unwrap(), PageHostSlot::Off));
        assert!(closed());
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(app_data).unwrap();
    }

    /// REVIEW-3b-P1 B1: a restore queued on a slot whose root another
    /// binding adopted gets the stale-binding outcome under the gate. It
    /// never enters the no-host arm on the adopted Store.
    #[test]
    fn review_p1_a_queued_restore_cannot_enter_after_slot_adoption() {
        use crate::state::{GraphRegistry, GraphSlot, PageHostSlot, STALE_BINDING};
        use std::sync::Arc;
        let root = scratch("restore-adopted");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/a.md"), "- a\n").unwrap();
        let app_data = scratch("restore-adopted-app");
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let canonical = Store::canonical_root(&root).unwrap();
        let mut old = GraphSlot::new(store.clone(), canonical.clone());
        let host = tine_store::PageHost::start_for_tests(&store, &app_data).unwrap();
        let reservation = host
            .reserve(
                || vec![tine_store::PageId::from("pages/a.md")],
                tine_store::Input::Refuse,
            )
            .unwrap();
        *old.host.get_mut().unwrap() = PageHostSlot::Running(host);
        let old = Arc::new(old);
        let mut registry = GraphRegistry::default();
        registry.bind("graph-1".into(), old.clone()).unwrap();
        assert!(registry.remove("graph-1").is_none(), "retiring");
        let (adopted, host) = registry.retirement.adopt(&canonical).unwrap();
        let mut fresh = GraphSlot::new(adopted, canonical);
        *fresh.host.get_mut().unwrap() = host;
        let mut entered = false;
        let reply = restore_hosted(&old.host, 0, || {
            entered = true;
            Ok(())
        });
        assert_eq!(reply, Err(STALE_BINDING.to_owned()));
        assert!(!entered, "the restore never ran on the adopted Store");
        assert!(fresh.host_slot().unwrap().running().is_some());
        drop(reservation);
        drop((fresh, old));
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(app_data).unwrap();
    }

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
        let mut out = Vec::new();
        let (copied, failed, failure) =
            read_store_area(&store, Area::Pages, "", is_graph_text, &|| false, &mut out);
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
        let result =
            restore_from_backup_source(&format!("dir:{stamp}"), &base, &store, source, |_| {
                panic!("verification precedes the safety snapshot")
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
            "dir:2026-07-22_12-00-00",
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
            "dir:2026-07-22_12-00-00",
            &root.join("backups"),
            &store,
            source,
            |_| panic!("missing snapshot area must be rejected before the safety snapshot"),
        );
        assert_eq!(result.unwrap_err(), DAMAGED);
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
        let outcome = snapshot(&base, &store, &source, "");
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let (id, manifest) = newest_zip(&base, &graph);
        let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "Root.md",
                "archive/deep/X.org",
                "assets/doc.edn",
                "journals/2026_07_01.md",
                "logseq/config.edn",
                "pages/A.md",
            ]
        );
        assert_eq!(manifest.schema, archive::ZIP_SCHEMA);
        assert_eq!(manifest.writer, SNAPSHOT_WRITER);
        let policy = &manifest.graph_text_policy;
        assert_eq!(
            (policy.version, policy.hidden.clone()),
            (2, vec!["private".to_owned()])
        );
        let before = tree(&graph);

        write(&graph.join("Root.md"), "- root edited\n");
        std::fs::remove_file(graph.join("archive/deep/X.org")).unwrap();
        write(&graph.join("archive/Later.md"), "- later\n");
        write(&graph.join("private/Later.md"), "- hidden later\n");
        restore_from_backup_source(&id, &base, &store, source, |source| {
            snapshot(&base, &store, source, "pre-restore")
        })
        .unwrap();
        let after = tree(&graph);
        for (rel, bytes) in &before {
            assert_eq!(
                after.get(rel),
                Some(bytes),
                "zip to graph is byte-identical: {rel}"
            );
        }
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
        restore_from_backup_source(&format!("dir:{stamp}"), &base, &store, source, |_| {
            BackupOutcome::success(1)
        })
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
            restore_from_backup_source(&format!("dir:{stamp}"), &base, &store, source, |_| {
                BackupOutcome::success(1)
            })
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
        let outcome = snapshot(&base, &store, &source, "");
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let (id, manifest) = newest_zip(&base, &graph);
        assert!(manifest.graph_text_policy.hidden_parse_failed_closed);
        assert_eq!(
            manifest
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            ["logseq/config.edn"]
        );
        write(&graph.join("pages/Later.md"), "- later\n");
        restore_from_backup_source(&id, &base, &store, source, |_| BackupOutcome::success(1))
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
        let mut out = Vec::new();
        let (copied, failed, failure) =
            read_store_area(&store, Area::Graph, "", is_graph_text, &|| false, &mut out);
        assert_eq!((copied, failed), (3, 0), "{failure:?}");
        let mut got: Vec<String> = out.into_iter().map(|(path, _)| path).collect();
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

    fn snapshot(
        base: &std::path::Path,
        store: &Store,
        source: &BackupSource,
        suffix: &str,
    ) -> BackupOutcome {
        write_snapshot(
            base,
            store,
            source.clone(),
            suffix,
            &|| false,
            BACKUP_KEEP_DEFAULT,
            BACKUP_LOCK_WAIT,
        )
    }

    /// The newest zip's restore id and verified manifest.
    fn newest_zip(
        base: &std::path::Path,
        graph: &std::path::Path,
    ) -> (String, archive::ZipManifest) {
        let listed = list_backups_from_base(base, graph);
        let id = listed
            .iter()
            .find(|info| info.id.starts_with("zip:"))
            .unwrap()
            .id
            .clone();
        let path = zip_base(base).join(format!("{}.zip", &id["zip:".len()..]));
        let bytes = read_archive(&path).unwrap();
        (id, archive::verify(&mut &bytes[..]).unwrap().manifest)
    }

    /// Every file under `graph` outside dot-directories, with its bytes.
    fn tree(graph: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![graph.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path
                        .strip_prefix(graph)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    /// Dossier restore gate: each damaged zip is refused as damaged before
    /// the safety snapshot, and nothing in the graph changes. Truncated, one
    /// flipped payload byte, a manifest edit that keeps valid JSON, a missing
    /// entry, a duplicate entry, a `../` path.
    #[test]
    fn damaged_zips_are_refused_before_anything_is_written() {
        let root = scratch("restore-damaged-zips");
        let graph = root.join("graph");
        let base = root.join("app/backups/g");
        write(&graph.join("pages/A.md"), "- a\n");
        write(&graph.join("pages/B.md"), "- b\n");
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        assert!(snapshot(&base, &store, &source, "").failure.is_none());
        let (id, manifest) = newest_zip(&base, &graph);
        let zips = zip_base(&base);
        let good = std::fs::read(zips.join(format!("{}.zip", &id["zip:".len()..]))).unwrap();
        let files: Vec<(String, Vec<u8>)> = manifest
            .files
            .iter()
            .zip(archive::verify(&mut &good[..]).unwrap().files)
            .map(|(file, bytes)| (file.path.clone(), bytes))
            .collect();
        let encoded = archive::encode_manifest(&manifest);
        let stamp = "2026-01-01_00-00-00";

        let truncated = good[..good.len() - 10].to_vec();
        let mut flipped = good.clone();
        let at = flipped
            .windows(10)
            .position(|w| w == b"pages/A.md")
            .unwrap()
            + 10
            + 1;
        flipped[at] ^= 0x01;
        let text = String::from_utf8(encoded.clone()).unwrap();
        let digest = &manifest.files[0].sha256;
        let edited = format!(
            "{}{}",
            if digest.starts_with('0') { "1" } else { "0" },
            &digest[1..]
        );
        let tampered = archive::write_archive(
            &files,
            text.replacen(digest.as_str(), &edited, 1).as_bytes(),
            stamp,
        )
        .unwrap();
        let missing = archive::write_archive(&files[1..], &encoded, stamp).unwrap();
        let entries = |files: &[(String, Vec<u8>)], manifest: &[u8]| {
            let mut all: Vec<(String, Vec<u8>)> = files.to_vec();
            all.push((archive::MANIFEST_NAME.into(), manifest.to_vec()));
            all
        };
        let raw = |all: Vec<(String, Vec<u8>)>| {
            let refs: Vec<(&str, &[u8])> = all
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect();
            archive::tests::raw(&refs)
        };
        let mut doubled = entries(&files, &encoded);
        doubled.insert(1, doubled[0].clone());
        let escape = vec![("../A.md".to_owned(), b"- escaped\n".to_vec())];
        let mut escape_manifest = manifest.clone();
        escape_manifest.files = vec![archive::ZipFile {
            path: "../A.md".into(),
            sha256: format!("{:x}", Sha256::digest(b"- escaped\n")),
            len: 10,
        }];
        let escaping = raw(entries(
            &escape,
            &archive::encode_manifest(&escape_manifest),
        ));

        write(&graph.join("pages/A.md"), "- live\n");
        let before = tree(&graph);
        for (k, (label, bytes)) in [
            ("truncated", truncated),
            ("flipped payload byte", flipped),
            ("manifest edit", tampered),
            ("missing entry", missing),
            ("duplicate entry", raw(doubled)),
            ("../ path", escaping),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("2026-01-0{}_00-00-00", k + 1);
            std::fs::write(zips.join(format!("{name}.zip")), bytes).unwrap();
            let result = restore_from_backup_source(
                &format!("zip:{name}"),
                &base,
                &store,
                source.clone(),
                |_| panic!("{label}: refused before the safety snapshot"),
            );
            assert_eq!(result.unwrap_err(), DAMAGED, "{label}");
            assert_eq!(tree(&graph), before, "{label}: nothing written");
        }
        assert!(!root.join("graph/../A.md").exists());
        store.close();
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D5 for both formats: config is live content, so an
    /// empty safety snapshot of a config-only graph refuses the restore.
    #[test]
    fn a_config_only_graph_needs_a_nonempty_safety_snapshot() {
        let root = scratch("restore-config-only");
        let graph = root.join("graph");
        let base = root.join("app/backups/g");
        write(&graph.join("logseq/config.edn"), "{:live true}\n");
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = snapshot(&base, &store, &source, "");
        assert_eq!(outcome.copied, 1, "the snapshot holds config");
        let (zip_id, _) = newest_zip(&base, &graph);
        let legacy = base.join("2026-09-01_00-00-00");
        write(&legacy.join("logseq/config.edn"), "{:live true}\n");
        std::fs::create_dir_all(legacy.join("graph")).unwrap();
        std::fs::create_dir_all(legacy.join("assets")).unwrap();
        write_manifest(
            &legacy,
            &SnapshotManifest {
                schema: SNAPSHOT_SCHEMA,
                root: source.root.display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                graph_text_policy: Some(SnapshotGraphTextPolicy {
                    version: GRAPH_TEXT_SCOPE_VERSION,
                    hidden: Vec::new(),
                    hidden_parse_failed_closed: false,
                }),
                writer: Some(SNAPSHOT_WRITER.into()),
                files: snapshot_inventory(&legacy).unwrap(),
                complete: true,
            },
        )
        .unwrap();
        std::fs::write(graph.join("logseq/config.edn"), "{:edited true}\n").unwrap();
        for id in [zip_id, "dir:2026-09-01_00-00-00".to_owned()] {
            let result = restore_from_backup_source(&id, &base, &store, source.clone(), |_| {
                BackupOutcome::success(0)
            });
            assert!(result.unwrap_err().contains("safety snapshot"), "{id}");
            assert_eq!(
                std::fs::read_to_string(graph.join("logseq/config.edn")).unwrap(),
                "{:edited true}\n",
                "{id}: nothing restored"
            );
        }
        store.close();
        let _ = std::fs::remove_dir_all(root);
    }

    /// D5's other side, for both formats: an empty graph has nothing to lose,
    /// so its empty safety snapshot lets the restore run.
    #[test]
    fn an_empty_graph_restores_without_a_safety_snapshot_copy() {
        let root = scratch("restore-empty-graph");
        let graph = root.join("graph");
        let base = root.join("app/backups/g");
        write(&graph.join("pages/A.md"), "- a\n");
        for dir in ["journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        assert!(snapshot(&base, &store, &source, "").failure.is_none());
        let (zip_id, _) = newest_zip(&base, &graph);
        let legacy = base.join("2026-09-01_00-00-00");
        write(&legacy.join("graph/pages/A.md"), "- a\n");
        std::fs::create_dir_all(legacy.join("assets")).unwrap();
        write_manifest(
            &legacy,
            &SnapshotManifest {
                schema: SNAPSHOT_SCHEMA,
                root: source.root.display().to_string(),
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                graph_text_policy: Some(SnapshotGraphTextPolicy {
                    version: GRAPH_TEXT_SCOPE_VERSION,
                    hidden: Vec::new(),
                    hidden_parse_failed_closed: false,
                }),
                writer: Some(SNAPSHOT_WRITER.into()),
                files: snapshot_inventory(&legacy).unwrap(),
                complete: true,
            },
        )
        .unwrap();
        for id in [zip_id, "dir:2026-09-01_00-00-00".to_owned()] {
            std::fs::remove_file(graph.join("pages/A.md")).unwrap();
            restore_from_backup_source(&id, &base, &store, source.clone(), |source| {
                snapshot(&base, &store, source, "pre-restore")
            })
            .unwrap();
            assert_eq!(
                std::fs::read_to_string(graph.join("pages/A.md")).unwrap(),
                "- a\n",
                "{id}"
            );
        }
        store.close();
        let _ = std::fs::remove_dir_all(root);
    }

    /// og-backup-cas D5: a failed safety snapshot refuses the restore even
    /// when the graph holds no live content to count.
    #[test]
    fn any_safety_snapshot_failure_refuses_the_restore() {
        assert!(require_safety_snapshot(
            BackupOutcome::failed(0, "reserve", ErrorKind::PermissionDenied),
            0
        )
        .unwrap_err()
        .starts_with("backup-failed:reserve:PermissionDenied"));
        assert!(require_safety_snapshot(BackupOutcome::success(0), 0).is_ok());
        assert!(require_safety_snapshot(BackupOutcome::success(0), 1).is_err());
    }

    /// R8: ids are namespace-qualified and validated by the name grammar.
    #[test]
    fn restore_ids_name_their_namespace() {
        let root = scratch("restore-ids");
        let graph = root.join("graph");
        write(&graph.join("pages/A.md"), "- a\n");
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        for id in [
            "2026-09-01_00-00-00",
            "tar:2026-09-01_00-00-00",
            "zip:../2026-09-01_00-00-00",
            "dir:2026-09-01_00-00-00/..",
            "zip:2026-09-01_00-00-00.zip",
        ] {
            let result =
                restore_from_backup_source(id, &root.join("b"), &store, source.clone(), |_| {
                    panic!("{id}")
                });
            assert_eq!(result.unwrap_err(), "invalid backup id", "{id}");
        }
        let result = restore_from_backup_source(
            "zip:2026-09-01_00-00-00",
            &root.join("b"),
            &store,
            source,
            |_| panic!("missing"),
        );
        assert_eq!(result.unwrap_err(), "backup not found");
        store.close();
        let _ = std::fs::remove_dir_all(root);
    }
}
