//! Sync transport (ADR 0078 §5), on all five shipped targets. Call only on the graph's dedicated
//! blocking git thread. This layer never applies a pull or changes graph files.
//! The apply lane can hold git_run::serialized across fetch/apply/push and
//! use the `_locked` entry points. Public entry points take that lock themselves.
//!
//! Tokens come from [`credentials`] (one small module, so a keychain could
//! replace its file). Providers receive the original HTTPS repository URL only
//! after a 401. Tokens never enter repository config or libgit2 callbacks.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use git2::{AutotagOption, FetchOptions, FetchPrune, Oid, PushOptions, RemoteCallbacks};

use super::git_run::{open_repo_at, raw_content_conversion, Repo, RUN_LOCK};

pub(crate) mod credentials;
mod http;
pub(crate) mod line_endings;

/// No Debug/Serialize implementation: a status or log must not print a token.
#[derive(Clone)]
pub(crate) struct Token {
    pub(crate) username: String,
    pub(crate) secret: String,
}

pub(crate) trait Credentials: Send + Sync {
    fn token(&self, repository_url: &str) -> Option<Token>;
}

/// Use while the credential storage choice is pending (public remotes work).
pub(crate) struct NoCredentials;
impl Credentials for NoCredentials {
    fn token(&self, _: &str) -> Option<Token> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Fetched objects and this remote-tracking tip are durable. Ahead/behind
    /// compare the captured local tip to upstream. The apply lane must recheck
    /// `head` before writing, since fetch has not advanced the local branch.
    Fetched {
        head: Oid,
        upstream: Oid,
        tracking_ref: String,
        ahead: usize,
        behind: usize,
    },
    /// Fetch: identical tips. Push: the remote accepted the current local tip.
    UpToDate,
    Diverged,
    AuthRequired,
    Network(String),
    Refused(String),
    /// Retryable lock contention, following autocommit's busy convention.
    Busy,
    /// An interrupted push may already have reached the server. Fetch again
    /// before retrying; libgit2 pack construction itself is not cancellable.
    Cancelled,
}

pub(crate) struct TransportOptions {
    pub(crate) connect_timeout: Duration,
    pub(crate) read_timeout: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) max_request_body: usize,
    #[cfg(test)]
    pub(super) test_root_pem: Option<Vec<u8>>,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(60),
            request_timeout: Duration::from_secs(300),
            max_request_body: 1 << 30,
            #[cfg(test)]
            test_root_pem: None,
        }
    }
}

/// Per-operation handle. Construct a fresh one for each fetch or push. It owns
/// the cancellation flag and transient credential/session state, never storage.
pub(crate) struct Operation {
    session: Arc<http::Session>,
}

impl Operation {
    pub(crate) fn new(
        credentials: Arc<dyn Credentials>,
        cancel: Arc<AtomicBool>,
        options: &TransportOptions,
    ) -> Result<Self, Outcome> {
        Ok(Self {
            session: http::Session::new(credentials, cancel, options)?,
        })
    }

    pub(crate) fn fetch(self, root: &Path) -> Outcome {
        let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        self.fetch_locked(root)
    }

    pub(crate) fn push_upstream(self, root: &Path) -> Outcome {
        let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        self.push_upstream_locked(root)
    }

