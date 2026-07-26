//! Canonical, inactive evidence for a deterministic multipart bootstrap import.
//!
//! This module deliberately has no object-store, engine, graph-writer, or
//! authority token dependency.  Its only job is to make the v1 bytes and their
//! validation rules unambiguous before a later packet connects them to I/O.

use std::cmp::Ordering;
use std::fmt;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::identity::BootstrapPartId;
use super::{BatchId, CanonicalGraphResourceId, ImportId, WorkspaceId};

pub(crate) const BOOTSTRAP_IMPORT_SCHEMA_VERSION: u32 = 1;
pub(crate) const BOOTSTRAP_PARTITION_POLICY_VERSION: u32 = 1;
pub(crate) const SOURCE_LEAF_SCHEMA_VERSION: u32 = 1;
pub(crate) const SOURCE_BLOB_SCHEMA_VERSION: u32 = 1;
pub(crate) const SOURCE_SPAN_SCHEMA_VERSION: u32 = 1;
pub(crate) const OPERATION_ROOT_SCHEMA_VERSION: u32 = 1;
pub(crate) const PAYLOAD_OBJECT_ROOT_SCHEMA_VERSION: u32 = 1;
pub(crate) const ARCHIVE_FRONTIER_SCHEMA_VERSION: u32 = 1;

pub(crate) const MAX_OPERATIONS_PER_BOOTSTRAP_PART: u32 = 4096;
pub(crate) const MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART: u32 = 4096;
pub(crate) const MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART: u64 = 48 * 1024 * 1024;
pub(crate) const MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART: u64 = 192 * 1024 * 1024;
pub(crate) const MAX_BOOTSTRAP_PART_EVIDENCE_BYTES: usize = 768 * 1024;
pub(crate) const MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES: usize = 768 * 1024;
pub(crate) const MAX_SOURCE_BLOB_CHUNK_BYTES: u32 = 1024 * 1024;

/// Fixed v1 caps that bound allocation in the pure schema.  They are not
/// partition policy knobs; any future change needs a new profile version.
pub(crate) const MAX_BOOTSTRAP_PARTS: u32 = 1024;
pub(crate) const MAX_SOURCE_INVENTORY_LEAVES: u32 = 65_536;
pub(crate) const MAX_SOURCE_BLOB_CHUNKS: u32 = 65_536;
pub(crate) const MAX_SOURCE_LOCATOR_BYTES: usize = 16 * 1024;
pub(crate) const MAX_SELECTED_PEERS: u32 = 64;
pub(crate) const MAX_CANONICAL_NESTING_DEPTH: u8 = 4;

const EVIDENCE_MAGIC: &[u8; 8] = b"TINBPE1\0";
const MANIFEST_MAGIC: &[u8; 8] = b"TINBAM1\0";

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name([u8; 32]);

        impl $name {
            pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub(crate) const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            fn digest(domain: &[u8], fields: &[&[u8]]) -> Self {
                Self(canonical_digest(domain, fields))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), hex(&self.0))
            }
        }
    };
}

digest_type!(BootstrapProfileDigestV1);
digest_type!(SourceLeafDigestV1);
digest_type!(SourceContentDigestV1);
digest_type!(SourceBlobChunkDigestV1);
digest_type!(SourceBlobChunkDescriptorDigestV1);
digest_type!(SourceSpanDigestV1);
digest_type!(OperationDigestV1);
digest_type!(PayloadObjectDigestV1);
digest_type!(BootstrapEvidenceDigestV1);
digest_type!(BootstrapManifestFingerprintV1);
digest_type!(BootstrapAcceptedEventBindingV1);
digest_type!(BootstrapArchiveIdentityDigestV1);
digest_type!(BootstrapAggregateDigestV1);
digest_type!(ArchiveFinalFrontierProofV1);

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceInventoryRootV1 {
    digest: [u8; 32],
    source_count: u32,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceBlobChunkRootV1 {
    digest: [u8; 32],
    chunk_count: u32,
    total_bytes: u64,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceSpanRootV1 {
    digest: [u8; 32],
    span_count: u32,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OperationRootV1 {
    digest: [u8; 32],
    operation_count: u32,
    semantic_effect_bytes: u64,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PayloadObjectRootV1 {
    digest: [u8; 32],
    object_count: u32,
    total_bytes: u64,
}

/// The aggregate schema calls this the full object root.  It is the same
/// canonical payload-object root defined for evidence; no second hash contract
/// is introduced for the same object set.
pub(crate) type FullObjectRootV1 = PayloadObjectRootV1;

macro_rules! root_debug {
    ($name:ident, $count:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("digest", &hex(&self.digest))
                    .field(stringify!($count), &self.$count)
                    .finish()
            }
        }
    };
}

root_debug!(SourceInventoryRootV1, source_count);
root_debug!(SourceSpanRootV1, span_count);

impl fmt::Debug for SourceBlobChunkRootV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceBlobChunkRootV1")
            .field("digest", &hex(&self.digest))
            .field("chunk_count", &self.chunk_count)
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

impl fmt::Debug for OperationRootV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationRootV1")
            .field("digest", &hex(&self.digest))
            .field("operation_count", &self.operation_count)
            .field("semantic_effect_bytes", &self.semantic_effect_bytes)
            .finish()
    }
}

impl fmt::Debug for PayloadObjectRootV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PayloadObjectRootV1")
            .field("digest", &hex(&self.digest))
            .field("object_count", &self.object_count)
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

impl SourceInventoryRootV1 {
    pub(crate) fn empty() -> Self {
        SourceInventoryRootBuilderV1::new().finish()
    }

    pub(crate) fn from_leaves(leaves: &[SourceLeafV1]) -> Result<Self, BootstrapImportError> {
        checked_count(
            leaves.len(),
            MAX_SOURCE_INVENTORY_LEAVES,
            "source inventory leaves",
        )?;
        let mut ordered = leaves.iter().collect::<Vec<_>>();
        ordered.sort_unstable_by(|left, right| left.canonical_cmp(right));
        let mut builder = SourceInventoryRootBuilderV1::new();
        for leaf in ordered {
            builder.push(leaf)?;
        }
        Ok(builder.finish())
    }

    pub(crate) const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub(crate) const fn source_count(self) -> u32 {
        self.source_count
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        root_bytes(self)
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        if self.source_count > MAX_SOURCE_INVENTORY_LEAVES {
            return Err(BootstrapImportError::CountLimit("source inventory leaves"));
        }
        Ok(())
    }

    fn encode_into(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.digest);
        bytes.extend_from_slice(&self.source_count.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() != 36 {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let value = Self {
            digest: array_32(&bytes[..32])?,
            source_count: read_u32(&bytes[32..])?,
        };
        value.validate()?;
        Ok(value)
    }
}

impl SourceBlobChunkRootV1 {
    pub(crate) fn empty() -> Self {
        SourceBlobChunkRootBuilderV1::new().finish()
    }

    pub(crate) fn from_descriptors(
        descriptors: &[SourceBlobChunkDescriptorV1],
    ) -> Result<Self, BootstrapImportError> {
        checked_count(
            descriptors.len(),
            MAX_SOURCE_BLOB_CHUNKS,
            "source blob chunks",
        )?;
        let mut ordered = descriptors.iter().collect::<Vec<_>>();
        ordered.sort_unstable();
        let mut builder = SourceBlobChunkRootBuilderV1::new();
        for descriptor in ordered {
            builder.push(*descriptor)?;
        }
        Ok(builder.finish())
    }

    pub(crate) const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub(crate) const fn chunk_count(self) -> u32 {
        self.chunk_count
    }

    pub(crate) const fn total_bytes(self) -> u64 {
        self.total_bytes
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        root_bytes(self)
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        if self.chunk_count > MAX_SOURCE_BLOB_CHUNKS {
            return Err(BootstrapImportError::CountLimit("source blob chunks"));
        }
        Ok(())
    }

    fn encode_into(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.digest);
        bytes.extend_from_slice(&self.chunk_count.to_be_bytes());
        bytes.extend_from_slice(&self.total_bytes.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() != 44 {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let value = Self {
            digest: array_32(&bytes[..32])?,
            chunk_count: read_u32(&bytes[32..36])?,
            total_bytes: read_u64(&bytes[36..])?,
        };
        value.validate()?;
        Ok(value)
    }
}

impl SourceSpanRootV1 {
    pub(crate) fn empty() -> Self {
        SourceSpanRootBuilderV1::new().finish()
    }

    pub(crate) fn from_spans(spans: &[SourceSpanV1]) -> Result<Self, BootstrapImportError> {
        checked_count(
            spans.len(),
            MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART,
            "source spans",
        )?;
        let mut ordered = spans.iter().collect::<Vec<_>>();
        ordered.sort_unstable();
        let mut builder = SourceSpanRootBuilderV1::new();
        for span in ordered {
            builder.push(*span)?;
        }
        Ok(builder.finish())
    }

    pub(crate) const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub(crate) const fn span_count(self) -> u32 {
        self.span_count
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        root_bytes(self)
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        if self.span_count > MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::CountLimit("source spans"));
        }
        Ok(())
    }

    fn encode_into(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.digest);
        bytes.extend_from_slice(&self.span_count.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() != 36 {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let value = Self {
            digest: array_32(&bytes[..32])?,
            span_count: read_u32(&bytes[32..])?,
        };
        value.validate()?;
        Ok(value)
    }
}

impl OperationRootV1 {
    pub(crate) fn empty() -> Self {
        OperationRootBuilderV1::new().finish()
    }

    pub(crate) fn from_operations(
        operations: &[OperationLeafV1],
    ) -> Result<Self, BootstrapImportError> {
        checked_count(
            operations.len(),
            MAX_OPERATIONS_PER_BOOTSTRAP_PART,
            "operations",
        )?;
        let mut ordered = operations.iter().collect::<Vec<_>>();
        ordered.sort_unstable();
        let mut builder = OperationRootBuilderV1::new();
        for operation in ordered {
            builder.push(*operation)?;
        }
        Ok(builder.finish())
    }

    pub(crate) const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub(crate) const fn operation_count(self) -> u32 {
        self.operation_count
    }

    pub(crate) const fn semantic_effect_bytes(self) -> u64 {
        self.semantic_effect_bytes
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        root_bytes(self)
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        if self.operation_count > MAX_OPERATIONS_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::CountLimit("operations"));
        }
        if self.semantic_effect_bytes > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::ByteLimit("semantic effect"));
        }
        Ok(())
    }

    fn encode_into(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.digest);
        bytes.extend_from_slice(&self.operation_count.to_be_bytes());
        bytes.extend_from_slice(&self.semantic_effect_bytes.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() != 44 {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let value = Self {
            digest: array_32(&bytes[..32])?,
            operation_count: read_u32(&bytes[32..36])?,
            semantic_effect_bytes: read_u64(&bytes[36..])?,
        };
        value.validate()?;
        Ok(value)
    }
}

