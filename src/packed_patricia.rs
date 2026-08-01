//! Bounded physical packing for exact authenticated Patricia node bytes.
//!
//! Version 1 is deliberately only the immutable pack primitive. A later
//! adapter owns pack discovery and catalog/head authority; this module never
//! changes Patricia roots or interprets node contents.
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

#![allow(dead_code)] // bounded first slice, package-tested before catalog/adapter wiring

use std::collections::BTreeMap;
use std::ops::Range;

use cap_std::fs::Dir;

use super::authenticated_patricia::{PatriciaNodePublisher, PatriciaPublicationError};
use super::filesystem::{read_required_regular, FilesystemError};
use super::ContentDigest;

const PACK_MAGIC: &[u8; 8] = b"TINEPPK\0";
const PACK_SCHEMA_VERSION: u32 = 1;
const PACK_HEADER_BYTES: usize = 8 + 4 + 4 + 4;
const PACK_INDEX_ENTRY_BYTES: usize = 32 + 4 + 4;
pub(crate) const PACK_SUFFIX: &str = ".patricia-pack-v1";

/// Keeps construction, reads, and index allocation independent of history
/// length. Larger publication sets must be split by the future adapter before
/// any catalog/head can name them.
pub(crate) const MAX_PACK_ENTRIES: usize = 4_096;
pub(crate) const MAX_PACK_ENTRY_BYTES: usize = 128 * 1024;
pub(crate) const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum PackedPatriciaError {
    Filesystem(FilesystemError),
    Publication(PatriciaPublicationError),
    Empty,
    TooManyEntries,
    EntryTooLarge(ContentDigest),
    PackTooLarge,
    PathMismatch(ContentDigest),
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
        })
    }

    pub(crate) const fn digest(&self) -> ContentDigest {
        self.digest
    }

    pub(crate) fn filename(&self) -> String {
        pack_filename(self.digest)
    }

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
        })
    }
}

/// Non-forgeable package-local evidence of completed pack publication.
pub(crate) struct PublishedPatriciaPack {
    digest: ContentDigest,
}

impl PublishedPatriciaPack {
    pub(crate) const fn digest(&self) -> ContentDigest {
        self.digest
    }
}

/// One fully validated, bounded pack reopened from immutable storage.
pub(crate) struct PackedPatriciaPack {
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

        Ok(Self { bytes, entries })
    }

    pub(crate) fn get(&self, digest: ContentDigest) -> Option<&[u8]> {
        self.entries
            .get(&digest)
            .map(|range| &self.bytes[range.clone()])
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
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

fn pack_filename(digest: ContentDigest) -> String {
    format!("{digest}{PACK_SUFFIX}")
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
