//! Command-line git runs (CLI slice 3; journal D-20261019-CL1…CL4).
//!
//! A run is the commits that carry a `Tine-Run: <id>` trailer. `run begin`
//! mints the id and checks the repository; nothing is stored anywhere else.
//! Each CLI write made with `--run` (or `TINE_RUN`) commits, after the guarded
//! save succeeded and for that one file only:
//!
//! 1. the bytes the write was based on, under the user's git identity, if
//!    they differ from the file at `HEAD` (the user's pending edits, Syncthing
//!    deliveries included, never land in the agent's commit);
//! 2. the bytes the write produced, under the agent identity
//!    (`--author`/`TINE_AUTHOR`, email `agent@tine.invalid`), with the trailer.
//!
//! Both commits are built from those exact bytes and `HEAD`'s tree, never from
//! the working tree or the index, and the branch moves by compare-and-swap, so
//! a concurrent commit (the user's, or another owner's) is retried on top, not
//! overwritten. `run end` squashes the run into one commit only when its
//! commits are contiguous at the branch tip, none is reachable from any other
//! ref (pushed, branched, tagged), and the tip has not moved. `run revert`
//! undoes the agent commits newest first, page by page, through
//! [`tine_graph_features::run_revert`] and the guarded save.
//!
//! The graph's files change only through the page host; this module writes
//! only `.git` objects, refs and the index entry of the file it committed
//! (CL4: not a storage mode). libgit2 runs with GIT_OPT_ENABLE_FSYNC_GITDIR.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use git2::{ErrorCode, Oid, Repository, RepositoryState, Signature};
use libgit2_sys::init as libgit2_init;
use serde_json::{json, Value};
use tine_store::PageId;

use super::wire::{ok, ErrorBody};
use crate::state::GraphSlot;

pub(crate) const TRAILER: &str = "Tine-Run";
const SNAPSHOT_TRAILER: &str = "Tine-Snapshot";
const REVERT_TRAILER: &str = "Tine-Run-Revert";
pub(crate) const AGENT_EMAIL: &str = "agent@tine.invalid";
pub(super) const USER_FALLBACK: (&str, &str) = ("Tine user", "user@tine.invalid");
/// How far before a run's begin time its commits are looked for (clock steps).
const WINDOW_SLACK_SECS: i64 = 600;
/// The most first-parent commits a run lookup walks.
const WALK_LIMIT: usize = 20_000;
/// Compare-and-swap attempts before a commit gives up.
pub(super) const CAS_ATTEMPTS: usize = 4;

/// Held across a run write's save and its commits, and across `run end` and
/// `run revert`, so the commits follow the order of the saves (CL1: "under
/// the page lock"). Two run writes racing on one page would otherwise commit
/// in the other order, or snapshot an agent's bytes under the user's name.
pub(super) static RUN_LOCK: Mutex<()> = Mutex::new(());

/// Run `f` holding [`RUN_LOCK`].
pub(crate) fn serialized<T>(f: impl FnOnce() -> T) -> T {
    let _guard = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    f()
}

