//! Explicit, one-off clone: fetch through the same HTTPS session as sync,
//! inspect the complete tree before creating any work-tree file, then checkout.
use super::*;
use git2::build::{CheckoutBuilder, CloneLocal, RepoBuilder};
use std::collections::HashSet;
use std::fs;
use tine_store::BoundDirectory;

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
        let mut folder = None;
        let mut checkout_started = false;
        let result = self.execute(|| {
            repository_name(url)?;
            super::super::git_run::configure_libgit2();
            let created = match std::fs::symlink_metadata(root) {
                Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                    if std::fs::read_dir(root).map_err(io_error)?.next().is_some() {
                        return Err(refused(
                            "The destination folder is not empty. Choose another location.",
                        ));
                    }
                    false
                }
                Ok(_) => return Err(refused("The destination must be a new, empty folder.")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(root).map_err(io_error)?;
                    true
                }
                Err(e) => return Err(io_error(e)),
            };
            let binding = CloneDirectory::new(root).map_err(|e| retained(root, io_error(e)))?;
            folder = Some(CloneFolder { binding, created, metadata: None });
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
                // Libgit2 has just initialized .git in the admitted empty
                // folder. Capture it before URL checks or any network IO.
                let owned = folder.as_mut().unwrap();
                owned.binding.check_path(&root.join(".git"))
                    .and_then(|_| plain_directory(&root.join(".git")))
                    .map_err(|e| git2::Error::from_str(&e.to_string()))?;
                owned.metadata = Some(CloneDirectory::new(&root.join(".git"))
                    .map_err(|e| git2::Error::from_str(&e.to_string()))?);
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
            let cloned = builder.clone(url, root);
            drop(builder);
            let repo = cloned.map_err(|e| self.session.outcome(e))?;
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
                // Checkout can write some files before returning an error.
                // From this point onward retain everything, including on open failure.
                checkout_started = true;
                repo.checkout_head(Some(&mut checkout)).map_err(git_error)?;
            }
            Ok(Outcome::UpToDate)
        });
        // All libgit2 handles (including a failed fetch) are closed first.
        if !checkout_started && result != Outcome::UpToDate {
            if let Some(folder) = folder {
                if folder.cleanup().is_err() {
                    return retained(root, result);
                }
            }
        }
        result
    }
}

/// Only the empty destination and .git initialized by this attempt are eligible.
/// These use the existing sequential identity/containment checks; as with other
/// native metadata writers, they do not eliminate check-to-syscall races.
struct CloneFolder {
    binding: CloneDirectory,
    created: bool,
    metadata: Option<CloneDirectory>,
}

/// Pin identity only during this attempt, including on Android/FUSE where an
/// unpinned inode can change after eviction. Windows handles allow delete sharing.
struct CloneDirectory {
    binding: BoundDirectory,
    identity: same_file::Handle,
}

impl CloneDirectory {
    fn new(root: &Path) -> std::io::Result<Self> {
        plain_directory(root)?;
        let identity = directory_identity(root)?;
        let binding = BoundDirectory::new(root)?;
        let owned = Self { binding, identity };
        owned.check()?;
        Ok(owned)
    }

    fn root(&self) -> &Path {
        self.binding.root()
    }

    fn check(&self) -> std::io::Result<()> {
        plain_directory(self.root())?;
        self.binding.check()?;
        if self.identity != directory_identity(self.root())? {
            return Err(std::io::Error::other("clone directory was replaced"));
        }
        Ok(())
    }

    fn check_path(&self, path: &Path) -> std::io::Result<()> {
        self.check()?;
        self.binding.check_path(path)
    }
}

fn directory_identity(root: &Path) -> std::io::Result<same_file::Handle> {
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(root)?
    };
    #[cfg(target_os = "windows")]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(root)?
    };
    same_file::Handle::from_file(file)
}

impl CloneFolder {
    fn cleanup(self) -> std::io::Result<()> {
        let root = self.binding.root();
        if self.created
            && fs::symlink_metadata(root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(()); // Libgit2 may already have removed its failed clone.
        }
        self.binding.check()?;
        plain_directory(root)?;
        // Complete the census before deleting anything. Unknown scratch or a
        // concurrently created page is retained along with the metadata.
        let mut has_metadata = false;
        for entry in fs::read_dir(root)? {
            if entry?.file_name() != ".git" || self.metadata.is_none() {
                return Err(std::io::Error::other("unexpected clone folder contents"));
            }
            has_metadata = true;
        }
        if let Some(metadata) = self.metadata.filter(|_| has_metadata) {
            self.binding.check_path(metadata.root())?;
            metadata.check()?;
            check_metadata(&metadata, metadata.root())?;
            self.binding.check()?;
            metadata.check()?;
            // std's recursive removal does not follow symlinks. The preflight
            // above also refuses symlinks/special entries rather than deleting them.
            fs::remove_dir_all(metadata.root())?;
        }
        if self.created {
            self.binding.check()?;
            plain_directory(root)?;
            // Non-recursive: a new arrival prevents removing the directory.
            fs::remove_dir(root)?;
        }
        Ok(())
    }
}

fn plain_directory(path: &Path) -> std::io::Result<()> {
    let metadata = plain_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "clone folder is no longer a plain directory",
        ));
    }
    Ok(())
}

fn plain_metadata(path: &Path) -> std::io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::other("clone metadata contains a symlink"));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        // Refuse all reparse points, including junctions and unknown tags.
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other(
                "clone metadata contains a reparse point",
            ));
        }
    }
    Ok(metadata)
}

fn check_metadata(binding: &CloneDirectory, path: &Path) -> std::io::Result<()> {
    binding.check_path(path)?;
    plain_directory(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        binding.check_path(&entry.path())?;
        let kind = plain_metadata(&entry.path())?.file_type();
        if kind.is_dir() {
            check_metadata(binding, &entry.path())?;
        } else if !kind.is_file() {
            return Err(std::io::Error::other("unexpected clone metadata entry"));
        }
    }
    Ok(())
}

fn retained(root: &Path, result: Outcome) -> Outcome {
    let reason = match result {
        Outcome::Refused(message) | Outcome::Network(message) => message,
        Outcome::AuthRequired => "Sign in to clone this repository.".into(),
        Outcome::Cancelled => "Clone cancelled.".into(),
        _ => "Clone could not finish.".into(),
    };
    refused(&format!("{reason} Tine could not safely clean up the clone folder {}; it was retained. Check it before retrying.", root.display()))
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

#[cfg(test)]
#[path = "clone_cleanup_tests.rs"]
mod cleanup_tests;
