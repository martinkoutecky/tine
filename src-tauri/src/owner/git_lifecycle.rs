//! The app's visibility edges on Android and iOS (ADR 0078 §3, slice 3):
//! the frontend reports each edge after flushing the pages, and every
//! running git worker, or one that starts while the app is away, follows it.

use super::*;

/// The app went to the background (ADR 0078 §3, slice 3; Android and iOS):
/// every worker commits its pending changes at once, unless it committed
/// less than an interval ago (the cap on autocommits, D-20261026-GC), and,
/// when it syncs automatically, pushes the commits its last fetch shows the
/// remote lacks. Until [`foregrounded`] no automatic cycle starts, so no
/// fetch and no pull start in the background, also in a worker that starts
/// meanwhile: a push writes nothing to this repository (`index.lock` is held
/// only for its local checks, not the transfer), so the OS freezing or
/// ending the app mid-push leaves it as it was, and the commit is the one
/// short local write.
pub(crate) fn backgrounded() {
    lifecycle_edge(false, None);
}

/// The app came back to the foreground (slice 3): every worker may sync
/// again, and a graph that syncs automatically runs a cycle at once. Unlike
/// "Sync now" it does not force a commit; pending edits commit on the
/// interval's cadence, except the pre-pull autocommit when the remote has
/// news (ADR 0078 §3), which follows the other device's pushes.
pub(crate) fn foregrounded() {
    lifecycle_edge(true, None);
}

/// The app's visibility as the last edge left it: a worker that starts
/// while the app is away (a bind ending after the person switched apps)
/// starts away (review r2 B1). Set before the running workers are told, and
/// read by a starting worker under its state lock, so no worker misses it.
static APP_AWAY: AtomicBool = AtomicBool::new(false);
/// Tests: the roots per-root edges left away, so parallel tests stay apart.
#[cfg(test)]
static AWAY_AT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub(super) fn app_away(root: &Path) -> bool {
    #[cfg(test)]
    if AWAY_AT.lock().unwrap().iter().any(|away| away == root) {
        return true;
    }
    let _ = root;
    APP_AWAY.load(Ordering::SeqCst)
}

/// One visibility edge for every running worker, or (tests) the one at `only`.
fn lifecycle_edge(foreground: bool, only: Option<&Path>) {
    match only {
        None => APP_AWAY.store(!foreground, Ordering::SeqCst),
        #[cfg(test)]
        Some(root) => {
            let mut away = AWAY_AT.lock().unwrap();
            away.retain(|at| at != root);
            if !foreground {
                away.push(root.to_path_buf());
            }
        }
        #[cfg(not(test))]
        Some(_) => {}
    }
    for worker in registry::all() {
        if only.is_some_and(|root| root != worker.root) {
            continue;
        }
        let mut state = worker.state.lock().unwrap();
        if !state.running {
            continue;
        }
        state.away = !foreground;
        if !foreground && state.settings.policy.commits() {
            state.background = true;
        }
        if foreground && state.settings.policy == Policy::SyncAutomatically {
            state.foreground = true;
            state.last.sync.running = true;
        }
        worker.wake.notify_all();
    }
}

#[cfg(test)]
pub(super) fn backgrounded_at(root: &Path) {
    lifecycle_edge(false, Some(root));
}

#[cfg(test)]
pub(super) fn foregrounded_at(root: &Path) {
    lifecycle_edge(true, Some(root));
}