/// The run a write belongs to, and who the agent is.
#[derive(Debug, Clone)]
pub(crate) struct RunCtx {
    pub(crate) id: RunId,
    pub(crate) author: String,
    /// For a revert's writes: the run they undo (`Tine-Run-Revert`).
    reverts: Option<RunId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunId {
    text: String,
    begun: i64,
}

impl RunId {
    fn mint(now: i64) -> Self {
        let random = uuid::Uuid::new_v4().simple().to_string();
        Self {
            text: format!("r{now:x}-{}", &random[..8]),
            begun: now,
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let rest = text.strip_prefix('r')?;
        let (time, random) = rest.split_once('-')?;
        let hex = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        };
        if !hex(time) || time.len() > 12 || random.len() != 8 || !hex(random) {
            return None;
        }
        Some(Self {
            text: text.to_string(),
            begun: i64::from_str_radix(time, 16).ok()?,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }
}

/// `run` and `author` from a write verb's arguments. A malformed id is
/// `bad-args`, never silently ignored.
pub(crate) fn run_of(args: &Value) -> Result<Option<RunCtx>, ErrorBody> {
    let Some(text) = args
        .get("run")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let id = RunId::parse(text).ok_or_else(|| {
        ErrorBody::new(
            "bad-args",
            format!("`{text}` is not a run id; `tine run begin` prints one"),
            false,
        )
    })?;
    Ok(Some(RunCtx {
        id,
        author: author_of(args),
        reverts: None,
    }))
}

pub(crate) fn author_of(args: &Value) -> String {
    args.get("author")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "agent".into())
}

pub(super) fn enable_git_fsync() {
    static FSYNC: Once = Once::new();
    FSYNC.call_once(|| {
        libgit2_init();
        // SAFETY: a process-global libgit2 option set once, before any
        // repository is opened by this module; takes one int argument.
        unsafe {
            libgit2_sys::git_libgit2_opts(
                libgit2_sys::GIT_OPT_ENABLE_FSYNC_GITDIR as std::os::raw::c_int,
                1 as std::os::raw::c_int,
            );
        }
    });
}

/// The graph's repository cannot take a run's commits: `refused:git`.
fn not_usable(message: impl Into<String>) -> ErrorBody {
    super::write::refused("git", None, message)
}

pub(super) fn git_failed(error: git2::Error) -> ErrorBody {
    super::write::refused("git", None, format!("git: {}", error.message()))
}

/// Refuse a run's write before it changes anything when its commits could
/// not be made (no repository, a merge in progress, a detached HEAD).
pub(crate) fn check_repository(slot: &GraphSlot) -> Result<(), ErrorBody> {
    let repo = open_repo(slot)?;
    // Another git process holds the index (git-sync DESIGN §3.1: retry later).
    if repo.repo.path().join("index.lock").exists() {
        return Err(super::write::refused(
            "git-busy",
            None,
            "another git command holds the repository's index (index.lock); try again when it is done",
        ));
    }
    Ok(())
}

/// Exact-path admission before a run can flush or save a page.
pub(crate) fn check_page(slot: &GraphSlot, page: &PageId) -> Result<(), ErrorBody> {
    let repo = open_repo(slot)?;
    repo.check_raw_content(&repo.path_of(page))
}

/// Raw-content eligibility shared by runs and autocommits. Never execute
/// filters: their working bytes may be plaintext that must not enter Git.
pub(super) fn raw_content_conversion(
    repo: &Repository,
    path: &Path,
) -> Result<Option<&'static str>, git2::Error> {
    for attr in ["filter", "ident", "working-tree-encoding"] {
        let value = repo.get_attr(path, attr, git2::AttrCheckFlags::FILE_THEN_INDEX)?;
        if !matches!(
            git2::AttrValue::from_string(value),
            git2::AttrValue::Unspecified | git2::AttrValue::False
        ) {
            return Ok(Some(attr));
        }
    }
    Ok(None)
}

/// The graph's repository, opened and checked for commits (git-sync DESIGN
/// §3.1): not bare, no merge/rebase/cherry-pick in progress, on a branch.
pub(super) struct Repo {
    pub(super) repo: Repository,
    /// Path from the work tree root to the graph root ("" when equal).
    pub(super) prefix: PathBuf,
    pub(super) branch: String,
}

fn open_repo(slot: &GraphSlot) -> Result<Repo, ErrorBody> {
    open_repo_at(&slot.root_key)
}

pub(super) fn open_repo_at(graph_root: &Path) -> Result<Repo, ErrorBody> {
    enable_git_fsync();
    let repo = match Repository::discover(graph_root) {
        Ok(repo) => repo,
        Err(error) if error.code() == ErrorCode::NotFound => return Err(not_usable(
            "the graph is not in a git repository; runs need one (`git init` in the graph folder)",
        )),
        Err(error) => return Err(git_failed(error)),
    };
    let Some(workdir) = repo.workdir() else {
        return Err(not_usable("the graph's repository is bare"));
    };
    if repo.state() != RepositoryState::Clean {
        return Err(not_usable(format!(
            "the repository is in the middle of a {:?}; finish it first",
            repo.state()
        )));
    }
    let root = std::fs::canonicalize(graph_root).map_err(|e| not_usable(e.to_string()))?;
    let workdir = std::fs::canonicalize(workdir).map_err(|e| not_usable(e.to_string()))?;
    let prefix = root
        .strip_prefix(&workdir)
        .map_err(|_| not_usable("the graph is not inside its repository's work tree"))?
        .to_path_buf();
    let head = repo.find_reference("HEAD").map_err(git_failed)?;
    let Some(branch) = head.symbolic_target().ok().flatten().map(str::to_string) else {
        return Err(not_usable("HEAD is detached; check out a branch first"));
    };
    drop(head);
    Ok(Repo {
        repo,
        prefix,
        branch,
    })
}

impl Repo {
    fn check_raw_content(&self, path: &str) -> Result<(), ErrorBody> {
        if let Some(attr) =
            raw_content_conversion(&self.repo, Path::new(path)).map_err(git_failed)?
        {
            return Err(not_usable(format!(
                "{path} has the Git {attr} attribute; the run cannot commit its raw bytes; use Git's conversion workflow"
            )));
        }
        Ok(())
    }

    /// Bind ref/index updates to the branch captured at open. A checkout
    /// changes the index's owner: refuse, never retry onto that new branch.
    pub(super) fn lock_index(&self, waiting: bool) -> Result<IndexLock, LockRefusal> {
        #[cfg(test)]
        lock_test_hook::pause();
        let lock = if waiting {
            IndexLock::acquire_waiting(&self.repo)
        } else {
            IndexLock::acquire(&self.repo)
        }?;
        let head = self
            .repo
            .find_reference("HEAD")
            .map_err(|e| LockRefusal::Failed(e.message().to_string()))?;
        if head.symbolic_target_bytes() != Some(self.branch.as_bytes()) {
            return Err(LockRefusal::BranchChanged);
        }
        Ok(lock)
    }

