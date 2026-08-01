//! Physical SQLite file-set naming, candidate cleanup, and publication.
//!
//! This module knows only filesystem mechanics. It does not decide whether a
//! candidate is semantically authorized, validate its SQLite contents, or
//! interpret the authenticated checkpoint stored beside it.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use cap_std::{ambient_authority, fs::Dir};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::filesystem::sync_dir_required;
use crate::ContentDigest;

/// Bytes authenticated independently at each physical SQLite file edge.
pub const SQLITE_CHECKPOINT_EDGE_BYTES: usize = 64 * 1024;
/// Maximum authenticated checkpoint-envelope bytes admitted for publication.
pub const MAX_SQLITE_CHECKPOINT_BYTES: usize = 64 * 1024;
/// Maximum total bytes read from deterministic interior sample ranges.
///
/// This is a bounded accidental-corruption tripwire, not byte-complete
/// authentication. Core retains the semantic comparison authority fence.
pub const SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES: usize = 1024 * 1024;
/// Bytes read from each range when a file interior exceeds the total budget.
pub const SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES: usize = 16 * 1024;
const SQLITE_CHECKPOINT_INTERIOR_MAX_RANGES: usize =
    SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES / SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES;
const SQLITE_CHECKPOINT_TEMP_ATTEMPTS: usize = 8;
const SQLITE_FORENSIC_NAMES: [&str; 4] = ["database", "wal", "shm", "auth"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointPublicationStage {
    FileSynced,
    Replaced,
    ParentSyncApplied,
}

/// Bounded physical identity of one SQLite database or WAL file.
///
/// Field order and serialization are part of core's existing authenticated
/// projection-checkpoint format. Storage computes these physical values, while
/// core remains responsible for serializing and authenticating the envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalFileCheckpoint {
    pub length: u64,
    pub first_chunk_digest: ContentDigest,
    pub last_chunk_digest: ContentDigest,
    pub interior_sample_digest: ContentDigest,
}

/// Physical checkpoint material for a SQLite database and its optional WAL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalSqliteCheckpoint {
    pub database: PhysicalFileCheckpoint,
    pub wal: Option<PhysicalFileCheckpoint>,
}

