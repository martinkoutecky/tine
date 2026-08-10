#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};
use cap_std::fs::{Dir, OpenOptions};
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
use uuid::Uuid;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ImmutablePublicationTestStats {
    exact_durability_barriers: usize,
    batch_durability_barriers: usize,
}

#[cfg(test)]
thread_local! {
    static IMMUTABLE_PUBLICATION_TEST_STATS: std::cell::Cell<ImmutablePublicationTestStats> =
        const { std::cell::Cell::new(ImmutablePublicationTestStats {
            exact_durability_barriers: 0,
            batch_durability_barriers: 0,
        }) };
}

#[cfg(test)]
fn reset_immutable_publication_test_stats() {
    IMMUTABLE_PUBLICATION_TEST_STATS.with(|stats| stats.set(Default::default()));
}

#[cfg(test)]
fn immutable_publication_test_stats() -> ImmutablePublicationTestStats {
    IMMUTABLE_PUBLICATION_TEST_STATS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn note_exact_durability_barrier() {
    IMMUTABLE_PUBLICATION_TEST_STATS.with(|stats| {
        let mut current = stats.get();
        current.exact_durability_barriers = current.exact_durability_barriers.saturating_add(1);
        stats.set(current);
    });
}

#[cfg(test)]
fn note_batch_durability_barrier() {
    IMMUTABLE_PUBLICATION_TEST_STATS.with(|stats| {
        let mut current = stats.get();
        current.batch_durability_barriers = current.batch_durability_barriers.saturating_add(1);
        stats.set(current);
    });
}

/// A failure at the generic physical-filesystem boundary.
#[derive(Debug)]
pub enum FilesystemError {
    Io(io::Error),
    UnsafeEntry(String),
    StoredLengthMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    StoredFileTooLarge {
        path: String,
        length: u64,
        limit: u64,
    },
    ByteCollision,
}

impl fmt::Display for FilesystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::UnsafeEntry(message) => message.fmt(f),
            Self::StoredLengthMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "stored file length mismatch for {path}: expected {expected}, got {actual}"
            ),
            Self::StoredFileTooLarge {
                path,
                length,
                limit,
            } => write!(
                f,
                "stored file is too large for {path}: {length} bytes exceeds {limit}"
            ),
            Self::ByteCollision => f.write_str("immutable byte collision"),
        }
    }
}

impl std::error::Error for FilesystemError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for FilesystemError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

// `LockFileEx(..., LOCKFILE_FAIL_IMMEDIATELY, ...)` reports this Win32 code
// when another handle owns an overlapping byte-range lock. Keep the numeric
// value available to platform-neutral unit tests; the Windows SDK defines
// `ERROR_LOCK_VIOLATION` as 33.
const WINDOWS_ERROR_LOCK_VIOLATION: i32 = 33;

/// Whether one failed nonblocking file-lock attempt means genuine contention.
///
/// `WouldBlock` is the portable fs2 contention kind. On Windows, fs2 uses
/// `LockFileEx`; failed immediate acquisition surfaces `ERROR_LOCK_VIOLATION`
/// with `ErrorKind::Uncategorized`, so the raw code is part of the classifier.
/// `PermissionDenied` is deliberately not universal: callers that historically
/// treated it as contention retain that policy explicitly, while other lock
/// domains continue to fail closed. `ERROR_SHARING_VIOLATION` is likewise not
/// contention here: it is an open/share-mode conflict before `LockFileEx` runs.
pub fn nonblocking_lock_is_contended(error: &io::Error) -> bool {
    nonblocking_lock_is_contended_for_platform(error, cfg!(windows))
}

fn nonblocking_lock_is_contended_for_platform(error: &io::Error, windows: bool) -> bool {
    error.kind() == ErrorKind::WouldBlock
        || windows && error.raw_os_error() == Some(WINDOWS_ERROR_LOCK_VIOLATION)
}

/// A directory capability validated for a durable name-operation publication.
#[cfg(windows)]
pub struct ValidatedDirectorySync {
    // Retain the exact validated object for the whole publication. cap-std
    // opens directory capabilities without FILE_SHARE_DELETE, so this object
    // cannot be renamed or deleted underneath the operation.
    _capability: fs::File,
    entry_durability: WindowsDirectoryEntryDurability,
}

/// A directory capability validated for a durable name-operation publication.
#[cfg(not(windows))]
pub struct ValidatedDirectorySync<'a>(&'a Dir);

