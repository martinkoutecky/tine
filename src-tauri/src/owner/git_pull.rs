//! Applying a pull: the fast-forward of ADR 0078 §4 (rev 2), as the storage
//! model s3.7 specifies it (`docs/storage-model/spec/SPEC-s37-gitpull.md`;
//! journal D-20261028-SG, D-20261028-GU).
//!
//! Fetch leaves the upstream tip `U` in the branch's remote-tracking ref. With
//! `H` = `HEAD` and `U` a strict descendant of `H`, the changed paths are
//! applied without git's checkout:
//!
//! 1. a pre-check that refuses by name before anything is written;
//! 2. pages through page-host operations planned from `H`'s bytes, refusing
//!    unsaved input (one operation per detected rename, one for the rest);
//! 3. other files through the store's guarded transactions;
//! 4. under `index.lock`, a durability barrier over every changed path,
//!    those skipped as already at `U` included (P2);
//! 5. the index entries moved to `U`'s blobs and the index installed durably,
//!    then the branch moved from `H` to `U` by compare-and-swap (P1, P1b).
//!
//! Anything that stops before step 5 leaves `HEAD` at `H`; the next cycle
//! resumes from step 1. Meanwhile every autocommit skips ([`defers_autocommit`],
//! P3) while each path that differs from `H` holds its `U` blob or is one this
//! process's pull applied (P4). Stated limit (D-20261028-GU): a pulled path
//! edited back to exactly its `H` bytes, followed by a crash, power cut or
//! graph switch before step 5, is overwritten by the resume; that text is
//! `H`'s blob and stays in git history.
//!
//! The whole pull holds the runs' lock (ADR §3), so no autocommit or run
//! commit falls between its steps. Sync needs the graph root to be the work
//! tree root (ADR §1): a bigger repository's fast-forward changes files Tine
//! does not own.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use git2::{Delta, FileMode, ObjectType, Oid};
use tine_store::{
    ApplyRefusal, Content, EditKind, FileId, FileRev, HostEffect, HostOp, Input, PageId, TrashIf,
    TxOutcome, Why,
};

use super::git_autocommit::{self, Outcome, Unusable};
use super::git_run::{LockRefusal, Repo, RUN_LOCK};
use crate::state::GraphSlot;

const REFLOG: &str = "tine: pull (fast-forward)";
/// Tine's own files in the graph (`logseq/.tine-trash`, …), which the store
/// never writes for a caller.
const TINE_OWN: &str = "logseq/.tine-";

/// Why a pull did not land now, but may on a later cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Held {
    /// A pulled page has unsaved input (`pull-held: unsaved`).
    Unsaved(String),
    /// A file no longer holds its `H` bytes (`pull-held: changed`): the next
    /// cycle autocommits it, then the pull is `diverged`.
    Changed(String),
    /// Other work holds the pages, or another git command runs
    /// (`pull-held: busy`).
    Busy(String),
}

/// What one pull cycle came to (ADR 0078 §4, the refusal table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PullOutcome {
    /// `HEAD` already is, or descends from, the upstream tip.
    UpToDate,
    /// The branch moved from `from` to `to`; `written` paths were written
    /// this cycle (the others were already at `to`).
    Landed {
        from: Oid,
        to: Oid,
        written: usize,
    },
    /// The upstream tip is not a descendant of `HEAD` (or became not one):
    /// phase 1 stops. A race can leave earlier file publications in place.
    Diverged(String),
    Held(Held),
    /// Some paths may be at `U`, the ref is at `H`: the resume finishes it
    /// (`pull-incomplete`).
    Incomplete(String),
    /// The repository content needs the user (`pull-refused`); nothing was
    /// written this cycle.
    Refused {
        path: String,
        reason: String,
    },
    /// A refusal after publication began: files were already updated; the
    /// named path needs attention before this pending target can finish.
    StoppedPartway {
        path: String,
        reason: String,
    },
    /// The branch has no upstream to pull from.
    NoUpstream,
    /// The repository or its configuration cannot sync (`refused: …`).
    Unusable(Unusable),
}

impl PullOutcome {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UpToDate => "up-to-date",
            Self::Landed { .. } => "landed",
            Self::Diverged(_) => "diverged",
            Self::Held(Held::Unsaved(_)) => "pull-held-unsaved",
            Self::Held(Held::Changed(_)) => "pull-held-changed",
            Self::Held(Held::Busy(_)) => "pull-held-busy",
            Self::Incomplete(_) => "pull-incomplete",
            Self::Refused { .. } => "pull-refused",
            Self::StoppedPartway { .. } => "pull-stopped",
            Self::NoUpstream => "no-upstream",
            Self::Unusable(_) => "refused",
        }
    }
}

// ---- P4: what this process applied -------------------------------------------

/// Paths this process's pull put `U`'s blob into, for one `(H, U)` per work
/// tree (s3.7 P4). In memory only: a crash forgets it (the stated limit).
struct Applied {
    base: Oid,
    upstream: Oid,
    branch: String,
    paths: Arc<Mutex<BTreeSet<PageId>>>,
}

static APPLIED: Mutex<Option<HashMap<PathBuf, Applied>>> = Mutex::new(None);

