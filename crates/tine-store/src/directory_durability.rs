//! Synchronization of a directory selected by the caller.
//!
//! [`sync_directory_entry`] takes an existing directory path and costs one open
//! and sync on supported Unix targets. The caller selects the changed directory (usually a renamed
//! entry's parent); the helper does not discover it or perform a rename.
//! Callers choose when to invoke it and must supply an actual directory; the
//! helper does not verify the path type or reject symlinks. It reports real I/O
//! failures as wrapped errors retaining their `ErrorKind`, not raw OS codes.
//! After a post-rename error, callers must re-read disk state because
//! the new name may already be visible. The caller need not implement platform
//! directory flushing.

use std::io;
use std::path::Path;

#[derive(Debug)]
struct DirectorySyncFailure(io::Error);

impl std::fmt::Display for DirectorySyncFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "directory sync failed: {}", self.0)
    }
}

impl std::error::Error for DirectorySyncFailure {}

pub(crate) fn is_directory_sync_failure(error: &io::Error) -> bool {
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<DirectorySyncFailure>())
        .is_some()
}

/// The platform step an I/O error names: one labelled by
/// `platform_step::at`, or a directory sync. O(1), no I/O.
pub(crate) fn failure_step(error: &io::Error) -> Option<(&'static str, Option<i32>)> {
    crate::platform_step::step_of(error).or_else(|| {
        let failure = error.get_ref()?.downcast_ref::<DirectorySyncFailure>()?;
        Some(("fsync directory", failure.0.raw_os_error()))
    })
}

#[cfg(all(feature = "test-faults", unix))]
thread_local! {
    static FAIL_NEXT_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(all(feature = "test-faults", unix))]
pub(crate) fn fail_next_sync() {
    FAIL_NEXT_SYNC.with(|fail| fail.set(true));
}

#[cfg(feature = "test-faults")]
thread_local! {
    static SYNCED: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Test-only: the directories this thread asked to sync, in order, since the
/// last call. Lets a crash-ordering test name which entries were made durable.
#[cfg(feature = "test-faults")]
pub fn take_synced_directories() -> Vec<std::path::PathBuf> {
    SYNCED.with(|synced| std::mem::take(&mut *synced.borrow_mut()))
}

/// Synchronize the supplied directory where the platform supports it.
/// Errors that mean "this filesystem does not offer directory sync" are
/// tolerated ([`dir_sync_is_unsupported`]); a real failure (`EIO`, `ENOSPC`, …)
/// is returned, and the caller re-reads disk state. Windows returns success
/// without a directory flush; standard file opening cannot flush its directory
/// handles, and the caller is responsible for its own rename durability protocol.
pub fn sync_directory_entry(dir: &Path) -> io::Result<()> {
    sync_directory(dir, false).map(|_| ())
}

/// What a directory sync proved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryWitness {
    /// The platform synced the directory (or, on Windows, needs no flush).
    Durable,
    /// The filesystem does not offer directory sync: no platform witness.
    Unsupported,
}

/// Graph publication retains exactly Beta's tolerated error set, but reports
/// a weak witness distinctly so the host cannot claim strong durability.
pub fn sync_directory_witness(dir: &Path) -> io::Result<DirectoryWitness> {
    sync_directory(dir, false)
}

/// App-data metadata must meet the design's platform witness. Unix never
/// tolerates a directory-sync error here. Windows relies on write-through
/// moves / successful DeleteFileW and NTFS's ordered metadata journal; there
/// is no native directory flush and no native power-cut proof.
pub(crate) fn sync_private_directory(dir: &Path) -> io::Result<()> {
    sync_directory(dir, true).map(|_| ())
}

