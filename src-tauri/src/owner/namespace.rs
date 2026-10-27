//! Where a graph's owner lock and command endpoint live (CLI PROTOCOL §1):
//! one directory per user account, derived from the account and never from
//! the environment, so a GUI and an SSH shell with different `XDG_*` or
//! `TMPDIR` agree on it. Not `/tmp`: a reaper deleting a held lock file would
//! let a newcomer lock a fresh inode, i.e. two owners.

use std::path::{Path, PathBuf};

use super::OwnerError;

/// The account's owner directory, checked for use.
pub(crate) struct Namespace {
    dir: PathBuf,
}

/// The paths of one graph's owner artifacts.
#[derive(Clone, Debug)]
pub(crate) struct GraphPaths {
    pub(crate) lock: PathBuf,
    pub(crate) socket: PathBuf,
    pub(crate) start_log: PathBuf,
}

impl Namespace {
    /// The account's directory, created `0700` when absent, then checked.
    pub(crate) fn resolve() -> Result<Self, OwnerError> {
        let dir = account_dir()?;
        create_private(&dir)?;
        check_private(&dir)?;
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(crate) fn at(dir: PathBuf) -> Result<Self, OwnerError> {
        create_private(&dir)?;
        check_private(&dir)?;
        Ok(Self { dir })
    }

    /// `root`'s artifacts, named by the fixed-width graph hash only.
    pub(crate) fn graph(&self, root: &Path) -> Result<GraphPaths, OwnerError> {
        let key = format!("{:016x}", crate::settings::graph_hash(root));
        self.key(&key)
    }

    /// Caller has validated the existing host graph id (L9-01 adoption).
    pub(crate) fn graph_id(&self, id: &str) -> Result<GraphPaths, OwnerError> {
        let key = id.rsplit_once('-').map(|(_, key)| key).unwrap_or("");
        if key.len() != 16 || !key.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(OwnerError::Unsafe("invalid draft owner key".into()));
        }
        self.key(key)
    }

    fn key(&self, key: &str) -> Result<GraphPaths, OwnerError> {
        // Recheck at use, including an already-resolved namespace.
        check_private(&self.dir)?;
        let paths = GraphPaths {
            lock: self.dir.join(format!("{key}.lock")),
            #[cfg(unix)]
            socket: self.dir.join(format!("{key}.sock")),
            #[cfg(windows)]
            socket: super::windows_security::pipe_name(key)?,
            start_log: self.dir.join(format!("{key}.start.log")),
        };
        Ok(paths)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn account_home() -> Result<PathBuf, OwnerError> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    // getpwuid_r, never $HOME: the namespace must not move with the environment.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_dir.is_null() {
        return Err(OwnerError::Unsafe(
            "the account has no home directory in the user database".into(),
        ));
    }
    let home = unsafe { CStr::from_ptr(entry.pw_dir) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(home.to_bytes())))
}

#[cfg(target_os = "linux")]
fn account_dir() -> Result<PathBuf, OwnerError> {
    Ok(account_home()?.join(".local/state/tine/owners"))
}

#[cfg(target_os = "macos")]
fn account_dir() -> Result<PathBuf, OwnerError> {
    Ok(account_home()?.join("Library/Application Support/tine-owners"))
}

/// Account-wide across stable/Beta, independently of environment variables
/// and single-instance forwarding.
#[cfg(target_os = "windows")]
fn account_dir() -> Result<PathBuf, OwnerError> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath};
    let mut raw = std::ptr::null_mut();
    // SAFETY: the OS allocates a NUL-terminated path for the current account;
    // it is copied before releasing the allocation with CoTaskMemFree.
    let status =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, std::ptr::null_mut(), &mut raw) };
    if status < 0 {
        return Err(OwnerError::Unsafe(format!(
            "cannot locate the account's LocalAppData: {status:#x}"
        )));
    }
    let path = unsafe {
        let mut len = 0;
        while *raw.add(len) != 0 {
            len += 1;
        }
        let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(raw, len));
        CoTaskMemFree(raw.cast());
        PathBuf::from(path)
    };
    Ok(path.join("tine-owners"))
}

/// Mobile: one process per app, with per-app sandboxed data, no CLI host.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn account_dir() -> Result<PathBuf, OwnerError> {
    Err(OwnerError::Unsupported)
}