fn applied_paths(key: &Path, base: Oid, upstream: Oid) -> HashSet<String> {
    let map = APPLIED.lock().unwrap_or_else(|p| p.into_inner());
    map.as_ref()
        .and_then(|m| m.get(key))
        .filter(|a| a.base == base && a.upstream == upstream)
        .map(|a| {
            a.paths
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .map(|p| p.as_str().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn publication_sink(
    key: &Path,
    base: Oid,
    upstream: Oid,
    branch: &str,
) -> Arc<Mutex<BTreeSet<PageId>>> {
    let mut map = APPLIED.lock().unwrap_or_else(|p| p.into_inner());
    let entry = map
        .get_or_insert_with(HashMap::new)
        .entry(key.to_path_buf())
        .or_insert_with(|| Applied {
            base,
            upstream,
            branch: branch.to_string(),
            paths: Arc::default(),
        });
    if entry.base != base || entry.upstream != upstream {
        *entry = Applied {
            base,
            upstream,
            branch: branch.to_string(),
            paths: Arc::default(),
        };
    }
    entry.paths.clone()
}

/// Admission pins the target before asynchronous work can outlive the caller.
/// A fetch may advance its tracking ref but cannot replace this provenance.
fn pinned_target(repo: &Repo) -> Option<(Oid, Oid, String)> {
    let key = work_tree_key(repo)?;
    let map = APPLIED.lock().unwrap_or_else(|p| p.into_inner());
    let entry = map.as_ref()?.get(&key)?;
    Some((entry.base, entry.upstream, entry.branch.clone()))
}

fn forget_applied(key: &Path) {
    let mut map = APPLIED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(map) = map.as_mut() {
        map.remove(key);
    }
}

fn work_tree_key(repo: &Repo) -> Option<PathBuf> {
    std::fs::canonicalize(repo.repo.workdir()?).ok()
}

// ---- the upstream and P3 ---------------------------------------------------------

/// The branch's remote-tracking tip, as the last fetch left it.
fn upstream(repo: &Repo) -> Option<Oid> {
    let name = repo.repo.branch_upstream_name(&repo.branch).ok()?;
    repo.repo.refname_to_id(name.as_str().ok()?).ok()
}

/// A fetched `U` that is a strict descendant of `HEAD`'s `H`.
fn pending_upstream(repo: &Repo) -> Option<(Oid, Oid)> {
    let head = repo.tip().ok()??.id();
    if let Some((base, up, branch)) = pinned_target(repo) {
        return (head == base && repo.branch == branch).then_some((base, up));
    }
    let up = upstream(repo)?;
    (up != head && repo.repo.graph_descendant_of(up, head).ok()?).then_some((head, up))
}

/// The resume test (s3.7 P3), run before every autocommit: `true` when a
/// fetched `U` is a strict descendant of `H` and every path under the graph
/// root that differs from `H` holds its `U` blob or is a path this process's
/// pull applied (P4). Then the half-applied tree is the pull's, not a local
/// change, and committing it on `H` would make the next cycle diverge. Costs
/// one status scan, and only while such a `U` exists.
pub(super) fn defers_autocommit(repo: &Repo) -> bool {
    if !repo.prefix.as_os_str().is_empty() {
        return false;
    }
    let Some((head, up)) = pending_upstream(repo) else {
        return false;
    };
    let Some(key) = work_tree_key(repo) else {
        return false;
    };
    let (Ok(h_tree), Ok(u_tree)) = (
        repo.repo.find_commit(head).and_then(|c| c.tree()),
        repo.repo.find_commit(up).and_then(|c| c.tree()),
    ) else {
        return false;
    };
    let applied = applied_paths(&key, head, up);
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .exclude_submodules(true)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    let Ok(statuses) = repo.repo.statuses(Some(&mut options)) else {
        return false;
    };
    let mut differs = false;
    for entry in statuses.iter() {
        let Ok(path) = std::str::from_utf8(entry.path_bytes()) else {
            return false;
        };
        // Tine's own folders (the graph trash a pulled deletion fills) are
        // not the pull's paths: `U` cannot change them (step 1 refuses).
        if path.ends_with('/')
            || path.starts_with(TINE_OWN)
            || git_autocommit::tine_temp(path)
            || git_autocommit::syncthing_temp(path.as_bytes())
        {
            continue;
        }
        if entry.status().is_empty() || entry.status() == git2::Status::IGNORED {
            continue;
        }
        differs = true;
        if applied.contains(path) {
            continue;
        }
        let at_u = blob_at(&u_tree, path);
        let on_disk = match hash_on_disk(&key.join(path)) {
            Ok(blob) => blob,
            Err(_) => return false,
        };
        if on_disk != at_u {
            return false;
        }
        // A path `U` does not change differs from `H` only by a local edit.
        if at_u == blob_at(&h_tree, path) {
            return false;
        }
    }
    differs
}

fn blob_at(tree: &git2::Tree<'_>, path: &str) -> Option<Oid> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    (entry.kind() == Some(ObjectType::Blob)).then(|| entry.id())
}

/// The blob id of the regular file at `path`, `None` when absent; a folder,
/// symlink or other non-file is an error.
fn hash_on_disk(path: &Path) -> std::io::Result<Option<Oid>> {
    #[cfg(test)]
    if test_hook::folding() && !path.exists() {
        if let Some(parent) = path.parent() {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    if tine_store::file_kind::case_only(
                        &FileId::from(entry.file_name().to_string_lossy().into_owned()),
                        &FileId::from(
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        ),
                    ) {
                        return hash_on_disk(&entry.path());
                    }
                }
            }
        }
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
        Ok(meta) if meta.is_file() => Oid::hash_file(ObjectType::Blob, path)
            .map(Some)
            .map_err(|e| std::io::Error::other(e.message().to_string())),
        Ok(_) => Err(std::io::Error::other("not a regular file")),
    }
}

// ---- the pull ----------------------------------------------------------------------

/// One side of a changed entry: its path, blob and mode in `H` or `U`.
#[derive(Debug, Clone)]
struct Side {
    path: String,
    blob: Oid,
    mode: u32,
}

/// One changed entry between `H` and `U`. A detected page rename has both
/// sides at different paths; otherwise both paths are equal.
#[derive(Debug, Clone)]
struct Change {
    old: Option<Side>,
    new: Option<Side>,
    page: bool,
}

impl Change {
    fn paths(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for side in [&self.old, &self.new].into_iter().flatten() {
            if !out.contains(&side.path.as_str()) {
                out.push(&side.path);
            }
        }
        out
    }

    fn renamed(&self) -> bool {
        matches!((&self.old, &self.new), (Some(o), Some(n)) if o.path != n.path)
    }

    /// The blob `path` holds in `U` (`None`: absent).
    fn u_blob(&self, path: &str) -> Option<Oid> {
        self.new.as_ref().filter(|s| s.path == path).map(|s| s.blob)
    }

    /// The blob `path` holds in `H` (`None`: absent).
    fn h_blob(&self, path: &str) -> Option<Oid> {
        self.old.as_ref().filter(|s| s.path == path).map(|s| s.blob)
    }
}

fn refused(path: &str, reason: impl Into<String>) -> PullOutcome {
    PullOutcome::Refused {
        path: path.to_string(),
        reason: reason.into(),
    }
}

fn changed(path: &str) -> PullOutcome {
    PullOutcome::Held(Held::Changed(path.to_string()))
}

/// Pull now: the pre-pull autocommit (unless the resume test defers it),
/// then the fast-forward to the fetched upstream tip.
pub(crate) fn pull(slot: &GraphSlot) -> PullOutcome {
    let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    pull_locked(slot)
}

fn pull_locked(slot: &GraphSlot) -> PullOutcome {
    let root = &slot.root_key;
    let repo = match git_autocommit::open_checked(root) {
        Ok(repo) => repo,
        Err(error) => return PullOutcome::Unusable(error),
    };
    if let Err(error) = check_sync(&repo) {
        return error;
    }
    if let Some((_, _, branch)) = pinned_target(&repo) {
        if repo.branch != branch {
            return PullOutcome::Diverged("the branch changed while a pull was pending".into());
        }
    }
    let Some(fetched) = upstream(&repo) else {
        return PullOutcome::NoUpstream;
    };
    let head = match repo.tip() {
        Ok(Some(tip)) => tip.id(),
        Ok(None) => {
            return PullOutcome::Unusable(Unusable::Repository(
                "the branch has no commits yet; clone the remote instead".into(),
            ))
        }
        Err(error) => return PullOutcome::Unusable(Unusable::Failed(error.message)),
    };
    let descends = |a: Oid, b: Oid| repo.repo.graph_descendant_of(a, b).unwrap_or(false);
    let up = if let Some((base, pending, branch)) = pinned_target(&repo) {
        if head != base || repo.branch != branch {
            return PullOutcome::Diverged("the branch changed while a pull was pending".into());
        }
        if fetched != pending && !descends(fetched, pending) {
            return PullOutcome::Diverged(
                "the remote replaced the pending pull target (force push)".into(),
            );
        }
        pending
    } else {
        fetched
    };
    if up == head || descends(head, up) {
        return PullOutcome::UpToDate;
    }
    if !descends(up, head) {
        return PullOutcome::Diverged("this device and the remote both have new commits".into());
    }
    // The pre-pull autocommit (ADR §3); the resume test inside it (P3)
    // keeps a half-applied pull from being committed as a local change.
    match git_autocommit::commit_locked(root, git_autocommit::MESSAGE) {
        Ok(Outcome::Committed(_)) => {
            return PullOutcome::Diverged(
                "this device had changes, now committed; the remote has others".into(),
            )
        }
        Ok(Outcome::Clean | Outcome::PullPending) => {}
        Err(Unusable::Busy) => return PullOutcome::Held(Held::Busy("index.lock".into())),
        Err(error) => return PullOutcome::Unusable(error),
    }
    if repo.repo.path().join("gc.pid").exists() {
        return PullOutcome::Held(Held::Busy("gc.pid".into()));
    }
    let outcome = apply_pull(slot, &repo, head, up);
    late_refusal(
        outcome,
        work_tree_key(&repo).is_some_and(|key| !applied_paths(&key, head, up).is_empty()),
    )
}

/// Sync refuses what a checkout would rewrite (G5, ADR §1): a graph inside a
/// bigger work tree, and eol normalisation.
fn check_sync(repo: &Repo) -> Result<(), PullOutcome> {
    if !repo.prefix.as_os_str().is_empty() {
        return Err(PullOutcome::Unusable(Unusable::Repository(
            "sync needs the graph folder to be the repository's top folder; a pull would change files outside the graph".into(),
        )));
    }
    let config = repo
        .repo
        .config()
        .map_err(|e| PullOutcome::Unusable(Unusable::Failed(e.message().into())))?;
    if let Ok(value) = config.get_string("core.autocrlf") {
        if !value.eq_ignore_ascii_case("false") {
            return Err(PullOutcome::Unusable(Unusable::Conversion(format!(
                "core.autocrlf = {value}"
            ))));
        }
    }
    Ok(())
}

/// A changed path git would convert on checkout (`text`, `eol`, `crlf`), or
/// whose raw bytes must not be written (`filter`, `ident`,
/// `working-tree-encoding`).
fn conversion(repo: &Repo, path: &str) -> Result<Option<&'static str>, git2::Error> {
    if let Some(attr) = super::git_run::raw_content_conversion(&repo.repo, Path::new(path))? {
        return Ok(Some(attr));
    }
    for attr in ["text", "eol", "crlf"] {
        let value =
            repo.repo
                .get_attr(Path::new(path), attr, git2::AttrCheckFlags::FILE_THEN_INDEX)?;
        if !matches!(
            git2::AttrValue::from_string(value),
            git2::AttrValue::Unspecified | git2::AttrValue::False
        ) {
            return Ok(Some(attr));
        }
    }
    Ok(None)
}

fn regular(mode: FileMode) -> Option<u32> {
    match mode {
        FileMode::Blob | FileMode::BlobGroupWritable => Some(0o100644),
        FileMode::BlobExecutable => Some(0o100755),
        _ => None,
    }
}

/// The changed entries between `H` and `U` (step 1, first half: their kinds).
fn changes(
    slot: &GraphSlot,
    repo: &Repo,
    h_tree: &git2::Tree<'_>,
    u_tree: &git2::Tree<'_>,
) -> Result<Vec<Change>, PullOutcome> {
    let git = |e: git2::Error| PullOutcome::Unusable(Unusable::Failed(e.message().into()));
    let mut diff = repo
        .repo
        .diff_tree_to_tree(Some(h_tree), Some(u_tree), None)
        .map_err(git)?;
    diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))
        .map_err(git)?;
    let filemode = repo
        .repo
        .config()
        .ok()
        .and_then(|c| c.get_bool("core.filemode").ok())
        .unwrap_or(cfg!(unix));
    let is_page = |path: &str| {
        slot.store
            .as_page(&FileId::from(path.to_string()))
            .is_some()
    };
    let mut out = Vec::new();
    for delta in diff.deltas() {
        let name = |file: git2::DiffFile<'_>| -> Result<String, PullOutcome> {
            file.path()
                .and_then(Path::to_str)
                .map(|p| p.replace('\\', "/"))
                .ok_or_else(|| refused("?", "a path that is not UTF-8"))
        };
        let (old_path, new_path) = (name(delta.old_file())?, name(delta.new_file())?);
        let side = |file: git2::DiffFile<'_>, path: &str| -> Result<Option<Side>, PullOutcome> {
            if file.mode() == FileMode::Unreadable {
                return Ok(None);
            }
            match regular(file.mode()) {
                Some(mode) => Ok(Some(Side {
                    path: path.to_string(),
                    blob: file.id(),
                    mode,
                })),
                None => Err(refused(
                    path,
                    match file.mode() {
                        FileMode::Link => "a symbolic link",
                        FileMode::Commit => "a submodule",
                        _ => "not a regular file",
                    },
                )),
            }
        };
        match delta.status() {
            Delta::Added | Delta::Deleted | Delta::Modified | Delta::Renamed => {}
            Delta::Typechange => return Err(refused(&new_path, "a type change")),
            other => {
                return Err(refused(
                    &new_path,
                    format!("an unexpected change ({other:?})"),
                ))
            }
        }
        let old = side(delta.old_file(), &old_path)?;
        let new = side(delta.new_file(), &new_path)?;
        match (&old, &new) {
            (Some(o), Some(n)) if o.mode != n.mode => {
                return Err(refused(&new_path, "a file mode change"))
            }
            (None, Some(n)) if filemode && n.mode == 0o100755 => {
                return Err(refused(&new_path, "a new executable file"))
            }
            _ => {}
        }
        let renamed = old_path != new_path && old.is_some() && new.is_some();
        if renamed && !(is_page(&old_path) && is_page(&new_path)) {
            // Only a page rename is a host move; other files are a delete
            // and an add.
            out.push(Change {
                page: is_page(&old_path),
                old,
                new: None,
            });
            out.push(Change {
                page: is_page(&new_path),
                old: None,
                new,
            });
            continue;
        }
        out.push(Change {
            page: is_page(&new_path),
            old,
            new,
        });
    }
    // A file that becomes a folder or the other way round.
    let old_paths: BTreeSet<&str> = out
        .iter()
        .filter_map(|c| c.old.as_ref().map(|s| s.path.as_str()))
        .collect();
    let new_paths: BTreeSet<&str> = out
        .iter()
        .filter_map(|c| c.new.as_ref().map(|s| s.path.as_str()))
        .collect();
    for (from, to) in [(&old_paths, &new_paths), (&new_paths, &old_paths)] {
        for path in to.iter() {
            let mut at = 0;
            while let Some(slash) = path[at..].find('/') {
                at += slash;
                if from.contains(&path[..at]) {
                    return Err(refused(path, "a file becomes a folder or a folder a file"));
                }
                at += 1;
            }
        }
    }
    Ok(out)
}

