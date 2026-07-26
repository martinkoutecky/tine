#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::str::FromStr;

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
#[cfg(windows)]
use cap_std::fs::{MetadataExt as _, OpenOptions};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[cfg(unix)]
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;

use super::object_store::sync_dir_required;

macro_rules! opaque_uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Mint an ordinary application identity. Deterministic derivation is
            /// intentionally not the default creation path.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Construct an explicitly supplied application identity.
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

opaque_uuid_id!(
    /// Opaque identity of a managed Tine workspace.
    WorkspaceId
);
opaque_uuid_id!(
    /// Opaque identity of a page, independent of its name or path.
    PageId
);
opaque_uuid_id!(
    /// Opaque identity of a block, independent of any Logseq UUID.
    BlockId
);
opaque_uuid_id!(
    /// Opaque identity of one atomic semantic operation batch.
    BatchId
);
opaque_uuid_id!(
    /// Opaque identity of an authoring device.
    DeviceId
);
opaque_uuid_id!(
    /// Opaque identity of one application session.
    SessionId
);
opaque_uuid_id!(
    /// Device-local identity of one canonical graph projection endpoint.
    ///
    /// Enrollment later binds this identity to one WorkspaceId, DeviceId, and
    /// canonical graph root. It is intentionally not derived from a portable
    /// path because receiver-local roots and formatting are not universal.
    ProjectionEndpointId
);
opaque_uuid_id!(
    /// Sharding-neutral identity of a causal document.
    DocumentId
);

/// Opaque, engine-neutral identity of a CRDT peer within a causal document.
///
/// The numeric representation is an interchange value, not a Loro peer type.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CrdtPeerId(u64);

impl CrdtPeerId {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Derive one deterministic candidate for the synthetic external-import
    /// author. Zero rejection, collision probing, and selection are deliberately
    /// left to the importer that owns the target CRDT document.
    pub(crate) fn external_import_candidate(
        workspace_id: WorkspaceId,
        import_id: ImportId,
        attempt: u64,
    ) -> Self {
        Self(derived_u64(
            b"tine/import/crdt-peer-id/v1\0",
            &[
                workspace_id.as_uuid().as_bytes(),
                import_id.as_bytes(),
                &attempt.to_be_bytes(),
            ],
        ))
    }
}

impl fmt::Display for CrdtPeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A syntactically valid Logseq UUID, kept distinct from Tine's internal BlockId.
///
/// Parsing accepts UUID syntax understood by the UUID library. Serialization is
/// always the canonical lower-case hyphenated representation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LogseqUuid(Uuid);

