//! OS-watch installation and event/poll scheduling.

use super::*;

/// The watcher's idle cycle: poll interval, and the longest a notify-mode
/// watcher sleeps before re-checking its watch and external assets.
const CYCLE: Duration = Duration::from_secs(3);

/// Storage spec §5.4: a launch diff that left racy paths runs one follow-up
/// full diff this long after it, by when every stamp it saw is outside the
/// racy window and the follow-up settles it.
pub(crate) const RACY_FOLLOW_UP: Duration = RACY_WINDOW.saturating_add(Duration::from_millis(100));

pub(super) fn run(
    core: Arc<Core>,
    mode: Arc<Mutex<WatchMode>>,
    wake: Sender<()>,
    rx: Receiver<()>,
) {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let mut watcher: Option<notify::RecommendedWatcher> = None;
    let mut active = None;
    let mut active_dirs: Option<[PathBuf; 1]> = None;
    let mut active_dir_ids: Option<[Option<u128>; 1]> = None;
    let mut assets_watched = false;
    let mut assets_failure: Option<String> = None;
    while !core.closed.load(Ordering::Acquire) {
        let selected = *mode.lock().unwrap();
        let dirs = core.dirs.read().unwrap().clone();
        let dir_ids = [directory_identity(&dirs[0])];
        // A refused watch is retried every cycle: polling covers the gap (I-9:
        // the refusal was reported, so it is never silent staleness).
        let retry = selected == WatchMode::Notify && watcher.is_none();
        if retry
            || active != Some(selected)
            || active_dirs.as_ref() != Some(&dirs)
            || active_dir_ids.as_ref() != Some(&dir_ids)
        {
            let retrying = retry && active == Some(selected);
            watcher = None;
            assets_watched = false;
            active = Some(selected);
            active_dirs = Some(dirs.clone());
            active_dir_ids = Some(dir_ids);
            if selected == WatchMode::Notify {
                match install_watch(&core, &dirs, &pending, &wake) {
                    Ok(created) => {
                        watcher = Some(created);
                        // The asset baseline predates registration. An edit in
                        // that gap has no event; retain the scan until Ready.
                        pending.lock().unwrap().assets.scan_after_install();
                        let _ = wake.send(());
                        core.set_refusal(None, true);
                    }
                    Err(message) => core.set_refusal(Some(message), true),
                }
            } else {
                core.set_refusal(None, false);
            }
            if core.ready() && (watcher.is_some() || !retrying) {
                // A recreated root, or a watch installed after polling, can
                // already contain files (or a config.edn) it never reported.
                // Reconcile once, config included.
                let _ = core.reconcile(None, true, false, DiffTrigger::WatchInstall);
            }
        }
        let external_assets = core
            .assets
            .scope()
            .external_root(&dirs[0])
            .map(Path::to_path_buf);
        let assets_polled = match (watcher.as_mut(), external_assets) {
            (Some(live), Some(root)) if !assets_watched => {
                assets_watched = watch_external_assets(&core, live, &root, &mut assets_failure);
                !assets_watched
            }
            _ => false,
        };
        if watcher.is_some() {
            match rx.recv_timeout(core.follow_up_wait(CYCLE)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    core.follow_up_if_due();
                    if assets_polled && core.ready() {
                        core.observe_assets(&HashSet::new(), true);
                    }
                    continue;
                }
                Ok(()) => {}
            }
            std::thread::sleep(Duration::from_millis(200));
            while rx.try_recv().is_ok() {}
            if core.closed.load(Ordering::Acquire) {
                break;
            }
            if !core.ready() {
                continue;
            }
            core.follow_up_if_due();
            #[cfg(test)]
            if core.deaf.load(Ordering::Acquire) {
                *pending.lock().unwrap() = Pending::default();
                continue;
            }
            let (paths, full, config, first_event_at, assets) = {
                let mut pending = pending.lock().unwrap();
                let config = std::mem::take(&mut pending.config);
                let (paths, full, first_event_at) = pending.drain();
                (paths, full, config, first_event_at, pending.assets.drain())
            };
            // A rescan or unusable event may hide a config write: re-check it.
            #[cfg(test)]
            if full {
                core.event_full_diffs.fetch_add(1, Ordering::Relaxed);
            }
            if full || config || !paths.is_empty() {
                let batch = WatchBatch {
                    first_event_at,
                    reconcile_started: Instant::now(),
                    poll: false,
                    full_diff: full,
                    event_paths: paths.len(),
                };
                let _ = core.reconcile_batch(
                    if full { None } else { Some(&paths) },
                    full || config,
                    batch,
                );
            }
            // A rescan/unusable event hides asset changes too: scan them.
            if full || assets.1 || assets_polled || !assets.0.is_empty() {
                core.observe_assets(&assets.0, full || assets.1 || assets_polled);
            }
        } else {
            // Poll mode has no event paths: every cycle re-checks the config
            // (one stat and one hash of a small file beside the full stat scan).
            let _ = rx.recv_timeout(core.follow_up_wait(CYCLE));
            if core.ready() && !core.closed.load(Ordering::Acquire) {
                // This cycle's full diff is the racy follow-up (§5.4).
                core.follow_up.lock().unwrap().take();
                let batch = WatchBatch {
                    first_event_at: None,
                    reconcile_started: Instant::now(),
                    poll: true,
                    full_diff: true,
                    event_paths: 0,
                };
                let _ = core.reconcile_batch(None, true, batch);
                core.observe_assets(&HashSet::new(), true);
            }
        }
    }
}
