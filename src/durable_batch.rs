use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::content_digest::{parse_digest, write_hex};
use crate::ContentDigest;

pub const OPLOG_PROTOCOL_VERSION: u32 = 2;
pub const OBJECT_ENVELOPE_SCHEMA_VERSION: u32 = 2;
pub const MANIFEST_ENCODING_VERSION: u32 = 4;
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_OBJECT_BYTES: usize = 256 * 1024 * 1024;

const OBJECT_MAGIC: &[u8; 8] = b"TINEOBJ2";
const CHECKSUM_LEN: usize = 32;
const OBJECT_PREFIX_LEN: usize = OBJECT_MAGIC.len() + 4 + 8;
const MAX_OBJECT_HEADER_BYTES: usize = 64 * 1024;

/// Product-owned types and policy needed by the generic durable batch codec.
pub trait DurableBatchContract: Clone + fmt::Debug + Eq + Hash + Ord + Sized + 'static {
    type WorkspaceId: Copy + fmt::Debug + fmt::Display + Eq + Serialize + DeserializeOwned;
    type DocumentId: Copy
        + fmt::Debug
        + fmt::Display
        + Eq
        + Hash
        + Ord
        + Serialize
        + DeserializeOwned;
    type BatchId: Copy + fmt::Debug + fmt::Display + Eq + Hash + Ord + Serialize + DeserializeOwned;
    type DeviceId: Copy + fmt::Debug + fmt::Display + Eq + Hash + Ord + Serialize + DeserializeOwned;
    type SessionId: Copy + fmt::Debug + fmt::Display + Eq + Serialize + DeserializeOwned;
    type Origin: Copy + fmt::Debug + Eq + Serialize + DeserializeOwned;
    type DependencyFrontier: Clone + fmt::Debug + Eq + Serialize + DeserializeOwned;
    type ManifestValidationState;

    const OPERATION_SCHEMA_VERSION: u32;
    const MANAGED_ENTITY_SET_VERSION: u32;

    fn begin_manifest_validation() -> Self::ManifestValidationState;

    fn validate_descriptor_policy(
        state: &mut Self::ManifestValidationState,
        descriptor: &ObjectDescriptor<Self>,
    ) -> Result<(), BatchError<Self>>;

    fn finish_manifest_validation(
        state: Self::ManifestValidationState,
    ) -> Result<(), BatchError<Self>>;
}

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
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({self})", stringify!($name))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(&self.0, formatter)
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
    /// Immutable lineage or genesis digest carried by every durable batch.
    LineageDigest
);
digest_type!(
    /// Digest of the contract-defined semantic-effect payload.
    SemanticEffectDigest
);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    SemanticEffect,
    CrdtUpdate,
    ProjectionIntent,
    AnnotatedBaseBlob,
    ExternalImportObservation,
}

#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct CausalPeerId<C: DurableBatchContract>(C::DeviceId);

impl<C: DurableBatchContract> Clone for CausalPeerId<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: DurableBatchContract> Copy for CausalPeerId<C> {}

impl<C: DurableBatchContract> CausalPeerId<C> {
    pub const fn from_device_id(device_id: C::DeviceId) -> Self {
        Self(device_id)
    }

    pub const fn as_device_id(self) -> C::DeviceId {
        self.0
    }
}

#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
pub struct BatchCausalDot<C: DurableBatchContract> {
    peer_id: CausalPeerId<C>,
    counter: u64,
}

impl<C: DurableBatchContract> Clone for BatchCausalDot<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: DurableBatchContract> Copy for BatchCausalDot<C> {}

impl<C: DurableBatchContract> BatchCausalDot<C> {
    pub fn new(peer_id: CausalPeerId<C>, counter: u64) -> Result<Self, BatchError<C>> {
        if counter == 0 {
            return Err(BatchError::InvalidCausalDot);
        }
        Ok(Self { peer_id, counter })
    }

    pub const fn peer_id(self) -> CausalPeerId<C> {
        self.peer_id
    }

    pub const fn counter(self) -> u64 {
        self.counter
    }
}

#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(bound(serialize = ""))]
pub struct ObjectDescriptor<C: DurableBatchContract> {
    document_id: C::DocumentId,
    kind: ObjectKind,
    content_digest: ContentDigest,
    encoded_byte_length: u64,
}