/// Exact source-to-preserved path mapping for one physical SQLite file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteForensicPathMapping {
    pub original_path: PathBuf,
    pub preserved_path: PathBuf,
}

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

    /// Sample the database and optional WAL using the bounded physical
    /// checkpoint algorithm used by the authenticated core envelope.
    pub fn physical_checkpoint(&self) -> Result<PhysicalSqliteCheckpoint, SqliteFileSetError> {
        Ok(PhysicalSqliteCheckpoint {
            database: physical_file_checkpoint(&self.database)?,
            wal: optional_physical_file_checkpoint(&self.wal)?,
        })
    }

    /// Durably publish already-authenticated checkpoint-envelope bytes.
    ///
    /// Core owns the byte construction and authentication. Storage admits only
    /// the bounded envelope, creates a no-follow temporary beside the stable
    /// checkpoint name, synchronizes its contents, atomically replaces the
    /// predecessor, and applies the shared parent-directory sync contract.
    pub fn publish_checkpoint(&self, bytes: &[u8]) -> Result<(), SqliteFileSetError> {
        self.publish_checkpoint_with(
            bytes,
            |name, _attempt| format!(".{name}.tmp-{}", Uuid::new_v4()),
            |_stage| Ok(()),
        )
    }

    fn publish_checkpoint_with<N, F>(
        &self,
        bytes: &[u8],
        mut temporary_name: N,
        mut post_stage: F,
    ) -> Result<(), SqliteFileSetError>
    where
        N: FnMut(&str, usize) -> String,
        F: FnMut(CheckpointPublicationStage) -> Result<(), SqliteFileSetError>,
    {
        if bytes.len() > MAX_SQLITE_CHECKPOINT_BYTES {
            return Err(SqliteFileSetError::CheckpointTooLarge {
                length: bytes.len(),
                limit: MAX_SQLITE_CHECKPOINT_BYTES,
            });
        }
        let parent = self
            .checkpoint
            .parent()
            .ok_or_else(|| SqliteFileSetError::UnsafePath("checkpoint has no parent".into()))?;
        let name = self
            .checkpoint
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SqliteFileSetError::UnsafePath("checkpoint name is not UTF-8".into()))?;
        let mut last_collision = None;
        for attempt in 0..SQLITE_CHECKPOINT_TEMP_ATTEMPTS {
            let temporary = parent.join(temporary_name(name, attempt));
            // Atomic create-new is the predecessor's cross-platform no-follow
            // final-component creation primitive: an existing file or symlink
            // is a collision and is never opened.
            let mut file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    last_collision = Some(error);
                    continue;
                }
                Err(error) => return Err(SqliteFileSetError::Io(error)),
            };
            let result = (|| {
                file.write_all(bytes).map_err(SqliteFileSetError::Io)?;
                file.sync_all().map_err(SqliteFileSetError::Io)?;
                post_stage(CheckpointPublicationStage::FileSynced)?;
                drop(file);
                // Keep std::fs::rename deliberately: its atomic predecessor-
                // replacement semantics are the contract being extracted.
                fs::rename(&temporary, &self.checkpoint).map_err(SqliteFileSetError::Io)?;
                post_stage(CheckpointPublicationStage::Replaced)?;
                let directory = Dir::open_ambient_dir(parent, ambient_authority())
                    .map_err(SqliteFileSetError::Io)?;
                sync_dir_required(&directory).map_err(SqliteFileSetError::Io)?;
                post_stage(CheckpointPublicationStage::ParentSyncApplied)
            })();
            let cleanup = fs::remove_file(&temporary);
            if let Err(error) = result {
                let _ = cleanup;
                return Err(error);
            }
            if cleanup
                .as_ref()
                .is_err_and(|error| error.kind() != io::ErrorKind::NotFound)
            {
                cleanup.map_err(SqliteFileSetError::Io)?;
            }
            return Ok(());
        }
        Err(SqliteFileSetError::Io(last_collision.unwrap_or_else(
            || {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "checkpoint temporary name collision",
                )
            },
        )))
    }

    /// Database-first physical paths, in stable forensic-move order.
    pub fn paths(&self) -> [&Path; 4] {
        [&self.database, &self.wal, &self.shm, &self.checkpoint]
    }

    /// Move every present regular file into a newly allocated forensic
    /// directory, in stable database/WAL/SHM/checkpoint order.
    ///
    /// The callback runs after each rename and both required directory-sync
    /// operations. Core uses it only to retain its crash-test orchestration.
    pub fn preserve_forensic_files<F>(
        &self,
        directory: &Path,
        mut after_move: F,
    ) -> Result<Vec<SqliteForensicPathMapping>, SqliteFileSetError>
    where
        F: FnMut(usize),
    {
        let parent = self
            .database
            .parent()
            .ok_or_else(|| SqliteFileSetError::UnsafePath("database path has no parent".into()))?;
        let mut preserved = Vec::new();
        for mapping in self.forensic_path_mappings(directory) {
            match fs::symlink_metadata(&mapping.original_path) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(SqliteFileSetError::UnsafePath(format!(
                            "projection evidence {} is not a regular file",
                            mapping.original_path.display()
                        )));
                    }
                    fs::rename(&mapping.original_path, &mapping.preserved_path)
                        .map_err(SqliteFileSetError::Io)?;
                    sync_directory(directory)?;
                    sync_directory(parent)?;
                    preserved.push(mapping);
                    after_move(preserved.len());
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(SqliteFileSetError::Io(error)),
            }
        }
        Ok(preserved)
    }

    /// Resume or observe one previously allocated forensic directory.
    ///
    /// Core interprets the evidence-completion marker and supplies its state.
    /// Before completion, an original and its preserved name may not coexist;
    /// present originals are validated and moved. After completion, this is an
    /// observation only and reports whichever preserved names exist.
    pub fn resume_forensic_files(
        &self,
        directory: &Path,
        evidence_complete: bool,
    ) -> Result<Vec<SqliteForensicPathMapping>, SqliteFileSetError> {
        let parent = self
            .database
            .parent()
            .ok_or_else(|| SqliteFileSetError::UnsafePath("database path has no parent".into()))?;
        let mut preserved = Vec::new();
        for mapping in self.forensic_path_mappings(directory) {
            let original_exists = mapping.original_path.exists();
            let preserved_exists = mapping.preserved_path.exists();
            if !evidence_complete && original_exists && preserved_exists {
                return Err(SqliteFileSetError::Corrupt(format!(
                    "forensic recovery found both {} and {}",
                    mapping.original_path.display(),
                    mapping.preserved_path.display()
                )));
            }
            if !evidence_complete && original_exists {
                let metadata =
                    fs::symlink_metadata(&mapping.original_path).map_err(SqliteFileSetError::Io)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SqliteFileSetError::UnsafePath(format!(
                        "projection evidence {} is not a regular file",
                        mapping.original_path.display()
                    )));
                }
                fs::rename(&mapping.original_path, &mapping.preserved_path)
                    .map_err(SqliteFileSetError::Io)?;
                sync_directory(directory)?;
                sync_directory(parent)?;
            }
            if mapping.preserved_path.exists() {
                preserved.push(mapping);
            }
        }
        Ok(preserved)
    }

    fn forensic_path_mappings(&self, directory: &Path) -> [SqliteForensicPathMapping; 4] {
        let paths = self.paths();
        std::array::from_fn(|index| SqliteForensicPathMapping {
            original_path: paths[index].to_path_buf(),
            preserved_path: directory.join(SQLITE_FORENSIC_NAMES[index]),
        })
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
    Corrupt(String),
    CheckpointTooLarge { length: usize, limit: usize },
    CandidateRetainedSidecars,
}

impl fmt::Display for SqliteFileSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::UnsafePath(error) => error.fmt(formatter),
            Self::Corrupt(error) => error.fmt(formatter),
            Self::CheckpointTooLarge { length, limit } => write!(
                formatter,
                "SQLite projection checkpoint is too large: {length} bytes exceeds {limit}"
            ),
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
            Self::UnsafePath(_)
            | Self::Corrupt(_)
            | Self::CheckpointTooLarge { .. }
            | Self::CandidateRetainedSidecars => None,
        }
    }
}

