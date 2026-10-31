//! Explicit, one-off clone: fetch through the same HTTPS session as sync,
//! inspect the complete tree before creating any work-tree file, then checkout.
use super::*;
use git2::build::{CheckoutBuilder, CloneLocal, RepoBuilder};
use std::collections::HashSet;

#[cfg(any(target_os = "android", target_os = "macos", target_os = "ios"))]
const FOLD_NAMES: bool = true;
#[cfg(any(target_os = "linux", target_os = "windows"))]
const FOLD_NAMES: bool = false;

/// Validate before making a directory or retaining a credential. UI accepts
/// HTTPS only; file remotes are available to Rust fixture tests.
pub(crate) fn repository_name(url: &str) -> Result<String, Outcome> {
    #[cfg(test)]
    let local = url.starts_with("file://");
    #[cfg(not(test))]
    let local = false;
    let parsed = if local {
        reqwest::Url::parse(url).map_err(|_| refused("invalid repository URL"))?
    } else {
        http::https_url(url)?
    };
    let name = parsed
        .path_segments()
        .and_then(|mut p| p.next_back())
        .unwrap_or("")
        .trim_end_matches(".git");
    // A single portable literal component, never an encoded slash or traversal.
    if name.is_empty()
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c.is_control() || "/\\%:<>\"|?*".contains(c))
        || name.ends_with(['.', ' '])
    {
        return Err(refused(
            "The repository URL must end in a valid folder name (optionally .git).",
        ));
    }
    Ok(name.into())
}

impl Operation {
    pub(crate) fn clone_graph(self, url: &str, root: &Path) -> Outcome {
        self.clone_on_volume(url, root, None)
    }

    pub(super) fn clone_on_volume(self, url: &str, root: &Path, folding: Option<bool>) -> Outcome {
        let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        self.execute(|| {
            repository_name(url)?;
            super::super::git_run::configure_libgit2();
            match std::fs::symlink_metadata(root) {
                Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                    if std::fs::read_dir(root).map_err(io_error)?.next().is_some() {
                        return Err(refused(
                            "The destination folder is not empty. Choose another location.",
                        ));
                    }
                }
                Ok(_) => return Err(refused("The destination must be a new, empty folder.")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(e)),
            }
            let mut fetch = FetchOptions::new();
            fetch.remote_callbacks(self.callbacks());
            let mut checkout = CheckoutBuilder::new();
            checkout.dry_run(); // GIT_CHECKOUT_NONE: libgit2 clone.c skips checkout entirely.
            let mut builder = RepoBuilder::new();
            builder
                .clone_local(CloneLocal::None)
                .fetch_options(fetch)
                .with_checkout(checkout);
            builder.remote_create(|repo, name, url| {
                // Never allow global insteadOf rules to redirect a token or
                // escape our HTTPS transport, including during clone.
                let remote = repo.remote(name, url)?;
                if remote.url()? != url || remote.pushurl()?.is_some_and(|u| u != url) {
                    return Err(git2::Error::from_str(
                        "Clone refuses URL rewrites that change the remote endpoint.",
                    ));
                }
                Ok(remote)
            });
            let repo = builder
                .clone(url, root)
                .map_err(|e| self.session.outcome(e))?;
            if repo.is_empty().map_err(git_error)? {
                return Err(refused("This remote has no commits yet. Create a new graph, then choose Make this graph a git repository and paste this empty remote URL."));
            }
            let mut config = repo
                .config()
                .and_then(|config| config.open_level(git2::ConfigLevel::Local))
                .map_err(git_error)?;
            config.set_bool("core.autocrlf", false).map_err(git_error)?;
            config
                .set_bool("core.precomposeunicode", true)
                .map_err(git_error)?;
            {
                let head = repo.head().map_err(git_error)?;
                if !head.is_branch() { return Err(refused("The remote must name a default branch before cloning.")); }
                repo.branch_upstream_name(head.name().map_err(git_error)?).map_err(git_error)?;
                let tree = head.peel_to_tree().map_err(git_error)?;
                // Android's shared storage folds names even when libgit2's
                // probe doesn't report it. Other targets use the volume probe.
                let folds = folding.unwrap_or_else(|| {
                    FOLD_NAMES || config.get_bool("core.ignorecase").unwrap_or(false)
                });
                check_tree(&repo, &tree, folds)?;
                self.check_cancel()?;
                let mut checkout = CheckoutBuilder::new();
                checkout.safe().disable_filters(true);
                repo.checkout_head(Some(&mut checkout)).map_err(git_error)?;
            }
            Ok(Outcome::UpToDate)
        })
    }
}

fn io_error(error: std::io::Error) -> Outcome {
    refused(&format!("Couldn't prepare the clone folder: {error}"))
}

fn check_tree(repo: &git2::Repository, tree: &git2::Tree<'_>, folds: bool) -> Result<(), Outcome> {
    let mut pending = vec![tree.id()];
    while let Some(id) = pending.pop() {
        let tree = repo.find_tree(id).map_err(git_error)?;
        let mut names = HashSet::new();
        for entry in &tree {
            let name = entry
                .name()
                .map_err(|_| refused("Clone requires UTF-8 file names."))?;
            if folds && !names.insert(tine_store::file_kind::fold_entry_name(name)) {
                return Err(refused("This repository contains names differing only in case or Unicode normalization. This device cannot store both; rename one in the remote repository before cloning."));
            }
            if entry.kind() == Some(git2::ObjectType::Tree) {
                pending.push(entry.id());
            }
        }
    }
    Ok(())
}
