//! Process-local identity of the graph directory selected at open. Paths remain
//! the persisted identity; this witness only rejects a detached live binding.
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

impl crate::Store {
    /// Check the directory witness taken at open before reusing a retiring
    /// Store at the same path. Graph writes also check the witness at their
    /// existing guarded boundaries. One metadata probe on Unix; one identity
    /// open/query on Windows. No persisted identity or new path is exposed.
    pub fn check_graph_root(&self) -> Result<(), crate::IoError> {
        self.graph.root_binding.check().map_err(Into::into)
    }
}

#[derive(Debug)]
pub(crate) struct GraphRootMoved;

impl std::fmt::Display for GraphRootMoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GraphRootMoved: graph folder moved, replaced or disconnected; unsaved edits kept on this device; reopen the graph")
    }
}

impl std::error::Error for GraphRootMoved {}

pub(crate) fn moved() -> io::Error {
    io::Error::other(GraphRootMoved)
}

pub(crate) struct RootBinding {
    path: PathBuf,
    identity: io::Result<Identity>,
}

impl RootBinding {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            identity: identity(path),
        }
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        #[cfg(test)]
        CHECKS.with(|count| count.set(count.get() + 1));
        let expected = self.identity.as_ref().map_err(copy_error)?;
        match current(&self.path) {
            Ok(actual) if expected.matches(&actual) => Ok(()),
            Ok(_) => Err(moved()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Err(moved())
            }
            Err(error) => Err(error),
        }
    }
}

fn copy_error(error: &io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::NotFound {
        moved()
    } else {
        io::Error::new(error.kind(), error.to_string())
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static CHECKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
struct Identity {
    // Holding the directory prevents inode reuse while this binding exists.
    _directory: File,
    dev: u64,
    ino: u64,
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
fn identity(path: &Path) -> io::Result<Identity> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = directory.metadata()?;
    Ok(Identity {
        _directory: directory,
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
fn current(path: &Path) -> io::Result<fs::Metadata> {
    fs::metadata(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
impl Identity {
    fn matches(&self, actual: &fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        actual.is_dir() && self.dev == actual.dev() && self.ino == actual.ino()
    }
}

#[cfg(target_os = "windows")]
#[derive(PartialEq, Eq)]
struct Identity {
    volume: u64,
    id: [u8; 16],
}

#[cfg(target_os = "windows")]
fn identity(path: &Path) -> io::Result<Identity> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = FILE_ID_INFO::default();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        volume: info.VolumeSerialNumber,
        id: info.FileId.Identifier,
    })
}

#[cfg(target_os = "windows")]
fn current(path: &Path) -> io::Result<Identity> {
    identity(path)
}

#[cfg(target_os = "windows")]
impl Identity {
    fn matches(&self, actual: &Self) -> bool {
        self == actual
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_os = "windows"
)))]
compile_error!("graph root identity needs an explicit platform implementation");