impl PayloadObjectRootV1 {
    pub(crate) fn empty() -> Self {
        PayloadObjectRootBuilderV1::new().finish()
    }

    pub(crate) fn from_objects(
        objects: &[PayloadObjectDescriptorV1],
    ) -> Result<Self, BootstrapImportError> {
        checked_count(
            objects.len(),
            MAX_OPERATIONS_PER_BOOTSTRAP_PART,
            "payload objects",
        )?;
        let mut ordered = objects.iter().collect::<Vec<_>>();
        ordered.sort_unstable();
        let mut builder = PayloadObjectRootBuilderV1::new();
        for object in ordered {
            builder.push(*object)?;
        }
        Ok(builder.finish())
    }

    pub(crate) const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub(crate) const fn object_count(self) -> u32 {
        self.object_count
    }

    pub(crate) const fn total_bytes(self) -> u64 {
        self.total_bytes
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        root_bytes(self)
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        if self.object_count > MAX_OPERATIONS_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::CountLimit("payload objects"));
        }
        if self.total_bytes > MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::ByteLimit("batch object"));
        }
        Ok(())
    }

    fn encode_into(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.digest);
        bytes.extend_from_slice(&self.object_count.to_be_bytes());
        bytes.extend_from_slice(&self.total_bytes.to_be_bytes());
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() != 44 {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let value = Self {
            digest: array_32(&bytes[..32])?,
            object_count: read_u32(&bytes[32..36])?,
            total_bytes: read_u64(&bytes[36..])?,
        };
        value.validate()?;
        Ok(value)
    }
}

/// The fixed v1 partition profile.  The digest binds every byte/count cap and
/// every schema/policy version above, so a receiver never infers a profile from
/// ambient configuration.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BootstrapPartitionProfileV1 {
    digest: BootstrapProfileDigestV1,
}

impl BootstrapPartitionProfileV1 {
    pub(crate) fn v1() -> Self {
        let mut bytes = Vec::with_capacity(16 * 8);
        for value in [
            BOOTSTRAP_IMPORT_SCHEMA_VERSION,
            BOOTSTRAP_PARTITION_POLICY_VERSION,
            SOURCE_LEAF_SCHEMA_VERSION,
            SOURCE_BLOB_SCHEMA_VERSION,
            SOURCE_SPAN_SCHEMA_VERSION,
            OPERATION_ROOT_SCHEMA_VERSION,
            PAYLOAD_OBJECT_ROOT_SCHEMA_VERSION,
            ARCHIVE_FRONTIER_SCHEMA_VERSION,
            MAX_OPERATIONS_PER_BOOTSTRAP_PART,
            MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART,
            MAX_SOURCE_BLOB_CHUNK_BYTES,
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        for value in [
            MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART,
            MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART,
            MAX_BOOTSTRAP_PART_EVIDENCE_BYTES as u64,
            MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES as u64,
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        Self {
            digest: BootstrapProfileDigestV1::digest(
                b"tine/bootstrap-import/partition-profile/v1\0",
                &[&bytes],
            ),
        }
    }

    pub(crate) const fn digest(self) -> BootstrapProfileDigestV1 {
        self.digest
    }

    fn validate_digest(digest: BootstrapProfileDigestV1) -> Result<(), BootstrapImportError> {
        if digest != Self::v1().digest {
            return Err(BootstrapImportError::ProfileMismatch);
        }
        Ok(())
    }
}

/// One source item in the complete inventory.  Locators are exact portable
/// bytes, not platform paths or serialized maps.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceLeafV1 {
    locator: Vec<u8>,
    content_digest: SourceContentDigestV1,
    byte_length: u64,
}

impl SourceLeafV1 {
    pub(crate) fn new(
        locator: Vec<u8>,
        content_digest: SourceContentDigestV1,
        byte_length: u64,
    ) -> Result<Self, BootstrapImportError> {
        if locator.is_empty() || locator.len() > MAX_SOURCE_LOCATOR_BYTES {
            return Err(BootstrapImportError::LocatorLimit);
        }
        Ok(Self {
            locator,
            content_digest,
            byte_length,
        })
    }

    pub(crate) fn digest(&self) -> SourceLeafDigestV1 {
        SourceLeafDigestV1::digest(
            b"tine/bootstrap-import/source-leaf/v1\0",
            &[
                &SOURCE_LEAF_SCHEMA_VERSION.to_be_bytes(),
                &self.locator,
                self.content_digest.as_bytes(),
                &self.byte_length.to_be_bytes(),
            ],
        )
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        canonical_encode(
            b"tine/bootstrap-import/source-leaf/v1\0",
            &[
                &SOURCE_LEAF_SCHEMA_VERSION.to_be_bytes(),
                &self.locator,
                self.content_digest.as_bytes(),
                &self.byte_length.to_be_bytes(),
            ],
        )
    }

    fn canonical_cmp(&self, other: &Self) -> Ordering {
        self.encode().cmp(&other.encode())
    }
}

/// A fixed-size, independently addressable source-blob chunk.  The caller is
/// responsible for choosing the chunks; this schema only binds and bounds them.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceBlobChunkDescriptorV1 {
    source_leaf: SourceLeafDigestV1,
    ordinal: u32,
    count: u32,
    offset: u64,
    byte_length: u32,
    content_digest: SourceBlobChunkDigestV1,
}

impl SourceBlobChunkDescriptorV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        source_leaf: SourceLeafDigestV1,
        ordinal: u32,
        count: u32,
        offset: u64,
        byte_length: u32,
        content_digest: SourceBlobChunkDigestV1,
    ) -> Result<Self, BootstrapImportError> {
        if count == 0 || ordinal >= count {
            return Err(BootstrapImportError::InvalidOrdinal);
        }
        if byte_length == 0 || byte_length > MAX_SOURCE_BLOB_CHUNK_BYTES {
            return Err(BootstrapImportError::ByteLimit("source blob chunk"));
        }
        offset
            .checked_add(u64::from(byte_length))
            .ok_or(BootstrapImportError::LengthOverflow)?;
        Ok(Self {
            source_leaf,
            ordinal,
            count,
            offset,
            byte_length,
            content_digest,
        })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        canonical_encode(
            b"tine/bootstrap-import/source-blob-chunk/v1\0",
            &[
                &SOURCE_BLOB_SCHEMA_VERSION.to_be_bytes(),
                self.source_leaf.as_bytes(),
                &self.ordinal.to_be_bytes(),
                &self.count.to_be_bytes(),
                &self.offset.to_be_bytes(),
                &self.byte_length.to_be_bytes(),
                self.content_digest.as_bytes(),
            ],
        )
    }

    pub(crate) fn digest(&self) -> SourceBlobChunkDescriptorDigestV1 {
        SourceBlobChunkDescriptorDigestV1::digest(
            b"tine/bootstrap-import/source-blob-chunk-digest/v1\0",
            &[&self.encode()],
        )
    }
}

/// A source range attributed to one part.  Ranges are deliberately source
/// metadata only: they do not carry a filesystem path or a live file handle.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceSpanV1 {
    source_leaf: SourceLeafDigestV1,
    offset: u64,
    byte_length: u64,
}

impl SourceSpanV1 {
    pub(crate) fn new(
        source_leaf: SourceLeafDigestV1,
        offset: u64,
        byte_length: u64,
    ) -> Result<Self, BootstrapImportError> {
        if byte_length == 0 {
            return Err(BootstrapImportError::EmptySourceSpan);
        }
        offset
            .checked_add(byte_length)
            .ok_or(BootstrapImportError::LengthOverflow)?;
        Ok(Self {
            source_leaf,
            offset,
            byte_length,
        })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        canonical_encode(
            b"tine/bootstrap-import/source-span/v1\0",
            &[
                &SOURCE_SPAN_SCHEMA_VERSION.to_be_bytes(),
                self.source_leaf.as_bytes(),
                &self.offset.to_be_bytes(),
                &self.byte_length.to_be_bytes(),
            ],
        )
    }

    pub(crate) fn digest(&self) -> SourceSpanDigestV1 {
        SourceSpanDigestV1::digest(
            b"tine/bootstrap-import/source-span-digest/v1\0",
            &[&self.encode()],
        )
    }
}

/// A digest-only operation commitment plus the semantic bytes it consumes.
/// The operation bytes remain in the payload object set and are not decoded by
/// this pure validation packet.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OperationLeafV1 {
    operation_digest: OperationDigestV1,
    semantic_effect_bytes: u64,
}

impl OperationLeafV1 {
    pub(crate) fn new(
        operation_digest: OperationDigestV1,
        semantic_effect_bytes: u64,
    ) -> Result<Self, BootstrapImportError> {
        if semantic_effect_bytes > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::ByteLimit("semantic effect"));
        }
        Ok(Self {
            operation_digest,
            semantic_effect_bytes,
        })
    }

    fn encode(&self) -> Vec<u8> {
        canonical_encode(
            b"tine/bootstrap-import/operation-leaf/v1\0",
            &[
                &OPERATION_ROOT_SCHEMA_VERSION.to_be_bytes(),
                self.operation_digest.as_bytes(),
                &self.semantic_effect_bytes.to_be_bytes(),
            ],
        )
    }
}

/// One full encoded object referenced by a bootstrap part.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PayloadObjectDescriptorV1 {
    content_digest: PayloadObjectDigestV1,
    byte_length: u64,
}

impl PayloadObjectDescriptorV1 {
    pub(crate) fn new(
        content_digest: PayloadObjectDigestV1,
        byte_length: u64,
    ) -> Result<Self, BootstrapImportError> {
        if byte_length == 0 || byte_length > MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::ByteLimit("batch object"));
        }
        Ok(Self {
            content_digest,
            byte_length,
        })
    }

    fn encode(&self) -> Vec<u8> {
        canonical_encode(
            b"tine/bootstrap-import/payload-object/v1\0",
            &[
                &PAYLOAD_OBJECT_ROOT_SCHEMA_VERSION.to_be_bytes(),
                self.content_digest.as_bytes(),
                &self.byte_length.to_be_bytes(),
            ],
        )
    }
}

struct RootHasherV1 {
    hasher: Sha256,
    count: u32,
}

impl RootHasherV1 {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(domain);
        Self { hasher, count: 0 }
    }

    fn push(
        &mut self,
        encoded: &[u8],
        limit: u32,
        label: &'static str,
    ) -> Result<(), BootstrapImportError> {
        if self.count >= limit {
            return Err(BootstrapImportError::CountLimit(label));
        }
        self.hasher.update((encoded.len() as u64).to_be_bytes());
        self.hasher.update(encoded);
        self.count += 1;
        Ok(())
    }

    fn finish(mut self) -> [u8; 32] {
        self.hasher.update(4_u64.to_be_bytes());
        self.hasher.update(self.count.to_be_bytes());
        self.hasher.finalize().into()
    }
}