    fn path_of(&self, page: &PageId) -> String {
        let path = self.prefix.join(page.as_str());
        path.to_string_lossy().replace('\\', "/")
    }

    pub(super) fn tip(&self) -> Result<Option<git2::Commit<'_>>, ErrorBody> {
        match self.repo.find_reference(&self.branch) {
            Ok(r) => Ok(Some(r.peel_to_commit().map_err(git_failed)?)),
            Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(git_failed(e)),
        }
    }

    pub(super) fn user(&self) -> Signature<'static> {
        self.repo
            .signature()
            .map(|s| s.to_owned())
            .unwrap_or_else(|_| {
                Signature::now(USER_FALLBACK.0, USER_FALLBACK.1).expect("valid fallback")
            })
    }

    /// The blob at `path` in `tree`, if any.
    fn blob_at(&self, tree: Option<&git2::Tree<'_>>, path: &str) -> Option<Oid> {
        let entry = tree?.get_path(Path::new(path)).ok()?;
        (entry.kind() == Some(git2::ObjectType::Blob)).then(|| entry.id())
    }

    /// `tree` with `path` set to `blob` (or removed when `None`).
    pub(super) fn tree_with(
        &self,
        tree: Option<&git2::Tree<'_>>,
        path: &str,
        blob: Option<Oid>,
        mode: i32,
    ) -> Result<Oid, git2::Error> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        self.tree_with_parts(tree, &parts, blob, mode)
    }

    fn tree_with_parts(
        &self,
        tree: Option<&git2::Tree<'_>>,
        parts: &[&str],
        blob: Option<Oid>,
        mode: i32,
    ) -> Result<Oid, git2::Error> {
        let mut builder = self.repo.treebuilder(tree)?;
        let (name, rest) = parts.split_first().expect("a non-empty path");
        if rest.is_empty() {
            match blob {
                Some(blob) => {
                    builder.insert(name, blob, mode)?;
                }
                None => {
                    if builder.get(name)?.is_some() {
                        builder.remove(name)?;
                    }
                }
            }
        } else {
            let sub = match tree.and_then(|t| t.get_name(name)) {
                Some(entry) if entry.kind() == Some(git2::ObjectType::Tree) => {
                    Some(self.repo.find_tree(entry.id())?)
                }
                _ => None,
            };
            let sub = self.tree_with_parts(sub.as_ref(), rest, blob, mode)?;
            if self.repo.find_tree(sub)?.is_empty() {
                if builder.get(name)?.is_some() {
                    builder.remove(name)?;
                }
            } else {
                builder.insert(name, sub, 0o040000)?;
            }
        }
        builder.write()
    }

    fn file_mode(&self, tree: Option<&git2::Tree<'_>>, path: &str) -> i32 {
        tree.and_then(|t| t.get_path(Path::new(path)).ok())
            .map(|e| e.filemode())
            .filter(|m| *m == 0o100755 || *m == 0o100644)
            .unwrap_or(0o100644)
    }

    /// Move the branch from `old` to `new`, failing if it moved meanwhile.
    pub(super) fn advance(
        &self,
        old: Option<Oid>,
        new: Oid,
        log: &str,
    ) -> Result<bool, git2::Error> {
        let result = match old {
            Some(old) => self
                .repo
                .reference_matching(&self.branch, new, true, old, log)
                .map(|_| ()),
            None => self
                .repo
                .reference(&self.branch, new, false, log)
                .map(|_| ()),
        };
        match result {
            Ok(()) => Ok(true),
            Err(e)
                if matches!(
                    e.code(),
                    ErrorCode::Modified | ErrorCode::Exists | ErrorCode::Locked
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}

/// The index, locked by Tine under git's own lock protocol, around a private
/// copy for libgit2 to read and write (its own write would want the lock this
/// holds). `index.lock` is created exclusively first: a held lock means
/// another git command is at work, and the index is left alone. The index is
/// read while the lock is held, and the new index is written into the lock
/// file and renamed over the index, so a concurrent `git add` either finishes
/// before the read or fails on the lock; it is never overwritten. Dropped
/// without [`IndexLock::install`], the lock and the copy are removed and the
/// index is as it was.
///
/// A branch move that the index must follow happens while this is held (ADR
/// 0078 §4, review R2): a moved `HEAD` over an index still naming the old
/// blobs would let the user's next plain `git commit` record a reversion.
pub(super) struct IndexLock {
    lock: std::fs::File,
    lock_file: PathBuf,
    index_file: PathBuf,
    scratch: PathBuf,
    pub(super) index: git2::Index,
    installed: bool,
}

/// The prefix of the private index copies [`IndexLock`] makes in `.git`;
/// a crash can leave one, which [`remove_stale_copies`] removes.
const SCRATCH_PREFIX: &str = "tine-index-";

/// Why [`IndexLock::acquire`] could not take the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LockRefusal {
    /// Another git command holds `index.lock`.
    Held,
    /// The index has unmerged entries (a conflicted `git stash pop`, say).
    Conflicted,
    /// `HEAD` has commits but there is no index: committing from an empty
    /// index would record every file as deleted (`rm .git/index` before a
    /// `git reset`).
    Missing,
    /// An honest checkout changed HEAD before the index lock was taken.
    BranchChanged,
    Failed(String),
}

impl LockRefusal {
    pub(super) fn message(&self) -> String {
        match self {
            Self::Held => {
                "Another git command is using the repository (its index.lock is held).".into()
            }
            Self::Conflicted => {
                "The repository's index has unresolved conflicts; resolve them in git first.".into()
            }
            Self::Missing => {
                "The repository's index is missing; run `git reset` in it to rebuild it.".into()
            }
            Self::BranchChanged => "The active branch changed while the commit was preparing; inspect the saved file and branch before trying again.".into(),
            Self::Failed(m) => m.clone(),
        }
    }
}

/// The entry count a version 2–4 index file's header declares.
fn header_entries(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 12 || &bytes[..4] != b"DIRC" {
        return None;
    }
    Some(u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize)
}

impl IndexLock {
    /// Take `index.lock` and open a private copy of the index under it.
    /// Every honest copy holder holds the lock, so copies present once it is
    /// taken are a crash's leftovers and are removed here, never elsewhere.
    pub(super) fn acquire(repo: &Repository) -> Result<Self, LockRefusal> {
        let gitdir = repo.path();
        let index_file = gitdir.join("index");
        let lock_file = gitdir.join("index.lock");
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_file)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::AlreadyExists => LockRefusal::Held,
                _ => LockRefusal::Failed(format!("{}: {e}", lock_file.display())),
            })?;
        remove_stale_copies(gitdir);
        let scratch = gitdir.join(format!(
            "{SCRATCH_PREFIX}{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let opened = (|| -> Result<git2::Index, LockRefusal> {
            let failed = |what: &dyn std::fmt::Display| {
                LockRefusal::Failed(format!("the index could not be read: {what}"))
            };
            let bytes = match std::fs::read(&index_file) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(failed(&e)),
            };
            let expected = match &bytes {
                None if repo.head().and_then(|h| h.peel_to_commit()).is_ok() => {
                    return Err(LockRefusal::Missing)
                }
                None => None,
                Some(bytes) => {
                    std::fs::write(&scratch, bytes).map_err(|e| failed(&e))?;
                    Some(header_entries(bytes).ok_or_else(|| failed(&"not an index file"))?)
                }
            };
            let index = git2::Index::open(&scratch).map_err(|e| failed(&e.message()))?;
            if expected.is_some_and(|n| n != index.len()) {
                return Err(failed(&"the copy is incomplete"));
            }
            if index.has_conflicts() {
                return Err(LockRefusal::Conflicted);
            }
            Ok(index)
        })();
        match opened {
            Ok(index) => Ok(Self {
                lock,
                lock_file,
                index_file,
                scratch,
                index,
                installed: false,
            }),
            Err(refusal) => {
                let _ = std::fs::remove_file(&scratch);
                let _ = std::fs::remove_file(scratch.with_extension("tmp.lock"));
                drop(lock);
                let _ = std::fs::remove_file(&lock_file);
                Err(refusal)
            }
        }
    }

    /// [`Self::acquire`], waiting up to about a second for a git command that
    /// holds the lock briefly.
    pub(super) fn acquire_waiting(repo: &Repository) -> Result<Self, LockRefusal> {
        let mut attempt = 0;
        loop {
            match Self::acquire(repo) {
                Err(LockRefusal::Held) if attempt < 9 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                other => return other,
            }
        }
    }

    /// Point the entry of `path` at a committed `blob`, unless the user had
    /// other content staged for it than `tip` (what `HEAD` had) or `old` (the
    /// commit's base): then it stays staged, and nothing staged is lost.
    pub(super) fn set_committed(
        &mut self,
        path: &str,
        blob: Option<Oid>,
        size: u32,
        mode: u32,
        tip: Option<Oid>,
        old: Option<Oid>,
    ) -> Result<IndexSync, git2::Error> {
        let staged = self.index.get_path(Path::new(path), 0).map(|e| e.id);
        if staged != tip && staged != old {
            return Ok(IndexSync::StagedKept);
        }
        match blob {
            Some(id) => self.index.add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode,
                uid: 0,
                gid: 0,
                file_size: size,
                id,
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            })?,
            None => {
                let _ = self.index.remove_path(Path::new(path));
            }
        }
        Ok(IndexSync::Synced)
    }

    /// Write the copy into the lock file and rename it over the index.
    pub(super) fn install(mut self) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        self.index.write()?;
        self.lock.write_all(&std::fs::read(&self.scratch)?)?;
        self.lock.sync_all()?;
        std::fs::rename(&self.lock_file, &self.index_file)?;
        self.installed = true;
        Ok(())
    }
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.scratch);
        let _ = std::fs::remove_file(self.scratch.with_extension("tmp.lock"));
        if !self.installed {
            let _ = std::fs::remove_file(&self.lock_file);
        }
    }
}

