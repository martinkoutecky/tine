//! Physical SQLite file-set naming, candidate cleanup, and publication.
//!
//! This module knows only filesystem mechanics. It does not decide whether a
//! candidate is semantically authorized, validate its SQLite contents, or
//! interpret the authenticated checkpoint stored beside it.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use cap_std::{ambient_authority, fs::Dir};
use uuid::Uuid;

use crate::filesystem::sync_dir_required;

/// The database and physical sidecars that form one disposable SQLite file set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteFileSet {
    database: PathBuf,
    wal: PathBuf,
    shm: PathBuf,
    checkpoint: PathBuf,
}

impl SqliteFileSet {
    /// Derive the exact SQLite WAL, SHM, and Tine checkpoint names for `database`.
    pub fn new(database: &Path) -> Self {
        Self {
            database: database.to_path_buf(),
            wal: appended_path(database, "-wal"),
            shm: appended_path(database, "-shm"),
            checkpoint: appended_path(database, "-auth"),
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.database
    }

    pub fn wal_path(&self) -> &Path {
        &self.wal
    }

    pub fn shm_path(&self) -> &Path {
        &self.shm
    }

    pub fn checkpoint_path(&self) -> &Path {
        &self.checkpoint
    }

    /// Database-first physical paths, in stable forensic-move order.
    pub fn paths(&self) -> [&Path; 4] {
        [&self.database, &self.wal, &self.shm, &self.checkpoint]
    }

    /// Preserve the prior path-based existence semantics, including I/O errors
    /// being treated as absence at this probe-only boundary.
    pub fn any_exists(&self) -> bool {
        self.paths().into_iter().any(Path::exists)
    }

    /// Remove every file in the set if present.
    ///
    /// `remove_file` unlinks a final-component symlink rather than following it,
    /// matching the candidate cleanup behavior this primitive replaces.
    pub fn remove(&self) -> Result<(), SqliteFileSetError> {
        for path in self.paths() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(SqliteFileSetError::Io(error)),
            }
        }
        Ok(())
    }

    /// Allocate and clean an unpublished candidate file set beside `target`.
    pub fn prepare_candidate(target: &Path) -> Result<Self, SqliteFileSetError> {
        let parent = target
            .parent()
            .ok_or_else(|| SqliteFileSetError::UnsafePath("database path has no parent".into()))?;
        let name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                SqliteFileSetError::UnsafePath("database file name is not UTF-8".into())
            })?;
        let candidate =
            Self::new(&parent.join(format!(".{name}.candidate-{}.sqlite", Uuid::new_v4())));
        candidate.remove()?;
        Ok(candidate)
    }

    /// Publish a fully checkpointed candidate at `target` and apply the shared
    /// platform directory-sync contract after the rename.
    ///
    /// Unix requires an explicit parent-directory sync. Windows currently has
    /// no equivalent implementation in the shared filesystem substrate, so the
    /// rename remains atomic but directory-entry crash durability is
    /// best-effort there. This preserves the predecessor's platform behavior.
    ///
    /// The candidate's WAL and SHM must already be gone. Its semantic
    /// checkpoint is deliberately discarded: core writes a new authenticated
    /// checkpoint only after reopening and validating the published database.
    pub fn publish_candidate(&self, target: &Path) -> Result<(), SqliteFileSetError> {
        self.publish_candidate_with_post_rename(target, || Ok(()))
    }

    fn publish_candidate_with_post_rename<F>(
        &self,
        target: &Path,
        post_rename: F,
    ) -> Result<(), SqliteFileSetError>
    where
        F: FnOnce() -> Result<(), SqliteFileSetError>,
    {
        if self.wal.exists() || self.shm.exists() {
            self.remove()?;
            return Err(SqliteFileSetError::CandidateRetainedSidecars);
        }
        match fs::remove_file(&self.checkpoint) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(SqliteFileSetError::Io(error)),
        }
        fs::rename(&self.database, target).map_err(SqliteFileSetError::Io)?;
        post_rename()?;
        let parent = target
            .parent()
            .ok_or_else(|| SqliteFileSetError::UnsafePath("database has no parent".into()))?;
        let directory =
            Dir::open_ambient_dir(parent, ambient_authority()).map_err(SqliteFileSetError::Io)?;
        sync_dir_required(&directory).map_err(SqliteFileSetError::Io)
    }
}