pub(crate) struct SourceInventoryRootBuilderV1 {
    root: RootHasherV1,
    last: Option<SourceLeafV1>,
}

impl SourceInventoryRootBuilderV1 {
    pub(crate) fn new() -> Self {
        Self {
            root: RootHasherV1::new(b"tine/bootstrap-import/source-inventory-root/v1\0"),
            last: None,
        }
    }

    pub(crate) fn push(&mut self, leaf: &SourceLeafV1) -> Result<(), BootstrapImportError> {
        if let Some(last) = &self.last {
            match last.canonical_cmp(leaf) {
                Ordering::Less => {}
                Ordering::Equal => return Err(BootstrapImportError::DuplicateCanonicalItem),
                Ordering::Greater => return Err(BootstrapImportError::NonCanonicalOrder),
            }
        }
        let encoded = leaf.encode();
        self.root.push(
            &encoded,
            MAX_SOURCE_INVENTORY_LEAVES,
            "source inventory leaves",
        )?;
        self.last = Some(leaf.clone());
        Ok(())
    }

    pub(crate) fn finish(self) -> SourceInventoryRootV1 {
        let count = self.root.count;
        SourceInventoryRootV1 {
            digest: self.root.finish(),
            source_count: count,
        }
    }
}

pub(crate) struct SourceBlobChunkRootBuilderV1 {
    root: RootHasherV1,
    last: Option<SourceBlobChunkDescriptorV1>,
    total_bytes: u64,
}

impl SourceBlobChunkRootBuilderV1 {
    pub(crate) fn new() -> Self {
        Self {
            root: RootHasherV1::new(b"tine/bootstrap-import/source-blob-root/v1\0"),
            last: None,
            total_bytes: 0,
        }
    }

    pub(crate) fn push(
        &mut self,
        descriptor: SourceBlobChunkDescriptorV1,
    ) -> Result<(), BootstrapImportError> {
        if let Some(last) = self.last {
            if descriptor == last {
                return Err(BootstrapImportError::DuplicateCanonicalItem);
            }
            if descriptor < last {
                return Err(BootstrapImportError::NonCanonicalOrder);
            }
        }
        let encoded = descriptor.encode();
        self.root
            .push(&encoded, MAX_SOURCE_BLOB_CHUNKS, "source blob chunks")?;
        self.total_bytes = self
            .total_bytes
            .checked_add(u64::from(descriptor.byte_length))
            .ok_or(BootstrapImportError::LengthOverflow)?;
        self.last = Some(descriptor);
        Ok(())
    }

    pub(crate) fn finish(self) -> SourceBlobChunkRootV1 {
        let count = self.root.count;
        SourceBlobChunkRootV1 {
            digest: self.root.finish(),
            chunk_count: count,
            total_bytes: self.total_bytes,
        }
    }
}

pub(crate) struct SourceSpanRootBuilderV1 {
    root: RootHasherV1,
    last: Option<SourceSpanV1>,
}

impl SourceSpanRootBuilderV1 {
    pub(crate) fn new() -> Self {
        Self {
            root: RootHasherV1::new(b"tine/bootstrap-import/source-span-root/v1\0"),
            last: None,
        }
    }

    pub(crate) fn push(&mut self, span: SourceSpanV1) -> Result<(), BootstrapImportError> {
        if let Some(last) = self.last {
            if span == last {
                return Err(BootstrapImportError::DuplicateCanonicalItem);
            }
            if span < last {
                return Err(BootstrapImportError::NonCanonicalOrder);
            }
        }
        let encoded = span.encode();
        self.root.push(
            &encoded,
            MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART,
            "source spans",
        )?;
        self.last = Some(span);
        Ok(())
    }

    pub(crate) fn finish(self) -> SourceSpanRootV1 {
        let count = self.root.count;
        SourceSpanRootV1 {
            digest: self.root.finish(),
            span_count: count,
        }
    }
}

pub(crate) struct OperationRootBuilderV1 {
    root: RootHasherV1,
    last: Option<OperationLeafV1>,
    semantic_effect_bytes: u64,
}

impl OperationRootBuilderV1 {
    pub(crate) fn new() -> Self {
        Self {
            root: RootHasherV1::new(b"tine/bootstrap-import/operation-root/v1\0"),
            last: None,
            semantic_effect_bytes: 0,
        }
    }

    pub(crate) fn push(&mut self, operation: OperationLeafV1) -> Result<(), BootstrapImportError> {
        if let Some(last) = self.last {
            if operation == last {
                return Err(BootstrapImportError::DuplicateCanonicalItem);
            }
            if operation < last {
                return Err(BootstrapImportError::NonCanonicalOrder);
            }
        }
        self.semantic_effect_bytes = self
            .semantic_effect_bytes
            .checked_add(operation.semantic_effect_bytes)
            .ok_or(BootstrapImportError::LengthOverflow)?;
        if self.semantic_effect_bytes > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::ByteLimit("semantic effect"));
        }
        let encoded = operation.encode();
        self.root
            .push(&encoded, MAX_OPERATIONS_PER_BOOTSTRAP_PART, "operations")?;
        self.last = Some(operation);
        Ok(())
    }

    pub(crate) fn finish(self) -> OperationRootV1 {
        let count = self.root.count;
        OperationRootV1 {
            digest: self.root.finish(),
            operation_count: count,
            semantic_effect_bytes: self.semantic_effect_bytes,
        }
    }
}

pub(crate) struct PayloadObjectRootBuilderV1 {
    root: RootHasherV1,
    last: Option<PayloadObjectDescriptorV1>,
    total_bytes: u64,
}

impl PayloadObjectRootBuilderV1 {
    pub(crate) fn new() -> Self {
        Self {
            root: RootHasherV1::new(b"tine/bootstrap-import/payload-object-root/v1\0"),
            last: None,
            total_bytes: 0,
        }
    }

    pub(crate) fn push(
        &mut self,
        object: PayloadObjectDescriptorV1,
    ) -> Result<(), BootstrapImportError> {
        if let Some(last) = self.last {
            if object == last {
                return Err(BootstrapImportError::DuplicateCanonicalItem);
            }
            if object < last {
                return Err(BootstrapImportError::NonCanonicalOrder);
            }
        }
        self.total_bytes = self
            .total_bytes
            .checked_add(object.byte_length)
            .ok_or(BootstrapImportError::LengthOverflow)?;
        if self.total_bytes > MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapImportError::ByteLimit("batch object"));
        }
        let encoded = object.encode();
        self.root.push(
            &encoded,
            MAX_OPERATIONS_PER_BOOTSTRAP_PART,
            "payload objects",
        )?;
        self.last = Some(object);
        Ok(())
    }

    pub(crate) fn finish(self) -> PayloadObjectRootV1 {
        let count = self.root.count;
        PayloadObjectRootV1 {
            digest: self.root.finish(),
            object_count: count,
            total_bytes: self.total_bytes,
        }
    }
}

/// The cycle-free evidence carried by one candidate part.  It intentionally
/// omits its own digest, batch-manifest fingerprint, and full descriptor root.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BootstrapImportPartEvidenceV1 {
    import_id: ImportId,
    profile_digest: BootstrapProfileDigestV1,
    ordinal: u32,
    part_count: u32,
    source_span_root: SourceSpanRootV1,
    operation_root: OperationRootV1,
    payload_object_root: PayloadObjectRootV1,
    predecessor: Option<BootstrapPartId>,
}

impl BootstrapImportPartEvidenceV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        import_id: ImportId,
        profile_digest: BootstrapProfileDigestV1,
        ordinal: u32,
        part_count: u32,
        source_span_root: SourceSpanRootV1,
        operation_root: OperationRootV1,
        payload_object_root: PayloadObjectRootV1,
        predecessor: Option<BootstrapPartId>,
    ) -> Result<Self, BootstrapImportError> {
        let value = Self {
            import_id,
            profile_digest,
            ordinal,
            part_count,
            source_span_root,
            operation_root,
            payload_object_root,
            predecessor,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, BootstrapImportError> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(320);
        bytes.extend_from_slice(EVIDENCE_MAGIC);
        put_field(
            &mut bytes,
            1,
            &BOOTSTRAP_IMPORT_SCHEMA_VERSION.to_be_bytes(),
        )?;
        put_field(&mut bytes, 2, self.import_id.as_bytes())?;
        put_field(&mut bytes, 3, self.profile_digest.as_bytes())?;
        put_field(&mut bytes, 4, &self.ordinal.to_be_bytes())?;
        put_field(&mut bytes, 5, &self.part_count.to_be_bytes())?;
        put_field(&mut bytes, 6, &root_bytes(self.source_span_root))?;
        put_field(&mut bytes, 7, &root_bytes(self.operation_root))?;
        put_field(&mut bytes, 8, &root_bytes(self.payload_object_root))?;
        put_field(&mut bytes, 9, &part_id_option_bytes(self.predecessor))?;
        if bytes.len() > MAX_BOOTSTRAP_PART_EVIDENCE_BYTES {
            return Err(BootstrapImportError::EncodedSizeLimit("part evidence"));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() > MAX_BOOTSTRAP_PART_EVIDENCE_BYTES {
            return Err(BootstrapImportError::EncodedSizeLimit("part evidence"));
        }
        let fields = CanonicalFieldsV1::parse(bytes, EVIDENCE_MAGIC, 9, 1)?;
        let version = read_u32(fields.required(1)?)?;
        if version != BOOTSTRAP_IMPORT_SCHEMA_VERSION {
            return Err(BootstrapImportError::UnsupportedVersion(version));
        }
        let value = Self::new(
            ImportId::from_digest(array_32(fields.required(2)?)?),
            BootstrapProfileDigestV1::from_bytes(array_32(fields.required(3)?)?),
            read_u32(fields.required(4)?)?,
            read_u32(fields.required(5)?)?,
            SourceSpanRootV1::decode(fields.required(6)?)?,
            OperationRootV1::decode(fields.required(7)?)?,
            PayloadObjectRootV1::decode(fields.required(8)?)?,
            decode_part_id_option(fields.required(9)?)?,
        )?;
        if value.encode()?.as_slice() != bytes {
            return Err(BootstrapImportError::NonCanonicalBytes);
        }
        Ok(value)
    }

    pub(crate) fn part_id(&self) -> BootstrapPartId {
        BootstrapPartId::derive(
            self.import_id,
            self.profile_digest.as_bytes(),
            self.ordinal,
            self.source_span_root.digest(),
            self.operation_root.digest(),
        )
    }

    pub(crate) fn batch_id(&self) -> BatchId {
        BatchId::for_bootstrap_part(self.part_id())
    }

    pub(crate) fn evidence_digest(&self) -> BootstrapEvidenceDigestV1 {
        let bytes = self
            .encode()
            .expect("validated fixed-size bootstrap evidence encodes");
        BootstrapEvidenceDigestV1::digest(
            b"tine/bootstrap-import/part-evidence-digest/v1\0",
            &[&bytes],
        )
    }

    pub(crate) const fn import_id(self) -> ImportId {
        self.import_id
    }

    pub(crate) const fn profile_digest(self) -> BootstrapProfileDigestV1 {
        self.profile_digest
    }

    pub(crate) const fn ordinal(self) -> u32 {
        self.ordinal
    }

    pub(crate) const fn part_count(self) -> u32 {
        self.part_count
    }

    pub(crate) const fn source_span_root(self) -> SourceSpanRootV1 {
        self.source_span_root
    }

    pub(crate) const fn operation_root(self) -> OperationRootV1 {
        self.operation_root
    }

    pub(crate) const fn payload_object_root(self) -> PayloadObjectRootV1 {
        self.payload_object_root
    }

    pub(crate) const fn predecessor(self) -> Option<BootstrapPartId> {
        self.predecessor
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        BootstrapPartitionProfileV1::validate_digest(self.profile_digest)?;
        if self.part_count == 0 || self.part_count > MAX_BOOTSTRAP_PARTS {
            return Err(BootstrapImportError::InvalidPartCount);
        }
        if self.ordinal >= self.part_count {
            return Err(BootstrapImportError::InvalidOrdinal);
        }
        match (self.ordinal, self.part_count, self.predecessor) {
            (0, _, None) => {}
            (0, _, Some(_)) => return Err(BootstrapImportError::UnexpectedPredecessor),
            (_, 1, Some(_)) => return Err(BootstrapImportError::UnexpectedPredecessor),
            (_, _, None) => return Err(BootstrapImportError::MissingPredecessor),
            _ => {}
        }
        self.source_span_root.validate()?;
        self.operation_root.validate()?;
        self.payload_object_root.validate()?;
        if self.operation_root.operation_count == 0 {
            return Err(BootstrapImportError::EmptyOperationTransaction);
        }
        Ok(())
    }
}

/// An archive-local frontier commitment.  It has no dependency on
/// `AcceptedFrontierRoot` or any engine type, and therefore cannot accidentally
/// become a serialization of a live acceptance structure.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ArchiveLocalFrontierBindingV1 {
    digest: [u8; 32],
    accepted_count: u32,
    last_part: Option<BootstrapPartId>,
}