impl LogseqUuid {
    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for LogseqUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for LogseqUuid {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for LogseqUuid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LogseqUuid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// Deterministic identity of one external reconciliation transaction.
///
/// ImportId is a full SHA-256 digest so the inventory identity is not confused
/// with an ordinary randomly minted UUID.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ImportId([u8; 32]);

impl ImportId {
    pub(crate) const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Stable identity of one canonical graph-root filesystem resource.
///
/// The digest is derived only from a retained no-follow directory capability:
/// device/inode on Unix (including Android) and volume/file ID on Windows.
/// Ambient path strings never enter this identity, so moving or renaming the
/// graph preserves enrollment while substituting another directory does not.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalGraphResourceId([u8; 32]);

impl CanonicalGraphResourceId {
    pub(crate) fn from_capability_identity(platform: &[u8], identity: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/canonical-graph-resource/v1\0");
        hasher.update((platform.len() as u64).to_be_bytes());
        hasher.update(platform);
        hasher.update((identity.len() as u64).to_be_bytes());
        hasher.update(identity);
        Self(hasher.finalize().into())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CanonicalGraphResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CanonicalGraphResourceId({self})")
    }
}

impl fmt::Display for CanonicalGraphResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl FromStr for CanonicalGraphResourceId {
    type Err = DigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_digest(value).map(Self)
    }
}

impl Serialize for CanonicalGraphResourceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CanonicalGraphResourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

const ARCHIVE_INSTANCE_CLAIM_FILE: &str = "archive-instance-v1.claim";
const ARCHIVE_INSTANCE_CLAIM_SCHEMA_VERSION: u32 = 1;
const MAX_ARCHIVE_INSTANCE_CLAIM_BYTES: usize = 256;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchiveInstanceClaimV1 {
    schema_version: u32,
    instance_id: Uuid,
}

/// Stable identity of one claimed canonical device-local archive directory.
///
/// The digest binds a create-new random, versioned claim retained inside the
/// archive and the exact opened directory capability identity. Paths are not
/// evidence. Existing enrolled archives are opened only by checking their
/// persisted claim against the expected identity; absence never mints a new
/// claim.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalArchiveResourceId([u8; 32]);

impl CanonicalArchiveResourceId {
    fn from_claim_and_capability_identity(platform: &[u8], identity: &[u8], claim: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/canonical-archive-resource/v2\0");
        hasher.update((platform.len() as u64).to_be_bytes());
        hasher.update(platform);
        hasher.update((identity.len() as u64).to_be_bytes());
        hasher.update(identity);
        hasher.update((claim.len() as u64).to_be_bytes());
        hasher.update(claim);
        Self(hasher.finalize().into())
    }

    /// Provision one archive instance exactly once.
    ///
    /// If any claim artifact already exists, including a partial artifact from
    /// an interrupted provision, this fails without replacing or repairing it.
    pub(crate) fn provision_in_retained_directory(
        directory: &cap_std::fs::Dir,
    ) -> std::io::Result<Self> {
        let claim = ArchiveInstanceClaimV1 {
            schema_version: ARCHIVE_INSTANCE_CLAIM_SCHEMA_VERSION,
            instance_id: Uuid::new_v4(),
        };
        let bytes = serde_json::to_vec(&claim)
            .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
        let mut file = create_new_archive_claim(directory)?;
        validate_archive_claim_handle(&file)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        sync_dir_required(directory).map_err(|error| std::io::Error::other(error.to_string()))?;
        derive_archive_resource_id(directory, &bytes)
    }

    /// Open an already-enrolled archive using its exact persisted claim.
    ///
    /// Missing, incompatible, substituted, non-regular, or reparse claims fail
    /// closed. This function never provisions a replacement.
    pub(crate) fn open_enrolled_in_retained_directory(
        directory: &cap_std::fs::Dir,
        expected: Self,
    ) -> std::io::Result<Self> {
        let metadata = directory.symlink_metadata(ARCHIVE_INSTANCE_CLAIM_FILE)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || cap_metadata_is_windows_reparse(&metadata)
        {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "archive instance claim is not a regular no-follow file",
            ));
        }
        if metadata.len() > MAX_ARCHIVE_INSTANCE_CLAIM_BYTES as u64 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "archive instance claim exceeds its byte bound",
            ));
        }
        let file = open_archive_claim(directory)?;
        validate_archive_claim_handle(&file)?;
        let opened_len = file.metadata()?.len();
        if opened_len > MAX_ARCHIVE_INSTANCE_CLAIM_BYTES as u64 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "opened archive instance claim exceeds its byte bound",
            ));
        }
        let mut bytes = Vec::with_capacity(opened_len as usize);
        file.take((MAX_ARCHIVE_INSTANCE_CLAIM_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        let claim: ArchiveInstanceClaimV1 = serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
        if claim.schema_version != ARCHIVE_INSTANCE_CLAIM_SCHEMA_VERSION {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "unsupported archive instance claim schema",
            ));
        }
        let canonical = serde_json::to_vec(&claim)
            .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
        if canonical != bytes {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "archive instance claim is not canonical",
            ));
        }
        let found = derive_archive_resource_id(directory, &bytes)?;
        if found != expected {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "archive instance claim or capability identity was substituted",
            ));
        }
        Ok(found)
    }

    #[cfg(test)]
    pub(crate) fn from_capability_identity(platform: &[u8], identity: &[u8]) -> Self {
        Self::from_claim_and_capability_identity(platform, identity, b"test-only-claim")
    }

    #[cfg(test)]
    pub(crate) fn from_test_claim_and_capability_identity(
        platform: &[u8],
        identity: &[u8],
        claim: &[u8],
    ) -> Self {
        Self::from_claim_and_capability_identity(platform, identity, claim)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn derive_archive_resource_id(
    directory: &cap_std::fs::Dir,
    claim: &[u8],
) -> std::io::Result<CanonicalArchiveResourceId> {
    let (platform, identity) = archive_directory_capability_identity(directory)?;
    Ok(CanonicalArchiveResourceId::from_claim_and_capability_identity(platform, &identity, claim))
}

#[cfg(unix)]
fn archive_directory_capability_identity(
    directory: &cap_std::fs::Dir,
) -> std::io::Result<(&'static [u8], Vec<u8>)> {
    let metadata = directory.try_clone()?.into_std_file().metadata()?;
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "archive capability is not a directory",
        ));
    }
    let mut identity = Vec::with_capacity(16);
    identity.extend_from_slice(&metadata.dev().to_be_bytes());
    identity.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok((b"unix-dev-inode", identity))
}

