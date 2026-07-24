use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use super::identity::{parse_digest, write_hex};
use super::{
    BatchId, DeviceId, DocumentId, FrontierV2, ImportId, SessionId, WorkspaceId,
    MANAGED_ENTITY_SET_VERSION,
};

pub const OPLOG_PROTOCOL_VERSION: u32 = 2;
pub const OPERATION_SCHEMA_VERSION: u32 = 6;
pub const OBJECT_ENVELOPE_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_ENCODING_VERSION: u32 = 4;
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_OBJECT_BYTES: usize = 256 * 1024 * 1024;

const OBJECT_MAGIC: &[u8; 8] = b"TINEOBJ2";
const CHECKSUM_LEN: usize = 32;
const OBJECT_PREFIX_LEN: usize = OBJECT_MAGIC.len() + 4 + 8;
const MAX_OBJECT_HEADER_BYTES: usize = 64 * 1024;

macro_rules! digest_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            pub fn of(bytes: &[u8]) -> Self {
                Self(Sha256::digest(bytes).into())
            }

            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({self})", stringify!($name))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(&self.0, f)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                parse_digest(&value)
                    .map(Self)
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

digest_type!(
    /// SHA-256 identity of a complete encoded operation object.
    ContentDigest
);
digest_type!(
    /// Immutable workspace lineage or genesis digest carried by every batch.
    LineageDigest
);
digest_type!(
    /// Digest of the canonical semantic-effect payload.
    SemanticEffectDigest
);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    SemanticEffect,
    CrdtUpdate,
    ProjectionIntent,
    AnnotatedBaseBlob,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchOrigin {
    LocalMutation,
    ExternalReconciliation { import_id: ImportId },
    BootstrapImport,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CausalPeerId(DeviceId);

impl CausalPeerId {
    pub const fn from_device_id(device_id: DeviceId) -> Self {
        Self(device_id)
    }

    pub const fn as_device_id(self) -> DeviceId {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchCausalDot {
    peer_id: CausalPeerId,
    counter: u64,
}

impl BatchCausalDot {
    pub fn new(peer_id: CausalPeerId, counter: u64) -> Result<Self, BatchError> {
        if counter == 0 {
            return Err(BatchError::InvalidCausalDot);
        }
        Ok(Self { peer_id, counter })
    }

    pub const fn peer_id(self) -> CausalPeerId {
        self.peer_id
    }

    pub const fn counter(self) -> u64 {
        self.counter
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ObjectDescriptor {
    document_id: DocumentId,
    kind: ObjectKind,
    content_digest: ContentDigest,
    encoded_byte_length: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectDescriptorWire {
    document_id: DocumentId,
    kind: ObjectKind,
    content_digest: ContentDigest,
    encoded_byte_length: u64,
}

impl ObjectDescriptor {
    pub fn new(
        document_id: DocumentId,
        kind: ObjectKind,
        content_digest: ContentDigest,
        encoded_byte_length: u64,
    ) -> Result<Self, BatchError> {
        if encoded_byte_length == 0 || encoded_byte_length > MAX_OBJECT_BYTES as u64 {
            return Err(BatchError::InvalidObjectLength(encoded_byte_length));
        }
        Ok(Self {
            document_id,
            kind,
            content_digest,
            encoded_byte_length,
        })
    }

    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub const fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }

    pub const fn encoded_byte_length(&self) -> u64 {
        self.encoded_byte_length
    }
}

impl<'de> Deserialize<'de> for ObjectDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ObjectDescriptorWire::deserialize(deserializer)?;
        Self::new(
            wire.document_id,
            wire.kind,
            wire.content_digest,
            wire.encoded_byte_length,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OperationBatch {
    manifest_encoding_version: u32,
    protocol_version: u32,
    operation_schema_version: u32,
    object_envelope_schema_version: u32,
    managed_entity_set_version: u32,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    batch_id: BatchId,
    author_device_id: DeviceId,
    author_session_id: SessionId,
    origin: BatchOrigin,
    causal_dot: BatchCausalDot,
    causal_dependency_heads: Vec<BatchId>,
    dependency_frontier: FrontierV2,
    semantic_effect_digest: SemanticEffectDigest,
    required_objects: Vec<ObjectDescriptor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationBatchWire {
    manifest_encoding_version: u32,
    protocol_version: u32,
    operation_schema_version: u32,
    object_envelope_schema_version: u32,
    managed_entity_set_version: u32,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    batch_id: BatchId,
    author_device_id: DeviceId,
    author_session_id: SessionId,
    origin: BatchOrigin,
    causal_dot: BatchCausalDot,
    causal_dependency_heads: Vec<BatchId>,
    dependency_frontier: FrontierV2,
    semantic_effect_digest: SemanticEffectDigest,
    required_objects: Vec<ObjectDescriptor>,
}

impl OperationBatch {
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_causality(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        batch_id: BatchId,
        author_device_id: DeviceId,
        author_session_id: SessionId,
        origin: BatchOrigin,
        causal_dot: BatchCausalDot,
        mut causal_dependency_heads: Vec<BatchId>,
        dependency_frontier: FrontierV2,
        semantic_effect_digest: SemanticEffectDigest,
        mut required_objects: Vec<ObjectDescriptor>,
    ) -> Result<Self, BatchError> {
        required_objects.sort_unstable();
        causal_dependency_heads.sort_unstable();
        causal_dependency_heads.dedup();
        let batch = Self {
            manifest_encoding_version: MANIFEST_ENCODING_VERSION,
            protocol_version: OPLOG_PROTOCOL_VERSION,
            operation_schema_version: OPERATION_SCHEMA_VERSION,
            object_envelope_schema_version: OBJECT_ENVELOPE_SCHEMA_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
            workspace_id,
            lineage_digest,
            batch_id,
            author_device_id,
            author_session_id,
            origin,
            causal_dot,
            causal_dependency_heads,
            dependency_frontier,
            semantic_effect_digest,
            required_objects,
        };
        batch.validate()?;
        Ok(batch)
    }

    pub fn encode(&self) -> Result<Vec<u8>, BatchError> {
        let bytes =
            serde_json::to_vec(self).map_err(|error| BatchError::Encode(error.to_string()))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(BatchError::ManifestTooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    /// Decode the current deterministic candidate representation. The exact
    /// bytes remain unfrozen until later receipt/engine format gates, but this
    /// version rejects any non-canonical representation.
    pub fn decode(bytes: &[u8]) -> Result<Self, BatchError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(BatchError::ManifestTooLarge(bytes.len()));
        }
        let wire: OperationBatchWire =
            serde_json::from_slice(bytes).map_err(|error| BatchError::Decode(error.to_string()))?;
        let batch = Self::from_wire(wire);
        batch.validate()?;
        if batch.encode()?.as_slice() != bytes {
            return Err(BatchError::NonCanonicalManifest);
        }
        Ok(batch)
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub const fn author_device_id(&self) -> DeviceId {
        self.author_device_id
    }

    pub const fn author_session_id(&self) -> SessionId {
        self.author_session_id
    }

    pub const fn origin(&self) -> BatchOrigin {
        self.origin
    }

    pub const fn causal_dot(&self) -> BatchCausalDot {
        self.causal_dot
    }

    pub fn causal_dependency_heads(&self) -> &[BatchId] {
        &self.causal_dependency_heads
    }

    pub fn dependency_frontier(&self) -> &FrontierV2 {
        &self.dependency_frontier
    }

    pub const fn semantic_effect_digest(&self) -> SemanticEffectDigest {
        self.semantic_effect_digest
    }

    pub fn required_objects(&self) -> &[ObjectDescriptor] {
        &self.required_objects
    }

    fn from_wire(wire: OperationBatchWire) -> Self {
        Self {
            manifest_encoding_version: wire.manifest_encoding_version,
            protocol_version: wire.protocol_version,
            operation_schema_version: wire.operation_schema_version,
            object_envelope_schema_version: wire.object_envelope_schema_version,
            managed_entity_set_version: wire.managed_entity_set_version,
            workspace_id: wire.workspace_id,
            lineage_digest: wire.lineage_digest,
            batch_id: wire.batch_id,
            author_device_id: wire.author_device_id,
            author_session_id: wire.author_session_id,
            origin: wire.origin,
            causal_dot: wire.causal_dot,
            causal_dependency_heads: wire.causal_dependency_heads,
            dependency_frontier: wire.dependency_frontier,
            semantic_effect_digest: wire.semantic_effect_digest,
            required_objects: wire.required_objects,
        }
    }

    fn validate(&self) -> Result<(), BatchError> {
        for (field, found, expected) in [
            (
                "manifest_encoding_version",
                self.manifest_encoding_version,
                MANIFEST_ENCODING_VERSION,
            ),
            (
                "protocol_version",
                self.protocol_version,
                OPLOG_PROTOCOL_VERSION,
            ),
            (
                "operation_schema_version",
                self.operation_schema_version,
                OPERATION_SCHEMA_VERSION,
            ),
            (
                "object_envelope_schema_version",
                self.object_envelope_schema_version,
                OBJECT_ENVELOPE_SCHEMA_VERSION,
            ),
            (
                "managed_entity_set_version",
                self.managed_entity_set_version,
                MANAGED_ENTITY_SET_VERSION,
            ),
        ] {
            if found != expected {
                return Err(BatchError::UnknownVersion {
                    field,
                    expected,
                    found,
                });
            }
        }

        if self.causal_dot.counter == 0 {
            return Err(BatchError::InvalidCausalDot);
        }
        if !is_strictly_sorted(&self.causal_dependency_heads)
            && !self.causal_dependency_heads.is_empty()
        {
            return Err(BatchError::NonCanonicalCausalDependencies);
        }
        if self
            .causal_dependency_heads
            .binary_search(&self.batch_id)
            .is_ok()
        {
            return Err(BatchError::CausalSelfDependency(self.batch_id));
        }

        if !is_strictly_sorted(&self.required_objects) {
            if let Some(duplicate) = adjacent_duplicate(&self.required_objects) {
                return Err(BatchError::DuplicateDescriptor(duplicate.clone()));
            }
            return Err(BatchError::NonCanonicalDescriptors);
        }

        let mut digests = HashSet::with_capacity(self.required_objects.len());
        let mut crdt_documents = HashSet::new();
        let mut semantic_count = 0;
        for descriptor in &self.required_objects {
            if descriptor.encoded_byte_length == 0
                || descriptor.encoded_byte_length > MAX_OBJECT_BYTES as u64
            {
                return Err(BatchError::InvalidObjectLength(
                    descriptor.encoded_byte_length,
                ));
            }
            if !digests.insert(descriptor.content_digest) {
                return Err(BatchError::DuplicateObjectDigest(descriptor.content_digest));
            }
            match descriptor.kind {
                ObjectKind::SemanticEffect => semantic_count += 1,
                ObjectKind::CrdtUpdate => {
                    if !crdt_documents.insert(descriptor.document_id) {
                        return Err(BatchError::DuplicateCrdtDocument(descriptor.document_id));
                    }
                }
                ObjectKind::ProjectionIntent | ObjectKind::AnnotatedBaseBlob => {}
            }
        }
        if semantic_count != 1 {
            return Err(BatchError::SemanticEffectCardinality(semantic_count));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EncryptionMode {
    None,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectHeader {
    envelope_schema_version: u32,
    workspace_id: WorkspaceId,
    document_id: DocumentId,
    kind: ObjectKind,
    encryption: EncryptionMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationObject {
    workspace_id: WorkspaceId,
    document_id: DocumentId,
    kind: ObjectKind,
    payload: Vec<u8>,
}

impl OperationObject {
    pub fn new(
        workspace_id: WorkspaceId,
        document_id: DocumentId,
        kind: ObjectKind,
        payload: Vec<u8>,
    ) -> Result<Self, BatchError> {
        let object = Self {
            workspace_id,
            document_id,
            kind,
            payload,
        };
        let encoded_len = object.encoded_len()?;
        if encoded_len > MAX_OBJECT_BYTES {
            return Err(BatchError::ObjectTooLarge(encoded_len));
        }
        Ok(object)
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn encode(&self) -> Result<Vec<u8>, BatchError> {
        let header = ObjectHeader {
            envelope_schema_version: OBJECT_ENVELOPE_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            document_id: self.document_id,
            kind: self.kind,
            encryption: EncryptionMode::None,
        };
        let header_bytes =
            serde_json::to_vec(&header).map_err(|error| BatchError::Encode(error.to_string()))?;
        if header_bytes.len() > MAX_OBJECT_HEADER_BYTES {
            return Err(BatchError::ObjectHeaderTooLarge(header_bytes.len()));
        }
        let header_len = u32::try_from(header_bytes.len())
            .map_err(|_| BatchError::ObjectHeaderTooLarge(header_bytes.len()))?;
        let payload_len = u64::try_from(self.payload.len())
            .map_err(|_| BatchError::ObjectTooLarge(usize::MAX))?;
        let total = OBJECT_PREFIX_LEN
            .checked_add(header_bytes.len())
            .and_then(|length| length.checked_add(self.payload.len()))
            .and_then(|length| length.checked_add(CHECKSUM_LEN))
            .ok_or(BatchError::LengthOverflow)?;
        if total > MAX_OBJECT_BYTES {
            return Err(BatchError::ObjectTooLarge(total));
        }
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(OBJECT_MAGIC);
        bytes.extend_from_slice(&header_len.to_be_bytes());
        bytes.extend_from_slice(&payload_len.to_be_bytes());
        bytes.extend_from_slice(&header_bytes);
        bytes.extend_from_slice(&self.payload);
        bytes.extend_from_slice(&Sha256::digest(&bytes));
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, BatchError> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(BatchError::ObjectTooLarge(bytes.len()));
        }
        if bytes.len() < OBJECT_PREFIX_LEN + CHECKSUM_LEN {
            return Err(BatchError::TruncatedObject);
        }
        if &bytes[..OBJECT_MAGIC.len()] != OBJECT_MAGIC {
            return Err(BatchError::InvalidObjectMagic);
        }
        let header_len = u32::from_be_bytes(
            bytes[OBJECT_MAGIC.len()..OBJECT_MAGIC.len() + 4]
                .try_into()
                .expect("fixed header length"),
        ) as usize;
        if header_len > MAX_OBJECT_HEADER_BYTES {
            return Err(BatchError::ObjectHeaderTooLarge(header_len));
        }
        let payload_len = u64::from_be_bytes(
            bytes[OBJECT_MAGIC.len() + 4..OBJECT_PREFIX_LEN]
                .try_into()
                .expect("fixed payload length"),
        );
        let payload_len = usize::try_from(payload_len).map_err(|_| BatchError::LengthOverflow)?;
        let body_len = OBJECT_PREFIX_LEN
            .checked_add(header_len)
            .and_then(|length| length.checked_add(payload_len))
            .ok_or(BatchError::LengthOverflow)?;
        let expected_len = body_len
            .checked_add(CHECKSUM_LEN)
            .ok_or(BatchError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(BatchError::ObjectLengthMismatch {
                expected: expected_len,
                actual: bytes.len(),
            });
        }
        if bytes[body_len..] != Sha256::digest(&bytes[..body_len])[..] {
            return Err(BatchError::ChecksumMismatch);
        }
        let header_bytes = &bytes[OBJECT_PREFIX_LEN..OBJECT_PREFIX_LEN + header_len];
        let header: ObjectHeader = serde_json::from_slice(header_bytes)
            .map_err(|error| BatchError::Decode(error.to_string()))?;
        if header.envelope_schema_version != OBJECT_ENVELOPE_SCHEMA_VERSION {
            return Err(BatchError::UnknownVersion {
                field: "object_envelope_schema_version",
                expected: OBJECT_ENVELOPE_SCHEMA_VERSION,
                found: header.envelope_schema_version,
            });
        }
        if header.encryption != EncryptionMode::None {
            return Err(BatchError::UnsupportedEncryption);
        }
        let canonical_header =
            serde_json::to_vec(&header).map_err(|error| BatchError::Encode(error.to_string()))?;
        if canonical_header.as_slice() != header_bytes {
            return Err(BatchError::NonCanonicalObjectHeader);
        }
        let payload_start = OBJECT_PREFIX_LEN + header_len;
        Ok(Self {
            workspace_id: header.workspace_id,
            document_id: header.document_id,
            kind: header.kind,
            payload: bytes[payload_start..body_len].to_vec(),
        })
    }

    pub fn descriptor(&self) -> Result<ObjectDescriptor, BatchError> {
        let bytes = self.encode()?;
        ObjectDescriptor::new(
            self.document_id,
            self.kind,
            ContentDigest::of(&bytes),
            bytes.len() as u64,
        )
    }

    fn encoded_len(&self) -> Result<usize, BatchError> {
        self.encode().map(|bytes| bytes.len())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A complete generic batch object set. This validates object identity, type,
/// cardinality, descriptor equality, and the declared semantic-effect digest.
/// P1A.2 must validate the semantic effect against dependency-frontier state;
/// P1B.1 must prove which projection intents require which annotated base
/// blobs.
pub struct PreparedBatch {
    manifest: OperationBatch,
    objects: Vec<OperationObject>,
}

impl PreparedBatch {
    pub fn new(
        manifest: OperationBatch,
        objects: Vec<OperationObject>,
    ) -> Result<Self, BatchError> {
        let mut by_digest = BTreeMap::new();
        for object in objects {
            if object.workspace_id != manifest.workspace_id {
                return Err(BatchError::WorkspaceMismatch {
                    expected: manifest.workspace_id,
                    found: object.workspace_id,
                });
            }
            let descriptor = object.descriptor()?;
            let digest = descriptor.content_digest;
            if by_digest.insert(digest, (descriptor, object)).is_some() {
                return Err(BatchError::DuplicateObjectDigest(digest));
            }
        }

        let mut ordered = Vec::with_capacity(manifest.required_objects.len());
        for expected in &manifest.required_objects {
            let Some((actual, object)) = by_digest.remove(&expected.content_digest) else {
                return Err(BatchError::MissingObject(expected.clone()));
            };
            if actual != *expected {
                return Err(BatchError::DescriptorMismatch {
                    expected: expected.clone(),
                    actual,
                });
            }
            ordered.push(object);
        }
        if let Some((_, (descriptor, _))) = by_digest.pop_first() {
            return Err(BatchError::UnexpectedObject(descriptor));
        }
        let semantic = ordered
            .iter()
            .find(|object| object.kind == ObjectKind::SemanticEffect)
            .expect("validated manifests contain exactly one semantic effect");
        let actual_semantic_digest = SemanticEffectDigest::of(semantic.payload());
        if actual_semantic_digest != manifest.semantic_effect_digest {
            return Err(BatchError::SemanticEffectDigestMismatch {
                expected: manifest.semantic_effect_digest,
                actual: actual_semantic_digest,
            });
        }
        super::projection_manifest::validate_projection_object_set(&manifest, &ordered)
            .map_err(|error| BatchError::ProjectionObject(error.to_string()))?;
        Ok(Self {
            manifest,
            objects: ordered,
        })
    }

    pub fn manifest(&self) -> &OperationBatch {
        &self.manifest
    }

    pub fn objects(&self) -> &[OperationObject] {
        &self.objects
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedBatch(PreparedBatch);

impl ValidatedBatch {
    pub(crate) fn new(batch: PreparedBatch) -> Self {
        Self(batch)
    }

    pub fn manifest(&self) -> &OperationBatch {
        self.0.manifest()
    }

    pub fn objects(&self) -> &[OperationObject] {
        self.0.objects()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchError {
    Encode(String),
    Decode(String),
    ManifestTooLarge(usize),
    ObjectTooLarge(usize),
    ObjectHeaderTooLarge(usize),
    UnknownVersion {
        field: &'static str,
        expected: u32,
        found: u32,
    },
    UnsupportedEncryption,
    InvalidCausalDot,
    NonCanonicalCausalDependencies,
    CausalSelfDependency(BatchId),
    NonCanonicalManifest,
    NonCanonicalDescriptors,
    NonCanonicalObjectHeader,
    DuplicateDescriptor(ObjectDescriptor),
    DuplicateObjectDigest(ContentDigest),
    DuplicateCrdtDocument(DocumentId),
    SemanticEffectCardinality(usize),
    SemanticEffectDigestMismatch {
        expected: SemanticEffectDigest,
        actual: SemanticEffectDigest,
    },
    ProjectionObject(String),
    InvalidObjectLength(u64),
    TruncatedObject,
    InvalidObjectMagic,
    LengthOverflow,
    ObjectLengthMismatch {
        expected: usize,
        actual: usize,
    },
    ChecksumMismatch,
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    MissingObject(ObjectDescriptor),
    UnexpectedObject(ObjectDescriptor),
    DescriptorMismatch {
        expected: ObjectDescriptor,
        actual: ObjectDescriptor,
    },
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(error) => write!(f, "batch encode failed: {error}"),
            Self::Decode(error) => write!(f, "batch decode failed: {error}"),
            Self::ManifestTooLarge(length) => write!(f, "manifest is too large: {length} bytes"),
            Self::ObjectTooLarge(length) => write!(f, "object is too large: {length} bytes"),
            Self::ObjectHeaderTooLarge(length) => {
                write!(f, "object header is too large: {length} bytes")
            }
            Self::UnknownVersion {
                field,
                expected,
                found,
            } => {
                write!(f, "unknown {field} {found}; expected {expected}")
            }
            Self::UnsupportedEncryption => f.write_str("only unencrypted objects are supported"),
            Self::InvalidCausalDot => f.write_str("batch causal counter must be nonzero"),
            Self::NonCanonicalCausalDependencies => {
                f.write_str("causal dependency heads are not canonically sorted")
            }
            Self::CausalSelfDependency(batch_id) => {
                write!(f, "batch {batch_id} causally depends on itself")
            }
            Self::NonCanonicalManifest => f.write_str("manifest bytes are not canonical"),
            Self::NonCanonicalDescriptors => {
                f.write_str("object descriptors are not canonically sorted")
            }
            Self::NonCanonicalObjectHeader => f.write_str("object header is not canonical"),
            Self::DuplicateDescriptor(descriptor) => {
                write!(f, "duplicate object descriptor: {descriptor:?}")
            }
            Self::DuplicateObjectDigest(digest) => write!(f, "duplicate object digest {digest}"),
            Self::DuplicateCrdtDocument(document) => {
                write!(f, "duplicate CRDT update for document {document}")
            }
            Self::SemanticEffectCardinality(count) => {
                write!(
                    f,
                    "expected exactly one semantic-effect object, found {count}"
                )
            }
            Self::SemanticEffectDigestMismatch { expected, actual } => write!(
                f,
                "semantic-effect payload digest mismatch: expected {expected}, found {actual}"
            ),
            Self::ProjectionObject(error) => {
                write!(f, "projection object-set validation failed: {error}")
            }
            Self::InvalidObjectLength(length) => {
                write!(f, "invalid encoded object length {length}")
            }
            Self::TruncatedObject => f.write_str("truncated object envelope"),
            Self::InvalidObjectMagic => f.write_str("invalid object envelope magic"),
            Self::LengthOverflow => f.write_str("object envelope length overflow"),
            Self::ObjectLengthMismatch { expected, actual } => write!(
                f,
                "object envelope length mismatch: expected {expected}, found {actual}"
            ),
            Self::ChecksumMismatch => f.write_str("object envelope checksum mismatch"),
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::MissingObject(descriptor) => write!(f, "missing object {descriptor:?}"),
            Self::UnexpectedObject(descriptor) => write!(f, "unexpected object {descriptor:?}"),
            Self::DescriptorMismatch { expected, actual } => write!(
                f,
                "object descriptor mismatch: expected {expected:?}, found {actual:?}"
            ),
        }
    }
}

impl std::error::Error for BatchError {}

fn is_strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn adjacent_duplicate<T: Eq>(values: &[T]) -> Option<&T> {
    values
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| &pair[0])
}