impl ArchiveLocalFrontierBindingV1 {
    pub(crate) fn initial(import_id: ImportId, profile_digest: BootstrapProfileDigestV1) -> Self {
        Self {
            digest: canonical_digest(
                b"tine/bootstrap-import/archive-frontier-initial/v1\0",
                &[import_id.as_bytes(), profile_digest.as_bytes()],
            ),
            accepted_count: 0,
            last_part: None,
        }
    }

    pub(crate) fn advance(
        self,
        part_id: BootstrapPartId,
        accepted_event: BootstrapAcceptedEventBindingV1,
    ) -> Result<Self, BootstrapImportError> {
        let accepted_count = self
            .accepted_count
            .checked_add(1)
            .ok_or(BootstrapImportError::LengthOverflow)?;
        if accepted_count > MAX_BOOTSTRAP_PARTS {
            return Err(BootstrapImportError::CountLimit("bootstrap parts"));
        }
        Ok(Self {
            digest: canonical_digest(
                b"tine/bootstrap-import/archive-frontier-step/v1\0",
                &[
                    &ARCHIVE_FRONTIER_SCHEMA_VERSION.to_be_bytes(),
                    &self.digest,
                    &accepted_count.to_be_bytes(),
                    part_id.as_bytes(),
                    accepted_event.as_bytes(),
                ],
            ),
            accepted_count,
            last_part: Some(part_id),
        })
    }

    pub(crate) fn final_proof(self) -> ArchiveFinalFrontierProofV1 {
        ArchiveFinalFrontierProofV1::digest(
            b"tine/bootstrap-import/archive-final-frontier-proof/v1\0",
            &[&archive_frontier_bytes(self)],
        )
    }

    fn validate(self) -> Result<(), BootstrapImportError> {
        if self.accepted_count > MAX_BOOTSTRAP_PARTS {
            return Err(BootstrapImportError::CountLimit("bootstrap parts"));
        }
        if (self.accepted_count == 0) != self.last_part.is_none() {
            return Err(BootstrapImportError::InvalidArchiveFrontier);
        }
        Ok(())
    }
}

/// A fixed, deterministic identity for the bounded peer sample selected by a
/// future importer.  This packet never probes a peer or grants authority.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BootstrapPeerProbeEntryV1 {
    peer_identity: [u8; 32],
    observation_digest: [u8; 32],
}

impl BootstrapPeerProbeEntryV1 {
    pub(crate) const fn new(peer_identity: [u8; 32], observation_digest: [u8; 32]) -> Self {
        Self {
            peer_identity,
            observation_digest,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BootstrapPeerProbeV1 {
    entries: Vec<BootstrapPeerProbeEntryV1>,
}

impl BootstrapPeerProbeV1 {
    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn new(
        mut entries: Vec<BootstrapPeerProbeEntryV1>,
    ) -> Result<Self, BootstrapImportError> {
        checked_count(entries.len(), MAX_SELECTED_PEERS, "selected peers")?;
        entries.sort_unstable();
        if entries.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(BootstrapImportError::DuplicateCanonicalItem);
        }
        Ok(Self { entries })
    }

    pub(crate) fn entries(&self) -> &[BootstrapPeerProbeEntryV1] {
        &self.entries
    }

    fn encode(&self) -> Result<Vec<u8>, BootstrapImportError> {
        checked_count(self.entries.len(), MAX_SELECTED_PEERS, "selected peers")?;
        let mut bytes = Vec::with_capacity(4 + self.entries.len() * 64);
        bytes.extend_from_slice(&(self.entries.len() as u32).to_be_bytes());
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.peer_identity);
            bytes.extend_from_slice(&entry.observation_digest);
        }
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() < 4 {
            return Err(BootstrapImportError::Truncated);
        }
        let count = read_u32(&bytes[..4])?;
        if count > MAX_SELECTED_PEERS {
            return Err(BootstrapImportError::CountLimit("selected peers"));
        }
        let expected = 4_usize
            .checked_add(
                (count as usize)
                    .checked_mul(64)
                    .ok_or(BootstrapImportError::LengthOverflow)?,
            )
            .ok_or(BootstrapImportError::LengthOverflow)?;
        if expected != bytes.len() {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let mut entries = Vec::with_capacity(count as usize);
        for chunk in bytes[4..].chunks_exact(64) {
            entries.push(BootstrapPeerProbeEntryV1::new(
                array_32(&chunk[..32])?,
                array_32(&chunk[32..])?,
            ));
        }
        let value = Self::new(entries)?;
        if value.encode()?.as_slice() != bytes {
            return Err(BootstrapImportError::NonCanonicalBytes);
        }
        Ok(value)
    }
}

/// The descriptor carried in the aggregate manifest for one accepted part.
/// Its evidence is reconstructed from the fixed fields below; the evidence
/// digest is therefore checkable without fetching a separate object.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BootstrapPartDescriptorV1 {
    evidence: BootstrapImportPartEvidenceV1,
    part_id: BootstrapPartId,
    batch_id: BatchId,
    evidence_digest: BootstrapEvidenceDigestV1,
    manifest_fingerprint: BootstrapManifestFingerprintV1,
    full_object_root: FullObjectRootV1,
    accepted_event: BootstrapAcceptedEventBindingV1,
    acceptance_sequence: u32,
    prior_frontier: ArchiveLocalFrontierBindingV1,
    post_frontier: ArchiveLocalFrontierBindingV1,
}

impl BootstrapPartDescriptorV1 {
    /// Build the next archive-local accepted descriptor without consulting an
    /// engine.  The aggregate validator still verifies ordinal and predecessor
    /// continuity once every descriptor is present.
    pub(crate) fn accepted(
        evidence: BootstrapImportPartEvidenceV1,
        manifest_fingerprint: BootstrapManifestFingerprintV1,
        prior_frontier: ArchiveLocalFrontierBindingV1,
    ) -> Result<Self, BootstrapImportError> {
        let acceptance_sequence = prior_frontier
            .accepted_count
            .checked_add(1)
            .ok_or(BootstrapImportError::LengthOverflow)?;
        let part_id = evidence.part_id();
        let evidence_digest = evidence.evidence_digest();
        let full_object_root = evidence.payload_object_root();
        let accepted_event = accepted_event_binding(
            part_id,
            evidence_digest,
            manifest_fingerprint,
            full_object_root,
            acceptance_sequence,
        );
        let post_frontier = prior_frontier.advance(part_id, accepted_event)?;
        Self::new(
            evidence,
            manifest_fingerprint,
            full_object_root,
            acceptance_sequence,
            prior_frontier,
            post_frontier,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        evidence: BootstrapImportPartEvidenceV1,
        manifest_fingerprint: BootstrapManifestFingerprintV1,
        full_object_root: FullObjectRootV1,
        acceptance_sequence: u32,
        prior_frontier: ArchiveLocalFrontierBindingV1,
        post_frontier: ArchiveLocalFrontierBindingV1,
    ) -> Result<Self, BootstrapImportError> {
        if full_object_root != evidence.payload_object_root() {
            return Err(BootstrapImportError::FullObjectRootMismatch);
        }
        let part_id = evidence.part_id();
        let evidence_digest = evidence.evidence_digest();
        let accepted_event = accepted_event_binding(
            part_id,
            evidence_digest,
            manifest_fingerprint,
            full_object_root,
            acceptance_sequence,
        );
        let value = Self {
            evidence,
            part_id,
            batch_id: evidence.batch_id(),
            evidence_digest,
            manifest_fingerprint,
            full_object_root,
            accepted_event,
            acceptance_sequence,
            prior_frontier,
            post_frontier,
        };
        value.validate_self()?;
        Ok(value)
    }

    pub(crate) const fn evidence(self) -> BootstrapImportPartEvidenceV1 {
        self.evidence
    }

    pub(crate) const fn part_id(self) -> BootstrapPartId {
        self.part_id
    }

    pub(crate) const fn batch_id(self) -> BatchId {
        self.batch_id
    }

    pub(crate) const fn acceptance_sequence(self) -> u32 {
        self.acceptance_sequence
    }

    fn validate_self(self) -> Result<(), BootstrapImportError> {
        self.evidence.validate()?;
        if self.part_id != self.evidence.part_id() {
            return Err(BootstrapImportError::PartIdMismatch);
        }
        if self.batch_id != self.evidence.batch_id() {
            return Err(BootstrapImportError::BatchIdMismatch);
        }
        if self.evidence_digest != self.evidence.evidence_digest() {
            return Err(BootstrapImportError::EvidenceDigestMismatch);
        }
        if self.full_object_root != self.evidence.payload_object_root() {
            return Err(BootstrapImportError::FullObjectRootMismatch);
        }
        if self.accepted_event
            != accepted_event_binding(
                self.part_id,
                self.evidence_digest,
                self.manifest_fingerprint,
                self.full_object_root,
                self.acceptance_sequence,
            )
        {
            return Err(BootstrapImportError::AcceptedEventBindingMismatch);
        }
        self.prior_frontier.validate()?;
        self.post_frontier.validate()?;
        Ok(())
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PART_DESCRIPTOR_BYTES);
        bytes.extend_from_slice(self.part_id.as_bytes());
        bytes.extend_from_slice(self.batch_id.as_uuid().as_bytes());
        bytes.extend_from_slice(&self.evidence.ordinal().to_be_bytes());
        bytes.extend_from_slice(&self.evidence.part_count().to_be_bytes());
        self.evidence.source_span_root().encode_into(&mut bytes);
        self.evidence.operation_root().encode_into(&mut bytes);
        self.evidence.payload_object_root().encode_into(&mut bytes);
        bytes.extend_from_slice(&part_id_option_bytes(self.evidence.predecessor()));
        bytes.extend_from_slice(self.evidence_digest.as_bytes());
        bytes.extend_from_slice(self.manifest_fingerprint.as_bytes());
        self.full_object_root.encode_into(&mut bytes);
        bytes.extend_from_slice(self.accepted_event.as_bytes());
        bytes.extend_from_slice(&self.acceptance_sequence.to_be_bytes());
        bytes.extend_from_slice(&archive_frontier_bytes(self.prior_frontier));
        bytes.extend_from_slice(&archive_frontier_bytes(self.post_frontier));
        debug_assert_eq!(bytes.len(), PART_DESCRIPTOR_BYTES);
        bytes
    }