#[cfg(windows)]
fn archive_directory_capability_identity(
    directory: &cap_std::fs::Dir,
) -> std::io::Result<(&'static [u8], Vec<u8>)> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ID_INFO,
    };

    let file = directory.try_clone()?.into_std_file();
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "archive capability is not an exact non-reparse directory",
        ));
    }
    let mut information = FILE_ID_INFO::default();
    // SAFETY: `file` remains live and `information` is correctly sized.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut identity = Vec::with_capacity(24);
    identity.extend_from_slice(&information.VolumeSerialNumber.to_be_bytes());
    identity.extend_from_slice(&information.FileId.Identifier);
    Ok((b"windows-volume-file-id", identity))
}

#[cfg(not(any(unix, windows)))]
fn archive_directory_capability_identity(
    _directory: &cap_std::fs::Dir,
) -> std::io::Result<(&'static [u8], Vec<u8>)> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "canonical archive directory identity is unavailable on this platform",
    ))
}

#[cfg(unix)]
fn create_new_archive_claim(directory: &cap_std::fs::Dir) -> std::io::Result<File> {
    openat_archive_claim(
        directory,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )
}

#[cfg(windows)]
fn create_new_archive_claim(directory: &cap_std::fs::Dir) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    Ok(directory
        .open_with(ARCHIVE_INSTANCE_CLAIM_FILE, &options)?
        .into_std())
}

#[cfg(not(any(unix, windows)))]
fn create_new_archive_claim(_directory: &cap_std::fs::Dir) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "archive claims are unsupported on this platform",
    ))
}

#[cfg(unix)]
fn open_archive_claim(directory: &cap_std::fs::Dir) -> std::io::Result<File> {
    openat_archive_claim(directory, libc::O_RDONLY, 0)
}

#[cfg(windows)]
fn open_archive_claim(directory: &cap_std::fs::Dir) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    Ok(directory
        .open_with(ARCHIVE_INSTANCE_CLAIM_FILE, &options)?
        .into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_archive_claim(_directory: &cap_std::fs::Dir) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "archive claims are unsupported on this platform",
    ))
}

#[cfg(unix)]
fn openat_archive_claim(
    directory: &cap_std::fs::Dir,
    flags: i32,
    mode: u32,
) -> std::io::Result<File> {
    let name = CString::new(ARCHIVE_INSTANCE_CLAIM_FILE).expect("static claim name has no NUL");
    // SAFETY: `name` is live and relative to the retained directory descriptor.
    let descriptor = unsafe {
        libc::openat(
            directory.as_fd().as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: openat returned one newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

fn validate_archive_claim_handle(file: &File) -> std::io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "opened archive claim is not a regular file",
        ));
    }
    #[cfg(unix)]
    if metadata.uid() !=
        // SAFETY: geteuid has no arguments or memory-safety preconditions.
        unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "opened archive claim has unsafe ownership or links",
        ));
    }
    #[cfg(windows)]
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "opened archive claim is a reparse point",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn cap_metadata_is_windows_reparse(metadata: &cap_std::fs::Metadata) -> bool {
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn cap_metadata_is_windows_reparse(_metadata: &cap_std::fs::Metadata) -> bool {
    false
}

impl fmt::Debug for CanonicalArchiveResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CanonicalArchiveResourceId({self})")
    }
}

