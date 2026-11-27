//! The one answer to "may this path be opened as a regular file".
//!
//! Threat scenario (sync delivery, external editors, imported content): a
//! graph file, an asset or a caller-selected device path can name a FIFO,
//! socket or device node. A blocking `open` of a FIFO with no writer hangs the
//! caller before any later check runs, and opening a device node can have side
//! effects. Path metadata refuses a non-regular file before the open; the open
//! is nonblocking on Unix, so a path swapped for a FIFO after that check cannot
//! hang either; the opened handle's metadata rechecks the race. `O_NONBLOCK`
//! has no effect on reads of a regular file.
//!
//! Cost: one path `stat` (or `lstat`), one `open`, one `fstat`.

use std::fs::{self, File};
use std::io;
use std::path::Path;

/// What the opener does when the path's final component is a symlink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalSymlink {
    /// Open the symlink's target (it must itself be a regular file).
    Follow,
    /// Refuse it as [`OpenRegularError::FinalSymlink`]; on Unix the open also
    /// carries `O_NOFOLLOW`, so a symlink swapped in after the check is refused.
    Refuse,
}

/// Why [`open_regular_file`] refused a path.
#[derive(Debug)]
pub enum OpenRegularError {
    /// The path's metadata or the open itself failed.
    Io(io::Error),
    /// The path names a directory, FIFO, socket or device node.
    NotRegular,
    /// The final component is a symlink and the caller asked to refuse it.
    FinalSymlink,
}

impl From<io::Error> for OpenRegularError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<OpenRegularError> for io::Error {
    fn from(error: OpenRegularError) -> Self {
        match error {
            OpenRegularError::Io(error) => error,
            OpenRegularError::NotRegular => {
                io::Error::new(io::ErrorKind::InvalidInput, "not a file")
            }
            OpenRegularError::FinalSymlink => {
                io::Error::new(io::ErrorKind::InvalidInput, "final symlink refused")
            }
        }
    }
}

/// Open `path` for reading only if it is a regular file; returns the handle
/// and its metadata (from the handle, so callers need no second `fstat`).
pub fn open_regular_file(
    path: &Path,
    final_symlink: FinalSymlink,
) -> Result<(File, fs::Metadata), OpenRegularError> {
    let meta = match final_symlink {
        FinalSymlink::Follow => fs::metadata(path)?,
        FinalSymlink::Refuse => fs::symlink_metadata(path)?,
    };
    if meta.file_type().is_symlink() {
        return Err(OpenRegularError::FinalSymlink);
    }
    if !meta.is_file() {
        return Err(OpenRegularError::NotRegular);
    }
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = match final_symlink {
            FinalSymlink::Follow => 0,
            FinalSymlink::Refuse => libc::O_NOFOLLOW,
        };
        fs::File::options()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | nofollow)
            .open(path)?
    };
    #[cfg(not(unix))]
    let file = fs::File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(OpenRegularError::NotRegular);
    }
    Ok((file, meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn a_regular_file_opens_under_either_policy() {
        let dir = crate::test_support::fresh_dir("tine-regular-file-");
        let path = dir.join("a.txt");
        fs::write(&path, b"bytes").unwrap();
        for policy in [FinalSymlink::Follow, FinalSymlink::Refuse] {
            let mut text = String::new();
            open_regular_file(&path, policy)
                .unwrap()
                .0
                .read_to_string(&mut text)
                .unwrap();
            assert_eq!(text, "bytes");
        }
        assert!(matches!(
            open_regular_file(&dir, FinalSymlink::Follow),
            Err(OpenRegularError::NotRegular)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_final_symlink_follows_or_is_refused_as_asked() {
        let dir = crate::test_support::fresh_dir("tine-regular-file-link-");
        fs::write(dir.join("target.txt"), b"bytes").unwrap();
        std::os::unix::fs::symlink(dir.join("target.txt"), dir.join("link.txt")).unwrap();
        assert!(open_regular_file(&dir.join("link.txt"), FinalSymlink::Follow).is_ok());
        assert!(matches!(
            open_regular_file(&dir.join("link.txt"), FinalSymlink::Refuse),
            Err(OpenRegularError::FinalSymlink)
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