    /// Caller holds RUN_LOCK, including across any pre-pull autocommit/apply.
    pub(crate) fn fetch_locked(self, root: &Path) -> Outcome {
        self.execute(|| {
            let repo = checked(root)?;
            let _index = repo.lock_index(false).map_err(lock_error)?;
            let target = Target::read(&repo)?;
            let head = repo
                .tip()
                .map_err(|e| Outcome::Refused(e.message))?
                .ok_or_else(|| refused("commit the branch before syncing"))?
                .id();
            target.check_ref(&repo)?;
            let previous = match repo.repo.find_reference(&target.tracking) {
                Ok(reference) => reference.target(),
                Err(error) if error.code() == git2::ErrorCode::NotFound => None,
                Err(error) => return Err(git_error(error)),
            };
            // Anonymous remote discards configured wildcard/mirror refspecs,
            // tag rules, pruning and local branch destinations. Fetch exactly
            // the tracked branch, allowing only its remote-tracking ref to move.
            let mut remote = validated_remote(&repo, &target.fetch_url)?;
            let mut options = FetchOptions::new();
            options
                .download_tags(AutotagOption::None)
                .prune(FetchPrune::Off)
                .update_fetchhead(false)
                .remote_callbacks(self.callbacks());
            remote
                .download(&[&target.merge], Some(&mut options))
                .map_err(|e| self.session.outcome(e))?;
            let upstream = remote
                .list()
                .map_err(git_error)?
                .iter()
                .find(|head| head.name() == target.merge)
                .ok_or_else(|| refused("the tracked upstream branch is absent from the remote"))?
                .oid();
            remote.disconnect().map_err(|e| self.session.outcome(e))?;
            self.check_cancel()?;
            // Recheck containment after network IO, before remote ref/reflog
            // writes. No FETCH_HEAD, tags, HEAD, index or worktree updates.
            target.check_ref(&repo)?;
            let current = match repo.repo.find_reference(&target.tracking) {
                Ok(reference) => reference.target(),
                Err(error) if error.code() == git2::ErrorCode::NotFound => None,
                Err(error) => return Err(git_error(error)),
            };
            if current != previous {
                return Err(Outcome::Busy);
            }
            repo.repo.find_commit(upstream).map_err(git_error)?;
            // libgit2 update_tips unconditionally truncates FETCH_HEAD, even
            // without UPDATE_FETCHHEAD. Publish exactly one ref ourselves,
            // using Git's ref lock and old-oid comparison (including absence).
            if previous != Some(upstream) {
                let result = match previous {
                    Some(previous) => repo.repo.reference_matching(
                        &target.tracking,
                        upstream,
                        true,
                        previous,
                        "tine: fetch upstream",
                    ),
                    None => repo.repo.reference(
                        &target.tracking,
                        upstream,
                        false,
                        "tine: fetch upstream",
                    ),
                };
                result.map_err(|error| match error.code() {
                    git2::ErrorCode::Modified
                    | git2::ErrorCode::Exists
                    | git2::ErrorCode::Locked => Outcome::Busy,
                    _ => git_error(error),
                })?;
            }
            if head == upstream {
                return Ok(Outcome::UpToDate);
            }
            let (ahead, behind) = match repo.repo.graph_ahead_behind(head, upstream) {
                Ok(counts) => counts,
                // Unrelated or shallow histories cannot prove a fast-forward.
                Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(Outcome::Diverged),
                Err(e) => return Err(git_error(e)),
            };
            if ahead > 0 && behind > 0 {
                return Ok(Outcome::Diverged);
            }
            Ok(Outcome::Fetched {
                head,
                upstream,
                tracking_ref: target.tracking,
                ahead,
                behind,
            })
        })
    }