/// Remove the private index copies a crash left in `gitdir` (never
/// `index.lock`: that may be another git command's).
pub(super) fn remove_stale_copies(gitdir: &Path) {
    let Ok(entries) = std::fs::read_dir(gitdir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(SCRATCH_PREFIX)
            && (name.ends_with(".tmp") || name.ends_with(".tmp.lock"))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What became of the index entry of a committed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexSync {
    Synced,
    /// The user had other content staged for the file; it stays staged.
    StagedKept,
    /// The index was locked, conflicted or changing; it was not touched.
    Busy,
}

impl IndexSync {
    fn code(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::StagedKept => "staged-kept",
            Self::Busy => "busy",
        }
    }
}

/// What the commits of one write were.
pub(crate) struct Committed {
    snapshot: Option<Oid>,
    agent: Oid,
    index: IndexSync,
}

impl Committed {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "committed": true,
            "commit": self.agent.to_string(),
            "snapshot": self.snapshot.map(|o| o.to_string()),
            "index": self.index.code(),
        })
    }
}

/// Commit one write of a run: the user's snapshot of `old` if `HEAD` differs
/// from it, then the agent's `new`. `None` = the file did not exist.
pub(crate) fn commit_write(
    slot: &GraphSlot,
    run: &RunCtx,
    page: &PageId,
    summary: &str,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) -> Result<Committed, ErrorBody> {
    let repo = open_repo(slot)?;
    commit_pair(
        &repo,
        page,
        old,
        new,
        &snapshot_message(page, run.id.as_str()),
        &agent_message(summary, page, run),
        &run.author,
    )
}