impl<C: DurableBatchContract> Clone for ObjectDescriptor<C> {
    fn clone(&self) -> Self {
        Self {
            document_id: self.document_id,
            kind: self.kind,
            content_digest: self.content_digest,
            encoded_byte_length: self.encoded_byte_length,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = ""))]
struct ObjectDescriptorWire<C: DurableBatchContract> {
    document_id: C::DocumentId,
    kind: ObjectKind,
    content_digest: ContentDigest,
    encoded_byte_length: u64,
}

impl<C: DurableBatchContract> ObjectDescriptor<C> {
    pub fn new(
        document_id: C::DocumentId,
        kind: ObjectKind,
        content_digest: ContentDigest,
        encoded_byte_length: u64,
    ) -> Result<Self, BatchError<C>> {
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

    pub const fn document_id(&self) -> C::DocumentId {
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

impl<'de, C: DurableBatchContract> Deserialize<'de> for ObjectDescriptor<C> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ObjectDescriptorWire::<C>::deserialize(deserializer)?;
        Self::new(
            wire.document_id,
            wire.kind,
            wire.content_digest,
            wire.encoded_byte_length,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub struct OperationBatch<C: DurableBatchContract> {
    manifest_encoding_version: u32,
    protocol_version: u32,
    operation_schema_version: u32,
    object_envelope_schema_version: u32,
    managed_entity_set_version: u32,
    workspace_id: C::WorkspaceId,
    lineage_digest: LineageDigest,
    batch_id: C::BatchId,
    author_device_id: C::DeviceId,
    author_session_id: C::SessionId,
    origin: C::Origin,
    causal_dot: BatchCausalDot<C>,
    causal_dependency_heads: Vec<C::BatchId>,
    dependency_frontier: C::DependencyFrontier,
    semantic_effect_digest: SemanticEffectDigest,
    required_objects: Vec<ObjectDescriptor<C>>,
}

impl<C: DurableBatchContract> Clone for OperationBatch<C> {
    fn clone(&self) -> Self {
        Self {
            manifest_encoding_version: self.manifest_encoding_version,
            protocol_version: self.protocol_version,
            operation_schema_version: self.operation_schema_version,
            object_envelope_schema_version: self.object_envelope_schema_version,
            managed_entity_set_version: self.managed_entity_set_version,
            workspace_id: self.workspace_id,
            lineage_digest: self.lineage_digest,
            batch_id: self.batch_id,
            author_device_id: self.author_device_id,
            author_session_id: self.author_session_id,
            origin: self.origin,
            causal_dot: self.causal_dot,
            causal_dependency_heads: self.causal_dependency_heads.clone(),
            dependency_frontier: self.dependency_frontier.clone(),
            semantic_effect_digest: self.semantic_effect_digest,
            required_objects: self.required_objects.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = ""))]
struct OperationBatchWire<C: DurableBatchContract> {
    manifest_encoding_version: u32,
    protocol_version: u32,
    operation_schema_version: u32,
    object_envelope_schema_version: u32,
    managed_entity_set_version: u32,
    workspace_id: C::WorkspaceId,
    lineage_digest: LineageDigest,
    batch_id: C::BatchId,
    author_device_id: C::DeviceId,
    author_session_id: C::SessionId,
    origin: C::Origin,
    causal_dot: BatchCausalDot<C>,
    causal_dependency_heads: Vec<C::BatchId>,
    dependency_frontier: C::DependencyFrontier,
    semantic_effect_digest: SemanticEffectDigest,
    required_objects: Vec<ObjectDescriptor<C>>,
}

impl<C: DurableBatchContract> OperationBatch<C> {
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_causality(
        workspace_id: C::WorkspaceId,
        lineage_digest: LineageDigest,
        batch_id: C::BatchId,
        author_device_id: C::DeviceId,
        author_session_id: C::SessionId,
        origin: C::Origin,
        causal_dot: BatchCausalDot<C>,
        mut causal_dependency_heads: Vec<C::BatchId>,
        dependency_frontier: C::DependencyFrontier,
        semantic_effect_digest: SemanticEffectDigest,
        mut required_objects: Vec<ObjectDescriptor<C>>,
    ) -> Result<Self, BatchError<C>> {
        required_objects.sort_unstable();
        causal_dependency_heads.sort_unstable();
        causal_dependency_heads.dedup();
        let batch = Self {
            manifest_encoding_version: MANIFEST_ENCODING_VERSION,
            protocol_version: OPLOG_PROTOCOL_VERSION,
            operation_schema_version: C::OPERATION_SCHEMA_VERSION,
            object_envelope_schema_version: OBJECT_ENVELOPE_SCHEMA_VERSION,
            managed_entity_set_version: C::MANAGED_ENTITY_SET_VERSION,
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

    pub fn encode(&self) -> Result<Vec<u8>, BatchError<C>> {
        let bytes =
            serde_json::to_vec(self).map_err(|error| BatchError::Encode(error.to_string()))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(BatchError::ManifestTooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    /// Decode the deterministic representation, refusing non-canonical bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, BatchError<C>> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(BatchError::ManifestTooLarge(bytes.len()));
        }
        let wire: OperationBatchWire<C> =
            serde_json::from_slice(bytes).map_err(|error| BatchError::Decode(error.to_string()))?;
        let batch = Self::from_wire(wire);
        batch.validate()?;
        if batch.encode()?.as_slice() != bytes {
            return Err(BatchError::NonCanonicalManifest);
        }
        Ok(batch)
    }

    pub const fn workspace_id(&self) -> C::WorkspaceId {
        self.workspace_id
    }

    pub const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub const fn batch_id(&self) -> C::BatchId {
        self.batch_id
    }

    pub const fn author_device_id(&self) -> C::DeviceId {
        self.author_device_id
    }

    pub const fn author_session_id(&self) -> C::SessionId {
        self.author_session_id
    }

    pub const fn origin(&self) -> C::Origin {
        self.origin
    }

    pub const fn causal_dot(&self) -> BatchCausalDot<C> {
        self.causal_dot
    }

    pub fn causal_dependency_heads(&self) -> &[C::BatchId] {
        &self.causal_dependency_heads
    }

    pub fn dependency_frontier(&self) -> &C::DependencyFrontier {
        &self.dependency_frontier
    }

    pub const fn semantic_effect_digest(&self) -> SemanticEffectDigest {
        self.semantic_effect_digest
    }

    pub fn required_objects(&self) -> &[ObjectDescriptor<C>] {
        &self.required_objects
    }

    #[doc(hidden)]
    pub fn required_objects_for_document_kind(
        &self,
        document_id: C::DocumentId,
        kind: ObjectKind,
    ) -> &[ObjectDescriptor<C>] {
        let key = (document_id, kind);
        let start = self
            .required_objects
            .partition_point(|descriptor| (descriptor.document_id(), descriptor.kind()) < key);
        let end = start
            + self.required_objects[start..]
                .partition_point(|descriptor| (descriptor.document_id(), descriptor.kind()) == key);
        &self.required_objects[start..end]
    }

    fn from_wire(wire: OperationBatchWire<C>) -> Self {
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

    fn validate(&self) -> Result<(), BatchError<C>> {
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
                C::OPERATION_SCHEMA_VERSION,
            ),
            (
                "object_envelope_schema_version",
                self.object_envelope_schema_version,
                OBJECT_ENVELOPE_SCHEMA_VERSION,
            ),
            (
                "managed_entity_set_version",
                self.managed_entity_set_version,
                C::MANAGED_ENTITY_SET_VERSION,
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
        let mut policy_state = C::begin_manifest_validation();
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
            C::validate_descriptor_policy(&mut policy_state, descriptor)?;
        }
        C::finish_manifest_validation(policy_state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EncryptionMode {
    None,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
struct ObjectHeader<C: DurableBatchContract> {
    envelope_schema_version: u32,
    workspace_id: C::WorkspaceId,
    document_id: C::DocumentId,
    kind: ObjectKind,
    encryption: EncryptionMode,
}

#[derive(Debug, Eq, PartialEq)]
pub struct OperationObject<C: DurableBatchContract> {
    workspace_id: C::WorkspaceId,
    document_id: C::DocumentId,
    kind: ObjectKind,
    payload: Vec<u8>,
}

impl<C: DurableBatchContract> Clone for OperationObject<C> {
    fn clone(&self) -> Self {
        Self {
            workspace_id: self.workspace_id,
            document_id: self.document_id,
            kind: self.kind,
            payload: self.payload.clone(),
        }
    }
}

impl<C: DurableBatchContract> OperationObject<C> {
    pub fn new(
        workspace_id: C::WorkspaceId,
        document_id: C::DocumentId,
        kind: ObjectKind,
        payload: Vec<u8>,
    ) -> Result<Self, BatchError<C>> {
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

    pub const fn workspace_id(&self) -> C::WorkspaceId {
        self.workspace_id
    }

    pub const fn document_id(&self) -> C::DocumentId {
        self.document_id
    }

    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn encode(&self) -> Result<Vec<u8>, BatchError<C>> {
        let header = ObjectHeader::<C> {
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

    pub fn decode(bytes: &[u8]) -> Result<Self, BatchError<C>> {
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
        let header: ObjectHeader<C> = serde_json::from_slice(header_bytes)
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

    pub fn descriptor(&self) -> Result<ObjectDescriptor<C>, BatchError<C>> {
        let bytes = self.encode()?;
        ObjectDescriptor::new(
            self.document_id,
            self.kind,
            ContentDigest::of(&bytes),
            bytes.len() as u64,
        )
    }

    #[doc(hidden)]
    pub fn encoded_len(&self) -> Result<usize, BatchError<C>> {
        self.encode().map(|bytes| bytes.len())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchError<C: DurableBatchContract> {
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
    CausalSelfDependency(C::BatchId),
    NonCanonicalManifest,
    NonCanonicalDescriptors,
    NonCanonicalObjectHeader,
    DuplicateDescriptor(ObjectDescriptor<C>),
    DuplicateObjectDigest(ContentDigest),
    DuplicateCrdtDocument(C::DocumentId),
    SemanticEffectCardinality(usize),
    SemanticEffectDigestMismatch {
        expected: SemanticEffectDigest,
        actual: SemanticEffectDigest,
    },
    ProjectionObject(String),
    ExternalImportObject(String),
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
        expected: C::WorkspaceId,
        found: C::WorkspaceId,
    },
    MissingObject(ObjectDescriptor<C>),
    UnexpectedObject(ObjectDescriptor<C>),
    DescriptorMismatch {
        expected: ObjectDescriptor<C>,
        actual: ObjectDescriptor<C>,
    },
}

impl<C: DurableBatchContract> fmt::Display for BatchError<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(error) => write!(formatter, "batch encode failed: {error}"),
            Self::Decode(error) => write!(formatter, "batch decode failed: {error}"),
            Self::ManifestTooLarge(length) => {
                write!(formatter, "manifest is too large: {length} bytes")
            }
            Self::ObjectTooLarge(length) => {
                write!(formatter, "object is too large: {length} bytes")
            }
            Self::ObjectHeaderTooLarge(length) => {
                write!(formatter, "object header is too large: {length} bytes")
            }
            Self::UnknownVersion {
                field,
                expected,
                found,
            } => write!(formatter, "unknown {field} {found}; expected {expected}"),
            Self::UnsupportedEncryption => {
                formatter.write_str("only unencrypted objects are supported")
            }
            Self::InvalidCausalDot => formatter.write_str("batch causal counter must be nonzero"),
            Self::NonCanonicalCausalDependencies => {
                formatter.write_str("causal dependency heads are not canonically sorted")
            }
            Self::CausalSelfDependency(batch_id) => {
                write!(formatter, "batch {batch_id} causally depends on itself")
            }
            Self::NonCanonicalManifest => formatter.write_str("manifest bytes are not canonical"),
            Self::NonCanonicalDescriptors => {
                formatter.write_str("object descriptors are not canonically sorted")
            }
            Self::NonCanonicalObjectHeader => formatter.write_str("object header is not canonical"),
            Self::DuplicateDescriptor(descriptor) => {
                write!(formatter, "duplicate object descriptor: {descriptor:?}")
            }
            Self::DuplicateObjectDigest(digest) => {
                write!(formatter, "duplicate object digest {digest}")
            }
            Self::DuplicateCrdtDocument(document) => {
                write!(formatter, "duplicate CRDT update for document {document}")
            }
            Self::SemanticEffectCardinality(count) => write!(
                formatter,
                "expected exactly one semantic-effect object, found {count}"
            ),
            Self::SemanticEffectDigestMismatch { expected, actual } => write!(
                formatter,
                "semantic-effect payload digest mismatch: expected {expected}, found {actual}"
            ),
            Self::ProjectionObject(error) => {
                write!(
                    formatter,
                    "projection object-set validation failed: {error}"
                )
            }
            Self::ExternalImportObject(error) => {
                write!(
                    formatter,
                    "external-import object-set validation failed: {error}"
                )
            }
            Self::InvalidObjectLength(length) => {
                write!(formatter, "invalid encoded object length {length}")
            }
            Self::TruncatedObject => formatter.write_str("truncated object envelope"),
            Self::InvalidObjectMagic => formatter.write_str("invalid object envelope magic"),
            Self::LengthOverflow => formatter.write_str("object envelope length overflow"),
            Self::ObjectLengthMismatch { expected, actual } => write!(
                formatter,
                "object envelope length mismatch: expected {expected}, found {actual}"
            ),
            Self::ChecksumMismatch => formatter.write_str("object envelope checksum mismatch"),
            Self::WorkspaceMismatch { expected, found } => {
                write!(
                    formatter,
                    "workspace mismatch: expected {expected}, found {found}"
                )
            }
            Self::MissingObject(descriptor) => write!(formatter, "missing object {descriptor:?}"),
            Self::UnexpectedObject(descriptor) => {
                write!(formatter, "unexpected object {descriptor:?}")
            }
            Self::DescriptorMismatch { expected, actual } => write!(
                formatter,
                "object descriptor mismatch: expected {expected:?}, found {actual:?}"
            ),
        }
    }
}

impl<C: DurableBatchContract> std::error::Error for BatchError<C> {}

fn is_strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn adjacent_duplicate<T: Eq>(values: &[T]) -> Option<&T> {
    values
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| &pair[0])
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use uuid::Uuid;

    use super::*;

    #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct SyntheticContract;

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum SyntheticOrigin {
        LocalMutation,
    }

    impl DurableBatchContract for SyntheticContract {
        type WorkspaceId = Uuid;
        type DocumentId = Uuid;
        type BatchId = Uuid;
        type DeviceId = Uuid;
        type SessionId = Uuid;
        type Origin = SyntheticOrigin;
        type DependencyFrontier = Vec<()>;
        type ManifestValidationState = usize;

        const OPERATION_SCHEMA_VERSION: u32 = 7;
        const MANAGED_ENTITY_SET_VERSION: u32 = 2;

        fn begin_manifest_validation() -> Self::ManifestValidationState {
            0
        }

        fn validate_descriptor_policy(
            semantic_count: &mut Self::ManifestValidationState,
            descriptor: &ObjectDescriptor<Self>,
        ) -> Result<(), BatchError<Self>> {
            if descriptor.kind() == ObjectKind::SemanticEffect {
                *semantic_count += 1;
            }
            Ok(())
        }

        fn finish_manifest_validation(
            semantic_count: Self::ManifestValidationState,
        ) -> Result<(), BatchError<Self>> {
            if semantic_count != 1 {
                return Err(BatchError::SemanticEffectCardinality(semantic_count));
            }
            Ok(())
        }
    }

    type SyntheticObject = OperationObject<SyntheticContract>;
    type SyntheticBatch = OperationBatch<SyntheticContract>;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn golden_object() -> SyntheticObject {
        SyntheticObject::new(
            id(1),
            id(2),
            ObjectKind::SemanticEffect,
            b"golden payload".to_vec(),
        )
        .unwrap()
    }

    fn golden_manifest(descriptor: ObjectDescriptor<SyntheticContract>) -> SyntheticBatch {
        SyntheticBatch::new_with_causality(
            id(1),
            LineageDigest::from_bytes([0x11; 32]),
            id(3),
            id(4),
            id(5),
            SyntheticOrigin::LocalMutation,
            BatchCausalDot::new(CausalPeerId::from_device_id(id(4)), 1).unwrap(),
            Vec::new(),
            Vec::new(),
            SemanticEffectDigest::of(b"golden payload"),
            vec![descriptor],
        )
        .unwrap()
    }

    const PRE_EXTRACTION_OBJECT_HEX: &str = "54494e454f424a32000000b5000000000000000e7b22656e76656c6f70655f736368656d615f76657273696f6e223a322c22776f726b73706163655f6964223a2230303030303030302d303030302d303030302d303030302d303030303030303030303031222c22646f63756d656e745f6964223a2230303030303030302d303030302d303030302d303030302d303030303030303030303032222c226b696e64223a2273656d616e7469635f656666656374222c22656e6372797074696f6e223a226e6f6e65227d676f6c64656e207061796c6f6164f57b1aa52243f7955dec04cd9657510d852e0aa73dc185d898cc532004b3c91a";
    const PRE_EXTRACTION_MANIFEST_BYTES: &[u8] = br#"{"manifest_encoding_version":4,"protocol_version":2,"operation_schema_version":7,"object_envelope_schema_version":2,"managed_entity_set_version":2,"workspace_id":"00000000-0000-0000-0000-000000000001","lineage_digest":"1111111111111111111111111111111111111111111111111111111111111111","batch_id":"00000000-0000-0000-0000-000000000003","author_device_id":"00000000-0000-0000-0000-000000000004","author_session_id":"00000000-0000-0000-0000-000000000005","origin":"local_mutation","causal_dot":{"peer_id":"00000000-0000-0000-0000-000000000004","counter":1},"causal_dependency_heads":[],"dependency_frontier":[],"semantic_effect_digest":"99b2f51b5b653a94d7bb1b0d069192a6ff6f1089ab696b64d00581440cb8e31f","required_objects":[{"document_id":"00000000-0000-0000-0000-000000000002","kind":"semantic_effect","content_digest":"189435215458fe83055927ddd7e60967bcca931b99edc3b9e32b7efc9f6362e1","encoded_byte_length":247}]}"#;

    fn decode_hex_fixture(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => panic!("fixture contains a non-hexadecimal byte"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    #[test]
    fn fixed_pre_extraction_bytes_decode_and_reencode_identically() {
        let object_bytes = golden_object().encode().unwrap();
        let manifest_bytes = golden_manifest(golden_object().descriptor().unwrap())
            .encode()
            .unwrap();
        let pre_extraction_object_bytes = decode_hex_fixture(PRE_EXTRACTION_OBJECT_HEX);

        assert_eq!(object_bytes, pre_extraction_object_bytes);
        assert_eq!(manifest_bytes, PRE_EXTRACTION_MANIFEST_BYTES);
        assert_eq!(
            SyntheticObject::decode(&pre_extraction_object_bytes)
                .unwrap()
                .encode()
                .unwrap(),
            pre_extraction_object_bytes
        );
        assert_eq!(
            SyntheticBatch::decode(PRE_EXTRACTION_MANIFEST_BYTES)
                .unwrap()
                .encode()
                .unwrap(),
            PRE_EXTRACTION_MANIFEST_BYTES
        );
    }

    #[test]
    fn deterministic_roundtrip_and_canonical_refusal_remain_storage_owned() {
        let object = golden_object();
        let object_bytes = object.encode().unwrap();
        assert_eq!(SyntheticObject::decode(&object_bytes).unwrap(), object);
        assert_eq!(object.encode().unwrap(), object_bytes);

        let manifest = golden_manifest(object.descriptor().unwrap());
        let manifest_bytes = manifest.encode().unwrap();
        assert_eq!(SyntheticBatch::decode(&manifest_bytes).unwrap(), manifest);
        assert_eq!(manifest.encode().unwrap(), manifest_bytes);

        let manifest_value: Value = serde_json::from_slice(&manifest_bytes).unwrap();
        let noncanonical_manifest = serde_json::to_vec_pretty(&manifest_value).unwrap();
        assert_eq!(
            SyntheticBatch::decode(&noncanonical_manifest).unwrap_err(),
            BatchError::NonCanonicalManifest
        );

        let header_len = u32::from_be_bytes(object_bytes[8..12].try_into().unwrap()) as usize;
        let payload_len = u64::from_be_bytes(object_bytes[12..20].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&object_bytes[20..20 + header_len]).unwrap();
        let pretty_header = serde_json::to_vec_pretty(&header).unwrap();
        let mut noncanonical_object = Vec::new();
        noncanonical_object.extend_from_slice(OBJECT_MAGIC);
        noncanonical_object.extend_from_slice(&(pretty_header.len() as u32).to_be_bytes());
        noncanonical_object.extend_from_slice(&(payload_len as u64).to_be_bytes());
        noncanonical_object.extend_from_slice(&pretty_header);
        noncanonical_object
            .extend_from_slice(&object_bytes[20 + header_len..20 + header_len + payload_len]);
        noncanonical_object.extend_from_slice(&Sha256::digest(&noncanonical_object));
        assert_eq!(
            SyntheticObject::decode(&noncanonical_object).unwrap_err(),
            BatchError::NonCanonicalObjectHeader
        );
    }
}
