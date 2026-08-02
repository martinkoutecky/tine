//! Bounded physical packing for exact authenticated Patricia node bytes.
//!
//! Packing keeps Patricia roots and node encodings unchanged. Pack schema 1
//! has the historical 4,096-entry ceiling; current writers emit schema 2,
//! whose entry ceiling is derived from the unchanged hard byte bound. Both
//! schemas use the same canonical layout; the current decoder authenticates
//! both and remains backward-readable for schema 1.
//! Immutable packs are named by their complete-byte digest; one fixed,
//! atomically published head names one immutable content-addressed catalog.
//! Catalog schemas 1 and 2 remain backward-readable. Catalog schema 2 is an
//! oldest-to-newest layered sequence and may contain as many as 256 packs in
//! one derived byte-size tier. Schema 3 certifies at most four packs per tier.
//! Ordinary writes normalize old schema-2 catalogs incrementally by coalescing
//! one oldest group of five per tier, while schema-3 histories remain bounded.
//! Discovery never scans the node directory.
//!
//! The canonical byte layout is:
//!
//! ```text
//! magic[8] = "TINEPPK\0"
//! schema_version: u32 little endian = 1 or 2
//! entry_count: u32 little endian
//! payload_bytes: u32 little endian
//! entry[entry_count] = digest[32], payload_offset: u32 LE, length: u32 LE
//! payload[payload_bytes] = exact node bytes concatenated in digest order
//! ```
//!
//! Entries are non-empty, strictly digest-sorted, and densely laid out with no
//! gaps. Every entry digest is SHA-256 of its exact payload slice. The pack
//! filename is SHA-256 of the complete canonical pack bytes followed by
//! [`PACK_SUFFIX`]. The existing `-v1` suffix identifies this physical file
//! family, not the authenticated header schema, so it remains valid for both
//! schemas without changing exact naming or scan-free discovery. These rules
//! make retries byte-exact and allow readers to reject truncation,
//! non-canonical indexes, entry tampering, and path/content mismatch before
//! exposing any node bytes.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::fs;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::ops::Range;
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;

use cap_std::fs::{Dir, OpenOptions};
use fs2::FileExt as _;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::authenticated_patricia::{PatriciaNodePublisher, PatriciaPublicationError};
use super::content_digest::parse_digest;
use super::filesystem::{
    read_optional_regular, read_required_regular, transition_regular_exact, FilesystemError,
    StagedExactImmutablePublication, ValidatedDirectorySync,
};
use super::ContentDigest;

const PACK_MAGIC: &[u8; 8] = b"TINEPPK\0";
const LEGACY_PACK_SCHEMA_VERSION: u32 = 1;
const PACK_SCHEMA_VERSION: u32 = 2;
const PACK_HEADER_BYTES: usize = 8 + 4 + 4 + 4;
const PACK_INDEX_ENTRY_BYTES: usize = 32 + 4 + 4;
/// Physical family suffix retained across authenticated header schemas. The
/// complete-byte digest distinguishes schema-1 and schema-2 encodings.
pub(crate) const PACK_SUFFIX: &str = ".patricia-pack-v1";
const CATALOG_MAGIC: &[u8; 8] = b"TINEPCT\0";
const LEGACY_CATALOG_SCHEMA_VERSION: u32 = 1;
const LAYERED_CATALOG_SCHEMA_VERSION: u32 = 2;
const CATALOG_SCHEMA_VERSION: u32 = 3;
const CATALOG_HEADER_BYTES: usize = 8 + 4 + 4 + 4 + 8;
const CATALOG_ENTRY_BYTES: usize = 32 + 32 + 32 + 4 + 4;
const CATALOG_SUFFIX: &str = ".patricia-catalog-v1";
const HEAD_MAGIC: &[u8; 8] = b"TINEPHD\0";
const HEAD_SCHEMA_VERSION: u32 = 1;
pub(crate) const HEAD_BYTES: usize = 8 + 4 + 32 + 4;
pub(crate) const HEAD_FILENAME: &str = "patricia-pack-head-v1";
pub(crate) const OPERATION_LOCK_FILENAME: &str = "patricia-pack-operation-lock-v1";

const LEGACY_MAX_PACK_ENTRIES: usize = 4_096;
/// Schema 2 uses the byte bound, rather than an unrelated entry-count ceiling,
/// as the hard pack allocation bound. This permits compaction to coalesce packs
/// containing many tiny nodes instead of retaining an unbounded number of
/// entry-limited packs in one byte-size tier.
pub(crate) const MAX_PACK_ENTRIES: usize =
    (MAX_PACK_BYTES - PACK_HEADER_BYTES) / (PACK_INDEX_ENTRY_BYTES + 1);
pub(crate) const MAX_PACK_ENTRY_BYTES: usize = 128 * 1024;
pub(crate) const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;
/// Schema-3 catalog descriptors are grouped into power-of-two tiers derived
/// solely from authenticated pack bytes. Five packs in a tier are coalesced,
/// so a byte can advance through at most the fixed number of tiers admitted by
/// the hard pack and catalog limits.
const MAX_PACKS_PER_SIZE_TIER: usize = 4;
const MIN_PACK_BYTES: usize = PACK_HEADER_BYTES + PACK_INDEX_ENTRY_BYTES + 1;
const PACK_SIZE_TIER_COUNT: usize = (MAX_PACK_BYTES.ilog2() - MIN_PACK_BYTES.ilog2() + 1) as usize;
/// In schema-3 histories, before a carry a tier has at most four packs, each
/// less than twice the tier's lower bound. Charging those selected bytes to the
/// arriving carry at every possible tier gives this conservative lifetime
/// amortized bound. Schema-2 migration instead has a fixed per-mutation bound.
#[cfg(test)]
const AMORTIZED_SELECTED_BYTE_FACTOR: usize = 2 * MAX_PACKS_PER_SIZE_TIER * PACK_SIZE_TIER_COUNT;
pub(crate) const MAX_CATALOG_PACKS: usize = 256;
pub(crate) const MAX_CATALOG_PACK_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_CATALOG_BYTES: usize =
    CATALOG_HEADER_BYTES + MAX_CATALOG_PACKS * CATALOG_ENTRY_BYTES;

// Conservative allocator ownership charged for every BTreeMap entry retained
// by construction packing. The largest inline pair is digest (32 bytes) plus
// Vec (24 bytes); payload capacity is charged separately. On the pinned 64-bit
// target, a B-tree node at minimum occupancy plus allocator metadata remains
// below 128 bytes per pair. The 256-byte charge also covers digest/Range maps
// with more than 2x layout headroom.
const PACKED_MAP_ENTRY_OWNERSHIP_BYTES: usize = 256;
const CONSTRUCTION_STREAM_BUFFER_BYTES: usize = 64 * 1024;
const CONSTRUCTION_STREAM_BUFFER_COUNT: usize = 4;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static RECLAMATION_DIRECTORY_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static NEXT_HEAD_TRANSITION_FAILURE: std::cell::Cell<Option<(usize, HeadTransitionFailureForTest)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
pub enum HeadTransitionFailureForTest {
    Before,
    After,
}

#[cfg(any(test, feature = "test-support"))]
pub fn fail_next_head_transition_for_test(failure: HeadTransitionFailureForTest) {
    fail_head_transition_after_for_test(0, failure);
}

#[cfg(any(test, feature = "test-support"))]
pub fn fail_head_transition_after_for_test(
    successful_transitions: usize,
    failure: HeadTransitionFailureForTest,
) {
    NEXT_HEAD_TRANSITION_FAILURE.with(|next| next.set(Some((successful_transitions, failure))));
}

#[cfg(test)]
pub(crate) fn reclamation_directory_scans() -> usize {
    RECLAMATION_DIRECTORY_SCANS.with(std::cell::Cell::get)
}

#[derive(Debug)]
pub(crate) enum PackedPatriciaError {
    Filesystem(FilesystemError),
    Publication(PatriciaPublicationError),
    Empty,
    TooManyEntries,
    EntryTooLarge(ContentDigest),
    PackTooLarge,
    PathMismatch(ContentDigest),
    UnexpectedHead,
    Malformed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PackedPatriciaReclamationReport {
    pub(crate) examined_files: usize,
    pub(crate) examined_bytes: u64,
    pub(crate) deleted_files: usize,
    pub(crate) deleted_bytes: u64,
    pub(crate) retained_files: usize,
    pub(crate) retained_bytes: u64,
}

pub(crate) enum PackedPatriciaReclamationError {
    Busy,
    Packed(PackedPatriciaError),
}

pub(crate) struct PackedOperationalGuard {
    file: fs::File,
}

impl Drop for PackedOperationalGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn open_operational_lock(dir: &Dir) -> Result<fs::File, PackedPatriciaError> {
    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    let file = match dir.open_with(OPERATION_LOCK_FILENAME, &create) {
        Ok(file) => {
            let file = file.into_std();
            if !file.metadata()?.is_file() {
                return Err(FilesystemError::UnsafeEntry(format!(
                    "{OPERATION_LOCK_FILENAME} is not a regular file"
                ))
                .into());
            }
            file.sync_all()?;
            ValidatedDirectorySync::open(dir)?.sync()?;
            file
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let mut open = OpenOptions::new();
            open.read(true).write(true);
            let file = dir.open_with(OPERATION_LOCK_FILENAME, &open)?.into_std();
            if !file.metadata()?.is_file() {
                return Err(FilesystemError::UnsafeEntry(format!(
                    "{OPERATION_LOCK_FILENAME} is not a regular file"
                ))
                .into());
            }
            file
        }
        Err(error) => return Err(error.into()),
    };
    Ok(file)
}

pub(crate) fn lock_packed_operation_shared(
    dir: &Dir,
) -> Result<PackedOperationalGuard, PackedPatriciaError> {
    let file = open_operational_lock(dir)?;
    fs2::FileExt::lock_shared(&file)?;
    Ok(PackedOperationalGuard { file })
}

fn try_lock_packed_operation_exclusive(
    dir: &Dir,
) -> Result<Option<PackedOperationalGuard>, PackedPatriciaError> {
    let file = open_operational_lock(dir)?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(PackedOperationalGuard { file })),
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error.into()),
    }
}

impl From<FilesystemError> for PackedPatriciaError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error)
    }
}

impl From<io::Error> for PackedPatriciaError {
    fn from(error: io::Error) -> Self {
        Self::Filesystem(error.into())
    }
}

/// Canonical bytes ready for one immutable exact publication.
pub(crate) struct PackedPatriciaPublication {
    digest: ContentDigest,
    bytes: Vec<u8>,
    first: ContentDigest,
    last: ContentDigest,
    entries: u32,
    ranges: BTreeMap<ContentDigest, Range<usize>>,
}

impl PackedPatriciaPublication {
    pub(crate) fn build(
        entries: &BTreeMap<ContentDigest, Vec<u8>>,
    ) -> Result<Self, PackedPatriciaError> {
        if entries.is_empty() {
            return Err(PackedPatriciaError::Empty);
        }
        if entries.len() > MAX_PACK_ENTRIES {
            return Err(PackedPatriciaError::TooManyEntries);
        }

        let index_bytes = entries
            .len()
            .checked_mul(PACK_INDEX_ENTRY_BYTES)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let payload_bytes = entries.iter().try_fold(0_usize, |total, (digest, bytes)| {
            if bytes.is_empty() || bytes.len() > MAX_PACK_ENTRY_BYTES {
                return Err(PackedPatriciaError::EntryTooLarge(*digest));
            }
            total
                .checked_add(bytes.len())
                .ok_or(PackedPatriciaError::PackTooLarge)
        })?;
        let total_bytes = PACK_HEADER_BYTES
            .checked_add(index_bytes)
            .and_then(|bytes| bytes.checked_add(payload_bytes))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if total_bytes > MAX_PACK_BYTES || payload_bytes > u32::MAX as usize {
            return Err(PackedPatriciaError::PackTooLarge);
        }

        let mut encoded = Vec::with_capacity(total_bytes);
        encoded.extend_from_slice(PACK_MAGIC);
        encoded.extend_from_slice(&PACK_SCHEMA_VERSION.to_le_bytes());
        encoded.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(payload_bytes as u32).to_le_bytes());

        let mut offset = 0_usize;
        for (digest, bytes) in entries {
            debug_assert_eq!(*digest, ContentDigest::of(bytes));
            if *digest != ContentDigest::of(bytes) {
                return Err(PackedPatriciaError::PathMismatch(*digest));
            }
            encoded.extend_from_slice(digest.as_bytes());
            encoded.extend_from_slice(&(offset as u32).to_le_bytes());
            encoded.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            offset += bytes.len();
        }
        let payload_start = encoded.len();
        for bytes in entries.values() {
            encoded.extend_from_slice(bytes);
        }
        debug_assert_eq!(encoded.len(), total_bytes);

        let mut ranges = BTreeMap::new();
        let mut offset = 0_usize;
        for (digest, bytes) in entries {
            ranges.insert(
                *digest,
                payload_start + offset..payload_start + offset + bytes.len(),
            );
            offset += bytes.len();
        }

        Ok(Self {
            digest: ContentDigest::of(&encoded),
            bytes: encoded,
            first: *entries.first_key_value().expect("validated non-empty").0,
            last: *entries.last_key_value().expect("validated non-empty").0,
            entries: entries.len() as u32,
            ranges,
        })
    }

    #[cfg(test)]
    pub(crate) const fn digest(&self) -> ContentDigest {
        self.digest
    }

    pub(crate) fn filename(&self) -> String {
        pack_filename(self.digest)
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns evidence only after the policy-owned exact publisher reports
    /// success. A later catalog/head API can require this evidence, preventing
    /// it from naming a pack that has not completed atomic publication.
    pub(crate) fn publish(
        &self,
        dir: &Dir,
        publisher: &dyn PatriciaNodePublisher,
    ) -> Result<PublishedPatriciaPack, PackedPatriciaError> {
        publisher
            .publish(dir, &self.filename(), &self.bytes)
            .map_err(PackedPatriciaError::Publication)?;
        Ok(PublishedPatriciaPack {
            digest: self.digest,
            first: self.first,
            last: self.last,
            entries: self.entries,
            bytes: self.bytes.len() as u32,
        })
    }

    fn into_opened(self) -> Result<PackedPatriciaPack, PackedPatriciaError> {
        PackedPatriciaPack::decode(self.digest, self.bytes)
    }

    fn descriptor(&self) -> PackDescriptor {
        PackDescriptor {
            digest: self.digest,
            first: self.first,
            last: self.last,
            entries: self.entries,
            bytes: self.bytes.len() as u32,
        }
    }
}

/// Non-forgeable package-local evidence of completed pack publication.
pub(crate) struct PublishedPatriciaPack {
    digest: ContentDigest,
    first: ContentDigest,
    last: ContentDigest,
    entries: u32,
    bytes: u32,
}

impl PublishedPatriciaPack {
    #[cfg(test)]
    pub(crate) const fn digest(&self) -> ContentDigest {
        self.digest
    }

    fn descriptor(&self) -> PackDescriptor {
        PackDescriptor {
            digest: self.digest,
            first: self.first,
            last: self.last,
            entries: self.entries,
            bytes: self.bytes,
        }
    }
}

/// One fully validated, bounded pack reopened from immutable storage.
pub(crate) struct PackedPatriciaPack {
    digest: ContentDigest,
    bytes: Vec<u8>,
    entry_count: usize,
    payload_start: usize,
}

impl PackedPatriciaPack {
    pub(crate) fn open(
        dir: &Dir,
        expected_digest: ContentDigest,
    ) -> Result<Self, PackedPatriciaError> {
        let bytes = read_required_regular(
            dir,
            &pack_filename(expected_digest),
            MAX_PACK_BYTES as u64,
            None,
        )?;
        Self::decode(expected_digest, bytes)
    }

    pub(crate) fn decode(
        expected_digest: ContentDigest,
        bytes: Vec<u8>,
    ) -> Result<Self, PackedPatriciaError> {
        if bytes.len() > MAX_PACK_BYTES || ContentDigest::of(&bytes) != expected_digest {
            return Err(PackedPatriciaError::PathMismatch(expected_digest));
        }
        if bytes.len() < PACK_HEADER_BYTES || &bytes[..8] != PACK_MAGIC {
            return Err(PackedPatriciaError::Malformed);
        }

        let schema_version = read_u32(&bytes, 8)?;
        let entry_count = read_u32(&bytes, 12)? as usize;
        let payload_bytes = read_u32(&bytes, 16)? as usize;
        let max_entries = match schema_version {
            LEGACY_PACK_SCHEMA_VERSION => LEGACY_MAX_PACK_ENTRIES,
            PACK_SCHEMA_VERSION => MAX_PACK_ENTRIES,
            _ => return Err(PackedPatriciaError::Malformed),
        };
        if entry_count == 0 || entry_count > max_entries {
            return Err(PackedPatriciaError::Malformed);
        }
        let index_bytes = entry_count
            .checked_mul(PACK_INDEX_ENTRY_BYTES)
            .ok_or(PackedPatriciaError::Malformed)?;
        let payload_start = PACK_HEADER_BYTES
            .checked_add(index_bytes)
            .ok_or(PackedPatriciaError::Malformed)?;
        let expected_length = payload_start
            .checked_add(payload_bytes)
            .ok_or(PackedPatriciaError::Malformed)?;
        if expected_length != bytes.len() {
            return Err(PackedPatriciaError::Malformed);
        }

        let mut expected_offset = 0_usize;
        let mut prior_digest = None;
        for index in 0..entry_count {
            let start = PACK_HEADER_BYTES + index * PACK_INDEX_ENTRY_BYTES;
            let mut digest_bytes = [0_u8; 32];
            digest_bytes.copy_from_slice(&bytes[start..start + 32]);
            let digest = ContentDigest::from_bytes(digest_bytes);
            let offset = read_u32(&bytes, start + 32)? as usize;
            let length = read_u32(&bytes, start + 36)? as usize;
            if prior_digest.is_some_and(|prior| prior >= digest)
                || offset != expected_offset
                || length == 0
                || length > MAX_PACK_ENTRY_BYTES
            {
                return Err(PackedPatriciaError::Malformed);
            }
            let end = offset
                .checked_add(length)
                .ok_or(PackedPatriciaError::Malformed)?;
            if end > payload_bytes {
                return Err(PackedPatriciaError::Malformed);
            }
            let range = payload_start + offset..payload_start + end;
            if ContentDigest::of(&bytes[range.clone()]) != digest {
                return Err(PackedPatriciaError::PathMismatch(digest));
            }
            prior_digest = Some(digest);
            expected_offset = end;
        }
        if expected_offset != payload_bytes {
            return Err(PackedPatriciaError::Malformed);
        }

        Ok(Self {
            digest: expected_digest,
            bytes,
            entry_count,
            payload_start,
        })
    }