#[cfg(any(test, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsDirectoryEntryDurability {
    UnsupportedAfterValidation,
}

#[cfg(windows)]
impl ValidatedDirectorySync {
    /// Validate and retain `dir` for the duration of a publication.
    pub fn open(dir: &Dir) -> io::Result<Self> {
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        let capability = dir.try_clone()?.into_std_file();
        let metadata = capability.metadata()?;
        let entry_durability = validated_windows_directory_entry_durability(
            metadata.is_dir(),
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        )?;

        Ok(Self {
            _capability: capability,
            entry_durability,
        })
    }

    /// Validate that directory synchronization can proceed.
    pub fn preflight(&self) -> io::Result<()> {
        self.sync()
    }

    /// Synchronize the directory entry or report the platform durability limit.
    pub fn sync(&self) -> io::Result<()> {
        match self.entry_durability {
            WindowsDirectoryEntryDurability::UnsupportedAfterValidation => Ok(()),
        }
    }
}

#[cfg(unix)]
impl<'a> ValidatedDirectorySync<'a> {
    /// Validate and retain `dir` for the duration of a publication.
    pub fn open(dir: &'a Dir) -> io::Result<Self> {
        Ok(Self(dir))
    }

    /// Validate that directory synchronization can proceed.
    pub fn preflight(&self) -> io::Result<()> {
        Ok(())
    }

    /// Synchronize the directory entry.
    pub fn sync(&self) -> io::Result<()> {
        // cap-std may retain an O_PATH capability, which is suitable for openat
        // but cannot itself be fsynced. Open `.` as a real directory descriptor.
        let fd = unsafe {
            libc::openat(
                self.0.as_fd().as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned one newly owned directory descriptor.
        unsafe { fs::File::from_raw_fd(fd) }.sync_all()
    }
}

#[cfg(not(any(unix, windows)))]
impl<'a> ValidatedDirectorySync<'a> {
    /// Validate and retain `dir` for the duration of a publication.
    pub fn open(dir: &'a Dir) -> io::Result<Self> {
        Ok(Self(dir))
    }

    /// Validate that directory synchronization can proceed.
    pub fn preflight(&self) -> io::Result<()> {
        Ok(())
    }

    /// Synchronize the directory entry.
    pub fn sync(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory durability is unsupported on this target",
        ))
    }
}

/// Synchronize `dir` after a required durable directory-entry update.
pub fn sync_dir_required(dir: &Dir) -> io::Result<()> {
    ValidatedDirectorySync::open(dir)?.sync()
}

pub fn ensure_directory_nofollow(root: &Dir, name: &str) -> Result<(), FilesystemError> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(FilesystemError::UnsafeEntry(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    root.create_dir(name)?;
    sync_dir_required(root)?;
    Ok(())
}

pub fn open_existing_dir_nofollow(root: &Dir, name: &str) -> Result<Option<Dir>, FilesystemError> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            FilesystemError::UnsafeEntry(format!("{name} is not a real no-follow directory")),
        ),
        Ok(_) => open_dir_nofollow(root, name).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
pub fn open_file_nofollow(dir: &Dir, path: &str) -> io::Result<fs::File> {
    let path = CString::new(path)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid stored filename"))?;
    // SAFETY: `path` is a live NUL-terminated string and `dir` is an opened
    // directory capability. O_NOFOLLOW binds validation and reading to the
    // same opened regular-file handle.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a newly owned descriptor.
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
pub fn open_file_nofollow(dir: &Dir, path: &str) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(path, &options)?.into_std();
    reject_windows_reparse(&file, path)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
pub fn open_file_nofollow(_dir: &Dir, _path: &str) -> io::Result<fs::File> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow reads are unsupported on this target",
    ))
}

#[cfg(unix)]
pub fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, FilesystemError> {
    let path = CString::new(path)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid directory name"))?;
    // SAFETY: as in `open_file_nofollow`; O_DIRECTORY rejects non-directories
    // and O_NOFOLLOW rejects a final-component symlink in the same operation.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: `openat` returned one newly owned directory descriptor.
    Ok(Dir::from_std_file(unsafe { fs::File::from_raw_fd(fd) }))
}

#[cfg(windows)]
pub fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, FilesystemError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .maybe_dir(true);
    let file = dir.open_with(path, &options)?.into_std();
    let metadata = file.metadata()?;
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
        || !metadata.is_dir()
    {
        return Err(FilesystemError::UnsafeEntry(format!(
            "{path} is not a real no-follow directory"
        )));
    }
    Ok(Dir::from_std_file(file))
}

