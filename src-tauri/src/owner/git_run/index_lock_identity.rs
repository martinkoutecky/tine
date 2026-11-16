//! File identity for IndexLock ownership, shared by install and cleanup.

use super::LockRefusal;
use std::path::Path;

/// The held handle prevents ID reuse while ownership is checked. No time or
/// size fallback: unavailable/zero IDs refuse; unstable filesystem IDs can
/// refuse a safe commit. This is a check before each mutation, not an atomic
/// conditional rename/unlink (see the storage contract's limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IndexLockIdentity {
    volume: u64,
    file: u128,
}

#[cfg(test)]
thread_local! {
    /// Test seam: make the next identity capture fail, as FileIdInfo and the
    /// file-index fallback both would on a filesystem without file IDs.
    pub(crate) static FAIL_CAPTURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl IndexLockIdentity {
    /// The identity of the lock Tine just created.
    pub(super) fn capture(file: &std::fs::File) -> std::io::Result<Self> {
        #[cfg(test)]
        if FAIL_CAPTURE.with(|fail| fail.replace(false)) {
            return Err(std::io::Error::other("forced identity failure"));
        }
        Self::from_file(file)
    }

    /// The identity of the lock `create_new` just made at `lock_file`. If it
    /// cannot be read, the lock is ours, so it is removed again: leaving it
    /// behind would make every later commit (and the user's own git) report
    /// the index as locked.
    pub(super) fn capture_new(
        lock: std::fs::File,
        lock_file: &Path,
    ) -> Result<(std::fs::File, Self), LockRefusal> {
        match Self::capture(&lock) {
            Ok(identity) => Ok((lock, identity)),
            Err(e) => {
                drop(lock);
                let _ = std::fs::remove_file(lock_file);
                Err(LockRefusal::Failed(format!(
                    "the index lock identity could not be read: {e}"
                )))
            }
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    pub(super) fn from_file(file: &std::fs::File) -> std::io::Result<Self> {
        Self::from_metadata(&file.metadata()?)
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    fn from_metadata(meta: &std::fs::Metadata) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        Self::checked(meta.dev(), u128::from(meta.ino()))
    }

    #[cfg(target_os = "windows")]
    pub(super) fn from_file(file: &std::fs::File) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
        };
        let mut info = FILE_ID_INFO::default();
        let result = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if result != 0 {
            return Self::checked(
                info.VolumeSerialNumber,
                u128::from_le_bytes(info.FileId.Identifier),
            );
        }
        // FAT32/exFAT (fastfat) and Samba shares do not implement FileIdInfo;
        // they do answer the older 32-bit volume serial + 64-bit file index.
        Self::from_file_index(file)
    }

    #[cfg(target_os = "windows")]
    fn from_file_index(file: &std::fs::File) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Self::checked(
            u64::from(info.dwVolumeSerialNumber),
            (u128::from(info.nFileIndexHigh) << 32) | u128::from(info.nFileIndexLow),
        )
    }

    fn checked(volume: u64, file: u128) -> std::io::Result<Self> {
        if file == 0 {
            return Err(std::io::Error::other(
                "the filesystem supplies no index lock identity",
            ));
        }
        Ok(Self { volume, file })
    }

    pub(super) fn matches_path(self, path: &Path) -> bool {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return false;
        };
        if !meta.file_type().is_file() {
            return false;
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let current = Self::from_metadata(&meta);
        #[cfg(target_os = "windows")]
        let current = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
            // std metadata's file-ID accessors are unstable. Open the entry
            // without following reparse points, sharing deletion like the
            // held handle, then use the existing Win32 dependency.
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .and_then(|file| {
                    if !file.metadata()?.file_type().is_file() {
                        return Err(std::io::Error::other("the index lock is not a file"));
                    }
                    Self::from_file(&file)
                })
        };
        current.is_ok_and(|identity| identity == self)
    }
}

impl super::IndexLock {
    pub(super) fn check_ownership(&self) -> Result<(), LockRefusal> {
        self.guard
            .check_path(&self.lock_file)
            .map_err(|e| LockRefusal::Failed(e.to_string()))?;
        if !self.identity.matches_path(&self.lock_file) {
            return Err(LockRefusal::Failed(
                "the index lock was removed while Tine held it; Tine left the index and current lock alone".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_lock_identity_names_every_shipped_target() {
        let source = include_str!("index_lock_identity.rs");
        let identity = source
            .split("impl IndexLockIdentity {")
            .nth(1)
            .unwrap()
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        for os in ["linux", "macos", "ios", "android", "windows"] {
            let expected = 3;
            assert_eq!(
                identity.matches(&format!("target_os = \"{os}\"")).count(),
                expected,
                "{os}"
            );
        }
        assert_eq!(identity.matches("fn from_file(").count(), 2);
    }

    #[test]
    fn index_lock_identity_refuses_unavailable_file_ids() {
        assert!(IndexLockIdentity::checked(1, 0).is_err());
    }
}