    pub(crate) fn get(&self, digest: ContentDigest) -> Option<&[u8]> {
        let mut low = 0_usize;
        let mut high = self.entry_count;
        while low < high {
            let middle = low + (high - low) / 2;
            let (found, range) = self.entry(middle);
            match found.cmp(&digest) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(&self.bytes[range]),
            }
        }
        None
    }

    fn entry(&self, index: usize) -> (ContentDigest, Range<usize>) {
        let start = PACK_HEADER_BYTES + index * PACK_INDEX_ENTRY_BYTES;
        let mut digest_bytes = [0_u8; 32];
        digest_bytes.copy_from_slice(&self.bytes[start..start + 32]);
        let offset = u32::from_le_bytes(
            self.bytes[start + 32..start + 36]
                .try_into()
                .expect("validated pack offset"),
        ) as usize;
        let length = u32::from_le_bytes(
            self.bytes[start + 36..start + 40]
                .try_into()
                .expect("validated pack length"),
        ) as usize;
        (
            ContentDigest::from_bytes(digest_bytes),
            self.payload_start + offset..self.payload_start + offset + length,
        )
    }

    fn iter_entries(&self) -> impl Iterator<Item = (ContentDigest, Range<usize>)> + '_ {
        (0..self.entry_count).map(|index| self.entry(index))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entry_count
    }

    fn descriptor(&self) -> PackDescriptor {
        PackDescriptor {
            digest: self.digest,
            first: self.entry(0).0,
            last: self.entry(self.entry_count - 1).0,
            entries: self.entry_count as u32,
            bytes: self.bytes.len() as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PackDescriptor {
    digest: ContentDigest,
    first: ContentDigest,
    last: ContentDigest,
    entries: u32,
    bytes: u32,
}

/// Canonical immutable catalog bytes constructed only from successful pack
/// publication evidence. Schemas 2 and 3 preserve descriptor order as
/// oldest-to-newest layers. Ranges may overlap, but duplicate node digests must
/// resolve to exact identical bytes when the catalog is authenticated.
pub(crate) struct PackedPatriciaCatalogPublication {
    digest: ContentDigest,
    bytes: Vec<u8>,
}

impl PackedPatriciaCatalogPublication {
    #[cfg(test)]
    pub(crate) fn build(packs: &[PublishedPatriciaPack]) -> Result<Self, PackedPatriciaError> {
        if packs.is_empty() {
            return Err(PackedPatriciaError::Empty);
        }
        if packs.len() > MAX_CATALOG_PACKS {
            return Err(PackedPatriciaError::TooManyEntries);
        }
        let mut descriptors = packs
            .iter()
            .map(|pack| PackDescriptor {
                digest: pack.digest,
                first: pack.first,
                last: pack.last,
                entries: pack.entries,
                bytes: pack.bytes,
            })
            .collect::<Vec<_>>();
        descriptors.sort_unstable_by_key(|descriptor| descriptor.first);
        validate_descriptors(&descriptors, true)?;
        Self::build_descriptors_for_schema(&descriptors, CATALOG_SCHEMA_VERSION)
    }

    fn build_descriptors(descriptors: &[PackDescriptor]) -> Result<Self, PackedPatriciaError> {
        let schema_version = if size_tiers_are_bounded(descriptors) {
            CATALOG_SCHEMA_VERSION
        } else {
            LAYERED_CATALOG_SCHEMA_VERSION
        };
        Self::build_descriptors_for_schema(descriptors, schema_version)
    }

    fn build_descriptors_for_schema(
        descriptors: &[PackDescriptor],
        schema_version: u32,
    ) -> Result<Self, PackedPatriciaError> {
        if descriptors.is_empty() {
            return Err(PackedPatriciaError::Empty);
        }
        if descriptors.len() > MAX_CATALOG_PACKS {
            return Err(PackedPatriciaError::TooManyEntries);
        }
        if !matches!(
            schema_version,
            LEGACY_CATALOG_SCHEMA_VERSION | LAYERED_CATALOG_SCHEMA_VERSION | CATALOG_SCHEMA_VERSION
        ) {
            return Err(PackedPatriciaError::Malformed);
        }
        validate_descriptors(descriptors, schema_version == LEGACY_CATALOG_SCHEMA_VERSION)?;
        if schema_version == CATALOG_SCHEMA_VERSION && !size_tiers_are_bounded(descriptors) {
            return Err(PackedPatriciaError::Malformed);
        }
        let total_entries = descriptors.iter().try_fold(0_u32, |total, descriptor| {
            total
                .checked_add(descriptor.entries)
                .ok_or(PackedPatriciaError::TooManyEntries)
        })?;
        let total_pack_bytes = descriptors.iter().try_fold(0_u64, |total, descriptor| {
            total
                .checked_add(u64::from(descriptor.bytes))
                .ok_or(PackedPatriciaError::PackTooLarge)
        })?;
        if total_pack_bytes > MAX_CATALOG_PACK_BYTES as u64 {
            return Err(PackedPatriciaError::PackTooLarge);
        }

        let mut bytes =
            Vec::with_capacity(CATALOG_HEADER_BYTES + descriptors.len() * CATALOG_ENTRY_BYTES);
        bytes.extend_from_slice(CATALOG_MAGIC);
        bytes.extend_from_slice(&schema_version.to_le_bytes());
        bytes.extend_from_slice(&(descriptors.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&total_entries.to_le_bytes());
        bytes.extend_from_slice(&total_pack_bytes.to_le_bytes());
        for descriptor in descriptors {
            bytes.extend_from_slice(descriptor.digest.as_bytes());
            bytes.extend_from_slice(descriptor.first.as_bytes());
            bytes.extend_from_slice(descriptor.last.as_bytes());
            bytes.extend_from_slice(&descriptor.entries.to_le_bytes());
            bytes.extend_from_slice(&descriptor.bytes.to_le_bytes());
        }
        debug_assert!(bytes.len() <= MAX_CATALOG_BYTES);
        Ok(Self {
            digest: ContentDigest::of(&bytes),
            bytes,
        })
    }

    pub(crate) fn publish(
        &self,
        dir: &Dir,
        publisher: &dyn PatriciaNodePublisher,
    ) -> Result<PublishedPatriciaCatalog, PackedPatriciaError> {
        publisher
            .publish(dir, &catalog_filename(self.digest), &self.bytes)
            .map_err(PackedPatriciaError::Publication)?;
        Ok(PublishedPatriciaCatalog {
            digest: self.digest,
            bytes: self.bytes.len() as u32,
        })
    }
}

/// Non-forgeable package-local evidence of completed catalog publication.
pub(crate) struct PublishedPatriciaCatalog {
    digest: ContentDigest,
    bytes: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PackedPatriciaPublicationWork {
    /// New exact node payload bytes considered for this catalog transition.
    pub(crate) new_payload_bytes: usize,
    /// Historical bytes compared only for a matching content digest. This is
    /// bounded by the new payload, never by the lifetime catalog payload.
    pub(crate) existing_payload_bytes_compared: usize,
    /// Canonical bytes of the newly arrived delta before any tier carries.
    pub(crate) delta_pack_bytes_encoded: usize,
    /// Canonical input-pack bytes selected by size-tier compaction. A pack is
    /// counted again only if a carry reaches another derived tier. Schema-3
    /// histories have the documented lifetime amortized bound; schema-2
    /// normalization selects at most five packs per derived tier per mutation.
    pub(crate) compaction_pack_bytes_selected: usize,
    /// Exact count corresponding to `compaction_pack_bytes_selected`.
    pub(crate) compaction_packs_selected: usize,
    /// Exact node payload bytes copied while coalescing selected tiers.
    pub(crate) compaction_payload_bytes_reencoded: usize,
    pub(crate) pack_bytes_encoded: usize,
    pub(crate) pack_bytes_published: usize,
    pub(crate) catalog_metadata_bytes_encoded: usize,
    pub(crate) catalog_metadata_bytes_published: usize,
    pub(crate) packs_published: usize,
    /// Complete conservative construction residency observed by the bounded
    /// planner, including ownership retained by its caller and resolver.
    pub(crate) peak_resident_bytes: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct PackedPatriciaResidencyBudget {
    /// Ownership which remains live in the construction caller while the
    /// packed planner runs: staged nodes, record/traversal state, and sink.
    pub(crate) retained_bytes: usize,
    pub(crate) maximum_bytes: usize,
}

struct PackedPatriciaResidencyTracker {
    retained_bytes: usize,
    maximum_bytes: usize,
    peak_bytes: usize,
}

impl PackedPatriciaResidencyTracker {
    fn new(
        budget: PackedPatriciaResidencyBudget,
        resolver_bytes: usize,
    ) -> Result<Self, PackedPatriciaError> {
        let retained_bytes = budget
            .retained_bytes
            .checked_add(resolver_bytes)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if retained_bytes > budget.maximum_bytes {
            return Err(PackedPatriciaError::PackTooLarge);
        }
        Ok(Self {
            retained_bytes,
            maximum_bytes: budget.maximum_bytes,
            peak_bytes: retained_bytes,
        })
    }

    fn observe(&mut self, planner_bytes: usize) -> Result<(), PackedPatriciaError> {
        let resident_bytes = self
            .retained_bytes
            .checked_add(planner_bytes)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        self.peak_bytes = self.peak_bytes.max(resident_bytes);
        if resident_bytes > self.maximum_bytes {
            return Err(PackedPatriciaError::PackTooLarge);
        }
        Ok(())
    }
}

fn exact_entries_owned_bytes(entries: &BTreeMap<ContentDigest, Vec<u8>>) -> usize {
    entries
        .values()
        .fold(0_usize, |total, bytes| {
            total.saturating_add(bytes.capacity())
        })
        .saturating_add(
            entries
                .len()
                .saturating_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES),
        )
}

fn publication_owned_bytes(publication: &PackedPatriciaPublication) -> usize {
    publication.bytes.capacity().saturating_add(
        publication
            .ranges
            .len()
            .saturating_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES),
    )
}

fn planned_owned_bytes(planned: &Vec<PlannedPatriciaPack>) -> usize {
    planned
        .capacity()
        .saturating_mul(std::mem::size_of::<PlannedPatriciaPack>())
        .saturating_add(planned.iter().fold(0_usize, |total, pack| {
            total.saturating_add(match pack {
                PlannedPatriciaPack::Existing { .. } => 0,
                PlannedPatriciaPack::Publication(publication) => {
                    publication_owned_bytes(publication)
                }
            })
        }))
}

fn partition_conservative_scratch_bytes(
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
) -> Result<usize, PackedPatriciaError> {
    let mut completed_publication_bytes = 0_usize;
    let mut chunk_entries = 0_usize;
    let mut chunk_payload_bytes = 0_usize;
    let mut peak_bytes = 0_usize;
    let mut publication_count = 0_usize;
    for bytes in entries.values() {
        let candidate_entries = chunk_entries
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
        let candidate_bytes = PACK_HEADER_BYTES
            .checked_add(
                candidate_entries
                    .checked_mul(PACK_INDEX_ENTRY_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .and_then(|total| total.checked_add(chunk_payload_bytes))
            .and_then(|total| total.checked_add(bytes.len()))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if chunk_entries != 0
            && (candidate_entries > MAX_PACK_ENTRIES || candidate_bytes > MAX_PACK_BYTES)
        {
            let completed_chunk_owned = chunk_payload_bytes
                .checked_add(
                    chunk_entries
                        .checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)
                        .ok_or(PackedPatriciaError::PackTooLarge)?,
                )
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            let encoded = PACK_HEADER_BYTES
                .checked_add(chunk_entries * PACK_INDEX_ENTRY_BYTES)
                .and_then(|total| total.checked_add(chunk_payload_bytes))
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            completed_publication_bytes = completed_publication_bytes
                .checked_add(encoded)
                .and_then(|total| {
                    total.checked_add(chunk_entries.checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)?)
                })
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            publication_count = publication_count
                .checked_add(1)
                .ok_or(PackedPatriciaError::TooManyEntries)?;
            peak_bytes = peak_bytes.max(
                completed_publication_bytes
                    .checked_add(completed_chunk_owned)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            );
            chunk_entries = 0;
            chunk_payload_bytes = 0;
        }
        chunk_entries = chunk_entries
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
        chunk_payload_bytes = chunk_payload_bytes
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let chunk_owned = chunk_payload_bytes
            .checked_add(
                chunk_entries
                    .checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        peak_bytes = peak_bytes.max(
            completed_publication_bytes
                .checked_add(chunk_owned)
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        );
    }
    if chunk_entries != 0 {
        let encoded = PACK_HEADER_BYTES
            .checked_add(chunk_entries * PACK_INDEX_ENTRY_BYTES)
            .and_then(|total| total.checked_add(chunk_payload_bytes))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        completed_publication_bytes = completed_publication_bytes
            .checked_add(encoded)
            .and_then(|total| {
                total.checked_add(chunk_entries.checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)?)
            })
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        publication_count = publication_count
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
        let chunk_owned = chunk_payload_bytes
            .checked_add(
                chunk_entries
                    .checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        peak_bytes = peak_bytes.max(
            completed_publication_bytes
                .checked_add(chunk_owned)
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        );
    }
    peak_bytes
        .checked_add(
            publication_count
                .checked_mul(std::mem::size_of::<PackedPatriciaPublication>())
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        )
        .ok_or(PackedPatriciaError::PackTooLarge)
}

/// Mutation-bounded exact-node input for construction publication.
///
/// Patricia construction produces records child-before-parent, while the
/// existing pack schema is canonical in digest order. The sink retains that
/// arrival order only for a pre-publication loose fallback and owns at most one
/// catalog-bounded canonical delta. No pack, catalog, or head is touched while
/// records are accepted.
pub(crate) struct PackedPatriciaConstructionSink {
    entries: BTreeMap<ContentDigest, Vec<u8>>,
    child_before_parent: Vec<ContentDigest>,
    payload_bytes: usize,
    payload_capacity_bytes: usize,
    owned_byte_limit: usize,
}

impl PackedPatriciaConstructionSink {
    pub(crate) fn new(owned_byte_limit: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            child_before_parent: Vec::new(),
            payload_bytes: 0,
            payload_capacity_bytes: 0,
            owned_byte_limit,
        }
    }

    /// Accept one exact node without performing physical publication.
    /// `Ok(false)` is a capacity refusal, not malformed input.
    pub(crate) fn accept(
        &mut self,
        digest: ContentDigest,
        bytes: Vec<u8>,
    ) -> Result<bool, PackedPatriciaError> {
        if bytes.is_empty() || bytes.len() > MAX_PACK_ENTRY_BYTES {
            return Err(PackedPatriciaError::EntryTooLarge(digest));
        }
        if ContentDigest::of(&bytes) != digest {
            return Err(PackedPatriciaError::PathMismatch(digest));
        }
        if let Some(existing) = self.entries.get(&digest) {
            if existing != &bytes {
                return Err(PackedPatriciaError::PathMismatch(digest));
            }
            return Ok(true);
        }
        let next_entries = self
            .entries
            .len()
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
        let next_payload = self
            .payload_bytes
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let next_payload_capacity = self
            .payload_capacity_bytes
            .checked_add(bytes.capacity())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let encoded_bytes = PACK_HEADER_BYTES
            .checked_add(
                next_entries
                    .checked_mul(PACK_INDEX_ENTRY_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .and_then(|total| total.checked_add(next_payload))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if encoded_bytes > MAX_CATALOG_PACK_BYTES {
            return Ok(false);
        }
        let projected_owned_bytes = next_payload_capacity
            .checked_add(
                next_entries
                    .checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .and_then(|owned| {
                self.child_before_parent
                    .capacity()
                    .max(next_entries.saturating_mul(2))
                    .checked_mul(std::mem::size_of::<ContentDigest>())
                    .and_then(|order| owned.checked_add(order))
            })
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if projected_owned_bytes > self.owned_byte_limit {
            return Ok(false);
        }
        self.payload_bytes = next_payload;
        self.payload_capacity_bytes = next_payload_capacity;
        self.child_before_parent.push(digest);
        self.entries.insert(digest, bytes);
        Ok(true)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn entries(&self) -> &BTreeMap<ContentDigest, Vec<u8>> {
        &self.entries
    }

    pub(crate) fn child_before_parent(&self) -> &[ContentDigest] {
        &self.child_before_parent
    }

    /// Conservative owned residency including map/vector allocation slack.
    pub(crate) fn owned_bytes(&self) -> usize {
        self.payload_capacity_bytes
            .saturating_add(
                self.entries
                    .len()
                    .saturating_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES),
            )
            .saturating_add(
                self.child_before_parent
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ContentDigest>()),
            )
    }
}

pub(crate) struct PendingPackedPatriciaCatalog {
    catalog: PublishedPatriciaCatalog,
    descriptors: Vec<PackDescriptor>,
    packs: Vec<PendingPatriciaPack>,
}

/// Construction publication needs only the published catalog authority. A
/// successful head CAS invalidates the old resident resolver and later reads
/// reopen the authenticated catalog instead of retaining replacement packs.
pub(crate) struct PendingStreamingPackedPatriciaCatalog {
    catalog: PublishedPatriciaCatalog,
}

impl PendingStreamingPackedPatriciaCatalog {
    pub(crate) fn published_catalog(&self) -> &PublishedPatriciaCatalog {
        &self.catalog
    }
}

enum PendingPatriciaPack {
    Existing(usize),
    Published(PackedPatriciaPack),
}

impl PendingPackedPatriciaCatalog {
    pub(crate) fn published_catalog(&self) -> &PublishedPatriciaCatalog {
        &self.catalog
    }

    /// Install the already-published final tier layout in memory only after the
    /// fixed head transition succeeds. Unselected historical pack payloads are
    /// moved, not cloned or reopened.
    pub(crate) fn finish(self, current: Option<PackedPatriciaCatalog>) -> PackedPatriciaCatalog {
        let mut existing = current
            .map(|catalog| catalog.packs.into_iter().map(Some).collect::<Vec<_>>())
            .unwrap_or_default();
        let packs = self
            .packs
            .into_iter()
            .map(|pack| match pack {
                PendingPatriciaPack::Existing(index) => existing
                    .get_mut(index)
                    .and_then(Option::take)
                    .expect("pending catalog retains each historical pack at most once"),
                PendingPatriciaPack::Published(pack) => pack,
            })
            .collect();
        PackedPatriciaCatalog {
            authority: PackedPatriciaHeadAuthority {
                catalog_digest: self.catalog.digest,
                catalog_bytes: self.catalog.bytes,
            },
            descriptors: self.descriptors,
            packs,
        }
    }
}

enum PlannedPatriciaPack {
    Existing {
        index: usize,
        descriptor: PackDescriptor,
    },
    Publication(PackedPatriciaPublication),
}

enum PlannedStreamingPatriciaPack {
    Existing {
        resolver_index: usize,
        descriptor: PackDescriptor,
    },
    Staged {
        publication: StagedExactImmutablePublication,
        descriptor: PackDescriptor,
    },
}

impl PlannedStreamingPatriciaPack {
    fn descriptor(&self) -> PackDescriptor {
        match self {
            Self::Existing { descriptor, .. } | Self::Staged { descriptor, .. } => *descriptor,
        }
    }
}

impl PlannedPatriciaPack {
    fn descriptor(&self) -> PackDescriptor {
        match self {
            Self::Existing { descriptor, .. } => *descriptor,
            Self::Publication(publication) => publication.descriptor(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PackedPatriciaHeadAuthority {
    catalog_digest: ContentDigest,
    catalog_bytes: u32,
}

impl PackedPatriciaHeadAuthority {
    fn encode(self) -> Vec<u8> {
        encode_catalog_head(self.catalog_digest, self.catalog_bytes)
    }
}

/// Publish the initial fixed discovery head. Exact immutable publication makes
/// a retry of the same catalog idempotent and rejects a competing authority.
#[cfg(test)]
pub(crate) fn publish_catalog_head(
    dir: &Dir,
    catalog: &PublishedPatriciaCatalog,
    publisher: &dyn PatriciaNodePublisher,
) -> Result<(), PackedPatriciaError> {
    let bytes = encode_catalog_head(catalog.digest, catalog.bytes);
    publisher
        .publish(dir, HEAD_FILENAME, &bytes)
        .map_err(PackedPatriciaError::Publication)
}

/// Transition the fixed head under the caller's existing single-writer lease.
/// Immutable prerequisites are represented by publication evidence, and an
/// authenticated prior head is the only replaceable authority. Routine
/// publication retains superseded immutable files; only explicit maintenance
/// under the exclusive operational guard reclaims them.
pub(crate) fn transition_catalog_head(
    dir: &Dir,
    expected: Option<PackedPatriciaHeadAuthority>,
    catalog: &PublishedPatriciaCatalog,
) -> Result<(), PackedPatriciaError> {
    #[cfg(any(test, feature = "test-support"))]
    let injected = NEXT_HEAD_TRANSITION_FAILURE.with(|next| match next.take() {
        Some((0, failure)) => Some(failure),
        Some((remaining, failure)) => {
            next.set(Some((remaining - 1, failure)));
            None
        }
        None => None,
    });
    #[cfg(any(test, feature = "test-support"))]
    if matches!(injected, Some(HeadTransitionFailureForTest::Before)) {
        return Err(PackedPatriciaError::Filesystem(FilesystemError::Io(
            io::Error::other("injected failure before packed head transition"),
        )));
    }
    let expected = expected.map(PackedPatriciaHeadAuthority::encode);
    let replacement = encode_catalog_head(catalog.digest, catalog.bytes);
    transition_regular_exact(dir, HEAD_FILENAME, expected.as_deref(), &replacement).map_err(
        |error| match error {
            FilesystemError::ByteCollision => PackedPatriciaError::UnexpectedHead,
            error => PackedPatriciaError::Filesystem(error),
        },
    )?;
    #[cfg(any(test, feature = "test-support"))]
    if matches!(injected, Some(HeadTransitionFailureForTest::After)) {
        return Err(PackedPatriciaError::Filesystem(FilesystemError::Io(
            io::Error::other("injected failure after packed head transition"),
        )));
    }
    Ok(())
}

pub(crate) fn reclaim_unreachable_packed_files(
    dir: &Dir,
) -> Result<PackedPatriciaReclamationReport, PackedPatriciaReclamationError> {
    let Some(_guard) =
        try_lock_packed_operation_exclusive(dir).map_err(PackedPatriciaReclamationError::Packed)?
    else {
        return Err(PackedPatriciaReclamationError::Busy);
    };

    // Authenticate the complete authority before even opening the directory
    // iterator. A malformed head, catalog, or named pack therefore cannot
    // authorize any deletion.
    let current = PackedPatriciaCatalog::discover_under_guard(dir)
        .map_err(PackedPatriciaReclamationError::Packed)?
        .ok_or(PackedPatriciaReclamationError::Packed(
            PackedPatriciaError::Malformed,
        ))?;
    let live = current.live_filenames();
    let durability = ValidatedDirectorySync::open(dir)
        .and_then(|sync| {
            sync.preflight()?;
            Ok(sync)
        })
        .map_err(|error| {
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error.into()))
        })?;

    let mut report = PackedPatriciaReclamationReport::default();
    let mut deletions = Vec::new();
    #[cfg(test)]
    RECLAMATION_DIRECTORY_SCANS.with(|scans| scans.set(scans.get().saturating_add(1)));
    let entries = dir.entries().map_err(|error| {
        PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error.into()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error.into()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error.into()))
        })?;
        if !file_type.is_file() {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| {
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error.into()))
        })?;
        let length = metadata.len();
        report.examined_files = report.examined_files.saturating_add(1);
        report.examined_bytes = report.examined_bytes.saturating_add(length);
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_reclaimable_packed_name(name) && !live.contains(name) {
            deletions.push((name.to_owned(), length));
        }
    }

    for (name, length) in deletions {
        if let Err(error) = remove_reclamation_file(dir, &name) {
            if report.deleted_files != 0 {
                durability.sync().map_err(|sync_error| {
                    PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(
                        sync_error.into(),
                    ))
                })?;
            }
            return Err(PackedPatriciaReclamationError::Packed(
                PackedPatriciaError::Filesystem(error.into()),
            ));
        }
        report.deleted_files = report.deleted_files.saturating_add(1);
        report.deleted_bytes = report.deleted_bytes.saturating_add(length);
    }
    if report.deleted_files != 0 {
        durability.sync().map_err(|error| {
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error.into()))
        })?;
    }
    report.retained_files = report.examined_files.saturating_sub(report.deleted_files);
    report.retained_bytes = report.examined_bytes.saturating_sub(report.deleted_bytes);
    Ok(report)
}

