//! Directory-entry durability primitives shared by graph projections and oplog
//! publication.
//!
//! Regular files are flushed by their callers before an atomic name operation.
//! On Windows we clone and validate the retained directory capability as an
//! exact real directory rather than a reparse point. Win32 does not document a
//! directory-entry flush operation: `FlushFileBuffers` requires a
//! `GENERIC_WRITE` file handle, while directory handles are valid only for APIs
//! that explicitly accept them. The validated Windows state therefore models
//! directory-entry flushing as unsupported instead of interpreting an error
//! from an attempted reopen or flush. File bytes remain flushed and publication
//! remains atomic, but the directory entry is not promised to survive a crash.
//! Clone, metadata, validation, publication, and regular-file flush failures
//! remain fatal.

use cap_std::fs::Dir;
use std::fs;
use std::io;

/// A directory capability validated for a durable name-operation publication.
#[cfg(windows)]
pub struct ValidatedDirectorySync {
    // Retain the exact validated object for the whole publication. cap-std
    // opens directory capabilities without FILE_SHARE_DELETE, so this object
    // cannot be renamed or deleted underneath the operation.
    _capability: fs::File,
    entry_durability: WindowsDirectoryEntryDurability,
}

/// A directory capability validated for a durable name-operation publication.
#[cfg(not(windows))]
pub struct ValidatedDirectorySync<'a>(&'a Dir);

#[cfg(any(test, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsDirectoryEntryDurability {
    UnsupportedAfterValidation,
}

#[cfg(windows)]
impl ValidatedDirectorySync {
    /// Validate and retain `dir` for the duration of a publication.
    pub fn open(dir: &Dir) -> io::Result<Self> {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        let capability = dir.try_clone()?.into_std_file();
        let metadata = capability.metadata()?;
        let entry_durability = validated_windows_directory_entry_durability(
            metadata.is_dir(),
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        )?;

        // No path is opened after this validation. The exact cloned capability
        // moves into the result, so the platform-limit state cannot be rebound
        // to a followed or substituted directory.
        Ok(Self {
            _capability: capability,
            entry_durability,
        })
    }

    /// Validate that directory synchronization can proceed.
    pub fn preflight(&self) -> io::Result<()> {
        self.sync()
    }

    /// Synchronize the directory entry or report the platform durability limit.
    pub fn sync(&self) -> io::Result<()> {
        match self.entry_durability {
            // This state is constructed only after the retained handle's clone,
            // metadata query, and exact-target validation have succeeded. It
            // accepts no io::Error, so an access-denied or other failure from
            // any real I/O stage cannot accidentally become the fallback.
            WindowsDirectoryEntryDurability::UnsupportedAfterValidation => Ok(()),
        }
    }
}

#[cfg(unix)]
impl<'a> ValidatedDirectorySync<'a> {
    /// Validate and retain `dir` for the duration of a publication.
    pub fn open(dir: &'a Dir) -> io::Result<Self> {
        Ok(Self(dir))
    }

    /// Validate that directory synchronization can proceed.
    pub fn preflight(&self) -> io::Result<()> {
        Ok(())
    }

    /// Synchronize the directory entry.
    pub fn sync(&self) -> io::Result<()> {
        use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};

        // cap-std may retain an O_PATH capability, which is suitable for openat
        // but cannot itself be fsynced. Open `.` as a real directory descriptor.
        let fd = unsafe {
            libc::openat(
                self.0.as_fd().as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned one newly owned directory descriptor.
        unsafe { fs::File::from_raw_fd(fd) }.sync_all()
    }
}

#[cfg(not(any(unix, windows)))]
impl<'a> ValidatedDirectorySync<'a> {
    /// Validate and retain `dir` for the duration of a publication.
    pub fn open(dir: &'a Dir) -> io::Result<Self> {
        Ok(Self(dir))
    }

    /// Validate that directory synchronization can proceed.
    pub fn preflight(&self) -> io::Result<()> {
        Ok(())
    }

    /// Synchronize the directory entry.
    pub fn sync(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory durability is unsupported on this target",
        ))
    }
}

/// Synchronize `dir` after a required durable directory-entry update.
pub fn sync_dir_required(dir: &Dir) -> io::Result<()> {
    ValidatedDirectorySync::open(dir)?.sync()
}

#[cfg(any(test, windows))]
fn validated_windows_directory_entry_durability(
    is_dir: bool,
    is_reparse: bool,
) -> io::Result<WindowsDirectoryEntryDurability> {
    if !is_dir || is_reparse {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory durability handle is not a real no-follow directory",
        ));
    }
    Ok(WindowsDirectoryEntryDurability::UnsupportedAfterValidation)
}

#[cfg(test)]
mod tests {
    use super::{validated_windows_directory_entry_durability, WindowsDirectoryEntryDurability};
    use std::io;

    #[test]
    fn validated_real_directory_has_explicit_windows_durability_limit() {
        assert_eq!(
            validated_windows_directory_entry_durability(true, false).unwrap(),
            WindowsDirectoryEntryDurability::UnsupportedAfterValidation
        );
    }

    #[test]
    fn windows_directory_validation_rejects_reparse_and_non_directory_handles() {
        assert_eq!(
            validated_windows_directory_entry_durability(false, false)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            validated_windows_directory_entry_durability(true, true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_retained_directory_handle_validates_before_platform_limit() {
        use super::ValidatedDirectorySync;
        use cap_std::{ambient_authority, fs::Dir};
        use std::fs;
        use uuid::Uuid;

        let path =
            std::env::temp_dir().join(format!("tine-directory-durability-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let sync = ValidatedDirectorySync::open(&dir).unwrap();
        assert_eq!(
            sync.entry_durability,
            WindowsDirectoryEntryDurability::UnsupportedAfterValidation
        );
        sync.preflight().unwrap();
        sync.sync().unwrap();
        drop(sync);
        drop(dir);
        fs::remove_dir(path).unwrap();
    }
}
