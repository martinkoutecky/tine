//! Platform durability helpers shared by managed-storage bootstrap paths.
//!
//! Linux can cheaply flush the filesystem containing a complete private tree.
//! Android exposes the same syscall, but some kernels and ROMs deny that
//! filesystem-wide operation even though ordinary app-private file and
//! directory synchronization works. In that case we synchronize the exact
//! private tree instead of refusing managed-storage activation.

use std::fs;
use std::fs::File;
#[cfg(any(test, not(target_os = "linux")))]
use std::fs::OpenOptions;
use std::io;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::fd::{AsFd as _, AsRawFd as _};
use std::path::Path;

#[cfg(any(test, not(target_os = "linux")))]
use cap_std::ambient_authority;
use cap_std::fs::Dir;

pub(crate) fn sync_private_tree(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        return sync_filesystem(path);
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
    finish_android_reconstructible_directory_sync(tine_storage::sync_dir_required(directory))
}

/// Synchronize one file in a pre-promotion tree which can be recreated from
/// unchanged Markdown. Some Android storage stacks permit the complete
/// create/write/read sequence but refuse `fsync` with a capability-style
/// error. A returned success in that case does not promote the file to
/// authority: callers must still reread exact bytes, and an interrupted
/// activation discards/rebuilds the enclosing tree.
pub(crate) fn sync_reconstructible_file(file: &File) -> io::Result<()> {
    finish_android_reconstructible_file_sync(file.sync_all())
}

#[cfg(target_os = "android")]
fn finish_android_reconstructible_directory_sync(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if android_filesystem_sync_may_fallback(error.kind()) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "android")]
fn finish_android_reconstructible_file_sync(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if android_filesystem_sync_may_fallback(error.kind()) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(target_os = "android"))]
fn finish_android_reconstructible_file_sync(result: io::Result<()>) -> io::Result<()> {
    result
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

#[cfg(test)]
fn simulate_android_reconstructible_file_sync(result: io::Result<()>) -> io::Result<()> {
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

    #[test]
    fn android_reconstructible_file_accepts_only_capability_refusals() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported,
            io::ErrorKind::InvalidInput,
        ] {
            simulate_android_reconstructible_file_sync(Err(io::Error::new(kind, "injected")))
                .unwrap();
        }
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::Interrupted,
            io::ErrorKind::InvalidData,
            io::ErrorKind::WriteZero,
            io::ErrorKind::Other,
        ] {
            assert_eq!(
                simulate_android_reconstructible_file_sync(Err(io::Error::new(kind, "injected")))
                    .unwrap_err()
                    .kind(),
                kind
            );
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