fn is_reclaimable_packed_name(name: &str) -> bool {
    digest_from_filename(name, PACK_SUFFIX).is_some()
        || digest_from_filename(name, CATALOG_SUFFIX).is_some()
        || is_publication_temp_name(name)
}

fn digest_from_filename(name: &str, suffix: &str) -> Option<ContentDigest> {
    let digest = name.strip_suffix(suffix)?;
    parse_digest(digest).ok().map(ContentDigest::from_bytes)
}

fn is_publication_temp_name(name: &str) -> bool {
    let Some(uuid) = name.strip_prefix(".tmp-") else {
        return false;
    };
    Uuid::parse_str(uuid)
        .ok()
        .is_some_and(|parsed| parsed.to_string() == uuid)
}

#[cfg(not(windows))]
fn remove_reclamation_file(dir: &Dir, name: &str) -> io::Result<()> {
    dir.remove_file(name)
}

#[cfg(windows)]
fn remove_reclamation_file(dir: &Dir, name: &str) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};

    const ATTEMPTS: usize = 4;
    for attempt in 0..ATTEMPTS {
        match dir.remove_file(name) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < ATTEMPTS
                    && matches!(
                        error.raw_os_error(),
                        Some(code)
                            if code == ERROR_SHARING_VIOLATION as i32
                                || code == ERROR_ACCESS_DENIED as i32
                    ) =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded deletion retry returns on every final attempt")
}

/// Fully authenticated catalog snapshot used by the Patricia adapter. All
/// named packs are opened and checked once before any cataloged node is
/// exposed, so an absent or corrupt non-target pack invalidates the authority.
pub(crate) struct PackedPatriciaCatalog {
    authority: PackedPatriciaHeadAuthority,
    descriptors: Vec<PackDescriptor>,
    packs: Vec<PackedPatriciaPack>,
}

impl PackedPatriciaCatalog {
    pub(crate) fn discover(dir: &Dir) -> Result<Option<Self>, PackedPatriciaError> {
        let _guard = lock_packed_operation_shared(dir)?;
        Self::discover_under_guard(dir)
    }

    pub(crate) fn discover_under_guard(dir: &Dir) -> Result<Option<Self>, PackedPatriciaError> {
        let Some(head) = read_optional_regular(
            dir,
            HEAD_FILENAME,
            HEAD_BYTES as u64,
            Some(HEAD_BYTES as u64),
        )?
        else {
            return Ok(None);
        };
        if &head[..8] != HEAD_MAGIC || read_u32(&head, 8)? != HEAD_SCHEMA_VERSION {
            return Err(PackedPatriciaError::Malformed);
        }
        let authority = PackedPatriciaHeadAuthority {
            catalog_digest: read_digest(&head, 12)?,
            catalog_bytes: read_u32(&head, 44)?,
        };
        let catalog_digest = authority.catalog_digest;
        let catalog_bytes = authority.catalog_bytes as usize;
        if !(CATALOG_HEADER_BYTES..=MAX_CATALOG_BYTES).contains(&catalog_bytes) {
            return Err(PackedPatriciaError::Malformed);
        }
        let bytes = read_required_regular(
            dir,
            &catalog_filename(catalog_digest),
            MAX_CATALOG_BYTES as u64,
            Some(catalog_bytes as u64),
        )?;
        if ContentDigest::of(&bytes) != catalog_digest {
            return Err(PackedPatriciaError::PathMismatch(catalog_digest));
        }
        Self::open_catalog(dir, authority, bytes).map(Some)
    }

    /// Discover an existing resolver only when its complete conservative open
    /// peak fits the caller's remaining construction residency. Refusal occurs
    /// after bounded head/catalog authentication but before the first pack is
    /// opened and before any immutable publication.
    pub(crate) fn discover_under_guard_bounded(
        dir: &Dir,
        maximum_resolver_bytes: usize,
    ) -> Result<(Option<Self>, usize), PackedPatriciaError> {
        let Some(head) = read_optional_regular(
            dir,
            HEAD_FILENAME,
            HEAD_BYTES as u64,
            Some(HEAD_BYTES as u64),
        )?
        else {
            return Ok((None, 0));
        };
        if &head[..8] != HEAD_MAGIC || read_u32(&head, 8)? != HEAD_SCHEMA_VERSION {
            return Err(PackedPatriciaError::Malformed);
        }
        let authority = PackedPatriciaHeadAuthority {
            catalog_digest: read_digest(&head, 12)?,
            catalog_bytes: read_u32(&head, 44)?,
        };
        let catalog_digest = authority.catalog_digest;
        let catalog_bytes = authority.catalog_bytes as usize;
        if !(CATALOG_HEADER_BYTES..=MAX_CATALOG_BYTES).contains(&catalog_bytes) {
            return Err(PackedPatriciaError::Malformed);
        }
        let bytes = read_required_regular(
            dir,
            &catalog_filename(catalog_digest),
            MAX_CATALOG_BYTES as u64,
            Some(catalog_bytes as u64),
        )?;
        if ContentDigest::of(&bytes) != catalog_digest {
            return Err(PackedPatriciaError::PathMismatch(catalog_digest));
        }
        let descriptors = decode_catalog(&bytes)?;
        let resolver_open_peak = head
            .len()
            .checked_mul(2)
            .and_then(|total| total.checked_add(bytes.len().checked_mul(2)?))
            .and_then(|total| {
                total.checked_add(
                    descriptors
                        .capacity()
                        .checked_mul(std::mem::size_of::<PackDescriptor>())?,
                )
            })
            .and_then(|total| {
                total.checked_add(
                    descriptors
                        .len()
                        .checked_mul(std::mem::size_of::<PackedPatriciaPack>())?,
                )
            })
            .and_then(|total| {
                descriptors.iter().try_fold(total, |resident, descriptor| {
                    resident.checked_add((descriptor.bytes as usize).checked_mul(2)?)
                })
            })
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if resolver_open_peak > maximum_resolver_bytes {
            return Err(PackedPatriciaError::PackTooLarge);
        }
        drop(descriptors);
        let catalog = Self::open_catalog(dir, authority, bytes)?;
        if catalog.resident_bytes() > maximum_resolver_bytes {
            return Err(PackedPatriciaError::Malformed);
        }
        Ok((Some(catalog), resolver_open_peak))
    }

    #[cfg(test)]
    pub(crate) fn open_published(
        dir: &Dir,
        catalog: &PublishedPatriciaCatalog,
    ) -> Result<Self, PackedPatriciaError> {
        let authority = PackedPatriciaHeadAuthority {
            catalog_digest: catalog.digest,
            catalog_bytes: catalog.bytes,
        };
        let bytes = read_required_regular(
            dir,
            &catalog_filename(catalog.digest),
            MAX_CATALOG_BYTES as u64,
            Some(catalog.bytes as u64),
        )?;
        if ContentDigest::of(&bytes) != catalog.digest {
            return Err(PackedPatriciaError::PathMismatch(catalog.digest));
        }
        Self::open_catalog(dir, authority, bytes)
    }

    fn open_catalog(
        dir: &Dir,
        authority: PackedPatriciaHeadAuthority,
        bytes: Vec<u8>,
    ) -> Result<Self, PackedPatriciaError> {
        let descriptors = decode_catalog(&bytes)?;
        let mut packs = Vec::with_capacity(descriptors.len());
        for expected in &descriptors {
            let pack = PackedPatriciaPack::open(dir, expected.digest)?;
            if pack.descriptor() != *expected {
                return Err(PackedPatriciaError::Malformed);
            }
            packs.push(pack);
        }
        validate_duplicate_nodes(&packs)?;
        Ok(Self {
            authority,
            descriptors,
            packs,
        })
    }

    pub(crate) const fn authority(&self) -> PackedPatriciaHeadAuthority {
        self.authority
    }

    pub(crate) fn live_filenames(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::from([
            HEAD_FILENAME.to_owned(),
            OPERATION_LOCK_FILENAME.to_owned(),
            catalog_filename(self.authority.catalog_digest),
        ]);
        names.extend(
            self.descriptors
                .iter()
                .map(|descriptor| pack_filename(descriptor.digest)),
        );
        names
    }

    pub(crate) fn get(&self, digest: ContentDigest) -> Option<&[u8]> {
        // Newest layers usually contain recently authored paths. Duplicate
        // digests are safe because authenticated open checked exact equality.
        for (descriptor, pack) in self.descriptors.iter().zip(&self.packs).rev() {
            if descriptor.first <= digest && digest <= descriptor.last {
                if let Some(bytes) = pack.get(digest) {
                    return Some(bytes);
                }
            }
        }
        None
    }

    /// Conservative owned bytes retained by the authenticated resolver.
    pub(crate) fn resident_bytes(&self) -> usize {
        self.descriptors
            .capacity()
            .saturating_mul(std::mem::size_of::<PackDescriptor>())
            .saturating_add(
                self.packs
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PackedPatriciaPack>()),
            )
            .saturating_add(self.packs.iter().fold(0_usize, |total, pack| {
                total.saturating_add(pack.bytes.capacity())
            }))
    }

    #[cfg(test)]
    fn pack_count(&self) -> usize {
        self.packs.len()
    }
}

