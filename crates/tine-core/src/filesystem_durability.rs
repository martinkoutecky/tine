//! Platform durability helpers shared by managed-storage bootstrap paths.
//!
//! Linux can cheaply flush the filesystem containing a complete private tree.
//! Android exposes the same syscall, but some kernels and ROMs deny that
//! filesystem-wide operation even though ordinary app-private file and
//! directory synchronization works. In that case we synchronize the exact
//! private tree instead of refusing managed-storage activation.

use std::fs;
#[cfg(any(test, not(target_os = "linux")))]
use std::fs::OpenOptions;
use std::io;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::fd::{AsFd as _, AsRawFd as _};
use std::path::Path;

use cap_std::ambient_authority;
use cap_std::fs::Dir;

pub(crate) fn sync_private_tree(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        return crate::perf_count::timed(crate::perf_count::P::SyncFs, || sync_filesystem(path));
    }

    #[cfg(target_os = "android")]
    {
        return finish_android_private_tree_sync(path, sync_filesystem(path));
    }

    #[cfg(not(target_os = "linux"))]
    sync_private_tree_exact(path)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn sync_filesystem(path: &Path) -> io::Result<()> {
    let directory = fs::File::open(path)?;
    // SAFETY: the opened descriptor names the filesystem containing the
    // complete private tree. Android may reject this filesystem-wide operation
    // and then takes the exact-tree path below.
    let result = unsafe { libc::syncfs(directory.as_fd().as_raw_fd()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(test, target_os = "android"))]
const fn android_filesystem_sync_may_fallback(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
    )
}

/// Which durability class an artifact belongs to. Platform durability policy is
/// stated per class and per platform; it is never global and never a blanket
/// "ignore I/O errors".
///
/// The two classes are not symmetric, and the asymmetry is the whole contract:
///
/// * [`DurabilityArtifactClass::PrivateDurableAuthority`] — the oplog manifest,
///   its object archive, the local journal and the receipt store, all below
///   app-private storage. These *are* the durable truth. A barrier they cannot
///   provide is a real durability failure and stays fatal on every platform,
///   Android included.
/// * [`DurabilityArtifactClass::SharedReconstructibleProjection`] — the
///   Markdown/Org projection of an already-accepted manifest into the user's
///   graph tree. On Android that tree is shared storage served by FUSE
///   (MediaProvider) or sdcardfs, which does not uniformly provide the
///   filesystem-wide and directory-level flush primitives and reports the
///   refusal as `EPERM`/`ENOTSUP`/`EINVAL`. Those bytes are derived state: the
///   accepted manifest in private storage still records them, and a crash that
///   loses an unflushed directory entry is repaired by the projection drain on
///   the next open, exactly as an interrupted projection already is. Retrying a
///   capability refusal cannot ever succeed, so retrying it forever is not
///   crash-safety — it is an availability bug that strands the user's edit.
///   On Android only, and only for those three capability errors, the barrier
///   degrades. Every other errno (a real I/O error, `ENOSPC`, `EIO`) stays
///   fatal, and every other platform stays strict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DurabilityArtifactClass {
    PrivateDurableAuthority,
    SharedReconstructibleProjection,
}

/// Apply the per-class, per-platform policy to one durability barrier result.
///
/// Only the Android leg can degrade, and only for the reconstructible
/// projection class. Every other platform returns the result unchanged, so a
/// desktop or Windows build has no branch that can swallow a barrier failure.
pub(crate) fn finish_durability_barrier(
    class: DurabilityArtifactClass,
    result: io::Result<()>,
) -> io::Result<()> {
    #[cfg(target_os = "android")]
    {
        return android_durability_barrier(class, result);
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = class;
        result
    }
}

/// The exact Android arm of [`finish_durability_barrier`], reachable from host
/// tests so the branch the device takes is the branch under test.
#[cfg(any(test, target_os = "android"))]
pub(crate) fn android_durability_barrier(
    class: DurabilityArtifactClass,
    result: io::Result<()>,
) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                class,
                DurabilityArtifactClass::SharedReconstructibleProjection
            ) && android_filesystem_sync_may_fallback(error.kind()) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(test, target_os = "android"))]
fn finish_android_private_tree_sync(path: &Path, result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if android_filesystem_sync_may_fallback(error.kind()) => {
            sync_private_tree_exact(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(test, not(target_os = "linux")))]
fn sync_private_tree_exact(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private durability tree contains a symlink",
            ));
        }
        if metadata.is_dir() {
            sync_private_tree_exact(&child)?;
        } else if metadata.is_file() {
            sync_regular_file(&child)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private durability tree contains a non-file entry",
            ));
        }
    }
    let directory = Dir::open_ambient_dir(path, ambient_authority())?;
    sync_reconstructible_directory(&directory)
}

