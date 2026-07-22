#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use uuid::Uuid;

use super::{
    BatchError, BatchId, ContentDigest, LineageDigest, ObjectDescriptor, OperationBatch,
    OperationObject, PreparedBatch, ValidatedBatch, WorkspaceId, MAX_MANIFEST_BYTES,
    MAX_OBJECT_BYTES,
};

const OBJECTS_DIR: &str = "objects";
const BATCHES_DIR: &str = "batches";

/// A caller-rooted, v2-candidate immutable object and batch-manifest store.
///
/// Opening this type is the only persistence trigger. It is intentionally not
/// connected to graph startup, enrollment, or the legacy managed-sync store.
#[derive(Debug)]
pub struct ObjectStore {
    root_path: PathBuf,
    workspace_id: WorkspaceId,
    capability: Dir,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchInspection {
    /// No manifest commit marker exists. Object-only residue remains invisible.
    Absent,
    /// The manifest is valid, but these canonical descriptors are not present.
    Staged {
        manifest: OperationBatch,
        missing: Vec<ObjectDescriptor>,
    },
    /// The manifest and its exact closed object set have been validated.
    Ready(ValidatedBatch),
}

impl ObjectStore {
    /// Open or create a store at an explicit root and retain the opened
    /// directory capability for all later operations.
    pub fn open(root: &Path, workspace_id: WorkspaceId) -> Result<Self, StoreError> {
        let name = root
            .file_name()
            .ok_or_else(|| StoreError::UnsafeEntry("store root has no final component".into()))?;
        if !matches!(root.components().next_back(), Some(Component::Normal(_))) {
            return Err(StoreError::UnsafeEntry(
                "store root must end in a normal path component".into(),
            ));
        }
        let parent = root.parent().ok_or_else(|| {
            StoreError::UnsafeEntry("store root must have an existing parent".into())
        })?;
        let canonical_parent = fs::canonicalize(parent)?;
        let parent_capability = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
        let relative = Path::new(name);
        let name = name.to_str().ok_or_else(|| {
            StoreError::UnsafeEntry("store root final component is not UTF-8".into())
        })?;

        match parent_capability.symlink_metadata(relative) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(StoreError::UnsafeEntry(
                    "store root is not a real no-follow directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                parent_capability.create_dir(relative)?;
                sync_dir_required(&parent_capability)?;
            }
            Err(error) => return Err(error.into()),
        }

        let capability = open_dir_nofollow(&parent_capability, name)?;
        ensure_directory(&capability, OBJECTS_DIR)?;
        ensure_directory(&capability, BATCHES_DIR)?;
        let store = Self {
            root_path: canonical_parent.join(name),
            workspace_id,
            capability,
        };
        store.validate_namespace()?;
        Ok(store)
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// Validate and retain one object independently of any manifest delivery.
    pub fn stage_object_bytes(&self, bytes: &[u8]) -> Result<ContentDigest, StoreError> {
        let object = OperationObject::decode(bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        let digest = ContentDigest::of(bytes);
        let objects = self.open_namespace(OBJECTS_DIR)?;
        publish_immutable(
            &objects,
            &object_filename(digest),
            bytes,
            Collision::Object(digest),
        )?;
        Ok(digest)
    }

    /// Validate and publish the sole batch commit marker. Missing objects do
    /// not prevent staging the marker and remain invisible until complete.
    pub fn stage_manifest_bytes(&self, bytes: &[u8]) -> Result<BatchId, StoreError> {
        let manifest = OperationBatch::decode(bytes)?;
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        let batch_id = manifest.batch_id();
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        if read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)?.is_some() {
            publish_immutable(&batches, &filename, bytes, Collision::Batch(batch_id))?;
            return Ok(batch_id);
        }
        if let Some(existing) = self.committed_manifests()?.first() {
            if existing.lineage_digest() != manifest.lineage_digest() {
                return Err(StoreError::LineageMismatch {
                    expected: existing.lineage_digest(),
                    found: manifest.lineage_digest(),
                });
            }
        }
        publish_immutable(&batches, &filename, bytes, Collision::Batch(batch_id))?;
        Ok(batch_id)
    }

    /// Publish a prevalidated complete batch in the required order: every
    /// content-addressed object first, then the manifest commit marker.
    pub fn publish_prepared(&self, batch: &PreparedBatch) -> Result<(), StoreError> {
        if batch.manifest().workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: batch.manifest().workspace_id(),
            });
        }
        for object in batch.objects() {
            self.stage_object_bytes(&object.encode()?)?;
        }
        self.stage_manifest_bytes(&batch.manifest().encode()?)?;
        Ok(())
    }

    /// Inspect a single manifest and validate every present required object.
    /// Missing objects stage the batch; corrupt or mismatched objects reject it.
    pub fn inspect_batch(&self, batch_id: BatchId) -> Result<BatchInspection, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        let manifest_bytes =
            match read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)? {
                None => return Ok(BatchInspection::Absent),
                Some(bytes) => bytes,
            };
        let manifest = OperationBatch::decode(&manifest_bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }

        let objects_dir = self.open_namespace(OBJECTS_DIR)?;
        let mut missing = Vec::new();
        let mut objects = Vec::with_capacity(manifest.required_objects().len());
        for descriptor in manifest.required_objects() {
            let filename = object_filename(descriptor.content_digest());
            let Some(bytes) = read_optional_regular(
                &objects_dir,
                &filename,
                MAX_OBJECT_BYTES as u64,
                Some(descriptor.encoded_byte_length()),
            )?
            else {
                missing.push(descriptor.clone());
                continue;
            };
            if ContentDigest::of(&bytes) != descriptor.content_digest() {
                return Err(StoreError::ObjectPathMismatch(descriptor.content_digest()));
            }
            let object = OperationObject::decode(&bytes)?;
            if object.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: object.workspace_id(),
                });
            }
            let actual = object.descriptor()?;
            if actual != *descriptor {
                return Err(StoreError::Batch(BatchError::DescriptorMismatch {
                    expected: descriptor.clone(),
                    actual,
                }));
            }
            objects.push(object);
        }

        if !missing.is_empty() {
            return Ok(BatchInspection::Staged { manifest, missing });
        }
        // A batch cannot become exposable while any other stored commit marker
        // belongs to a different lineage. This scan is deliberately repeated
        // at the Ready boundary: providers may populate the namespace without
        // calling `stage_manifest_bytes`, and concurrent first publishers may
        // both have observed an initially empty store.
        self.committed_manifests()?;
        let prepared = PreparedBatch::new(manifest, objects)?;
        Ok(BatchInspection::Ready(ValidatedBatch::new(prepared)))
    }

    /// Enumerate all manifest commit markers in deterministic BatchId order.
    /// Staged manifests are included; readiness is determined by `inspect_batch`.
    pub fn committed_manifests(&self) -> Result<Vec<OperationBatch>, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let mut manifests = Vec::new();
        for entry in batches.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| StoreError::MalformedPath("non-UTF-8 batch entry".into()))?;
            if is_temp_name(name) {
                require_regular_entry(&entry.file_type()?, name)?;
                continue;
            }
            require_regular_entry(&entry.file_type()?, name)?;
            let batch_id = parse_manifest_filename(name)?;
            let bytes = read_required_regular(&batches, name, MAX_MANIFEST_BYTES as u64, None)?;
            let manifest = OperationBatch::decode(&bytes)?;
            if manifest.batch_id() != batch_id {
                return Err(StoreError::ManifestPathMismatch {
                    expected: batch_id,
                    found: manifest.batch_id(),
                });
            }
            if manifest.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: manifest.workspace_id(),
                });
            }
            manifests.push(manifest);
        }
        manifests.sort_unstable_by_key(OperationBatch::batch_id);
        if let Some(first) = manifests.first() {
            for manifest in &manifests[1..] {
                if manifest.lineage_digest() != first.lineage_digest() {
                    return Err(StoreError::LineageMismatch {
                        expected: first.lineage_digest(),
                        found: manifest.lineage_digest(),
                    });
                }
            }
        }
        Ok(manifests)
    }

    pub fn contains_object(&self, digest: ContentDigest) -> Result<bool, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let Some(bytes) = read_optional_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?
        else {
            return Ok(false);
        };
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(true)
    }

    fn validate_namespace(&self) -> Result<(), StoreError> {
        let mut manifests = Vec::new();
        for (directory, kind) in [
            (OBJECTS_DIR, NamespaceKind::Objects),
            (BATCHES_DIR, NamespaceKind::Batches),
        ] {
            let dir = self.open_namespace(directory)?;
            for entry in dir.entries()? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_str().ok_or_else(|| {
                    StoreError::MalformedPath(format!("non-UTF-8 entry under {directory}"))
                })?;
                require_regular_entry(&entry.file_type()?, name)?;
                if is_temp_name(name) {
                    let limit = match kind {
                        NamespaceKind::Objects => MAX_OBJECT_BYTES as u64,
                        NamespaceKind::Batches => MAX_MANIFEST_BYTES as u64,
                    };
                    read_required_regular(&dir, name, limit, None)?;
                    continue;
                }
                match kind {
                    NamespaceKind::Objects => {
                        let expected = parse_object_filename(name)?;
                        let bytes =
                            read_required_regular(&dir, name, MAX_OBJECT_BYTES as u64, None)?;
                        if ContentDigest::of(&bytes) != expected {
                            return Err(StoreError::ObjectPathMismatch(expected));
                        }
                        let object = OperationObject::decode(&bytes)?;
                        if object.workspace_id() != self.workspace_id {
                            return Err(StoreError::WorkspaceMismatch {
                                expected: self.workspace_id,
                                found: object.workspace_id(),
                            });
                        }
                        if object.encode()?.as_slice() != bytes {
                            return Err(StoreError::ObjectPathMismatch(expected));
                        }
                    }
                    NamespaceKind::Batches => {
                        let expected = parse_manifest_filename(name)?;
                        let bytes =
                            read_required_regular(&dir, name, MAX_MANIFEST_BYTES as u64, None)?;
                        let manifest = OperationBatch::decode(&bytes)?;
                        if manifest.batch_id() != expected {
                            return Err(StoreError::ManifestPathMismatch {
                                expected,
                                found: manifest.batch_id(),
                            });
                        }
                        if manifest.workspace_id() != self.workspace_id {
                            return Err(StoreError::WorkspaceMismatch {
                                expected: self.workspace_id,
                                found: manifest.workspace_id(),
                            });
                        }
                        manifests.push(manifest);
                    }
                }
            }
        }
        ensure_single_lineage(&manifests)?;
        Ok(())
    }

    fn open_namespace(&self, name: &str) -> Result<Dir, StoreError> {
        let metadata = self.capability.symlink_metadata(name)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::UnsafeEntry(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        open_dir_nofollow(&self.capability, name)
    }
}

