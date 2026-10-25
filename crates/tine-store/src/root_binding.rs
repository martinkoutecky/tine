//! Process-local identity of the graph directory selected at open. Paths remain
//! the persisted identity; this witness only rejects a detached live binding.
use std::fs;
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

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
#[derive(PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    birth: Option<std::time::SystemTime>,
}

#[cfg(target_os = "linux")]
fn identity(path: &Path) -> io::Result<Identity> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut stat = std::mem::MaybeUninit::<libc::statx>::uninit();
    // One statx obtains dev/ino and btime without retaining a volume-pinning
    // descriptor. Old kernels/filesystems may not expose birth time.
    let result = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            name.as_ptr(),
            libc::AT_STATX_SYNC_AS_STAT,
            libc::STATX_BASIC_STATS | libc::STATX_BTIME,
            stat.as_mut_ptr(),
        )
    };
    if result == 0 {
        let stat = unsafe { stat.assume_init() };
        if u32::from(stat.stx_mode) & libc::S_IFMT != libc::S_IFDIR {
            return Err(moved());
        }
        let birth = (stat.stx_mask & libc::STATX_BTIME != 0)
            .then(|| {
                let seconds = stat.stx_btime.tv_sec;
                let whole = std::time::Duration::from_secs(seconds.unsigned_abs());
                let time = if seconds < 0 {
                    std::time::UNIX_EPOCH.checked_sub(whole)
                } else {
                    std::time::UNIX_EPOCH.checked_add(whole)
                };
                time.and_then(|time| {
                    time.checked_add(std::time::Duration::from_nanos(u64::from(
                        stat.stx_btime.tv_nsec,
                    )))
                })
            })
            .flatten();
        return Ok(Identity {
            dev: libc::makedev(stat.stx_dev_major, stat.stx_dev_minor),
            ino: stat.stx_ino,
            birth,
        });
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        return Err(error);
    }
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(moved());
    }
    Ok(Identity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        birth: None,
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn identity(path: &Path) -> io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(moved());
    }
    Ok(Identity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        // Apple's Metadata::created reads st_birthtime from this same stat.
        birth: metadata.created().ok(),
    })
}

// Android shared-storage FUSE inode stability has not been device-verified.
// Use the approved vanish-only fallback rather than rejecting valid saves.
#[cfg(target_os = "android")]
#[derive(PartialEq, Eq)]
struct Identity;

#[cfg(target_os = "android")]
fn identity(path: &Path) -> io::Result<Identity> {
    if fs::metadata(path)?.is_dir() {
        Ok(Identity)
    } else {
        Err(moved())
    }
}

#[cfg(not(target_os = "windows"))]
fn current(path: &Path) -> io::Result<Identity> {
    identity(path)
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
impl Identity {
    fn matches(&self, actual: &Self) -> bool {
        self == actual
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "android")]
    #[test]
    fn l9_android_fallback_checks_presence_without_comparing_fuse_inodes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("graph");
        fs::create_dir(&root).unwrap();
        let binding = RootBinding::new(&root);
        fs::rename(&root, dir.path().join("moved")).unwrap();
        assert!(binding.check().is_err());
        fs::create_dir(&root).unwrap();
        binding.check().unwrap();
    }

    #[test]
    fn l9_platform_policy_pins_android_vanish_only_and_stat_identity() {
        let source = include_str!("root_binding.rs");
        let code = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        assert!(code.contains(
            "#[cfg(target_os = \"android\")]\n#[derive(PartialEq, Eq)]\nstruct Identity;"
        ));
        assert!(code.contains("libc::STATX_BASIC_STATS | libc::STATX_BTIME"));
        assert!(code.contains("birth: metadata.created().ok()"));
        assert!(!code.contains("_directory:"));
        for target in ["linux", "macos", "ios", "android", "windows"] {
            assert!(code.contains(&format!("target_os = \"{target}\"")));
        }
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn l9_stat_binding_rejects_replacement_and_a_non_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("graph");
        fs::create_dir(&root).unwrap();
        let binding = RootBinding::new(&root);
        fs::rename(&root, dir.path().join("moved")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(binding
            .check()
            .unwrap_err()
            .to_string()
            .contains("GraphRootMoved"));
        fs::remove_dir(&root).unwrap();
        fs::write(&root, b"replacement file").unwrap();
        assert!(binding.check().is_err());
    }

    #[test]
    #[ignore = "platform micro-measurement; run explicitly with --ignored --nocapture"]
    fn l9_measure_per_save_root_checks() {
        let dir = tempfile::tempdir().unwrap();
        let binding = RootBinding::new(dir.path());
        binding.check().unwrap();
        const N: u32 = 100_000;
        let began = std::time::Instant::now();
        for _ in 0..N {
            std::hint::black_box(binding.check()).unwrap();
        }
        let elapsed = began.elapsed();
        eprintln!(
            "L9 root checks platform={} n={N} total={elapsed:?} ns/check={} six-check-save={:?}",
            std::env::consts::OS,
            elapsed.as_nanos() / u128::from(N),
            elapsed / N * 6
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn l9_root_binding_holds_no_directory_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let binding = RootBinding::new(dir.path());
        binding.check().unwrap();
        let held = fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target == dir.path()));
        assert!(!held, "a live binding must permit volume eject/unmount");
        binding.check().unwrap();
    }
}
