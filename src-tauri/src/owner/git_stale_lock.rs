//! A stale `index.lock` the person can remove with one click (ADR 0078 §1,
//! slice 3; approved D-20261029-GSL).
//!
//! When the OS ends Tine during a commit, the empty `index.lock` Tine's own
//! commit holds ([`super::git_run::IndexLock`]) stays behind, and every later
//! commit is busy until someone removes it; on a phone that takes a file
//! manager or a git app. Tine therefore *offers* to remove it, never removes
//! it by itself, and only when all of these hold:
//!
//! 1. Tine runs no git operation itself: [`RUN_LOCK`] is free (every Tine path
//!    that takes `index.lock` holds it), excluding this process's live lock.
//! 2. The lock is a regular, **empty** file. Tine's lock is always empty; a
//!    git program writes the new index into its lock as it goes, and the one
//!    long honest holder, `git commit -a` (or `git commit <paths>`) waiting
//!    in an editor, holds an index already written into it. A lock with
//!    content keeps today's "busy" behaviour, and Settings names it.
//! 3. It has not changed for [`STALE_AFTER`] (by its modification time; a
//!    time in the future counts as fresh).
//!
//! Known limit: a process the OS froze or suspended while it held an empty
//! lock (Android freezer, iOS suspension, e.g. Tine beside Tine Beta, or a git
//! app paused mid-checkout) is indistinguishable by age. Each process has its
//! own RUN_LOCK. IndexLock's held-file identity check prevents a resumed Tine
//! from installing its old index or deleting a replacement lock after it
//! observes lost ownership; a frozen third-party client remains a gap in
//! D-20261029-GSL's "never for a lock another git program holds" requirement.
//! Server/client or synced-peer clock skew can also make a live lock look old.
//!
//! The click passes back the lock's identity as offered ([`StaleLock::token`]:
//! size, modification and creation times). Under [`RUN_LOCK`] the click
//! rechecks all three conditions and that identity, then moves the lock
//! aside under a Tine name and checks the moved file is still the one it
//! checked before deleting it. A lock another program created between the
//! check and the move is linked back, never deleted. Only `index.lock` is
//! ever touched: ref locks (`refs/heads/<branch>.lock`, `HEAD.lock`,
//! `packed-refs.lock`) and `shallow.lock` are left to the person.

use std::fs::Metadata;
use std::path::Path;
use std::sync::{MutexGuard, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::git_run::{open_repo_at, RUN_LOCK};

/// How long an empty `index.lock` must stay unchanged before Tine offers to
/// remove it.
pub(crate) const STALE_AFTER: Duration = Duration::from_secs(10 * 60);
/// How long the worker's offer check waits for Tine's own git operation.
const OFFER_WAIT: Duration = Duration::from_millis(250);
/// How long the click waits for Tine's own git operation before refusing.
const CLICK_WAIT: Duration = Duration::from_secs(5);
/// The name a removed lock is moved to just before it is deleted.
const ASIDE_PREFIX: &str = "tine-stale-index-";

/// A stale `index.lock` Tine offers to remove.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct StaleLock {
    pub(crate) path: String,
    /// Unix seconds of the lock's last modification.
    pub(crate) modified: i64,
    /// What the click must still find: the lock's size and times.
    pub(crate) token: String,
}

/// Why the click removed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Tine itself was running a git operation.
    TineBusy,
    /// The lock is not the one offered, or no longer looks stale.
    Changed,
    Failed(String),
}

impl Refusal {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::TineBusy => {
                "Tine is using the repository right now; try again in a moment.".into()
            }
            Self::Changed => "The lock file changed since Tine offered to remove it, so a git program may be using it; Tine left it in place.".into(),
            Self::Failed(m) => m.clone(),
        }
    }
}

/// What the click did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removed {
    Removed,
    /// There was no `index.lock` any more (someone removed it meanwhile).
    Gone,
}

/// The offer for `root`'s repository, after a commit or sync found
/// `index.lock` held. `None` when any condition fails.
pub(crate) fn offer(root: &Path) -> Option<StaleLock> {
    let _run = run_lock_within(OFFER_WAIT)?;
    assess(root, SystemTime::now())
}

