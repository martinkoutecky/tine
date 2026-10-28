//! One git sync cycle (ADR 0078 §3–§5, slice 2 on desktop): fetch, then the
//! §4 fast-forward, then push, on the binding's git thread (the autocommit
//! worker's; no new thread, no async worker). Each step takes the runs' lock
//! itself ([`super::git_run::RUN_LOCK`]), so an autocommit, a run commit or a
//! structural bracket never falls inside one, and the worker never runs two at
//! once. A cycle changes the graph only through the pull ([`git_pull::pull`]).
//!
//! When nothing changed on either side a cycle costs one fetch negotiation
//! (the remote's ref advertisement; no pack, no push). Push runs only when the
//! fetch found local commits the remote lacks, and never forces.
//!
//! Each outcome is one refusal-table row (ADR §4) that Settings and the
//! app's notice name. Rows that need the user (diverged, sign-in, a refused
//! path or configuration) are shown at once; rows that may pass by
//! themselves (held, busy, network, incomplete) after [`PERSIST_AFTER`]
//! cycles in a row. Failures back off; held and busy retry next cycle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::Serialize;

use super::git_autocommit::Unusable;
use super::git_pull::{self, Held, PullOutcome};
use super::git_sync::{Credentials, Operation, Outcome as Net, TransportOptions};
use crate::state::GraphSlot;

/// Cycles in a row a self-clearing problem lasts before the app shows it
/// outside Settings.
pub(crate) const PERSIST_AFTER: u32 = 3;
/// The longest an automatic retry waits after repeated failures.
const BACKOFF_CAP: Duration = Duration::from_secs(60 * 60);

/// How a cycle ended, for scheduling and the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Ok,
    /// Held, busy or incomplete: retried at the next cycle.
    Held,
    /// The remote could not be reached: backs off.
    Network,
    /// The user must act (diverged, sign in, a refused path or setup).
    NeedsUser,
    /// Tine is quitting; nothing is recorded.
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cycle {
    pub(crate) class: Class,
    /// The refusal-table row: `up-to-date`, `pulled`, `pushed`, `diverged`,
    /// `auth-required`, `network`, `pull-held-unsaved`, `pull-held-changed`,
    /// `pull-held-busy`, `busy`, `pull-incomplete`, `pull-refused`,
    /// `no-upstream`, `refused`.
    pub(crate) code: &'static str,
    pub(crate) message: String,
    /// The file a held or refused pull names.
    pub(crate) path: Option<String>,
}

impl Cycle {
    /// Network and needs-the-user rows wait out their back-off even when a
    /// new autocommit would otherwise sync at once.
    pub(crate) fn backs_off(&self) -> bool {
        matches!(self.class, Class::Network | Class::NeedsUser)
    }
}

fn cycle_of(class: Class, code: &'static str, message: impl Into<String>) -> Cycle {
    Cycle {
        class,
        code,
        message: message.into(),
        path: None,
    }
}

fn ok(code: &'static str) -> Cycle {
    cycle_of(Class::Ok, code, "")
}

// ---- quitting ----------------------------------------------------------------

static STOPPED: AtomicBool = AtomicBool::new(false);