fn snapshot_message(page: &PageId, id: &str) -> String {
    format!(
        "Your edits to {} before an agent change\n\n{SNAPSHOT_TRAILER}: {id}\n",
        page.as_str()
    )
}

fn agent_message(summary: &str, page: &PageId, run: &RunCtx) -> String {
    let mut message = format!(
        "{summary}: {}\n\n{TRAILER}: {}\n",
        page.as_str(),
        run.id.as_str()
    );
    if let Some(undone) = &run.reverts {
        message.push_str(&format!("{REVERT_TRAILER}: {}\n", undone.as_str()));
    }
    message
}

fn commit_pair(
    repo: &Repo,
    page: &PageId,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    snapshot_msg: &str,
    agent_msg: &str,
    author: &str,
) -> Result<Committed, ErrorBody> {
    let path = repo.path_of(page);
    // Recheck at the raw-object boundary, even after pre-save admission.
    repo.check_raw_content(&path)?;
    if repo
        .repo
        .status_should_ignore(Path::new(&path))
        .unwrap_or(false)
    {
        return Err(not_usable(format!(
            "{path} is ignored by git; the run does not commit it"
        )));
    }
    let agent = Signature::now(author, AGENT_EMAIL).map_err(git_failed)?;
    let old_blob = old
        .map(|b| repo.repo.blob(b))
        .transpose()
        .map_err(git_failed)?;
    let new_blob = new
        .map(|b| repo.repo.blob(b))
        .transpose()
        .map_err(git_failed)?;
    for _ in 0..CAS_ATTEMPTS {
        let tip = repo.tip()?;
        let tip_tree = tip
            .as_ref()
            .map(|c| c.tree())
            .transpose()
            .map_err(git_failed)?;
        let mode = repo.file_mode(tip_tree.as_ref(), &path);
        let mut parent = tip.clone();
        let mut tree = tip_tree.clone();
        let mut snapshot = None;
        if repo.blob_at(tip_tree.as_ref(), &path) != old_blob {
            let user = repo.user();
            let snap_tree = repo
                .tree_with(tip_tree.as_ref(), &path, old_blob, mode)
                .and_then(|t| repo.repo.find_tree(t))
                .map_err(git_failed)?;
            let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
            let oid = repo
                .repo
                .commit(None, &user, &user, snapshot_msg, &snap_tree, &parents)
                .map_err(git_failed)?;
            snapshot = Some(oid);
            parent = Some(repo.repo.find_commit(oid).map_err(git_failed)?);
            tree = Some(snap_tree);
        }
        let agent_tree = repo
            .tree_with(tree.as_ref(), &path, new_blob, mode)
            .and_then(|t| repo.repo.find_tree(t))
            .map_err(git_failed)?;
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
        let oid = repo
            .repo
            .commit(None, &agent, &agent, agent_msg, &agent_tree, &parents)
            .map_err(git_failed)?;
        // No index lock, no branch move (ADR 0078 review R2).
        let mut lock = match repo.lock_index(true) {
            Ok(lock) => lock,
            Err(LockRefusal::Held) => return Err(super::write::refused(
                "git-busy",
                None,
                "another git command held the repository's index; the file is saved, its commit is missing",
            )),
            Err(refusal) => return Err(super::write::refused(
                "git",
                None,
                format!("{} The file is saved, its commit is missing.", refusal.message()),
            )),
        };
        if repo
            .advance(tip.as_ref().map(|c| c.id()), oid, "tine run: commit")
            .map_err(git_failed)?
        {
            let index = match lock.set_committed(
                &path,
                new_blob,
                new.map_or(0, |b| b.len() as u32),
                mode as u32,
                repo.blob_at(tip_tree.as_ref(), &path),
                old_blob,
            ) {
                Ok(IndexSync::Synced) => match lock.install() {
                    Ok(()) => IndexSync::Synced,
                    Err(_) => IndexSync::Busy,
                },
                Ok(other) => other,
                Err(_) => IndexSync::Busy,
            };
            return Ok(Committed {
                snapshot,
                agent: oid,
                index,
            });
        }
    }
    Err(super::write::refused(
        "git-busy",
        None,
        "the branch kept moving while the run committed; the file is saved, its commit is missing",
    ))
}