#[cfg(windows)]
fn reject_windows_reparse(file: &fs::File, path: &str) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("opened path is a reparse point: {path}"),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub fn open_dir_nofollow(_dir: &Dir, _path: &str) -> Result<Dir, FilesystemError> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow directory opens are unsupported on this target",
    )
    .into())
}

pub fn require_regular_entry(
    file_type: &cap_std::fs::FileType,
    name: &str,
) -> Result<(), FilesystemError> {
    if file_type.is_symlink() || !file_type.is_file() {
        Err(FilesystemError::UnsafeEntry(format!(
            "namespace entry is not a regular no-follow file: {name}"
        )))
    } else {
        Ok(())
    }
}

pub fn read_optional_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Option<Vec<u8>>, FilesystemError> {
    // Windows refuses to open a directory through the file-only capability
    // before we can classify its handle. Preclassify an existing non-file,
    // then still validate the opened handle below so a concurrent replacement
    // cannot turn this check into authority.
    match dir.symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(FilesystemError::UnsafeEntry(format!(
                "stored path is not a regular no-follow file: {path}"
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut file = match open_file_nofollow(dir, path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(FilesystemError::UnsafeEntry(format!(
            "stored path is not a regular no-follow file: {path}"
        )));
    }
    let length = metadata.len();
    if let Some(expected) = expected_length {
        if length != expected {
            return Err(FilesystemError::StoredLengthMismatch {
                path: path.into(),
                expected,
                actual: length,
            });
        }
    }
    if length > limit {
        return Err(FilesystemError::StoredFileTooLarge {
            path: path.into(),
            length,
            limit,
        });
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(FilesystemError::StoredFileTooLarge {
            path: path.into(),
            length: bytes.len() as u64,
            limit,
        });
    }
    if bytes.len() as u64 != length {
        return Err(FilesystemError::StoredLengthMismatch {
            path: path.into(),
            expected: length,
            actual: bytes.len() as u64,
        });
    }
    Ok(Some(bytes))
}

pub fn read_required_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Vec<u8>, FilesystemError> {
    read_optional_regular(dir, path, limit, expected_length)?.ok_or_else(|| {
        FilesystemError::Io(io::Error::new(
            ErrorKind::NotFound,
            format!("missing stored file {path}"),
        ))
    })
}

pub fn publish_immutable_exact(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
) -> Result<(), FilesystemError> {
    // Windows clones, retains, and validates the exact directory capability
    // before inserting an immutable target name. Win32 exposes no documented
    // directory-entry flush, so that validated state explicitly records the
    // platform limitation; it never classifies an I/O error as success.
    let publication_sync = ValidatedDirectorySync::open(dir)?;
    publication_sync.preflight()?;
    let temp_name = format!(".tmp-{}", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut temp = dir.open_with(&temp_name, &options)?;
    let result = (|| {
        temp.write_all(bytes)?;
        temp.sync_all()?;
        drop(temp);
        match rename_noreplace(dir, &temp_name, filename) {
            // A post-insertion sync error can leave the correct immutable
            // target present. Retrying verifies bytes and retries the barrier.
            Ok(()) => publication_sync.sync().map_err(FilesystemError::from),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                verify_existing(dir, filename, bytes)?;
                publication_sync.sync().map_err(FilesystemError::from)
            }
            Err(error) => Err(error.into()),
        }
    })();
    let cleanup = dir.remove_file(&temp_name);
    if let Err(error) = result {
        let _ = cleanup;
        return Err(error);
    }
    if cleanup
        .as_ref()
        .is_err_and(|error| error.kind() != ErrorKind::NotFound)
    {
        cleanup?;
    }
    #[cfg(test)]
    note_exact_durability_barrier();
    Ok(())
}

/// One synced, unpublished exact immutable file.
///
/// Construction writes through the supplied file handle and returns the exact
/// final name and length only after it has finished deriving the content
/// address. The staged object owns the temporary name and removes it on drop
/// until a consuming no-replace commit installs (or exact-verifies) the final
/// immutable name.
pub struct StagedExactImmutablePublication {
    dir: Dir,
    temp_name: String,
    final_name: String,
    exact_length: u64,
}

impl StagedExactImmutablePublication {
    pub fn construct(
        dir: &Dir,
        construct: impl FnOnce(&mut fs::File) -> io::Result<(String, u64)>,
    ) -> Result<Self, FilesystemError> {
        let publication_sync = ValidatedDirectorySync::open(dir)?;
        publication_sync.preflight()?;
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        let mut temp = dir.open_with(&temp_name, &options)?.into_std();
        let constructed = construct(&mut temp);
        let (final_name, exact_length) = match constructed {
            Ok(constructed) => constructed,
            Err(error) => {
                drop(temp);
                let _ = dir.remove_file(&temp_name);
                return Err(error.into());
            }
        };
        let actual = match temp.metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                drop(temp);
                let _ = dir.remove_file(&temp_name);
                return Err(error.into());
            }
        };
        if actual != exact_length {
            drop(temp);
            let _ = dir.remove_file(&temp_name);
            return Err(FilesystemError::StoredLengthMismatch {
                path: final_name,
                expected: exact_length,
                actual,
            });
        }
        if let Err(error) = temp.sync_all() {
            drop(temp);
            let _ = dir.remove_file(&temp_name);
            return Err(error.into());
        }
        drop(temp);
        let staged_dir = match dir.try_clone() {
            Ok(dir) => dir,
            Err(error) => {
                let _ = dir.remove_file(&temp_name);
                return Err(error.into());
            }
        };
        Ok(Self {
            dir: staged_dir,
            temp_name,
            final_name,
            exact_length,
        })
    }

    /// Open the synced temporary bytes for a bounded construction cursor.
    pub(crate) fn open_staged(&self) -> Result<fs::File, FilesystemError> {
        let file = open_file_nofollow(&self.dir, &self.temp_name)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(FilesystemError::UnsafeEntry(format!(
                "staged path is not a regular no-follow file: {}",
                self.temp_name
            )));
        }
        if metadata.len() != self.exact_length {
            return Err(FilesystemError::StoredLengthMismatch {
                path: self.temp_name.clone(),
                expected: self.exact_length,
                actual: metadata.len(),
            });
        }
        Ok(file)
    }

    pub(crate) fn owned_name_bytes(&self) -> usize {
        self.temp_name
            .capacity()
            .saturating_add(self.final_name.capacity())
    }

    /// Atomically install the exact final name without replacement, or stream-
    /// compare an existing winner before repeating the directory barrier.
    pub fn commit(self) -> Result<(), FilesystemError> {
        let publication_sync = ValidatedDirectorySync::open(&self.dir)?;
        publication_sync.preflight()?;
        match rename_noreplace(&self.dir, &self.temp_name, &self.final_name) {
            Ok(()) => publication_sync.sync()?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                verify_existing_staged(&self)?;
                publication_sync.sync()?;
            }
            Err(error) => return Err(error.into()),
        }
        #[cfg(test)]
        note_exact_durability_barrier();
        Ok(())
    }
}

