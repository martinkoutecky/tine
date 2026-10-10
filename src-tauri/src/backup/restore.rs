//! Restore a verified snapshot into the live graph (split from `backup.rs`
//! along the snapshot/restore seam). Creating, listing and pruning snapshots
//! stays in the parent module; this module only reads a published snapshot,
//! checks it (a full copy, or a schema-4 snapshot's blobs) against its
//! manifest, and hands the verified files to `Store::restore`, the graph's
//! one guarded restore write path.

use super::*;

/// Restore a snapshot into the live graph. Schemas 3 and 4 put graph text
/// back at its graph-relative path across the whole graph; schema 2 keeps the old
/// configured-root behaviour. Both restore asset `.edn` sidecars and
/// `config.edn`. Takes a fresh safety snapshot of the *current* state first
/// (so a mistaken restore is itself reversible). With a page host, the
/// restore runs between its stop and a fresh host (`restore_hosted`);
/// `consumed_last_id` is the last host answer the window consumed (0 with
/// no host). Destructive — the frontend confirms.
#[tauri::command]
pub(crate) async fn restore_backup(
    stamp: String,
    consumed_last_id: u64,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<RestoreReply, String> {
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
        restore_hosted(&slot.host, consumed_last_id, || {
            restore_from_backup_source(&stamp, &base, &slot.store, source, |source| {
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

fn restore_from_backup_source(
    stamp: &str,
    base: &std::path::Path,
    store: &Store,
    source: BackupSource,
    snapshot_current: impl FnOnce(&BackupSource) -> BackupOutcome,
) -> Result<(), String> {
    let cas = cas_dir(base);
    // Read and check everything under the namespace lock, so no prune or
    // blob repair changes the snapshot mid-read; the restore then consumes
    // exactly the verified bytes (REVIEW N1) and never reads the snapshot
    // again. The lock is released before the safety snapshot takes it.
    let (manifest, payloads, scope) = {
        let _lock = match lock_cas(&cas, Some(CAS_LOCK_WAIT)) {
            Ok(Some(lock)) => lock,
            Ok(None) => return Err(BUSY.into()),
            Err(error) => return Err(format!("backup-failed:lock:{:?}", error.kind())),
        };
        let src = [cas.join(CAS_SNAPSHOTS).join(stamp), base.join(stamp)]
            .into_iter()
            .find(|dir| dir.is_dir())
            .ok_or("backup not found")?;
        let manifest = read_manifest(&src).ok_or(DAMAGED)?;
        if manifest.root != source.root.display().to_string() {
            return Err("backup belongs to a different graph".into());
        }
        let payloads = load_verified_payloads(&src, &cas, &manifest).ok_or(DAMAGED)?;
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
        // Schema 3/4 text sits at its graph-relative path, so the recorded
        // pages and journals directories do not steer it (master 336833b13);
        // `hidden` is the graph-text scope it covered. `[""]` hides all:
        // master's fail-closed `:hidden` snapshot holds no text, so it must
        // retire none.
        let scope = match (&manifest.graph_text_policy, manifest.schema) {
            (Some(policy), GRAPH_COPY_SNAPSHOT_SCHEMA | SNAPSHOT_SCHEMA)
                if policy.hidden_parse_failed_closed =>
            {
                Some(vec![String::new()])
            }
            (Some(policy), GRAPH_COPY_SNAPSHOT_SCHEMA | SNAPSHOT_SCHEMA) => {
                Some(policy.hidden.clone())
            }
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
        // A missing full-copy area would read as "no files" and retire the
        // live ones. A schema-4 snapshot has no areas on disk: its manifest
        // is its only listing, and its checksum matched.
        if manifest.schema != SNAPSHOT_SCHEMA && areas.iter().any(|area| !src.join(area).is_dir()) {
            return Err(DAMAGED.into());
        }
        (manifest, payloads, scope)
    };
    let selected = select_restore_files(&manifest, &source, scope.is_some())?;
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
    let files = selected
        .into_iter()
        .map(|(area, rel, index)| {
            let bytes = &payloads[index];
            let source = stage_restore_bytes(&cas, bytes)
                .map_err(|error| format!("restore-failed:{:?}: stage", error.kind()))?;
            Ok(RestoreFile {
                area,
                rel,
                source,
                len: bytes.len() as u64,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    #[cfg(test)]
    if let Some(hook) = BEFORE_STORE_RESTORE.with(|hook| hook.take()) {
        hook();
    }
    store
        .restore(tine_store::EditKind::ReplacePage, files, scope.as_deref())
        .map_err(|error| format_restore_failure(&error))?;
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    /// Runs once between staging and `Store::restore`.
    static BEFORE_STORE_RESTORE: std::cell::Cell<Option<Box<dyn FnOnce()>>> = const { std::cell::Cell::new(None) };
}

/// Every refusal of a snapshot whose own bytes are missing, torn or
/// unverifiable, its manifest checksum included (power loss, a disk error).
/// Nothing in the graph has changed. Refusal row: docs/storage-contract.md,
/// `src-tauri::backup` restore selection.
const DAMAGED: &str = "this backup is damaged; pick another snapshot";

/// A restore that waited its bound for another Tine process's backup work
/// on this graph (concurrent honest instances). Nothing in the graph has
/// changed. Refusal row: docs/storage-contract.md, `src-tauri::backup`
/// namespace lock.
const BUSY: &str =
    "backups are busy in another Tine window or process; try the restore again in a moment";

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

/// The verified files a restore writes, as (area, area-relative path,
/// index into the manifest's files), in the old area order: graph text (or
/// journals, pages), asset sidecars, config. An unsafe path refuses the
/// restore before the safety snapshot.
fn select_restore_files(
    manifest: &SnapshotManifest,
    source: &BackupSource,
    graph_wide: bool,
) -> Result<Vec<(Area, String, usize)>, String> {
    let mut files = Vec::new();
    let text: &[(&str, Area)] = if graph_wide {
        &[("graph", Area::Graph)]
    } else {
        &[("journals", Area::Journals), ("pages", Area::Pages)]
    };
    for &(prefix, area) in text.iter().chain(&[
        (source.assets_dir_name.as_str(), Area::Assets),
        ("logseq", Area::Meta),
    ]) {
        for (index, entry) in manifest.files.iter().enumerate() {
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
            files.push((area, rel.to_owned(), index));
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
        let mut files = Vec::new();
        let (copied, failed, failure) = copy_store_area(
            &store,
            Area::Pages,
            &root.join("backup-out"),
            "pages",
            is_graph_text,
            &mut files,
            &|| false,
        );
        assert_eq!((copied, failed), (0, 3));
        assert!(require_safety_snapshot(
            BackupOutcome {
                copied,
                failure,
                published: None,
            },
            2,
        )
        .is_err());
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
            false,
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
        write_manifest(&snapshot, &manifest, false).unwrap();
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
            false,
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
        let outcome = write_snapshot(&base, &store, source.clone(), "", &|| false);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let stamp = outcome.published.clone().unwrap();
        let manifest = read_manifest(&cas_dir(&base).join(CAS_SNAPSHOTS).join(&stamp)).unwrap();
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
        let stamp = outcome.published.clone().unwrap();
        let manifest = read_manifest(&cas_dir(&base).join(CAS_SNAPSHOTS).join(&stamp)).unwrap();
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
        let blobs = root.join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let mut files = Vec::new();
        let (copied, failed, failure) = copy_store_area(
            &store,
            Area::Graph,
            &blobs,
            "graph",
            is_graph_text,
            &mut files,
            &|| false,
        );
        assert_eq!((copied, failed), (3, 0), "{failure:?}");
        let mut got: Vec<String> = files
            .into_iter()
            .map(|file| file.path["graph/".len()..].to_owned())
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

    /// Every file under `dir` with its bytes, for byte-identity checks.
    fn tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current).unwrap().flatten() {
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    stack.push(path);
                } else {
                    let rel = path
                        .strip_prefix(dir)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    /// og-backup-cas: a schema-4 snapshot restores byte-identical, and a
    /// snapshot with a corrupted blob, a missing blob or a torn manifest
    /// (power loss, a disk error) is refused before the safety snapshot and
    /// before any graph write.
    #[test]
    fn schema_4_restore_is_byte_identical_and_damage_is_refused() {
        let root = scratch("schema-4-round-trip");
        let graph = root.join("graph");
        let base = root.join("backups");
        whole_graph(&graph);
        write(&graph.join("pages/Crlf.md"), "- one\r\n- dvě ✓\r\n");
        write(&graph.join("pages/Same.md"), "- a\n");
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = write_snapshot(&base, &store, source.clone(), "", &|| false);
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let stamp = outcome.published.clone().unwrap();
        let snapshot = cas_dir(&base).join(CAS_SNAPSHOTS).join(&stamp);
        let manifest = read_manifest(&snapshot).unwrap();
        assert_eq!(manifest.schema, SNAPSHOT_SCHEMA);
        let original = tree(&graph);

        write(&graph.join("pages/Crlf.md"), "- changed\n");
        write(&graph.join("pages/A.md"), "- changed\n");
        std::fs::remove_file(graph.join("Root.md")).unwrap();
        restore_from_backup_source(&stamp, &base, &store, source.clone(), |_| {
            BackupOutcome::success(1)
        })
        .unwrap();
        let restored = tree(&graph);
        for (rel, bytes) in &original {
            assert_eq!(
                restored.get(rel),
                Some(bytes),
                "{rel} restores byte-identical"
            );
        }

        let blob = |path: &str| {
            let file = manifest
                .files
                .iter()
                .find(|file| file.path == path)
                .unwrap();
            cas_dir(&base).join(BLOB_DIR).join(&file.sha256)
        };
        let corrupt = blob("graph/pages/Crlf.md");
        let missing = blob("graph/Root.md");
        let saved = (
            std::fs::read(&corrupt).unwrap(),
            std::fs::read(&missing).unwrap(),
        );
        write(&graph.join("pages/Crlf.md"), "- live\n");
        let live = tree(&graph);
        let refused = |label: &str| {
            let result = restore_from_backup_source(&stamp, &base, &store, source.clone(), |_| {
                panic!("{label}: a damaged backup must be refused before the safety snapshot")
            });
            assert_eq!(result.unwrap_err(), DAMAGED, "{label}");
            assert_eq!(tree(&graph), live, "{label}: nothing written to the graph");
        };
        let mut flipped = saved.0.clone();
        flipped[0] ^= 1;
        std::fs::write(&corrupt, &flipped).unwrap();
        refused("corrupted blob");
        std::fs::write(&corrupt, &saved.0).unwrap();
        std::fs::remove_file(&missing).unwrap();
        refused("missing blob");
        std::fs::write(&missing, &saved.1).unwrap();
        let manifest_bytes = std::fs::read(snapshot.join(SNAPSHOT_MANIFEST)).unwrap();
        std::fs::write(
            snapshot.join(SNAPSHOT_MANIFEST),
            &manifest_bytes[..manifest_bytes.len() / 2],
        )
        .unwrap();
        refused("torn manifest");
        std::fs::write(
            snapshot.join(SNAPSHOT_MANIFEST),
            vec![0u8; manifest_bytes.len()],
        )
        .unwrap();
        refused("zero-filled manifest");
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A full-copy schema-3 snapshot exactly as og wrote it before
    /// og-backup-cas (payload under `graph/`, the assets folder and
    /// `logseq/`, inventory-hashed manifest, `writer: "og"`) still lists and
    /// restores beside schema-4 snapshots, and the keep-count prunes both
    /// kinds alike without touching the blob store.
    #[test]
    fn og_full_copy_snapshot_restores_and_prunes_beside_schema_4() {
        let root = scratch("og-full-copy-compat");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph_with_root(&graph);
        let old = base.join("2026-09-01_00-00-00");
        write(&old.join("graph/pages/A.md"), "- old a\n");
        write(&old.join("graph/Root.md"), "- old root\n");
        write(&old.join("assets/doc.edn"), "{:old 1}\n");
        write(&old.join("logseq/config.edn"), "{}\n");
        let canonical = std::fs::canonicalize(&graph).unwrap().display().to_string();
        write_manifest(
            &old,
            &SnapshotManifest {
                schema: GRAPH_COPY_SNAPSHOT_SCHEMA,
                root: canonical,
                journals_dir: "journals".into(),
                pages_dir: "pages".into(),
                graph_text_policy: Some(SnapshotGraphTextPolicy {
                    version: GRAPH_TEXT_SCOPE_VERSION,
                    hidden: Vec::new(),
                    hidden_parse_failed_closed: false,
                }),
                writer: Some(SNAPSHOT_WRITER.into()),
                files: snapshot_inventory(&old).unwrap(),
                complete: true,
            },
            false,
        )
        .unwrap();
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        assert!(write_snapshot(&base, &store, source.clone(), "", &|| false)
            .failure
            .is_none());
        assert_eq!(list_backups_from_base(&base, &graph).len(), 2);
        restore_from_backup_source("2026-09-01_00-00-00", &base, &store, source, |_| {
            BackupOutcome::success(1)
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(graph.join("pages/A.md")).unwrap(),
            "- old a\n"
        );
        assert_eq!(
            std::fs::read_to_string(graph.join("Root.md")).unwrap(),
            "- old root\n"
        );
        assert!(!graph.join("pages/B.md").exists(), "retired into recovery");
        prune_backups(&base, 1);
        let left: Vec<_> = list_backups_from_base(&base, &graph)
            .into_iter()
            .map(|info| info.stamp)
            .collect();
        assert_eq!(left.len(), 1);
        assert_ne!(
            left[0], "2026-09-01_00-00-00",
            "the older full copy is pruned"
        );
        assert!(cas_dir(&base).join(BLOB_DIR).is_dir());
        let newest = cas_dir(&base).join(CAS_SNAPSHOTS).join(&left[0]);
        assert!(verify_snapshot(&newest, &read_manifest(&newest).unwrap()));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// REVIEW-backup-cas B1: one flipped bit in a listed path (`A` to `C`)
    /// leaves valid JSON and a blob that hashes right, and would restore A's
    /// bytes as C.md and retire A.md. The manifest checksum makes it damage:
    /// not listed, and refused before the safety snapshot. A schema-4
    /// manifest without its checksum is damaged too.
    #[test]
    fn a_bit_flip_in_a_manifest_path_is_refused() {
        let root = scratch("manifest-bit-flip");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph_with_root(&graph);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = write_snapshot(&base, &store, source.clone(), "", &|| false);
        let stamp = outcome.published.unwrap();
        let path = cas_dir(&base)
            .join(CAS_SNAPSHOTS)
            .join(&stamp)
            .join(SNAPSHOT_MANIFEST);
        let saved = std::fs::read(&path).unwrap();
        let at = saved
            .windows(b"graph/pages/A.md".len())
            .position(|window| window == b"graph/pages/A.md")
            .unwrap()
            + b"graph/pages/".len();
        let mut flipped = saved.clone();
        flipped[at] ^= 0b10;
        assert_eq!(flipped[at], b'C');
        assert!(serde_json::from_slice::<serde_json::Value>(&flipped).is_ok());
        let live = tree(&graph);
        let refused = |label: &str| {
            assert!(
                list_backups_from_base(&base, &graph).is_empty(),
                "{label}: not listed"
            );
            let result = restore_from_backup_source(&stamp, &base, &store, source.clone(), |_| {
                panic!("{label}: a damaged manifest must be refused before the safety snapshot")
            });
            assert_eq!(result.unwrap_err(), DAMAGED, "{label}");
            assert_eq!(tree(&graph), live, "{label}: nothing written to the graph");
        };
        std::fs::write(&path, &flipped).unwrap();
        refused("bit flip in a path");
        let mut unchecked: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        unchecked.as_object_mut().unwrap().remove(MANIFEST_CHECKSUM);
        std::fs::write(&path, serde_json::to_vec_pretty(&unchecked).unwrap()).unwrap();
        refused("no checksum");
        std::fs::write(&path, &saved).unwrap();
        assert_eq!(list_backups_from_base(&base, &graph).len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    /// REVIEW N1: restore writes the bytes it verified. A blob changed in
    /// place (same length, so no length check sees it) after verification
    /// and right before `Store::restore` copies never reaches the graph.
    #[test]
    fn a_blob_changed_after_verification_never_reaches_the_graph() {
        let root = scratch("restore-verified-bytes");
        let graph = root.join("graph");
        let base = root.join("backups");
        small_graph_with_root(&graph);
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let outcome = write_snapshot(&base, &store, source.clone(), "", &|| false);
        let stamp = outcome.published.unwrap();
        let manifest = read_manifest(&cas_dir(&base).join(CAS_SNAPSHOTS).join(&stamp)).unwrap();
        let sha256 = &manifest
            .files
            .iter()
            .find(|file| file.path == "graph/pages/A.md")
            .unwrap()
            .sha256;
        let blob = cas_dir(&base).join(BLOB_DIR).join(sha256);
        write(&graph.join("pages/A.md"), "- live\n");
        let changed = blob.clone();
        BEFORE_STORE_RESTORE.with(|hook| {
            hook.set(Some(Box::new(move || {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&changed)
                    .unwrap();
                file.write_all(b"- X\n").unwrap();
            })))
        });
        restore_from_backup_source(&stamp, &base, &store, source, |_| BackupOutcome::success(1))
            .unwrap();
        assert_eq!(std::fs::read(&blob).unwrap(), b"- X\n", "the blob changed");
        assert_eq!(
            std::fs::read_to_string(graph.join("pages/A.md")).unwrap(),
            "- a\n",
            "the graph holds the verified bytes"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    fn small_graph_with_root(graph: &std::path::Path) {
        write(&graph.join("pages/A.md"), "- a\n");
        write(&graph.join("pages/B.md"), "- b\n");
        write(&graph.join("Root.md"), "- root\n");
        write(&graph.join("logseq/config.edn"), "{}\n");
        write(&graph.join("assets/doc.edn"), "{:a 1}\n");
        std::fs::create_dir_all(graph.join("journals")).unwrap();
    }
}