// ---- verbs ---------------------------------------------------------------

/// `run.begin`: check the repository, mint an id.
pub(crate) fn run_begin(slot: &GraphSlot) -> Result<Value, ErrorBody> {
    let repo = open_repo(slot)?;
    let id = RunId::mint(chrono::Utc::now().timestamp());
    Ok(ok(
        "run",
        json!({
            "run": id.as_str(),
            "branch": repo.branch.trim_start_matches("refs/heads/"),
        }),
    ))
}

/// The commit carries `trailer: id` in its trailer block (git's own parse,
/// as `git interpret-trailers --parse`), not merely somewhere in its prose.
fn has_trailer(commit: &git2::Commit<'_>, trailer: &str, id: &str) -> bool {
    let Ok(message) = commit.message_raw() else {
        return false;
    };
    git2::message_trailers_strs(message)
        .is_ok_and(|t| t.iter().any(|(k, v)| k == trailer && v.trim() == id))
}

/// First-parent history from the tip back to just before the run began.
fn window<'r>(repo: &'r Repo, id: &RunId) -> Result<Vec<git2::Commit<'r>>, ErrorBody> {
    let mut out = Vec::new();
    let mut next = repo.tip()?;
    while let Some(commit) = next {
        if out.len() >= WALK_LIMIT || commit.time().seconds() < id.begun - WINDOW_SLACK_SECS {
            break;
        }
        next = commit.parents().next();
        out.push(commit);
    }
    Ok(out)
}

fn subject(commit: &git2::Commit<'_>) -> String {
    commit.summary().ok().flatten().unwrap_or("").to_string()
}

/// `run.show`: the run's commits, newest first.
pub(crate) fn run_show(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let id = id_arg(args)?;
    let repo = open_repo(slot)?;
    let commits: Vec<Value> = window(&repo, &id)?
        .iter()
        .filter(|c| {
            has_trailer(c, TRAILER, id.as_str()) || has_trailer(c, SNAPSHOT_TRAILER, id.as_str())
        })
        .map(|c| {
            json!({
                "commit": c.id().to_string(),
                "agent": has_trailer(c, TRAILER, id.as_str()),
                "author": c.author().name().ok().unwrap_or(""),
                "subject": subject(c),
            })
        })
        .collect();
    Ok(ok(
        "run-commits",
        json!({ "run": id.as_str(), "commits": commits }),
    ))
}

fn id_arg(args: &Value) -> Result<RunId, ErrorBody> {
    let text = args.get("run").and_then(Value::as_str).unwrap_or("");
    RunId::parse(text)
        .ok_or_else(|| ErrorBody::new("bad-args", format!("`{text}` is not a run id"), false))
}

/// `run.end`: squash the run into one commit when that rewrites nothing
/// anyone else can see; otherwise leave its commits as they are.
pub(crate) fn run_end(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    let id = id_arg(args)?;
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let repo = open_repo(slot)?;
    let reply = |squashed: Option<Oid>, kept: usize, why: Option<&str>| {
        ok(
            "run-end",
            json!({
                "run": id.as_str(),
                "squashed": squashed.map(|o| o.to_string()),
                "commits": kept,
                "kept_because": why,
            }),
        )
    };
    let history = window(&repo, &id)?;
    let run: Vec<&git2::Commit<'_>> = history
        .iter()
        .filter(|c| has_trailer(c, TRAILER, id.as_str()))
        .collect();
    if run.is_empty() {
        return Ok(reply(None, 0, Some("the run made no commits")));
    }
    let leading = history
        .iter()
        .take_while(|c| has_trailer(c, TRAILER, id.as_str()))
        .count();
    if leading != run.len() {
        return Ok(reply(
            None,
            run.len(),
            Some("other commits are between the run's commits"),
        ));
    }
    if run.len() == 1 {
        return Ok(reply(None, 1, Some("the run is one commit already")));
    }
    let oldest = run[run.len() - 1];
    if let Some(name) = reachable_elsewhere(&repo, oldest.id())? {
        return Ok(reply(
            None,
            run.len(),
            Some(&format!("its commits are already on {name}")),
        ));
    }
    let tip = run[0];
    let base = oldest.parents().next();
    let author = tip.author().to_owned();
    let mut body = String::new();
    for commit in run.iter().rev() {
        body.push_str("- ");
        body.push_str(&subject(commit));
        body.push('\n');
    }
    let title = message
        .map(str::to_string)
        .unwrap_or_else(|| format!("Agent run {}", id.as_str()));
    let full = format!("{title}\n\n{body}\n{TRAILER}: {}\n", id.as_str());
    let tree = tip.tree().map_err(git_failed)?;
    let parents: Vec<&git2::Commit<'_>> = base.iter().collect();
    let squashed = repo
        .repo
        .commit(None, &author, &author, &full, &tree, &parents)
        .map_err(git_failed)?;
    if !repo
        .advance(Some(tip.id()), squashed, "tine run: squash")
        .map_err(git_failed)?
    {
        return Ok(reply(
            None,
            run.len(),
            Some("the branch moved while the run ended"),
        ));
    }
    Ok(reply(Some(squashed), run.len(), None))
}