impl Drop for StagedExactImmutablePublication {
    fn drop(&mut self) {
        let _ = self.dir.remove_file(&self.temp_name);
    }
}

const STAGED_COMPARE_BUFFER_BYTES: usize = 64 * 1024;

fn verify_existing_staged(staged: &StagedExactImmutablePublication) -> Result<(), FilesystemError> {
    let mut existing = match open_file_nofollow(&staged.dir, &staged.final_name) {
        Ok(file) => file,
        Err(error) => return Err(error.into()),
    };
    let existing_metadata = existing.metadata()?;
    if !existing_metadata.is_file() || existing_metadata.len() != staged.exact_length {
        return Err(FilesystemError::ByteCollision);
    }
    let mut source = staged.open_staged()?;
    let mut existing_buffer = [0_u8; STAGED_COMPARE_BUFFER_BYTES];
    let mut source_buffer = [0_u8; STAGED_COMPARE_BUFFER_BYTES];
    let mut remaining = staged.exact_length;
    while remaining != 0 {
        let chunk = usize::try_from(remaining.min(STAGED_COMPARE_BUFFER_BYTES as u64))
            .map_err(|_| FilesystemError::ByteCollision)?;
        existing.read_exact(&mut existing_buffer[..chunk])?;
        source.read_exact(&mut source_buffer[..chunk])?;
        if existing_buffer[..chunk] != source_buffer[..chunk] {
            return Err(FilesystemError::ByteCollision);
        }
        remaining -= chunk as u64;
    }
    Ok(())
}

