//! Tine's own commits on git graphs (ADR 0078 slice 1; journal
//! D-20261018-G2/G3/G5, D-20261019-G8/G9).
//!
//! An autocommit records every change git sees under the graph root,
//! honouring `.gitignore`: Tine's saves, other programs' edits, deletions.
//! It is built on a private copy of the index taken under `index.lock`
//! ([`IndexLock`]): the graph root's changes are staged into the copy, its
//! tree is written, only the graph root's subtree goes into `HEAD`'s tree (a
//! user's staged changes outside the graph are never committed), the branch
//! moves by compare-and-swap while the lock is held, and the copy replaces
//! the index. The graph root is matched as a literal path prefix, never as a
//! pathspec pattern (a folder named `Notes [work]` is a glob).
//!
//! When (G9): when nothing was published for [`IDLE`] and the interval
//! (a setting, default 10 min) has passed since the last autocommit; at once
//! when the binding ends or the app exits; and around a structural
//! operation ([`around`]). One worker thread per graph root does the timed
//! commits; libgit2 never runs on an async worker. Autocommits take the CLI
//! runs' lock ([`RUN_LOCK`]), so they never fall between a run write's
//! snapshot and its agent commit.
//!
//! Only `.git` objects, refs and the index are normally written here. The
//! single graph-file exception (D-20261026-GI) appends `.syncthing.*` once
//! to the root `.gitignore` through Store's guarded non-page transaction.
//! Repacking deletes only loose objects contained in the pack it
//! just wrote and fsynced (git's `prune-packed`), never unreachable ones.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use git2::{Oid, Repository, Status, StatusOptions};
use serde::Serialize;

#[cfg(test)]
use super::git_run::IndexLock;
use super::git_run::{
    open_repo_at, raw_content_conversion as converting, LockRefusal, Repo, CAS_ATTEMPTS, RUN_LOCK,
};
use crate::state::GraphSlot;

/// No publication for this long before a timed autocommit (G9).
pub(crate) const IDLE: Duration = Duration::from_secs(60);
/// The default interval between timed autocommits (G9; a setting).
pub(crate) const DEFAULT_INTERVAL_MINUTES: u32 = 10;
const MIN_INTERVAL_MINUTES: u32 = 1;
const MAX_INTERVAL_MINUTES: u32 = 24 * 60;
/// How often the worker looks at the graph's publication revision.
const TICK: Duration = Duration::from_secs(15);
/// Tine's commits between repacks (G9).
const REPACK_COMMITS: u32 = 50;
/// Loose objects that trigger a repack regardless (git's `gc.auto` default).
const REPACK_LOOSE_OBJECTS: usize = 6700;
/// Failed maintenance must not run again on every commit/tick.
const REPACK_RETRY: Duration = Duration::from_secs(10 * 60);
/// Busy cycles over this long before Settings names a stale `index.lock`.
const STALE_LOCK_AFTER: Duration = Duration::from_secs(30 * 60);
const STALE_LOCK_CYCLES: u32 = 3;
/// The device-settings key (`tine-settings.json`), keyed by graph root.
pub(crate) const SETTINGS_KEY: &str = "git_autocommit";
const MESSAGE: &str = "Tine: autosave";
const SYNCTHING_IGNORE_CHECKED: &str = "git-syncthing-ignore-checked";
const SYNCTHING_IGNORE_LINE: &[u8] = b".syncthing.*";
static IGNORE_SETUP_LOCK: Mutex<()> = Mutex::new(());
/// What the exit flush leaves an aborting commit to release its lock.
const EXIT_GRACE: Duration = Duration::from_millis(500);

/// The app is exiting: a commit in flight aborts at its next file or before
/// it moves the branch, and none starts (review N2: a commit killed by the
/// exit would leave `index.lock` behind).
static EXITING: AtomicBool = AtomicBool::new(false);

/// What Tine does with a git graph on this device (ADR 0078 §1). Slice 1
/// offers `Off` and `Commit`; sync arrives with slice 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Policy {
    #[default]
    Off,
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GitSettings {
    pub(crate) policy: Policy,
    pub(crate) interval: Duration,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            policy: Policy::Off,
            interval: minutes(DEFAULT_INTERVAL_MINUTES),
        }
    }
}

fn minutes(n: u32) -> Duration {
    Duration::from_secs(u64::from(n) * 60)
}

pub(crate) fn clamp_interval(minutes: u32) -> u32 {
    minutes.clamp(MIN_INTERVAL_MINUTES, MAX_INTERVAL_MINUTES)
}

/// The graph root's key in the settings map.
pub(crate) fn settings_key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

/// This graph's settings from the device settings JSON (absent or malformed
/// entries are the defaults: Off).
pub(crate) fn settings_from_json(json: &serde_json::Value, root: &Path) -> GitSettings {
    let entry = &json[SETTINGS_KEY][settings_key(root)];
    let policy = serde_json::from_value(entry["policy"].clone()).unwrap_or_default();
    let interval = entry["interval_minutes"]
        .as_u64()
        .map(|m| clamp_interval(m.min(u64::from(u32::MAX)) as u32))
        .unwrap_or(DEFAULT_INTERVAL_MINUTES);
    GitSettings {
        policy,
        interval: minutes(interval),
    }
}