/// A ref other than the run's branch (a remote-tracking branch, another
/// branch, a tag) from which `commit` is reachable.
fn reachable_elsewhere(repo: &Repo, commit: Oid) -> Result<Option<String>, ErrorBody> {
    for reference in repo.repo.references().map_err(git_failed)? {
        let reference = reference.map_err(git_failed)?;
        let Ok(name) = reference.name() else { continue };
        if name == repo.branch || name == "HEAD" || name.starts_with("refs/stash") {
            continue;
        }
        let Ok(target) = reference.peel_to_commit() else {
            continue;
        };
        if target.id() == commit
            || repo
                .repo
                .graph_descendant_of(target.id(), commit)
                .unwrap_or(false)
        {
            return Ok(Some(name.trim_start_matches("refs/").to_string()));
        }
    }
    Ok(None)
}

/// One page's part of a revert: its before/after per agent commit, newest first.
pub(crate) struct PageSteps {
    pub(crate) page: PageId,
    pub(crate) steps: Vec<(Option<Vec<u8>>, Option<Vec<u8>>)>,
}

/// The pages the run's agent commits touched, with their texts.
pub(crate) fn revert_plan(slot: &GraphSlot, id: &RunId) -> Result<Vec<PageSteps>, ErrorBody> {
    let repo = open_repo(slot)?;
    let history = window(&repo, id)?;
    let mut pages: Vec<PageSteps> = Vec::new();
    for commit in history
        .iter()
        .filter(|c| has_trailer(c, TRAILER, id.as_str()))
    {
        let tree = commit.tree().map_err(git_failed)?;
        let parent_tree = commit
            .parents()
            .next()
            .map(|p| p.tree())
            .transpose()
            .map_err(git_failed)?;
        let diff = repo
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(git_failed)?;
        for delta in diff.deltas() {
            let Some(path) = delta.new_file().path().or(delta.old_file().path()) else {
                continue;
            };
            let Ok(rel) = path.strip_prefix(&repo.prefix) else {
                continue;
            };
            let Some(rel) = rel.to_str() else { continue };
            let page = PageId::from(rel.replace('\\', "/"));
            let read = |oid: Oid| -> Result<Option<Vec<u8>>, ErrorBody> {
                if oid.is_zero() {
                    return Ok(None);
                }
                Ok(Some(
                    repo.repo
                        .find_blob(oid)
                        .map_err(git_failed)?
                        .content()
                        .to_vec(),
                ))
            };
            let before = read(delta.old_file().id())?;
            let after = read(delta.new_file().id())?;
            match pages.iter_mut().find(|p| p.page == page) {
                Some(entry) => entry.steps.push((before, after)),
                None => pages.push(PageSteps {
                    page,
                    steps: vec![(before, after)],
                }),
            }
        }
    }
    Ok(pages)
}