fn optional_physical_file_checkpoint(
    path: &Path,
) -> Result<Option<PhysicalFileCheckpoint>, SqliteFileSetError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.len() == 0 => Ok(None),
        Ok(_) => physical_file_checkpoint(path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(SqliteFileSetError::Io(error)),
    }
}

fn physical_file_checkpoint(path: &Path) -> Result<PhysicalFileCheckpoint, SqliteFileSetError> {
    physical_file_checkpoint_with_post_metadata(path, || Ok(()))
}

fn physical_file_checkpoint_with_post_metadata<F>(
    path: &Path,
    post_metadata: F,
) -> Result<PhysicalFileCheckpoint, SqliteFileSetError>
where
    F: FnOnce() -> io::Result<()>,
{
    let metadata = fs::symlink_metadata(path).map_err(SqliteFileSetError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SqliteFileSetError::UnsafePath(format!(
            "SQLite projection file {} is not regular",
            path.display()
        )));
    }
    let length = metadata.len();
    let chunk_len = usize::try_from(physical_file_checkpoint_sample_bytes(length) / 2)
        .map_err(|_| SqliteFileSetError::Corrupt("projection file length exceeds usize".into()))?;
    post_metadata().map_err(SqliteFileSetError::Io)?;
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(SqliteFileSetError::Io)?;
    let mut first = vec![0_u8; chunk_len];
    file.read_exact(&mut first)
        .map_err(SqliteFileSetError::Io)?;
    let mut last = vec![0_u8; chunk_len];
    if length > chunk_len as u64 {
        file.seek(SeekFrom::Start(length - chunk_len as u64))
            .map_err(SqliteFileSetError::Io)?;
        file.read_exact(&mut last).map_err(SqliteFileSetError::Io)?;
    } else {
        last.copy_from_slice(&first);
    }
    let mut first_bound = b"tine/sqlite/checkpoint/v2/first\0".to_vec();
    first_bound.extend_from_slice(&length.to_be_bytes());
    first_bound.extend_from_slice(&first);
    let mut last_bound = b"tine/sqlite/checkpoint/v2/last\0".to_vec();
    last_bound.extend_from_slice(&length.to_be_bytes());
    last_bound.extend_from_slice(&last);
    let interior_sample_digest = physical_file_checkpoint_interior_digest(&mut file, length)?;
    Ok(PhysicalFileCheckpoint {
        length,
        first_chunk_digest: ContentDigest::of(&first_bound),
        last_chunk_digest: ContentDigest::of(&last_bound),
        interior_sample_digest,
    })
}

