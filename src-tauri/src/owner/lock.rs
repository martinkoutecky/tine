//! The owner lock (CLI PROTOCOL §3.3): an advisory exclusive lock on
//! `<owners>/<hash>.lock`, held for as long as a process owns the graph's
//! page host. The OS releases it when the process dies, so a crash leaves no
//! stale owner. The file is never unlinked or recreated (a contender may
//! hold an open handle to it); its content is diagnostic only.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::OwnerError;

/// A held owner lock. Dropping it (closing the handle) releases it.
pub(crate) struct OwnerLock {
    file: File,
    path: PathBuf,
}

impl std::fmt::Debug for OwnerLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerLock")
            .field("path", &self.path)
            .finish()
    }
}

impl OwnerLock {
    /// Take the lock: `Ok(None)` when another owner holds it. Any other
    /// failure is not contention (`owner-lock`, §3.5).
    pub(crate) fn try_take(path: &Path, owner: &str) -> Result<Option<Self>, OwnerError> {
        for _ in 0..2 {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
                .map_err(|e| OwnerError::Lock(format!("cannot open {}: {e}", path.display())))?;
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(e)) => {
                    return Err(OwnerError::Lock(format!(
                        "cannot lock {}: {e}",
                        path.display()
                    )))
                }
            }
            // The path still names the inode we locked (it was not replaced
            // between open and lock); else retry once.
            let same = same_file::Handle::from_file(file.try_clone().map_err(lock_io)?)
                .ok()
                .zip(same_file::Handle::from_path(path).ok())
                .is_some_and(|(held, named)| held == named);
            if !same {
                continue;
            }
            let mut lock = Self {
                file,
                path: path.to_owned(),
            };
            lock.describe(owner);
            return Ok(Some(lock));
        }
        Err(OwnerError::Lock(format!(
            "{} keeps being replaced while it is locked",
            path.display()
        )))
    }

    /// Diagnostic content: pid, owner kind, version. Nobody trusts it.
    fn describe(&mut self, owner: &str) {
        let _ = self.file.set_len(0);
        let _ = writeln!(
            self.file,
            "pid {}\nowner {owner}\nversion {}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        );
    }
}

fn lock_io(e: std::io::Error) -> OwnerError {
    OwnerError::Lock(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_at_a_time_and_release_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.lock");
        let first = OwnerLock::try_take(&path, "serve").unwrap().expect("free");
        assert!(OwnerLock::try_take(&path, "app").unwrap().is_none(), "held");
        drop(first);
        assert!(
            OwnerLock::try_take(&path, "app").unwrap().is_some(),
            "released"
        );
        assert!(path.exists(), "the lock file is never unlinked");
    }
}
