//! Bounded physical packing for exact authenticated Patricia node bytes.
//!
//! Version 1 keeps Patricia roots and node encodings unchanged. Immutable
//! packs are named by their complete-byte digest; one fixed, atomically
//! published head names one immutable content-addressed catalog. Catalog
//! Catalog schemas 1 and 2 remain backward-readable. Schema 2 is an
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
const LAYERED_CATALOG_SCHEMA_VERSION: u32 = 2;
const CATALOG_SCHEMA_VERSION: u32 = 3;
const CATALOG_HEADER_BYTES: usize = 8 + 4 + 4 + 4 + 8;
const CATALOG_ENTRY_BYTES: usize = 32 + 32 + 32 + 4 + 4;
const CATALOG_SUFFIX: &str = ".patricia-catalog-v1";
const HEAD_MAGIC: &[u8; 8] = b"TINEPHD\0";
const HEAD_SCHEMA_VERSION: u32 = 1;
const HEAD_BYTES: usize = 8 + 4 + 32 + 4;
pub(crate) const HEAD_FILENAME: &str = "patricia-pack-head-v1";

/// The byte bound, rather than an unrelated entry-count ceiling, is the hard
/// pack allocation bound. This permits compaction to coalesce packs containing
/// many tiny nodes instead of retaining an unbounded number of entry-limited
/// packs in one byte-size tier.
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
        Ok(PackedPatriciaPack {
            digest: self.digest,
            bytes: self.bytes,
            entries: self.ranges,
        })
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
}

pub(crate) struct PendingPackedPatriciaCatalog {
    catalog: PublishedPatriciaCatalog,
    descriptors: Vec<PackDescriptor>,
    packs: Vec<PendingPatriciaPack>,
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
/// authenticated prior head is the only replaceable authority. Superseded
/// immutable files are intentionally retained: the current external reader
/// contract has no persistent pin proving that no reader observed the old head.
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
    compact_size_tiers(current, &mut planned, &mut work)?;

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
    Ok((
        Some(PendingPackedPatriciaCatalog {
            catalog,
            descriptors,
            packs,
        }),
        work,
    ))
}

fn compact_size_tiers(
    current: Option<&PackedPatriciaCatalog>,
    planned: &mut Vec<PlannedPatriciaPack>,
    work: &mut PackedPatriciaPublicationWork,
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
                    copy_exact_entries(&historical.bytes, &historical.entries, &mut entries, work)?;
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
            compact_size_tiers(None, &mut planned, &mut work).unwrap();
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