/// The person clicked "Remove the stale lock" on the lock `token` names.
pub(crate) fn remove(root: &Path, token: &str) -> Result<Removed, Refusal> {
    remove_within(root, token, CLICK_WAIT, SystemTime::now())
}

fn run_lock_within(wait: Duration) -> Option<MutexGuard<'static, ()>> {
    let until = Instant::now() + wait;
    loop {
        match RUN_LOCK.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

/// Conditions 2 and 3 for the lock at `root`. The caller holds [`RUN_LOCK`].
pub(super) fn assess(root: &Path, now: SystemTime) -> Option<StaleLock> {
    let repo = open_repo_at(root).ok()?;
    let path = repo.repo.path().join("index.lock");
    repo.guard.check_path(&path).ok()?;
    let meta = std::fs::symlink_metadata(&path).ok()?;
    stale(&meta, now).then(|| StaleLock {
        path: path.display().to_string(),
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64),
        token: token(&meta),
    })
}

fn stale(meta: &Metadata, now: SystemTime) -> bool {
    meta.file_type().is_file()
        && meta.len() == 0
        && meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_AFTER)
}

/// The lock's identity as the click must find it. A rename keeps all of it;
/// a lock created anew has new times.
pub(super) fn token(meta: &Metadata) -> String {
    let nanos = |time: std::io::Result<SystemTime>| {
        time.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos())
    };
    format!(
        "{}:{}:{}",
        meta.len(),
        nanos(meta.modified()),
        nanos(meta.created())
    )
}

pub(super) fn remove_within(
    root: &Path,
    offered: &str,
    wait: Duration,
    now: SystemTime,
) -> Result<Removed, Refusal> {
    let _run = run_lock_within(wait).ok_or(Refusal::TineBusy)?;
    let repo = open_repo_at(root).map_err(|e| Refusal::Failed(e.message))?;
    let gitdir = repo.repo.path().to_path_buf();
    let lock = gitdir.join("index.lock");
    let aside = gitdir.join(format!(
        "{ASIDE_PREFIX}{}.lock",
        uuid::Uuid::new_v4().simple()
    ));
    let contained = |path: &Path| {
        repo.guard
            .check_path(path)
            .map_err(|e| Refusal::Failed(e.to_string()))
    };
    contained(&lock)?;
    contained(&aside)?;
    let meta = match std::fs::symlink_metadata(&lock) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Removed::Gone),
        Err(e) => return Err(Refusal::Failed(format!("{}: {e}", lock.display()))),
    };
    if token(&meta) != offered || !stale(&meta, now) {
        return Err(Refusal::Changed);
    }
    #[cfg(test)]
    BEFORE_MOVE.with(|h| {
        let hook = h.borrow_mut().take();
        if let Some(hook) = hook {
            hook();
        }
    });
    match std::fs::rename(&lock, &aside) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Removed::Gone),
        Err(e) => return Err(Refusal::Failed(format!("{}: {e}", lock.display()))),
    }
    let moved = std::fs::symlink_metadata(&aside).map(|m| token(&m));
    if moved.as_deref().ok() != Some(offered) {
        // Another program replaced the lock between the check and the move:
        // give it back (a link never replaces a lock created meanwhile).
        if std::fs::hard_link(&aside, &lock).is_ok() {
            let _ = std::fs::remove_file(&aside);
            return Err(Refusal::Changed);
        }
        return Err(Refusal::Failed(format!(
            "The lock file changed while Tine was removing it, and Tine could not put it back; it is now {}. If a git program is still running, let it finish and rename that file back to index.lock; otherwise delete it.",
            aside.display()
        )));
    }
    std::fs::remove_file(&aside)
        .map_err(|e| Refusal::Failed(format!("{}: {e}", aside.display())))?;
    crate::debug::diag_private(
        "git-stale-lock-removed",
        format!("git-stale-lock-removed {} ({offered})", lock.display()),
    );
    Ok(Removed::Removed)
}

#[cfg(test)]
thread_local! {
    static BEFORE_MOVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
pub(super) fn before_move_test_hook(hook: impl FnOnce() + 'static) {
    BEFORE_MOVE.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
}