/// Corrupts the packed bytes that currently contain `node_digest` while
/// retaining the immutable pack's content-addressed filename. This exists only
/// for crate tests or explicit test-support builds so adapter callers can
/// prove that reopen rejects existing path/content mismatches; ordinary
/// production builds have no corruption surface.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn corrupt_packed_node_for_test(
    dir: &Dir,
    node_digest: ContentDigest,
) -> Result<(), PackedPatriciaError> {
    let catalog = PackedPatriciaCatalog::discover(dir)?.ok_or(PackedPatriciaError::Malformed)?;
    let (descriptor, pack, range) = catalog
        .descriptors
        .iter()
        .zip(&catalog.packs)
        .find_map(|(descriptor, pack)| {
            pack.get(node_digest).map(|bytes| {
                let start = bytes.as_ptr() as usize - pack.bytes.as_ptr() as usize;
                (descriptor, pack, start..start + bytes.len())
            })
        })
        .ok_or(PackedPatriciaError::Malformed)?;
    let mut corrupted = pack.bytes.clone();
    corrupted[range.start] ^= 0x01;
    dir.write(pack_filename(descriptor.digest), corrupted)
        .map_err(FilesystemError::Io)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn publish_partitioned_catalog(
    dir: &Dir,
    publisher: &dyn PatriciaNodePublisher,
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
    catalog_pack_byte_limit: usize,
) -> Result<(PublishedPatriciaCatalog, PackedPatriciaCatalog), PackedPatriciaError> {
    let publications = partition_publications(entries)?;
    let total_pack_bytes = publications
        .iter()
        .try_fold(0_usize, |total, publication| {
            total
                .checked_add(publication.bytes.len())
                .ok_or(PackedPatriciaError::PackTooLarge)
        })?;
    if publications.len() > MAX_CATALOG_PACKS
        || total_pack_bytes > catalog_pack_byte_limit.min(MAX_CATALOG_PACK_BYTES)
    {
        return Err(PackedPatriciaError::PackTooLarge);
    }

    let packs = publications
        .iter()
        .map(|publication| publication.publish(dir, publisher))
        .collect::<Result<Vec<_>, _>>()?;
    let catalog = PackedPatriciaCatalogPublication::build(&packs)?;
    let catalog = catalog.publish(dir, publisher)?;
    let opened = PackedPatriciaCatalog::open_published(dir, &catalog)?;
    Ok((catalog, opened))
}

/// Publish new delta packs, coalescing only overflowing comparable-size tiers,
/// and then publish one bounded metadata catalog. Capacity is checked before
/// the first immutable publication so the caller can safely use its frozen
/// loose-node fallback without changing the catalog head.
pub(crate) fn publish_appended_catalog(
    dir: &Dir,
    publisher: &dyn PatriciaNodePublisher,
    current: Option<&PackedPatriciaCatalog>,
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
    catalog_pack_byte_limit: usize,
) -> Result<
    (
        Option<PendingPackedPatriciaCatalog>,
        PackedPatriciaPublicationWork,
    ),
    PackedPatriciaError,
> {
    publish_appended_catalog_with_residency(
        dir,
        publisher,
        current,
        entries,
        catalog_pack_byte_limit,
        None,
    )
}

fn publish_appended_catalog_with_residency(
    dir: &Dir,
    publisher: &dyn PatriciaNodePublisher,
    current: Option<&PackedPatriciaCatalog>,
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
    catalog_pack_byte_limit: usize,
    residency_budget: Option<PackedPatriciaResidencyBudget>,
) -> Result<
    (
        Option<PendingPackedPatriciaCatalog>,
        PackedPatriciaPublicationWork,
    ),
    PackedPatriciaError,
> {
    if entries.is_empty() {
        return Err(PackedPatriciaError::Empty);
    }

    let mut work = PackedPatriciaPublicationWork::default();
    let mut residency = residency_budget
        .map(|budget| {
            PackedPatriciaResidencyTracker::new(
                budget,
                current.map_or(0, PackedPatriciaCatalog::resident_bytes),
            )
        })
        .transpose()?;
    let mut delta = BTreeMap::new();
    for (digest, bytes) in entries {
        if bytes.is_empty() || bytes.len() > MAX_PACK_ENTRY_BYTES {
            return Err(PackedPatriciaError::EntryTooLarge(*digest));
        }
        if ContentDigest::of(bytes) != *digest {
            return Err(PackedPatriciaError::PathMismatch(*digest));
        }
        if let Some(existing) = current.and_then(|catalog| catalog.get(*digest)) {
            work.existing_payload_bytes_compared = work
                .existing_payload_bytes_compared
                .checked_add(existing.len())
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            if existing != bytes {
                return Err(PackedPatriciaError::PathMismatch(*digest));
            }
        } else {
            work.new_payload_bytes = work
                .new_payload_bytes
                .checked_add(bytes.len())
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            if let Some(residency) = residency.as_mut() {
                let projected_delta_bytes = exact_entries_owned_bytes(&delta)
                    .checked_add(bytes.len())
                    .and_then(|total| total.checked_add(PACKED_MAP_ENTRY_OWNERSHIP_BYTES))
                    .ok_or(PackedPatriciaError::PackTooLarge)?;
                residency.observe(projected_delta_bytes)?;
            }
            delta.insert(*digest, bytes.clone());
        }
    }
    if delta.is_empty() {
        if let Some(residency) = residency {
            work.peak_resident_bytes = residency.peak_bytes;
        }
        return Ok((None, work));
    }

    let delta_owned_bytes = exact_entries_owned_bytes(&delta);
    if let Some(residency) = residency.as_mut() {
        residency.observe(
            delta_owned_bytes
                .checked_add(partition_conservative_scratch_bytes(&delta)?)
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        )?;
    }
    let publications = partition_publications(&delta)?;
    let new_pack_bytes = publications
        .iter()
        .try_fold(0_usize, |total, publication| {
            total
                .checked_add(publication.bytes.len())
                .ok_or(PackedPatriciaError::PackTooLarge)
        })?;
    work.delta_pack_bytes_encoded = new_pack_bytes;
    work.pack_bytes_encoded = new_pack_bytes;

    let mut planned = current
        .map(|catalog| {
            catalog
                .descriptors
                .iter()
                .copied()
                .enumerate()
                .map(|(index, descriptor)| PlannedPatriciaPack::Existing { index, descriptor })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    planned.extend(
        publications
            .into_iter()
            .map(PlannedPatriciaPack::Publication),
    );
    if let Some(residency) = residency.as_mut() {
        residency.observe(
            delta_owned_bytes
                .checked_add(planned_owned_bytes(&planned))
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        )?;
    }
    compact_size_tiers(
        current,
        &mut planned,
        &mut work,
        residency.as_mut(),
        delta_owned_bytes,
    )?;

    let descriptors = planned
        .iter()
        .map(PlannedPatriciaPack::descriptor)
        .collect::<Vec<_>>();
    let final_pack_bytes = descriptors.iter().try_fold(0_usize, |total, descriptor| {
        total
            .checked_add(descriptor.bytes as usize)
            .ok_or(PackedPatriciaError::PackTooLarge)
    })?;
    if descriptors.len() > MAX_CATALOG_PACKS
        || final_pack_bytes > catalog_pack_byte_limit.min(MAX_CATALOG_PACK_BYTES)
    {
        return Err(PackedPatriciaError::PackTooLarge);
    }
    let input_was_tier_bounded = current
        .map(|catalog| size_tiers_are_bounded(&catalog.descriptors))
        .unwrap_or(true);
    if input_was_tier_bounded && !size_tiers_are_bounded(&descriptors) {
        return Err(PackedPatriciaError::PackTooLarge);
    }
    let catalog_publication = if input_was_tier_bounded {
        PackedPatriciaCatalogPublication::build_descriptors_for_schema(
            &descriptors,
            CATALOG_SCHEMA_VERSION,
        )?
    } else {
        PackedPatriciaCatalogPublication::build_descriptors(&descriptors)?
    };
    work.catalog_metadata_bytes_encoded = catalog_publication.bytes.len();

    if let Some(residency) = residency.as_mut() {
        let descriptor_bytes = descriptors
            .capacity()
            .checked_mul(std::mem::size_of::<PackDescriptor>())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let pending_pack_vector_bytes = planned
            .len()
            .checked_mul(std::mem::size_of::<PendingPatriciaPack>())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        // This is checked before the first pack publication and covers both
        // the current planning phase and the later pending-catalog/head phase.
        // Publication bytes move into opened replacement packs; they are not
        // counted twice, while the old and replacement Vec allocations are.
        let prepublication_bytes = delta_owned_bytes
            .checked_add(planned_owned_bytes(&planned))
            .and_then(|total| total.checked_add(descriptor_bytes))
            .and_then(|total| total.checked_add(catalog_publication.bytes.capacity()))
            .and_then(|total| total.checked_add(pending_pack_vector_bytes))
            .and_then(|total| total.checked_add(2 * HEAD_BYTES))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        residency.observe(prepublication_bytes)?;
    }

    for pack in &planned {
        if let PlannedPatriciaPack::Publication(publication) = pack {
            let published = publication.publish(dir, publisher)?;
            if published.descriptor() != publication.descriptor() {
                return Err(PackedPatriciaError::Malformed);
            }
            work.pack_bytes_published = work
                .pack_bytes_published
                .checked_add(publication.bytes.len())
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            work.packs_published += 1;
        }
    }
    let catalog = catalog_publication.publish(dir, publisher)?;
    work.catalog_metadata_bytes_published = catalog.bytes as usize;
    let packs = planned
        .into_iter()
        .map(|pack| match pack {
            PlannedPatriciaPack::Existing { index, .. } => Ok(PendingPatriciaPack::Existing(index)),
            PlannedPatriciaPack::Publication(publication) => publication
                .into_opened()
                .map(PendingPatriciaPack::Published),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(residency) = residency {
        work.peak_resident_bytes = residency.peak_bytes;
    }
    Ok((
        Some(PendingPackedPatriciaCatalog {
            catalog,
            descriptors,
            packs,
        }),
        work,
    ))
}

#[derive(Clone, Copy)]
struct StreamPartitionShape {
    start_ordinal: usize,
    entries: usize,
    payload_bytes: usize,
    first: ContentDigest,
    last: ContentDigest,
}

impl StreamPartitionShape {
    fn total_bytes(self) -> Result<usize, PackedPatriciaError> {
        PACK_HEADER_BYTES
            .checked_add(
                self.entries
                    .checked_mul(PACK_INDEX_ENTRY_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .and_then(|total| total.checked_add(self.payload_bytes))
            .ok_or(PackedPatriciaError::PackTooLarge)
    }
}

fn push_stream_shape(
    shapes: &mut Vec<StreamPartitionShape>,
    start_ordinal: usize,
    entries: usize,
    payload_bytes: usize,
    first: Option<ContentDigest>,
    last: Option<ContentDigest>,
) {
    if entries != 0 {
        shapes.push(StreamPartitionShape {
            start_ordinal,
            entries,
            payload_bytes,
            first: first.expect("non-empty stream partition has a first digest"),
            last: last.expect("non-empty stream partition has a last digest"),
        });
    }
}

fn plan_delta_stream(
    current: Option<&PackedPatriciaCatalog>,
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
    work: &mut PackedPatriciaPublicationWork,
) -> Result<Vec<StreamPartitionShape>, PackedPatriciaError> {
    let mut shapes = Vec::new();
    let mut start_ordinal = 0_usize;
    let mut unique_ordinal = 0_usize;
    let mut chunk_entries = 0_usize;
    let mut chunk_payload = 0_usize;
    let mut first = None;
    let mut last = None;
    for (digest, bytes) in entries {
        if bytes.is_empty() || bytes.len() > MAX_PACK_ENTRY_BYTES {
            return Err(PackedPatriciaError::EntryTooLarge(*digest));
        }
        if ContentDigest::of(bytes) != *digest {
            return Err(PackedPatriciaError::PathMismatch(*digest));
        }
        if let Some(existing) = current.and_then(|catalog| catalog.get(*digest)) {
            work.existing_payload_bytes_compared = work
                .existing_payload_bytes_compared
                .checked_add(existing.len())
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            if existing != bytes {
                return Err(PackedPatriciaError::PathMismatch(*digest));
            }
            continue;
        }
        work.new_payload_bytes = work
            .new_payload_bytes
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let candidate_entries = chunk_entries
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
        let candidate_total = PACK_HEADER_BYTES
            .checked_add(
                candidate_entries
                    .checked_mul(PACK_INDEX_ENTRY_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .and_then(|total| total.checked_add(chunk_payload))
            .and_then(|total| total.checked_add(bytes.len()))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if chunk_entries != 0
            && (candidate_entries > MAX_PACK_ENTRIES || candidate_total > MAX_PACK_BYTES)
        {
            push_stream_shape(
                &mut shapes,
                start_ordinal,
                chunk_entries,
                chunk_payload,
                first,
                last,
            );
            start_ordinal = unique_ordinal;
            chunk_entries = 0;
            chunk_payload = 0;
            first = None;
        }
        chunk_entries = chunk_entries
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
        chunk_payload = chunk_payload
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        first.get_or_insert(*digest);
        last = Some(*digest);
        unique_ordinal = unique_ordinal
            .checked_add(1)
            .ok_or(PackedPatriciaError::TooManyEntries)?;
    }
    push_stream_shape(
        &mut shapes,
        start_ordinal,
        chunk_entries,
        chunk_payload,
        first,
        last,
    );
    Ok(shapes)
}

fn write_stream_header(
    file: &mut fs::File,
    shape: StreamPartitionShape,
) -> Result<(), PackedPatriciaError> {
    file.seek(SeekFrom::Start(0))?;
    file.write_all(PACK_MAGIC)?;
    file.write_all(&PACK_SCHEMA_VERSION.to_le_bytes())?;
    file.write_all(
        &u32::try_from(shape.entries)
            .map_err(|_| PackedPatriciaError::TooManyEntries)?
            .to_le_bytes(),
    )?;
    file.write_all(
        &u32::try_from(shape.payload_bytes)
            .map_err(|_| PackedPatriciaError::PackTooLarge)?
            .to_le_bytes(),
    )?;
    Ok(())
}

fn hash_stream_prefix(
    file: &mut fs::File,
    prefix_bytes: usize,
    hasher: &mut Sha256,
) -> Result<(), PackedPatriciaError> {
    file.seek(SeekFrom::Start(0))?;
    let mut remaining = prefix_bytes;
    let mut buffer = [0_u8; CONSTRUCTION_STREAM_BUFFER_BYTES];
    while remaining != 0 {
        let chunk = remaining.min(buffer.len());
        file.read_exact(&mut buffer[..chunk])?;
        hasher.update(&buffer[..chunk]);
        remaining -= chunk;
    }
    Ok(())
}

fn stage_delta_partition(
    dir: &Dir,
    current: Option<&PackedPatriciaCatalog>,
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
    shape: StreamPartitionShape,
) -> Result<PlannedStreamingPatriciaPack, PackedPatriciaError> {
    let total_bytes = shape.total_bytes()?;
    let end_ordinal = shape
        .start_ordinal
        .checked_add(shape.entries)
        .ok_or(PackedPatriciaError::TooManyEntries)?;
    let mut staged_digest = None;
    let publication = StagedExactImmutablePublication::construct(dir, |file| {
        write_stream_header(file, shape).map_err(packed_io_error)?;
        let mut ordinal = 0_usize;
        let mut payload_offset = 0_usize;
        for (digest, bytes) in entries {
            if current.and_then(|catalog| catalog.get(*digest)).is_some() {
                continue;
            }
            if (shape.start_ordinal..end_ordinal).contains(&ordinal) {
                file.write_all(digest.as_bytes())?;
                file.write_all(
                    &u32::try_from(payload_offset)
                        .map_err(|_| io::Error::from(ErrorKind::InvalidData))?
                        .to_le_bytes(),
                )?;
                file.write_all(
                    &u32::try_from(bytes.len())
                        .map_err(|_| io::Error::from(ErrorKind::InvalidData))?
                        .to_le_bytes(),
                )?;
                payload_offset += bytes.len();
            }
            ordinal += 1;
        }
        if payload_offset != shape.payload_bytes {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        let payload_start = PACK_HEADER_BYTES + shape.entries * PACK_INDEX_ENTRY_BYTES;
        let mut hasher = Sha256::new();
        hash_stream_prefix(file, payload_start, &mut hasher).map_err(packed_io_error)?;
        file.seek(SeekFrom::Start(payload_start as u64))?;
        ordinal = 0;
        for (digest, bytes) in entries {
            if current.and_then(|catalog| catalog.get(*digest)).is_some() {
                continue;
            }
            if (shape.start_ordinal..end_ordinal).contains(&ordinal) {
                if ContentDigest::of(bytes) != *digest {
                    return Err(io::Error::from(ErrorKind::InvalidData));
                }
                file.write_all(bytes)?;
                hasher.update(bytes);
            }
            ordinal += 1;
        }
        let digest = ContentDigest::from_bytes(hasher.finalize().into());
        staged_digest = Some(digest);
        Ok((pack_filename(digest), total_bytes as u64))
    })?;
    let digest = staged_digest.ok_or(PackedPatriciaError::Malformed)?;
    Ok(PlannedStreamingPatriciaPack::Staged {
        publication,
        descriptor: PackDescriptor {
            digest,
            first: shape.first,
            last: shape.last,
            entries: shape.entries as u32,
            bytes: total_bytes as u32,
        },
    })
}

fn packed_io_error(error: PackedPatriciaError) -> io::Error {
    match error {
        PackedPatriciaError::Filesystem(FilesystemError::Io(error)) => error,
        _ => io::Error::from(ErrorKind::InvalidData),
    }
}

#[derive(Clone, Copy)]
struct StreamCursorEntry {
    digest: ContentDigest,
    payload_offset: usize,
    length: usize,
}

fn validate_staged_stream_pack(
    publication: &StagedExactImmutablePublication,
    descriptor: PackDescriptor,
) -> Result<(), PackedPatriciaError> {
    let mut file = publication.open_staged()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; CONSTRUCTION_STREAM_BUFFER_BYTES];
    let mut total = 0_usize;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        hasher.update(&buffer[..read]);
    }
    if total != descriptor.bytes as usize
        || ContentDigest::from_bytes(hasher.finalize().into()) != descriptor.digest
    {
        return Err(PackedPatriciaError::PathMismatch(descriptor.digest));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0_u8; PACK_HEADER_BYTES];
    file.read_exact(&mut header)?;
    if &header[..8] != PACK_MAGIC
        || u32::from_le_bytes(header[8..12].try_into().expect("fixed schema"))
            != PACK_SCHEMA_VERSION
        || u32::from_le_bytes(header[12..16].try_into().expect("fixed entries"))
            != descriptor.entries
    {
        return Err(PackedPatriciaError::Malformed);
    }
    let payload_bytes =
        u32::from_le_bytes(header[16..20].try_into().expect("fixed payload")) as usize;
    let payload_start = PACK_HEADER_BYTES
        .checked_add(
            (descriptor.entries as usize)
                .checked_mul(PACK_INDEX_ENTRY_BYTES)
                .ok_or(PackedPatriciaError::Malformed)?,
        )
        .ok_or(PackedPatriciaError::Malformed)?;
    if payload_start
        .checked_add(payload_bytes)
        .is_none_or(|length| length != descriptor.bytes as usize)
    {
        return Err(PackedPatriciaError::Malformed);
    }
    let mut prior = None;
    let mut expected_offset = 0_usize;
    for index in 0..descriptor.entries as usize {
        let mut encoded = [0_u8; PACK_INDEX_ENTRY_BYTES];
        file.seek(SeekFrom::Start(
            (PACK_HEADER_BYTES + index * PACK_INDEX_ENTRY_BYTES) as u64,
        ))?;
        file.read_exact(&mut encoded)?;
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&encoded[..32]);
        let digest = ContentDigest::from_bytes(digest);
        let offset = u32::from_le_bytes(encoded[32..36].try_into().expect("fixed offset")) as usize;
        let length = u32::from_le_bytes(encoded[36..40].try_into().expect("fixed length")) as usize;
        if prior.is_some_and(|prior| prior >= digest)
            || offset != expected_offset
            || length == 0
            || length > MAX_PACK_ENTRY_BYTES
            || offset
                .checked_add(length)
                .is_none_or(|end| end > payload_bytes)
        {
            return Err(PackedPatriciaError::Malformed);
        }
        let mut payload_hasher = Sha256::new();
        let mut remaining = length;
        let mut payload_offset = payload_start + offset;
        while remaining != 0 {
            let chunk = remaining.min(buffer.len());
            file.seek(SeekFrom::Start(payload_offset as u64))?;
            file.read_exact(&mut buffer[..chunk])?;
            payload_hasher.update(&buffer[..chunk]);
            remaining -= chunk;
            payload_offset += chunk;
        }
        if ContentDigest::from_bytes(payload_hasher.finalize().into()) != digest {
            return Err(PackedPatriciaError::PathMismatch(digest));
        }
        prior = Some(digest);
        expected_offset += length;
    }
    if expected_offset != payload_bytes
        || prior.is_none()
        || descriptor.first != {
            file.seek(SeekFrom::Start(PACK_HEADER_BYTES as u64))?;
            let mut digest = [0_u8; 32];
            file.read_exact(&mut digest)?;
            ContentDigest::from_bytes(digest)
        }
        || descriptor.last != prior.expect("validated non-empty pack")
    {
        return Err(PackedPatriciaError::Malformed);
    }
    Ok(())
}

enum StreamCursorSource<'a> {
    Resident(&'a [u8]),
    Staged(fs::File),
}

impl StreamCursorSource<'_> {
    fn read_exact_at(
        &mut self,
        offset: usize,
        target: &mut [u8],
    ) -> Result<(), PackedPatriciaError> {
        match self {
            Self::Resident(bytes) => target.copy_from_slice(
                bytes
                    .get(offset..offset + target.len())
                    .ok_or(PackedPatriciaError::Malformed)?,
            ),
            Self::Staged(file) => {
                file.seek(SeekFrom::Start(offset as u64))?;
                file.read_exact(target)?;
            }
        }
        Ok(())
    }
}

struct StreamPackCursor<'a> {
    source: StreamCursorSource<'a>,
    entry_count: usize,
    payload_start: usize,
    next_index: usize,
    current: StreamCursorEntry,
}

impl<'a> StreamPackCursor<'a> {
    fn open(
        planned: &'a PlannedStreamingPatriciaPack,
        current: Option<&'a PackedPatriciaCatalog>,
    ) -> Result<Self, PackedPatriciaError> {
        let descriptor = planned.descriptor();
        let source = match planned {
            PlannedStreamingPatriciaPack::Existing { resolver_index, .. } => {
                let pack = current
                    .and_then(|catalog| catalog.packs.get(*resolver_index))
                    .ok_or(PackedPatriciaError::Malformed)?;
                StreamCursorSource::Resident(&pack.bytes)
            }
            PlannedStreamingPatriciaPack::Staged { publication, .. } => {
                StreamCursorSource::Staged(publication.open_staged()?)
            }
        };
        let entry_count = descriptor.entries as usize;
        let payload_start = PACK_HEADER_BYTES
            .checked_add(
                entry_count
                    .checked_mul(PACK_INDEX_ENTRY_BYTES)
                    .ok_or(PackedPatriciaError::Malformed)?,
            )
            .ok_or(PackedPatriciaError::Malformed)?;
        let mut cursor = Self {
            source,
            entry_count,
            payload_start,
            next_index: 1,
            current: StreamCursorEntry {
                digest: descriptor.first,
                payload_offset: 0,
                length: 0,
            },
        };
        cursor.current = cursor.read_entry(0)?;
        Ok(cursor)
    }

    fn read_entry(&mut self, index: usize) -> Result<StreamCursorEntry, PackedPatriciaError> {
        let mut encoded = [0_u8; PACK_INDEX_ENTRY_BYTES];
        self.source.read_exact_at(
            PACK_HEADER_BYTES + index * PACK_INDEX_ENTRY_BYTES,
            &mut encoded,
        )?;
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&encoded[..32]);
        let offset = u32::from_le_bytes(encoded[32..36].try_into().expect("fixed offset")) as usize;
        let length = u32::from_le_bytes(encoded[36..40].try_into().expect("fixed length")) as usize;
        Ok(StreamCursorEntry {
            digest: ContentDigest::from_bytes(digest),
            payload_offset: self
                .payload_start
                .checked_add(offset)
                .ok_or(PackedPatriciaError::Malformed)?,
            length,
        })
    }

    fn advance(&mut self) -> Result<Option<ContentDigest>, PackedPatriciaError> {
        if self.next_index >= self.entry_count {
            return Ok(None);
        }
        self.current = self.read_entry(self.next_index)?;
        self.next_index += 1;
        Ok(Some(self.current.digest))
    }

    fn read_payload(
        &mut self,
        entry: StreamCursorEntry,
        relative_offset: usize,
        target: &mut [u8],
    ) -> Result<(), PackedPatriciaError> {
        if relative_offset
            .checked_add(target.len())
            .is_none_or(|end| end > entry.length)
        {
            return Err(PackedPatriciaError::Malformed);
        }
        self.source.read_exact_at(
            entry
                .payload_offset
                .checked_add(relative_offset)
                .ok_or(PackedPatriciaError::Malformed)?,
            target,
        )
    }
}

struct MergedStreamEntry {
    source_index: usize,
    entry: StreamCursorEntry,
}

struct FiveWayMerge<'a> {
    cursors: Vec<StreamPackCursor<'a>>,
    pending: BinaryHeap<Reverse<(ContentDigest, usize)>>,
    left_buffer: [u8; CONSTRUCTION_STREAM_BUFFER_BYTES],
    right_buffer: [u8; CONSTRUCTION_STREAM_BUFFER_BYTES],
}

impl<'a> FiveWayMerge<'a> {
    fn open(
        planned: &'a [PlannedStreamingPatriciaPack],
        current: Option<&'a PackedPatriciaCatalog>,
        selected: &[usize],
    ) -> Result<Self, PackedPatriciaError> {
        if selected.is_empty() || selected.len() > MAX_PACKS_PER_SIZE_TIER + 1 {
            return Err(PackedPatriciaError::Malformed);
        }
        let mut cursors = Vec::with_capacity(selected.len());
        let mut pending = BinaryHeap::with_capacity(selected.len());
        for index in selected {
            let cursor = StreamPackCursor::open(
                planned.get(*index).ok_or(PackedPatriciaError::Malformed)?,
                current,
            )?;
            let cursor_index = cursors.len();
            pending.push(Reverse((cursor.current.digest, cursor_index)));
            cursors.push(cursor);
        }
        Ok(Self {
            cursors,
            pending,
            left_buffer: [0; CONSTRUCTION_STREAM_BUFFER_BYTES],
            right_buffer: [0; CONSTRUCTION_STREAM_BUFFER_BYTES],
        })
    }

    fn entries_equal(
        &mut self,
        left_source: usize,
        left: StreamCursorEntry,
        right_source: usize,
        right: StreamCursorEntry,
    ) -> Result<bool, PackedPatriciaError> {
        if left.length != right.length {
            return Ok(false);
        }
        let mut offset = 0_usize;
        while offset != left.length {
            let chunk = (left.length - offset).min(CONSTRUCTION_STREAM_BUFFER_BYTES);
            self.cursors[left_source].read_payload(left, offset, &mut self.left_buffer[..chunk])?;
            self.cursors[right_source].read_payload(
                right,
                offset,
                &mut self.right_buffer[..chunk],
            )?;
            if self.left_buffer[..chunk] != self.right_buffer[..chunk] {
                return Ok(false);
            }
            offset += chunk;
        }
        Ok(true)
    }

    fn next_unique(&mut self) -> Result<Option<MergedStreamEntry>, PackedPatriciaError> {
        let Some(Reverse((digest, first_source))) = self.pending.pop() else {
            return Ok(None);
        };
        let mut duplicates = Vec::with_capacity(MAX_PACKS_PER_SIZE_TIER + 1);
        duplicates.push(first_source);
        while self
            .pending
            .peek()
            .is_some_and(|Reverse((next, _))| *next == digest)
        {
            let Reverse((_, source)) = self.pending.pop().expect("peeked duplicate");
            duplicates.push(source);
        }
        let first = self.cursors[first_source].current;
        for source in duplicates.iter().copied().skip(1) {
            let candidate = self.cursors[source].current;
            if !self.entries_equal(first_source, first, source, candidate)? {
                return Err(PackedPatriciaError::PathMismatch(digest));
            }
        }
        for source in duplicates {
            if let Some(next) = self.cursors[source].advance()? {
                self.pending.push(Reverse((next, source)));
            }
        }
        Ok(Some(MergedStreamEntry {
            source_index: first_source,
            entry: first,
        }))
    }

    fn read_payload(
        &mut self,
        entry: &MergedStreamEntry,
        offset: usize,
        target: &mut [u8],
    ) -> Result<(), PackedPatriciaError> {
        self.cursors[entry.source_index].read_payload(entry.entry, offset, target)
    }
}

fn plan_merged_stream(
    current: Option<&PackedPatriciaCatalog>,
    planned: &[PlannedStreamingPatriciaPack],
    selected: &[usize],
    work: &mut PackedPatriciaPublicationWork,
) -> Result<Vec<StreamPartitionShape>, PackedPatriciaError> {
    for index in selected {
        if let PlannedStreamingPatriciaPack::Staged {
            publication,
            descriptor,
        } = planned.get(*index).ok_or(PackedPatriciaError::Malformed)?
        {
            validate_staged_stream_pack(publication, *descriptor)?;
        }
    }
    let mut merge = FiveWayMerge::open(planned, current, selected)?;
    let mut shapes = Vec::new();
    let mut start_ordinal = 0_usize;
    let mut ordinal = 0_usize;
    let mut chunk_entries = 0_usize;
    let mut chunk_payload = 0_usize;
    let mut first = None;
    let mut last = None;
    while let Some(merged) = merge.next_unique()? {
        let candidate_entries = chunk_entries + 1;
        let candidate_total = PACK_HEADER_BYTES
            .checked_add(
                candidate_entries
                    .checked_mul(PACK_INDEX_ENTRY_BYTES)
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )
            .and_then(|total| total.checked_add(chunk_payload))
            .and_then(|total| total.checked_add(merged.entry.length))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if chunk_entries != 0
            && (candidate_entries > MAX_PACK_ENTRIES || candidate_total > MAX_PACK_BYTES)
        {
            push_stream_shape(
                &mut shapes,
                start_ordinal,
                chunk_entries,
                chunk_payload,
                first,
                last,
            );
            start_ordinal = ordinal;
            chunk_entries = 0;
            chunk_payload = 0;
            first = None;
        }
        chunk_entries += 1;
        chunk_payload = chunk_payload
            .checked_add(merged.entry.length)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        work.compaction_payload_bytes_reencoded = work
            .compaction_payload_bytes_reencoded
            .checked_add(merged.entry.length)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        first.get_or_insert(merged.entry.digest);
        last = Some(merged.entry.digest);
        ordinal += 1;
    }
    push_stream_shape(
        &mut shapes,
        start_ordinal,
        chunk_entries,
        chunk_payload,
        first,
        last,
    );
    Ok(shapes)
}

fn stage_merged_partition(
    dir: &Dir,
    current: Option<&PackedPatriciaCatalog>,
    planned: &[PlannedStreamingPatriciaPack],
    selected: &[usize],
    shape: StreamPartitionShape,
) -> Result<PlannedStreamingPatriciaPack, PackedPatriciaError> {
    let total_bytes = shape.total_bytes()?;
    let end_ordinal = shape.start_ordinal + shape.entries;
    let mut staged_digest = None;
    let publication = StagedExactImmutablePublication::construct(dir, |file| {
        write_stream_header(file, shape).map_err(packed_io_error)?;
        let mut merge = FiveWayMerge::open(planned, current, selected).map_err(packed_io_error)?;
        let mut ordinal = 0_usize;
        let mut payload_offset = 0_usize;
        while let Some(merged) = merge.next_unique().map_err(packed_io_error)? {
            if (shape.start_ordinal..end_ordinal).contains(&ordinal) {
                file.write_all(merged.entry.digest.as_bytes())?;
                file.write_all(
                    &u32::try_from(payload_offset)
                        .map_err(|_| io::Error::from(ErrorKind::InvalidData))?
                        .to_le_bytes(),
                )?;
                file.write_all(
                    &u32::try_from(merged.entry.length)
                        .map_err(|_| io::Error::from(ErrorKind::InvalidData))?
                        .to_le_bytes(),
                )?;
                payload_offset += merged.entry.length;
            }
            ordinal += 1;
        }
        if payload_offset != shape.payload_bytes {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        drop(merge);
        let payload_start = PACK_HEADER_BYTES + shape.entries * PACK_INDEX_ENTRY_BYTES;
        let mut hasher = Sha256::new();
        hash_stream_prefix(file, payload_start, &mut hasher).map_err(packed_io_error)?;
        file.seek(SeekFrom::Start(payload_start as u64))?;
        let mut merge = FiveWayMerge::open(planned, current, selected).map_err(packed_io_error)?;
        let mut payload_buffer = [0_u8; CONSTRUCTION_STREAM_BUFFER_BYTES];
        ordinal = 0;
        while let Some(merged) = merge.next_unique().map_err(packed_io_error)? {
            if (shape.start_ordinal..end_ordinal).contains(&ordinal) {
                let mut offset = 0_usize;
                while offset != merged.entry.length {
                    let chunk = (merged.entry.length - offset).min(payload_buffer.len());
                    merge
                        .read_payload(&merged, offset, &mut payload_buffer[..chunk])
                        .map_err(packed_io_error)?;
                    file.write_all(&payload_buffer[..chunk])?;
                    hasher.update(&payload_buffer[..chunk]);
                    offset += chunk;
                }
            }
            ordinal += 1;
        }
        let digest = ContentDigest::from_bytes(hasher.finalize().into());
        staged_digest = Some(digest);
        Ok((pack_filename(digest), total_bytes as u64))
    })?;
    let digest = staged_digest.ok_or(PackedPatriciaError::Malformed)?;
    Ok(PlannedStreamingPatriciaPack::Staged {
        publication,
        descriptor: PackDescriptor {
            digest,
            first: shape.first,
            last: shape.last,
            entries: shape.entries as u32,
            bytes: total_bytes as u32,
        },
    })
}

fn streaming_plan_owned_bytes(
    planned: &Vec<PlannedStreamingPatriciaPack>,
) -> Result<usize, PackedPatriciaError> {
    let inline = planned
        .capacity()
        .checked_mul(std::mem::size_of::<PlannedStreamingPatriciaPack>())
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    planned.iter().try_fold(inline, |total, pack| match pack {
        PlannedStreamingPatriciaPack::Existing { .. } => Ok(total),
        PlannedStreamingPatriciaPack::Staged { publication, .. } => total
            .checked_add(publication.owned_name_bytes())
            .ok_or(PackedPatriciaError::PackTooLarge),
    })
}

fn streaming_fixed_bytes() -> Result<usize, PackedPatriciaError> {
    let cursors = (MAX_PACKS_PER_SIZE_TIER + 1)
        .checked_mul(
            std::mem::size_of::<StreamPackCursor<'static>>()
                + std::mem::size_of::<Reverse<(ContentDigest, usize)>>()
                + std::mem::size_of::<usize>(),
        )
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    cursors
        .checked_add(
            CONSTRUCTION_STREAM_BUFFER_COUNT
                .checked_mul(CONSTRUCTION_STREAM_BUFFER_BYTES)
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        )
        .and_then(|total| total.checked_add(2 * HEAD_BYTES))
        .ok_or(PackedPatriciaError::PackTooLarge)
}

fn observe_streaming_plan(
    residency: &mut PackedPatriciaResidencyTracker,
    planned: &Vec<PlannedStreamingPatriciaPack>,
    transient_shape_capacity: usize,
    additional_plan_owned_bytes: usize,
    additional_pack_count: usize,
) -> Result<(), PackedPatriciaError> {
    let descriptors = planned
        .len()
        .checked_add(additional_pack_count)
        .ok_or(PackedPatriciaError::TooManyEntries)?
        .checked_mul(std::mem::size_of::<PackDescriptor>())
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    let shapes = transient_shape_capacity
        .checked_mul(std::mem::size_of::<StreamPartitionShape>())
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    let catalog_capacity = CATALOG_HEADER_BYTES
        .checked_add(
            planned
                .len()
                .checked_add(additional_pack_count)
                .ok_or(PackedPatriciaError::TooManyEntries)?
                .checked_mul(CATALOG_ENTRY_BYTES)
                .ok_or(PackedPatriciaError::PackTooLarge)?,
        )
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    residency.observe(
        streaming_plan_owned_bytes(planned)?
            .checked_add(additional_plan_owned_bytes)
            .and_then(|total| total.checked_add(descriptors))
            .and_then(|total| total.checked_add(shapes))
            .and_then(|total| total.checked_add(catalog_capacity))
            .and_then(|total| total.checked_add(streaming_fixed_bytes().ok()?))
            .ok_or(PackedPatriciaError::PackTooLarge)?,
    )
}

fn observe_tier_replacement_plan(
    residency: &mut PackedPatriciaResidencyTracker,
    planned: &Vec<PlannedStreamingPatriciaPack>,
    replacements: &Vec<PlannedStreamingPatriciaPack>,
    transient_shape_capacity: usize,
    next_capacity: usize,
) -> Result<(), PackedPatriciaError> {
    let next_vector_bytes = next_capacity
        .checked_mul(std::mem::size_of::<PlannedStreamingPatriciaPack>())
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    observe_streaming_plan(
        residency,
        planned,
        transient_shape_capacity,
        streaming_plan_owned_bytes(replacements)?
            .checked_add(next_vector_bytes)
            .ok_or(PackedPatriciaError::PackTooLarge)?,
        replacements.len(),
    )
}

fn compact_size_tiers_streaming(
    dir: &Dir,
    current: Option<&PackedPatriciaCatalog>,
    planned: &mut Vec<PlannedStreamingPatriciaPack>,
    work: &mut PackedPatriciaPublicationWork,
    residency: &mut PackedPatriciaResidencyTracker,
) -> Result<(), PackedPatriciaError> {
    let mut carried_tiers = [false; PACK_SIZE_TIER_COUNT];
    loop {
        let mut tier_counts = [0_usize; PACK_SIZE_TIER_COUNT];
        for pack in planned.iter() {
            tier_counts[pack_size_tier(pack.descriptor().bytes)] += 1;
        }
        let Some(tier) = tier_counts.iter().enumerate().find_map(|(tier, count)| {
            (*count > MAX_PACKS_PER_SIZE_TIER && !carried_tiers[tier]).then_some(tier)
        }) else {
            return Ok(());
        };
        carried_tiers[tier] = true;
        let selected = planned
            .iter()
            .enumerate()
            .filter_map(|(index, pack)| {
                (pack_size_tier(pack.descriptor().bytes) == tier).then_some(index)
            })
            .take(MAX_PACKS_PER_SIZE_TIER + 1)
            .collect::<Vec<_>>();
        let insert_after = *selected.last().ok_or(PackedPatriciaError::Malformed)?;
        for index in &selected {
            let descriptor = planned[*index].descriptor();
            work.compaction_packs_selected = work
                .compaction_packs_selected
                .checked_add(1)
                .ok_or(PackedPatriciaError::TooManyEntries)?;
            work.compaction_pack_bytes_selected = work
                .compaction_pack_bytes_selected
                .checked_add(descriptor.bytes as usize)
                .ok_or(PackedPatriciaError::PackTooLarge)?;
        }
        let shapes = plan_merged_stream(current, planned, &selected, work)?;
        if shapes.len() >= selected.len()
            && shapes.iter().all(|shape| {
                pack_size_tier(shape.total_bytes().unwrap_or(u32::MAX as usize) as u32) == tier
            })
        {
            return Err(PackedPatriciaError::PackTooLarge);
        }
        observe_streaming_plan(residency, planned, shapes.capacity(), 0, 0)?;
        let mut replacements = Vec::with_capacity(shapes.len());
        for shape in shapes.iter().copied() {
            let staged = stage_merged_partition(dir, current, planned, &selected, shape)?;
            work.pack_bytes_encoded = work
                .pack_bytes_encoded
                .checked_add(staged.descriptor().bytes as usize)
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            replacements.push(staged);
            observe_streaming_plan(
                residency,
                planned,
                shapes.capacity(),
                streaming_plan_owned_bytes(&replacements)?,
                replacements.len(),
            )?;
        }
        let next_capacity = planned.len() - selected.len() + replacements.len();
        observe_tier_replacement_plan(
            residency,
            planned,
            &replacements,
            shapes.capacity(),
            next_capacity,
        )?;
        let mut replacements = replacements.into_iter();
        let mut next = Vec::with_capacity(next_capacity);
        for (index, pack) in planned.drain(..).enumerate() {
            if !selected.contains(&index) {
                next.push(pack);
            }
            if index == insert_after {
                next.extend(replacements.by_ref());
            }
        }
        *planned = next;
        observe_streaming_plan(residency, planned, shapes.capacity(), 0, 0)?;
    }
}

/// Construction-only append which never clones delta or historical payloads
/// and never owns a complete replacement pack buffer or replacement range map.
pub(crate) fn publish_appended_catalog_bounded_streaming(
    dir: &Dir,
    publisher: &dyn PatriciaNodePublisher,
    current: Option<&PackedPatriciaCatalog>,
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
    catalog_pack_byte_limit: usize,
    residency_budget: PackedPatriciaResidencyBudget,
) -> Result<
    (
        Option<PendingStreamingPackedPatriciaCatalog>,
        PackedPatriciaPublicationWork,
    ),
    PackedPatriciaError,
> {
    if entries.is_empty() {
        return Err(PackedPatriciaError::Empty);
    }
    let mut work = PackedPatriciaPublicationWork::default();
    let mut residency = PackedPatriciaResidencyTracker::new(
        residency_budget,
        current.map_or(0, PackedPatriciaCatalog::resident_bytes),
    )?;
    let shapes = plan_delta_stream(current, entries, &mut work)?;
    if shapes.is_empty() {
        work.peak_resident_bytes = residency.peak_bytes;
        return Ok((None, work));
    }
    let mut planned = current
        .map(|catalog| {
            catalog
                .descriptors
                .iter()
                .copied()
                .enumerate()
                .map(
                    |(resolver_index, descriptor)| PlannedStreamingPatriciaPack::Existing {
                        resolver_index,
                        descriptor,
                    },
                )
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    observe_streaming_plan(&mut residency, &planned, shapes.capacity(), 0, 0)?;
    for shape in shapes.iter().copied() {
        let staged = stage_delta_partition(dir, current, entries, shape)?;
        let bytes = staged.descriptor().bytes as usize;
        work.delta_pack_bytes_encoded = work
            .delta_pack_bytes_encoded
            .checked_add(bytes)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        work.pack_bytes_encoded = work
            .pack_bytes_encoded
            .checked_add(bytes)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        planned.push(staged);
        observe_streaming_plan(&mut residency, &planned, shapes.capacity(), 0, 0)?;
    }
    drop(shapes);
    compact_size_tiers_streaming(dir, current, &mut planned, &mut work, &mut residency)?;
    let descriptors = planned
        .iter()
        .map(PlannedStreamingPatriciaPack::descriptor)
        .collect::<Vec<_>>();
    let final_pack_bytes = descriptors.iter().try_fold(0_usize, |total, descriptor| {
        total
            .checked_add(descriptor.bytes as usize)
            .ok_or(PackedPatriciaError::PackTooLarge)
    })?;
    if descriptors.len() > MAX_CATALOG_PACKS
        || final_pack_bytes > catalog_pack_byte_limit.min(MAX_CATALOG_PACK_BYTES)
    {
        return Err(PackedPatriciaError::PackTooLarge);
    }
    let input_was_tier_bounded = current
        .map(|catalog| size_tiers_are_bounded(&catalog.descriptors))
        .unwrap_or(true);
    if input_was_tier_bounded && !size_tiers_are_bounded(&descriptors) {
        return Err(PackedPatriciaError::PackTooLarge);
    }
    let catalog_publication = if input_was_tier_bounded {
        PackedPatriciaCatalogPublication::build_descriptors_for_schema(
            &descriptors,
            CATALOG_SCHEMA_VERSION,
        )?
    } else {
        PackedPatriciaCatalogPublication::build_descriptors(&descriptors)?
    };
    work.catalog_metadata_bytes_encoded = catalog_publication.bytes.len();
    let descriptor_capacity = descriptors
        .capacity()
        .checked_mul(std::mem::size_of::<PackDescriptor>())
        .ok_or(PackedPatriciaError::PackTooLarge)?;
    residency.observe(
        streaming_plan_owned_bytes(&planned)?
            .checked_add(descriptor_capacity)
            .and_then(|total| total.checked_add(catalog_publication.bytes.capacity()))
            .and_then(|total| total.checked_add(streaming_fixed_bytes().ok()?))
            .ok_or(PackedPatriciaError::PackTooLarge)?,
    )?;

    for pack in planned {
        if let PlannedStreamingPatriciaPack::Staged {
            publication,
            descriptor,
        } = pack
        {
            validate_staged_stream_pack(&publication, descriptor)?;
            publisher
                .publish_staged_construction_exact(publication)
                .map_err(PackedPatriciaError::Publication)?;
            work.pack_bytes_published = work
                .pack_bytes_published
                .checked_add(descriptor.bytes as usize)
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            work.packs_published = work
                .packs_published
                .checked_add(1)
                .ok_or(PackedPatriciaError::TooManyEntries)?;
        }
    }
    let catalog = catalog_publication.publish(dir, publisher)?;
    work.catalog_metadata_bytes_published = catalog.bytes as usize;
    work.peak_resident_bytes = residency.peak_bytes;
    Ok((
        Some(PendingStreamingPackedPatriciaCatalog { catalog }),
        work,
    ))
}

fn compact_size_tiers(
    current: Option<&PackedPatriciaCatalog>,
    planned: &mut Vec<PlannedPatriciaPack>,
    work: &mut PackedPatriciaPublicationWork,
    mut residency: Option<&mut PackedPatriciaResidencyTracker>,
    retained_planner_bytes: usize,
) -> Result<(), PackedPatriciaError> {
    // A schema-2 input may begin arbitrarily overfull. Carrying a tier at most
    // once bounds one routine mutation independently of that historical
    // fan-in; subsequent mutations continue making deterministic progress.
    let mut carried_tiers = [false; PACK_SIZE_TIER_COUNT];
    loop {
        let mut tier_counts = [0_usize; PACK_SIZE_TIER_COUNT];
        for pack in planned.iter() {
            tier_counts[pack_size_tier(pack.descriptor().bytes)] += 1;
        }
        let Some(tier) = tier_counts.iter().enumerate().find_map(|(tier, count)| {
            (*count > MAX_PACKS_PER_SIZE_TIER && !carried_tiers[tier]).then_some(tier)
        }) else {
            return Ok(());
        };
        carried_tiers[tier] = true;
        let selected = planned
            .iter()
            .enumerate()
            .filter_map(|(index, pack)| {
                (pack_size_tier(pack.descriptor().bytes) == tier).then_some(index)
            })
            .take(MAX_PACKS_PER_SIZE_TIER + 1)
            .collect::<Vec<_>>();
        let selected_set = selected
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let insert_after = *selected.last().expect("overflowing tier is non-empty");
        if let Some(residency) = residency.as_deref_mut() {
            let (selected_pack_bytes, selected_entries) =
                selected
                    .iter()
                    .try_fold((0_usize, 0_usize), |(bytes, entries), index| {
                        let descriptor = planned[*index].descriptor();
                        Ok::<_, PackedPatriciaError>((
                            bytes
                                .checked_add(descriptor.bytes as usize)
                                .ok_or(PackedPatriciaError::PackTooLarge)?,
                            entries
                                .checked_add(descriptor.entries as usize)
                                .ok_or(PackedPatriciaError::TooManyEntries)?,
                        ))
                    })?;
            let cloned_entries_bytes = selected_pack_bytes
                .checked_add(
                    selected_entries
                        .checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)
                        .ok_or(PackedPatriciaError::PackTooLarge)?,
                )
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            let selected_metadata_bytes = selected
                .len()
                .checked_mul(PACKED_MAP_ENTRY_OWNERSHIP_BYTES)
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            let replacement_vector_bytes = planned
                .len()
                .checked_mul(std::mem::size_of::<PlannedPatriciaPack>())
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            // `entries` clones the selected historical payload while the
            // resolver and selected planned publications remain live. Pack
            // partitioning then owns a cloned chunk and encoded replacement
            // publications concurrently. Three clone-sized charges cover the
            // target map, the largest partition chunk, and all replacement
            // encoded/range-map ownership before the selected plans are
            // drained into their replacement vector.
            let carry_scratch_bytes = cloned_entries_bytes
                .checked_mul(3)
                .and_then(|total| total.checked_add(selected_metadata_bytes))
                .and_then(|total| total.checked_add(replacement_vector_bytes))
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            residency.observe(
                retained_planner_bytes
                    .checked_add(planned_owned_bytes(planned))
                    .and_then(|total| total.checked_add(carry_scratch_bytes))
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )?;
        }
        let mut entries = BTreeMap::new();
        for index in &selected {
            let pack = &planned[*index];
            work.compaction_packs_selected = work
                .compaction_packs_selected
                .checked_add(1)
                .ok_or(PackedPatriciaError::TooManyEntries)?;
            work.compaction_pack_bytes_selected = work
                .compaction_pack_bytes_selected
                .checked_add(pack.descriptor().bytes as usize)
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            match pack {
                PlannedPatriciaPack::Existing { index, .. } => {
                    let historical = current
                        .and_then(|catalog| catalog.packs.get(*index))
                        .ok_or(PackedPatriciaError::Malformed)?;
                    copy_opened_entries(historical, &mut entries, work)?;
                }
                PlannedPatriciaPack::Publication(publication) => {
                    copy_exact_entries(&publication.bytes, &publication.ranges, &mut entries, work)?
                }
            }
        }
        let publications = partition_publications(&entries)?;
        if publications.len() >= selected.len()
            && publications
                .iter()
                .all(|publication| pack_size_tier(publication.bytes.len() as u32) == tier)
        {
            return Err(PackedPatriciaError::PackTooLarge);
        }
        work.pack_bytes_encoded =
            publications
                .iter()
                .try_fold(work.pack_bytes_encoded, |total, publication| {
                    total
                        .checked_add(publication.bytes.len())
                        .ok_or(PackedPatriciaError::PackTooLarge)
                })?;

        let mut publications = publications.into_iter();
        let mut next = Vec::with_capacity(planned.len() - selected.len() + publications.len());
        for (index, pack) in planned.drain(..).enumerate() {
            if !selected_set.contains(&index) {
                next.push(pack);
            }
            if index == insert_after {
                next.extend(publications.by_ref().map(PlannedPatriciaPack::Publication));
            }
        }
        debug_assert!(publications.next().is_none());
        *planned = next;
        if let Some(residency) = residency.as_deref_mut() {
            residency.observe(
                retained_planner_bytes
                    .checked_add(planned_owned_bytes(planned))
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
            )?;
        }
    }
}

fn copy_exact_entries(
    source_bytes: &[u8],
    source_entries: &BTreeMap<ContentDigest, Range<usize>>,
    target: &mut BTreeMap<ContentDigest, Vec<u8>>,
    work: &mut PackedPatriciaPublicationWork,
) -> Result<(), PackedPatriciaError> {
    for (digest, range) in source_entries {
        let bytes = source_bytes
            .get(range.clone())
            .ok_or(PackedPatriciaError::Malformed)?;
        work.compaction_payload_bytes_reencoded = work
            .compaction_payload_bytes_reencoded
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if let Some(existing) = target.get(digest) {
            if existing.as_slice() != bytes {
                return Err(PackedPatriciaError::PathMismatch(*digest));
            }
        } else {
            target.insert(*digest, bytes.to_vec());
        }
    }
    Ok(())
}

fn copy_opened_entries(
    source: &PackedPatriciaPack,
    target: &mut BTreeMap<ContentDigest, Vec<u8>>,
    work: &mut PackedPatriciaPublicationWork,
) -> Result<(), PackedPatriciaError> {
    for (digest, range) in source.iter_entries() {
        let bytes = source
            .bytes
            .get(range)
            .ok_or(PackedPatriciaError::Malformed)?;
        work.compaction_payload_bytes_reencoded = work
            .compaction_payload_bytes_reencoded
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if let Some(existing) = target.get(&digest) {
            if existing.as_slice() != bytes {
                return Err(PackedPatriciaError::PathMismatch(digest));
            }
        } else {
            target.insert(digest, bytes.to_vec());
        }
    }
    Ok(())
}

fn pack_size_tier(bytes: u32) -> usize {
    let bytes = bytes as usize;
    debug_assert!((MIN_PACK_BYTES..=MAX_PACK_BYTES).contains(&bytes));
    (bytes.ilog2() - MIN_PACK_BYTES.ilog2()) as usize
}

fn partition_publications(
    entries: &BTreeMap<ContentDigest, Vec<u8>>,
) -> Result<Vec<PackedPatriciaPublication>, PackedPatriciaError> {
    if entries.is_empty() {
        return Err(PackedPatriciaError::Empty);
    }
    let mut publications = Vec::new();
    let mut chunk = BTreeMap::new();
    let mut chunk_payload_bytes = 0_usize;
    for (digest, bytes) in entries {
        let candidate_entries = chunk.len() + 1;
        let candidate_bytes = PACK_HEADER_BYTES
            .checked_add(candidate_entries * PACK_INDEX_ENTRY_BYTES)
            .and_then(|total| total.checked_add(chunk_payload_bytes))
            .and_then(|total| total.checked_add(bytes.len()))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if !chunk.is_empty()
            && (candidate_entries > MAX_PACK_ENTRIES || candidate_bytes > MAX_PACK_BYTES)
        {
            publications.push(PackedPatriciaPublication::build(&chunk)?);
            chunk.clear();
            chunk_payload_bytes = 0;
        }
        chunk_payload_bytes = chunk_payload_bytes
            .checked_add(bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        chunk.insert(*digest, bytes.clone());
    }
    if !chunk.is_empty() {
        publications.push(PackedPatriciaPublication::build(&chunk)?);
    }
    Ok(publications)
}

fn encode_catalog_head(catalog_digest: ContentDigest, catalog_bytes: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEAD_BYTES);
    bytes.extend_from_slice(HEAD_MAGIC);
    bytes.extend_from_slice(&HEAD_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(catalog_digest.as_bytes());
    bytes.extend_from_slice(&catalog_bytes.to_le_bytes());
    debug_assert_eq!(bytes.len(), HEAD_BYTES);
    bytes
}

fn decode_catalog(bytes: &[u8]) -> Result<Vec<PackDescriptor>, PackedPatriciaError> {
    if bytes.len() < CATALOG_HEADER_BYTES
        || bytes.len() > MAX_CATALOG_BYTES
        || &bytes[..8] != CATALOG_MAGIC
    {
        return Err(PackedPatriciaError::Malformed);
    }
    let schema_version = read_u32(bytes, 8)?;
    if !matches!(
        schema_version,
        LEGACY_CATALOG_SCHEMA_VERSION | LAYERED_CATALOG_SCHEMA_VERSION | CATALOG_SCHEMA_VERSION
    ) {
        return Err(PackedPatriciaError::Malformed);
    }
    let pack_count = read_u32(bytes, 12)? as usize;
    let total_entries = read_u32(bytes, 16)?;
    let total_pack_bytes = read_u64(bytes, 20)?;
    if pack_count == 0
        || bytes.len() != CATALOG_HEADER_BYTES + pack_count * CATALOG_ENTRY_BYTES
        || total_pack_bytes > MAX_CATALOG_PACK_BYTES as u64
    {
        return Err(PackedPatriciaError::Malformed);
    }
    let mut descriptors = Vec::with_capacity(pack_count);
    for index in 0..pack_count {
        let start = CATALOG_HEADER_BYTES + index * CATALOG_ENTRY_BYTES;
        descriptors.push(PackDescriptor {
            digest: read_digest(bytes, start)?,
            first: read_digest(bytes, start + 32)?,
            last: read_digest(bytes, start + 64)?,
            entries: read_u32(bytes, start + 96)?,
            bytes: read_u32(bytes, start + 100)?,
        });
    }
    validate_descriptors(
        &descriptors,
        schema_version == LEGACY_CATALOG_SCHEMA_VERSION,
    )?;
    if schema_version == CATALOG_SCHEMA_VERSION && !size_tiers_are_bounded(&descriptors) {
        return Err(PackedPatriciaError::Malformed);
    }
    if descriptors
        .iter()
        .map(|descriptor| descriptor.entries)
        .sum::<u32>()
        != total_entries
        || descriptors
            .iter()
            .map(|descriptor| u64::from(descriptor.bytes))
            .sum::<u64>()
            != total_pack_bytes
    {
        return Err(PackedPatriciaError::Malformed);
    }
    Ok(descriptors)
}

fn validate_descriptors(
    descriptors: &[PackDescriptor],
    require_disjoint_ranges: bool,
) -> Result<(), PackedPatriciaError> {
    let mut digests = std::collections::BTreeSet::new();
    for (index, descriptor) in descriptors.iter().enumerate() {
        if descriptor.first > descriptor.last
            || descriptor.entries == 0
            || descriptor.entries as usize > MAX_PACK_ENTRIES
            || (descriptor.bytes as usize) > MAX_PACK_BYTES
            || (descriptor.bytes as usize) < MIN_PACK_BYTES
            || !digests.insert(descriptor.digest)
            || require_disjoint_ranges
                && index > 0
                && descriptors[index - 1].last >= descriptor.first
        {
            return Err(PackedPatriciaError::Malformed);
        }
    }
    Ok(())
}

fn size_tiers_are_bounded(descriptors: &[PackDescriptor]) -> bool {
    let mut tier_counts = [0_usize; PACK_SIZE_TIER_COUNT];
    for descriptor in descriptors {
        if !(MIN_PACK_BYTES..=MAX_PACK_BYTES).contains(&(descriptor.bytes as usize)) {
            return false;
        }
        let tier = pack_size_tier(descriptor.bytes);
        tier_counts[tier] += 1;
        if tier_counts[tier] > MAX_PACKS_PER_SIZE_TIER {
            return false;
        }
    }
    true
}

fn validate_duplicate_nodes(packs: &[PackedPatriciaPack]) -> Result<(), PackedPatriciaError> {
    let mut pending = BinaryHeap::new();
    for (pack_index, pack) in packs.iter().enumerate() {
        if pack.entry_count != 0 {
            pending.push(Reverse((pack.entry(0).0, pack_index, 0_usize)));
        }
    }
    let mut prior: Option<(ContentDigest, usize, Range<usize>)> = None;
    while let Some(Reverse((digest, pack_index, entry_index))) = pending.pop() {
        let pack = &packs[pack_index];
        let (_, range) = pack.entry(entry_index);
        if let Some((prior_digest, prior_pack_index, prior_range)) = &prior {
            if *prior_digest == digest
                && packs[*prior_pack_index].bytes[prior_range.clone()] != pack.bytes[range.clone()]
            {
                return Err(PackedPatriciaError::PathMismatch(digest));
            }
        }
        if prior
            .as_ref()
            .is_none_or(|(prior_digest, _, _)| *prior_digest != digest)
        {
            prior = Some((digest, pack_index, range));
        }
        let next_index = entry_index + 1;
        if next_index < pack.entry_count {
            pending.push(Reverse((pack.entry(next_index).0, pack_index, next_index)));
        }
    }
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, PackedPatriciaError> {
    let encoded = bytes
        .get(offset..offset + 4)
        .ok_or(PackedPatriciaError::Malformed)?;
    Ok(u32::from_le_bytes(
        encoded
            .try_into()
            .map_err(|_| PackedPatriciaError::Malformed)?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, PackedPatriciaError> {
    let encoded = bytes
        .get(offset..offset + 8)
        .ok_or(PackedPatriciaError::Malformed)?;
    Ok(u64::from_le_bytes(
        encoded
            .try_into()
            .map_err(|_| PackedPatriciaError::Malformed)?,
    ))
}

fn read_digest(bytes: &[u8], offset: usize) -> Result<ContentDigest, PackedPatriciaError> {
    let encoded = bytes
        .get(offset..offset + 32)
        .ok_or(PackedPatriciaError::Malformed)?;
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(encoded);
    Ok(ContentDigest::from_bytes(digest))
}

fn pack_filename(digest: ContentDigest) -> String {
    format!("{digest}{PACK_SUFFIX}")
}

fn catalog_filename(digest: ContentDigest) -> String {
    format!("{digest}{CATALOG_SUFFIX}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use cap_std::ambient_authority;
    use uuid::Uuid;

    use super::*;
    use crate::{publish_immutable_exact, PatriciaPublicationError};

    struct ExactPublisher;

    impl PatriciaNodePublisher for ExactPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }
    }

    struct FailingPublisher {
        calls: AtomicUsize,
    }

    impl PatriciaNodePublisher for FailingPublisher {
        fn publish(
            &self,
            _dir: &Dir,
            _filename: &str,
            _bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(PatriciaPublicationError::new("injected pre-commit crash"))
        }
    }

    struct CrashAfterCommitPublisher {
        fail_once: AtomicBool,
    }

    impl PatriciaNodePublisher for CrashAfterCommitPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)?;
            if self.fail_once.swap(false, Ordering::Relaxed) {
                return Err(PatriciaPublicationError::new(
                    "injected crash after immutable commit",
                ));
            }
            Ok(())
        }
    }

    struct Fixture {
        path: std::path::PathBuf,
        dir: Dir,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-packed-patricia-{name}-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            let dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
            Self { path, dir }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }

    fn representative_nodes_in_family(
        family: &str,
        count: usize,
    ) -> BTreeMap<ContentDigest, Vec<u8>> {
        (0..count)
            .map(|index| {
                let bytes = format!(
                    "{family}/{index:04}/pages/Unicode α-{index:04}.md/值-{}",
                    index % 17
                )
                .into_bytes();
                (ContentDigest::of(&bytes), bytes)
            })
            .collect()
    }

    fn representative_nodes(count: usize) -> BTreeMap<ContentDigest, Vec<u8>> {
        representative_nodes_in_family("node-v1", count)
    }

    fn publication_with_schema(
        entries: &BTreeMap<ContentDigest, Vec<u8>>,
        schema_version: u32,
    ) -> PackedPatriciaPublication {
        let mut publication = PackedPatriciaPublication::build(entries).unwrap();
        publication.bytes[8..12].copy_from_slice(&schema_version.to_le_bytes());
        publication.digest = ContentDigest::of(&publication.bytes);
        publication
    }

    fn publish_loose(fixture: &Fixture, nodes: &BTreeMap<ContentDigest, Vec<u8>>) {
        for (digest, bytes) in nodes {
            publish_immutable_exact(&fixture.dir, &format!("{digest}.patricia-node"), bytes)
                .unwrap();
        }
    }

    fn publish_cataloged(
        fixture: &Fixture,
        nodes: &BTreeMap<ContentDigest, Vec<u8>>,
        entries_per_pack: usize,
    ) -> Vec<ContentDigest> {
        let mut published = Vec::new();
        let mut digests = Vec::new();
        for chunk in nodes.iter().collect::<Vec<_>>().chunks(entries_per_pack) {
            let entries = chunk
                .iter()
                .map(|(digest, bytes)| (**digest, (*bytes).clone()))
                .collect();
            let publication = PackedPatriciaPublication::build(&entries).unwrap();
            digests.push(publication.digest());
            published.push(publication.publish(&fixture.dir, &ExactPublisher).unwrap());
        }
        let catalog = PackedPatriciaCatalogPublication::build(&published).unwrap();
        let catalog = catalog.publish(&fixture.dir, &ExactPublisher).unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();
        digests
    }

    fn append_catalog(
        fixture: &Fixture,
        current: Option<PackedPatriciaCatalog>,
        entries: &BTreeMap<ContentDigest, Vec<u8>>,
        publisher: &dyn PatriciaNodePublisher,
    ) -> Result<(PackedPatriciaCatalog, PackedPatriciaPublicationWork), PackedPatriciaError> {
        let expected = current.as_ref().map(PackedPatriciaCatalog::authority);
        let (pending, work) = publish_appended_catalog(
            &fixture.dir,
            publisher,
            current.as_ref(),
            entries,
            MAX_CATALOG_PACK_BYTES,
        )?;
        let pending = pending.expect("fixture appends one previously absent digest");
        transition_catalog_head(&fixture.dir, expected, pending.published_catalog())?;
        Ok((pending.finish(current), work))
    }

    fn append_catalog_streaming(
        fixture: &Fixture,
        current: Option<PackedPatriciaCatalog>,
        entries: &BTreeMap<ContentDigest, Vec<u8>>,
    ) -> Result<(PackedPatriciaCatalog, PackedPatriciaPublicationWork), PackedPatriciaError> {
        let expected = current.as_ref().map(PackedPatriciaCatalog::authority);
        let (pending, work) = publish_appended_catalog_bounded_streaming(
            &fixture.dir,
            &ExactPublisher,
            current.as_ref(),
            entries,
            MAX_CATALOG_PACK_BYTES,
            PackedPatriciaResidencyBudget {
                retained_bytes: 0,
                maximum_bytes: 64 * 1024 * 1024,
            },
        )?;
        let pending = pending.expect("fixture appends one previously absent digest");
        transition_catalog_head(&fixture.dir, expected, pending.published_catalog())?;
        drop(current);
        PackedPatriciaCatalog::discover(&fixture.dir)?
            .ok_or(PackedPatriciaError::Malformed)
            .map(|catalog| (catalog, work))
    }

    fn publish_legacy_layered_catalog(
        fixture: &Fixture,
        packs: &[BTreeMap<ContentDigest, Vec<u8>>],
    ) -> PackedPatriciaCatalog {
        let descriptors = packs
            .iter()
            .map(|entries| {
                let publication = publication_with_schema(entries, LEGACY_PACK_SCHEMA_VERSION);
                publication
                    .publish(&fixture.dir, &ExactPublisher)
                    .unwrap()
                    .descriptor()
            })
            .collect::<Vec<_>>();
        let catalog = PackedPatriciaCatalogPublication::build_descriptors_for_schema(
            &descriptors,
            LAYERED_CATALOG_SCHEMA_VERSION,
        )
        .unwrap()
        .publish(&fixture.dir, &ExactPublisher)
        .unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();
        PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap()
    }

    fn live_catalog_bytes(
        fixture: &Fixture,
        catalog: &PackedPatriciaCatalog,
    ) -> BTreeMap<String, Vec<u8>> {
        catalog
            .live_filenames()
            .into_iter()
            .filter(|name| name != OPERATION_LOCK_FILENAME)
            .map(|name| {
                let bytes = fixture.dir.read(&name).unwrap();
                (name, bytes)
            })
            .collect()
    }

    fn staged_test_plan(
        fixture: &Fixture,
        indexed_digest: ContentDigest,
        payload: &[u8],
    ) -> PlannedStreamingPatriciaPack {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PACK_MAGIC);
        bytes.extend_from_slice(&PACK_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(indexed_digest.as_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        let pack_digest = ContentDigest::of(&bytes);
        let exact_length = bytes.len() as u64;
        let publication = StagedExactImmutablePublication::construct(&fixture.dir, |file| {
            file.write_all(&bytes)?;
            Ok((pack_filename(pack_digest), exact_length))
        })
        .unwrap();
        PlannedStreamingPatriciaPack::Staged {
            publication,
            descriptor: PackDescriptor {
                digest: pack_digest,
                first: indexed_digest,
                last: indexed_digest,
                entries: 1,
                bytes: exact_length as u32,
            },
        }
    }

    fn catalog_schema_version(fixture: &Fixture, catalog: &PackedPatriciaCatalog) -> u32 {
        let bytes = fs::read(
            fixture
                .path
                .join(catalog_filename(catalog.authority.catalog_digest)),
        )
        .unwrap();
        read_u32(&bytes, 8).unwrap()
    }

    #[test]
    fn tier_replacement_preflight_accounts_all_live_vector_capacities() {
        let digest = ContentDigest::of(b"tier replacement capacity");
        let descriptor = PackDescriptor {
            digest,
            first: digest,
            last: digest,
            entries: 1,
            bytes: MIN_PACK_BYTES as u32,
        };
        let mut planned = Vec::with_capacity(5);
        for resolver_index in 0..5 {
            planned.push(PlannedStreamingPatriciaPack::Existing {
                resolver_index,
                descriptor,
            });
        }
        let mut replacements = Vec::with_capacity(1);
        replacements.push(PlannedStreamingPatriciaPack::Existing {
            resolver_index: 5,
            descriptor,
        });
        let shape_capacity = 2;
        let next_capacity = 1;
        let simultaneous_vector_bytes =
            (planned.capacity() + replacements.capacity() + next_capacity)
                * std::mem::size_of::<PlannedStreamingPatriciaPack>();
        let pending_pack_count = planned.len() + replacements.len();
        let expected_peak = simultaneous_vector_bytes
            + pending_pack_count * std::mem::size_of::<PackDescriptor>()
            + shape_capacity * std::mem::size_of::<StreamPartitionShape>()
            + CATALOG_HEADER_BYTES
            + pending_pack_count * CATALOG_ENTRY_BYTES
            + streaming_fixed_bytes().unwrap();
        let mut residency = PackedPatriciaResidencyTracker::new(
            PackedPatriciaResidencyBudget {
                retained_bytes: 0,
                maximum_bytes: expected_peak - 1,
            },
            0,
        )
        .unwrap();

        assert!(matches!(
            observe_tier_replacement_plan(
                &mut residency,
                &planned,
                &replacements,
                shape_capacity,
                next_capacity,
            ),
            Err(PackedPatriciaError::PackTooLarge)
        ));
        assert_eq!(residency.peak_bytes, expected_peak);
    }

    #[test]
    fn construction_streamed_delta_and_schema_one_five_way_carry_are_byte_identical() {
        let canonical_delta_fixture = Fixture::new("stream-delta-canonical");
        let streamed_delta_fixture = Fixture::new("stream-delta-streamed");
        let delta = representative_nodes_in_family("streamed-delta", 257);
        let (canonical_delta, _) =
            append_catalog(&canonical_delta_fixture, None, &delta, &ExactPublisher).unwrap();
        let (streamed_delta, delta_work) =
            append_catalog_streaming(&streamed_delta_fixture, None, &delta).unwrap();
        assert_eq!(
            live_catalog_bytes(&canonical_delta_fixture, &canonical_delta),
            live_catalog_bytes(&streamed_delta_fixture, &streamed_delta),
        );
        assert!(delta_work.peak_resident_bytes <= 64 * 1024 * 1024);

        let shared = b"exact duplicate shared by legacy packs".to_vec();
        let shared_digest = ContentDigest::of(&shared);
        let legacy_packs = (0..4)
            .map(|index| {
                let unique = format!("legacy schema one unique {index}").into_bytes();
                let mut entries = BTreeMap::from([(ContentDigest::of(&unique), unique)]);
                if index < 2 {
                    entries.insert(shared_digest, shared.clone());
                } else {
                    let peer = format!("legacy schema one peer   {index}").into_bytes();
                    entries.insert(ContentDigest::of(&peer), peer);
                }
                entries
            })
            .collect::<Vec<_>>();
        let carry_bytes = b"schema two arriving carry".to_vec();
        let carry_peer = b"schema two arriving peer ".to_vec();
        let carry = BTreeMap::from([
            (ContentDigest::of(&carry_bytes), carry_bytes),
            (ContentDigest::of(&carry_peer), carry_peer),
        ]);
        let canonical_fixture = Fixture::new("stream-carry-canonical");
        let streamed_fixture = Fixture::new("stream-carry-streamed");
        let canonical_current = publish_legacy_layered_catalog(&canonical_fixture, &legacy_packs);
        let streamed_current = publish_legacy_layered_catalog(&streamed_fixture, &legacy_packs);
        let (canonical, canonical_work) = append_catalog(
            &canonical_fixture,
            Some(canonical_current),
            &carry,
            &ExactPublisher,
        )
        .unwrap();
        let (streamed, streamed_work) =
            append_catalog_streaming(&streamed_fixture, Some(streamed_current), &carry).unwrap();
        assert_eq!(canonical_work.compaction_packs_selected, 5);
        assert_eq!(streamed_work.compaction_packs_selected, 5);
        assert_eq!(
            live_catalog_bytes(&canonical_fixture, &canonical),
            live_catalog_bytes(&streamed_fixture, &streamed),
        );
        assert_eq!(streamed.get(shared_digest), Some(shared.as_slice()));
        for pack in &streamed.packs {
            assert_eq!(read_u32(&pack.bytes, 8).unwrap(), PACK_SCHEMA_VERSION);
        }
    }

    #[test]
    fn five_way_stream_deduplicates_exact_bytes_and_refuses_conflicting_duplicates() {
        let exact_fixture = Fixture::new("stream-duplicate-exact");
        let indexed_digest = ContentDigest::of(b"claimed duplicate digest");
        let exact = vec![
            staged_test_plan(&exact_fixture, indexed_digest, b"exact shared bytes"),
            staged_test_plan(&exact_fixture, indexed_digest, b"exact shared bytes"),
            staged_test_plan(
                &exact_fixture,
                ContentDigest::of(b"third digest"),
                b"third bytes",
            ),
            staged_test_plan(
                &exact_fixture,
                ContentDigest::of(b"fourth digest"),
                b"fourth bytes",
            ),
            staged_test_plan(
                &exact_fixture,
                ContentDigest::of(b"fifth digest"),
                b"fifth bytes",
            ),
        ];
        let mut merge = FiveWayMerge::open(&exact, None, &[0, 1, 2, 3, 4]).unwrap();
        let mut duplicate_emissions = 0;
        while let Some(entry) = merge.next_unique().unwrap() {
            duplicate_emissions += usize::from(entry.entry.digest == indexed_digest);
        }
        assert_eq!(duplicate_emissions, 1);

        let conflicting_fixture = Fixture::new("stream-duplicate-conflict");
        let conflicting = vec![
            staged_test_plan(&conflicting_fixture, indexed_digest, b"first bytes"),
            staged_test_plan(&conflicting_fixture, indexed_digest, b"conflicting bytes"),
            staged_test_plan(
                &conflicting_fixture,
                ContentDigest::of(b"third digest"),
                b"third bytes",
            ),
            staged_test_plan(
                &conflicting_fixture,
                ContentDigest::of(b"fourth digest"),
                b"fourth bytes",
            ),
            staged_test_plan(
                &conflicting_fixture,
                ContentDigest::of(b"fifth digest"),
                b"fifth bytes",
            ),
        ];
        let mut merge = FiveWayMerge::open(&conflicting, None, &[0, 1, 2, 3, 4]).unwrap();
        let mut refused = false;
        loop {
            match merge.next_unique() {
                Err(PackedPatriciaError::PathMismatch(digest)) if digest == indexed_digest => {
                    refused = true;
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => panic!("unexpected five-way merge error: {error:?}"),
            }
        }
        assert!(refused, "conflicting duplicate reached no refusal");
    }

    #[test]
    fn staged_stream_pack_truncation_and_tamper_fail_before_publication() {
        fn staged(fixture: &Fixture) -> PlannedStreamingPatriciaPack {
            let entries = representative_nodes_in_family("staged-damage", 8);
            let mut work = PackedPatriciaPublicationWork::default();
            let shape = plan_delta_stream(None, &entries, &mut work)
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            stage_delta_partition(&fixture.dir, None, &entries, shape).unwrap()
        }

        let truncated_fixture = Fixture::new("stream-staged-truncated");
        let truncated = staged(&truncated_fixture);
        let truncated_path = fs::read_dir(&truncated_fixture.path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".tmp-")
            })
            .unwrap();
        let descriptor = truncated.descriptor();
        fs::OpenOptions::new()
            .write(true)
            .open(&truncated_path)
            .unwrap()
            .set_len(u64::from(descriptor.bytes) - 1)
            .unwrap();
        let PlannedStreamingPatriciaPack::Staged { publication, .. } = truncated else {
            unreachable!()
        };
        assert!(matches!(
            validate_staged_stream_pack(&publication, descriptor),
            Err(PackedPatriciaError::Filesystem(
                FilesystemError::StoredLengthMismatch { .. }
            ))
        ));

        let tampered_fixture = Fixture::new("stream-staged-tampered");
        let tampered = staged(&tampered_fixture);
        let tampered_path = fs::read_dir(&tampered_fixture.path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".tmp-")
            })
            .unwrap();
        let descriptor = tampered.descriptor();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(&tampered_path)
            .unwrap();
        file.seek(SeekFrom::Start(u64::from(descriptor.bytes) - 1))
            .unwrap();
        file.write_all(&[0xff]).unwrap();
        file.sync_all().unwrap();
        let PlannedStreamingPatriciaPack::Staged { publication, .. } = tampered else {
            unreachable!()
        };
        assert!(matches!(
            validate_staged_stream_pack(&publication, descriptor),
            Err(PackedPatriciaError::PathMismatch(digest)) if digest == descriptor.digest
        ));
    }

    #[test]
    fn schema_two_is_deterministic_bounded_and_exactly_reopenable() {
        let nodes = representative_nodes(257);
        let publication = PackedPatriciaPublication::build(&nodes).unwrap();
        let same = PackedPatriciaPublication::build(&nodes).unwrap();
        assert_eq!(publication.bytes(), same.bytes());
        assert_eq!(publication.digest(), same.digest());
        assert!(publication.bytes().len() <= MAX_PACK_BYTES);
        assert_eq!(&publication.bytes()[..8], PACK_MAGIC);
        assert_eq!(PACK_SCHEMA_VERSION, 2);
        assert_eq!(
            read_u32(publication.bytes(), 8).unwrap(),
            PACK_SCHEMA_VERSION
        );
        assert_eq!(read_u32(publication.bytes(), 12).unwrap(), 257);
        assert_eq!(
            read_u32(publication.bytes(), 16).unwrap() as usize,
            nodes.values().map(Vec::len).sum::<usize>()
        );

        let reopened =
            PackedPatriciaPack::decode(publication.digest(), publication.bytes.clone()).unwrap();
        assert_eq!(reopened.len(), nodes.len());
        for (digest, expected) in nodes {
            assert_eq!(reopened.get(digest), Some(expected.as_slice()));
        }
    }

    #[test]
    fn schema_one_reopens_at_legacy_entry_boundary_and_rejects_the_next_entry() {
        let boundary_nodes =
            representative_nodes_in_family("legacy-boundary", LEGACY_MAX_PACK_ENTRIES);
        let boundary = publication_with_schema(&boundary_nodes, LEGACY_PACK_SCHEMA_VERSION);
        let reopened =
            PackedPatriciaPack::decode(boundary.digest(), boundary.bytes.clone()).unwrap();
        assert_eq!(reopened.len(), boundary_nodes.len());
        for (digest, expected) in boundary_nodes {
            assert_eq!(reopened.get(digest), Some(expected.as_slice()));
        }

        let over_boundary_nodes =
            representative_nodes_in_family("legacy-over-boundary", LEGACY_MAX_PACK_ENTRIES + 1);
        let over_boundary =
            publication_with_schema(&over_boundary_nodes, LEGACY_PACK_SCHEMA_VERSION);
        assert!(matches!(
            PackedPatriciaPack::decode(over_boundary.digest(), over_boundary.bytes),
            Err(PackedPatriciaError::Malformed)
        ));
    }

    #[test]
    fn schema_two_crosses_legacy_entry_boundary_and_reopens_exactly() {
        let nodes = representative_nodes_in_family("schema-two-dense", LEGACY_MAX_PACK_ENTRIES + 1);
        let publication = PackedPatriciaPublication::build(&nodes).unwrap();
        assert_eq!(
            read_u32(publication.bytes(), 8).unwrap(),
            PACK_SCHEMA_VERSION
        );
        assert!(nodes.len() > LEGACY_MAX_PACK_ENTRIES);

        let reopened =
            PackedPatriciaPack::decode(publication.digest(), publication.bytes.clone()).unwrap();
        assert_eq!(reopened.len(), nodes.len());
        for (digest, expected) in nodes {
            assert_eq!(reopened.get(digest), Some(expected.as_slice()));
        }
    }

    #[test]
    fn catalog_mixes_schema_one_and_two_packs_and_reopens_exactly() {
        let fixture = Fixture::new("mixed-pack-schemas");
        let legacy_nodes = representative_nodes_in_family("mixed-legacy", 64);
        let current_nodes = representative_nodes_in_family("mixed-current", 65);
        let legacy = publication_with_schema(&legacy_nodes, LEGACY_PACK_SCHEMA_VERSION);
        let current = PackedPatriciaPublication::build(&current_nodes).unwrap();
        assert_eq!(
            read_u32(legacy.bytes(), 8).unwrap(),
            LEGACY_PACK_SCHEMA_VERSION
        );
        assert_eq!(read_u32(current.bytes(), 8).unwrap(), PACK_SCHEMA_VERSION);
        assert!(legacy.filename().ends_with(PACK_SUFFIX));
        assert!(current.filename().ends_with(PACK_SUFFIX));
        assert_ne!(legacy.filename(), current.filename());

        let packs = [
            legacy.publish(&fixture.dir, &ExactPublisher).unwrap(),
            current.publish(&fixture.dir, &ExactPublisher).unwrap(),
        ];
        let descriptors = packs
            .iter()
            .map(PublishedPatriciaPack::descriptor)
            .collect::<Vec<_>>();
        let catalog = PackedPatriciaCatalogPublication::build_descriptors(&descriptors).unwrap();
        let catalog = catalog.publish(&fixture.dir, &ExactPublisher).unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();

        let reopened = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();
        for (digest, expected) in legacy_nodes.iter().chain(&current_nodes) {
            assert_eq!(reopened.get(*digest), Some(expected.as_slice()));
        }
    }

    #[test]
    fn packed_and_legacy_loose_publications_are_byte_differential_with_lower_fanout() {
        const REPRESENTATIVE_NODES: usize = 512;

        let loose = Fixture::new("loose-differential");
        let packed = Fixture::new("packed-differential");
        let nodes = representative_nodes(REPRESENTATIVE_NODES);
        publish_loose(&loose, &nodes);

        let publication = PackedPatriciaPublication::build(&nodes).unwrap();
        let completed = publication.publish(&packed.dir, &ExactPublisher).unwrap();
        assert_eq!(completed.digest(), publication.digest());
        let reopened = PackedPatriciaPack::open(&packed.dir, completed.digest()).unwrap();

        for (digest, expected) in &nodes {
            let loose_bytes = crate::read_required_regular(
                &loose.dir,
                &format!("{digest}.patricia-node"),
                MAX_PACK_ENTRY_BYTES as u64,
                None,
            )
            .unwrap();
            assert_eq!(reopened.get(*digest), Some(loose_bytes.as_slice()));
            assert_eq!(&loose_bytes, expected);
        }
        assert_eq!(
            fs::read_dir(&loose.path).unwrap().count(),
            REPRESENTATIVE_NODES
        );
        assert_eq!(fs::read_dir(&packed.path).unwrap().count(), 1);
    }

    #[test]
    fn failed_atomic_publication_cannot_produce_completion_evidence() {
        let fixture = Fixture::new("failed-publication");
        let publication = PackedPatriciaPublication::build(&representative_nodes(32)).unwrap();
        let publisher = FailingPublisher {
            calls: AtomicUsize::new(0),
        };

        assert!(matches!(
            publication.publish(&fixture.dir, &publisher),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert_eq!(publisher.calls.load(Ordering::Relaxed), 1);
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 0);

        let completed = publication.publish(&fixture.dir, &ExactPublisher).unwrap();
        assert_eq!(completed.digest(), publication.digest());
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);
        assert!(PackedPatriciaPack::open(&fixture.dir, completed.digest()).is_ok());

        fs::write(fixture.path.join(publication.filename()), b"conflict").unwrap();
        assert!(matches!(
            publication.publish(&fixture.dir, &ExactPublisher),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert_eq!(
            fs::read(fixture.path.join(publication.filename())).unwrap(),
            b"conflict"
        );
    }

    #[test]
    fn retry_recovers_an_orphan_left_by_a_crash_after_atomic_commit() {
        let fixture = Fixture::new("post-commit-crash");
        let publication = PackedPatriciaPublication::build(&representative_nodes(32)).unwrap();
        let publisher = CrashAfterCommitPublisher {
            fail_once: AtomicBool::new(true),
        };

        assert!(matches!(
            publication.publish(&fixture.dir, &publisher),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);

        let completed = publication.publish(&fixture.dir, &publisher).unwrap();
        assert_eq!(completed.digest(), publication.digest());
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), 1);
        assert!(PackedPatriciaPack::open(&fixture.dir, completed.digest()).is_ok());
    }

    #[test]
    fn catalog_head_is_canonical_direct_bounded_discovery() {
        const NODES: usize = 1_024;
        const PACKS: usize = 4;
        const UNRELATED_LIFETIME_FILES: usize = 2_000;

        let fixture = Fixture::new("catalog-direct-discovery");
        let nodes = representative_nodes(NODES);
        let pack_digests = publish_cataloged(&fixture, &nodes, NODES / PACKS);
        for index in 0..UNRELATED_LIFETIME_FILES {
            fs::write(
                fixture.path.join(format!("unrelated-history-{index:04}")),
                b"ignored",
            )
            .unwrap();
        }

        let catalog = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();
        assert_eq!(catalog.pack_count(), PACKS);
        assert_eq!(pack_digests.len(), PACKS);
        for (digest, expected) in &nodes {
            assert_eq!(catalog.get(*digest), Some(expected.as_slice()));
        }
        assert_eq!(
            fs::read_dir(&fixture.path).unwrap().count(),
            UNRELATED_LIFETIME_FILES + PACKS + 3,
            "four packs, one catalog, one fixed head, and one stable lock replace 1,024 loose files"
        );

        let head = fs::read(fixture.path.join(HEAD_FILENAME)).unwrap();
        assert_eq!(head.len(), HEAD_BYTES);
        assert_eq!(&head[..8], HEAD_MAGIC);
        assert_eq!(read_u32(&head, 8).unwrap(), HEAD_SCHEMA_VERSION);

        let too_many_fixture = Fixture::new("catalog-pack-bound");
        let mut published = Vec::new();
        for entry in representative_nodes(MAX_CATALOG_PACKS + 1) {
            let publication = PackedPatriciaPublication::build(&BTreeMap::from([entry])).unwrap();
            published.push(
                publication
                    .publish(&too_many_fixture.dir, &ExactPublisher)
                    .unwrap(),
            );
        }
        assert!(matches!(
            PackedPatriciaCatalogPublication::build(&published),
            Err(PackedPatriciaError::TooManyEntries)
        ));
    }

    #[test]
    fn schema_two_layers_accept_exact_duplicates_and_legacy_catalogs_still_reopen() {
        let fixture = Fixture::new("catalog-layered-duplicates");
        let duplicate_bytes = b"exact duplicate node".to_vec();
        let duplicate_digest = ContentDigest::of(&duplicate_bytes);
        let old_only = b"old layer node".to_vec();
        let new_only = b"new layer node".to_vec();
        let old_entries = BTreeMap::from([
            (duplicate_digest, duplicate_bytes.clone()),
            (ContentDigest::of(&old_only), old_only.clone()),
        ]);
        let new_entries = BTreeMap::from([
            (duplicate_digest, duplicate_bytes.clone()),
            (ContentDigest::of(&new_only), new_only.clone()),
        ]);
        let old_pack = PackedPatriciaPublication::build(&old_entries)
            .unwrap()
            .publish(&fixture.dir, &ExactPublisher)
            .unwrap();
        let new_pack = PackedPatriciaPublication::build(&new_entries)
            .unwrap()
            .publish(&fixture.dir, &ExactPublisher)
            .unwrap();
        let descriptors = [old_pack, new_pack]
            .iter()
            .map(|pack| PackDescriptor {
                digest: pack.digest,
                first: pack.first,
                last: pack.last,
                entries: pack.entries,
                bytes: pack.bytes,
            })
            .collect::<Vec<_>>();
        let catalog = PackedPatriciaCatalogPublication::build_descriptors_for_schema(
            &descriptors,
            LAYERED_CATALOG_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(
            read_u32(&catalog.bytes, 8).unwrap(),
            LAYERED_CATALOG_SCHEMA_VERSION
        );
        let catalog = catalog.publish(&fixture.dir, &ExactPublisher).unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();
        let opened = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();
        assert_eq!(
            opened.get(duplicate_digest),
            Some(duplicate_bytes.as_slice())
        );
        assert_eq!(
            opened.get(ContentDigest::of(&old_only)),
            Some(old_only.as_slice())
        );
        assert_eq!(
            opened.get(ContentDigest::of(&new_only)),
            Some(new_only.as_slice())
        );

        let collision_digest = ContentDigest::of(b"synthetic collision");
        let synthetic_pack = |pack_digest, payload: u8| {
            let payload_start = PACK_HEADER_BYTES + PACK_INDEX_ENTRY_BYTES;
            let mut bytes = vec![0_u8; payload_start];
            bytes[PACK_HEADER_BYTES..PACK_HEADER_BYTES + 32]
                .copy_from_slice(collision_digest.as_bytes());
            bytes[PACK_HEADER_BYTES + 36..PACK_HEADER_BYTES + 40]
                .copy_from_slice(&1_u32.to_le_bytes());
            bytes.push(payload);
            PackedPatriciaPack {
                digest: pack_digest,
                bytes,
                entry_count: 1,
                payload_start,
            }
        };
        let conflicting = [
            synthetic_pack(ContentDigest::of(b"pack one"), b'a'),
            synthetic_pack(ContentDigest::of(b"pack two"), b'b'),
        ];
        assert!(matches!(
            validate_duplicate_nodes(&conflicting),
            Err(PackedPatriciaError::PathMismatch(digest)) if digest == collision_digest
        ));

        let legacy_fixture = Fixture::new("catalog-legacy-schema-one");
        let legacy_nodes = representative_nodes(32);
        let legacy_pack = PackedPatriciaPublication::build(&legacy_nodes)
            .unwrap()
            .publish(&legacy_fixture.dir, &ExactPublisher)
            .unwrap();
        let mut legacy = PackedPatriciaCatalogPublication::build(&[legacy_pack]).unwrap();
        legacy.bytes[8..12].copy_from_slice(&LEGACY_CATALOG_SCHEMA_VERSION.to_le_bytes());
        legacy.digest = ContentDigest::of(&legacy.bytes);
        let legacy = legacy
            .publish(&legacy_fixture.dir, &ExactPublisher)
            .unwrap();
        publish_catalog_head(&legacy_fixture.dir, &legacy, &ExactPublisher).unwrap();
        let reopened = PackedPatriciaCatalog::discover(&legacy_fixture.dir)
            .unwrap()
            .unwrap();
        for (digest, bytes) in legacy_nodes {
            assert_eq!(reopened.get(digest), Some(bytes.as_slice()));
        }
    }

    #[test]
    fn one_append_to_maximally_overfull_schema_two_compacts_only_five_oldest_packs() {
        let fixture = Fixture::new("catalog-schema-two-bounded-normalization");
        let mut expected = BTreeMap::new();
        let mut oldest_group = BTreeMap::new();
        let mut published = Vec::new();
        let mut single_pack_bytes = None;
        for index in 0..MAX_CATALOG_PACKS {
            let bytes = format!("overfull historical node {index:03}").into_bytes();
            let digest = ContentDigest::of(&bytes);
            expected.insert(digest, bytes.clone());
            if index <= MAX_PACKS_PER_SIZE_TIER {
                oldest_group.insert(digest, bytes.clone());
            }
            let publication =
                PackedPatriciaPublication::build(&BTreeMap::from([(digest, bytes)])).unwrap();
            assert_eq!(
                *single_pack_bytes.get_or_insert(publication.bytes().len()),
                publication.bytes().len()
            );
            published.push(publication.publish(&fixture.dir, &ExactPublisher).unwrap());
        }
        let descriptors = published
            .iter()
            .map(PublishedPatriciaPack::descriptor)
            .collect::<Vec<_>>();
        let catalog = PackedPatriciaCatalogPublication::build_descriptors_for_schema(
            &descriptors,
            LAYERED_CATALOG_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(
            read_u32(&catalog.bytes, 8).unwrap(),
            LAYERED_CATALOG_SCHEMA_VERSION
        );
        let catalog = catalog.publish(&fixture.dir, &ExactPublisher).unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();
        let current = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();

        let bytes = b"overfull historical node 256".to_vec();
        let digest = ContentDigest::of(&bytes);
        expected.insert(digest, bytes.clone());
        let (next, work) = append_catalog(
            &fixture,
            Some(current),
            &BTreeMap::from([(digest, bytes)]),
            &ExactPublisher,
        )
        .unwrap();

        assert_eq!(
            work.compaction_pack_bytes_selected,
            5 * single_pack_bytes.unwrap()
        );
        assert_eq!(work.compaction_packs_selected, 5);
        assert_eq!(
            work.compaction_payload_bytes_reencoded,
            5 * b"overfull historical node 000".len()
        );
        assert_eq!(
            next.descriptors.first().copied(),
            Some(
                PackedPatriciaPublication::build(&oldest_group)
                    .unwrap()
                    .descriptor()
            ),
            "the carry must replace the fixed oldest group"
        );
        assert_eq!(next.pack_count(), MAX_CATALOG_PACKS - 3);
        assert_eq!(
            catalog_schema_version(&fixture, &next),
            LAYERED_CATALOG_SCHEMA_VERSION
        );
        for (digest, bytes) in &expected {
            assert_eq!(next.get(*digest), Some(bytes.as_slice()));
        }
        drop(next);
        let mut current = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();
        for (digest, bytes) in &expected {
            assert_eq!(current.get(*digest), Some(bytes.as_slice()));
        }

        for index in (MAX_CATALOG_PACKS + 1)..(MAX_CATALOG_PACKS + 129) {
            if catalog_schema_version(&fixture, &current) == CATALOG_SCHEMA_VERSION {
                break;
            }
            let bytes = format!("overfull historical node {index:03}").into_bytes();
            let digest = ContentDigest::of(&bytes);
            expected.insert(digest, bytes.clone());
            let (next, work) = append_catalog(
                &fixture,
                Some(current),
                &BTreeMap::from([(digest, bytes)]),
                &ExactPublisher,
            )
            .unwrap();
            assert!(work.compaction_packs_selected > 0);
            assert_eq!(
                work.compaction_packs_selected % (MAX_PACKS_PER_SIZE_TIER + 1),
                0
            );
            assert!(
                work.compaction_packs_selected
                    <= (MAX_PACKS_PER_SIZE_TIER + 1) * PACK_SIZE_TIER_COUNT
            );
            current = next;
        }
        assert_eq!(
            catalog_schema_version(&fixture, &current),
            CATALOG_SCHEMA_VERSION,
            "bounded mutations must eventually certify the schema-3 invariant"
        );
        assert!(size_tiers_are_bounded(&current.descriptors));
        for (digest, bytes) in &expected {
            assert_eq!(current.get(*digest), Some(bytes.as_slice()));
        }
        drop(current);

        let reopened = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();
        assert_eq!(
            catalog_schema_version(&fixture, &reopened),
            CATALOG_SCHEMA_VERSION
        );
        for (digest, bytes) in expected {
            assert_eq!(reopened.get(digest), Some(bytes.as_slice()));
        }
    }

    #[test]
    fn schema_three_rejects_an_overfull_derived_size_tier() {
        let fixture = Fixture::new("catalog-schema-three-tier-invariant");
        let mut published = Vec::new();
        for index in 0..=MAX_PACKS_PER_SIZE_TIER {
            let bytes = format!("schema three invalid tier {index}").into_bytes();
            let digest = ContentDigest::of(&bytes);
            let publication =
                PackedPatriciaPublication::build(&BTreeMap::from([(digest, bytes)])).unwrap();
            published.push(publication.publish(&fixture.dir, &ExactPublisher).unwrap());
        }
        let descriptors = published
            .iter()
            .map(PublishedPatriciaPack::descriptor)
            .collect::<Vec<_>>();
        let mut catalog = PackedPatriciaCatalogPublication::build_descriptors_for_schema(
            &descriptors,
            LAYERED_CATALOG_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(
            read_u32(&catalog.bytes, 8).unwrap(),
            LAYERED_CATALOG_SCHEMA_VERSION
        );
        catalog.bytes[8..12].copy_from_slice(&CATALOG_SCHEMA_VERSION.to_le_bytes());
        catalog.digest = ContentDigest::of(&catalog.bytes);
        let catalog = catalog.publish(&fixture.dir, &ExactPublisher).unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();

        assert!(matches!(
            PackedPatriciaCatalog::discover(&fixture.dir),
            Err(PackedPatriciaError::Malformed)
        ));
    }

    #[test]
    fn many_tiny_deltas_stay_tier_bounded_reopen_exactly_and_have_bounded_amortized_work() {
        const DELTAS: usize = 160;

        let fixture = Fixture::new("catalog-size-tiered-deltas");
        let mut current = None;
        let mut expected = BTreeMap::new();
        let mut delta_pack_bytes = 0_usize;
        let mut selected_pack_bytes = 0_usize;
        for index in 0..DELTAS {
            let bytes = format!("tiny historical node {index:04}").into_bytes();
            let digest = ContentDigest::of(&bytes);
            let entries = BTreeMap::from([(digest, bytes.clone())]);
            let (next, work) =
                append_catalog(&fixture, current, &entries, &ExactPublisher).unwrap();
            delta_pack_bytes += work.delta_pack_bytes_encoded;
            selected_pack_bytes += work.compaction_pack_bytes_selected;
            expected.insert(digest, bytes);

            let mut tiers = [0_usize; PACK_SIZE_TIER_COUNT];
            for descriptor in &next.descriptors {
                tiers[pack_size_tier(descriptor.bytes)] += 1;
            }
            assert!(tiers.iter().all(|count| *count <= MAX_PACKS_PER_SIZE_TIER));
            assert!(next.pack_count() <= MAX_PACKS_PER_SIZE_TIER * PACK_SIZE_TIER_COUNT);
            assert_eq!(
                catalog_schema_version(&fixture, &next),
                CATALOG_SCHEMA_VERSION
            );
            current = Some(next);
        }

        assert!(selected_pack_bytes > delta_pack_bytes);
        assert!(
            selected_pack_bytes <= delta_pack_bytes * AMORTIZED_SELECTED_BYTE_FACTOR,
            "selected canonical bytes must satisfy the explicit lifetime tier bound"
        );
        let current = current.unwrap();
        assert!(current.pack_count() < DELTAS / 4);
        for (digest, bytes) in &expected {
            assert_eq!(current.get(*digest), Some(bytes.as_slice()));
        }
        drop(current);

        let reopened = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap();
        for (digest, bytes) in expected {
            assert_eq!(reopened.get(digest), Some(bytes.as_slice()));
        }
    }

    #[test]
    fn entry_dense_tier_carries_cross_the_old_pack_count_ceiling() {
        const DELTAS: usize = 20_000;

        let mut planned = Vec::new();
        let mut work = PackedPatriciaPublicationWork::default();
        for index in 0..DELTAS as u32 {
            let bytes = index.to_le_bytes().to_vec();
            let entries = BTreeMap::from([(ContentDigest::of(&bytes), bytes)]);
            let publication = PackedPatriciaPublication::build(&entries).unwrap();
            work.delta_pack_bytes_encoded += publication.bytes.len();
            work.pack_bytes_encoded += publication.bytes.len();
            planned.push(PlannedPatriciaPack::Publication(publication));
            compact_size_tiers(None, &mut planned, &mut work, None, 0).unwrap();
        }

        let mut tiers = [0_usize; PACK_SIZE_TIER_COUNT];
        let mut entries = 0_usize;
        for pack in &planned {
            let descriptor = pack.descriptor();
            tiers[pack_size_tier(descriptor.bytes)] += 1;
            entries += descriptor.entries as usize;
        }
        assert_eq!(entries, DELTAS);
        assert!(tiers.iter().all(|count| *count <= MAX_PACKS_PER_SIZE_TIER));
        assert!(planned.len() <= MAX_PACKS_PER_SIZE_TIER * PACK_SIZE_TIER_COUNT);
        assert!(
            work.compaction_pack_bytes_selected
                <= work.delta_pack_bytes_encoded * AMORTIZED_SELECTED_BYTE_FACTOR
        );
    }

    struct BoundaryCrashPublisher {
        calls: AtomicUsize,
        fail_at: usize,
        after_commit: bool,
    }

    impl PatriciaNodePublisher for BoundaryCrashPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call == self.fail_at && !self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected crash before immutable commit",
                ));
            }
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)?;
            if call == self.fail_at {
                return Err(PatriciaPublicationError::new(
                    "injected crash after immutable commit",
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn compacted_pack_and_catalog_crash_cuts_leave_old_authority_and_retry_exactly() {
        for (fail_at, after_commit) in [(1, false), (1, true), (2, false), (2, true)] {
            let fixture = Fixture::new(&format!(
                "catalog-compaction-crash-{fail_at}-{after_commit}"
            ));
            let mut current = None;
            let mut historical = BTreeMap::new();
            for index in 0..MAX_PACKS_PER_SIZE_TIER {
                let bytes = format!("same-tier historical node {index}").into_bytes();
                let digest = ContentDigest::of(&bytes);
                historical.insert(digest, bytes.clone());
                let entries = BTreeMap::from([(digest, bytes)]);
                current = Some(
                    append_catalog(&fixture, current, &entries, &ExactPublisher)
                        .unwrap()
                        .0,
                );
            }
            let current = current.unwrap();
            assert_eq!(current.pack_count(), MAX_PACKS_PER_SIZE_TIER);
            let old_authority = current.authority();
            let new_bytes = b"same-tier historical node new carry".to_vec();
            let new_digest = ContentDigest::of(&new_bytes);
            let entries = BTreeMap::from([(new_digest, new_bytes.clone())]);
            let publisher = BoundaryCrashPublisher {
                calls: AtomicUsize::new(0),
                fail_at,
                after_commit,
            };

            assert!(matches!(
                publish_appended_catalog(
                    &fixture.dir,
                    &publisher,
                    Some(&current),
                    &entries,
                    MAX_CATALOG_PACK_BYTES,
                ),
                Err(PackedPatriciaError::Publication(_))
            ));
            let still_old = PackedPatriciaCatalog::discover(&fixture.dir)
                .unwrap()
                .unwrap();
            assert_eq!(still_old.authority(), old_authority);
            assert_eq!(still_old.get(new_digest), None);
            for (digest, bytes) in &historical {
                assert_eq!(still_old.get(*digest), Some(bytes.as_slice()));
            }
            drop(still_old);

            let (pending, _) = publish_appended_catalog(
                &fixture.dir,
                &publisher,
                Some(&current),
                &entries,
                MAX_CATALOG_PACK_BYTES,
            )
            .unwrap();
            let pending = pending.unwrap();
            assert_eq!(
                PackedPatriciaCatalog::discover(&fixture.dir)
                    .unwrap()
                    .unwrap()
                    .authority(),
                old_authority
            );
            transition_catalog_head(
                &fixture.dir,
                Some(old_authority),
                pending.published_catalog(),
            )
            .unwrap();
            transition_catalog_head(
                &fixture.dir,
                Some(old_authority),
                pending.published_catalog(),
            )
            .unwrap();
            let compacted = pending.finish(Some(current));
            assert_eq!(compacted.get(new_digest), Some(new_bytes.as_slice()));
            for (digest, bytes) in historical {
                assert_eq!(compacted.get(digest), Some(bytes.as_slice()));
            }
        }
    }

    #[test]
    fn compaction_capacity_preflight_publishes_no_immutable_orphan() {
        let fixture = Fixture::new("catalog-compaction-capacity-preflight");
        let mut current = None;
        for index in 0..MAX_PACKS_PER_SIZE_TIER {
            let bytes = format!("capacity tier node {index}").into_bytes();
            let entries = BTreeMap::from([(ContentDigest::of(&bytes), bytes)]);
            current = Some(
                append_catalog(&fixture, current, &entries, &ExactPublisher)
                    .unwrap()
                    .0,
            );
        }
        let current = current.unwrap();
        let files_before = fs::read_dir(&fixture.path).unwrap().count();
        let bytes = b"capacity tier node carry".to_vec();
        let entries = BTreeMap::from([(ContentDigest::of(&bytes), bytes)]);
        let publisher = FailingPublisher {
            calls: AtomicUsize::new(0),
        };
        assert!(matches!(
            publish_appended_catalog(&fixture.dir, &publisher, Some(&current), &entries, 1),
            Err(PackedPatriciaError::PackTooLarge)
        ));
        assert_eq!(publisher.calls.load(Ordering::Relaxed), 0);
        assert_eq!(fs::read_dir(&fixture.path).unwrap().count(), files_before);
        assert_eq!(
            PackedPatriciaCatalog::discover(&fixture.dir)
                .unwrap()
                .unwrap()
                .authority(),
            current.authority()
        );
    }

    #[test]
    fn pack_catalog_and_head_crash_windows_are_invisible_or_retry_exactly() {
        let fixture = Fixture::new("catalog-crash-windows");
        let publication = PackedPatriciaPublication::build(&representative_nodes(32)).unwrap();
        let pack = publication.publish(&fixture.dir, &ExactPublisher).unwrap();
        let catalog = PackedPatriciaCatalogPublication::build(&[pack]).unwrap();

        let fail_before_catalog = FailingPublisher {
            calls: AtomicUsize::new(0),
        };
        assert!(matches!(
            catalog.publish(&fixture.dir, &fail_before_catalog),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert!(PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .is_none());

        let crash_after_catalog = CrashAfterCommitPublisher {
            fail_once: AtomicBool::new(true),
        };
        assert!(matches!(
            catalog.publish(&fixture.dir, &crash_after_catalog),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert!(PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .is_none());
        let catalog = catalog.publish(&fixture.dir, &crash_after_catalog).unwrap();

        let fail_before_head = FailingPublisher {
            calls: AtomicUsize::new(0),
        };
        assert!(matches!(
            publish_catalog_head(&fixture.dir, &catalog, &fail_before_head),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert!(PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .is_none());

        let crash_after_head = CrashAfterCommitPublisher {
            fail_once: AtomicBool::new(true),
        };
        assert!(matches!(
            publish_catalog_head(&fixture.dir, &catalog, &crash_after_head),
            Err(PackedPatriciaError::Publication(_))
        ));
        assert!(PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .is_some());
        publish_catalog_head(&fixture.dir, &catalog, &crash_after_head).unwrap();
        assert!(PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .is_some());
    }

    #[test]
    fn catalog_rejects_absent_or_tampered_named_pack_before_exposing_nodes() {
        let fixture = Fixture::new("catalog-pack-corruption");
        let nodes = representative_nodes(32);
        let publication = PackedPatriciaPublication::build(&nodes).unwrap();
        let pack_filename = publication.filename();
        let pack_bytes = publication.bytes().to_vec();
        let pack = publication.publish(&fixture.dir, &ExactPublisher).unwrap();
        let catalog = PackedPatriciaCatalogPublication::build(&[pack]).unwrap();
        let catalog = catalog.publish(&fixture.dir, &ExactPublisher).unwrap();
        publish_catalog_head(&fixture.dir, &catalog, &ExactPublisher).unwrap();

        fs::remove_file(fixture.path.join(&pack_filename)).unwrap();
        assert!(matches!(
            PackedPatriciaCatalog::discover(&fixture.dir),
            Err(PackedPatriciaError::Filesystem(_))
        ));

        publish_immutable_exact(&fixture.dir, &pack_filename, &pack_bytes).unwrap();
        let last = pack_bytes.len() - 1;
        let mut tampered = pack_bytes;
        tampered[last] ^= 1;
        fs::write(fixture.path.join(pack_filename), tampered).unwrap();
        assert!(matches!(
            PackedPatriciaCatalog::discover(&fixture.dir),
            Err(PackedPatriciaError::PathMismatch(_))
        ));
    }

    #[test]
    fn replacement_head_checks_authenticated_prior_and_retries_identical_target() {
        let fixture = Fixture::new("catalog-head-replacement");
        let first_nodes = representative_nodes(32);
        let (first, _) =
            publish_partitioned_catalog(&fixture.dir, &ExactPublisher, &first_nodes, usize::MAX)
                .unwrap();
        transition_catalog_head(&fixture.dir, None, &first).unwrap();
        let first_authority = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap()
            .authority();

        let mut second_nodes = first_nodes.clone();
        let second_bytes = b"second catalog exact node".to_vec();
        second_nodes.insert(ContentDigest::of(&second_bytes), second_bytes);
        let (second, _) =
            publish_partitioned_catalog(&fixture.dir, &ExactPublisher, &second_nodes, usize::MAX)
                .unwrap();

        // A crash before the transition leaves the authenticated old head.
        assert_eq!(
            PackedPatriciaCatalog::discover(&fixture.dir)
                .unwrap()
                .unwrap()
                .authority(),
            first_authority
        );
        transition_catalog_head(&fixture.dir, Some(first_authority), &second).unwrap();

        // A crash immediately after replacement is retried with stale prior
        // evidence; the already-installed identical target is success.
        transition_catalog_head(&fixture.dir, Some(first_authority), &second).unwrap();
        let second_authority = PackedPatriciaCatalog::discover(&fixture.dir)
            .unwrap()
            .unwrap()
            .authority();
        assert_ne!(second_authority, first_authority);

        let mut third_nodes = second_nodes;
        let third_bytes = b"conflicting third catalog exact node".to_vec();
        third_nodes.insert(ContentDigest::of(&third_bytes), third_bytes);
        let (third, _) =
            publish_partitioned_catalog(&fixture.dir, &ExactPublisher, &third_nodes, usize::MAX)
                .unwrap();
        assert!(matches!(
            transition_catalog_head(&fixture.dir, Some(first_authority), &third),
            Err(PackedPatriciaError::UnexpectedHead)
        ));
        assert_eq!(
            PackedPatriciaCatalog::discover(&fixture.dir)
                .unwrap()
                .unwrap()
                .authority(),
            second_authority
        );
    }

    #[test]
    fn truncation_tampering_noncanonical_indexes_and_wrong_names_fail_closed() {
        let publication = PackedPatriciaPublication::build(&representative_nodes(3)).unwrap();

        let mut truncated = publication.bytes().to_vec();
        truncated.pop();
        let truncated_digest = ContentDigest::of(&truncated);
        assert!(matches!(
            PackedPatriciaPack::decode(truncated_digest, truncated),
            Err(PackedPatriciaError::Malformed)
        ));

        let mut tampered = publication.bytes().to_vec();
        *tampered.last_mut().unwrap() ^= 1;
        let tampered_digest = ContentDigest::of(&tampered);
        assert!(matches!(
            PackedPatriciaPack::decode(tampered_digest, tampered),
            Err(PackedPatriciaError::PathMismatch(_))
        ));

        let mut gapped = publication.bytes().to_vec();
        gapped[PACK_HEADER_BYTES + 32..PACK_HEADER_BYTES + 36]
            .copy_from_slice(&1_u32.to_le_bytes());
        let gapped_digest = ContentDigest::of(&gapped);
        assert!(matches!(
            PackedPatriciaPack::decode(gapped_digest, gapped),
            Err(PackedPatriciaError::Malformed)
        ));

        assert!(matches!(
            PackedPatriciaPack::decode(ContentDigest::of(b"wrong name"), publication.bytes),
            Err(PackedPatriciaError::PathMismatch(_))
        ));
    }

    #[test]
    fn construction_limits_reject_before_publication() {
        let too_many = representative_nodes(MAX_PACK_ENTRIES + 1);
        assert!(matches!(
            PackedPatriciaPublication::build(&too_many),
            Err(PackedPatriciaError::TooManyEntries)
        ));

        let oversized = vec![0x5a; MAX_PACK_ENTRY_BYTES + 1];
        let oversized = BTreeMap::from([(ContentDigest::of(&oversized), oversized)]);
        assert!(matches!(
            PackedPatriciaPublication::build(&oversized),
            Err(PackedPatriciaError::EntryTooLarge(_))
        ));

        let aggregate_oversized = (0..128_u16)
            .map(|index| {
                let mut bytes = vec![0x5a; MAX_PACK_ENTRY_BYTES];
                bytes[..2].copy_from_slice(&index.to_le_bytes());
                (ContentDigest::of(&bytes), bytes)
            })
            .collect();
        assert!(matches!(
            PackedPatriciaPublication::build(&aggregate_oversized),
            Err(PackedPatriciaError::PackTooLarge)
        ));
    }
}