    fn decode(
        bytes: &[u8],
        import_id: ImportId,
        profile_digest: BootstrapProfileDigestV1,
    ) -> Result<Self, BootstrapImportError> {
        if bytes.len() != PART_DESCRIPTOR_BYTES {
            return Err(BootstrapImportError::InvalidFieldLength);
        }
        let mut cursor = FixedReader::new(bytes);
        let part_id = BootstrapPartId::from_digest(cursor.array_32()?);
        let batch_id = BatchId::from_uuid(Uuid::from_bytes(cursor.array_16()?));
        let ordinal = cursor.u32()?;
        let part_count = cursor.u32()?;
        let source_span_root = SourceSpanRootV1::decode(cursor.take(36)?)?;
        let operation_root = OperationRootV1::decode(cursor.take(44)?)?;
        let payload_object_root = PayloadObjectRootV1::decode(cursor.take(44)?)?;
        let predecessor = decode_part_id_option(cursor.take(33)?)?;
        let evidence_digest = BootstrapEvidenceDigestV1::from_bytes(cursor.array_32()?);
        let manifest_fingerprint = BootstrapManifestFingerprintV1::from_bytes(cursor.array_32()?);
        let full_object_root = PayloadObjectRootV1::decode(cursor.take(44)?)?;
        let accepted_event = BootstrapAcceptedEventBindingV1::from_bytes(cursor.array_32()?);
        let acceptance_sequence = cursor.u32()?;
        let prior_frontier = decode_archive_frontier(cursor.take(69)?)?;
        let post_frontier = decode_archive_frontier(cursor.take(69)?)?;
        cursor.finish()?;
        let evidence = BootstrapImportPartEvidenceV1::new(
            import_id,
            profile_digest,
            ordinal,
            part_count,
            source_span_root,
            operation_root,
            payload_object_root,
            predecessor,
        )?;
        let value = Self {
            evidence,
            part_id,
            batch_id,
            evidence_digest,
            manifest_fingerprint,
            full_object_root,
            accepted_event,
            acceptance_sequence,
            prior_frontier,
            post_frontier,
        };
        value.validate_self()?;
        if value.encode().as_slice() != bytes {
            return Err(BootstrapImportError::NonCanonicalBytes);
        }
        Ok(value)
    }
}

/// A complete archive-local aggregate.  The archive identity is an opaque
/// digest supplied by the archive format; this module only binds and validates
/// it, never opens an archive or publishes one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapAggregateManifestV1 {
    workspace_id: WorkspaceId,
    graph_resource: CanonicalGraphResourceId,
    archive_identity: BootstrapArchiveIdentityDigestV1,
    import_id: ImportId,
    complete_source_count: u32,
    source_inventory_root: SourceInventoryRootV1,
    source_blob_root: SourceBlobChunkRootV1,
    profile_digest: BootstrapProfileDigestV1,
    peer_probe: BootstrapPeerProbeV1,
    parts: Vec<BootstrapPartDescriptorV1>,
    initial_frontier: ArchiveLocalFrontierBindingV1,
    final_frontier: ArchiveLocalFrontierBindingV1,
    final_frontier_proof: ArchiveFinalFrontierProofV1,
}