/// Not a Tine platform.
#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "ios",
    target_os = "android"
)))]
fn account_dir() -> Result<PathBuf, OwnerError> {
    Err(OwnerError::Unsupported)
}

#[cfg(unix)]
fn create_private(dir: &Path) -> Result<(), OwnerError> {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(parent) = dir.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| OwnerError::Unsafe(format!("cannot create {}: {e}", parent.display())))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(OwnerError::Unsafe(format!(
            "cannot create {}: {e}",
            dir.display()
        ))),
    }
}

#[cfg(windows)]
fn create_private(dir: &Path) -> Result<(), OwnerError> {
    // The Known Folder is trusted. Create only our coordination directory,
    // outside stable's case-insensitive LocalAppData\Tine install directory.
    super::windows_security::create_directory(dir)
}

/// A directory, not a symlink, owned by this uid, with no group or other
/// permission. In-scope scenario: on a shared machine another account
/// pre-created or loosened it and could read or answer the agent's requests.
#[cfg(unix)]
fn check_private(dir: &Path) -> Result<(), OwnerError> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|e| OwnerError::Unsafe(format!("cannot inspect {}: {e}", dir.display())))?;
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(OwnerError::Unsafe(format!(
            "{} must be a private directory owned by you (mode 0700, not a symlink)",
            dir.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn check_private(dir: &Path) -> Result<(), OwnerError> {
    // Never repair a pre-existing DACL: a loosened directory may already
    // contain a lock file controlled by another account.
    super::windows_security::check_directory(dir)
}

/// A Unix socket path must fit `sockaddr_un.sun_path` (108 bytes on Linux,
/// 104 on macOS) with its terminating NUL.
#[cfg(unix)]
pub(crate) fn check_socket_path(socket: &Path) -> Result<(), OwnerError> {
    use std::os::unix::ffi::OsStrExt;
    let room = unsafe { std::mem::zeroed::<libc::sockaddr_un>() }
        .sun_path
        .len();
    if socket.as_os_str().as_bytes().len() >= room {
        return Err(OwnerError::Unsafe(format!(
            "the endpoint path {} is longer than the system allows ({} bytes)",
            socket.display(),
            room - 1
        )));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn check_socket_path(_: &Path) -> Result<(), OwnerError> {
    Ok(())
}

/// No automatic repair at discovery: the user explicitly authorizes tightening
/// this account's directory. Never replace/unlink a held lock inode. §5 trusts
/// same-account filesystem access; a symlink or foreign owner is still refused.
#[cfg(unix)]
pub(crate) fn repair_private(dir: &Path) -> Result<(), OwnerError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|e| {
            OwnerError::Unsafe(format!("cannot inspect {} for repair: {e}", dir.display()))
        })?;
    let meta = file
        .metadata()
        .map_err(|e| OwnerError::Unsafe(e.to_string()))?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::getuid() } {
        return Err(OwnerError::Unsafe(
            "only your own non-symlink coordination directory can be repaired".into(),
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(|e| OwnerError::Unsafe(format!("cannot repair {}: {e}", dir.display())))?;
    check_private(dir)
}

pub(crate) fn repair_available() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return account_dir()
            .ok()
            .and_then(|dir| std::fs::symlink_metadata(dir).ok())
            .is_some_and(|meta| {
                meta.is_dir() && meta.uid() == unsafe { libc::getuid() } && meta.mode() & 0o077 != 0
            });
    }
    #[cfg(windows)]
    {
        false
    }
}

pub(crate) fn repair() -> Result<(), OwnerError> {
    #[cfg(unix)]
    {
        repair_private(&account_dir()?)
    }
    #[cfg(windows)]
    {
        Err(OwnerError::Unsafe(
            "change the coordination folder permissions in Windows Security settings".into(),
        ))
    }
}

pub(crate) fn directory() -> Result<PathBuf, OwnerError> {
    account_dir()
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn explicit_permission_repair_keeps_the_held_inode_and_refuses_symlinks() {
        use super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let data = tempfile::tempdir().unwrap();
        let owners = data.path().join("owners");
        let ns = Namespace::at(owners.clone()).unwrap();
        let paths = ns.graph(Path::new("/graph")).unwrap();
        let held = crate::owner::lock::OwnerLock::try_take(&paths.lock, "serve")
            .unwrap()
            .unwrap();
        let inode = std::fs::metadata(&paths.lock).unwrap().ino();
        std::fs::set_permissions(&owners, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            Namespace::at(owners.clone()).is_err(),
            "repair requires an explicit action"
        );
        repair_private(&owners).unwrap();
        assert_eq!(std::fs::metadata(&owners).unwrap().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&paths.lock).unwrap().ino(), inode);
        assert!(crate::owner::lock::OwnerLock::try_take(&paths.lock, "app")
            .unwrap()
            .is_none());
        let link = data.path().join("link");
        std::os::unix::fs::symlink(&owners, &link).unwrap();
        assert!(repair_private(&link).is_err());
        assert!(std::fs::symlink_metadata(link)
            .unwrap()
            .file_type()
            .is_symlink());
        drop(held);
    }
    #[cfg(unix)]
    #[test]
    fn missing_state_parents_are_created_privately() {
        use std::os::unix::fs::PermissionsExt;
        let data = tempfile::tempdir().unwrap();
        let owners = data.path().join(".local/state/tine/owners");
        super::Namespace::at(owners.clone()).unwrap();
        for dir in [
            ".local",
            ".local/state",
            ".local/state/tine",
            ".local/state/tine/owners",
        ] {
            assert_eq!(
                std::fs::metadata(data.path().join(dir))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "{dir}"
            );
        }
    }
    #[test]
    fn windows_and_unix_enable_the_shared_owner_protocol() {
        let source = include_str!("serve.rs");
        let function = &source[source.find("fn serve_graph(").unwrap()..];
        let before_claim = &function[..function.find("lease::claim(").unwrap()];
        assert!(!before_claim.contains("OwnerError::Unsupported"));
        assert!(include_str!("windows.rs").contains("first_pipe_instance(first)"));
        assert!(!include_str!("client.rs").contains("Err(OwnerError::Unsupported)"));
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_owner_directory_rejects_files_and_reparse_points() {
        use super::*;
        let data = tempfile::tempdir().unwrap();
        let owners = data.path().join("tine-owners");
        create_private(&owners).unwrap();
        check_private(&owners).unwrap();
        let file = data.path().join("file");
        std::fs::write(&file, b"not a directory").unwrap();
        assert!(check_private(&file).is_err());
        let link = data.path().join("link");
        match std::os::windows::fs::symlink_dir(&owners, &link) {
            Ok(()) => assert!(check_private(&link).is_err()),
            Err(error) => {
                eprintln!("reparse runtime check unavailable without symlink privilege: {error}")
            }
        }
    }
    #[test]
    fn desktop_exclusion_names_all_five_targets() {
        let source = include_str!("namespace.rs");
        let production = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        for os in ["linux", "macos", "windows", "ios", "android"] {
            assert!(production.contains(&format!("target_os = \"{os}\"")));
        }
        let recovery = include_str!("../owner_protection.rs");
        let cfgs: Vec<_> = recovery
            .match_indices("fn recovery_instructions(")
            .map(|(at, _)| {
                let head = &recovery[..at];
                &head[head.rfind("#[cfg(").unwrap()..]
            })
            .collect();
        for os in ["linux", "macos", "windows", "ios", "android"] {
            assert_eq!(
                cfgs.iter()
                    .filter(|cfg| cfg.contains(&format!("target_os = \"{os}\"")))
                    .count(),
                1,
                "one recovery policy per shipped target: {os}"
            );
        }
        let start = production
            .find("#[cfg(target_os = \"windows\")]\nfn account_dir")
            .expect("Windows takes an account-wide owner lock, independently of single-instance");
        let arm = &production[start..];
        let arm = &arm[..arm.find("\n}\n").unwrap()];
        assert!(arm.contains("SHGetKnownFolderPath"));
        assert!(arm.contains("FOLDERID_LocalAppData"));
        assert!(arm.contains("join(\"tine-owners\")"));
        assert!(!arm.contains("std::env"));
        assert!(!arm.contains("Unsupported"));
        let mobile = production
            .find("#[cfg(any(target_os = \"ios\", target_os = \"android\"))]\nfn account_dir")
            .expect("mobile exclusion is one process per sandboxed app");
        assert!(production[mobile..].contains("Err(OwnerError::Unsupported)"));
    }
}