#[cfg(test)]
thread_local! {
    pub(crate) static SYNC_ERROR: std::cell::Cell<Option<io::ErrorKind>> = const { std::cell::Cell::new(None) };
    pub(crate) static BEFORE_CREATE_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static BEFORE_MKDIR: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

fn sync_directory(dir: &Path, private: bool) -> io::Result<DirectoryWitness> {
    #[cfg(feature = "test-faults")]
    SYNCED.with(|synced| synced.borrow_mut().push(dir.to_path_buf()));
    #[cfg(all(feature = "test-faults", unix))]
    if FAIL_NEXT_SYNC.with(|fail| fail.replace(false)) {
        return Err(report_failure(io::Error::other(
            "injected directory sync I/O failure",
        )));
    }
    #[cfg(test)]
    if let Some(kind) = SYNC_ERROR.with(|error| error.take()) {
        return classify_sync(
            Err(io::Error::new(kind, "injected directory sync failure")),
            private,
        );
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        classify_sync(
            std::fs::File::open(dir).and_then(|directory| directory.sync_all()),
            private,
        )
    }
    #[cfg(target_os = "windows")]
    {
        // Directory handles cannot be synced with std::fs. NTFS renames are
        // journaled, but this helper does not verify the filesystem or rename.
        let _ = dir;
        // The caller's synced temp + WRITE_THROUGH move is the Windows
        // publication recipe (SPEC-s2 §4.7), not a directory flush.
        let _ = private;
        Ok(DirectoryWitness::Durable)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        let _ = (dir, private);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory sync unsupported on this target",
        ))
    }
}

fn classify_sync(result: io::Result<()>, private: bool) -> io::Result<DirectoryWitness> {
    match result {
        Ok(()) => Ok(DirectoryWitness::Durable),
        Err(error) if !private && dir_sync_is_unsupported(&error) => {
            Ok(DirectoryWitness::Unsupported)
        }
        Err(error) => Err(report_failure(error)),
    }
}

/// Create descendants of an existing bound one level at a time, then sync
/// the parent of every directory created, so a file published into `dir` survives power loss
/// with its whole directory chain. Scenario: power loss after a move into a
/// new trash/recovery or page subdirectory — the source parent's sync makes the
/// removal durable while the new directory's own entry is not, losing the file.
/// Cost: one `metadata` per missing ancestor plus the existing one, a root
/// binding/containment check before each mkdir and parent sync, plus entry and
/// parent identity captures and pre/post-sync probes for retained debt. An
/// existing dir costs one `metadata` and no directory creation or debt probes.
pub(crate) fn create_dir_all_durable(
    dir: &Path,
    bound: &Path,
    binding: &crate::root_binding::RootBinding,
) -> io::Result<()> {
    DirectoryCreation::new(dir, bound)?.finish(binding, sync_directory_entry)
}

pub(crate) fn create_dir_all_with_sync(
    dir: &Path,
    sync: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<()> {
    // App data has no graph-root boundary. Select its existing ancestor.
    let mut bound = dir;
    while !bound.is_dir() {
        bound = bound
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "no existing app-data ancestor")
            })?;
    }
    // App data has no graph's open-time witness; bind its selected ancestor
    // for this attempt only. Graph callers always supply the original binding.
    let mut creation = DirectoryCreation::new(dir, bound)?;
    if creation.missing.is_empty() {
        return Ok(());
    }
    let binding = crate::root_binding::RootBinding::new(bound);
    creation.finish(&binding, sync)
}

/// The same audited directory-chain creation with retained sync custody. A
/// retry after a successful mkdir cannot infer durability from its existence.
pub(crate) struct DirectoryCreation {
    bound: std::path::PathBuf,
    target: std::path::PathBuf,
    missing: std::collections::VecDeque<std::path::PathBuf>,
    parents: std::collections::VecDeque<DirectoryDebt>,
    replan: bool,
}

/// Both the changed entry and its containing directory belong to this debt.
/// Reuse the root's platform identity policy without retaining descriptors.
struct DirectoryDebt {
    entry: crate::root_binding::RootBinding,
    parent: crate::root_binding::RootBinding,
    path: std::path::PathBuf,
}

impl DirectoryDebt {
    fn check(&self) -> io::Result<()> {
        self.parent
            .check()
            .and_then(|_| self.entry.check())
            .map_err(|_| {
                io::Error::other(
                    "directory creation changed before its sync; retry the current chain",
                )
            })
    }
}