/// Write this graph's settings into the device settings JSON.
pub(crate) fn settings_into_json(json: &mut serde_json::Value, root: &Path, settings: GitSettings) {
    if !json[SETTINGS_KEY].is_object() {
        json[SETTINGS_KEY] = serde_json::json!({});
    }
    json[SETTINGS_KEY][settings_key(root)] = serde_json::json!({
        "policy": settings.policy,
        "interval_minutes": settings.interval.as_secs() / 60,
    });
}

// ---- the repository ------------------------------------------------------

/// One durable attempt per device/graph, before any graph write. Marking first
/// favours respecting a later removal over retrying after a crash or I/O error.
/// A broader/parent git rule counts; staging remains the unconditional backstop.
pub(crate) fn prepare_syncthing_ignore(
    store: &tine_store::Store,
    root: &Path,
    app_data: &Path,
) -> Result<bool, String> {
    let _setup = IGNORE_SETUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if crate::settings::notice_dismissed(app_data, root, SYNCTHING_IGNORE_CHECKED) {
        return Ok(false);
    }
    let repo = open_checked(root).map_err(|e| e.message())?;
    let ignored = repo
        .repo
        .status_should_ignore(&repo.prefix.join(".syncthing.tine-ignore-check.tmp"))
        .map_err(|e| e.to_string())?;
    crate::settings::set_notice_at(app_data, root, SYNCTHING_IGNORE_CHECKED, true)?;
    if ignored {
        return Ok(false);
    }
    append_syncthing_ignore(store)?;
    Ok(true)
}

/// Fixed non-page target; read exact bytes once, then create without replacing
/// an occupied path or replace against that byte revision under Store's lock.
fn append_syncthing_ignore(store: &tine_store::Store) -> Result<(), String> {
    use tine_store::{Content, FileId, StoreError, TxOutcome};
    let id = FileId::from(".gitignore".to_owned());
    let (mut bytes, base) = match store.read(&id, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
        Ok((bytes, rev)) => (bytes, Some(rev)),
        Err(StoreError::NotFound) => (Vec::new(), None),
        Err(error) => return Err(format!("{error:?}")),
    };
    // Use the first existing newline's style, preserving every original byte.
    let newline: &[u8] = match bytes.iter().position(|b| *b == b'\n') {
        Some(at) if at > 0 && bytes[at - 1] == b'\r' => b"\r\n",
        _ => b"\n",
    };
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.extend_from_slice(newline);
    }
    bytes.extend_from_slice(SYNCTHING_IGNORE_LINE);
    bytes.extend_from_slice(newline);
    let mut tx = store.transaction(None);
    if let Some(base) = base {
        tx.replace(&id, base, bytes);
    } else {
        tx.create(&id, Content::Bytes(bytes));
    }
    match tx.commit() {
        TxOutcome::Committed { .. } => Ok(()),
        other => {
            crate::debug::diag_private("git-ignore-write-failed", format!("{other:?}"));
            Err("the file changed or could not be safely written".into())
        }
    }
}

/// Keep this setup result on the existing Settings git status surface. A later
/// no-op check must not erase the notice before the user can read it.
pub(crate) fn report_ignore_setup(root: &Path, result: Result<bool, String>) {
    if let Some(worker) = registry::get(root) {
        let mut state = worker.state.lock().unwrap();
        match result {
            Ok(true) => {
                state.last.syncthing_ignore_added =
                    Some(root.join(".gitignore").display().to_string());
                state.last.syncthing_ignore_failed = false;
            }
            Ok(false) => {}
            Err(error) => {
                crate::debug::diag_private("git-ignore-setup-failed", error);
                state.last.syncthing_ignore_failed = true;
            }
        }
    }
}

/// Why the graph's repository cannot take an autocommit (ADR 0078 §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unusable {
    /// Not inside a git work tree: Tine offers nothing.
    NotARepository,
    /// Bare, mid-merge, detached, or not inside its work tree.
    Repository(String),
    /// eol or filter conversion of graph text: git's bytes would differ from
    /// Tine's files.
    Conversion(String),
    /// Sparse checkout: excluded files would be committed as deleted.
    Sparse,
    /// macOS/iOS without `core.precomposeunicode`: decomposed names would
    /// commit every diacritic page as a delete plus an add.
    #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
    Precompose,
    /// Another git command holds `index.lock` (retried).
    Busy,
    /// The index has unresolved conflicts.
    Conflicted,
    /// Tine is quitting; the commit was abandoned before it changed anything.
    Quitting,
    /// libgit2 or the file system failed.
    Failed(String),
}

impl Unusable {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::NotARepository => "not-a-repository",
            Self::Repository(_) => "repository",
            Self::Conversion(_) => "conversion",
            Self::Sparse => "sparse",
            Self::Precompose => "precompose",
            Self::Busy => "busy",
            Self::Conflicted => "conflicted",
            Self::Quitting => "quitting",
            Self::Failed(_) => "failed",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotARepository => "The graph is not in a git repository.".into(),
            Self::Repository(m) | Self::Failed(m) => m.clone(),
            Self::Conversion(m) => format!(
                "the repository filters or re-encodes this graph's files ({m}); committing them as they are on disk would bypass that"
            ),
            Self::Sparse => "The repository uses sparse checkout; Tine would record the files it leaves out as deleted.".into(),
            Self::Precompose => "On this Mac the repository needs `git config core.precomposeunicode true`, or page names with accents would be recorded as renames.".into(),
            Self::Busy => LockRefusal::Held.message(),
            Self::Conflicted => LockRefusal::Conflicted.message(),
            Self::Quitting => "Tine quit before this commit finished; it is made on the next start.".into(),
        }
    }
}