    /// Push current branch to its tracked upstream, even when the upstream's
    /// name differs. Configured push refspecs/mirrors are never executed.
    pub(crate) fn push_upstream_locked(self, root: &Path) -> Outcome {
        self.execute(|| {
            let repo = checked(root)?;
            let _index = repo.lock_index(false).map_err(lock_error)?;
            let target = Target::read(&repo)?;
            let head = repo
                .tip()
                .map_err(|e| Outcome::Refused(e.message))?
                .ok_or_else(|| refused("commit the branch before syncing"))?
                .id();
            target.check_ref(&repo)?;
            let mut remote = validated_remote(&repo, &target.push_url)?;
            let mut rejection = None;
            let mut local_refusal = None;
            let mut callbacks = self.callbacks();
            callbacks.push_negotiation(|updates| {
                if updates.iter().any(|u| u.dst() != head) {
                    local_refusal = Some(refused("local branch changed before push"));
                    return Err(git2::Error::from_str("local branch changed before push"));
                }
                if let Err(outcome) = target.check_ref(&repo) {
                    local_refusal = Some(outcome);
                    return Err(git2::Error::from_str("repository changed before push"));
                }
                if let Err(error) = repo.guard.check_git(&repo.repo, &repo.branch) {
                    local_refusal = Some(git_error(error));
                    return Err(git2::Error::from_str("repository changed before push"));
                }
                if self.session.cancelled() {
                    return Err(git2::Error::from_str("cancelled"));
                }
                Ok(())
            });
            callbacks.push_update_reference(|_, status| {
                if let Some(status) = status {
                    rejection = Some(
                        if status.contains("non-fast-forward") || status.contains("fetch first") {
                            Outcome::Diverged
                        } else {
                            refused("the remote rejected the push")
                        },
                    );
                }
                Ok(())
            });
            let mut options = PushOptions::new();
            options.remote_callbacks(callbacks);
            // No '+' under any circumstances; libgit2 checks fast-forward and
            // the server enforces its old-oid comparison atomically as well.
            let result = remote.push(
                &[format!("{}:{}", repo.branch, target.merge)],
                Some(&mut options),
            );
            drop(options);
            if let Some(refusal) = local_refusal {
                return Err(refusal);
            }
            if let Some(rejection) = rejection {
                return Ok(rejection);
            }
            result.map_err(|e| self.session.outcome(e))?;
            Ok(Outcome::UpToDate)
        })
    }

    fn execute(&self, f: impl FnOnce() -> Result<Outcome, Outcome>) -> Outcome {
        if let Err(outcome) = self.check_cancel() {
            return outcome;
        }
        match self.session.run(f) {
            Ok(Ok(outcome)) | Ok(Err(outcome)) | Err(outcome) => outcome,
        }
    }

    fn check_cancel(&self) -> Result<(), Outcome> {
        if self.session.cancelled() {
            Err(Outcome::Cancelled)
        } else {
            Ok(())
        }
    }

    fn callbacks(&self) -> RemoteCallbacks<'_> {
        let mut callbacks = RemoteCallbacks::new();
        callbacks.transfer_progress(|_| !self.session.cancelled());
        callbacks.sideband_progress(|_| !self.session.cancelled());
        callbacks
    }
}

fn refused(message: &str) -> Outcome {
    Outcome::Refused(message.into())
}
fn git_error(error: git2::Error) -> Outcome {
    refused(&format!("git: {}", error.message()))
}
fn lock_error(error: super::git_run::LockRefusal) -> Outcome {
    if matches!(error, super::git_run::LockRefusal::Held) {
        Outcome::Busy
    } else {
        refused(&error.message())
    }
}

/// Anonymous remotes still apply Git's URL rewrite configuration. Inspect both
/// effective URLs before creating a transport so an insteadOf/pushInsteadOf
/// rule cannot bypass HTTPS/credential admission or silently change the remote.
fn validated_remote<'a>(repo: &'a Repo, url: &str) -> Result<git2::Remote<'a>, Outcome> {
    let remote = repo.repo.remote_anonymous(url).map_err(git_error)?;
    if remote.url().map_err(git_error)? != url
        || remote
            .pushurl()
            .map_err(git_error)?
            .is_some_and(|push| push != url)
    {
        return Err(refused(
            "sync refuses URL rewrites that change the remote endpoint",
        ));
    }
    Ok(remote)
}

/// What Settings shows before sync is chosen: the tracked remote, or why sync
/// cannot run. Reads configuration only; each cycle's [`checked`] also checks
/// every tracked path's attributes.
pub(crate) fn remote(root: &Path) -> Result<Remote, Outcome> {
    let repo = config_checked(root)?;
    let target = Target::read(&repo)?;
    Ok(Remote {
        https_origin: https_origin(&target.fetch_url),
        name: target.remote,
        // Target refuses an HTTPS URL carrying credentials; a file URL holds none.
        url: target.fetch_url,
    })
}