impl DirectoryCreation {
    pub(crate) fn new(dir: &Path, bound: &Path) -> io::Result<Self> {
        if !dir.starts_with(bound) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory escapes its creation bound",
            ));
        }
        if dir
            .strip_prefix(bound)
            .unwrap()
            .components()
            .any(|component| {
                !matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory escapes its creation bound",
            ));
        }
        let mut missing = Vec::new();
        let mut probe = dir;
        loop {
            match std::fs::metadata(probe) {
                Ok(metadata) if metadata.is_dir() => break,
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "directory creation ancestor is a file",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if probe == bound {
                        return Err(crate::root_binding::moved());
                    }
                    missing.push(probe.to_path_buf());
                    probe = probe.parent().ok_or_else(crate::root_binding::moved)?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Self {
            bound: bound.to_path_buf(),
            target: dir.to_path_buf(),
            parents: Default::default(),
            missing: missing.into_iter().rev().collect(),
            replan: false,
        })
    }

    pub(crate) fn has_sync_debt(&self) -> bool {
        !self.parents.is_empty()
    }

    pub(crate) fn finish(
        &mut self,
        binding: &crate::root_binding::RootBinding,
        sync: impl Fn(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        let result = self.finish_current_chain(binding, sync);
        if let Err(error) = &result {
            // An ancestor can disappear at the pre-mkdir check, mkdir itself,
            // or sync, including during a current-chain retry. Keep the queues
            // and unpaid debt on this failure; the next attempt explicitly
            // replaces them through the same re-plan policy as stale identities.
            // Root detachment and outside substitutions still fail their guards.
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) {
                self.replan = true;
            }
        }
        result
    }

    fn finish_current_chain(
        &mut self,
        binding: &crate::root_binding::RootBinding,
        sync: impl Fn(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.replan {
            // The previous attempt reported non-durable creation, not payment
            // of the detached epoch's debt. Retry owes the entire current
            // chain, including existing replacements left by another writer.
            let mut next = Self::new(&self.target, &self.bound)?;
            let mut entry = self.target.as_path();
            while entry != self.bound {
                if entry.try_exists()? {
                    next.check_directory(entry, binding)?;
                    next.parents.push_back(DirectoryDebt {
                        entry: crate::root_binding::RootBinding::new(entry),
                        parent: crate::root_binding::RootBinding::new(entry.parent().unwrap()),
                        path: entry.parent().unwrap().into(),
                    });
                }
                entry = entry.parent().unwrap();
            }
            *self = next;
        }
        while let Some(path) = self.missing.front() {
            #[cfg(test)]
            {
                let step = BEFORE_CREATE_CHECK.with(|step| step.borrow_mut().take());
                if let Some(step) = step {
                    step();
                }
            }
            let parent = path.parent().unwrap();
            // Reuse the original open-time binding, with its platform policy.
            // Each actual parent is checked immediately before path-based mkdir;
            // the final check-to-create race remains (see A-PATH-IDENTITY).
            self.check_directory(parent, binding)?;
            let parent_identity = crate::root_binding::RootBinding::new(parent);
            #[cfg(test)]
            {
                let step = BEFORE_MKDIR.with(|step| step.borrow_mut().take());
                if let Some(step) = step {
                    step();
                }
            }
            match std::fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {
                    self.check_directory(path, binding)?;
                }
                Err(error) => return Err(error),
            }
            self.parents.push_back(DirectoryDebt {
                entry: crate::root_binding::RootBinding::new(path),
                parent: parent_identity,
                path: parent.to_path_buf(),
            });
            self.missing.pop_front();
        }
        while let Some(debt) = self.parents.front() {
            self.check_directory(&debt.path, binding)?;
            if let Err(error) = debt.check() {
                self.replan = true;
                return Err(error);
            }
            sync(&debt.path)?;
            // A successful syscall is not payment if either name changed
            // during it. The probes and sync remain sequential, not atomic.
            if let Err(error) = debt.check() {
                self.replan = true;
                return Err(error);
            }
            self.parents.pop_front();
        }
        Ok(())
    }

    fn check_directory(
        &self,
        directory: &Path,
        binding: &crate::root_binding::RootBinding,
    ) -> io::Result<()> {
        binding.check()?;
        let bound = std::fs::canonicalize(&self.bound)?;
        if !std::fs::canonicalize(directory)?.starts_with(bound) {
            return Err(io::Error::other(
                "directory parent moved outside its bound; unsaved edits kept on this device",
            ));
        }
        Ok(())
    }
}