/// Step 1: refuse by name before anything is written, and return what is
/// left to apply: each change minus its paths already at `U` (written by an
/// earlier cycle, or by this process's pull: P4). Every path must hold its
/// `H` or its `U` content; anything else is a local change (`pull-held`).
fn pre_check(
    slot: &GraphSlot,
    repo: &Repo,
    changes: &[Change],
    applied: &HashSet<String>,
) -> Result<Vec<Change>, PullOutcome> {
    let git = |e: git2::Error| PullOutcome::Unusable(Unusable::Failed(e.message().into()));
    let index = repo.repo.index().map_err(git)?;
    let mut todo = Vec::new();
    for change in changes {
        let mut pending: Vec<&str> = Vec::new();
        for path in change.paths() {
            if path.starts_with(TINE_OWN) {
                return Err(refused(path, "Tine's own folder"));
            }
            if let Some(attr) = conversion(repo, path).map_err(git)? {
                return Err(refused(
                    path,
                    format!("git converts it on checkout ({attr})"),
                ));
            }
            let id = FileId::from(path.to_string());
            let full = match slot.store.path_for_os_handoff(&id, false) {
                Ok(full) => full,
                Err(
                    tine_store::StoreError::InvalidTarget(_)
                    | tine_store::StoreError::PageSource(_)
                    | tine_store::StoreError::StreamSymlink(_),
                ) => return Err(refused(path, "Tine does not write this file")),
                Err(error) => return Err(PullOutcome::Incomplete(format!("{path}: {error:?}"))),
            };
            let (h, u) = (change.h_blob(path), change.u_blob(path));
            let staged = index.get_path(Path::new(path), 0).map(|e| e.id);
            if staged != h && staged != u {
                return Err(refused(
                    path,
                    "a staged change other than this device's or the remote's",
                ));
            }
            // A case-only move names directory entries, not two independent
            // byte slots. On a folding volume the unlisted destination opens
            // the source; it is still absent for this rename's admission.
            // On a sensitive volume this gives the same two-sided HostOp.
            let case_move = change.renamed()
                && matches!((&change.old, &change.new),
                (Some(old), Some(new)) if tine_store::file_kind::case_only(
                    &FileId::from(old.path.clone()), &FileId::from(new.path.clone())));
            // The OS handoff may canonicalize an alias to its source. Test
            // the requested lexical spelling, not that resolved handoff.
            let disk = match if case_move
                && !listed_entry(&slot.root_key.join(path))
                    .map_err(|e| PullOutcome::Incomplete(format!("{path}: {e}")))?
            {
                Ok(None)
            } else {
                hash_on_disk(&full)
            } {
                Ok(disk) => disk,
                Err(_) => return Err(refused(path, "a folder or link is in the way")),
            };
            if applied.contains(path) || disk == u {
                continue;
            }
            if disk != h {
                if h.is_none()
                    && repo
                        .repo
                        .status_should_ignore(Path::new(path))
                        .unwrap_or(false)
                {
                    return Err(refused(path, "an ignored local file is in the way"));
                }
                return Err(changed(path));
            }
            pending.push(path);
        }
        match pending.len() {
            0 => {}
            n if n == change.paths().len() => todo.push(change.clone()),
            // A rename half done: only the side still at `H` is left (its
            // receiver written: delete the source; its source gone: write
            // the receiver).
            _ => {
                let keep = |side: &Option<Side>| {
                    side.clone().filter(|s| pending.contains(&s.path.as_str()))
                };
                todo.push(Change {
                    old: keep(&change.old),
                    new: keep(&change.new),
                    page: change.page,
                });
            }
        }
    }
    Ok(todo)
}