fn cancel() -> Arc<AtomicBool> {
    static CANCEL: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    CANCEL
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

/// The app is quitting: no new network step starts, and one in flight is
/// cancelled at its next transfer callback (a cancelled push may already
/// have reached the server; the next start fetches first). The exit flush
/// then waits only for the local quit autocommit.
pub(crate) fn stop_network() {
    STOPPED.store(true, Ordering::Release);
    cancel().store(true, Ordering::Release);
}

fn stopped() -> bool {
    STOPPED.load(Ordering::Acquire)
}

// ---- the cycle ------------------------------------------------------------------

fn operation(credentials: &Arc<dyn Credentials>) -> Result<Operation, Net> {
    Operation::new(credentials.clone(), cancel(), &TransportOptions::default())
}

/// Fetch, fast-forward, push. The pull's own pre-pull autocommit (ADR §3)
/// runs inside [`git_pull::pull`].
pub(crate) fn cycle(slot: &GraphSlot, credentials: &Arc<dyn Credentials>) -> Cycle {
    let root = &slot.root_key;
    if stopped() {
        return cycle_of(Class::Quit, "cancelled", "");
    }
    let (ahead, behind) = match operation(credentials).map(|op| op.fetch(root)) {
        Ok(Net::UpToDate) => return ok("up-to-date"),
        Ok(Net::Fetched { ahead, behind, .. }) => (ahead, behind),
        Ok(other) | Err(other) => return network_outcome(other),
    };
    if behind > 0 {
        match git_pull::pull(slot) {
            PullOutcome::Landed { from, to, .. } => tell_window(slot, from, to),
            PullOutcome::UpToDate => {}
            other => return pull_outcome(other),
        }
        // A fetch that saw both sides move answers `Diverged`, so a branch
        // that was behind has nothing of its own to push.
        if ahead == 0 {
            return ok("pulled");
        }
    }
    if ahead == 0 {
        return ok("up-to-date");
    }
    if stopped() {
        return cycle_of(Class::Quit, "cancelled", "");
    }
    match operation(credentials).map(|op| op.push_upstream(root)) {
        Ok(Net::UpToDate) => ok("pushed"),
        Ok(other) | Err(other) => network_outcome(other),
    }
}

/// A landed pull's pages, told to the window as the watcher tells it of
/// outside edits. The host publishes the pull as its own write, which mails
/// only the pages it holds; a page the window just displays reloads on this
/// event, as for a command-line write (`owner::window`). Sync runs only with
/// the graph root as the work tree root, so git's paths are page ids. A
/// path that is no page (an asset, `config.edn`) reaches the window through
/// the watcher, as the pull wrote it as an outside change.
fn tell_window(slot: &GraphSlot, from: git2::Oid, to: git2::Oid) {
    let Some(link) = slot.window.get() else {
        return;
    };
    let Ok(repo) = git2::Repository::open(&slot.root_key) else {
        return;
    };
    let tree = |oid| repo.find_commit(oid).and_then(|c| c.tree());
    let (Ok(old), Ok(new)) = (tree(from), tree(to)) else {
        return;
    };
    let Ok(diff) = repo.diff_tree_to_tree(Some(&old), Some(&new), None) else {
        return;
    };
    let mut pages = Vec::new();
    for delta in diff.deltas() {
        let removed = delta.status() == git2::Delta::Deleted;
        let file = if removed {
            delta.old_file()
        } else {
            delta.new_file()
        };
        let Some(path) = file.path().and_then(|p| p.to_str()) else {
            continue;
        };
        let id = tine_store::PageId::from(path.to_string());
        let read = if removed {
            repo.find_blob(file.id())
                .ok()
                .and_then(|blob| slot.store.page_of(&id, blob.content()).ok())
        } else {
            slot.store.page(&id).ok()
        };
        if let Some(read) = read {
            pages.push(crate::watcher::GraphChange {
                path: path.to_string(),
                name: read.doc.name,
                kind: read.doc.kind,
                created: delta.status() == git2::Delta::Added,
                removed,
            });
        }
    }
    link.pages_changed(pages);
}

fn network_outcome(outcome: Net) -> Cycle {
    match outcome {
        Net::UpToDate => ok("up-to-date"),
        Net::Fetched { .. } => ok("up-to-date"),
        Net::Diverged => cycle_of(
            Class::NeedsUser,
            "diverged",
            "this device and the remote both have new commits",
        ),
        Net::AuthRequired => cycle_of(Class::NeedsUser, "auth-required", ""),
        Net::Network(message) => cycle_of(Class::Network, "network", message),
        Net::Refused(message) => cycle_of(Class::NeedsUser, "refused", message),
        Net::Busy => cycle_of(Class::Held, "busy", "index.lock"),
        Net::Cancelled => cycle_of(Class::Quit, "cancelled", ""),
    }
}

fn pull_outcome(outcome: PullOutcome) -> Cycle {
    // The pull names its own refusal-table rows; `landed` reads `pulled` here.
    let code = outcome.code();
    let held = |message: &str, path: Option<String>| Cycle {
        class: Class::Held,
        code,
        message: message.to_string(),
        path,
    };
    match outcome {
        PullOutcome::Landed { .. } => ok("pulled"),
        PullOutcome::UpToDate => ok(code),
        PullOutcome::Diverged(message) => cycle_of(Class::NeedsUser, code, message),
        PullOutcome::Held(Held::Unsaved(page)) => held("", Some(page)),
        PullOutcome::Held(Held::Changed(path)) => held("", Some(path).filter(|p| !p.is_empty())),
        PullOutcome::Held(Held::Busy(what)) => held(&what, None),
        PullOutcome::Incomplete(why) => held(&why, None),
        PullOutcome::Refused { path, reason } => Cycle {
            class: Class::NeedsUser,
            code,
            message: reason,
            path: Some(path),
        },
        // Files were already updated: the named path needs the user in git, then a Sync.
        PullOutcome::StoppedPartway { path, reason } => Cycle {
            class: Class::NeedsUser,
            code,
            message: reason,
            path: Some(path),
        },
        PullOutcome::NoUpstream => cycle_of(Class::NeedsUser, code, ""),
        PullOutcome::Unusable(Unusable::Busy) => Cycle {
            code: "busy",
            ..held("index.lock", None)
        },
        PullOutcome::Unusable(Unusable::Quitting) => cycle_of(Class::Quit, "cancelled", ""),
        PullOutcome::Unusable(error) => cycle_of(Class::NeedsUser, code, error.message()),
    }
}

// ---- what Settings and the notice show ------------------------------------------

/// The last sync cycle, as Settings and the app's notice show it.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct SyncStatus {
    /// A cycle is running now.
    pub(crate) running: bool,
    /// Unix seconds when the last cycle finished.
    pub(crate) at: Option<i64>,
    /// Its row ([`Cycle::code`]) and message, and the path it names.
    pub(crate) code: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) path: Option<String>,
    /// Unix seconds of the last cycle that succeeded.
    pub(crate) ok_at: Option<i64>,
    /// Failed cycles in a row.
    pub(crate) failures: u32,
    /// The problem should show outside Settings too.
    pub(crate) persistent: bool,
}