#[derive(Clone, Copy)]
enum NamespaceKind {
    Objects,
    Batches,
}

#[derive(Clone, Copy)]
enum Collision {
    Object(ContentDigest),
    Batch(BatchId),
}

fn ensure_single_lineage(manifests: &[OperationBatch]) -> Result<(), StoreError> {
    if let Some(first) = manifests.first() {
        for manifest in &manifests[1..] {
            if manifest.lineage_digest() != first.lineage_digest() {
                return Err(StoreError::LineageMismatch {
                    expected: first.lineage_digest(),
                    found: manifest.lineage_digest(),
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Batch(BatchError),
    UnsafeEntry(String),
    MalformedPath(String),
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    LineageMismatch {
        expected: LineageDigest,
        found: LineageDigest,
    },
    ObjectCollision(ContentDigest),
    BatchCollision(BatchId),
    ObjectPathMismatch(ContentDigest),
    ManifestPathMismatch {
        expected: BatchId,
        found: BatchId,
    },
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
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Batch(error) => error.fmt(f),
            Self::UnsafeEntry(message) => write!(f, "unsafe store entry: {message}"),
            Self::MalformedPath(path) => write!(f, "malformed store path: {path}"),
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::LineageMismatch { expected, found } => {
                write!(f, "lineage mismatch: expected {expected}, found {found}")
            }
            Self::ObjectCollision(digest) => write!(f, "content-address collision at {digest}"),
            Self::BatchCollision(batch_id) => {
                write!(f, "fatal manifest collision for batch {batch_id}")
            }
            Self::ObjectPathMismatch(digest) => {
                write!(f, "stored object bytes do not match path {digest}")
            }
            Self::ManifestPathMismatch { expected, found } => write!(
                f,
                "manifest path names batch {expected}, but bytes name {found}"
            ),
            Self::StoredLengthMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "stored file length mismatch at {path}: expected {expected}, found {actual}"
            ),
            Self::StoredFileTooLarge {
                path,
                length,
                limit,
            } => write!(
                f,
                "stored file at {path} is {length} bytes, exceeding limit {limit}"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Batch(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BatchError> for StoreError {
    fn from(error: BatchError) -> Self {
        Self::Batch(error)
    }
}

fn ensure_directory(root: &Dir, name: &str) -> Result<(), StoreError> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(StoreError::UnsafeEntry(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    root.create_dir(name)?;
    sync_dir_required(root)
}

fn publish_immutable(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    collision: Collision,
) -> Result<(), StoreError> {
    // Windows must establish that this directory supports the required
    // write-capable FlushFileBuffers operation before inserting an immutable
    // target name. The retained handle is reused for the post-insertion flush,
    // so namespace retargeting cannot redirect durability to another path.
    let publication_sync = PublicationDirSync::open(dir)?;
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
            // target present. Retrying is safe: the AlreadyExists path below
            // verifies identical bytes and retries this same required sync.
            Ok(()) => publication_sync.sync(),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                verify_existing(dir, filename, bytes, collision)?;
                publication_sync.sync()
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
    Ok(())
}

fn verify_existing(
    dir: &Dir,
    filename: &str,
    expected: &[u8],
    collision: Collision,
) -> Result<(), StoreError> {
    let existing = match read_required_regular(
        dir,
        filename,
        expected.len() as u64,
        Some(expected.len() as u64),
    ) {
        Ok(existing) => existing,
        Err(StoreError::StoredLengthMismatch { .. } | StoreError::StoredFileTooLarge { .. }) => {
            return Err(collision_error(collision));
        }
        Err(error) => return Err(error),
    };
    if existing == expected {
        return Ok(());
    }
    Err(collision_error(collision))
}

fn collision_error(collision: Collision) -> StoreError {
    match collision {
        Collision::Object(digest) => StoreError::ObjectCollision(digest),
        Collision::Batch(batch_id) => StoreError::BatchCollision(batch_id),
    }
}

#[cfg(unix)]
fn open_file_nofollow(dir: &Dir, path: &str) -> std::io::Result<fs::File> {
    let path = CString::new(path)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid stored filename"))?;
    // SAFETY: `path` is a live NUL-terminated string and `dir` is an opened
    // directory capability. O_NOFOLLOW binds validation and reading to the
    // same opened regular-file handle.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a newly owned descriptor.
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
fn open_file_nofollow(dir: &Dir, path: &str) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(path, &options)?.into_std();
    reject_windows_reparse(&file, path)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_file_nofollow(_dir: &Dir, _path: &str) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow reads are unsupported on this target",
    ))
}

#[cfg(unix)]
fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
    let path = CString::new(path)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid directory name"))?;
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
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `openat` returned a newly owned directory descriptor.
    Ok(Dir::from_std_file(unsafe { fs::File::from_raw_fd(fd) }))
}

#[cfg(windows)]
fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
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
        return Err(StoreError::UnsafeEntry(format!(
            "{path} is not a real no-follow directory"
        )));
    }
    Ok(Dir::from_std_file(file))
}

#[cfg(windows)]
fn reject_windows_reparse(file: &fs::File, path: &str) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("opened path is a reparse point: {path}"),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn open_dir_nofollow(_dir: &Dir, _path: &str) -> Result<Dir, StoreError> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow directory opens are unsupported on this target",
    )
    .into())
}