/// Durably replace one fixed regular file after checking the exact authority
/// the caller observed under its external single-writer contract.
///
/// This is deliberately not a lock-free compare-and-swap. The equality check
/// and rename are serialized by the caller's writer lease. A retry that finds
/// the replacement already installed succeeds after repeating the directory
/// durability barrier; any other unexpected current bytes fail closed.
pub(crate) fn transition_regular_exact(
    dir: &Dir,
    filename: &str,
    expected: Option<&[u8]>,
    replacement: &[u8],
) -> Result<(), FilesystemError> {
    let publication_sync = ValidatedDirectorySync::open(dir)?;
    publication_sync.preflight()?;
    let current = read_optional_regular(
        dir,
        filename,
        replacement.len() as u64,
        Some(replacement.len() as u64),
    )?;
    if current.as_deref() == Some(replacement) {
        publication_sync.sync()?;
        return Ok(());
    }
    if current.as_deref() != expected {
        return Err(FilesystemError::ByteCollision);
    }

    let temp_name = format!(".tmp-{}", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut temp = dir.open_with(&temp_name, &options)?;
    let result = (|| {
        temp.write_all(replacement)?;
        temp.sync_all()?;
        drop(temp);
        if expected.is_some() {
            dir.rename(&temp_name, dir, filename)?;
        } else {
            rename_noreplace(dir, &temp_name, filename)?;
        }
        publication_sync.sync().map_err(FilesystemError::from)
    })();
    let cleanup = dir.remove_file(&temp_name);
    if let Err(error) = result {
        let _ = cleanup;
        return Err(error);
    }
    if cleanup
        .as_ref()
        .is_err_and(|error| error.kind() != ErrorKind::NotFound)
    {
        cleanup?;
    }
    Ok(())
}

/// An exact immutable publication batch which yields completion only from
/// `finish` after its platform durability construction has completed.
pub struct ExactImmutablePublicationBatch {
    archive: Dir,
    publications: usize,
    existing_publications: usize,
}

/// Non-forgeable evidence that an exact immutable publication batch finished.
pub struct CompletedExactImmutablePublicationBatch {
    _private: (),
    publications: usize,
    existing_publications: usize,
}

impl ExactImmutablePublicationBatch {
    pub fn new(archive: &Dir) -> Result<Self, FilesystemError> {
        Ok(Self {
            archive: archive.try_clone()?,
            publications: 0,
            existing_publications: 0,
        })
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub fn publish(
        &mut self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let existing = stage_immutable_unflushed(dir, filename, bytes)?;
        self.publications = self.publications.saturating_add(1);
        self.existing_publications = self
            .existing_publications
            .saturating_add(usize::from(existing));
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    pub fn publish(
        &mut self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        publish_immutable_exact(dir, filename, bytes)?;
        self.publications = self.publications.saturating_add(1);
        Ok(())
    }

    pub fn finish(self) -> Result<CompletedExactImmutablePublicationBatch, FilesystemError> {
        flush_exact_batch(&self.archive)?;
        #[cfg(test)]
        note_batch_durability_barrier();
        Ok(CompletedExactImmutablePublicationBatch {
            _private: (),
            publications: self.publications,
            existing_publications: self.existing_publications,
        })
    }
}

impl CompletedExactImmutablePublicationBatch {
    pub const fn publication_count(&self) -> usize {
        self.publications
    }

    pub const fn existing_publication_count(&self) -> usize {
        self.existing_publications
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn stage_immutable_unflushed(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
) -> Result<bool, FilesystemError> {
    let temp_name = format!(".tmp-{}", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut temp = dir.open_with(&temp_name, &options)?;
    temp.write_all(bytes)?;
    drop(temp);
    let result = match rename_noreplace(dir, &temp_name, filename) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            verify_existing(dir, filename, bytes).map(|()| true)
        }
        Err(error) => Err(error.into()),
    };
    let cleanup = dir.remove_file(&temp_name);
    if let Err(error) = result {
        let _ = cleanup;
        return Err(error);
    }
    if cleanup
        .as_ref()
        .is_err_and(|error| error.kind() != ErrorKind::NotFound)
    {
        cleanup?;
    }
    result
}

fn verify_existing(dir: &Dir, filename: &str, expected: &[u8]) -> Result<(), FilesystemError> {
    let existing = match read_required_regular(
        dir,
        filename,
        expected.len() as u64,
        Some(expected.len() as u64),
    ) {
        Ok(existing) => existing,
        Err(
            FilesystemError::StoredLengthMismatch { .. }
            | FilesystemError::StoredFileTooLarge { .. },
        ) => return Err(FilesystemError::ByteCollision),
        Err(error) => return Err(error),
    };
    if existing == expected {
        Ok(())
    } else {
        Err(FilesystemError::ByteCollision)
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(dir: &Dir, from: &str, to: &str) -> io::Result<()> {
    let from = CString::new(from)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid temporary name"))?;
    let to = CString::new(to)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid target name"))?;
    // SAFETY: both C strings are alive for the call, contain no interior NUL,
    // and both relative paths are resolved beneath the already-open directory.
    let result = unsafe {
        libc::renameat2(
            dir.as_fd().as_raw_fd(),
            from.as_ptr(),
            dir.as_fd().as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "android", windows))]
fn rename_noreplace(dir: &Dir, from: &str, to: &str) -> io::Result<()> {
    dir.hard_link(from, dir, to)?;
    dir.remove_file(from)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
)))]
fn rename_noreplace(_dir: &Dir, _from: &str, _to: &str) -> io::Result<()> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-clobber publication is unsupported on this target",
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn flush_exact_batch(archive: &Dir) -> Result<(), FilesystemError> {
    // cap-std may retain an O_PATH descriptor. Derive a real descriptor only
    // through that retained archive capability before issuing the one barrier.
    let fd = unsafe {
        libc::openat(
            archive.as_fd().as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: openat returned one newly owned directory descriptor.
    let archive = unsafe { fs::File::from_raw_fd(fd) };
    let result = unsafe { libc::syncfs(archive.as_raw_fd()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().into())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn flush_exact_batch(_archive: &Dir) -> Result<(), FilesystemError> {
    // Each entry already passed through the ordinary durable publisher.
    Ok(())
}

#[cfg(any(test, windows))]
fn validated_windows_directory_entry_durability(
    is_dir: bool,
    is_reparse: bool,
) -> io::Result<WindowsDirectoryEntryDurability> {
    if !is_dir || is_reparse {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory durability handle is not a real no-follow directory",
        ));
    }
    Ok(WindowsDirectoryEntryDurability::UnsupportedAfterValidation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::sync::{Arc, Barrier};
    use std::thread;

    struct TestDirectory {
        path: std::path::PathBuf,
        dir: Dir,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("tine-storage-{label}-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            let dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
            Self { path, dir }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn temporary_entries(dir: &Dir) -> Vec<String> {
        dir.entries()
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tmp-"))
            .collect()
    }

    fn assert_persisted_entries(fixture: &TestDirectory, entries: &[(&str, &[u8])]) {
        for (filename, bytes) in entries {
            assert_eq!(fixture.dir.read(filename).unwrap(), *bytes);
        }
        assert!(temporary_entries(&fixture.dir).is_empty());
    }

    fn publish_exact_sequence(
        fixture: &TestDirectory,
        entries: &[(&str, &[u8])],
    ) -> ImmutablePublicationTestStats {
        reset_immutable_publication_test_stats();
        for (filename, bytes) in entries {
            publish_immutable_exact(&fixture.dir, filename, bytes).unwrap();
        }
        immutable_publication_test_stats()
    }

    fn publish_batched_sequence(
        fixture: &TestDirectory,
        entries: &[(&str, &[u8])],
    ) -> (
        CompletedExactImmutablePublicationBatch,
        ImmutablePublicationTestStats,
    ) {
        reset_immutable_publication_test_stats();
        let mut batch = ExactImmutablePublicationBatch::new(&fixture.dir).unwrap();
        for (filename, bytes) in entries {
            batch.publish(&fixture.dir, filename, bytes).unwrap();
        }
        let completed = batch.finish().unwrap();
        let stats = immutable_publication_test_stats();
        (completed, stats)
    }

    #[test]
    fn exact_publish_retries_identically_without_temporary_residue() {
        let fixture = TestDirectory::new("exact-retry");
        publish_immutable_exact(&fixture.dir, "entry", b"exact bytes").unwrap();
        publish_immutable_exact(&fixture.dir, "entry", b"exact bytes").unwrap();
        assert_persisted_entries(&fixture, &[("entry", b"exact bytes")]);
    }

    fn staged_bytes(
        fixture: &TestDirectory,
        final_name: &str,
        bytes: &[u8],
    ) -> StagedExactImmutablePublication {
        StagedExactImmutablePublication::construct(&fixture.dir, |file| {
            file.write_all(bytes)?;
            Ok((final_name.to_owned(), bytes.len() as u64))
        })
        .unwrap()
    }

    #[test]
    fn staged_exact_commit_retries_streamingly_and_drop_cleans_unpublished_temp() {
        let fixture = TestDirectory::new("staged-exact");
        let abandoned = staged_bytes(&fixture, "abandoned", b"unpublished");
        assert_eq!(temporary_entries(&fixture.dir).len(), 1);
        drop(abandoned);
        assert!(temporary_entries(&fixture.dir).is_empty());

        staged_bytes(&fixture, "entry", b"exact streamed bytes")
            .commit()
            .unwrap();
        staged_bytes(&fixture, "entry", b"exact streamed bytes")
            .commit()
            .unwrap();
        assert_eq!(fixture.dir.read("entry").unwrap(), b"exact streamed bytes");
        assert!(temporary_entries(&fixture.dir).is_empty());

        assert!(matches!(
            staged_bytes(&fixture, "entry", b"conflicting streamed bytes").commit(),
            Err(FilesystemError::ByteCollision)
        ));
        assert_eq!(fixture.dir.read("entry").unwrap(), b"exact streamed bytes");
        assert!(temporary_entries(&fixture.dir).is_empty());
    }

    #[test]
    fn divergent_existing_bytes_collide_without_clobbering() {
        let fixture = TestDirectory::new("collision");
        publish_immutable_exact(&fixture.dir, "entry", b"winner").unwrap();
        assert!(matches!(
            publish_immutable_exact(&fixture.dir, "entry", b"different"),
            Err(FilesystemError::ByteCollision)
        ));
        assert_eq!(fixture.dir.read("entry").unwrap(), b"winner");
        assert!(temporary_entries(&fixture.dir).is_empty());
    }

    #[test]
    fn bounded_optional_and_required_reads_reject_invalid_entries() {
        let fixture = TestDirectory::new("bounded-read");
        fixture.dir.write("entry", b"12345").unwrap();
        assert_eq!(
            read_optional_regular(&fixture.dir, "entry", 5, Some(5)).unwrap(),
            Some(b"12345".to_vec())
        );
        assert!(matches!(
            read_optional_regular(&fixture.dir, "entry", 4, None),
            Err(FilesystemError::StoredFileTooLarge {
                path,
                length: 5,
                limit: 4,
            }) if path == "entry"
        ));
        assert!(matches!(
            read_optional_regular(&fixture.dir, "entry", 5, Some(4)),
            Err(FilesystemError::StoredLengthMismatch {
                path,
                expected: 4,
                actual: 5,
            }) if path == "entry"
        ));
        assert_eq!(
            read_optional_regular(&fixture.dir, "absent", 5, None).unwrap(),
            None
        );
        assert!(matches!(
            read_required_regular(&fixture.dir, "absent", 5, None),
            Err(FilesystemError::Io(error)) if error.kind() == ErrorKind::NotFound
        ));
        fixture.dir.create_dir("unsafe").unwrap();
        assert!(matches!(
            read_optional_regular(&fixture.dir, "unsafe", 5, None),
            Err(FilesystemError::UnsafeEntry(message))
                if message == "stored path is not a regular no-follow file: unsafe"
        ));
    }

    #[test]
    fn deferred_batch_returns_completion_only_from_finish() {
        let fixture = TestDirectory::new("batch");
        let entries = [("first", b"one".as_slice()), ("second", b"two".as_slice())];
        let (completed, batch_stats) = publish_batched_sequence(&fixture, &entries);
        assert_eq!(completed.publication_count(), 2);
        assert_eq!(completed.existing_publication_count(), 0);
        assert_persisted_entries(&fixture, &entries);
        assert_eq!(batch_stats.batch_durability_barriers, 1);

        let ordinary_fixture = TestDirectory::new("ordinary");
        let ordinary_stats = publish_exact_sequence(&ordinary_fixture, &entries);
        assert_persisted_entries(&ordinary_fixture, &entries);
        assert_eq!(ordinary_stats.batch_durability_barriers, 0);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(batch_stats.exact_durability_barriers < ordinary_stats.exact_durability_barriers);
    }

    #[test]
    fn abandoned_batch_retry_verifies_existing_bytes_and_finishes_once() {
        let fixture = TestDirectory::new("batch-retry");
        let mut abandoned = ExactImmutablePublicationBatch::new(&fixture.dir).unwrap();
        abandoned
            .publish(&fixture.dir, "existing", b"exact bytes")
            .unwrap();
        drop(abandoned);

        reset_immutable_publication_test_stats();
        let mut retry = ExactImmutablePublicationBatch::new(&fixture.dir).unwrap();
        retry
            .publish(&fixture.dir, "existing", b"exact bytes")
            .unwrap();
        let completed = retry.finish().unwrap();
        assert_eq!(completed.publication_count(), 1);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert_eq!(completed.existing_publication_count(), 1);
        assert_eq!(
            immutable_publication_test_stats().batch_durability_barriers,
            1
        );
        assert_persisted_entries(&fixture, &[("existing", b"exact bytes")]);
    }

    #[test]
    fn deferred_batch_conflicting_existing_name_fails_closed() {
        let fixture = TestDirectory::new("batch-collision");
        fixture.dir.write("collision", b"winner").unwrap();
        let mut batch = ExactImmutablePublicationBatch::new(&fixture.dir).unwrap();
        assert!(matches!(
            batch.publish(&fixture.dir, "collision", b"different"),
            Err(FilesystemError::ByteCollision)
        ));
        assert_eq!(fixture.dir.read("collision").unwrap(), b"winner");
        assert!(temporary_entries(&fixture.dir).is_empty());
    }

    #[test]
    fn concurrent_publishers_converge_and_preserve_one_divergent_winner() {
        let fixture = TestDirectory::new("concurrent");
        let path = Arc::new(fixture.path.clone());
        let barrier = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let dir = Dir::open_ambient_dir(path.as_ref(), ambient_authority()).unwrap();
                    barrier.wait();
                    publish_immutable_exact(&dir, "identical", b"same")
                })
            })
            .collect::<Vec<_>>();
        assert!(threads
            .into_iter()
            .all(|thread| thread.join().unwrap().is_ok()));
        assert_eq!(fixture.dir.read("identical").unwrap(), b"same");

        let barrier = Arc::new(Barrier::new(2));
        let threads = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .map(|bytes| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let dir = Dir::open_ambient_dir(path.as_ref(), ambient_authority()).unwrap();
                    barrier.wait();
                    publish_immutable_exact(&dir, "divergent", bytes)
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(FilesystemError::ByteCollision)))
                .count(),
            1
        );
        let winner = fixture.dir.read("divergent").unwrap();
        assert!(winner == b"first" || winner == b"second");
        assert!(temporary_entries(&fixture.dir).is_empty());
    }

    #[test]
    fn nonblocking_lock_contention_classifier_is_narrow_and_platform_explicit() {
        let would_block = io::Error::new(ErrorKind::WouldBlock, "busy");
        let permission_denied = io::Error::new(ErrorKind::PermissionDenied, "busy");
        assert!(nonblocking_lock_is_contended_for_platform(
            &would_block,
            false
        ));
        assert!(!nonblocking_lock_is_contended_for_platform(
            &permission_denied,
            false
        ));
        assert!(!nonblocking_lock_is_contended_for_platform(
            &permission_denied,
            true
        ));

        let lock_violation = io::Error::from_raw_os_error(WINDOWS_ERROR_LOCK_VIOLATION);
        assert!(nonblocking_lock_is_contended_for_platform(
            &lock_violation,
            true
        ));
        assert!(!nonblocking_lock_is_contended_for_platform(
            &lock_violation,
            false
        ));

        // ERROR_SHARING_VIOLATION (32) is an open/share-mode failure, not the
        // result of an already-open handle's nonblocking LockFileEx attempt.
        let sharing_violation = io::Error::from_raw_os_error(32);
        assert!(!nonblocking_lock_is_contended_for_platform(
            &sharing_violation,
            true
        ));
        let unrelated = io::Error::from_raw_os_error(87);
        assert!(!nonblocking_lock_is_contended_for_platform(
            &unrelated, true
        ));
    }

    #[test]
    fn validated_real_directory_has_explicit_windows_durability_limit() {
        assert_eq!(
            validated_windows_directory_entry_durability(true, false).unwrap(),
            WindowsDirectoryEntryDurability::UnsupportedAfterValidation
        );
    }

    #[test]
    fn windows_directory_validation_rejects_reparse_and_non_directory_handles() {
        assert_eq!(
            validated_windows_directory_entry_durability(false, false)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            validated_windows_directory_entry_durability(true, true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
