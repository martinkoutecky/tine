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

/// Synchronize the supplied directory where the platform supports it.
/// `EINVAL` and `ENOTSUP` from sync are treated as filesystem non-support;
/// other open or sync errors are returned. Master 54dfcc1b6674 additionally
/// swallows `EBADF`, `EACCES`, `EISDIR`, `PermissionDenied` and `NotFound`;
/// og does not, because none of them is a "this filesystem never offers
/// directory sync" signal on a shipped target: the open here is `O_RDONLY`,
/// which never yields `EISDIR` and gives a descriptor Linux, Android, macOS and
/// iOS accept for `fsync` (so `EBADF` would be a real fault); an unopenable
/// directory (`EACCES`) and a vanished one (`NotFound`, which also took the
/// renamed file) give no durability at all. Swallowing them would acknowledge
/// a save a crash can lose (I-2); as errors, the caller re-reads disk state.
/// Pinned by `tests/directory_durability_guard.rs`. Windows returns success without a
/// directory flush; standard file opening cannot flush its directory handles,
/// and the caller is responsible for its own rename durability protocol.
pub fn sync_directory_entry(dir: &Path) -> io::Result<()> {
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
        let directory = std::fs::File::open(dir).map_err(report_failure)?;
        match directory.sync_all() {
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::EINVAL) | Some(libc::ENOTSUP)
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(report_failure(error)),
            Ok(()) => Ok(()),
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

fn report_failure(error: io::Error) -> io::Error {
    io::Error::new(error.kind(), DirectorySyncFailure(error))
}