/// `run.revert`: undo the run's agent commits, newest first (D-20261019-CL3).
/// A block the user changed since keeps the user's text and is listed.
///
/// The revert is itself a run with a new id (its commits carry `Tine-Run:
/// <new id>` and `Tine-Run-Revert: <undone id>`), so `run revert <new id>`
/// undoes the revert in one command. Every page is read and merged before any
/// is written: a page that cannot be read or merged refuses the revert, with
/// nothing changed. Pages are then saved one by one; a page whose save fails
/// later is listed with its error, and the pages saved before it stay
/// reverted (each one's commit is part of the revert run).
pub(crate) fn run_revert(slot: &GraphSlot, args: &Value) -> Result<Value, ErrorBody> {
    use tine_graph_features::run_revert::{revert_page, RevertStep};
    let id = id_arg(args)?;
    // The same pre-write gate as a `--run` write (index lock, repository state).
    check_repository(slot)?;
    let plan = revert_plan(slot, &id)?;
    let ctx = RunCtx {
        id: RunId::mint(chrono::Utc::now().timestamp()),
        author: author_of(args),
        reverts: Some(id.clone()),
    };
    let texts: Vec<Vec<(String, String)>> = plan
        .iter()
        .map(|entry| {
            entry
                .steps
                .iter()
                .map(|(before, after)| (text_of(before.as_deref()), text_of(after.as_deref())))
                .collect()
        })
        .collect();
    // One pass over the pages: `write` false merges and stops before saving.
    let pass = |write: bool, outcome: &mut Vec<Value>| -> Vec<(String, ErrorBody)> {
        let mut refused = Vec::new();
        for (entry, texts) in plan.iter().zip(&texts) {
            let steps: Vec<RevertStep<'_>> = texts
                .iter()
                .map(|(before, after)| RevertStep { before, after })
                .collect();
            let mut kept = Vec::new();
            let mut unchanged = false;
            let mut merged = false;
            let compute = |_: &_, page: &PageId, bytes: &[u8]| {
                let current = std::str::from_utf8(bytes).map_err(|_| {
                    super::write::refused("store", Some(page), "the page is not UTF-8 text")
                })?;
                let reverted = revert_page(&slot.store, page, current, &steps)
                    .map_err(|e| super::write::refused("merge", Some(page), e.to_string()))?;
                kept = reverted.kept;
                merged = true;
                if !reverted.changed {
                    unchanged = true;
                    return Err(ErrorBody::new(UNCHANGED, "", false));
                }
                if !write {
                    return Err(ErrorBody::new(DRY_RUN, "", false));
                }
                Ok(super::write::Planned {
                    doc: reverted.doc,
                    changed: Vec::new(),
                })
            };
            let result = super::write::in_run(Some(ctx.clone()), "Revert agent run", || {
                super::write::write_page_from_bytes(
                    slot,
                    &entry.page,
                    tine_store::EditKind::ReplacePage,
                    None,
                    false,
                    compute,
                )
            });
            if !write {
                if let Err(error) = result {
                    if !merged {
                        refused.push((entry.page.as_str().to_string(), error));
                    }
                }
                continue;
            }
            let kept: Vec<Value> = kept
                .iter()
                .map(|k| json!({ "text": k.text, "reason": k.reason.code() }))
                .collect();
            outcome.push(match result {
                Ok(reply) => json!({
                    "page": entry.page.as_str(),
                    "reverted": true,
                    "rev": reply.get("rev"),
                    "git": reply.get("git"),
                    "kept": kept,
                }),
                Err(_) if unchanged => json!({
                    "page": entry.page.as_str(),
                    "reverted": false,
                    "kept": kept,
                }),
                Err(error) => json!({
                    "page": entry.page.as_str(),
                    "reverted": false,
                    "error": reason_of(&error),
                    "message": error.message,
                }),
            });
        }
        refused
    };
    let refused = pass(false, &mut Vec::new());
    if let Some((page, error)) = refused.into_iter().next() {
        return Err(super::write::refused(
            "merge",
            None,
            format!(
                "{page} cannot be reverted ({}): {}; nothing was changed",
                reason_of(&error),
                error.message
            ),
        ));
    }
    let mut pages = Vec::new();
    pass(true, &mut pages);
    Ok(ok(
        "run-revert",
        json!({ "run": id.as_str(), "revert_run": ctx.id.as_str(), "pages": pages }),
    ))
}

/// `refused:<reason>` as the text output spells it, else the code.
pub(crate) fn reason_of(error: &ErrorBody) -> String {
    match error
        .detail
        .as_ref()
        .and_then(|d| d.get("reason"))
        .and_then(Value::as_str)
    {
        Some(reason) => format!("{}:{reason}", error.code),
        None => error.code.clone(),
    }
}

/// The sentinel a revert's compute returns when the page needs no write.
const UNCHANGED: &str = "unchanged";
/// The sentinel a revert's checking pass returns instead of saving.
const DRY_RUN: &str = "dry-run";

fn text_of(bytes: Option<&[u8]>) -> String {
    bytes
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) mod lock_test_hook {
    thread_local! {
        static HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
            std::cell::RefCell::new(None)
        };
    }

    pub(crate) fn install(hook: impl FnOnce() + 'static) {
        HOOK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn pause() {
        if let Some(hook) = HOOK.with(|h| h.borrow_mut().take()) {
            hook();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_round_trip_and_reject_junk() {
        let id = RunId::mint(0x6713_a2bc);
        assert_eq!(RunId::parse(id.as_str()), Some(id.clone()));
        assert_eq!(id.begun, 0x6713_a2bc);
        for bad in [
            "",
            "r",
            "r12-",
            "rzz-12345678",
            "r12-1234567",
            "r12-ABCDEF12",
            "x12-12345678",
            "r12-12345678-1",
        ] {
            assert_eq!(RunId::parse(bad), None, "{bad}");
        }
    }
}