fn read_optional_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Option<Vec<u8>>, StoreError> {
    let mut file = match open_file_nofollow(dir, path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(StoreError::UnsafeEntry(format!(
            "stored path is not a regular no-follow file: {path}"
        )));
    }
    let length = metadata.len();
    if let Some(expected) = expected_length {
        if length != expected {
            return Err(StoreError::StoredLengthMismatch {
                path: path.into(),
                expected,
                actual: length,
            });
        }
    }
    if length > limit {
        return Err(StoreError::StoredFileTooLarge {
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
        return Err(StoreError::StoredFileTooLarge {
            path: path.into(),
            length: bytes.len() as u64,
            limit,
        });
    }
    if bytes.len() as u64 != length {
        return Err(StoreError::StoredLengthMismatch {
            path: path.into(),
            expected: length,
            actual: bytes.len() as u64,
        });
    }
    Ok(Some(bytes))
}

fn read_required_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Vec<u8>, StoreError> {
    read_optional_regular(dir, path, limit, expected_length)?.ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            ErrorKind::NotFound,
            format!("missing stored file {path}"),
        ))
    })
}

fn object_filename(digest: ContentDigest) -> String {
    format!("{digest}.object")
}

fn manifest_filename(batch_id: BatchId) -> String {
    format!("{batch_id}.manifest")
}

