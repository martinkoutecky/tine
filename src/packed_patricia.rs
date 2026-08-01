//! Bounded physical packing for exact authenticated Patricia node bytes.
//!
//! Version 1 keeps Patricia roots and node encodings unchanged. Immutable
//! packs are named by their complete-byte digest; one fixed, atomically
//! published head names one immutable content-addressed catalog. Catalog
//! schema 2 is a bounded append-only sequence of immutable packs. Historical
//! pack names remain unchanged across ordinary writes; lookup examines at
//! most the fixed catalog pack bound. Discovery never scans the node directory.
//!
//! The canonical byte layout is:
//!
//! ```text
//! magic[8] = "TINEPPK\0"
//! schema_version: u32 little endian = 1
//! entry_count: u32 little endian
//! payload_bytes: u32 little endian
//! entry[entry_count] = digest[32], payload_offset: u32 LE, length: u32 LE
//! payload[payload_bytes] = exact node bytes concatenated in digest order
//! ```
//!
//! Entries are non-empty, strictly digest-sorted, and densely laid out with no
//! gaps. Every entry digest is SHA-256 of its exact payload slice. The pack
//! filename is SHA-256 of the complete canonical pack bytes followed by
//! [`PACK_SUFFIX`]. These rules make retries byte-exact and allow readers to
//! reject truncation, non-canonical indexes, entry tampering, and path/content
//! mismatch before exposing any node bytes.

use std::collections::BTreeMap;
use std::ops::Range;

use cap_std::fs::Dir;

use super::authenticated_patricia::{PatriciaNodePublisher, PatriciaPublicationError};
use super::filesystem::{
    read_optional_regular, read_required_regular, transition_regular_exact, FilesystemError,
};
use super::ContentDigest;

const PACK_MAGIC: &[u8; 8] = b"TINEPPK\0";
const PACK_SCHEMA_VERSION: u32 = 1;
const PACK_HEADER_BYTES: usize = 8 + 4 + 4 + 4;
const PACK_INDEX_ENTRY_BYTES: usize = 32 + 4 + 4;
pub(crate) const PACK_SUFFIX: &str = ".patricia-pack-v1";
const CATALOG_MAGIC: &[u8; 8] = b"TINEPCT\0";
const LEGACY_CATALOG_SCHEMA_VERSION: u32 = 1;
const CATALOG_SCHEMA_VERSION: u32 = 2;
const CATALOG_HEADER_BYTES: usize = 8 + 4 + 4 + 4 + 8;
const CATALOG_ENTRY_BYTES: usize = 32 + 32 + 32 + 4 + 4;
const CATALOG_SUFFIX: &str = ".patricia-catalog-v1";
const HEAD_MAGIC: &[u8; 8] = b"TINEPHD\0";
const HEAD_SCHEMA_VERSION: u32 = 1;
const HEAD_BYTES: usize = 8 + 4 + 32 + 4;
pub(crate) const HEAD_FILENAME: &str = "patricia-pack-head-v1";

/// Keeps construction, reads, and index allocation independent of history
/// length. Larger publication sets must be split by the future adapter before
/// any catalog/head can name them.
pub(crate) const MAX_PACK_ENTRIES: usize = 4_096;
pub(crate) const MAX_PACK_ENTRY_BYTES: usize = 128 * 1024;
pub(crate) const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;
/// A bounded delta window with useful ordinary-write capacity. Reaching this
/// limit safely falls back to loose publication. A later size-tiered compactor
/// can amortize merging a full tier without changing the catalog/head protocol.
pub(crate) const MAX_CATALOG_PACKS: usize = 256;
pub(crate) const MAX_CATALOG_PACK_BYTES: usize = 64 * 1024 * 1024;
const MAX_CATALOG_BYTES: usize = CATALOG_HEADER_BYTES + MAX_CATALOG_PACKS * CATALOG_ENTRY_BYTES;

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

impl From<FilesystemError> for PackedPatriciaError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error)
    }
}

/// Canonical bytes ready for one immutable exact publication.
pub(crate) struct PackedPatriciaPublication {
    digest: ContentDigest,
    bytes: Vec<u8>,
    first: ContentDigest,
    last: ContentDigest,
    entries: u32,
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
        for bytes in entries.values() {
            encoded.extend_from_slice(bytes);
        }
        debug_assert_eq!(encoded.len(), total_bytes);