impl fmt::Display for CanonicalArchiveResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl FromStr for CanonicalArchiveResourceId {
    type Err = DigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_digest(value).map(Self)
    }
}

impl Serialize for CanonicalArchiveResourceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CanonicalArchiveResourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// Stable identity of one projection-receipt directory capability.
///
/// Like the graph resource identity, this is derived from the opened directory
/// resource rather than its ambient pathname. It is also durably recorded in
/// the receipt-store claim, so another directory cannot copy an endpoint tuple
/// and become the engine's enrolled receipt authority.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectionReceiptStoreId([u8; 32]);

impl ProjectionReceiptStoreId {
    pub(crate) fn from_capability_identity(platform: &[u8], identity: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/projection-receipt-store-resource/v1\0");
        hasher.update((platform.len() as u64).to_be_bytes());
        hasher.update(platform);
        hasher.update((identity.len() as u64).to_be_bytes());
        hasher.update(identity);
        Self(hasher.finalize().into())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ProjectionReceiptStoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProjectionReceiptStoreId({self})")
    }
}

impl fmt::Display for ProjectionReceiptStoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl FromStr for ProjectionReceiptStoreId {
    type Err = DigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_digest(value).map(Self)
    }
}

impl Serialize for ProjectionReceiptStoreId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ProjectionReceiptStoreId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl fmt::Debug for ImportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImportId({self})")
    }
}

impl fmt::Display for ImportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl FromStr for ImportId {
    type Err = DigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_digest(value).map(Self)
    }
}

impl Serialize for ImportId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ImportId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl BatchId {
    /// Derive the sole batch identity for a deterministic external import.
    pub fn for_import(import_id: ImportId) -> Self {
        Self(derived_uuid(
            b"tine/import/batch-id/v1\0",
            &[import_id.as_bytes()],
        ))
    }
}

impl DocumentId {
    /// Derive the home document for one unmatched external page from its
    /// workspace and exact managed relative path.
    pub(crate) fn for_unmatched_import_page(
        workspace_id: WorkspaceId,
        managed_relative_path: &[u8],
    ) -> Self {
        Self(derived_uuid(
            b"tine/import/unmatched-page-home-document-id/v1\0",
            &[workspace_id.as_uuid().as_bytes(), managed_relative_path],
        ))
    }

    /// Derive the external-observation document for one import transaction.
    pub(crate) fn for_external_import_observation(
        workspace_id: WorkspaceId,
        import_id: ImportId,
    ) -> Self {
        Self(derived_uuid(
            b"tine/import/external-observation-document-id/v1\0",
            &[workspace_id.as_uuid().as_bytes(), import_id.as_bytes()],
        ))
    }
}

impl DeviceId {
    /// Derive the synthetic external author device for a workspace.
    pub(crate) fn for_external_import_author(workspace_id: WorkspaceId) -> Self {
        Self(derived_uuid(
            b"tine/import/external-author-device-id/v1\0",
            &[workspace_id.as_uuid().as_bytes()],
        ))
    }
}

impl SessionId {
    /// Derive the synthetic external author session for one import transaction.
    pub(crate) fn for_external_import_author(
        workspace_id: WorkspaceId,
        import_id: ImportId,
    ) -> Self {
        Self(derived_uuid(
            b"tine/import/external-author-session-id/v1\0",
            &[workspace_id.as_uuid().as_bytes(), import_id.as_bytes()],
        ))
    }
}

impl PageId {
    /// Derive the identity of an unmatched imported page.
    pub fn for_unmatched_import(import_id: ImportId, locator: &[u8]) -> Self {
        Self(derived_uuid(
            b"tine/import/unmatched-page-id/v1\0",
            &[import_id.as_bytes(), locator],
        ))
    }
}

impl BlockId {
    /// Derive the identity of an unmatched imported block.
    pub fn for_unmatched_import(import_id: ImportId, locator: &[u8]) -> Self {
        Self(derived_uuid(
            b"tine/import/unmatched-block-id/v1\0",
            &[import_id.as_bytes(), locator],
        ))
    }
}