fn parse_object_filename(name: &str) -> Result<ContentDigest, StoreError> {
    let Some(digest) = name.strip_suffix(".object") else {
        return Err(StoreError::MalformedPath(name.into()));
    };
    if digest.len() != 64
        || digest
            .bytes()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(StoreError::MalformedPath(name.into()));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in digest.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]).expect("validated hex") << 4)
            | hex_nibble(pair[1]).expect("validated hex");
    }
    Ok(ContentDigest::from_bytes(bytes))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_manifest_filename(name: &str) -> Result<BatchId, StoreError> {
    let Some(batch_id) = name.strip_suffix(".manifest") else {
        return Err(StoreError::MalformedPath(name.into()));
    };
    let parsed = batch_id
        .parse::<BatchId>()
        .map_err(|_| StoreError::MalformedPath(name.into()))?;
    if parsed.to_string() != batch_id {
        return Err(StoreError::MalformedPath(name.into()));
    }
    Ok(parsed)
}

fn is_temp_name(name: &str) -> bool {
    name.strip_prefix(".tmp-")
        .and_then(|value| Uuid::parse_str(value).ok())
        .is_some()
}

fn require_regular_entry(file_type: &cap_std::fs::FileType, name: &str) -> Result<(), StoreError> {
    if file_type.is_symlink() || !file_type.is_file() {
        Err(StoreError::UnsafeEntry(format!(
            "namespace entry is not a regular no-follow file: {name}"
        )))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(dir: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    let from = CString::new(from)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid temporary name"))?;
    let to = CString::new(to)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid target name"))?;
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
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "android", windows))]
fn rename_noreplace(dir: &Dir, from: &str, to: &str) -> std::io::Result<()> {
    // Hard-link creation is an atomic exclusive name insertion on these
    // platforms: it fails if `to` already exists. Both names are in the same
    // opened directory and the source is a private, synced regular file.
    dir.hard_link(from, dir, to)?;
    dir.remove_file(from)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
)))]
fn rename_noreplace(_dir: &Dir, _from: &str, _to: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-clobber publication is unsupported on this target",
    ))
}

