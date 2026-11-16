//! Opening the graph's repository: libgit2's process options, git's own
//! discovery of the repository and its work tree, and where the graph sits in
//! that work tree.

use std::path::{Path, PathBuf};
use std::sync::Once;

use git2::{ErrorCode, Repository};
use libgit2_sys::init as libgit2_init;

/// Whether libgit2 checks that a repository belongs to this OS user (its
/// `safe.directory` rule, CVE-2022-24765), per shipped target: Linux, macOS,
/// Windows and iOS yes; Android no. Android shared storage answers `stat`
/// with the backing file's owner, never the app's uid (MediaProvider's FUSE
/// daemon, `pf_getattr`; CI `android-git-storage`), so the check refuses
/// every repository there. The check guards against a repository another
/// user planted; on Android any app with storage access can rewrite the
/// graph itself, which Tine's trust model leaves out of scope
/// (`specs/notes/2026-08-07-trust-model-and-threat-model-decision.md`).
pub(crate) const VERIFY_OWNER: bool = !cfg!(target_os = "android");

/// libgit2's process-wide options, set once before this process opens a
/// repository: fsync on (D-20261019-G8) and [`VERIFY_OWNER`]. Production
/// opens go through [`discover`].
pub(crate) fn configure_libgit2() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        libgit2_init();
        // SAFETY: process-global libgit2 options set once, before any
        // repository is opened; each takes one int argument.
        unsafe {
            libgit2_sys::git_libgit2_opts(
                libgit2_sys::GIT_OPT_ENABLE_FSYNC_GITDIR as std::os::raw::c_int,
                1 as std::os::raw::c_int,
            );
            libgit2_sys::git_libgit2_opts(
                libgit2_sys::GIT_OPT_SET_OWNER_VALIDATION as std::os::raw::c_int,
                VERIFY_OWNER as std::os::raw::c_int,
            );
        }
    });
}

/// The repository holding `path`, found as git finds it, after
/// [`configure_libgit2`] (so no open depends on another having configured
/// libgit2 first: Android's owner check). The search goes upward from `path`
/// and keeps the `.git` file it went through, so the work tree of a separate
/// git directory (`git init --separate-git-dir`, which Logseq's version
/// control runs) is the folder holding that file, as `git rev-parse
/// --show-toplevel` says (GH #872). `Repository::discover` reopened the git
/// directory it found and answered that directory's parent instead.
/// A `.git` file at `path` naming no repository is an error, as in git
/// ("not a git repository"), never "no repository here".
pub(crate) fn discover(path: &Path) -> Result<Repository, git2::Error> {
    configure_libgit2();
    let found = Repository::open_ext(
        path,
        git2::RepositoryOpenFlags::CROSS_FS,
        std::iter::empty::<&std::ffi::OsStr>(),
    );
    match found {
        Err(error) if error.code() == ErrorCode::NotFound => {
            let gitfile = path.join(".git");
            if std::fs::symlink_metadata(&gitfile).is_ok_and(|m| m.is_file()) {
                return Err(git2::Error::new(
                    ErrorCode::GenericError,
                    git2::ErrorClass::Repository,
                    format!(
                        "{} names a git directory that is not a git repository (missing or moved); repair or remove that file",
                        gitfile.display()
                    ),
                ));
            }
            Err(error)
        }
        other => other,
    }
}

/// The path from the work tree root to the graph root ("" when equal): the
/// one answer to "is the graph inside this repository's work tree", compared
/// canonically. The work tree is git's ([`discover`]).
pub(crate) fn graph_prefix(repo: &Repository, graph_root: &Path) -> Result<PathBuf, String> {
    let workdir = repo.workdir().ok_or("the graph's repository is bare")?;
    let root = std::fs::canonicalize(graph_root).map_err(|e| e.to_string())?;
    let workdir = std::fs::canonicalize(workdir).map_err(|e| e.to_string())?;
    root.strip_prefix(&workdir)
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "the graph is not inside its repository's work tree ({})",
                workdir.display()
            )
        })
}