fn derived_uuid(domain: &[u8], parts: &[&[u8]]) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // Mark deterministic application IDs as RFC 9562 UUIDv8 values.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn derived_u64(domain: &[u8], parts: &[&[u8]]) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DigestParseError;

impl fmt::Display for DigestParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected exactly 64 lower-case hexadecimal characters")
    }
}

impl std::error::Error for DigestParseError {}

pub(crate) fn parse_digest(value: &str) -> Result<[u8; 32], DigestParseError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(DigestParseError);
    }
    let mut result = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(result)
}

fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!("validated hexadecimal nibble"),
    }
}

pub(crate) fn write_hex(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        f.write_str(
            std::str::from_utf8(&[HEX[(byte >> 4) as usize], HEX[(byte & 0x0f) as usize]])
                .expect("hexadecimal is UTF-8"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(0x1020_3040_5060_7080_90a0_b0c0_d0e0_f001))
    }

    fn import_id() -> ImportId {
        ImportId::from_digest([0x5a; 32])
    }

    #[test]
    fn publisher_p1_external_import_derivations_are_deterministic_and_domain_separated() {
        let workspace = workspace();
        let import = import_id();
        let home = DocumentId::for_unmatched_import_page(workspace, b"pages/nested/naive.md");
        let device = DeviceId::for_external_import_author(workspace);
        let session = SessionId::for_external_import_author(workspace, import);
        let observation = DocumentId::for_external_import_observation(workspace, import);
        let peer = CrdtPeerId::external_import_candidate(workspace, import, 7);

        assert_eq!(
            home,
            DocumentId::for_unmatched_import_page(workspace, b"pages/nested/naive.md")
        );
        assert_eq!(device, DeviceId::for_external_import_author(workspace));
        assert_eq!(
            session,
            SessionId::for_external_import_author(workspace, import)
        );
        assert_eq!(
            observation,
            DocumentId::for_external_import_observation(workspace, import)
        );
        assert_eq!(
            peer,
            CrdtPeerId::external_import_candidate(workspace, import, 7)
        );

        let rendered = [
            home.to_string(),
            device.to_string(),
            session.to_string(),
            observation.to_string(),
            peer.to_string(),
        ];
        for (left_index, left) in rendered.iter().enumerate() {
            for right in rendered.iter().skip(left_index + 1) {
                assert_ne!(left, right, "derivation domains must remain separate");
            }
        }
        assert_ne!(
            peer,
            CrdtPeerId::external_import_candidate(workspace, import, 8)
        );

        assert_eq!(home.to_string(), "737b3bff-157d-8cfe-a3e8-be0ca069e2d6");
        assert_eq!(device.to_string(), "6c3c7276-a0d6-803e-9e0c-e24f8b58d13c");
        assert_eq!(session.to_string(), "5e69f6b5-0b83-8916-904c-36f09da566e1");
        assert_eq!(
            observation.to_string(),
            "54588e2e-938c-8f75-bc5c-f9ddbcf4ddb7"
        );
        assert_eq!(peer.as_u64(), 2_725_213_283_319_468_303);
    }

    #[test]
    fn canonical_archive_resource_identity_is_persistable_and_domain_separated() {
        let archive = CanonicalArchiveResourceId::from_capability_identity(
            b"test-platform",
            b"same-retained-directory-identity",
        );
        let graph = CanonicalGraphResourceId::from_capability_identity(
            b"test-platform",
            b"same-retained-directory-identity",
        );
        let receipt = ProjectionReceiptStoreId::from_capability_identity(
            b"test-platform",
            b"same-retained-directory-identity",
        );

        assert_ne!(archive.as_bytes(), graph.as_bytes());
        assert_ne!(archive.as_bytes(), receipt.as_bytes());
        assert_eq!(
            archive
                .to_string()
                .parse::<CanonicalArchiveResourceId>()
                .unwrap(),
            archive
        );
        assert_eq!(
            serde_json::from_slice::<CanonicalArchiveResourceId>(
                &serde_json::to_vec(&archive).unwrap()
            )
            .unwrap(),
            archive
        );
    }
}
