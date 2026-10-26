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
        let paths = GraphPaths {
            lock: self.dir.join(format!("{key}.lock")),
            socket: self.dir.join(format!("{key}.sock")),
            start_log: self.dir.join(format!("{key}.start.log")),
        };
        check_socket_path(&paths.socket)?;
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

/// Windows: the endpoint (a named pipe with a user-only DACL and a checked
/// directory DACL) is not built yet; the command line refuses there rather
/// than run a second owner, and the app opens without an endpoint (CLI
/// PROTOCOL §1, slice 1a). iOS and Android have no command line.
#[cfg(any(target_os = "windows", target_os = "ios", target_os = "android"))]
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
        std::fs::create_dir_all(parent)
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
fn create_private(_: &Path) -> Result<(), OwnerError> {
    Err(OwnerError::Unsupported)
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
fn check_private(_: &Path) -> Result<(), OwnerError> {
    Err(OwnerError::Unsupported)
}

/// A Unix socket path must fit `sockaddr_un.sun_path` (108 bytes on Linux,
/// 104 on macOS) with its terminating NUL.
#[cfg(unix)]
fn check_socket_path(socket: &Path) -> Result<(), OwnerError> {
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
fn check_socket_path(_: &Path) -> Result<(), OwnerError> {
    Ok(())
}