fn failed(error: git2::Error) -> Unusable {
    Unusable::Failed(format!("git: {}", error.message()))
}

fn lock_refused(refusal: LockRefusal) -> Unusable {
    match refusal {
        LockRefusal::Held => Unusable::Busy,
        LockRefusal::Conflicted => Unusable::Conflicted,
        missing @ LockRefusal::Missing => Unusable::Repository(missing.message()),
        changed @ LockRefusal::BranchChanged => Unusable::Repository(changed.message()),
        LockRefusal::Failed(m) => Unusable::Failed(m),
    }
}

/// Open and check the repository holding `root` for autocommits.
fn open_checked(root: &Path) -> Result<Repo, Unusable> {
    match Repository::discover(root) {
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            return Err(Unusable::NotARepository)
        }
        Err(error) => return Err(failed(error)),
        Ok(_) => {}
    }
    let repo = open_repo_at(root).map_err(|error| Unusable::Repository(error.message))?;
    check_config(&repo)?;
    Ok(repo)
}

/// Refuse what would make a raw-bytes commit wrong. Autocommits store the
/// files' exact bytes, so eol normalisation (`core.autocrlf`, `text`, `eol`)
/// only costs `git status` noise here; sync refuses it (G5), since a checkout
/// would rewrite the files. A content filter (git-lfs, git-crypt), `ident` or
/// `working-tree-encoding` changes what the repository should hold: committing
/// the working tree's bytes would bypass it (an encrypted file pushed in
/// clear), so those refuse.
pub(super) fn check_config(repo: &Repo) -> Result<(), Unusable> {
    let config = repo.repo.config().map_err(failed)?;
    let flag = |key: &str| config.get_bool(key).ok();
    // Representative graph paths, so Settings can say so up front;
    // `stage` checks every file it adds (`.gitattributes` applies by pattern).
    for sample in [
        "pages/a.md",
        "journals/a.md",
        "pages/a.org",
        "a.md",
        "logseq/config.edn",
        "assets/a.png",
    ] {
        if let Some(attr) = converting(&repo.repo, &repo.prefix.join(sample)).map_err(failed)? {
            return Err(Unusable::Conversion(format!("{attr} for {sample}")));
        }
    }
    if flag("core.sparseCheckout") == Some(true) {
        return Err(Unusable::Sparse);
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if flag("core.precomposeunicode") != Some(true) {
        return Err(Unusable::Precompose);
    }
    Ok(())
}

/// Skip-worktree entries under the graph root (sparse checkout without the
/// config flag, `git update-index --skip-worktree`).
fn has_skip_worktree(index: &git2::Index, prefix: &str) -> bool {
    const SKIP_WORKTREE: u16 = 1 << 14;
    index
        .iter()
        .any(|entry| entry.flags_extended & SKIP_WORKTREE != 0 && under(&entry.path, prefix))
}

/// `path` (work-tree relative, `/`-separated) lies under `prefix` ("" = the
/// whole work tree), compared literally.
fn under(path: &[u8], prefix: &str) -> bool {
    prefix.is_empty()
        || (path.len() > prefix.len()
            && path.starts_with(prefix.as_bytes())
            && path[prefix.len()] == b'/')
}

fn prefix_of(repo: &Repo) -> String {
    repo.prefix.to_string_lossy().replace('\\', "/")
}

fn syncthing_temp(path: &[u8]) -> bool {
    path.split(|b| *b == b'/')
        .any(|part| part.starts_with(b".syncthing."))
}

/// One of Tine's own in-flight atomic-save temp files
/// (`.{name}.{pid}.{seq}{tag}.tmp`, `tine_store::atomic_file::temp_path`).
fn tine_temp(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some(stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    let Some((before, seq)) = stem.rsplit_once('.') else {
        return false;
    };
    let Some((target, pid)) = before.rsplit_once('.') else {
        return false;
    };
    let digits = seq.bytes().take_while(u8::is_ascii_digit).count();
    target.starts_with('.')
        && !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && digits > 0
}

/// What an autocommit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Committed(Oid),
    /// Nothing under the graph root differed from `HEAD`.
    Clean,
}

/// Commit every change under the graph root with `message`, holding the CLI
/// runs' lock.
pub(crate) fn commit_now(root: &Path, message: &str) -> Result<Outcome, Unusable> {
    let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    commit_locked(root, message)
}

/// [`commit_now`] for a caller that already holds [`RUN_LOCK`].
fn commit_locked(root: &Path, message: &str) -> Result<Outcome, Unusable> {
    let repo = open_checked(root)?;
    let prefix = prefix_of(&repo);
    for _ in 0..CAS_ATTEMPTS {
        if EXITING.load(Ordering::Acquire) {
            return Err(Unusable::Quitting);
        }
        let mut lock = repo.lock_index(false).map_err(lock_refused)?;
        if has_skip_worktree(&lock.index, &prefix) {
            return Err(Unusable::Sparse);
        }
        let tip = repo.tip().map_err(|e| Unusable::Failed(e.message))?;
        let tip_tree = tip.as_ref().map(|c| c.tree()).transpose().map_err(failed)?;
        stage(&repo, &prefix, &mut lock.index)?;
        repo.guard
            .check_git(&repo.repo, &repo.branch)
            .map_err(failed)?;
        let staged = lock.index.write_tree_to(&repo.repo).map_err(failed)?;
        let tree = if prefix.is_empty() {
            staged
        } else {
            let staged = repo.repo.find_tree(staged).map_err(failed)?;
            let sub = staged.get_path(Path::new(&prefix)).ok().map(|e| e.id());
            match sub {
                Some(sub) => repo.tree_with(tip_tree.as_ref(), &prefix, Some(sub), 0o040000),
                None => repo.tree_with(tip_tree.as_ref(), &prefix, None, 0o040000),
            }
            .map_err(failed)?
        };
        if tip_tree.as_ref().map(|t| t.id()) == Some(tree) {
            // The copy may hold refreshed entries; the index is left as it was.
            return Ok(Outcome::Clean);
        }
        let tree = repo.repo.find_tree(tree).map_err(failed)?;
        let user = repo.user();
        let parents: Vec<&git2::Commit<'_>> = tip.iter().collect();
        repo.guard
            .check_git(&repo.repo, &repo.branch)
            .map_err(failed)?;
        let oid = repo
            .repo
            .commit(None, &user, &user, message, &tree, &parents)
            .map_err(failed)?;
        if EXITING.load(Ordering::Acquire) {
            return Err(Unusable::Quitting);
        }
        if repo
            .advance(tip.as_ref().map(|c| c.id()), oid, "tine: autocommit")
            .map_err(failed)?
        {
            lock.install()
                .map_err(|e| Unusable::Failed(format!("the index could not be written: {e}")))?;
            return Ok(Outcome::Committed(oid));
        }
    }
    Err(Unusable::Busy)
}

/// Stage every change git sees under the graph root into `index` (a copy):
/// new and modified regular files with their exact bytes, deletions removed.
/// Ignored files, untracked nested repositories, symlinks, Tine's own temp
/// files and Syncthing's in-flight copies are left out. A file `.gitattributes`
/// filters or re-encodes refuses the whole commit (git-lfs, git-crypt: its raw bytes must not be
/// committed). A file the user untracked with `git rm --cached` but did not
/// ignore is added back: G5 commits every change git sees, and `.gitignore`
/// is how a file stays out.
fn stage(repo: &Repo, prefix: &str, index: &mut git2::Index) -> Result<(), Unusable> {
    let graph_start = if prefix.is_empty() {
        0
    } else {
        prefix.len() + 1
    };
    // A copied index can already contain a temp staged by another git program.
    // Removing it here also retires old tracked temps without touching disk.
    let temps: Vec<Vec<u8>> = index
        .iter()
        .filter(|e| under(&e.path, prefix) && syncthing_temp(&e.path[graph_start..]))
        .map(|e| e.path)
        .collect();
    for path in temps {
        let path = std::str::from_utf8(&path)
            .map_err(|_| Unusable::Failed("a Syncthing temp path is not UTF-8".into()))?;
        index.remove_path(Path::new(path)).map_err(failed)?;
    }
    let workdir = repo
        .repo
        .workdir()
        .ok_or_else(|| Unusable::Repository("the graph's repository is bare".into()))?
        .to_path_buf();
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .exclude_submodules(true)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    if !prefix.is_empty() {
        options.pathspec(prefix).disable_pathspec_match(true);
    }
    let statuses = repo.repo.statuses(Some(&mut options)).map_err(failed)?;
    let filemode = repo
        .repo
        .config()
        .ok()
        .and_then(|c| c.get_bool("core.filemode").ok())
        .unwrap_or(cfg!(unix));
    for entry in statuses.iter() {
        let path = entry.path_bytes();
        if !under(path, prefix) || path.ends_with(b"/") {
            continue;
        }
        let Ok(path) = std::str::from_utf8(path) else {
            continue;
        };
        if tine_temp(path) || syncthing_temp(&path.as_bytes()[graph_start..]) {
            continue;
        }
        if EXITING.load(Ordering::Acquire) {
            return Err(Unusable::Quitting);
        }
        let status = entry.status();
        if !status.intersects(
            Status::WT_NEW
                | Status::WT_MODIFIED
                | Status::WT_DELETED
                | Status::WT_TYPECHANGE
                | Status::WT_RENAMED,
        ) {
            continue;
        }
        let full = workdir.join(path);
        match std::fs::symlink_metadata(&full) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = index.remove_path(Path::new(path));
            }
            Err(e) => return Err(Unusable::Failed(format!("{path}: {e}"))),
            Ok(meta) if !meta.is_file() => {}
            Ok(meta) => {
                if let Some(attr) = converting(&repo.repo, Path::new(path)).map_err(failed)? {
                    return Err(Unusable::Conversion(format!("{attr} for {path}")));
                }
                let io = |e: std::io::Error| Unusable::Failed(format!("{path}: {e}"));
                // Streamed: an asset is never held in memory whole.
                let mut file = std::fs::File::open(&full).map_err(io)?;
                repo.guard
                    .check_git(&repo.repo, &repo.branch)
                    .map_err(failed)?;
                let mut writer = repo.repo.blob_writer(None).map_err(failed)?;
                let size =
                    copy_unless_exiting(&mut file, &mut writer).map_err(|e| match e.kind() {
                        std::io::ErrorKind::Interrupted => Unusable::Quitting,
                        _ => io(e),
                    })?;
                repo.guard
                    .check_git(&repo.repo, &repo.branch)
                    .map_err(failed)?;
                let id = writer.commit().map_err(failed)?;
                let old_mode = index.get_path(Path::new(path), 0).map(|e| e.mode);
                index
                    .add(&entry_for(path, id, size, &meta, filemode, old_mode))
                    .map_err(failed)?;
            }
        }
    }
    Ok(())
}

