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
    #[cfg(feature = "test-faults")]
    SYNCED.with(|synced| synced.borrow_mut().push(dir.to_path_buf()));
    #[cfg(all(feature = "test-faults", unix))]
    if FAIL_NEXT_SYNC.with(|fail| fail.replace(false)) {
        return Err(report_failure(io::Error::other(
            "injected directory sync I/O failure",
        )));
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        match std::fs::File::open(dir).and_then(|directory| directory.sync_all()) {
            Ok(()) => Ok(()),
            Err(error) if dir_sync_is_unsupported(&error) => Ok(()),
            Err(error) => Err(report_failure(error)),
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Directory handles cannot be synced with std::fs. NTFS renames are
        // journaled, but this helper does not verify the filesystem or rename.
        let _ = dir;
        Ok(())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        let _ = dir;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory sync unsupported on this target",
        ))
    }
}

/// `fs::create_dir_all(dir)`, then sync the parent of every directory it
/// created, so a file published into `dir` afterwards survives power loss
/// with its whole directory chain. Scenario: power loss after a move into a
/// new trash/recovery or page subdirectory — the source parent's sync makes the
/// removal durable while the new directory's own entry is not, losing the file.
/// Cost: one `metadata` per missing ancestor plus the existing one, and one
/// sync per created directory; nothing beyond one `metadata` when `dir` exists.
pub(crate) fn create_dir_all_durable(dir: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut probe = dir;
    while std::fs::metadata(probe).is_err() {
        missing.push(probe);
        match probe.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => probe = parent,
            _ => break,
        }
    }
    std::fs::create_dir_all(dir)?;
    for created in missing.into_iter().rev() {
        if let Some(parent) = created.parent().filter(|p| !p.as_os_str().is_empty()) {
            sync_directory_entry(parent)?;
        }
    }
    Ok(())
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
        io::ErrorKind::Unsupported
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::NotFound
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
        for errno in [libc::EIO, libc::ENOSPC, libc::EROFS, libc::EDQUOT] {
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
