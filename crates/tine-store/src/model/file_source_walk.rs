//! The disk's walk down a spelling's names from the graph root, one name per
//! step, each asked relative to the directory the step before opened: a
//! step submits one component to the filesystem, never a whole prefix, so
//! finding the deepest existing prefix of a depth-`d` spelling costs `O(d)`
//! components ([`super::locate_deepest`]). std has no relative open, and
//! cap-std on Windows re-joins its directory's full path for every open, so
//! each platform asks its own: `openat`/`fstatat` on Unix, `NtCreateFile`
//! with a root directory handle on Windows.
use super::EntryKind;
use std::io;
use std::path::Path;

/// A walk from `root`, opened on its first step. A step follows a link, as
/// a lookup through an ancestor does; a directory it finds becomes the next
/// step's base, entered only when a next step comes. The only buffer is one
/// name, reused (no allocation per step once it holds the longest name).
pub(super) struct DiskWalk<'r> {
    root: &'r Path,
    dir: Option<Base>,
    /// The directory the last step found (named by `name`), not yet entered.
    found: Option<Found>,
    name: Vec<NameUnit>,
}

#[cfg(unix)]
type Base = std::os::fd::OwnedFd;
/// Unix finds a directory by `fstatat` and enters it by name (`openat`).
#[cfg(unix)]
type Found = ();
#[cfg(unix)]
type NameUnit = u8;
#[cfg(windows)]
type Base = std::fs::File;
/// Windows finds a directory by opening it: the handle is the next base.
#[cfg(windows)]
type Found = std::fs::File;
#[cfg(windows)]
type NameUnit = u16;
#[cfg(not(any(unix, windows)))]
type Base = ();
#[cfg(not(any(unix, windows)))]
type Found = ();
#[cfg(not(any(unix, windows)))]
type NameUnit = u8;

impl<'r> DiskWalk<'r> {
    pub(super) fn new(root: &'r Path) -> DiskWalk<'r> {
        DiskWalk {
            root,
            dir: None,
            found: None,
            name: Vec::new(),
        }
    }

    /// What `name` is under the directory the last step found (the root on
    /// the first): one probe, after entering that directory.
    pub(super) fn step(&mut self, name: &str) -> io::Result<EntryKind> {
        #[cfg(test)]
        super::names_tests::DISK_PROBES.with(|count| count.set(count.get() + 1));
        if self.dir.is_none() {
            self.dir = Some(open_root(self.root)?);
        }
        if let Some(found) = self.found.take() {
            let dir = self.dir.as_ref().expect("opened above");
            self.dir = Some(enter(dir, found, &self.name)?);
        }
        let dir = self.dir.as_ref().expect("opened above");
        let (kind, found) = step_in(dir, name, &mut self.name)?;
        self.found = found;
        Ok(kind)
    }
}

#[cfg(test)]
fn submitted() {
    super::names_tests::COMPONENTS.with(|count| count.set(count.get() + 1));
}

#[cfg(unix)]
fn open_root(root: &Path) -> io::Result<Base> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | dir_flags())
        .open(root)?;
    Ok(file.into())
}

/// A directory opened only to be walked through: no read of its entries is
/// needed where the platform can say so (Linux and Android `O_PATH`).
#[cfg(unix)]
fn dir_flags() -> libc::c_int {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let flags = libc::O_PATH;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let flags = 0;
    flags
}

/// `fstatat` of `name` under `dir`, following a link: one component. The
/// name stays in `buffer`, NUL-terminated, for [`enter`].
#[cfg(unix)]
fn step_in(dir: &Base, name: &str, buffer: &mut Vec<u8>) -> io::Result<(EntryKind, Option<Found>)> {
    use std::os::fd::AsRawFd;
    buffer.clear();
    buffer.extend_from_slice(name.as_bytes());
    buffer.push(0);
    let name = std::ffi::CStr::from_bytes_with_nul(buffer)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name holds NUL"))?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    #[cfg(test)]
    submitted();
    // SAFETY: `dir` is an open descriptor, `name` is NUL-terminated and
    // `stat` is writable storage for one `stat`.
    if unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), stat.as_mut_ptr(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fstatat` succeeded, so it filled `stat`.
    let mode = unsafe { stat.assume_init() }.st_mode & libc::S_IFMT;
    Ok(match mode {
        libc::S_IFDIR => (EntryKind::Dir, Some(())),
        libc::S_IFREG => (EntryKind::File, None),
        _ => (EntryKind::Other, None),
    })
}

/// `openat` of the directory the last step found, by its name in `buffer`
/// (following a link, as its `fstatat` did): one component.
#[cfg(unix)]
fn enter(dir: &Base, (): Found, buffer: &[u8]) -> io::Result<Base> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = std::ffi::CStr::from_bytes_with_nul(buffer)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name holds NUL"))?;
    #[cfg(test)]
    submitted();
    let flags = libc::O_DIRECTORY | libc::O_CLOEXEC | dir_flags();
    // SAFETY: `dir` is an open descriptor and `name` is NUL-terminated;
    // `openat` returns a new descriptor or -1.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor nothing else owns.
    Ok(unsafe { Base::from_raw_fd(fd) })
}

#[cfg(windows)]
fn open_root(root: &Path) -> io::Result<Base> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, SYNCHRONIZE,
    };
    std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(root)
}

/// `NtCreateFile` of `name` relative to `dir` (following a link,
/// case-insensitive as Win32 opens are), read for its attributes only, then
/// its metadata from the handle: one component. The name is Tine's, as is:
/// no Win32 rewriting of a trailing `.` or space, nor of `..`.
#[cfg(windows)]
fn step_in(
    dir: &Base,
    name: &str,
    buffer: &mut Vec<u16>,
) -> io::Result<(EntryKind, Option<Found>)> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        NtCreateFile, FILE_OPEN, FILE_OPEN_FOR_BACKUP_INTENT, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        RtlNtStatusToDosError, HANDLE, OBJ_CASE_INSENSITIVE, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    buffer.clear();
    buffer.extend(name.encode_utf16());
    let bytes = u16::try_from(buffer.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidFilename, "a name is too long"))?;
    let object_name = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: buffer.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: dir.as_raw_handle() as HANDLE,
        ObjectName: &object_name,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    #[cfg(test)]
    submitted();
    // SAFETY: every pointer is to a live local; `object_name` borrows
    // `buffer`, untouched until the call returns.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        // SAFETY: a pure status-code translation.
        let code = unsafe { RtlNtStatusToDosError(status) };
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    // SAFETY: `NtCreateFile` succeeded, so `handle` is a fresh handle.
    let file = unsafe { Base::from_raw_handle(handle as _) };
    let kind = super::entry_kind(&file.metadata()?);
    Ok(match kind {
        EntryKind::Dir => (kind, Some(file)),
        _ => (kind, None),
    })
}

/// The directory the last step opened is the next base: nothing submitted.
#[cfg(windows)]
fn enter(_: &Base, found: Found, _: &[u16]) -> io::Result<Base> {
    Ok(found)
}

#[cfg(not(any(unix, windows)))]
fn open_root(_: &Path) -> io::Result<Base> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(any(unix, windows)))]
fn step_in(_: &Base, _: &str, _: &mut Vec<u8>) -> io::Result<(EntryKind, Option<Found>)> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(any(unix, windows)))]
fn enter(_: &Base, (): Found, _: &[u8]) -> io::Result<Base> {
    Err(io::ErrorKind::Unsupported.into())
}
