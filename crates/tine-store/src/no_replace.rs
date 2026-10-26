//! One platform-specific no-replace move for graph, restore, and device files.

use cap_std::fs::Dir;
use std::ffi::OsStr;
use std::io;
use std::path::Path;

/// Move an absolute or caller-resolved path atomically if its destination is absent.
pub(crate) fn move_file_noreplace(src: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(test)]
    if let Some(kind) = MOVE_ERRORS.with(|errors| errors.borrow_mut().pop_front()) {
        return Err(io::Error::new(kind, "injected no-replace move failure"));
    }
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
    #[cfg(test)]
    let refused_for_test = REFUSE_NOREPLACE_FLAG.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let refused_for_test = false;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = if refused_for_test {
        -1
    } else {
        unsafe {
            // Android's renameat2 wrapper is unavailable on older API levels.
            libc::syscall(
                libc::SYS_renameat2,
                from_dir,
                from.as_ptr(),
                to_dir,
                to.as_ptr(),
                libc::RENAME_NOREPLACE as libc::c_uint,
            )
        }
    };
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let result = if refused_for_test {
        -1
    } else {
        unsafe {
            libc::renameatx_np(
                from_dir,
                from.as_ptr(),
                to_dir,
                to.as_ptr(),
                libc::RENAME_EXCL as libc::c_uint,
            ) as libc::c_long
        }
    };
    if result == 0 {
        return Ok(());
    }
    let error = match refused_for_test {
        true => io::Error::from_raw_os_error(libc::EINVAL),
        false => io::Error::last_os_error(),
    };
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const FLAG_STEP: &str = "renameat2(RENAME_NOREPLACE)";
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const FLAG_STEP: &str = "renameatx_np(RENAME_EXCL)";
    destination_absent_where_the_flag_is_refused(error, to_dir, &to)
        .map_err(super::platform_step::at(FLAG_STEP))?;
    let result = unsafe { libc::renameat(from_dir, from.as_ptr(), to_dir, to.as_ptr()) };
    (result == 0).then_some(()).ok_or_else(|| {
        super::platform_step::at("renameat after the no-replace flag was refused")(
            io::Error::last_os_error(),
        )
    })
}

/// GH #538 (master 1739c5109a72; Martin's decision B1, 2026-09-24): Android
/// 11-14 shared storage (MediaProvider's FUSE daemon) and NFS answer the
/// no-replace flag with `EINVAL` while a plain rename works, so every save,
/// create and page rename failed there. Only a refusal of the flag itself
/// (`EINVAL`, `ENOSYS`, `EOPNOTSUPP`/`ENOTSUP`) lets [`move_at`] fall back to a
/// plain rename, and only after this absence check; an occupied `to` stays
/// `EEXIST` with nothing moved, and every other failure is returned. Given up
/// on such storage only: a file an external writer creates at `to` in the
/// microseconds between the check and the rename is replaced. Windows has no
/// such refusal on record and keeps its single call.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn destination_absent_where_the_flag_is_refused(
    error: io::Error,
    to_dir: std::os::fd::RawFd,
    to: &std::ffi::CStr,
) -> io::Result<()> {
    let refused = [libc::EINVAL, libc::ENOSYS, libc::EOPNOTSUPP, libc::ENOTSUP];
    if !error
        .raw_os_error()
        .is_some_and(|errno| refused.contains(&errno))
    {
        return Err(error);
    }
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let probe = unsafe {
        libc::fstatat(
            to_dir,
            to.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if probe == 0 {
        return Err(io::Error::from_raw_os_error(libc::EEXIST));
    }
    match io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        true => Ok(()),
        false => Err(error),
    }
}

#[cfg(test)]
thread_local! {
    /// Fault the audited move boundary without changing either real path.
    pub(crate) static MOVE_ERRORS: std::cell::RefCell<std::collections::VecDeque<io::ErrorKind>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// Make this thread's no-replace moves fail as flag-refusing storage does
    /// (`EINVAL`, GH #538); `no_replace_tests.rs` drives it through the store.
    pub(crate) static REFUSE_NOREPLACE_FLAG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(windows)]
fn move_windows(src: &Path, dest: &Path) -> io::Result<()> {
    // The shared path form: past MAX_PATH as `\\?\` (REVIEW-2b-r2 R2).
    let src = super::atomic_file::native_path(src);
    let dest = super::atomic_file::native_path(dest);
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
    (result != 0).then_some(()).ok_or_else(|| {
        super::platform_step::at("MoveFileExW(MOVEFILE_WRITE_THROUGH)")(io::Error::last_os_error())
    })
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
