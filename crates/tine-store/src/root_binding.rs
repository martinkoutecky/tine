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

/// Unix stat identity. The device always counts; the inode only where the
/// filesystem's inode numbers survive eviction of the unpinned root inode; the
/// birth time only when both sides report one (Linux only).
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
struct Identity {
    dev: u64,
    ino: u64,
    birth: Option<std::time::SystemTime>,
    /// Decided once at open; unused on the probed side.
    stable_ino: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
fn identity(path: &Path) -> io::Result<Identity> {
    let mut identity = stat_identity(path)?;
    identity.stable_ino = stable_inode_numbers(path);
    Ok(identity)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
fn current(path: &Path) -> io::Result<Identity> {
    stat_identity(path)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
fn stat_identity(path: &Path) -> io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    // One metadata call per probe, without retaining a volume-pinning
    // descriptor. On Linux std uses statx (btime included) and falls back to
    // stat where a kernel or seccomp profile refuses statx.
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(moved());
    }
    Ok(Identity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        birth: compared_birth(&metadata),
        stable_ino: true,
    })
}

#[cfg(target_os = "linux")]
fn compared_birth(metadata: &fs::Metadata) -> Option<std::time::SystemTime> {
    metadata.created().ok()
}

// APFS/HFS+ move st_birthtime back when utimes sets an earlier mtime (cp -p,
// rsync -t onto the folder), and neither reuses inode numbers, so Apple
// compares device and inode only.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn compared_birth(_: &fs::Metadata) -> Option<std::time::SystemTime> {
    None
}

/// SMB/CIFS mounted noserverino (by the user, or by the kernel itself when the
/// server's numbers are unusable) and FUSE filesystems without use_ino hand out
/// a fresh inode number once the root's in-core inode is evicted, which nothing
/// pins in poll mode. There only the device and birth time are compared. A
/// failed statfs also counts as unstable: a weaker check, never a false refusal.
#[cfg(target_os = "linux")]
fn stable_inode_numbers(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    const UNSTABLE_INODE_FS: [u32; 4] = [0xFF53_4D42, 0xFE53_4D42, 0x517B, 0x6573_5546];
    let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(name.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    // f_type is c_long on most targets; the magic numbers are 32-bit.
    let kind = unsafe { stat.assume_init() }.f_type as u32;
    !UNSTABLE_INODE_FS.contains(&kind)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn stable_inode_numbers(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(name.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    let stat = unsafe { stat.assume_init() };
    let kind = unsafe { std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr()) }.to_bytes();
    !(matches!(kind, b"smbfs" | b"afpfs" | b"webdav")
        || kind.starts_with(b"macfuse")
        || kind.starts_with(b"osxfuse")
        || kind.starts_with(b"fuse"))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
impl Identity {
    fn matches(&self, actual: &Self) -> bool {
        self.dev == actual.dev
            && (!self.stable_ino || self.ino == actual.ino)
            && match (self.birth, actual.birth) {
                (Some(expected), Some(actual)) => expected == actual,
                _ => true,
            }
    }
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

#[cfg(target_os = "android")]
fn current(path: &Path) -> io::Result<Identity> {
    identity(path)
}

#[cfg(target_os = "android")]
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
        assert!(code.contains(
            "const UNSTABLE_INODE_FS: [u32; 4] = [0xFF53_4D42, 0xFE53_4D42, 0x517B, 0x6573_5546];"
        ));
        assert!(code.contains(
            "(Some(expected), Some(actual)) => expected == actual,\n                _ => true,"
        ));
        assert!(
            !code.contains("libc::statx("),
            "std metadata carries the statx fallback"
        );
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

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
    #[test]
    fn l9_stat_identity_ignores_unstable_inode_numbers_and_one_sided_birth() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let id = |dev, ino, birth, stable_ino| Identity {
            dev,
            ino,
            birth,
            stable_ino,
        };
        // SMB noserverino / FUSE: a re-issued inode number for the same folder.
        assert!(id(1, 10, Some(t), false).matches(&id(1, 11, Some(t), true)));
        assert!(id(1, 10, None, false).matches(&id(1, 11, None, true)));
        // ...while a different birth time or device is still a replacement.
        let later = t + std::time::Duration::from_secs(1);
        assert!(!id(1, 10, Some(t), false).matches(&id(1, 11, Some(later), true)));
        assert!(!id(1, 10, None, false).matches(&id(2, 10, None, true)));
        // Local filesystems: a new inode number is a replacement.
        assert!(!id(1, 10, Some(t), true).matches(&id(1, 11, Some(t), true)));
        // A birth time reported on one side only (CIFS createtime 0) is unknown.
        assert!(id(1, 10, Some(t), true).matches(&id(1, 10, None, true)));
        assert!(id(1, 10, None, true).matches(&id(1, 10, Some(t), true)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn l9_local_tempdir_has_stable_inode_numbers() {
        let dir = tempfile::tempdir().unwrap();
        assert!(stable_inode_numbers(dir.path()));
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
