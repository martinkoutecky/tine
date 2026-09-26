//! One platform-specific no-replace move for graph, restore, and device files.

use cap_std::fs::Dir;
use std::ffi::OsStr;
use std::io;
use std::path::Path;

/// Move an absolute or caller-resolved path atomically if its destination is absent.
pub(crate) fn move_file_noreplace(src: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        use std::os::fd::RawFd;
        move_at(libc::AT_FDCWD as RawFd, src, libc::AT_FDCWD as RawFd, dest)
    }
    #[cfg(target_os = "windows")]
    {
        move_windows(src, dest)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        let _ = (src, dest);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no-replace move is unavailable",
        ))
    }
}

/// Move a name between already-open directory handles without replacing a target.
pub(crate) fn rename_noreplace_dir(
    from_dir: &Dir,
    from: &Path,
    to_dir: &Dir,
    to: &OsStr,
) -> io::Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        use std::os::fd::AsRawFd;
        move_at(
            from_dir.as_raw_fd(),
            from,
            to_dir.as_raw_fd(),
            Path::new(to),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let source = directory_path(from_dir)?.join(from);
        let target = directory_path(to_dir)?.join(to);
        move_windows(&source, &target)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        let _ = (from_dir, from, to_dir, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no-replace move is unavailable",
        ))
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn move_at(
    from_dir: std::os::fd::RawFd,
    from: &Path,
    to_dir: std::os::fd::RawFd,
    to: &Path,
) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = unsafe {
        // Android's renameat2 wrapper is unavailable on older API levels.
        libc::syscall(
            libc::SYS_renameat2,
            from_dir,
            from.as_ptr(),
            to_dir,
            to.as_ptr(),
            libc::RENAME_NOREPLACE as libc::c_uint,
        )
    };
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let result = unsafe {
        libc::renameatx_np(
            from_dir,
            from.as_ptr(),
            to_dir,
            to.as_ptr(),
            libc::RENAME_EXCL as libc::c_uint,
        ) as libc::c_long
    };
    (result == 0)
        .then_some(())
        .ok_or_else(io::Error::last_os_error)
}

#[cfg(windows)]
fn move_windows(src: &Path, dest: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let mut src: Vec<u16> = src.as_os_str().encode_wide().collect();
    let mut dest: Vec<u16> = dest.as_os_str().encode_wide().collect();
    src.push(0);
    dest.push(0);
    // Rust std::fs::rename uses MOVEFILE_REPLACE_EXISTING on Windows. This
    // write-through move omits that flag, so an existing target is refused.
    // Windows has no supported directory fsync; this also makes publication
    // wait for the namespace move to reach disk.
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            src.as_ptr(),
            dest.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    (result != 0)
        .then_some(())
        .ok_or_else(io::Error::last_os_error)
}

#[cfg(windows)]
fn directory_path(dir: &Dir) -> io::Result<std::path::PathBuf> {
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let handle = dir.as_raw_handle();
    let needed = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, 0) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut wide = vec![0u16; needed as usize + 1];
    let written =
        unsafe { GetFinalPathNameByHandleW(handle, wide.as_mut_ptr(), wide.len() as u32, 0) };
    if written == 0 || written as usize >= wide.len() {
        return Err(io::Error::last_os_error());
    }
    Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
        &wide[..written as usize],
    )))
}