/// True for directory-sync errors that mean "this filesystem does not offer
/// it", as opposed to a real durability failure. Ported from master
/// 54dfcc1b6674 (`dir_fsync_is_unsupported`; Martin 2026-10-03: follow master).
///
/// Several NFS and FUSE implementations (Android shared storage is FUSE)
/// answer an open or fsync of a directory with `EBADF`, `EACCES`, `EISDIR` or
/// `EINVAL`. Refusing every save there would make the graph uneditable over a
/// guarantee that filesystem cannot give; the file bytes themselves are
/// already written and fsynced. A real `EIO`/`ENOSPC` still fails the save,
/// because then the rename may not survive a crash.
pub(crate) fn dir_sync_is_unsupported(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied
    ) {
        return true;
    }
    #[cfg(unix)]
    {
        error.raw_os_error().is_some_and(|errno| {
            [
                libc::EBADF,
                libc::EACCES,
                libc::EISDIR,
                libc::EINVAL,
                libc::ENOTSUP,
            ]
            .contains(&errno)
        })
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::dir_sync_is_unsupported;
    use std::io;

    #[test]
    fn dircreate_existing_directory_adds_no_binding_or_sync_work() {
        let temp = tempfile::tempdir().unwrap();
        let binding = crate::root_binding::RootBinding::new(temp.path());
        let before = crate::root_binding::CHECKS.with(|count| count.get());
        let mut creation = super::DirectoryCreation::new(temp.path(), temp.path()).unwrap();
        creation
            .finish(&binding, |_| panic!("no mkdir, no sync"))
            .unwrap();
        assert_eq!(
            crate::root_binding::CHECKS.with(|count| count.get()),
            before
        );
        assert!(!creation.has_sync_debt());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn dircreate_checks_original_open_binding_even_if_replaced_before_planning() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        std::fs::create_dir(&root).unwrap();
        let binding = crate::root_binding::RootBinding::new(&root);
        std::fs::rename(&root, temp.path().join("moved")).unwrap();
        std::fs::create_dir(&root).unwrap();
        let mut creation = super::DirectoryCreation::new(&root.join("pages/sub"), &root).unwrap();
        let error = creation
            .finish(&binding, super::sync_directory_entry)
            .unwrap_err();
        assert!(error.to_string().contains("GraphRootMoved"));
        assert!(!root.join("pages").exists());
    }

    #[test]
    fn dircreate_rejects_parent_components() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            super::DirectoryCreation::new(&temp.path().join("../outside/sub"), temp.path())
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn dircreate_rechecks_each_actual_parent_before_mkdir() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let binding = crate::root_binding::RootBinding::new(&root);
        let mut creation =
            super::DirectoryCreation::new(&root.join("pages/sub/deep"), &root).unwrap();
        let pages = root.join("pages");
        let detached = root.join("old-pages");
        super::BEFORE_CREATE_CHECK.with(|step| {
            *step.borrow_mut() = Some(Box::new(move || {
                super::BEFORE_CREATE_CHECK.with(|step| {
                    *step.borrow_mut() = Some(Box::new(move || {
                        std::fs::rename(&pages, detached).unwrap();
                        std::os::unix::fs::symlink(outside, pages).unwrap();
                    }));
                });
            }));
        });
        assert!(creation
            .finish(&binding, super::sync_directory_entry)
            .is_err());
        assert!(!temp.path().join("outside/sub").exists());
        assert!(creation.has_sync_debt(), "first mkdir still owes root sync");
        // A contained replacement first reports the old creation non-durable,
        // then a retry re-plans and syncs the current chain.
        std::fs::remove_file(root.join("pages")).unwrap();
        std::fs::create_dir(root.join("pages")).unwrap();
        if !cfg!(target_os = "android") {
            assert!(creation
                .finish(&binding, super::sync_directory_entry)
                .is_err());
        }
        creation
            .finish(&binding, super::sync_directory_entry)
            .unwrap();
        assert!(root.join("pages/sub/deep").is_dir());
        assert!(!creation.has_sync_debt());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn dircreate_sync_debt_survives_failure_and_contained_parent_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        let binding = crate::root_binding::RootBinding::new(&root);
        let mut creation =
            super::DirectoryCreation::new(&root.join("pages/sub/deep"), &root).unwrap();
        assert!(creation
            .finish(&binding, |_| Err(io::Error::other("sync failed")))
            .is_err());
        assert_eq!(creation.parents.len(), 2);
        std::fs::rename(root.join("pages/sub"), root.join("old-sub")).unwrap();
        std::fs::create_dir(root.join("pages/sub")).unwrap();
        assert!(creation
            .finish(&binding, super::sync_directory_entry)
            .is_err());
        let synced = std::cell::RefCell::new(Vec::new());
        creation
            .finish(&binding, |parent| {
                synced.borrow_mut().push(parent.to_path_buf());
                super::sync_directory_entry(parent)
            })
            .unwrap();
        assert_eq!(
            *synced.borrow(),
            vec![root.join("pages"), root.clone(), root.join("pages/sub")]
        );
        assert!(root.join("pages/sub/deep").is_dir());
        assert!(!creation.has_sync_debt());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn dircreate_replacement_during_sync_is_not_payment_and_retry_replans() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let binding = crate::root_binding::RootBinding::new(root);
        let target = root.join("new/deep");
        let mut creation = super::DirectoryCreation::new(&target, root).unwrap();
        let error = creation.finish(&binding, |parent| {
            super::sync_directory_entry(parent)?;
            std::fs::rename(root.join("new"), root.join("old-new"))?;
            std::fs::create_dir_all(&target)
        });
        assert!(error.is_err(), "post-sync identity changed");
        assert!(creation.has_sync_debt());
        creation
            .finish(&binding, super::sync_directory_entry)
            .unwrap();
        assert!(!creation.has_sync_debt());
    }

    #[test]
    fn dircreate_missing_sync_parent_replans_instead_of_retrying_a_dead_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let binding = crate::root_binding::RootBinding::new(root);
        let target = root.join("new/deep");
        let mut creation = super::DirectoryCreation::new(&target, root).unwrap();
        assert!(creation
            .finish(&binding, |parent| {
                if parent == root {
                    Ok(())
                } else {
                    Err(io::Error::other("sync failed"))
                }
            })
            .is_err());
        std::fs::rename(root.join("new"), root.join("old-new")).unwrap();
        assert!(creation
            .finish(&binding, super::sync_directory_entry)
            .is_err());
        creation
            .finish(&binding, super::sync_directory_entry)
            .unwrap();
        assert!(target.is_dir());
        assert!(!creation.has_sync_debt());
    }

    fn interrupt_mkdir(after_check: bool, step: impl FnOnce() + 'static) {
        let hook = if after_check {
            &super::BEFORE_MKDIR
        } else {
            &super::BEFORE_CREATE_CHECK
        };
        hook.with(|hook| *hook.borrow_mut() = Some(Box::new(step)));
    }

    fn partial_missing_ancestor_recovers(replanned: bool, after_check: bool) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        std::fs::create_dir(&root).unwrap();
        let binding = crate::root_binding::RootBinding::new(&root);
        let target = root.join("new/deep");
        let mut creation = super::DirectoryCreation::new(&target, &root).unwrap();
        if replanned {
            assert!(creation
                .finish(&binding, |_| Err(io::Error::other("first sync failed")))
                .is_err());
            std::fs::rename(root.join("new"), root.join("old-new")).unwrap();
            // Disappearance detects stale debt even on Android's vanish-only policy.
            assert!(creation
                .finish(&binding, |_| panic!("detached debt must not sync"))
                .is_err());
            std::fs::create_dir(root.join("new")).unwrap();
        }
        let move_root = root.clone();
        let move_ancestor = move || {
            std::fs::rename(move_root.join("new"), move_root.join("moved-new")).unwrap();
        };
        if replanned {
            // Re-planning owes the replacement's entry and still needs deep.
            interrupt_mkdir(after_check, move_ancestor);
        } else {
            // Let mkdir new complete; interrupt the following mkdir deep.
            interrupt_mkdir(after_check, move || {
                interrupt_mkdir(after_check, move_ancestor)
            });
        }
        let error = creation
            .finish(&binding, |_| {
                panic!("missing ancestor must fail before sync")
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(creation.missing.len(), 1, "deep still needs creation");
        assert_eq!(creation.parents.len(), 1, "new still owes parent sync");
        assert!(creation.has_sync_debt());
        assert!(!target.exists());
        assert!(root.join("moved-new").is_dir());

        // No external repair, fresh plan, binding or graph reopen: one stable retry.
        let synced = std::cell::RefCell::new(Vec::new());
        creation
            .finish(&binding, |parent| {
                synced.borrow_mut().push(parent.to_path_buf());
                super::sync_directory_entry(parent)
            })
            .unwrap();
        assert_eq!(*synced.borrow(), [root.clone(), root.join("new")]);
        assert!(target.is_dir());
        assert!(!creation.has_sync_debt());
    }

    #[test]
    fn dircreate_partial_missing_ancestor_recovers() {
        partial_missing_ancestor_recovers(false, false);
    }

    #[test]
    fn dircreate_partial_missing_ancestor_recovers_at_mkdir() {
        partial_missing_ancestor_recovers(false, true);
    }

    #[test]
    fn dircreate_interrupted_replan_recovers() {
        partial_missing_ancestor_recovers(true, false);
    }

    #[test]
    fn dircreate_interrupted_replan_recovers_at_mkdir() {
        partial_missing_ancestor_recovers(true, true);
    }

    #[test]
    fn l9_directory_creation_never_recreates_a_vanished_bound() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        std::fs::create_dir(&root).unwrap();
        let binding = crate::root_binding::RootBinding::new(&root);
        let mut creation = super::DirectoryCreation::new(&root.join("pages/sub"), &root).unwrap();
        std::fs::rename(&root, temp.path().join("moved")).unwrap();
        assert!(creation.finish(&binding, |_| Ok(())).is_err());
        assert!(!root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn dircreate_sync_refusal_retains_unpaid_debt_until_safe_retry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let binding = crate::root_binding::RootBinding::new(&root);
        let mut creation = super::DirectoryCreation::new(&root.join("pages/sub"), &root).unwrap();
        assert!(creation
            .finish(&binding, |parent| {
                if parent == root {
                    Ok(())
                } else {
                    Err(io::Error::other("sync failed"))
                }
            })
            .is_err());
        assert_eq!(
            creation.parents.len(),
            1,
            "paid root debt removed, pages debt retained"
        );
        std::fs::rename(root.join("pages"), root.join("old-pages")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("pages")).unwrap();
        assert!(creation
            .finish(&binding, |_| panic!("outside path must not sync"))
            .is_err());
        assert!(creation.has_sync_debt());
        std::fs::remove_file(root.join("pages")).unwrap();
        std::fs::rename(root.join("old-pages"), root.join("pages")).unwrap();
        creation
            .finish(&binding, super::sync_directory_entry)
            .unwrap();
        assert!(!creation.has_sync_debt());
    }

    #[test]
    fn l9_directory_creation_refuses_an_already_missing_bound() {
        let temp = tempfile::tempdir().unwrap();
        let bound = temp.path().join("missing");
        let error = super::DirectoryCreation::new(&bound.join("pages/sub"), &bound)
            .err()
            .unwrap();
        assert!(error
            .get_ref()
            .unwrap()
            .is::<crate::root_binding::GraphRootMoved>());
        assert!(!bound.exists());
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_errnos_are_tolerated_and_real_failures_are_not() {
        for errno in [
            libc::EBADF,
            libc::EACCES,
            libc::EISDIR,
            libc::EINVAL,
            libc::ENOTSUP,
        ] {
            assert!(
                dir_sync_is_unsupported(&io::Error::from_raw_os_error(errno)),
                "errno {errno}"
            );
        }
        for errno in [
            libc::ENOENT,
            libc::EIO,
            libc::ENOSPC,
            libc::EROFS,
            libc::EDQUOT,
        ] {
            assert!(
                !dir_sync_is_unsupported(&io::Error::from_raw_os_error(errno)),
                "errno {errno}"
            );
        }
    }
}

fn report_failure(error: io::Error) -> io::Error {
    io::Error::new(error.kind(), DirectorySyncFailure(error))
}
