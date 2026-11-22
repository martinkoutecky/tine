//! Process-local identity of the graph directory selected at open. Paths remain
//! the persisted identity; this witness only rejects a detached live binding.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Process-local directory witness for application writers of repository
/// metadata. Uses the same root identity and target-containment policy as graph
/// writes. Checks remain sequential: a check-to-syscall race is not eliminated.
pub struct BoundDirectory {
    root: PathBuf,
    binding: RootBinding,
}

impl BoundDirectory {
    /// Capture an existing directory without retaining a directory descriptor.
    pub fn new(root: &Path) -> io::Result<Self> {
        // Preserve an absolute caller spelling (libgit2 may use a non-verbatim
        // Windows path); the shared containment check canonicalizes both sides.
        let root = if root.is_absolute() {
            root.to_path_buf()
        } else {
            fs::canonicalize(root)?
        };
        let binding = RootBinding::new(&root);
        binding.check()?;
        Ok(Self { root, binding })
    }

    /// The absolute directory spelling used for its mutation targets.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Recheck the captured directory identity.
    pub fn check(&self) -> io::Result<()> {
        self.binding.check()
    }

    /// Recheck root identity and the target's lexical and resolved containment.
    pub fn check_path(&self, path: &Path) -> io::Result<()> {
        crate::model::checked_write_target(
            &self.root,
            path,
            "repository metadata",
            Some(&self.binding),
        )
    }
}

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

    /// A graph held in memory: no root on disk is probed, and a check
    /// refuses (nothing writes such a graph).
    pub(crate) fn unbound(path: &Path) -> Self {
        Self {
            path: path.into(),
            identity: Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a graph held in memory has no root on disk",
            )),
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
    let mut identity = Identity::stat(path)?;
    identity.stable_ino = Identity::stable_inode_numbers(path);
    Ok(identity)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
fn current(path: &Path) -> io::Result<Identity> {
    Identity::stat(path)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
impl Identity {
    fn stat(path: &Path) -> io::Result<Identity> {
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
            birth: Self::compared_birth(&metadata),
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
// wasm32 (the browser build) binds no root: it has no file system, so every
// check there fails as a vanished root.
#[cfg(any(target_os = "android", target_arch = "wasm32"))]
#[derive(PartialEq, Eq)]
struct Identity;

#[cfg(any(target_os = "android", target_arch = "wasm32"))]
fn identity(path: &Path) -> io::Result<Identity> {
    if fs::metadata(path)?.is_dir() {
        Ok(Identity)
    } else {
        Err(moved())
    }
}

#[cfg(any(target_os = "android", target_arch = "wasm32"))]
fn current(path: &Path) -> io::Result<Identity> {
    identity(path)
}

#[cfg(any(target_os = "android", target_arch = "wasm32"))]
impl Identity {
    fn matches(&self, actual: &Self) -> bool {
        self == actual
    }
}

/// Windows directory identity, decided by what the file system answers. NTFS and
/// ReFS give the 128-bit FileIdInfo id. FAT32/exFAT, Samba shares and WSL's
/// `\\wsl.localhost` share refuse FileIdInfo (GH #973). There the older call's
/// volume serial counts, but its 64-bit file index does not: nothing pins the
/// root, and as with SMB/FUSE inode numbers on Linux there is no evidence the
/// index survives eviction on those file systems. Where neither call answers, the
/// check is vanish-only, as on Android. A weaker check, never a false refusal.
#[cfg(target_os = "windows")]
#[derive(Debug, PartialEq, Eq)]
struct Identity(IdentityKind);

#[cfg(target_os = "windows")]
#[derive(Debug, PartialEq, Eq)]
enum IdentityKind {
    FileId { volume: u64, id: [u8; 16] },
    Volume(u32),
    Present,
}

#[cfg(all(test, target_os = "windows"))]
thread_local! {
    /// Test seam: answer the next identity queries as a file system without
    /// FileIdInfo (1) or without either call (2) would.
    pub(crate) static NO_FILE_ID: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// The answer of a file system that lacks an information class: not supported,
/// invalid parameter, invalid function (FAT, SMB redirectors, 9P).
#[cfg(target_os = "windows")]
fn unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(1 | 50 | 87))
}

#[cfg(target_os = "windows")]
fn identity(path: &Path) -> io::Result<Identity> {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileIdInfo, GetFileAttributesExW, GetFileExInfoStandard,
        GetFileInformationByHandle, GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_DIRECTORY, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        WIN32_FILE_ATTRIBUTE_DATA,
    };
    #[cfg(test)]
    let forced = NO_FILE_ID.with(|forced| forced.get());
    #[cfg(not(test))]
    let forced = 0u8;
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = FILE_ID_INFO::default();
    let ok = forced == 0
        && unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } != 0;
    if ok {
        return Ok(Identity(IdentityKind::FileId {
            volume: info.VolumeSerialNumber,
            id: info.FileId.Identifier,
        }));
    }
    let error = if forced == 0 {
        io::Error::last_os_error()
    } else {
        io::Error::from_raw_os_error(50)
    };
    if !unsupported(&error) {
        return Err(error);
    }
    // The id above already tells a file from a folder; the older answers do
    // not, so each fallback reads the directory attribute itself.
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if forced < 2 && unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0 {
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(moved());
        }
        return Ok(Identity(IdentityKind::Volume(info.dwVolumeSerialNumber)));
    }
    let error = if forced < 2 {
        io::Error::last_os_error()
    } else {
        io::Error::from_raw_os_error(50)
    };
    if !unsupported(&error) {
        return Err(error);
    }
    // Neither identity query answers. `File::metadata` makes the same older
    // call, so the folder check asks for the basic attributes, then by path
    // (`GetFileAttributesW` fails on a share root; the Ex form does not). If
    // nothing answers, the open above is the only evidence: a weaker check,
    // never a refusal.
    let mut basic = FILE_BASIC_INFO::default();
    let attributes = if forced < 3
        && unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        } != 0
    {
        basic.FileAttributes
    } else {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
        let ok = unsafe {
            GetFileAttributesExW(
                wide.as_ptr(),
                GetFileExInfoStandard,
                (&mut data as *mut WIN32_FILE_ATTRIBUTE_DATA).cast(),
            )
        } != 0;
        if !ok {
            let error = io::Error::last_os_error();
            return if unsupported(&error) || error.raw_os_error() == Some(53) {
                Ok(Identity(IdentityKind::Present))
            } else {
                Err(error)
            };
        }
        data.dwFileAttributes
    };
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(moved());
    }
    Ok(Identity(IdentityKind::Present))
}