impl BootstrapAggregateManifestV1 {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        workspace_id: WorkspaceId,
        graph_resource: CanonicalGraphResourceId,
        archive_identity: BootstrapArchiveIdentityDigestV1,
        import_id: ImportId,
        complete_source_count: u32,
        source_inventory_root: SourceInventoryRootV1,
        source_blob_root: SourceBlobChunkRootV1,
        profile_digest: BootstrapProfileDigestV1,
        peer_probe: BootstrapPeerProbeV1,
        parts: Vec<BootstrapPartDescriptorV1>,
        initial_frontier: ArchiveLocalFrontierBindingV1,
        final_frontier: ArchiveLocalFrontierBindingV1,
        final_frontier_proof: ArchiveFinalFrontierProofV1,
    ) -> Result<Self, BootstrapImportError> {
        checked_count(parts.len(), MAX_BOOTSTRAP_PARTS, "bootstrap parts")?;
        let value = Self {
            workspace_id,
            graph_resource,
            archive_identity,
            import_id,
            complete_source_count,
            source_inventory_root,
            source_blob_root,
            profile_digest,
            peer_probe,
            parts,
            initial_frontier,
            final_frontier,
            final_frontier_proof,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn empty(
        workspace_id: WorkspaceId,
        graph_resource: CanonicalGraphResourceId,
        archive_identity: BootstrapArchiveIdentityDigestV1,
        import_id: ImportId,
        source_inventory_root: SourceInventoryRootV1,
        source_blob_root: SourceBlobChunkRootV1,
    ) -> Result<Self, BootstrapImportError> {
        let profile_digest = BootstrapPartitionProfileV1::v1().digest();
        let frontier = ArchiveLocalFrontierBindingV1::initial(import_id, profile_digest);
        let proof = final_frontier_proof(
            workspace_id,
            graph_resource,
            archive_identity,
            import_id,
            profile_digest,
            frontier,
        );
        Self::new(
            workspace_id,
            graph_resource,
            archive_identity,
            import_id,
            source_inventory_root.source_count(),
            source_inventory_root,
            source_blob_root,
            profile_digest,
            BootstrapPeerProbeV1::empty(),
            Vec::new(),
            frontier,
            frontier,
            proof,
        )
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, BootstrapImportError> {
        self.validate()?;
        let parts = self.encode_parts()?;
        let mut bytes = Vec::with_capacity(512 + parts.len());
        bytes.extend_from_slice(MANIFEST_MAGIC);
        put_field(
            &mut bytes,
            1,
            &BOOTSTRAP_IMPORT_SCHEMA_VERSION.to_be_bytes(),
        )?;
        put_field(&mut bytes, 2, self.workspace_id.as_uuid().as_bytes())?;
        put_field(&mut bytes, 3, self.graph_resource.as_bytes())?;
        put_field(&mut bytes, 4, self.archive_identity.as_bytes())?;
        put_field(&mut bytes, 5, self.import_id.as_bytes())?;
        put_field(&mut bytes, 6, &self.complete_source_count.to_be_bytes())?;
        put_field(&mut bytes, 7, &root_bytes(self.source_inventory_root))?;
        put_field(&mut bytes, 8, &root_bytes(self.source_blob_root))?;
        put_field(&mut bytes, 9, self.profile_digest.as_bytes())?;
        put_field(&mut bytes, 10, &self.peer_probe.encode()?)?;
        put_field(&mut bytes, 11, &parts)?;
        put_field(
            &mut bytes,
            12,
            &archive_frontier_bytes(self.initial_frontier),
        )?;
        put_field(&mut bytes, 13, &archive_frontier_bytes(self.final_frontier))?;
        put_field(&mut bytes, 14, self.final_frontier_proof.as_bytes())?;
        if bytes.len() > MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES {
            return Err(BootstrapImportError::EncodedSizeLimit("aggregate manifest"));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, BootstrapImportError> {
        if bytes.len() > MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES {
            return Err(BootstrapImportError::EncodedSizeLimit("aggregate manifest"));
        }
        let fields = CanonicalFieldsV1::parse(bytes, MANIFEST_MAGIC, 14, 1)?;
        let version = read_u32(fields.required(1)?)?;
        if version != BOOTSTRAP_IMPORT_SCHEMA_VERSION {
            return Err(BootstrapImportError::UnsupportedVersion(version));
        }
        let workspace_id = WorkspaceId::from_uuid(Uuid::from_bytes(array_16(fields.required(2)?)?));
        let graph_resource = CanonicalGraphResourceId::from_bytes(array_32(fields.required(3)?)?);
        let archive_identity =
            BootstrapArchiveIdentityDigestV1::from_bytes(array_32(fields.required(4)?)?);
        let import_id = ImportId::from_digest(array_32(fields.required(5)?)?);
        let complete_source_count = read_u32(fields.required(6)?)?;
        let source_inventory_root = SourceInventoryRootV1::decode(fields.required(7)?)?;
        let source_blob_root = SourceBlobChunkRootV1::decode(fields.required(8)?)?;
        let profile_digest = BootstrapProfileDigestV1::from_bytes(array_32(fields.required(9)?)?);
        let peer_probe = BootstrapPeerProbeV1::decode(fields.required(10)?)?;
        let parts = decode_parts(fields.required(11)?, import_id, profile_digest)?;
        let initial_frontier = decode_archive_frontier(fields.required(12)?)?;
        let final_frontier = decode_archive_frontier(fields.required(13)?)?;
        let final_frontier_proof =
            ArchiveFinalFrontierProofV1::from_bytes(array_32(fields.required(14)?)?);
        let value = Self::new(
            workspace_id,
            graph_resource,
            archive_identity,
            import_id,
            complete_source_count,
            source_inventory_root,
            source_blob_root,
            profile_digest,
            peer_probe,
            parts,
            initial_frontier,
            final_frontier,
            final_frontier_proof,
        )?;
        if value.encode()?.as_slice() != bytes {
            return Err(BootstrapImportError::NonCanonicalBytes);
        }
        Ok(value)
    }

    pub(crate) fn aggregate_digest(&self) -> BootstrapAggregateDigestV1 {
        let bytes = self
            .encode()
            .expect("validated bounded aggregate manifest encodes");
        BootstrapAggregateDigestV1::digest(
            b"tine/bootstrap-import/aggregate-digest/v1\0",
            &[&bytes],
        )
    }

    pub(crate) fn parts(&self) -> &[BootstrapPartDescriptorV1] {
        &self.parts
    }

    pub(crate) fn final_frontier(&self) -> ArchiveLocalFrontierBindingV1 {
        self.final_frontier
    }

    fn encode_parts(&self) -> Result<Vec<u8>, BootstrapImportError> {
        checked_count(self.parts.len(), MAX_BOOTSTRAP_PARTS, "bootstrap parts")?;
        let mut bytes = Vec::with_capacity(4 + self.parts.len() * PART_DESCRIPTOR_BYTES);
        bytes.extend_from_slice(&(self.parts.len() as u32).to_be_bytes());
        for part in &self.parts {
            bytes.extend_from_slice(&part.encode());
        }
        Ok(bytes)
    }

    fn validate(&self) -> Result<(), BootstrapImportError> {
        BootstrapPartitionProfileV1::validate_digest(self.profile_digest)?;
        self.source_inventory_root.validate()?;
        self.source_blob_root.validate()?;
        if self.complete_source_count != self.source_inventory_root.source_count() {
            return Err(BootstrapImportError::SourceCountMismatch);
        }
        checked_count(self.parts.len(), MAX_BOOTSTRAP_PARTS, "bootstrap parts")?;
        self.initial_frontier.validate()?;
        self.final_frontier.validate()?;
        let expected_initial =
            ArchiveLocalFrontierBindingV1::initial(self.import_id, self.profile_digest);
        if self.initial_frontier != expected_initial {
            return Err(BootstrapImportError::InitialFrontierMismatch);
        }

        let part_count = self.parts.len() as u32;
        let mut predecessor = None;
        let mut frontier = self.initial_frontier;
        for (index, part) in self.parts.iter().enumerate() {
            let ordinal = index as u32;
            part.validate_self()?;
            if part.evidence.import_id() != self.import_id
                || part.evidence.profile_digest() != self.profile_digest
                || part.evidence.ordinal() != ordinal
                || part.evidence.part_count() != part_count
            {
                return Err(BootstrapImportError::PartContextMismatch);
            }
            if part.evidence.predecessor() != predecessor {
                return Err(BootstrapImportError::PredecessorMismatch);
            }
            let expected_sequence = ordinal
                .checked_add(1)
                .ok_or(BootstrapImportError::LengthOverflow)?;
            if part.acceptance_sequence != expected_sequence {
                return Err(BootstrapImportError::AcceptanceSequenceMismatch);
            }
            if part.prior_frontier != frontier {
                return Err(BootstrapImportError::FrontierChainMismatch);
            }
            let expected_post = frontier.advance(part.part_id, part.accepted_event)?;
            if part.post_frontier != expected_post {
                return Err(BootstrapImportError::FrontierChainMismatch);
            }
            predecessor = Some(part.part_id);
            frontier = expected_post;
        }
        if self.final_frontier != frontier {
            return Err(BootstrapImportError::FinalFrontierMismatch);
        }
        if self.final_frontier_proof
            != final_frontier_proof(
                self.workspace_id,
                self.graph_resource,
                self.archive_identity,
                self.import_id,
                self.profile_digest,
                self.final_frontier,
            )
        {
            return Err(BootstrapImportError::FinalFrontierProofMismatch);
        }
        Ok(())
    }
}

const PART_DESCRIPTOR_BYTES: usize = 495;

fn accepted_event_binding(
    part_id: BootstrapPartId,
    evidence_digest: BootstrapEvidenceDigestV1,
    manifest_fingerprint: BootstrapManifestFingerprintV1,
    full_object_root: FullObjectRootV1,
    acceptance_sequence: u32,
) -> BootstrapAcceptedEventBindingV1 {
    BootstrapAcceptedEventBindingV1::digest(
        b"tine/bootstrap-import/accepted-event-binding/v1\0",
        &[
            part_id.as_bytes(),
            evidence_digest.as_bytes(),
            manifest_fingerprint.as_bytes(),
            &root_bytes(full_object_root),
            &acceptance_sequence.to_be_bytes(),
        ],
    )
}

fn final_frontier_proof(
    workspace_id: WorkspaceId,
    graph_resource: CanonicalGraphResourceId,
    archive_identity: BootstrapArchiveIdentityDigestV1,
    import_id: ImportId,
    profile_digest: BootstrapProfileDigestV1,
    final_frontier: ArchiveLocalFrontierBindingV1,
) -> ArchiveFinalFrontierProofV1 {
    ArchiveFinalFrontierProofV1::digest(
        b"tine/bootstrap-import/archive-final-frontier-proof/v1\0",
        &[
            workspace_id.as_uuid().as_bytes(),
            graph_resource.as_bytes(),
            archive_identity.as_bytes(),
            import_id.as_bytes(),
            profile_digest.as_bytes(),
            &archive_frontier_bytes(final_frontier),
        ],
    )
}

fn decode_parts(
    bytes: &[u8],
    import_id: ImportId,
    profile_digest: BootstrapProfileDigestV1,
) -> Result<Vec<BootstrapPartDescriptorV1>, BootstrapImportError> {
    if bytes.len() < 4 {
        return Err(BootstrapImportError::Truncated);
    }
    let count = read_u32(&bytes[..4])?;
    if count > MAX_BOOTSTRAP_PARTS {
        return Err(BootstrapImportError::CountLimit("bootstrap parts"));
    }
    let expected = 4_usize
        .checked_add(
            (count as usize)
                .checked_mul(PART_DESCRIPTOR_BYTES)
                .ok_or(BootstrapImportError::LengthOverflow)?,
        )
        .ok_or(BootstrapImportError::LengthOverflow)?;
    if expected != bytes.len() {
        return Err(BootstrapImportError::InvalidFieldLength);
    }
    let mut parts = Vec::with_capacity(count as usize);
    for descriptor in bytes[4..].chunks_exact(PART_DESCRIPTOR_BYTES) {
        parts.push(BootstrapPartDescriptorV1::decode(
            descriptor,
            import_id,
            profile_digest,
        )?);
    }
    Ok(parts)
}

trait RootEncodeV1 {
    fn encode_root(self, bytes: &mut Vec<u8>);
}

impl RootEncodeV1 for SourceInventoryRootV1 {
    fn encode_root(self, bytes: &mut Vec<u8>) {
        self.encode_into(bytes);
    }
}

impl RootEncodeV1 for SourceBlobChunkRootV1 {
    fn encode_root(self, bytes: &mut Vec<u8>) {
        self.encode_into(bytes);
    }
}

impl RootEncodeV1 for SourceSpanRootV1 {
    fn encode_root(self, bytes: &mut Vec<u8>) {
        self.encode_into(bytes);
    }
}

impl RootEncodeV1 for OperationRootV1 {
    fn encode_root(self, bytes: &mut Vec<u8>) {
        self.encode_into(bytes);
    }
}

impl RootEncodeV1 for PayloadObjectRootV1 {
    fn encode_root(self, bytes: &mut Vec<u8>) {
        self.encode_into(bytes);
    }
}

fn root_bytes<T: RootEncodeV1>(root: T) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(44);
    root.encode_root(&mut bytes);
    bytes
}

fn archive_frontier_bytes(frontier: ArchiveLocalFrontierBindingV1) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(69);
    bytes.extend_from_slice(&frontier.digest);
    bytes.extend_from_slice(&frontier.accepted_count.to_be_bytes());
    match frontier.last_part {
        None => {
            bytes.push(0);
            bytes.extend_from_slice(&[0; 32]);
        }
        Some(part_id) => {
            bytes.push(1);
            bytes.extend_from_slice(part_id.as_bytes());
        }
    }
    bytes
}

fn decode_archive_frontier(
    bytes: &[u8],
) -> Result<ArchiveLocalFrontierBindingV1, BootstrapImportError> {
    if bytes.len() != 69 {
        return Err(BootstrapImportError::InvalidFieldLength);
    }
    let last_part = match bytes[36] {
        0 if bytes[37..].iter().all(|byte| *byte == 0) => None,
        1 => Some(BootstrapPartId::from_digest(array_32(&bytes[37..])?)),
        _ => return Err(BootstrapImportError::NonCanonicalBytes),
    };
    let value = ArchiveLocalFrontierBindingV1 {
        digest: array_32(&bytes[..32])?,
        accepted_count: read_u32(&bytes[32..36])?,
        last_part,
    };
    value.validate()?;
    Ok(value)
}

fn part_id_option_bytes(part_id: Option<BootstrapPartId>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(33);
    match part_id {
        None => {
            bytes.push(0);
            bytes.extend_from_slice(&[0; 32]);
        }
        Some(part_id) => {
            bytes.push(1);
            bytes.extend_from_slice(part_id.as_bytes());
        }
    }
    bytes
}

fn decode_part_id_option(bytes: &[u8]) -> Result<Option<BootstrapPartId>, BootstrapImportError> {
    if bytes.len() != 33 {
        return Err(BootstrapImportError::InvalidFieldLength);
    }
    match bytes[0] {
        0 if bytes[1..].iter().all(|byte| *byte == 0) => Ok(None),
        1 => Ok(Some(BootstrapPartId::from_digest(array_32(&bytes[1..])?))),
        _ => Err(BootstrapImportError::NonCanonicalBytes),
    }
}

fn canonical_encode(domain: &[u8], fields: &[&[u8]]) -> Vec<u8> {
    let capacity = domain.len() + fields.iter().map(|field| 8 + field.len()).sum::<usize>();
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(domain);
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    bytes
}

fn canonical_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}

fn put_field(bytes: &mut Vec<u8>, tag: u8, value: &[u8]) -> Result<(), BootstrapImportError> {
    let length = u32::try_from(value.len()).map_err(|_| BootstrapImportError::LengthOverflow)?;
    let projected = bytes
        .len()
        .checked_add(5)
        .and_then(|length| length.checked_add(value.len()))
        .ok_or(BootstrapImportError::LengthOverflow)?;
    if projected > MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES {
        return Err(BootstrapImportError::EncodedSizeLimit("canonical value"));
    }
    bytes.push(tag);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

struct CanonicalFieldsV1<'a> {
    fields: Vec<Option<&'a [u8]>>,
}

impl<'a> CanonicalFieldsV1<'a> {
    fn parse(
        bytes: &'a [u8],
        magic: &[u8; 8],
        max_tag: u8,
        depth: u8,
    ) -> Result<Self, BootstrapImportError> {
        if depth > MAX_CANONICAL_NESTING_DEPTH {
            return Err(BootstrapImportError::DepthLimit);
        }
        if bytes.len() < magic.len() || &bytes[..magic.len()] != magic {
            return Err(BootstrapImportError::InvalidMagic);
        }
        let mut fields = vec![None; usize::from(max_tag) + 1];
        let mut cursor = magic.len();
        let mut last_tag = 0;
        while cursor < bytes.len() {
            if bytes.len() - cursor < 5 {
                return Err(BootstrapImportError::Truncated);
            }
            let tag = bytes[cursor];
            cursor += 1;
            if tag == 0 || tag > max_tag {
                return Err(BootstrapImportError::UnknownField(tag));
            }
            if tag == last_tag {
                return Err(BootstrapImportError::DuplicateField(tag));
            }
            if tag < last_tag {
                return Err(BootstrapImportError::NonCanonicalFieldOrder);
            }
            last_tag = tag;
            let length = u32::from_be_bytes(
                bytes[cursor..cursor + 4]
                    .try_into()
                    .expect("four-byte field length"),
            ) as usize;
            cursor += 4;
            let end = cursor
                .checked_add(length)
                .ok_or(BootstrapImportError::LengthOverflow)?;
            if end > bytes.len() {
                return Err(BootstrapImportError::Truncated);
            }
            fields[usize::from(tag)] = Some(&bytes[cursor..end]);
            cursor = end;
        }
        Ok(Self { fields })
    }

    fn required(&self, tag: u8) -> Result<&'a [u8], BootstrapImportError> {
        self.fields
            .get(usize::from(tag))
            .and_then(|field| *field)
            .ok_or(BootstrapImportError::MissingField(tag))
    }
}