/// Copy `from` into `into` in 1 MiB chunks, stopping with `Interrupted` once
/// the app is exiting: a single large asset must not keep `index.lock` held
/// past the exit flush (review round 2, N2).
fn copy_unless_exiting(
    from: &mut impl std::io::Read,
    into: &mut impl std::io::Write,
) -> std::io::Result<u64> {
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if EXITING.load(Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Tine is quitting",
            ));
        }
        let n = match from.read(&mut buffer) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        into.write_all(&buffer[..n])?;
        total += n as u64;
    }
}

fn entry_for(
    path: &str,
    id: Oid,
    size: u64,
    meta: &std::fs::Metadata,
    filemode: bool,
    old_mode: Option<u32>,
) -> git2::IndexEntry {
    #[cfg(unix)]
    let (ctime, mtime, dev, ino, uid, gid, exec) = {
        use std::os::unix::fs::MetadataExt;
        (
            git2::IndexTime::new(meta.ctime() as i32, meta.ctime_nsec() as u32),
            git2::IndexTime::new(meta.mtime() as i32, meta.mtime_nsec() as u32),
            meta.dev() as u32,
            meta.ino() as u32,
            meta.uid(),
            meta.gid(),
            meta.mode() & 0o111 != 0,
        )
    };
    #[cfg(not(unix))]
    let (ctime, mtime, dev, ino, uid, gid, exec) = {
        let at = |t: std::io::Result<std::time::SystemTime>| {
            t.ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| git2::IndexTime::new(d.as_secs() as i32, d.subsec_nanos()))
                .unwrap_or_else(|| git2::IndexTime::new(0, 0))
        };
        (at(meta.created()), at(meta.modified()), 0, 0, 0, 0, false)
    };
    let mode = if filemode {
        if exec {
            0o100755
        } else {
            0o100644
        }
    } else {
        // Without file modes (Windows, `core.filemode = false`), keep the
        // recorded executable bit.
        old_mode.filter(|m| *m == 0o100755).unwrap_or(0o100644)
    };
    git2::IndexEntry {
        ctime,
        mtime,
        dev,
        ino,
        mode,
        uid,
        gid,
        // git keeps the low 32 bits of the size.
        file_size: size as u32,
        id,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    }
}