#[cfg(target_os = "windows")]
fn current(path: &Path) -> io::Result<Identity> {
    identity(path)
}

/// Each recheck takes the same branch as the open did on the same file system, so
/// plain equality compares like with like; a different answer means a different
/// file system or object at the path.
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
    target_os = "windows",
    target_arch = "wasm32"
)))]
compile_error!("graph root identity needs an explicit platform implementation");

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
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

    /// GH #973: a file system without FileIdInfo (WSL's share, FAT32/exFAT,
    /// Samba) must not refuse every save; the fallbacks keep the moved check.
    #[test]
    fn gh973_windows_identity_falls_back_where_file_ids_are_unsupported() {
        let source = include_str!("root_binding.rs");
        let code = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        let windows = &code[code
            .find("enum IdentityKind {\n    FileId")
            .expect("Windows identity has a file-id-free mode")..];
        assert!(windows.contains("GetFileInformationByHandle(file.as_raw_handle()"));
        assert!(windows.contains("Ok(Identity(IdentityKind::Present))"));
        assert!(windows.contains("Some(1 | 50 | 87)"));
        // `File::metadata` is GetFileInformationByHandle underneath: using it
        // would make the last fallback unreachable (merge review, 11/18).
        let arm = &windows[..windows.find("fn current(").unwrap()];
        assert!(!arm.contains(".metadata()"));
        // `GetFileAttributesW` fails on a share root (`\\server\share`).
        assert!(!arm.contains("GetFileAttributesW("));
        assert!(arm.contains("FileBasicInfo,"));
        assert!(arm.contains("GetFileAttributesExW("));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn gh973_binding_without_file_ids_saves_and_still_sees_a_moved_folder() {
        for mode in [1u8, 2, 3] {
            NO_FILE_ID.with(|forced| forced.set(mode));
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("graph");
            fs::create_dir(&root).unwrap();
            let binding = RootBinding::new(&root);
            binding
                .check()
                .unwrap_or_else(|e| panic!("mode {mode}: {e}"));
            fs::rename(&root, dir.path().join("moved")).unwrap();
            assert!(binding.check().is_err(), "mode {mode}: vanished folder");
            fs::write(&root, b"replacement file").unwrap();
            assert!(binding.check().is_err(), "mode {mode}: file in its place");
            fs::remove_file(&root).unwrap();
            fs::create_dir(&root).unwrap();
            binding
                .check()
                .unwrap_or_else(|e| panic!("mode {mode}: recreated: {e}"));
            NO_FILE_ID.with(|forced| forced.set(0));
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
        assert!(Identity::stable_inode_numbers(dir.path()));
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