struct FixedReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> FixedReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], BootstrapImportError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(BootstrapImportError::LengthOverflow)?;
        if end > self.bytes.len() {
            return Err(BootstrapImportError::Truncated);
        }
        let value = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(value)
    }

    fn array_16(&mut self) -> Result<[u8; 16], BootstrapImportError> {
        array_16(self.take(16)?)
    }

    fn array_32(&mut self) -> Result<[u8; 32], BootstrapImportError> {
        array_32(self.take(32)?)
    }

    fn u32(&mut self) -> Result<u32, BootstrapImportError> {
        read_u32(self.take(4)?)
    }

    fn finish(self) -> Result<(), BootstrapImportError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(BootstrapImportError::TrailingBytes)
        }
    }
}

fn array_16(bytes: &[u8]) -> Result<[u8; 16], BootstrapImportError> {
    bytes
        .try_into()
        .map_err(|_| BootstrapImportError::InvalidFieldLength)
}

fn array_32(bytes: &[u8]) -> Result<[u8; 32], BootstrapImportError> {
    bytes
        .try_into()
        .map_err(|_| BootstrapImportError::InvalidFieldLength)
}

fn read_u32(bytes: &[u8]) -> Result<u32, BootstrapImportError> {
    bytes
        .try_into()
        .map(u32::from_be_bytes)
        .map_err(|_| BootstrapImportError::InvalidFieldLength)
}

fn read_u64(bytes: &[u8]) -> Result<u64, BootstrapImportError> {
    bytes
        .try_into()
        .map(u64::from_be_bytes)
        .map_err(|_| BootstrapImportError::InvalidFieldLength)
}