// ---- repacking -------------------------------------------------------------

fn loose_object_count(objects: &Path) -> usize {
    let mut count = 0;
    for byte in 0..=255u8 {
        if let Ok(dir) = std::fs::read_dir(objects.join(format!("{byte:02x}"))) {
            count += dir.count();
        }
    }
    count
}

/// The ids of the loose objects under `objects`.
fn loose_objects(objects: &Path) -> Vec<Oid> {
    let mut out = Vec::new();
    for byte in 0..=255u8 {
        let hex = format!("{byte:02x}");
        let Ok(entries) = std::fs::read_dir(objects.join(&hex)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(rest) = name.to_str().filter(|r| r.len() == 38) else {
                continue;
            };
            if let Ok(oid) = Oid::from_str(&format!("{hex}{rest}")) {
                out.push(oid);
            }
        }
    }
    out
}

/// Pack the loose objects into one new pack, then delete only the loose
/// objects that pack holds (git's `repack -d` without `-a`, then
/// `prune-packed`). Existing packs are never rewritten, so a repack costs
/// what was added since the last one, not the history (review N1).
/// Unreachable loose objects are packed too, not dropped. Returns how many
/// were deleted; `Ok(None)` when skipped (a `git gc` is running, or the
/// repository is not SHA-1).
#[cfg(test)]
fn repack(repo: &Repository) -> Result<Option<usize>, String> {
    let checked = open_checked(repo.workdir().unwrap()).map_err(|e| e.message())?;
    repack_guarded(&checked)
}

fn repack_guarded(checked: &Repo) -> Result<Option<usize>, String> {
    let repo = &checked.repo;
    let guard = &checked.guard;
    let gitdir = repo.path();
    if gitdir.join("gc.pid").exists() {
        return Ok(None);
    }
    if repo
        .config()
        .ok()
        .and_then(|c| c.get_string("extensions.objectformat").ok())
        .is_some_and(|f| !f.eq_ignore_ascii_case("sha1"))
    {
        return Ok(None);
    }
    let objects = gitdir.join("objects");
    let pack_dir = objects.join("pack");
    guard.check_path(&pack_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&pack_dir).map_err(|e| e.to_string())?;
    let loose = loose_objects(&objects);
    if loose.is_empty() {
        return Ok(Some(0));
    }
    // libgit2 1.9.7's indexer fsyncs both files on their writable creation
    // handles, unmaps/closes the pack, then renames it into place. Enable
    // that guarantee here too, including for callers outside open_repo_at.
    super::git_run::enable_git_fsync();
    let mut builder = repo.packbuilder().map_err(|e| e.to_string())?;
    for oid in &loose {
        builder
            .insert_object(*oid, None)
            .map_err(|e| e.to_string())?;
    }
    guard.check_path(&pack_dir).map_err(|e| e.to_string())?;
    builder.write(&pack_dir, 0o444).map_err(|e| e.to_string())?;
    let name = builder
        .name()
        .map_err(|e| e.to_string())?
        .ok_or("the pack has no name")?
        .to_string();
    let idx = pack_dir.join(format!("pack-{name}.idx"));
    // Do not reopen the final read-only files for sync_all: on Windows
    // FlushFileBuffers requires GENERIC_WRITE and fails with Access denied.
    // builder.write already made their contents durable before publishing.
    sync_dir(&pack_dir, guard)?;
    let packed = idx_oids(&std::fs::read(&idx).map_err(|e| e.to_string())?)?;
    let mut removed = 0;
    for oid in loose.iter().filter(|oid| packed.contains(oid.as_bytes())) {
        let hex = oid.to_string();
        let file = objects.join(&hex[..2]).join(&hex[2..]);
        if remove_object(&file, guard).is_ok() {
            removed += 1;
        }
    }
    for byte in 0..=255u8 {
        let dir = objects.join(format!("{byte:02x}"));
        guard.check_path(&dir).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir(dir);
    }
    Ok(Some(removed))
}