/// Whether the branch has commits its remote-tracking ref (the last fetch)
/// lacks, and that ref has none the branch lacks: what a background push
/// sends (slice 3). Reads refs only; the push itself checks again under the
/// runs' lock.
pub(crate) fn push_pending(root: &Path) -> bool {
    let Ok(repo) = config_checked(root) else {
        return false;
    };
    let Ok(target) = Target::read(&repo) else {
        return false;
    };
    let (Ok(Some(head)), Ok(tracking)) = (repo.tip(), repo.repo.refname_to_id(&target.tracking))
    else {
        return false;
    };
    head.id() != tracking
        && repo
            .repo
            .graph_descendant_of(head.id(), tracking)
            .unwrap_or(false)
}

/// The tracked remote Settings names (Tine never changes remote configuration).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Remote {
    pub(crate) name: String,
    pub(crate) url: String,
    /// `https://host[:port]` for an HTTPS remote: what a token is kept for.
    pub(crate) https_origin: Option<String>,
}

/// `https://host[:port]` of an HTTPS remote URL, `None` for any other.
pub(crate) fn https_origin(url: &str) -> Option<String> {
    http::https_url(url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
}

fn checked(root: &Path) -> Result<Repo, Outcome> {
    checked_paths(config_checked(root)?)
}

fn config_checked(root: &Path) -> Result<Repo, Outcome> {
    let repo = open_repo_at(root).map_err(|e| Outcome::Refused(e.message))?;
    if !repo.prefix.as_os_str().is_empty() {
        return Err(refused(
            "sync requires the graph to be the repository work tree root",
        ));
    }
    super::git_autocommit::check_config(&repo)
        .map_err(|e| Outcome::Refused(format!("{}: {}", e.code(), e.message())))?;
    let config = repo.repo.config().map_err(git_error)?;
    match config.get_entry("core.autocrlf") {
        Ok(_)
            if !config
                .get_bool("core.autocrlf")
                .is_ok_and(|enabled| !enabled) =>
        {
            return Err(refused("sync refuses core.autocrlf content conversion"));
        }
        Err(error) if error.code() != git2::ErrorCode::NotFound => return Err(git_error(error)),
        _ => {}
    }
    Ok(repo)
}

fn checked_paths(repo: Repo) -> Result<Repo, Outcome> {
    check_paths(&repo)?;
    Ok(repo)
}

fn check_paths(repo: &Repo) -> Result<(), Outcome> {
    let index = repo.repo.index().map_err(git_error)?;
    if index.iter().any(|e| e.flags_extended & (1 << 14) != 0) {
        return Err(refused("sync refuses sparse/skip-worktree entries"));
    }
    // Check every tracked path, not only representative page names. This also
    // catches a conversion rule scoped to one asset or an unusual extension.
    let mut paths: Vec<_> = index.iter().map(|e| e.path).collect();
    paths.extend([
        b"pages/a.md".to_vec(),
        b"journals/a.md".to_vec(),
        b"pages/a.org".to_vec(),
        b"a.md".to_vec(),
        b"logseq/config.edn".to_vec(),
        b"assets/a.png".to_vec(),
    ]);
    for path in paths {
        let path = std::str::from_utf8(&path)
            .map_err(|_| refused("sync requires UTF-8 repository paths"))?;
        if let Some(attr) =
            raw_content_conversion(&repo.repo, Path::new(path)).map_err(git_error)?
        {
            return Err(refused(&format!(
                "sync refuses {attr} conversion for {path}"
            )));
        }
        for attr in ["text", "eol", "crlf"] {
            let value = repo
                .repo
                .get_attr(Path::new(path), attr, git2::AttrCheckFlags::FILE_THEN_INDEX)
                .map_err(git_error)?;
            if !matches!(
                git2::AttrValue::from_string(value),
                git2::AttrValue::Unspecified | git2::AttrValue::False
            ) {
                return Err(refused(&format!(
                    "sync refuses {attr} conversion for {path}"
                )));
            }
        }
    }
    Ok(())
}

struct Target {
    remote: String,
    merge: String,
    tracking: String,
    fetch_url: String,
    push_url: String,
}

impl Target {
    fn read(repo: &Repo) -> Result<Self, Outcome> {
        let config = repo.repo.config().map_err(git_error)?;
        let branch = repo
            .branch
            .strip_prefix("refs/heads/")
            .ok_or_else(|| refused("HEAD must name a local branch"))?;
        let one = |key: &str| -> Result<String, Outcome> {
            let mut entries = config
                .multivar(key, None)
                .map_err(|_| refused("configure one tracked upstream before syncing"))?;
            let entry = entries
                .next()
                .transpose()
                .map_err(git_error)?
                .ok_or_else(|| refused("configure one tracked upstream before syncing"))?
                .value()
                .map_err(git_error)?
                .to_owned();
            if entries.next().is_some() {
                return Err(refused("sync requires a single upstream and remote URL"));
            }
            Ok(entry)
        };
        let remote_name = one(&format!("branch.{branch}.remote"))?;
        if remote_name == "." {
            return Err(refused("sync needs an external upstream remote"));
        }
        let merge = one(&format!("branch.{branch}.merge"))?;
        if !merge.starts_with("refs/heads/") || !git2::Reference::is_valid_name(&merge) {
            return Err(refused("sync requires an upstream branch"));
        }
        let tracking = repo
            .repo
            .branch_upstream_name(&repo.branch)
            .map_err(git_error)?;
        let tracking = tracking.as_str().map_err(git_error)?.to_owned();
        if !tracking.starts_with(&format!("refs/remotes/{remote_name}/"))
            || tracking.ends_with("/HEAD")
        {
            return Err(refused("upstream must map to a remote-tracking branch"));
        }
        let fetch_url = one(&format!("remote.{remote_name}.url"))?;
        let push_key = format!("remote.{remote_name}.pushurl");
        let push_url = match config.get_entry(&push_key) {
            Ok(_) => one(&push_key)?,
            Err(error) if error.code() == git2::ErrorCode::NotFound => fetch_url.clone(),
            Err(error) => return Err(git_error(error)),
        };
        // A separate push endpoint is supported, but a push-remote override or
        // mirror could express an intent different from the tracked upstream.
        for key in [
            format!("branch.{branch}.pushRemote"),
            "remote.pushDefault".into(),
        ] {
            if config.get_string(&key).is_ok_and(|v| v != remote_name) {
                return Err(refused("sync refuses a different push remote"));
            }
        }
        if config
            .get_bool(&format!("remote.{remote_name}.mirror"))
            .unwrap_or(false)
        {
            return Err(refused("sync refuses mirror remotes"));
        }
        for url in [&fetch_url, &push_url] {
            if url.starts_with("file://") {
                let parsed =
                    reqwest::Url::parse(url).map_err(|_| refused("invalid file remote"))?;
                if parsed.to_file_path().is_err()
                    || parsed.query().is_some()
                    || parsed.fragment().is_some()
                {
                    return Err(refused("sync requires a local file remote"));
                }
            } else {
                http::https_url(url)?;
            }
        }
        Ok(Self {
            remote: remote_name,
            merge,
            tracking,
            fetch_url,
            push_url,
        })
    }

    fn check_ref(&self, repo: &Repo) -> Result<(), Outcome> {
        repo.guard
            .check_git(&repo.repo, &self.tracking)
            .map_err(git_error)?;
        if let Ok(reference) = repo.repo.find_reference(&self.tracking) {
            if reference.symbolic_target_bytes().is_some() {
                return Err(refused("upstream remote-tracking ref must not be symbolic"));
            }
        }
        let head = repo.repo.find_reference("HEAD").map_err(git_error)?;
        if head.symbolic_target_bytes() != Some(repo.branch.as_bytes()) {
            return Err(refused("the checked-out branch changed during sync"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "git_sync/transport_tests.rs"]
mod tests;