/// Synchronize a directory belonging to a pre-promotion, reconstructible
/// managed-storage tree. Android app sandboxes and vendor filesystems can deny
/// directory fsync even after permitting every exact file sync. That platform
/// limitation must not block activation; ordinary I/O errors remain fatal.
pub(crate) fn sync_reconstructible_directory(directory: &Dir) -> io::Result<()> {
    // Counted inside tine_storage::sync_dir_required (filesystem.rs:240).
    finish_android_reconstructible_directory_sync(tine_storage::sync_dir_required(directory))
}

/// Synchronize one already-created directory entry while its contents are
/// still reconstructible from Direct Files. This is the publication barrier
/// used immediately before the clean activation marker: Linux/desktop keeps
/// the strict directory barrier, while Android tolerates only the documented
/// directory-fsync capability refusal after all exact files were flushed.
pub(crate) fn sync_reconstructible_directory_path(path: &Path) -> io::Result<()> {
    let directory = Dir::open_ambient_dir(path, ambient_authority())?;
    sync_reconstructible_directory(&directory)
}

#[cfg(target_os = "android")]
fn finish_android_reconstructible_directory_sync(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if android_filesystem_sync_may_fallback(error.kind()) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(target_os = "android"))]
fn finish_android_reconstructible_directory_sync(result: io::Result<()>) -> io::Result<()> {
    result
}

#[cfg(test)]
fn simulate_android_reconstructible_directory_sync(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if android_filesystem_sync_may_fallback(error.kind()) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(any(test, not(target_os = "linux")))]
pub(crate) fn sync_regular_file(path: &Path) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    // FlushFileBuffers requires a write-capable handle on Windows.
    #[cfg(windows)]
    options.write(true);
    options.open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_falls_back_only_when_the_filesystem_wide_primitive_is_unavailable() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported,
            io::ErrorKind::InvalidInput,
        ] {
            assert!(android_filesystem_sync_may_fallback(kind));
        }
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::Interrupted,
            io::ErrorKind::InvalidData,
            io::ErrorKind::WriteZero,
            io::ErrorKind::Other,
        ] {
            assert!(!android_filesystem_sync_may_fallback(kind));
        }
    }

    #[test]
    fn android_capability_refusal_flushes_the_exact_private_tree() {
        let root = std::env::temp_dir().join(format!(
            "tine-android-private-tree-sync-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/artifact"), b"durable private state").unwrap();

        finish_android_private_tree_sync(
            &root,
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "simulated Android syncfs refusal",
            )),
        )
        .unwrap();

        assert_eq!(
            fs::read(root.join("nested/artifact")).unwrap(),
            b"durable private state"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// The artifact-class asymmetry, stated as a test rather than a comment:
    /// Android degrades the reconstructible projection barrier and NEVER the
    /// private durable authority, and no platform degrades anything that is not
    /// one of the three documented capability refusals.
    #[test]
    fn only_the_reconstructible_projection_class_degrades_and_only_on_android() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported,
            io::ErrorKind::InvalidInput,
        ] {
            android_durability_barrier(
                DurabilityArtifactClass::SharedReconstructibleProjection,
                Err(io::Error::new(kind, "capability refusal")),
            )
            .unwrap();

            // The durable authority keeps the strict barrier on Android too.
            let error = android_durability_barrier(
                DurabilityArtifactClass::PrivateDurableAuthority,
                Err(io::Error::new(kind, "capability refusal")),
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);

            // And off Android nothing degrades at all.
            #[cfg(not(target_os = "android"))]
            {
                let error = finish_durability_barrier(
                    DurabilityArtifactClass::SharedReconstructibleProjection,
                    Err(io::Error::new(kind, "capability refusal")),
                )
                .unwrap_err();
                assert_eq!(error.kind(), kind);
            }
        }
    }

    #[test]
    fn a_real_io_failure_on_the_projection_stays_fatal_even_on_android() {
        for kind in [
            io::ErrorKind::WriteZero,
            io::ErrorKind::StorageFull,
            io::ErrorKind::Other,
            io::ErrorKind::NotFound,
        ] {
            let error = android_durability_barrier(
                DurabilityArtifactClass::SharedReconstructibleProjection,
                Err(io::Error::new(kind, "real I/O failure")),
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);
        }
    }

    #[test]
    fn android_reconstructible_directory_accepts_only_capability_refusals() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported,
            io::ErrorKind::InvalidInput,
        ] {
            simulate_android_reconstructible_directory_sync(Err(io::Error::new(kind, "denied")))
                .unwrap();
        }
        let error = simulate_android_reconstructible_directory_sync(Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "real I/O failure",
        )))
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
    }
}