/// Delete a loose object file. git writes them read-only, which on Windows
/// refuses the unlink until the attribute is cleared (as git-for-windows
/// does).
fn remove_object(file: &Path, guard: &super::git_run::GitMutationGuard) -> std::io::Result<()> {
    guard.check_path(file)?;
    #[cfg(windows)]
    {
        let mut permissions = std::fs::metadata(file)?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        guard.check_path(file)?;
        std::fs::set_permissions(file, permissions)?;
    }
    guard.check_path(file)?;
    std::fs::remove_file(file)
}

fn sync_dir(dir: &Path, guard: &super::git_run::GitMutationGuard) -> Result<(), String> {
    guard.check_path(dir).map_err(|e| e.to_string())?;
    tine_store::directory_durability::sync_directory_entry(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// The object ids a version-2 SHA-1 pack index lists.
fn idx_oids(idx: &[u8]) -> Result<HashSet<[u8; 20]>, String> {
    const HEADER: &[u8] = b"\xfftOc\0\0\0\x02";
    let bad = || "not a version-2 pack index".to_string();
    if idx.len() < 8 + 1024 || &idx[..8] != HEADER {
        return Err(bad());
    }
    let fanout = |i: usize| {
        let at = 8 + i * 4;
        u32::from_be_bytes([idx[at], idx[at + 1], idx[at + 2], idx[at + 3]]) as usize
    };
    let count = fanout(255);
    let start = 8 + 1024;
    let end = start + count * 20;
    // Ids, CRCs, offsets, then two 20-byte trailers at least.
    if idx.len() < end + count * 8 + 40 {
        return Err(bad());
    }
    Ok(idx[start..end]
        .chunks_exact(20)
        .map(|c| c.try_into().expect("20 bytes"))
        .collect())
}

// ---- structural operations ---------------------------------------------------

/// Run a structural operation (rename, merge, delete or move of a page, a
/// restore) bracketed by commits when this graph autocommits (G3, G9):
/// pending changes first, then the operation's own files with `verb` as the
/// message. A failed operation leaves only the first commit. Commit failures
/// never fail the operation: its files are saved, and the next timed
/// autocommit records them.
pub(crate) fn around<T, E>(
    root: &Path,
    verb: impl FnOnce() -> String,
    op: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    if registry::settings(root).policy != Policy::Commit {
        return op();
    }
    let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _ = commit_locked(root, MESSAGE);
    let result = op();
    if result.is_ok() {
        if let Ok(Outcome::Committed(_)) = commit_locked(root, &verb()) {
            registry::committed(root);
        }
    }
    result
}

// ---- the worker ----------------------------------------------------------------

/// What Settings shows about the last attempt.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct LastAttempt {
    /// One setup event, retained for this worker's lifetime so Settings can
    /// disclose the changed file even when opened after the initial bind.
    pub(crate) syncthing_ignore_added: Option<String>,
    pub(crate) syncthing_ignore_failed: bool,
    /// Maintenance waits for retry; this is separate from a commit refusal.
    pub(crate) repack_deferred: bool,
    /// Unix seconds of the last autocommit this process made.
    pub(crate) committed_at: Option<i64>,
    pub(crate) commit: Option<String>,
    /// The last refusal (`Unusable::code`) and its message.
    pub(crate) refused: Option<(String, String)>,
    /// `index.lock` stayed held through several cycles: probably left by a
    /// crash. Settings names it; Tine never deletes it.
    pub(crate) stale_lock: Option<String>,
}

mod registry {
    use super::*;

    pub(super) struct Worker {
        pub(super) root: PathBuf,
        pub(super) state: Mutex<WorkerState>,
        pub(super) wake: Condvar,
    }

    #[derive(Default)]
    pub(super) struct WorkerState {
        pub(super) settings: GitSettings,
        pub(super) slot: Weak<GraphSlot>,
        pub(super) store: Option<Arc<tine_store::Store>>,
        pub(super) flush: bool,
        /// Bumped after every flush the worker finished.
        pub(super) flushed: u64,
        pub(super) last: LastAttempt,
        pub(super) commits_since_repack: u32,
        pub(super) repack_not_before: Option<Instant>,
        pub(super) running: bool,
        /// The worker left the registry; a new bind makes a new one.
        pub(super) retired: bool,
    }

    static WORKERS: Mutex<Option<HashMap<PathBuf, Arc<Worker>>>> = Mutex::new(None);

    pub(super) fn get(root: &Path) -> Option<Arc<Worker>> {
        WORKERS.lock().unwrap().as_ref()?.get(root).cloned()
    }

    pub(super) fn all() -> Vec<Arc<Worker>> {
        WORKERS
            .lock()
            .unwrap()
            .as_ref()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn get_or_insert(root: &Path) -> (Arc<Worker>, bool) {
        let mut map = WORKERS.lock().unwrap();
        let map = map.get_or_insert_with(HashMap::new);
        if let Some(worker) = map.get(root) {
            return (worker.clone(), false);
        }
        let worker = Arc::new(Worker {
            root: root.to_path_buf(),
            state: Mutex::new(WorkerState::default()),
            wake: Condvar::new(),
        });
        map.insert(root.to_path_buf(), worker.clone());
        (worker, true)
    }

    /// Take `worker` out of the registry. Lock order: a worker's state, then
    /// the registry; nothing holds the registry while taking a state.
    pub(super) fn unregister(worker: &Arc<Worker>) {
        let mut map = WORKERS.lock().unwrap();
        if let Some(map) = map.as_mut() {
            map.retain(|_, w| !Arc::ptr_eq(w, worker));
        }
    }

    pub(super) fn settings(root: &Path) -> GitSettings {
        get(root)
            .map(|w| w.state.lock().unwrap().settings)
            .unwrap_or_default()
    }

    pub(super) fn committed(root: &Path) {
        if let Some(worker) = get(root) {
            let mut state = worker.state.lock().unwrap();
            state.commits_since_repack += 1;
        }
    }
}

/// Start (or retarget) the autocommit worker of `slot`'s graph root with
/// `settings` read from the device settings at bind.
pub(crate) fn start_worker(slot: &Arc<GraphSlot>, settings: GitSettings) {
    let worker = loop {
        let (worker, fresh) = registry::get_or_insert(&slot.root_key);
        let mut state = worker.state.lock().unwrap();
        if state.retired {
            // Unregistered while we looked it up: the next lookup makes a
            // new worker (review F1).
            continue;
        }
        state.slot = Arc::downgrade(slot);
        state.store = Some(slot.store.clone());
        state.settings = settings;
        if !fresh && state.running {
            worker.wake.notify_all();
            return;
        }
        state.running = true;
        drop(state);
        break worker;
    };
    let spawned = std::thread::Builder::new().name("tine-git".into()).spawn({
        let worker = worker.clone();
        move || autocommit_worker(worker)
    });
    if spawned.is_err() {
        let mut state = worker.state.lock().unwrap();
        state.running = false;
        state.retired = true;
        registry::unregister(&worker);
    }
}

/// New settings for a graph (Settings → Graph → Git).
pub(crate) fn set_settings(root: &Path, settings: GitSettings) {
    if let Some(worker) = registry::get(root) {
        worker.state.lock().unwrap().settings = settings;
        worker.wake.notify_all();
    }
}

pub(crate) fn last_attempt(root: &Path) -> LastAttempt {
    registry::get(root)
        .map(|w| w.state.lock().unwrap().last.clone())
        .unwrap_or_default()
}

/// The worker retains the original store through its final commit on unbind.
/// Never recapture the graph identity after a folder replacement.
pub(super) fn bound_store(root: &Path) -> Option<Arc<tine_store::Store>> {
    registry::get(root).and_then(|worker| worker.state.lock().unwrap().store.clone())
}

/// App exit (G9 "at once on quit"): ask every worker for an autocommit and
/// wait for them until `budget` less [`EXIT_GRACE`]. Then no commit may run
/// on: [`EXITING`] makes one in flight abort, which releases its
/// `index.lock`, and taking [`RUN_LOCK`] for good, within the grace, makes
/// sure none is left when the process ends (review N2).
pub(crate) fn flush_all(budget: Duration) {
    let deadline = Instant::now() + budget.saturating_sub(EXIT_GRACE);
    let workers = registry::all();
    let mut waits = Vec::new();
    for worker in &workers {
        let mut state = worker.state.lock().unwrap();
        if state.settings.policy == Policy::Commit && state.running {
            state.flush = true;
            waits.push((worker.clone(), state.flushed));
            worker.wake.notify_all();
        }
    }
    'wait: for (worker, before) in waits {
        let mut state = worker.state.lock().unwrap();
        while state.flushed == before && state.running {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break 'wait;
            }
            state = worker.wake.wait_timeout(state, left).unwrap().0;
        }
    }
    EXITING.store(true, Ordering::Release);
    let grace = Instant::now() + EXIT_GRACE;
    loop {
        match RUN_LOCK.try_lock() {
            Ok(guard) => return std::mem::forget(guard),
            Err(std::sync::TryLockError::Poisoned(p)) => return std::mem::forget(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < grace => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(std::sync::TryLockError::WouldBlock) => return,
        }
    }
}

/// The slot is gone or its background work was cancelled (unbind).
fn ended(slot: Option<&Arc<GraphSlot>>) -> bool {
    slot.is_none_or(|s| s.background_cancelled.load(Ordering::Acquire))
}

fn autocommit_worker(worker: Arc<registry::Worker>) {
    let root = worker.root.clone();
    let mut last_rev = None;
    let mut changed_at: Option<Instant> = None;
    let mut last_commit: Option<Instant> = None;
    let mut busy_since: Option<(Instant, u32)> = None;
    let mut checked_loose = false;
    let started = Instant::now();
    loop {
        let (settings, flush, slot) = {
            let mut state = worker.state.lock().unwrap();
            if !state.flush {
                state = worker.wake.wait_timeout(state, TICK).unwrap().0;
            }
            let flush = std::mem::take(&mut state.flush);
            (state.settings, flush, state.slot.upgrade())
        };
        let ending = ended(slot.as_ref());
        let rev = slot
            .as_ref()
            .filter(|_| !ending)
            .and_then(|s| s.store.whole_graph().ok())
            .map(|view| view.rev());
        drop(slot);
        if rev.is_some() && rev != last_rev {
            if last_rev.is_some() {
                changed_at = Some(Instant::now());
            }
            last_rev = rev;
        }
        let commit = settings.policy == Policy::Commit && !EXITING.load(Ordering::Acquire);
        let due = changed_at.is_some_and(|at| at.elapsed() >= IDLE)
            && last_commit.is_none_or(|at| at.elapsed() >= settings.interval);
        // At bind: changes made while Tine was closed are committed after
        // the first idle period too.
        let first = last_commit.is_none()
            && changed_at.is_none()
            && rev.is_some()
            && started.elapsed() >= IDLE;
        if commit && (flush || ending || due || first) {
            let outcome = commit_now(&root, MESSAGE);
            let mut state = worker.state.lock().unwrap();
            match &outcome {
                Ok(done) => {
                    busy_since = None;
                    state.last.refused = None;
                    state.last.stale_lock = None;
                    changed_at = None;
                    if let Outcome::Committed(oid) = done {
                        last_commit = Some(Instant::now());
                        state.last.committed_at = Some(chrono::Utc::now().timestamp());
                        state.last.commit = Some(oid.to_string());
                        state.commits_since_repack += 1;
                    } else if first {
                        last_commit = Some(Instant::now());
                    }
                }
                Err(error) => {
                    if *error == Unusable::Busy {
                        let (since, cycles) = busy_since.get_or_insert((Instant::now(), 0));
                        *cycles += 1;
                        if *cycles >= STALE_LOCK_CYCLES && since.elapsed() >= STALE_LOCK_AFTER {
                            state.last.stale_lock = Repository::discover(&root)
                                .ok()
                                .map(|r| r.path().join("index.lock").display().to_string());
                        }
                    } else {
                        busy_since = None;
                    }
                    state.last.refused = Some((error.code().into(), error.message()));
                    if first && *error != Unusable::Busy {
                        // Not usable now: look again after an interval, not
                        // every tick. A held lock is retried every tick, so
                        // a stale one is named even on an idle graph.
                        last_commit = Some(Instant::now());
                    }
                }
            }
            let repack_due =
                state.commits_since_repack >= REPACK_COMMITS || state.last.repack_deferred;
            drop(state);
            if outcome.is_ok() && (repack_due || !checked_loose) {
                checked_loose = true;
                maybe_repack(&worker, &root, repack_due);
            }
        }
        if flush || ending {
            let mut state = worker.state.lock().unwrap();
            state.flushed += 1;
            worker.wake.notify_all();
            // Decided under the state lock: a bind that retargeted this
            // worker meanwhile keeps it (review F1).
            if ending && ended(state.slot.upgrade().as_ref()) {
                state.running = false;
                state.retired = true;
                registry::unregister(&worker);
                return;
            }
        }
    }
}

fn maybe_repack(worker: &registry::Worker, root: &Path, due: bool) {
    if worker
        .state
        .lock()
        .unwrap()
        .repack_not_before
        .is_some_and(|at| Instant::now() < at)
    {
        return;
    }
    let Ok(repo) = open_checked(root) else {
        return;
    };
    if !due && loose_object_count(&repo.repo.path().join("objects")) < REPACK_LOOSE_OBJECTS {
        return;
    }
    let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    match repack_guarded(&repo) {
        Ok(Some(_)) => {
            let mut state = worker.state.lock().unwrap();
            state.commits_since_repack = 0;
            state.last.repack_deferred = false;
            state.repack_not_before = None;
        }
        deferred => {
            let mut state = worker.state.lock().unwrap();
            state.last.repack_deferred = true;
            state.repack_not_before = Some(Instant::now() + REPACK_RETRY);
            drop(state);
            if let Err(error) = deferred {
                crate::debug::diag_private("git-repack-failed", error);
            }
        }
    }
}

// ---- Settings ---------------------------------------------------------------------

/// What Settings → Graph → Git shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct GitStatus {
    /// Git features exist on this platform (desktop in slice 1).
    pub(crate) available: bool,
    /// The graph is inside a git work tree.
    pub(crate) repository: bool,
    pub(crate) branch: Option<String>,
    /// Why autocommit cannot run, when it cannot (`code`, message).
    pub(crate) unusable: Option<(String, String)>,
    pub(crate) policy: Policy,
    pub(crate) interval_minutes: u32,
    pub(crate) last: LastAttempt,
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
    GitStatus {
        available: true,
        repository,
        branch,
        unusable,
        policy: settings.policy,
        interval_minutes: (settings.interval.as_secs() / 60) as u32,
        last: last_attempt(root),
    }
}

#[cfg(test)]
#[path = "git_autocommit_tests.rs"]
mod tests;
