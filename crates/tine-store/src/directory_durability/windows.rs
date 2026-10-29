//! Best-effort Windows directory flush; no stronger durability witness.
use super::DirectoryWitness;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
#[cfg(not(test))]
use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

#[cfg(test)]
use flush_fault::flush_file_buffers as FlushFileBuffers;

// Inject at the API call, after the real directory open. A test that replaces
// the FlushFileBuffers expression with success must leave the fault unconsumed.
#[cfg(test)]
mod flush_fault {
    use windows_sys::Win32::Foundation::{SetLastError, ERROR_WRITE_FAULT, HANDLE};

    thread_local! {
        pub(super) static FAIL_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub(super) unsafe fn flush_file_buffers(handle: HANDLE) -> i32 {
        if FAIL_NEXT.with(|fail| fail.replace(false)) {
            unsafe { SetLastError(ERROR_WRITE_FAULT) };
            0
        } else {
            unsafe { windows_sys::Win32::Storage::FileSystem::FlushFileBuffers(handle) }
        }
    }
}

pub(super) fn sync_directory(dir: &Path) -> DirectoryWitness {
    crate::storage_diag::record_windows_directory_flush(flush_directory(dir).is_ok());
    // Preserve the existing private caller's result on every open/flush
    // error. API success is not proof of namespace persistence.
    DirectoryWitness::Durable
}

fn flush_directory(dir: &Path) -> io::Result<()> {
    let path = crate::atomic_file::native_path(dir);
    if path[..path.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in directory path",
        ));
    }
    // FlushFileBuffers documents GENERIC_WRITE; MS-FSA 2.1.5.7 supplies no
    // smaller access recipe. BACKUP_SEMANTICS permits directory handles but
    // does not bypass ACLs without backup/restore privileges. Never use a
    // volume handle or request privileges. MS-FSA note <80> is NTFS-only:
    // other filesystems may return success without flushing anything.
    // Accepted latency risk for this best-effort app-data flush: GENERIC_WRITE
    // can wait for a conflicting oplock break acknowledgment despite share-all.
    // Microsoft documents no timeout (hence no finite bound) for that wait:
    // https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/breaking-oplocks
    // No timeout/cancellation here; results are preserved only once I/O returns.
    // App data may be redirected/cloud-backed; graph folders never use this.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // Own the valid handle so CloseHandle runs on both flush outcomes.
    let directory = unsafe { OwnedHandle::from_raw_handle(handle) };
    if unsafe { FlushFileBuffers(directory.as_raw_handle()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_durability::{
        sync_directory_entry, sync_directory_witness, sync_private_directory,
    };

    // Graph-folder callers never flush; only the private wrapper records.
    fn unchanged_success(dir: &Path) {
        assert_eq!(
            sync_directory_witness(dir).unwrap(),
            DirectoryWitness::Durable
        );
        sync_directory_entry(dir).unwrap();
        sync_private_directory(dir).unwrap();
    }

    #[test]
    fn windows_flush_failure_after_open_is_counted_and_preserves_wrapper_results() {
        let dir = tempfile::tempdir().unwrap();
        let inject = |run: &dyn Fn()| {
            struct ResetFault;
            impl Drop for ResetFault {
                fn drop(&mut self) {
                    flush_fault::FAIL_NEXT.with(|fail| fail.set(false));
                }
            }
            assert!(!flush_fault::FAIL_NEXT.with(|fail| fail.replace(true)));
            let _reset = ResetFault;
            run();
            assert!(
                !flush_fault::FAIL_NEXT.with(|fail| fail.get()),
                "must invoke FlushFileBuffers after successfully opening the directory"
            );
        };
        inject(&|| {
            assert_eq!(
                flush_directory(dir.path()).unwrap_err().raw_os_error(),
                Some(windows_sys::Win32::Foundation::ERROR_WRITE_FAULT as i32)
            );
        });

        let before = crate::storage_diag::StorageCounters::default().snapshot();
        inject(&|| {
            assert_eq!(
                sync_directory_witness(dir.path()).unwrap(),
                DirectoryWitness::Durable
            );
            sync_directory_entry(dir.path()).unwrap();
            assert!(
                flush_fault::FAIL_NEXT.with(|fail| fail.get()),
                "graph wrappers must not attempt the private flush"
            );
            sync_private_directory(dir.path()).unwrap();
        });
        let after = crate::storage_diag::StorageCounters::default().snapshot();
        assert!(
            after["windowsDirectoryFlush"]["apiFailed"]
                .as_u64()
                .unwrap()
                >= before["windowsDirectoryFlush"]["apiFailed"]
                    .as_u64()
                    .unwrap()
                    + 1
        );
        dir.close().unwrap();
    }

    #[test]
    fn windows_directory_flush_succeeds_on_hosted_ntfs() {
        // GitHub's Windows runners run elevated on NTFS. API success here
        // proves neither unprivileged access nor durability under power loss.
        let dir = tempfile::tempdir().unwrap();
        flush_directory(dir.path()).expect("writable hosted NTFS directory flush");
        let before = crate::storage_diag::StorageCounters::default().snapshot();
        unchanged_success(dir.path());
        let after = crate::storage_diag::StorageCounters::default().snapshot();
        assert!(
            after["windowsDirectoryFlush"]["apiSucceeded"]
                .as_u64()
                .unwrap()
                >= before["windowsDirectoryFlush"]["apiSucceeded"]
                    .as_u64()
                    .unwrap()
                    + 1
        );
        // Exercise immediate cleanup after the flush.
        dir.close().unwrap();
    }

    #[test]
    fn windows_directory_flush_failures_preserve_graph_and_private_results() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(
            flush_directory(&missing).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let before = crate::storage_diag::StorageCounters::default().snapshot();
        unchanged_success(&missing);

        // A real sharing violation at CreateFileW, independent of elevation.
        let exclusive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir.path())
            .unwrap();
        assert_eq!(
            flush_directory(dir.path()).unwrap_err().raw_os_error(),
            Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
        );
        unchanged_success(dir.path());
        drop(exclusive);
        let after = crate::storage_diag::StorageCounters::default().snapshot();
        assert_eq!(
            after["windowsDirectoryFlush"]["scope"],
            "process-since-launch"
        );
        assert!(
            after["windowsDirectoryFlush"]["apiFailed"]
                .as_u64()
                .unwrap()
                >= before["windowsDirectoryFlush"]["apiFailed"]
                    .as_u64()
                    .unwrap()
                    + 2
        );
        dir.close().unwrap();
    }
}
