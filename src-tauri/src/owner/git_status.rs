//! Shared Settings status and the explicit repository-setup result.
use super::*;
use crate::owner::{git_sync, git_sync_cycle};

/// What Settings → Graph → Git shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct GitStatus {
    /// Git features exist here: always in the app (all five targets, slice 3);
    /// the browser mock and a published graph send `false`.
    pub(crate) available: bool,
    /// The graph is inside a git work tree.
    pub(crate) repository: bool,
    pub(crate) branch: Option<String>,
    /// Why autocommit cannot run, when it cannot (`code`, message).
    pub(crate) unusable: Option<(String, String)>,
    pub(crate) policy: Policy,
    pub(crate) interval_minutes: u32,
    pub(crate) last: LastAttempt,
    /// The tracked remote (read from the repository config), when sync can
    /// run here.
    pub(crate) remote: Option<git_sync::Remote>,
    /// Why the sync options are refused (`code`, message), when they are.
    pub(crate) sync_unusable: Option<(String, String)>,
    pub(crate) line_endings: Option<git_sync::line_endings::Probe>,
    /// The username this device signed in with for the remote's host.
    pub(crate) signed_in: Option<String>,
    pub(crate) setup_pending: bool,
}

pub(crate) fn git_status(root: &Path, settings: GitSettings) -> GitStatus {
    let checked = open_checked(root).and_then(|repo| {
        let prefix = prefix_of(&repo);
        if let Ok(index) = repo.repo.index() {
            if has_skip_worktree(&index, &prefix) {
                return Err(Unusable::Sparse);
            }
        }
        Ok(repo)
    });
    let (repository, branch, unusable) = match checked {
        Ok(repo) => (
            true,
            Some(repo.branch.trim_start_matches("refs/heads/").to_string()),
            None,
        ),
        Err(Unusable::NotARepository) => (false, None, None),
        Err(error) => (
            true,
            None,
            Some((error.code().to_string(), error.message())),
        ),
    };
    let (remote, sync_unusable) = match (&unusable, repository) {
        (Some(why), _) => (None, Some(why.clone())),
        (None, false) => (None, None),
        (None, true) => match git_sync::remote(root) {
            Ok(remote) => (Some(remote), None),
            Err(git_sync::Outcome::Refused(why)) => (None, Some(("refused".into(), why))),
            Err(other) => (None, Some(("refused".into(), format!("{other:?}")))),
        },
    };
    let line_endings = sync_unusable
        .as_ref()
        .filter(|(code, why)| code == "conversion" || why.contains("conversion"))
        .map(|_| git_sync::line_endings::probe(root));
    GitStatus {
        available: true,
        repository,
        branch,
        unusable,
        policy: settings.policy,
        interval_minutes: (settings.interval.as_secs() / 60) as u32,
        last: last_attempt(root),
        remote,
        sync_unusable,
        line_endings,
        signed_in: None,
        setup_pending: git_sync::initial_push_pending(root),
    }
}

pub(crate) fn report_setup(root: &Path, result: git_sync::Outcome) {
    if let Some(worker) = registry::get(root) {
        let mut state = worker.state.lock().unwrap();
        let interval = state.settings.interval;
        state
            .last
            .sync
            .record(&git_sync_cycle::network_outcome(result), interval);
    }
}