/// A failure at the physical SQLite file-set boundary.
#[derive(Debug)]
pub enum SqliteFileSetError {
    Io(io::Error),
    UnsafePath(String),
    CandidateRetainedSidecars,
}

impl fmt::Display for SqliteFileSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::UnsafePath(error) => error.fmt(formatter),
            Self::CandidateRetainedSidecars => {
                formatter.write_str("checkpointed SQLite candidate retained sidecars")
            }
        }
    }
}

impl std::error::Error for SqliteFileSetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::UnsafePath(_) | Self::CandidateRetainedSidecars => None,
        }
    }
}

fn appended_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tine-storage-sqlite-fileset-{label}-{}-{}",
                std::process::id(),
                Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sqlite_file_set_appends_stable_physical_names() {
        let files = SqliteFileSet::new(Path::new("workspace.frontier.sqlite"));
        assert_eq!(
            files.database_path(),
            Path::new("workspace.frontier.sqlite")
        );
        assert_eq!(files.wal_path(), Path::new("workspace.frontier.sqlite-wal"));
        assert_eq!(files.shm_path(), Path::new("workspace.frontier.sqlite-shm"));
        assert_eq!(
            files.checkpoint_path(),
            Path::new("workspace.frontier.sqlite-auth")
        );
    }

    #[test]
    fn interrupted_candidate_sidecars_are_removed_without_publication() {
        let directory = TestDirectory::new("interrupted-candidate");
        let target = directory.path().join("frontier.sqlite");
        fs::write(&target, b"prior projection").unwrap();
        let candidate = SqliteFileSet::prepare_candidate(&target).unwrap();
        for path in candidate.paths() {
            fs::write(path, b"interrupted candidate").unwrap();
        }

        assert!(matches!(
            candidate.publish_candidate(&target),
            Err(SqliteFileSetError::CandidateRetainedSidecars)
        ));
        assert_eq!(fs::read(&target).unwrap(), b"prior projection");
        assert!(!candidate.any_exists());
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_candidate_cleanup_unlinks_sidecars_without_following_them() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("nofollow-cleanup");
        let target = directory.path().join("frontier.sqlite");
        let candidate = SqliteFileSet::prepare_candidate(&target).unwrap();
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"must survive").unwrap();
        symlink(&sentinel, candidate.wal_path()).unwrap();

        candidate.remove().unwrap();

        assert_eq!(fs::read(&sentinel).unwrap(), b"must survive");
        assert!(!candidate.wal_path().exists());
    }

    #[test]
    fn crash_after_atomic_rename_exposes_the_complete_candidate() {
        let directory = TestDirectory::new("post-rename-crash");
        let target = directory.path().join("frontier.sqlite");
        let candidate = SqliteFileSet::prepare_candidate(&target).unwrap();
        let mut file = fs::File::create(candidate.database_path()).unwrap();
        file.write_all(b"complete candidate").unwrap();
        file.sync_all().unwrap();
        fs::write(candidate.checkpoint_path(), b"candidate checkpoint").unwrap();

        let result = candidate.publish_candidate_with_post_rename(&target, || {
            Err(SqliteFileSetError::Io(io::Error::other(
                "simulated process crash after rename",
            )))
        });

        assert!(matches!(result, Err(SqliteFileSetError::Io(_))));
        assert_eq!(fs::read(&target).unwrap(), b"complete candidate");
        assert!(!candidate.database_path().exists());
        assert!(!candidate.checkpoint_path().exists());
    }
}