fn physical_file_checkpoint_sample_bytes(length: u64) -> u64 {
    length
        .min(SQLITE_CHECKPOINT_EDGE_BYTES as u64)
        .saturating_mul(2)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CheckpointInteriorRange {
    offset: u64,
    length: usize,
}

fn physical_file_checkpoint_interior_ranges(length: u64) -> Vec<CheckpointInteriorRange> {
    let edge_length = length.min(SQLITE_CHECKPOINT_EDGE_BYTES as u64);
    let interior_start = edge_length;
    let interior_end = length.saturating_sub(edge_length);
    let interior_length = interior_end.saturating_sub(interior_start);
    if interior_length == 0 {
        return Vec::new();
    }
    if interior_length <= SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES as u64 {
        return vec![CheckpointInteriorRange {
            offset: interior_start,
            length: usize::try_from(interior_length)
                .expect("bounded interior sample length fits usize"),
        }];
    }

    let range_length = SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES as u64;
    let available_start_span = interior_length - range_length;
    let denominator = (SQLITE_CHECKPOINT_INTERIOR_MAX_RANGES - 1) as u128;
    (0..SQLITE_CHECKPOINT_INTERIOR_MAX_RANGES)
        .map(|index| {
            let relative_offset = (u128::from(available_start_span) * index as u128) / denominator;
            CheckpointInteriorRange {
                offset: interior_start
                    + u64::try_from(relative_offset)
                        .expect("sample offset derived from a u64 file length"),
                length: SQLITE_CHECKPOINT_INTERIOR_RANGE_BYTES,
            }
        })
        .collect()
}

#[cfg(feature = "test-support")]
pub fn physical_checkpoint_interior_ranges_for_test(length: u64) -> Vec<(u64, usize)> {
    physical_file_checkpoint_interior_ranges(length)
        .into_iter()
        .map(|range| (range.offset, range.length))
        .collect()
}

fn physical_file_checkpoint_interior_digest(
    file: &mut File,
    length: u64,
) -> Result<ContentDigest, SqliteFileSetError> {
    let ranges = physical_file_checkpoint_interior_ranges(length);
    let mut bound = Vec::with_capacity(
        b"tine/sqlite/checkpoint/v2/interior-sample\0".len()
            + std::mem::size_of::<u64>()
            + std::mem::size_of::<u32>()
            + ranges.len() * (std::mem::size_of::<u64>() * 2)
            + ranges.iter().map(|range| range.length).sum::<usize>(),
    );
    bound.extend_from_slice(b"tine/sqlite/checkpoint/v2/interior-sample\0");
    bound.extend_from_slice(&length.to_be_bytes());
    bound.extend_from_slice(
        &u32::try_from(ranges.len())
            .expect("checkpoint interior range count is bounded")
            .to_be_bytes(),
    );
    for range in ranges {
        bound.extend_from_slice(&range.offset.to_be_bytes());
        bound.extend_from_slice(
            &u64::try_from(range.length)
                .expect("checkpoint interior range length fits u64")
                .to_be_bytes(),
        );
        file.seek(SeekFrom::Start(range.offset))
            .map_err(SqliteFileSetError::Io)?;
        let start = bound.len();
        bound.resize(start + range.length, 0);
        file.read_exact(&mut bound[start..])
            .map_err(SqliteFileSetError::Io)?;
    }
    Ok(ContentDigest::of(&bound))
}

fn appended_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn sync_directory(path: &Path) -> Result<(), SqliteFileSetError> {
    let directory =
        Dir::open_ambient_dir(path, ambient_authority()).map_err(SqliteFileSetError::Io)?;
    sync_dir_required(&directory).map_err(SqliteFileSetError::Io)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    const LEGACY_EDGE_BYTES: usize = 64 * 1024;
    const LEGACY_INTERIOR_SAMPLE_BYTES: usize = 1024 * 1024;
    const LEGACY_INTERIOR_RANGE_BYTES: usize = 16 * 1024;
    const LEGACY_INTERIOR_MAX_RANGES: usize =
        LEGACY_INTERIOR_SAMPLE_BYTES / LEGACY_INTERIOR_RANGE_BYTES;

    #[derive(Serialize)]
    struct LegacyBoundedFileCheckpoint {
        length: u64,
        first_chunk_digest: ContentDigest,
        last_chunk_digest: ContentDigest,
        interior_sample_digest: ContentDigest,
    }

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
    fn forensic_move_skips_absent_file_set_members() {
        let directory = TestDirectory::new("forensic-absent-members");
        let files = SqliteFileSet::new(&directory.path().join("frontier.sqlite"));
        let sources = write_forensic_sources(&files, [true, false, true, false]);
        let forensic = create_forensic_directory(&directory);

        let mappings = files
            .preserve_forensic_files(&forensic, |_moved| {})
            .unwrap();

        assert_eq!(
            mappings,
            expected_forensic_mappings(&files, &sources, &forensic)
        );
        assert_eq!(fs::read(&mappings[0].preserved_path).unwrap(), b"source-0");
        assert_eq!(fs::read(&mappings[1].preserved_path).unwrap(), b"source-2");
        assert!(!files.wal_path().exists());
        assert!(!files.checkpoint_path().exists());
    }

    #[test]
    fn forensic_move_preserves_full_set_order_paths_and_bytes() {
        let directory = TestDirectory::new("forensic-full-set");
        let files = SqliteFileSet::new(&directory.path().join("frontier.sqlite"));
        let sources = write_forensic_sources(&files, [true; 4]);
        let forensic = create_forensic_directory(&directory);
        let mut moved_counts = Vec::new();

        let mappings = files
            .preserve_forensic_files(&forensic, |moved| moved_counts.push(moved))
            .unwrap();

        assert_eq!(moved_counts, [1, 2, 3, 4]);
        assert_eq!(
            mappings,
            expected_forensic_mappings(&files, &sources, &forensic)
        );
        for (index, mapping) in mappings.iter().enumerate() {
            assert!(!mapping.original_path.exists());
            assert_eq!(
                fs::read(&mapping.preserved_path).unwrap(),
                format!("source-{index}").as_bytes()
            );
        }
    }

    #[test]
    fn forensic_resume_completes_interruption_shaped_partial_move() {
        let directory = TestDirectory::new("forensic-partial-resume");
        let files = SqliteFileSet::new(&directory.path().join("frontier.sqlite"));
        let sources = write_forensic_sources(&files, [true; 4]);
        let forensic = create_forensic_directory(&directory);

        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            files
                .preserve_forensic_files(&forensic, |moved| {
                    assert_ne!(moved, 1, "simulated process interruption after first move");
                })
                .unwrap();
        }));
        assert!(interrupted.is_err());
        assert!(!sources[0].0.exists());
        assert_eq!(fs::read(forensic.join("database")).unwrap(), b"source-0");
        for (path, bytes) in &sources[1..] {
            assert_eq!(fs::read(path).unwrap(), bytes.as_slice());
        }

        let mappings = files.resume_forensic_files(&forensic, false).unwrap();

        assert_eq!(
            mappings,
            expected_forensic_mappings(&files, &sources, &forensic)
        );
        for (index, mapping) in mappings.iter().enumerate() {
            assert!(!mapping.original_path.exists());
            assert_eq!(
                fs::read(&mapping.preserved_path).unwrap(),
                format!("source-{index}").as_bytes()
            );
        }
    }

    #[test]
    fn forensic_resume_rejects_original_and_preserved_before_completion() {
        let directory = TestDirectory::new("forensic-coexistence");
        let files = SqliteFileSet::new(&directory.path().join("frontier.sqlite"));
        let sources = write_forensic_sources(&files, [true; 4]);
        let forensic = create_forensic_directory(&directory);
        let preserved = forensic.join("database");
        fs::write(&preserved, b"preserved-database").unwrap();

        let result = files.resume_forensic_files(&forensic, false);

        assert!(matches!(
            result,
            Err(SqliteFileSetError::Corrupt(error))
                if error == format!(
                    "forensic recovery found both {} and {}",
                    sources[0].0.display(),
                    preserved.display()
                )
        ));
        assert_eq!(fs::read(&sources[0].0).unwrap(), sources[0].1.as_slice());
        assert_eq!(fs::read(&preserved).unwrap(), b"preserved-database");
        assert!(!forensic.join("wal").exists());
        assert_eq!(fs::read(&sources[1].0).unwrap(), sources[1].1.as_slice());
    }

    #[test]
    fn completed_forensic_evidence_is_observed_without_moving_sources() {
        let directory = TestDirectory::new("forensic-completed-observation");
        let files = SqliteFileSet::new(&directory.path().join("frontier.sqlite"));
        let sources = write_forensic_sources(&files, [true; 4]);
        let forensic = create_forensic_directory(&directory);
        fs::write(forensic.join("database"), b"preserved-0").unwrap();
        fs::write(forensic.join("shm"), b"preserved-2").unwrap();

        let mappings = files.resume_forensic_files(&forensic, true).unwrap();

        let expected = expected_forensic_mappings(
            &files,
            &[sources[0].clone(), sources[2].clone()],
            &forensic,
        );
        assert_eq!(mappings, expected);
        for (index, (path, bytes)) in sources.iter().enumerate() {
            assert_eq!(
                fs::read(path).unwrap(),
                bytes.as_slice(),
                "source {index} moved"
            );
        }
        assert_eq!(fs::read(forensic.join("database")).unwrap(), b"preserved-0");
        assert_eq!(fs::read(forensic.join("shm")).unwrap(), b"preserved-2");
        assert!(!forensic.join("wal").exists());
        assert!(!forensic.join("auth").exists());
    }

    #[test]
    fn physical_checkpoint_matches_legacy_values_and_wire_bytes_for_bounded_fixtures() {
        let directory = TestDirectory::new("checkpoint-differential");
        let fixtures = [
            ("empty", 0_u64, None),
            ("small", 4096, Some((2048, 0x5a))),
            (
                "large-sparse",
                64 * 1024 * 1024,
                Some((32 * 1024 * 1024, 0xa5)),
            ),
        ];

        for (label, length, marker) in fixtures {
            let path = directory.path().join(label);
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.set_len(length).unwrap();
            if let Some((offset, byte)) = marker {
                file.seek(SeekFrom::Start(offset)).unwrap();
                file.write_all(&[byte]).unwrap();
            }
            drop(file);

            let actual = physical_file_checkpoint(&path).unwrap();
            let legacy = legacy_bounded_file_checkpoint(&path);
            assert_eq!(actual.length, legacy.length, "{label}");
            assert_eq!(
                actual.first_chunk_digest, legacy.first_chunk_digest,
                "{label} first edge"
            );
            assert_eq!(
                actual.last_chunk_digest, legacy.last_chunk_digest,
                "{label} last edge"
            );
            assert_eq!(
                actual.interior_sample_digest, legacy.interior_sample_digest,
                "{label} interior"
            );
            assert_eq!(
                postcard::to_allocvec(&actual).unwrap(),
                postcard::to_allocvec(&legacy).unwrap(),
                "{label} checkpoint serialization changed"
            );
        }
    }

    #[test]
    fn physical_checkpoint_preserves_missing_empty_and_present_wal_semantics() {
        let directory = TestDirectory::new("optional-wal");
        let path = directory.path().join("frontier.sqlite");
        fs::write(&path, b"database").unwrap();
        let files = SqliteFileSet::new(&path);

        assert_eq!(files.physical_checkpoint().unwrap().wal, None);
        fs::write(files.wal_path(), []).unwrap();
        assert_eq!(files.physical_checkpoint().unwrap().wal, None);
        fs::write(files.wal_path(), b"wal").unwrap();
        let wal = files
            .physical_checkpoint()
            .unwrap()
            .wal
            .expect("non-empty WAL is checkpointed");
        assert_eq!(wal.length, 3);
    }

    #[test]
    fn large_sparse_checkpoint_has_fixed_sampling_and_detects_sampled_corruption() {
        let directory = TestDirectory::new("large-sparse-sampling");
        let path = directory.path().join("frontier.sqlite");
        let length = 64_u64 * 1024 * 1024;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(length).unwrap();
        file.write_all(&[1]).unwrap();
        file.seek(SeekFrom::Start(length - 1)).unwrap();
        file.write_all(&[2]).unwrap();
        drop(file);

        let before = physical_file_checkpoint(&path).unwrap();
        let ranges = physical_file_checkpoint_interior_ranges(length);
        assert_eq!(ranges.len(), SQLITE_CHECKPOINT_INTERIOR_MAX_RANGES);
        assert_eq!(
            ranges.iter().map(|range| range.length).sum::<usize>(),
            SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES
        );
        assert!(ranges
            .windows(2)
            .all(|pair| pair[0].offset + pair[0].length as u64 <= pair[1].offset));
        let range = ranges[SQLITE_CHECKPOINT_INTERIOR_MAX_RANGES / 2];
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(
            range.offset + u64::try_from(range.length / 2).unwrap(),
        ))
        .unwrap();
        file.write_all(&[3]).unwrap();
        drop(file);

        let after = physical_file_checkpoint(&path).unwrap();
        assert_eq!(after.length, before.length);
        assert_eq!(after.first_chunk_digest, before.first_chunk_digest);
        assert_eq!(after.last_chunk_digest, before.last_chunk_digest);
        assert_ne!(after.interior_sample_digest, before.interior_sample_digest);

        let maximum_ranges = physical_file_checkpoint_interior_ranges(u64::MAX);
        assert_eq!(maximum_ranges.len(), SQLITE_CHECKPOINT_INTERIOR_MAX_RANGES);
        assert_eq!(
            maximum_ranges
                .iter()
                .map(|range| range.length)
                .sum::<usize>(),
            SQLITE_CHECKPOINT_INTERIOR_SAMPLE_BYTES
        );
    }

    #[test]
    fn small_file_sampling_is_complete_without_duplicate_interior_ranges() {
        let edge = SQLITE_CHECKPOINT_EDGE_BYTES as u64;
        assert!(physical_file_checkpoint_interior_ranges(edge).is_empty());
        assert!(physical_file_checkpoint_interior_ranges(edge * 2).is_empty());

        let interior_length = 32_u64 * 1024;
        let length = edge * 2 + interior_length;
        assert_eq!(
            physical_file_checkpoint_interior_ranges(length),
            vec![CheckpointInteriorRange {
                offset: edge,
                length: interior_length as usize,
            }]
        );
        assert_eq!(
            physical_file_checkpoint_sample_bytes(length) + interior_length,
            length
        );
    }

    #[test]
    fn physical_checkpoint_rejects_non_regular_database() {
        let directory = TestDirectory::new("checkpoint-non-regular");
        let path = directory.path().join("frontier.sqlite");
        fs::create_dir(&path).unwrap();

        assert!(matches!(
            SqliteFileSet::new(&path).physical_checkpoint(),
            Err(SqliteFileSetError::UnsafePath(error))
                if error == format!(
                    "SQLite projection file {} is not regular",
                    path.display()
                )
        ));
    }

    #[test]
    fn file_truncated_after_metadata_preserves_exact_io_error_mapping() {
        let directory = TestDirectory::new("checkpoint-changing-file");
        let path = directory.path().join("frontier.sqlite");
        fs::write(&path, vec![7_u8; SQLITE_CHECKPOINT_EDGE_BYTES * 2 + 1]).unwrap();

        let result = physical_file_checkpoint_with_post_metadata(&path, || {
            OpenOptions::new().write(true).open(&path)?.set_len(0)
        });
        assert!(matches!(
            result,
            Err(SqliteFileSetError::Io(error))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn checkpoint_publication_admits_only_bounded_bytes() {
        let directory = TestDirectory::new("checkpoint-byte-bound");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        fs::write(files.checkpoint_path(), b"predecessor").unwrap();

        let result = files.publish_checkpoint(&vec![0_u8; MAX_SQLITE_CHECKPOINT_BYTES + 1]);

        assert!(matches!(
            result,
            Err(SqliteFileSetError::CheckpointTooLarge { length, limit })
                if length == MAX_SQLITE_CHECKPOINT_BYTES + 1
                    && limit == MAX_SQLITE_CHECKPOINT_BYTES
        ));
        assert_eq!(fs::read(files.checkpoint_path()).unwrap(), b"predecessor");

        let admitted = vec![7_u8; MAX_SQLITE_CHECKPOINT_BYTES];
        files.publish_checkpoint(&admitted).unwrap();
        assert_eq!(fs::read(files.checkpoint_path()).unwrap(), admitted);
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_temporary_symlink_collision_is_not_followed_and_retries() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("checkpoint-temp-symlink-collision");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"must survive").unwrap();
        let collision_name = ".frontier.sqlite-auth.tmp-collision";
        symlink(&sentinel, directory.path().join(collision_name)).unwrap();

        files
            .publish_checkpoint_with(
                b"authenticated envelope",
                |_name, attempt| {
                    if attempt == 0 {
                        collision_name.into()
                    } else {
                        ".frontier.sqlite-auth.tmp-retry".into()
                    }
                },
                |_stage| Ok(()),
            )
            .unwrap();

        assert_eq!(fs::read(&sentinel).unwrap(), b"must survive");
        assert_eq!(
            fs::read(files.checkpoint_path()).unwrap(),
            b"authenticated envelope"
        );
        assert!(fs::symlink_metadata(directory.path().join(collision_name))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!directory
            .path()
            .join(".frontier.sqlite-auth.tmp-retry")
            .exists());
    }

    #[test]
    fn checkpoint_temporary_collision_retry_is_explicitly_bounded() {
        let directory = TestDirectory::new("checkpoint-temp-collision-bound");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        fs::write(files.checkpoint_path(), b"predecessor").unwrap();
        for attempt in 0..SQLITE_CHECKPOINT_TEMP_ATTEMPTS {
            fs::write(
                directory.path().join(format!(".collision-{attempt}")),
                b"occupied",
            )
            .unwrap();
        }

        let result = files.publish_checkpoint_with(
            b"authenticated envelope",
            |_name, attempt| format!(".collision-{attempt}"),
            |_stage| Ok(()),
        );

        assert!(matches!(
            result,
            Err(SqliteFileSetError::Io(error))
                if error.kind() == io::ErrorKind::AlreadyExists
        ));
        assert_eq!(fs::read(files.checkpoint_path()).unwrap(), b"predecessor");
    }

    #[test]
    fn checkpoint_failure_after_file_sync_cleans_the_temporary() {
        let directory = TestDirectory::new("checkpoint-cleanup-after-file-sync");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        fs::write(files.checkpoint_path(), b"predecessor").unwrap();
        let temporary_name = ".frontier.sqlite-auth.tmp-injected";

        let result = files.publish_checkpoint_with(
            b"authenticated envelope",
            |_name, _attempt| temporary_name.into(),
            |stage| {
                if stage == CheckpointPublicationStage::FileSynced {
                    Err(SqliteFileSetError::Io(io::Error::other(
                        "simulated failure after file sync",
                    )))
                } else {
                    Ok(())
                }
            },
        );

        assert!(matches!(result, Err(SqliteFileSetError::Io(_))));
        assert_eq!(fs::read(files.checkpoint_path()).unwrap(), b"predecessor");
        assert!(!directory.path().join(temporary_name).exists());
    }

    #[test]
    fn checkpoint_retry_after_crash_before_replacement_keeps_predecessor_safe() {
        let directory = TestDirectory::new("checkpoint-crash-before-replace");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        fs::write(files.checkpoint_path(), b"predecessor").unwrap();

        // Recreate the durable state left by process death after the temporary
        // file was synced but before it replaced the stable checkpoint.
        let abandoned = directory.path().join(".frontier.sqlite-auth.tmp-abandoned");
        let mut abandoned_file = fs::File::create(&abandoned).unwrap();
        abandoned_file
            .write_all(b"complete but unpublished envelope")
            .unwrap();
        abandoned_file.sync_all().unwrap();
        drop(abandoned_file);
        assert_eq!(fs::read(files.checkpoint_path()).unwrap(), b"predecessor");

        files
            .publish_checkpoint_with(
                b"replacement envelope",
                |_name, _attempt| ".frontier.sqlite-auth.tmp-retry".into(),
                |_stage| Ok(()),
            )
            .unwrap();

        assert_eq!(
            fs::read(files.checkpoint_path()).unwrap(),
            b"replacement envelope"
        );
        assert_eq!(
            fs::read(abandoned).unwrap(),
            b"complete but unpublished envelope"
        );
    }

    #[test]
    fn checkpoint_crash_after_atomic_replacement_exposes_complete_bytes_and_retries() {
        let directory = TestDirectory::new("checkpoint-crash-after-replace");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        fs::write(files.checkpoint_path(), b"predecessor").unwrap();
        let temporary_name = ".frontier.sqlite-auth.tmp-crash";

        let result = files.publish_checkpoint_with(
            b"complete authenticated envelope",
            |_name, _attempt| temporary_name.into(),
            |stage| {
                if stage == CheckpointPublicationStage::Replaced {
                    Err(SqliteFileSetError::Io(io::Error::other(
                        "simulated process crash after replacement",
                    )))
                } else {
                    Ok(())
                }
            },
        );

        assert!(matches!(result, Err(SqliteFileSetError::Io(_))));
        assert_eq!(
            fs::read(files.checkpoint_path()).unwrap(),
            b"complete authenticated envelope"
        );
        assert!(!directory.path().join(temporary_name).exists());

        let mut retry_stages = Vec::new();
        files
            .publish_checkpoint_with(
                b"complete authenticated envelope",
                |_name, _attempt| ".frontier.sqlite-auth.tmp-retry-after-crash".into(),
                |stage| {
                    retry_stages.push(stage);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            retry_stages.last(),
            Some(&CheckpointPublicationStage::ParentSyncApplied)
        );
        assert_eq!(
            fs::read(files.checkpoint_path()).unwrap(),
            b"complete authenticated envelope"
        );
    }

    #[test]
    fn checkpoint_success_applies_file_and_platform_parent_sync_in_order() {
        let directory = TestDirectory::new("checkpoint-durability-order");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        let mut stages = Vec::new();

        files
            .publish_checkpoint_with(
                b"authenticated envelope",
                |_name, _attempt| ".frontier.sqlite-auth.tmp-durable".into(),
                |stage| {
                    stages.push(stage);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(
            stages,
            [
                CheckpointPublicationStage::FileSynced,
                CheckpointPublicationStage::Replaced,
                CheckpointPublicationStage::ParentSyncApplied,
            ]
        );
        assert_eq!(
            fs::read(files.checkpoint_path()).unwrap(),
            b"authenticated envelope"
        );
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_replacement_unlinks_predecessor_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("checkpoint-replace-symlink");
        let path = directory.path().join("frontier.sqlite");
        let files = SqliteFileSet::new(&path);
        let sentinel = directory.path().join("sentinel");
        fs::write(&sentinel, b"must survive").unwrap();
        symlink(&sentinel, files.checkpoint_path()).unwrap();

        files.publish_checkpoint(b"authenticated envelope").unwrap();

        assert_eq!(fs::read(&sentinel).unwrap(), b"must survive");
        assert_eq!(
            fs::read(files.checkpoint_path()).unwrap(),
            b"authenticated envelope"
        );
        assert!(!fs::symlink_metadata(files.checkpoint_path())
            .unwrap()
            .file_type()
            .is_symlink());
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

    fn create_forensic_directory(directory: &TestDirectory) -> PathBuf {
        let forensic = directory.path().join("frontier.sqlite.forensic-test");
        fs::create_dir(&forensic).unwrap();
        forensic
    }

    fn write_forensic_sources(
        files: &SqliteFileSet,
        present: [bool; 4],
    ) -> Vec<(PathBuf, Vec<u8>)> {
        files
            .paths()
            .into_iter()
            .enumerate()
            .filter_map(|(index, path)| {
                present[index].then(|| {
                    let bytes = format!("source-{index}").into_bytes();
                    fs::write(path, &bytes).unwrap();
                    (path.to_path_buf(), bytes)
                })
            })
            .collect()
    }

    fn expected_forensic_mappings(
        files: &SqliteFileSet,
        sources: &[(PathBuf, Vec<u8>)],
        directory: &Path,
    ) -> Vec<SqliteForensicPathMapping> {
        sources
            .iter()
            .map(|(original_path, _bytes)| {
                let index = files
                    .paths()
                    .into_iter()
                    .position(|path| path == original_path)
                    .expect("test source belongs to the SQLite file set");
                SqliteForensicPathMapping {
                    original_path: original_path.clone(),
                    preserved_path: directory.join(SQLITE_FORENSIC_NAMES[index]),
                }
            })
            .collect()
    }

    fn legacy_bounded_file_checkpoint(path: &Path) -> LegacyBoundedFileCheckpoint {
        let length = fs::metadata(path).unwrap().len();
        let chunk_len = usize::try_from(length.min(LEGACY_EDGE_BYTES as u64)).unwrap();
        let mut file = OpenOptions::new().read(true).open(path).unwrap();
        let mut first = vec![0_u8; chunk_len];
        file.read_exact(&mut first).unwrap();
        let mut last = vec![0_u8; chunk_len];
        if length > chunk_len as u64 {
            file.seek(SeekFrom::Start(length - chunk_len as u64))
                .unwrap();
            file.read_exact(&mut last).unwrap();
        } else {
            last.copy_from_slice(&first);
        }
        let mut first_bound = b"tine/sqlite/checkpoint/v2/first\0".to_vec();
        first_bound.extend_from_slice(&length.to_be_bytes());
        first_bound.extend_from_slice(&first);
        let mut last_bound = b"tine/sqlite/checkpoint/v2/last\0".to_vec();
        last_bound.extend_from_slice(&length.to_be_bytes());
        last_bound.extend_from_slice(&last);

        let ranges = legacy_interior_ranges(length);
        let mut bound = Vec::new();
        bound.extend_from_slice(b"tine/sqlite/checkpoint/v2/interior-sample\0");
        bound.extend_from_slice(&length.to_be_bytes());
        bound.extend_from_slice(&u32::try_from(ranges.len()).unwrap().to_be_bytes());
        for (offset, range_length) in ranges {
            bound.extend_from_slice(&offset.to_be_bytes());
            bound.extend_from_slice(&u64::try_from(range_length).unwrap().to_be_bytes());
            file.seek(SeekFrom::Start(offset)).unwrap();
            let start = bound.len();
            bound.resize(start + range_length, 0);
            file.read_exact(&mut bound[start..]).unwrap();
        }
        LegacyBoundedFileCheckpoint {
            length,
            first_chunk_digest: ContentDigest::of(&first_bound),
            last_chunk_digest: ContentDigest::of(&last_bound),
            interior_sample_digest: ContentDigest::of(&bound),
        }
    }

    fn legacy_interior_ranges(length: u64) -> Vec<(u64, usize)> {
        let edge_length = length.min(LEGACY_EDGE_BYTES as u64);
        let interior_start = edge_length;
        let interior_end = length.saturating_sub(edge_length);
        let interior_length = interior_end.saturating_sub(interior_start);
        if interior_length == 0 {
            return Vec::new();
        }
        if interior_length <= LEGACY_INTERIOR_SAMPLE_BYTES as u64 {
            return vec![(interior_start, usize::try_from(interior_length).unwrap())];
        }
        let range_length = LEGACY_INTERIOR_RANGE_BYTES as u64;
        let available_start_span = interior_length - range_length;
        let denominator = (LEGACY_INTERIOR_MAX_RANGES - 1) as u128;
        (0..LEGACY_INTERIOR_MAX_RANGES)
            .map(|index| {
                let relative_offset =
                    (u128::from(available_start_span) * index as u128) / denominator;
                (
                    interior_start + u64::try_from(relative_offset).unwrap(),
                    LEGACY_INTERIOR_RANGE_BYTES,
                )
            })
            .collect()
    }
}