#[cfg(unix)]
fn sync_dir_required(dir: &Dir) -> Result<(), StoreError> {
    // cap-std may retain an O_PATH directory capability, which is suitable for
    // openat but cannot itself be fsynced. Open the capability's `.` as a real
    // directory descriptor and propagate the result of syncing that handle.
    let dot = c".";
    // SAFETY: `dot` is a static C string and `dir` is an opened directory.
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            dot.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `openat` returned a newly owned directory descriptor.
    unsafe { fs::File::from_raw_fd(fd) }.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn sync_dir_required(dir: &Dir) -> Result<(), StoreError> {
    PublicationDirSync::open(dir)?.sync()
}

#[cfg(not(any(unix, windows)))]
fn sync_dir_required(_dir: &Dir) -> Result<(), StoreError> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "directory durability is unsupported on this target",
    )
    .into())
}

#[cfg(windows)]
struct PublicationDirSync(fs::File);

#[cfg(windows)]
impl PublicationDirSync {
    fn open(dir: &Dir) -> Result<Self, StoreError> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .follow(FollowSymlinks::No)
            .maybe_dir(true);
        let file = dir.open_with(".", &options)?.into_std();
        let metadata = file.metadata()?;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
            || !metadata.is_dir()
        {
            return Err(StoreError::UnsafeEntry(
                "directory durability handle is not a real no-follow directory".into(),
            ));
        }
        Ok(Self(file))
    }

    fn preflight(&self) -> Result<(), StoreError> {
        self.sync()
    }

    fn sync(&self) -> Result<(), StoreError> {
        use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;

        // SAFETY: the handle remains owned by `self` for the call. `open`
        // requested GENERIC_WRITE, which FlushFileBuffers requires, together
        // with directory and no-follow semantics.
        if unsafe { FlushFileBuffers(self.0.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
struct PublicationDirSync<'a>(&'a Dir);

#[cfg(not(windows))]
impl<'a> PublicationDirSync<'a> {
    fn open(dir: &'a Dir) -> Result<Self, StoreError> {
        Ok(Self(dir))
    }

    fn preflight(&self) -> Result<(), StoreError> {
        Ok(())
    }

    fn sync(&self) -> Result<(), StoreError> {
        sync_dir_required(self.0)
    }
}