        Ok(Self {
            digest: ContentDigest::of(&encoded),
            bytes: encoded,
            first: *entries.first_key_value().expect("validated non-empty").0,
            last: *entries.last_key_value().expect("validated non-empty").0,
            entries: entries.len() as u32,
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
}

/// One fully validated, bounded pack reopened from immutable storage.
pub(crate) struct PackedPatriciaPack {
    digest: ContentDigest,
    bytes: Vec<u8>,
    entries: BTreeMap<ContentDigest, Range<usize>>,
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
        if schema_version != PACK_SCHEMA_VERSION
            || entry_count == 0
            || entry_count > MAX_PACK_ENTRIES
        {
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

        let mut entries = BTreeMap::new();
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
            entries.insert(digest, range);
            prior_digest = Some(digest);
            expected_offset = end;
        }
        if expected_offset != payload_bytes {
            return Err(PackedPatriciaError::Malformed);
        }

        Ok(Self {
            digest: expected_digest,
            bytes,
            entries,
        })
    }

    pub(crate) fn get(&self, digest: ContentDigest) -> Option<&[u8]> {
        self.entries
            .get(&digest)
            .map(|range| &self.bytes[range.clone()])
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    fn descriptor(&self) -> PackDescriptor {
        PackDescriptor {
            digest: self.digest,
            first: *self
                .entries
                .first_key_value()
                .expect("validated non-empty")
                .0,
            last: *self
                .entries
                .last_key_value()
                .expect("validated non-empty")
                .0,
            entries: self.entries.len() as u32,
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
/// publication evidence. Schema 2 preserves descriptor order as bounded
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
        Self::build_descriptors(&descriptors)
    }

    fn build_descriptors(descriptors: &[PackDescriptor]) -> Result<Self, PackedPatriciaError> {
        if descriptors.is_empty() {
            return Err(PackedPatriciaError::Empty);
        }
        if descriptors.len() > MAX_CATALOG_PACKS {
            return Err(PackedPatriciaError::TooManyEntries);
        }
        validate_descriptors(descriptors, false)?;
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
        bytes.extend_from_slice(&CATALOG_SCHEMA_VERSION.to_le_bytes());
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
    pub(crate) pack_bytes_encoded: usize,
    pub(crate) pack_bytes_published: usize,
    pub(crate) catalog_metadata_bytes_encoded: usize,
    pub(crate) catalog_metadata_bytes_published: usize,
    pub(crate) packs_published: usize,
}

pub(crate) struct PendingPackedPatriciaCatalog {
    catalog: PublishedPatriciaCatalog,
    descriptors: Vec<PackDescriptor>,
    packs: Vec<PackedPatriciaPack>,
}

impl PendingPackedPatriciaCatalog {
    pub(crate) fn published_catalog(&self) -> &PublishedPatriciaCatalog {
        &self.catalog
    }

    /// Install the already-published delta in memory only after the fixed head
    /// transition succeeds. Historical pack payloads are moved, not cloned or
    /// reopened.
    pub(crate) fn finish(self, current: Option<PackedPatriciaCatalog>) -> PackedPatriciaCatalog {
        let (mut descriptors, mut packs) = current
            .map(|catalog| (catalog.descriptors, catalog.packs))
            .unwrap_or_default();
        descriptors.extend(self.descriptors);
        packs.extend(self.packs);
        PackedPatriciaCatalog {
            authority: PackedPatriciaHeadAuthority {
                catalog_digest: self.catalog.digest,
                catalog_bytes: self.catalog.bytes,
            },
            descriptors,
            packs,
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
/// authenticated prior head is the only replaceable authority.
pub(crate) fn transition_catalog_head(
    dir: &Dir,
    expected: Option<PackedPatriciaHeadAuthority>,
    catalog: &PublishedPatriciaCatalog,
) -> Result<(), PackedPatriciaError> {
    let expected = expected.map(PackedPatriciaHeadAuthority::encode);
    let replacement = encode_catalog_head(catalog.digest, catalog.bytes);
    transition_regular_exact(dir, HEAD_FILENAME, expected.as_deref(), &replacement).map_err(
        |error| match error {
            FilesystemError::ByteCollision => PackedPatriciaError::UnexpectedHead,
            error => PackedPatriciaError::Filesystem(error),
        },
    )
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

    fn pack_bytes(&self) -> usize {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.bytes as usize)
            .sum()
    }

    #[cfg(test)]
    fn pack_count(&self) -> usize {
        self.packs.len()
    }
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

/// Publish only new delta packs and a bounded metadata catalog. Existing pack
/// payloads are neither cloned, encoded, reopened, nor republished. Capacity is
/// checked before the first immutable publication so the caller can safely use
/// its frozen loose-node fallback without changing the catalog head.
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
    if entries.is_empty() {
        return Err(PackedPatriciaError::Empty);
    }

    let mut work = PackedPatriciaPublicationWork::default();
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
            delta.insert(*digest, bytes.clone());
        }
    }
    if delta.is_empty() {
        return Ok((None, work));
    }

    let publications = partition_publications(&delta)?;
    let new_pack_bytes = publications
        .iter()
        .try_fold(0_usize, |total, publication| {
            total
                .checked_add(publication.bytes.len())
                .ok_or(PackedPatriciaError::PackTooLarge)
        })?;
    work.pack_bytes_encoded = new_pack_bytes;
    let old_pack_count = current.map_or(0, |catalog| catalog.descriptors.len());
    let old_pack_bytes = current.map_or(0, PackedPatriciaCatalog::pack_bytes);
    if old_pack_count
        .checked_add(publications.len())
        .is_none_or(|count| count > MAX_CATALOG_PACKS)
        || old_pack_bytes
            .checked_add(new_pack_bytes)
            .is_none_or(|bytes| bytes > catalog_pack_byte_limit.min(MAX_CATALOG_PACK_BYTES))
    {
        return Err(PackedPatriciaError::PackTooLarge);
    }

    let mut descriptors = current
        .map(|catalog| catalog.descriptors.clone())
        .unwrap_or_default();
    descriptors.extend(publications.iter().map(|publication| PackDescriptor {
        digest: publication.digest,
        first: publication.first,
        last: publication.last,
        entries: publication.entries,
        bytes: publication.bytes.len() as u32,
    }));
    let catalog_publication = PackedPatriciaCatalogPublication::build_descriptors(&descriptors)?;
    work.catalog_metadata_bytes_encoded = catalog_publication.bytes.len();

    let mut published_descriptors = Vec::with_capacity(publications.len());
    for publication in &publications {
        let published = publication.publish(dir, publisher)?;
        work.pack_bytes_published = work
            .pack_bytes_published
            .checked_add(publication.bytes.len())
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        work.packs_published += 1;
        published_descriptors.push(PackDescriptor {
            digest: published.digest,
            first: published.first,
            last: published.last,
            entries: published.entries,
            bytes: published.bytes,
        });
    }
    let catalog = catalog_publication.publish(dir, publisher)?;
    work.catalog_metadata_bytes_published = catalog.bytes as usize;
    let packs = publications
        .into_iter()
        .map(PackedPatriciaPublication::into_opened)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        Some(PendingPackedPatriciaCatalog {
            catalog,
            descriptors: published_descriptors,
            packs,
        }),
        work,
    ))
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
    if schema_version != LEGACY_CATALOG_SCHEMA_VERSION && schema_version != CATALOG_SCHEMA_VERSION {
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
            || (descriptor.bytes as usize) < PACK_HEADER_BYTES
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

fn validate_duplicate_nodes(packs: &[PackedPatriciaPack]) -> Result<(), PackedPatriciaError> {
    let mut seen = BTreeMap::<ContentDigest, (usize, Range<usize>)>::new();
    for (pack_index, pack) in packs.iter().enumerate() {
        for (digest, range) in &pack.entries {
            if let Some((prior_pack_index, prior_range)) = seen.get(digest) {
                if packs[*prior_pack_index].bytes[prior_range.clone()] != pack.bytes[range.clone()]
                {
                    return Err(PackedPatriciaError::PathMismatch(*digest));
                }
            } else {
                seen.insert(*digest, (pack_index, range.clone()));
            }
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

    fn representative_nodes(count: usize) -> BTreeMap<ContentDigest, Vec<u8>> {
        (0..count)
            .map(|index| {
                let bytes = format!(
                    "node-v1/{index:04}/pages/Unicode α-{index:04}.md/值-{}",
                    index % 17
                )
                .into_bytes();
                (ContentDigest::of(&bytes), bytes)
            })
            .collect()
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

    #[test]
    fn v1_is_deterministic_bounded_and_exactly_reopenable() {
        let nodes = representative_nodes(257);
        let publication = PackedPatriciaPublication::build(&nodes).unwrap();
        let same = PackedPatriciaPublication::build(&nodes).unwrap();
        assert_eq!(publication.bytes(), same.bytes());
        assert_eq!(publication.digest(), same.digest());
        assert!(publication.bytes().len() <= MAX_PACK_BYTES);
        assert_eq!(&publication.bytes()[..8], PACK_MAGIC);
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
            UNRELATED_LIFETIME_FILES + PACKS + 2,
            "four directly named packs plus one catalog and one fixed head replace 1,024 loose files"
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
        let catalog = PackedPatriciaCatalogPublication::build_descriptors(&descriptors).unwrap();
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
        let conflicting = [
            PackedPatriciaPack {
                digest: ContentDigest::of(b"pack one"),
                bytes: b"a".to_vec(),
                entries: BTreeMap::from([(collision_digest, 0..1)]),
            },
            PackedPatriciaPack {
                digest: ContentDigest::of(b"pack two"),
                bytes: b"b".to_vec(),
                entries: BTreeMap::from([(collision_digest, 0..1)]),
            },
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