impl SyncStatus {
    /// Record a finished cycle; answers when the next automatic one is due.
    pub(crate) fn record(&mut self, cycle: &Cycle, interval: Duration) -> Duration {
        let now = chrono::Utc::now().timestamp();
        self.running = false;
        if cycle.class == Class::Quit {
            return interval;
        }
        self.at = Some(now);
        self.code = Some(cycle.code.to_string());
        self.message = Some(cycle.message.clone()).filter(|m| !m.is_empty());
        self.path = cycle.path.clone();
        if cycle.class == Class::Ok {
            self.ok_at = Some(now);
            self.failures = 0;
            self.persistent = false;
            return interval;
        }
        self.failures = self.failures.saturating_add(1);
        self.persistent = cycle.class == Class::NeedsUser || self.failures >= PERSIST_AFTER;
        if cycle.class == Class::Held {
            return interval;
        }
        let doubled = interval.saturating_mul(1 << (self.failures - 1).min(6));
        doubled.min(BACKOFF_CAP.max(interval))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_user_rows_persist_at_once_and_others_after_three_failures_with_backoff() {
        let minute = Duration::from_secs(60);
        let interval = 10 * minute;
        let mut status = SyncStatus::default();
        let net = cycle_of(Class::Network, "network", "HTTP request timed out");
        assert_eq!(status.record(&net, interval), interval);
        assert!(!status.persistent);
        assert_eq!(status.record(&net, interval), 20 * minute);
        assert_eq!(status.record(&net, interval), 40 * minute);
        assert!(
            status.persistent,
            "a third failure in a row shows outside Settings"
        );
        assert_eq!(
            status.record(&net, interval),
            60 * minute,
            "capped at an hour"
        );
        assert_eq!(status.record(&ok("up-to-date"), interval), interval);
        assert!(!status.persistent);
        assert_eq!((status.failures, status.message.as_deref()), (0, None));

        let diverged = cycle_of(Class::NeedsUser, "diverged", "both moved");
        assert_eq!(status.record(&diverged, interval), interval);
        assert!(status.persistent, "diverged needs the user at once");
        assert_eq!(
            status.record(&diverged, interval),
            20 * minute,
            "no retry loop"
        );

        let mut held = SyncStatus::default();
        let unsaved = Cycle {
            class: Class::Held,
            code: "pull-held-unsaved",
            message: String::new(),
            path: Some("pages/a.md".into()),
        };
        for _ in 0..3 {
            assert_eq!(
                held.record(&unsaved, interval),
                interval,
                "held retries next cycle"
            );
        }
        assert!(held.persistent);
        assert_eq!(held.path.as_deref(), Some("pages/a.md"));
        let before = held.clone();
        held.record(&cycle_of(Class::Quit, "cancelled", ""), interval);
        assert_eq!(held, before, "quitting records nothing");
    }

    #[test]
    fn every_pull_and_transport_outcome_names_its_row() {
        let rows = [
            (
                pull_outcome(PullOutcome::Diverged("x".into())).code,
                "diverged",
            ),
            (
                pull_outcome(PullOutcome::Held(Held::Changed(String::new()))).code,
                "pull-held-changed",
            ),
            (
                pull_outcome(PullOutcome::Held(Held::Busy("gc.pid".into()))).code,
                "pull-held-busy",
            ),
            (
                pull_outcome(PullOutcome::Incomplete("disk".into())).code,
                "pull-incomplete",
            ),
            (pull_outcome(PullOutcome::NoUpstream).code, "no-upstream"),
            (
                pull_outcome(PullOutcome::Unusable(Unusable::Busy)).code,
                "busy",
            ),
            (
                pull_outcome(PullOutcome::Unusable(Unusable::Sparse)).code,
                "refused",
            ),
            (network_outcome(Net::AuthRequired).code, "auth-required"),
            (network_outcome(Net::Diverged).code, "diverged"),
            (network_outcome(Net::Busy).code, "busy"),
            (network_outcome(Net::Network("x".into())).code, "network"),
            (network_outcome(Net::Refused("x".into())).code, "refused"),
        ];
        for (got, want) in rows {
            assert_eq!(got, want);
        }
        let refused = pull_outcome(PullOutcome::Refused {
            path: "pages/link.md".into(),
            reason: "a symbolic link".into(),
        });
        assert_eq!(
            (
                refused.class,
                refused.path.as_deref(),
                refused.message.as_str()
            ),
            (Class::NeedsUser, Some("pages/link.md"), "a symbolic link")
        );
        let stopped = pull_outcome(PullOutcome::StoppedPartway {
            path: "pages/b.md".into(),
            reason: "a staged change".into(),
        });
        assert_eq!(
            (stopped.class, stopped.code, stopped.path.as_deref()),
            (Class::NeedsUser, "pull-stopped", Some("pages/b.md"))
        );
        assert_eq!(network_outcome(Net::Cancelled).class, Class::Quit);
        assert_eq!(
            pull_outcome(PullOutcome::Unusable(Unusable::Quitting)).class,
            Class::Quit
        );
    }
}