fn checked_count(length: usize, max: u32, label: &'static str) -> Result<(), BootstrapImportError> {
    if length > max as usize {
        return Err(BootstrapImportError::CountLimit(label));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapImportError {
    AcceptedEventBindingMismatch,
    AcceptanceSequenceMismatch,
    BatchIdMismatch,
    ByteLimit(&'static str),
    CountLimit(&'static str),
    DepthLimit,
    DuplicateCanonicalItem,
    DuplicateField(u8),
    EmptyOperationTransaction,
    EmptySourceSpan,
    EncodedSizeLimit(&'static str),
    EvidenceDigestMismatch,
    FinalFrontierMismatch,
    FinalFrontierProofMismatch,
    FrontierChainMismatch,
    FullObjectRootMismatch,
    InitialFrontierMismatch,
    InvalidArchiveFrontier,
    InvalidFieldLength,
    InvalidMagic,
    InvalidOrdinal,
    InvalidPartCount,
    LengthOverflow,
    LocatorLimit,
    MissingField(u8),
    MissingPredecessor,
    NonCanonicalBytes,
    NonCanonicalFieldOrder,
    NonCanonicalOrder,
    PartContextMismatch,
    PartIdMismatch,
    PredecessorMismatch,
    ProfileMismatch,
    SourceCountMismatch,
    TrailingBytes,
    Truncated,
    UnexpectedPredecessor,
    UnknownField(u8),
    UnsupportedVersion(u32),
}

impl fmt::Display for BootstrapImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bootstrap-import canonical validation failed: {self:?}")
    }
}

impl std::error::Error for BootstrapImportError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn import_id() -> ImportId {
        ImportId::from_digest([0x11; 32])
    }

    fn workspace_id() -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(0x1020_3040_5060_7080_90a0_b0c0_d0e0_f001))
    }

    fn graph_resource() -> CanonicalGraphResourceId {
        CanonicalGraphResourceId::from_bytes([0x22; 32])
    }

    fn archive_identity() -> BootstrapArchiveIdentityDigestV1 {
        BootstrapArchiveIdentityDigestV1::from_bytes([0x33; 32])
    }

    fn profile() -> BootstrapProfileDigestV1 {
        BootstrapPartitionProfileV1::v1().digest()
    }

    fn operation(index: u32) -> OperationLeafV1 {
        let mut digest = [0_u8; 32];
        digest[..4].copy_from_slice(&index.to_be_bytes());
        OperationLeafV1::new(OperationDigestV1::from_bytes(digest), u64::from(index % 7)).unwrap()
    }

    fn payload(index: u32) -> PayloadObjectDescriptorV1 {
        let mut digest = [0_u8; 32];
        digest[28..].copy_from_slice(&index.to_be_bytes());
        PayloadObjectDescriptorV1::new(PayloadObjectDigestV1::from_bytes(digest), 16).unwrap()
    }

    fn source_span(index: u32) -> SourceSpanV1 {
        let mut digest = [0_u8; 32];
        digest[28..].copy_from_slice(&index.to_be_bytes());
        SourceSpanV1::new(
            SourceLeafDigestV1::from_bytes(digest),
            u64::from(index) * 16,
            16,
        )
        .unwrap()
    }

    fn evidence(
        ordinal: u32,
        count: u32,
        predecessor: Option<BootstrapPartId>,
    ) -> BootstrapImportPartEvidenceV1 {
        BootstrapImportPartEvidenceV1::new(
            import_id(),
            profile(),
            ordinal,
            count,
            SourceSpanRootV1::from_spans(&[source_span(ordinal)]).unwrap(),
            OperationRootV1::from_operations(&[operation(ordinal + 1)]).unwrap(),
            PayloadObjectRootV1::from_objects(&[payload(ordinal + 1)]).unwrap(),
            predecessor,
        )
        .unwrap()
    }

    fn fingerprint(index: u8) -> BootstrapManifestFingerprintV1 {
        BootstrapManifestFingerprintV1::from_bytes([index; 32])
    }

    fn aggregate(
        parts: Vec<BootstrapPartDescriptorV1>,
    ) -> Result<BootstrapAggregateManifestV1, BootstrapImportError> {
        let initial = ArchiveLocalFrontierBindingV1::initial(import_id(), profile());
        let final_frontier = parts.last().map_or(initial, |part| part.post_frontier);
        let proof = final_frontier_proof(
            workspace_id(),
            graph_resource(),
            archive_identity(),
            import_id(),
            profile(),
            final_frontier,
        );
        BootstrapAggregateManifestV1::new(
            workspace_id(),
            graph_resource(),
            archive_identity(),
            import_id(),
            0,
            SourceInventoryRootV1::empty(),
            SourceBlobChunkRootV1::empty(),
            profile(),
            BootstrapPeerProbeV1::new(vec![
                BootstrapPeerProbeEntryV1::new([2; 32], [3; 32]),
                BootstrapPeerProbeEntryV1::new([1; 32], [4; 32]),
            ])
            .unwrap(),
            parts,
            initial,
            final_frontier,
            proof,
        )
    }

    #[test]
    fn identity_is_domain_separated_from_legacy_import_batch() {
        let part = evidence(0, 1, None);
        assert_ne!(part.batch_id(), BatchId::for_import(import_id()));
        assert_ne!(part.part_id().as_bytes(), import_id().as_bytes());
        // The legacy external-observation batch derivation remains a singleton
        // and its bytes are deliberately not altered by this packet.
        assert_eq!(
            BatchId::for_import(import_id()),
            BatchId::for_import(import_id())
        );
    }

    #[test]
    fn canonical_roots_are_incremental_materialized_and_enumeration_stable() {
        let leaves = vec![
            SourceLeafV1::new(
                b"pages/b.md".to_vec(),
                SourceContentDigestV1::from_bytes([2; 32]),
                2,
            )
            .unwrap(),
            SourceLeafV1::new(
                b"pages/a.md".to_vec(),
                SourceContentDigestV1::from_bytes([1; 32]),
                1,
            )
            .unwrap(),
        ];
        let inventory = SourceInventoryRootV1::from_leaves(&leaves).unwrap();
        let reverse_inventory =
            SourceInventoryRootV1::from_leaves(&[leaves[1].clone(), leaves[0].clone()]).unwrap();
        assert_eq!(inventory, reverse_inventory);
        assert_eq!(SourceInventoryRootV1::empty().source_count(), 0);
        assert_eq!(
            SourceInventoryRootV1::from_leaves(&[leaves[0].clone()])
                .unwrap()
                .source_count(),
            1
        );
        let many_leaves = (0..MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART)
            .map(|index| {
                let mut digest = [0; 32];
                digest[28..].copy_from_slice(&index.to_be_bytes());
                SourceLeafV1::new(
                    format!("pages/{index:04}.md").into_bytes(),
                    SourceContentDigestV1::from_bytes(digest),
                    u64::from(index),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let materialized_inventory = SourceInventoryRootV1::from_leaves(&many_leaves).unwrap();
        let mut streaming_inventory = SourceInventoryRootBuilderV1::new();
        for leaf in &many_leaves {
            streaming_inventory.push(leaf).unwrap();
        }
        assert_eq!(streaming_inventory.finish(), materialized_inventory);

        let operations = (0..MAX_OPERATIONS_PER_BOOTSTRAP_PART)
            .map(operation)
            .collect::<Vec<_>>();
        let materialized = OperationRootV1::from_operations(&operations).unwrap();
        let mut streaming = OperationRootBuilderV1::new();
        for item in &operations {
            streaming.push(*item).unwrap();
        }
        assert_eq!(streaming.finish(), materialized);
        assert_eq!(OperationRootV1::empty().operation_count(), 0);
        assert_eq!(
            OperationRootV1::from_operations(&[operation(1)])
                .unwrap()
                .operation_count(),
            1
        );
        assert_eq!(
            materialized.operation_count(),
            MAX_OPERATIONS_PER_BOOTSTRAP_PART
        );
        assert!(OperationRootV1::from_operations(
            &(0..=MAX_OPERATIONS_PER_BOOTSTRAP_PART)
                .map(operation)
                .collect::<Vec<_>>()
        )
        .is_err());

        let objects = [payload(2), payload(1)];
        let root = PayloadObjectRootV1::from_objects(&objects).unwrap();
        let reverse = PayloadObjectRootV1::from_objects(&[objects[1], objects[0]]).unwrap();
        assert_eq!(root, reverse);

        let spans = (0..MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART)
            .map(source_span)
            .collect::<Vec<_>>();
        let materialized_spans = SourceSpanRootV1::from_spans(&spans).unwrap();
        let mut streaming_spans = SourceSpanRootBuilderV1::new();
        for span in &spans {
            streaming_spans.push(*span).unwrap();
        }
        assert_eq!(streaming_spans.finish(), materialized_spans);
        assert_eq!(SourceSpanRootV1::empty().span_count(), 0);
        assert_eq!(
            SourceSpanRootV1::from_spans(&[source_span(1)])
                .unwrap()
                .span_count(),
            1
        );

        let objects = (0..MAX_OPERATIONS_PER_BOOTSTRAP_PART)
            .map(payload)
            .collect::<Vec<_>>();
        let materialized_objects = PayloadObjectRootV1::from_objects(&objects).unwrap();
        let mut streaming_objects = PayloadObjectRootBuilderV1::new();
        for object in &objects {
            streaming_objects.push(*object).unwrap();
        }
        assert_eq!(streaming_objects.finish(), materialized_objects);

        let chunks = (0..MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART)
            .map(|index| {
                let mut digest = [0; 32];
                digest[28..].copy_from_slice(&index.to_be_bytes());
                SourceBlobChunkDescriptorV1::new(
                    SourceLeafDigestV1::from_bytes([9; 32]),
                    index,
                    MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART,
                    u64::from(index),
                    1,
                    SourceBlobChunkDigestV1::from_bytes(digest),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let materialized_chunks = SourceBlobChunkRootV1::from_descriptors(&chunks).unwrap();
        let mut streaming_chunks = SourceBlobChunkRootBuilderV1::new();
        for chunk in &chunks {
            streaming_chunks.push(*chunk).unwrap();
        }
        assert_eq!(streaming_chunks.finish(), materialized_chunks);
        assert_eq!(SourceBlobChunkRootV1::empty().chunk_count(), 0);
    }

    #[test]
    fn evidence_canonical_parser_rejects_hostile_and_noncanonical_bytes() {
        let evidence = evidence(0, 1, None);
        let bytes = evidence.encode().unwrap();
        assert_eq!(
            BootstrapImportPartEvidenceV1::decode(&bytes).unwrap(),
            evidence
        );
        assert!(matches!(
            BootstrapImportPartEvidenceV1::decode(&bytes[..bytes.len() - 1]),
            Err(BootstrapImportError::Truncated)
        ));
        let mut future = bytes.clone();
        future[16] = 2;
        assert!(matches!(
            BootstrapImportPartEvidenceV1::decode(&future),
            Err(BootstrapImportError::UnsupportedVersion(2))
        ));
        let mut unknown = bytes.clone();
        unknown.extend_from_slice(&[10, 0, 0, 0, 0]);
        assert!(matches!(
            BootstrapImportPartEvidenceV1::decode(&unknown),
            Err(BootstrapImportError::UnknownField(10))
        ));
        let mut duplicate = bytes.clone();
        duplicate.extend_from_slice(&[9, 0, 0, 0, 0]);
        assert!(matches!(
            BootstrapImportPartEvidenceV1::decode(&duplicate),
            Err(BootstrapImportError::DuplicateField(9))
        ));
        let mut reordered = bytes.clone();
        // First field tag after the magic is 1.  Making it 2 produces a
        // missing field after parsing, a canonical rejection without allocation.
        reordered[8] = 2;
        assert!(BootstrapImportPartEvidenceV1::decode(&reordered).is_err());
        assert!(matches!(
            BootstrapImportPartEvidenceV1::decode(&vec![0; MAX_BOOTSTRAP_PART_EVIDENCE_BYTES + 1]),
            Err(BootstrapImportError::EncodedSizeLimit("part evidence"))
        ));
        assert!(matches!(
            BootstrapPeerProbeV1::decode(&(MAX_SELECTED_PEERS + 1).to_be_bytes()),
            Err(BootstrapImportError::CountLimit("selected peers"))
        ));
    }

    #[test]
    fn part_constraints_cover_empty_transaction_and_chain_substitution() {
        assert!(matches!(
            BootstrapImportPartEvidenceV1::new(
                import_id(),
                profile(),
                0,
                1,
                SourceSpanRootV1::empty(),
                OperationRootV1::empty(),
                PayloadObjectRootV1::empty(),
                None,
            ),
            Err(BootstrapImportError::EmptyOperationTransaction)
        ));
        assert!(matches!(
            BootstrapImportPartEvidenceV1::new(
                import_id(),
                profile(),
                0,
                1,
                SourceSpanRootV1::empty(),
                OperationRootV1::from_operations(&[operation(1)]).unwrap(),
                PayloadObjectRootV1::empty(),
                Some(BootstrapPartId::from_digest([9; 32])),
            ),
            Err(BootstrapImportError::UnexpectedPredecessor)
        ));

        let initial = ArchiveLocalFrontierBindingV1::initial(import_id(), profile());
        let first =
            BootstrapPartDescriptorV1::accepted(evidence(0, 2, None), fingerprint(1), initial)
                .unwrap();
        let second = BootstrapPartDescriptorV1::accepted(
            evidence(1, 2, Some(first.part_id())),
            fingerprint(2),
            first.post_frontier,
        )
        .unwrap();
        assert!(aggregate(vec![first, second]).unwrap().encode().is_ok());

        let wrong_predecessor = BootstrapPartDescriptorV1::accepted(
            evidence(1, 2, Some(BootstrapPartId::from_digest([7; 32]))),
            fingerprint(2),
            first.post_frontier,
        )
        .unwrap();
        assert!(aggregate(vec![first, wrong_predecessor]).is_err());
        let substituted = BootstrapPartDescriptorV1::accepted(
            evidence(1, 2, Some(first.part_id())),
            fingerprint(9),
            first.post_frontier,
        )
        .unwrap();
        let mut reordered = vec![substituted, first];
        assert!(aggregate(std::mem::take(&mut reordered)).is_err());
        assert!(
            aggregate(vec![first]).is_err(),
            "dropping a declared part fails"
        );
        assert!(
            aggregate(vec![first, first]).is_err(),
            "duplicate part fails"
        );

        let mut wrong_batch = first;
        wrong_batch.batch_id = BatchId::from_uuid(Uuid::from_u128(99));
        assert!(aggregate(vec![wrong_batch, second]).is_err());
        let mut wrong_profile = first;
        wrong_profile.evidence.profile_digest = BootstrapProfileDigestV1::from_bytes([8; 32]);
        assert!(aggregate(vec![wrong_profile, second]).is_err());
        let mut wrong_import = first;
        wrong_import.evidence.import_id = ImportId::from_digest([8; 32]);
        assert!(aggregate(vec![wrong_import, second]).is_err());
        let mut wrong_root = first;
        wrong_root.evidence.operation_root =
            OperationRootV1::from_operations(&[operation(99)]).unwrap();
        assert!(aggregate(vec![wrong_root, second]).is_err());
        let mut substituted_binding = second;
        substituted_binding.manifest_fingerprint = fingerprint(9);
        assert!(aggregate(vec![first, substituted_binding]).is_err());
    }

    #[test]
    fn aggregate_validates_zero_one_many_frontier_and_archive_bindings() {
        let empty = BootstrapAggregateManifestV1::empty(
            workspace_id(),
            graph_resource(),
            archive_identity(),
            import_id(),
            SourceInventoryRootV1::empty(),
            SourceBlobChunkRootV1::empty(),
        )
        .unwrap();
        assert_eq!(
            BootstrapAggregateManifestV1::decode(&empty.encode().unwrap())
                .unwrap()
                .aggregate_digest(),
            empty.aggregate_digest()
        );

        let initial = ArchiveLocalFrontierBindingV1::initial(import_id(), profile());
        let one =
            BootstrapPartDescriptorV1::accepted(evidence(0, 1, None), fingerprint(1), initial)
                .unwrap();
        let aggregate = aggregate(vec![one]).unwrap();
        assert!(BootstrapAggregateManifestV1::decode(&aggregate.encode().unwrap()).is_ok());
        let mut wrong_archive = aggregate.clone();
        wrong_archive.archive_identity = BootstrapArchiveIdentityDigestV1::from_bytes([8; 32]);
        assert!(matches!(
            wrong_archive.encode(),
            Err(BootstrapImportError::FinalFrontierProofMismatch)
        ));
        let mut wrong_workspace = aggregate.clone();
        wrong_workspace.workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(2));
        assert!(matches!(
            wrong_workspace.encode(),
            Err(BootstrapImportError::FinalFrontierProofMismatch)
        ));
        let mut wrong_final = aggregate.clone();
        wrong_final.final_frontier = initial;
        assert!(matches!(
            wrong_final.encode(),
            Err(BootstrapImportError::FinalFrontierMismatch)
        ));
        assert!(matches!(
            BootstrapAggregateManifestV1::decode(&vec![
                0;
                MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES + 1
            ]),
            Err(BootstrapImportError::EncodedSizeLimit("aggregate manifest"))
        ));
    }

    #[test]
    fn golden_vectors_are_portable_and_fixed() {
        let leaf = SourceLeafV1::new(
            b"pages/a.md".to_vec(),
            SourceContentDigestV1::from_bytes([0x44; 32]),
            9,
        )
        .unwrap();
        let evidence = evidence(0, 1, None);
        assert_eq!(
            hex(leaf.digest().as_bytes()),
            "45b68c3a687b7a8d12fd80b0e11f95a285f25ffd554cef069867d114796c82bb"
        );
        assert_eq!(
            hex(profile().as_bytes()),
            "4357f4cb61fcef70fe698d6faac1efe30dc96f835093aa63698ee0a078237d27"
        );
        assert_eq!(
            hex(evidence.part_id().as_bytes()),
            "34958ad99352ca3f58f02f89630bac0402a9d10e6f4843034ff00c1d95cc68db"
        );
        assert_eq!(
            evidence.batch_id().to_string(),
            "04300487-fcb6-8b36-847c-0566e189a0ab"
        );
        assert_eq!(
            hex(evidence.evidence_digest().as_bytes()),
            "125850d29272b7ed55b863d3f24648e6cd9b783af94e54fba2dc6af2885c69b5"
        );
    }
}