fn listed_entry(path: &Path) -> std::io::Result<bool> {
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for entry in entries {
        if Some(entry?.file_name().as_os_str()) == path.file_name() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The pull's steps 1–5 over `H` = `head`, `U` = `up`.
fn apply_pull(slot: &GraphSlot, repo: &Repo, head: Oid, up: Oid) -> PullOutcome {
    let git = |e: git2::Error| PullOutcome::Unusable(Unusable::Failed(e.message().into()));
    let Some(key) = work_tree_key(repo) else {
        return PullOutcome::Unusable(Unusable::Repository("the work tree is gone".into()));
    };
    let trees = (|| {
        Ok::<_, git2::Error>((
            repo.repo.find_commit(head)?.tree()?,
            repo.repo.find_commit(up)?.tree()?,
        ))
    })();
    let (h_tree, u_tree) = match trees {
        Ok(trees) => trees,
        Err(error) => return git(error),
    };
    let changes = match changes(slot, repo, &h_tree, &u_tree) {
        Ok(changes) => changes,
        Err(outcome) => return outcome,
    };
    let applied = applied_paths(&key, head, up);
    let todo = match pre_check(slot, repo, &changes, &applied) {
        Ok(todo) => todo,
        Err(outcome) => return outcome,
    };
    let blob = |oid: Oid| -> Result<Vec<u8>, PullOutcome> {
        repo.repo
            .find_blob(oid)
            .map(|b| b.content().to_vec())
            .map_err(git)
    };
    // Step 2: pages. Each rename is its own operation (one move per op).
    let (renames, pages): (Vec<&Change>, Vec<&Change>) =
        todo.iter().filter(|c| c.page).partition(|c| c.renamed());
    let mut ops: Vec<HostOp> = Vec::new();
    for change in &renames {
        match plan_rename(slot, change, &blob) {
            Ok(op) => ops.push(op),
            Err(outcome) => return outcome,
        }
    }
    if !pages.is_empty() {
        // Identify each planning refusal before combining the pages. Why's
        // Refused variant has no path; a batch's first path is not evidence
        // that it was the page which refused.
        for change in &pages {
            if let Err(outcome) = plan_pages(slot, &[*change], &blob) {
                return outcome;
            }
        }
        match plan_pages(slot, &pages, &blob) {
            Ok(op) => ops.push(op),
            Err(outcome) => return outcome,
        }
    }
    let mut others = Vec::new();
    for change in todo.iter().filter(|c| !c.page) {
        let mut tx = match plan_other(slot, change, &blob) {
            Ok(tx) => tx,
            Err(outcome) => return outcome,
        };
        if let Err(why) = tx.validate() {
            return planned(why, change.paths()[0]);
        }
        others.push((change, tx));
    }
    let sink = publication_sink(&key, head, up, &repo.branch);
    let late = |outcome| {
        late_refusal(
            outcome,
            !sink.lock().unwrap_or_else(|p| p.into_inner()).is_empty(),
        )
    };
    for mut op in ops {
        op.publications = Some(sink.clone());
        if let Err(outcome) = host_apply(slot, op) {
            return late(outcome);
        }
    }
    #[cfg(test)]
    if test_hook::crash(Step::AfterPages) {
        forget_applied(&key);
        return PullOutcome::Incomplete("test: crash after the pages".into());
    }

    // Step 3: other files, each in its own guarded transaction.
    for (change, tx) in others {
        match tx.commit() {
            TxOutcome::Committed { .. } => {
                let mut paths = sink.lock().unwrap_or_else(|p| p.into_inner());
                paths.extend(change.paths().into_iter().map(PageId::from));
            }
            TxOutcome::NotCommitted { why, .. } => {
                return late(planned(why, change.paths()[0]));
            }
            TxOutcome::PublicationIncomplete { .. } => {
                // This outcome means commit published its steps, but the
                // subsequent readback failed. Do not lose that publication.
                sink.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .extend(change.paths().into_iter().map(PageId::from));
                return PullOutcome::Incomplete(format!(
                    "{}: a pulled file was written but could not be read back",
                    change.paths()[0]
                ));
            }
        }
    }
    if git_autocommit::exiting() {
        return PullOutcome::Incomplete("Tine is quitting".into());
    }

    // Step 4: the durability barrier, holding index.lock (P2).
    #[cfg(test)]
    test_hook::before_lock();
    let mut lock = match repo.lock_index(false) {
        Ok(lock) => lock,
        Err(LockRefusal::Held) => return PullOutcome::Held(Held::Busy("index.lock".into())),
        Err(LockRefusal::BranchChanged) => {
            return PullOutcome::Diverged(
                "the checked-out branch changed while the pull was applied".into(),
            )
        }
        Err(refusal) => return PullOutcome::Incomplete(refusal.message()),
    };
    if let Err(error) = barrier(slot, &key, &changes) {
        return PullOutcome::Incomplete(format!(
            "a pulled file could not be made durable: {error}"
        ));
    }
    #[cfg(test)]
    if test_hook::crash(Step::AfterBarrier) {
        forget_applied(&key);
        std::mem::forget(lock);
        return PullOutcome::Incomplete("test: crash after the barrier".into());
    }

    // Step 5: the index, then the ref (P1, P1b).
    for change in &changes {
        let mut entries: Vec<(&str, Option<&Side>)> = Vec::new();
        if let Some(old) = &change.old {
            if change.new.as_ref().is_none_or(|n| n.path != old.path) {
                entries.push((&old.path, None));
            }
        }
        if let Some(new) = &change.new {
            entries.push((&new.path, Some(new)));
        }
        for (path, side) in entries {
            let h = change.h_blob(path);
            let (blob, size, mode) = match side {
                Some(side) => {
                    let size = match repo.repo.find_blob(side.blob) {
                        Ok(b) => b.size() as u32,
                        Err(error) => return git(error),
                    };
                    (Some(side.blob), size, side.mode)
                }
                None => (None, 0, change.old.as_ref().map_or(0o100644, |o| o.mode)),
            };
            match lock.set_committed(path, blob, size, mode, h, h) {
                Ok(super::git_run::IndexSync::Synced) => {}
                Ok(super::git_run::IndexSync::StagedKept) => {
                    return late(refused(
                        path,
                        "a staged change other than this device's or the remote's",
                    ))
                }
                Err(error) => return git(error),
            }
        }
    }
    if let Err(error) = lock.install() {
        return PullOutcome::Incomplete(format!("the index could not be written: {error}"));
    }
    #[cfg(test)]
    if test_hook::crash(Step::AfterIndex) {
        forget_applied(&key);
        std::mem::forget(lock);
        return PullOutcome::Incomplete("test: crash between the index and the ref".into());
    }
    match repo.advance(Some(head), up, REFLOG) {
        Ok(true) => {
            forget_applied(&key);
            PullOutcome::Landed {
                from: head,
                to: up,
                written: todo.len(),
            }
        }
        // The index stays at `U`, which matches the working tree.
        Ok(false) => PullOutcome::Diverged("the branch moved while the pull was applied".into()),
        Err(error) => git(error),
    }
}

/// The edit kinds of a host operation's pages.
fn kinds_of(changes: &[&Change]) -> Vec<EditKind> {
    let mut kinds = Vec::new();
    for change in changes {
        let kind = match (&change.old, &change.new) {
            (Some(_), Some(_)) if change.renamed() => EditKind::RenamePage,
            (Some(_), Some(_)) => EditKind::ReplacePage,
            (None, Some(_)) => EditKind::CreatePage,
            (Some(_), None) => EditKind::DeletePage,
            (None, None) => continue,
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

fn late_refusal(outcome: PullOutcome, published: bool) -> PullOutcome {
    match outcome {
        PullOutcome::Refused { path, reason } if published => PullOutcome::StoppedPartway {
            path,
            reason: format!("files were already updated; {reason}"),
        },
        other => other,
    }
}

fn planned(why: Why, path: &str) -> PullOutcome {
    match why {
        Why::Conflict { file, .. } => changed(file.as_str()),
        Why::Refused(refusal) => PullOutcome::Refused {
            path: path.to_string(),
            reason: format!("{refusal:?}"),
        },
        Why::Failed(error) => PullOutcome::Incomplete(format!("{error:?}")),
    }
}

/// The non-rename pages as one host operation, planned from `H`'s bytes.
fn plan_pages(
    slot: &GraphSlot,
    changes: &[&Change],
    blob: &dyn Fn(Oid) -> Result<Vec<u8>, PullOutcome>,
) -> Result<HostOp, PullOutcome> {
    let mut tx = slot.store.page_plan(Some(EditKind::ReplacePage));
    for change in changes {
        match (&change.old, &change.new) {
            (Some(old), Some(new)) => {
                let id = PageId::from(new.path.clone());
                let rev = FileRev::from_bytes(&blob(old.blob)?);
                tx.replace_page_bytes(&id, rev, blob(new.blob)?);
            }
            (None, Some(new)) => {
                tx.create(
                    &FileId::from(new.path.clone()),
                    Content::Bytes(blob(new.blob)?),
                );
            }
            (Some(old), None) => {
                let rev = FileRev::from_bytes(&blob(old.blob)?);
                tx.trash(&FileId::from(old.path.clone()), rev, TrashIf::Any);
            }
            (None, None) => {}
        }
    }
    let mut op = tx
        .plan()
        .map_err(|why| planned(why, changes[0].paths()[0]))?;
    op.kinds = kinds_of(changes);
    op.policy = Input::Refuse;
    Ok(op)
}

/// One detected page rename as its own host operation: the move, with the
/// receiver taking `U`'s blob.
fn plan_rename(
    slot: &GraphSlot,
    change: &Change,
    blob: &dyn Fn(Oid) -> Result<Vec<u8>, PullOutcome>,
) -> Result<HostOp, PullOutcome> {
    let (Some(old), Some(new)) = (&change.old, &change.new) else {
        unreachable!("a rename has both sides")
    };
    let mut tx = slot.store.page_plan(Some(EditKind::RenamePage));
    let old_bytes = blob(old.blob)?;
    tx.move_file(
        &FileId::from(old.path.clone()),
        FileRev::from_bytes(&old_bytes),
        &FileId::from(new.path.clone()),
        None,
    );
    let mut op = tx.plan().map_err(|why| planned(why, &new.path))?;
    if new.blob != old.blob {
        let bytes = blob(new.blob)?;
        page_input(&new.path, &bytes)?;
        let target = PageId::from(new.path.clone());
        for (page, effect, _) in op.pages.iter_mut() {
            if page.as_str().eq_ignore_ascii_case(target.as_str()) {
                if let HostEffect::Write(_) = effect {
                    *effect = HostEffect::Write(bytes.clone().into());
                }
            }
        }
    }
    op.policy = Input::Refuse;
    Ok(op)
}

/// The page input checks `PagePlan::create` runs, for a renamed page's new
/// bytes (the move's receiver has no step of its own).
fn page_input(path: &str, bytes: &[u8]) -> Result<(), PullOutcome> {
    let bad = |why: &str| refused(path, why.to_string());
    if bytes.len() as u64 > tine_store::PARSE_INPUT_MAX_BYTES {
        return Err(bad("the page is over the 64 MiB limit"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| bad("the page is not UTF-8 text"))?;
    if !tine_store::parse_input_depth_within_limit(text) {
        return Err(bad("the outline nests over 128 levels"));
    }
    if path.to_ascii_lowercase().ends_with(".org")
        && !tine_core::org::headline_levels_within_limit(text, 128)
    {
        return Err(bad("the Org headlines nest over 128 levels"));
    }
    Ok(())
}

fn host_apply(slot: &GraphSlot, op: HostOp) -> Result<(), PullOutcome> {
    #[cfg(test)]
    let injected = test_hook::host(&op);
    #[cfg(not(test))]
    let injected: Option<(bool, Result<(), ApplyRefusal>)> = None;
    let result = match injected {
        Some((false, result)) => result,
        Some((true, result)) => {
            let real = crate::page_commands::with_host(slot, |host| host.apply(op))
                .map_err(PullOutcome::Incomplete)?;
            real.and(result)
        }
        None => crate::page_commands::with_host(slot, |host| host.apply(op))
            .map_err(PullOutcome::Incomplete)?,
    };
    #[cfg(test)]
    test_hook::after_host();
    result.map_err(|refusal| match refusal {
        ApplyRefusal::Unsaved(page) => PullOutcome::Held(Held::Unsaved(page.as_str().to_string())),
        ApplyRefusal::Changed => PullOutcome::Held(Held::Changed(String::new())),
        ApplyRefusal::Busy => PullOutcome::Held(Held::Busy("pages".into())),
        other => PullOutcome::Incomplete(format!("{other:?}")),
    })
}

/// Step 3 for one non-page change: the store's checked atomic writer.
fn plan_other<'a>(
    slot: &'a GraphSlot,
    change: &Change,
    blob: &dyn Fn(Oid) -> Result<Vec<u8>, PullOutcome>,
) -> Result<tine_store::Transaction<'a>, PullOutcome> {
    let mut tx = slot.store.transaction(None);
    match (&change.old, &change.new) {
        (Some(old), Some(new)) => {
            let rev = FileRev::from_bytes(&blob(old.blob)?);
            tx.replace(&FileId::from(new.path.clone()), rev, blob(new.blob)?);
        }
        (None, Some(new)) => {
            tx.create(
                &FileId::from(new.path.clone()),
                Content::Bytes(blob(new.blob)?),
            );
        }
        (Some(old), None) => {
            let rev = FileRev::from_bytes(&blob(old.blob)?);
            tx.trash(&FileId::from(old.path.clone()), rev, TrashIf::Any);
        }
        (None, None) => {}
    }
    Ok(tx)
}

/// Step 4 (P2): every changed path and its folder synced, those already at
/// `U` included; a deleted file's folder and the graph trash's folders. The
/// folder sync is the store's: strict on Unix, no flush on Windows (ADR 0078
/// §2, D-20261028-WDD).
fn barrier(slot: &GraphSlot, root: &Path, changes: &[Change]) -> std::io::Result<()> {
    #[cfg(test)]
    if test_hook::crash(Step::BarrierFails) {
        return Err(std::io::Error::other("test: the barrier's sync failed"));
    }
    use tine_store::directory_durability::sync_directory_witness;
    // Each file is opened only after the store's containment check binds it
    // to the graph root (the work tree root, ADR §1).
    let bound = tine_store::BoundDirectory::new(root)?;
    let mut folders = BTreeSet::new();
    let mut trash = false;
    for change in changes {
        for path in change.paths() {
            let full = slot
                .store
                .path_for_os_handoff(&FileId::from(path.to_string()), false)
                .map_err(|e| std::io::Error::other(format!("{path}: {e:?}")))?;
            if change.u_blob(path).is_some() {
                bound.check_path(&full)?;
                #[cfg(windows)]
                let file = std::fs::OpenOptions::new().write(true).open(&full)?;
                #[cfg(not(windows))]
                let file = std::fs::File::open(&full)?;
                file.sync_all()?;
            } else {
                trash = true;
            }
            if let Some(parent) = full.parent() {
                folders.insert(parent.to_path_buf());
            }
        }
    }
    if trash {
        let base = root.join("logseq/.tine-trash");
        for kind in ["pages", "journals", "assets", "conflicts", "other"] {
            let dir = base.join(kind);
            if dir.is_dir() {
                folders.insert(dir);
            }
        }
    }
    // A filesystem without directory sync is the store's weak witness,
    // tolerated as its own publication tolerates it.
    for folder in folders {
        if folder.is_dir() {
            sync_directory_witness(&folder)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    AfterPages,
    BarrierFails,
    AfterBarrier,
    AfterIndex,
}

#[cfg(test)]
pub(crate) mod test_hook {
    use super::*;

    type HostHook = Box<dyn FnMut(&HostOp) -> Option<(bool, Result<(), ApplyRefusal>)>>;
    thread_local! {
        static HOST: std::cell::RefCell<Option<HostHook>> = const { std::cell::RefCell::new(None) };
        static CRASH: std::cell::RefCell<Option<Step>> = const { std::cell::RefCell::new(None) };
        static BEFORE_LOCK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
        static AFTER_HOST: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
        static FOLDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Answer the next host operations: `Some((apply_first, result))`
    /// reports `result`, after really applying when `apply_first`.
    pub(crate) fn on_host(
        hook: impl FnMut(&HostOp) -> Option<(bool, Result<(), ApplyRefusal>)> + 'static,
    ) {
        HOST.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    }

    /// Stop the next pull at `step` as a crash would (one shot).
    pub(crate) fn crash_at(step: Step) {
        CRASH.with(|c| *c.borrow_mut() = Some(step));
    }

    pub(crate) fn clear() {
        HOST.with(|h| *h.borrow_mut() = None);
        CRASH.with(|c| *c.borrow_mut() = None);
        BEFORE_LOCK.with(|h| *h.borrow_mut() = None);
        AFTER_HOST.with(|h| *h.borrow_mut() = None);
        FOLDING.with(|f| f.set(false));
    }

    pub(super) fn host(op: &HostOp) -> Option<(bool, Result<(), ApplyRefusal>)> {
        HOST.with(|h| h.borrow_mut().as_mut().and_then(|hook| hook(op)))
    }

    pub(super) fn crash(step: Step) -> bool {
        CRASH.with(|c| {
            let hit = *c.borrow() == Some(step);
            if hit {
                *c.borrow_mut() = None;
            }
            hit
        })
    }

    /// What this process's P4 set holds for the graph at `root`.
    pub(crate) fn applied(root: &Path) -> Vec<String> {
        let key = std::fs::canonicalize(root).unwrap();
        let map = APPLIED.lock().unwrap();
        let mut paths: Vec<String> = map
            .as_ref()
            .and_then(|m| m.get(&key))
            .map(|a| {
                a.paths
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        paths
    }

    /// A process restart: the P4 set is gone.
    pub(crate) fn restart(root: &Path) {
        forget_applied(&std::fs::canonicalize(root).unwrap());
    }

    pub(crate) fn on_before_lock(hook: impl FnOnce() + 'static) {
        BEFORE_LOCK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn before_lock() {
        BEFORE_LOCK.with(|h| {
            if let Some(hook) = h.borrow_mut().take() {
                hook();
            }
        });
    }

    pub(crate) fn on_after_host(hook: impl FnOnce() + 'static) {
        AFTER_HOST.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn after_host() {
        AFTER_HOST.with(|h| {
            if let Some(hook) = h.borrow_mut().take() {
                hook();
            }
        });
    }

    pub(crate) fn simulate_folding() {
        FOLDING.with(|f| f.set(true));
    }
    pub(super) fn folding() -> bool {
        FOLDING.with(|f| f.get())
    }
}
