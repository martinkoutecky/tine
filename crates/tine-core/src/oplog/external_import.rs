use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::receipt::validate_annotations;
use super::{
    AnnotatedIdentity, BatchId, BatchOrigin, DocumentId, ImportId, ManagedPath, ManagedTextKind,
    ObjectKind, OperationBatch, OperationObject, PortablePathIndexRoot, PortablePathKeyDigest,
    ReceiptError, WorkspaceId, PORTABLE_PATH_KEY_VERSION,
};

pub(crate) const EXTERNAL_IMPORT_OBSERVATION_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_EXTERNAL_IMPORT_OBSERVATION_ENTRIES: usize = 100_000;

const OBSERVATION_MAGIC: &[u8; 8] = b"TINEEIO1";
const MAX_BOUNDED_VEC_PREALLOCATION: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExternalImportObservationState {
    Present {
        bytes: Vec<u8>,
        annotations: Vec<AnnotatedIdentity>,
    },
    Absent,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExternalImportObservationStateWire {
    Present {
        #[serde(deserialize_with = "deserialize_observed_bytes")]
        bytes: Vec<u8>,
        #[serde(deserialize_with = "deserialize_annotations")]
        annotations: Vec<AnnotatedIdentity>,
    },
    Absent,
}

impl<'de> Deserialize<'de> for ExternalImportObservationState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(
            match ExternalImportObservationStateWire::deserialize(deserializer)? {
                ExternalImportObservationStateWire::Present { bytes, annotations } => {
                    Self::Present { bytes, annotations }
                }
                ExternalImportObservationStateWire::Absent => Self::Absent,
            },
        )
    }
}

impl ExternalImportObservationState {
    pub(crate) fn present(
        bytes: Vec<u8>,
        annotations: Vec<AnnotatedIdentity>,
    ) -> Result<Self, ExternalImportObservationError> {
        let state = Self::Present { bytes, annotations };
        state.validate()?;
        Ok(state)
    }

    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Present { bytes, .. } => Some(bytes),
            Self::Absent => None,
        }
    }

    pub(crate) fn annotations(&self) -> &[AnnotatedIdentity] {
        match self {
            Self::Present { annotations, .. } => annotations,
            Self::Absent => &[],
        }
    }

    fn validate(&self) -> Result<(), ExternalImportObservationError> {
        if let Self::Present { bytes, annotations } = self {
            validate_annotations(annotations, bytes.len() as u64)
                .map_err(ExternalImportObservationError::InvalidAnnotations)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExternalImportObservationEntry {
    path: ManagedPath,
    kind: ManagedTextKind,
    portable_path_key_digest: PortablePathKeyDigest,
    state: ExternalImportObservationState,
}

impl ExternalImportObservationEntry {
    pub(crate) fn new(
        path: ManagedPath,
        kind: ManagedTextKind,
        state: ExternalImportObservationState,
    ) -> Result<Self, ExternalImportObservationError> {
        let portable_path_key_digest = path.portable_key().digest();
        let entry = Self {
            path,
            kind,
            portable_path_key_digest,
            state,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub(crate) fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub(crate) const fn kind(&self) -> ManagedTextKind {
        self.kind
    }

    pub(crate) const fn portable_path_key_digest(&self) -> PortablePathKeyDigest {
        self.portable_path_key_digest
    }

    pub(crate) const fn state(&self) -> &ExternalImportObservationState {
        &self.state
    }

    fn validate(&self) -> Result<(), ExternalImportObservationError> {
        if self.portable_path_key_digest != self.path.portable_key().digest() {
            return Err(ExternalImportObservationError::PortablePathDigestMismatch(
                self.path.clone(),
            ));
        }
        self.state.validate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ExternalImportObservation {
    schema_version: u32,
    workspace_id: WorkspaceId,
    source_batch_id: BatchId,
    import_id: ImportId,
    portable_path_key_version: u32,
    prospective_portable_path_root: PortablePathIndexRoot,
    entries: Vec<ExternalImportObservationEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalImportObservationWire {
    schema_version: u32,
    workspace_id: WorkspaceId,
    source_batch_id: BatchId,
    import_id: ImportId,
    portable_path_key_version: u32,
    prospective_portable_path_root: PortablePathIndexRoot,
    #[serde(deserialize_with = "deserialize_entries")]
    entries: Vec<ExternalImportObservationEntry>,
}

impl ExternalImportObservation {
    pub(crate) fn new(
        workspace_id: WorkspaceId,
        import_id: ImportId,
        prospective_portable_path_root: PortablePathIndexRoot,
        mut entries: Vec<ExternalImportObservationEntry>,
    ) -> Result<Self, ExternalImportObservationError> {
        entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        let observation = Self {
            schema_version: EXTERNAL_IMPORT_OBSERVATION_SCHEMA_VERSION,
            workspace_id,
            source_batch_id: import_id.batch_id(),
            import_id,
            portable_path_key_version: PORTABLE_PATH_KEY_VERSION,
            prospective_portable_path_root,
            entries,
        };
        observation.validate()?;
        Ok(observation)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, ExternalImportObservationError> {
        self.validate()?;
        let body = postcard::to_allocvec(self)
            .map_err(|error| ExternalImportObservationError::Encode(error.to_string()))?;
        let length = OBSERVATION_MAGIC.len().saturating_add(body.len());
        if length > MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES {
            return Err(ExternalImportObservationError::TooLarge(length));
        }
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(OBSERVATION_MAGIC);
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ExternalImportObservationError> {
        if bytes.len() > MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES {
            return Err(ExternalImportObservationError::TooLarge(bytes.len()));
        }
        let body = bytes
            .strip_prefix(OBSERVATION_MAGIC)
            .ok_or(ExternalImportObservationError::InvalidMagic)?;
        let wire: ExternalImportObservationWire = postcard::from_bytes(body)
            .map_err(|error| ExternalImportObservationError::Decode(error.to_string()))?;
        let observation = Self {
            schema_version: wire.schema_version,
            workspace_id: wire.workspace_id,
            source_batch_id: wire.source_batch_id,
            import_id: wire.import_id,
            portable_path_key_version: wire.portable_path_key_version,
            prospective_portable_path_root: wire.prospective_portable_path_root,
            entries: wire.entries,
        };
        observation.validate()?;
        if observation.encode()?.as_slice() != bytes {
            return Err(ExternalImportObservationError::NonCanonicalEncoding);
        }
        Ok(observation)
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }

    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) const fn portable_path_key_version(&self) -> u32 {
        self.portable_path_key_version
    }

    pub(crate) const fn prospective_portable_path_root(&self) -> PortablePathIndexRoot {
        self.prospective_portable_path_root
    }

    pub(crate) fn entries(&self) -> &[ExternalImportObservationEntry] {
        &self.entries
    }

    pub(crate) fn descriptor_document_id(&self) -> DocumentId {
        DocumentId::for_external_import_observation(self.workspace_id, self.import_id)
    }

    fn validate(&self) -> Result<(), ExternalImportObservationError> {
        if self.schema_version != EXTERNAL_IMPORT_OBSERVATION_SCHEMA_VERSION {
            return Err(ExternalImportObservationError::UnknownSchema(
                self.schema_version,
            ));
        }
        if self.source_batch_id != self.import_id.batch_id() {
            return Err(ExternalImportObservationError::SourceBatchMismatch);
        }
        if self.portable_path_key_version != PORTABLE_PATH_KEY_VERSION {
            return Err(
                ExternalImportObservationError::UnknownPortablePathKeyVersion(
                    self.portable_path_key_version,
                ),
            );
        }
        if self.entries.is_empty() {
            return Err(ExternalImportObservationError::EmptyEntries);
        }
        if self.entries.len() > MAX_EXTERNAL_IMPORT_OBSERVATION_ENTRIES {
            return Err(ExternalImportObservationError::TooManyEntries(
                self.entries.len(),
            ));
        }
        if !self
            .entries
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        {
            if let Some(duplicate) = self
                .entries
                .windows(2)
                .find(|pair| pair[0].path == pair[1].path)
            {
                return Err(ExternalImportObservationError::DuplicateExactPath(
                    duplicate[0].path.clone(),
                ));
            }
            return Err(ExternalImportObservationError::NonCanonicalEntries);
        }

        let mut portable_paths = BTreeMap::new();
        for entry in &self.entries {
            entry.validate()?;
            if let Some(prior) =
                portable_paths.insert(entry.portable_path_key_digest, entry.path.clone())
            {
                return Err(ExternalImportObservationError::PortablePathCollision {
                    first: prior,
                    second: entry.path.clone(),
                });
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_external_import_object_set(
    manifest: &OperationBatch,
    objects: &[OperationObject],
) -> Result<(), ExternalImportObservationError> {
    let observations = objects
        .iter()
        .filter(|object| object.kind() == ObjectKind::ExternalImportObservation)
        .collect::<Vec<_>>();

    let origin_import_id = match manifest.origin() {
        BatchOrigin::ExternalReconciliation { import_id } => import_id,
        origin => {
            if observations.is_empty() {
                return Ok(());
            }
            return Err(ExternalImportObservationError::UnexpectedObservation { origin });
        }
    };

    let object = match observations.as_slice() {
        [] => {
            return Err(ExternalImportObservationError::MissingObservation {
                import_id: origin_import_id,
            });
        }
        [object] => *object,
        [first, ..] => {
            return Err(ExternalImportObservationError::DuplicateObservation(
                first.document_id(),
            ));
        }
    };

    let observation = ExternalImportObservation::decode(object.payload())?;
    if observation.workspace_id() != object.workspace_id()
        || observation.workspace_id() != manifest.workspace_id()
    {
        return Err(ExternalImportObservationError::WorkspaceBindingMismatch);
    }
    if observation.source_batch_id() != manifest.batch_id() {
        return Err(ExternalImportObservationError::BatchBindingMismatch);
    }
    if observation.import_id() != origin_import_id {
        return Err(ExternalImportObservationError::OriginImportMismatch {
            expected: origin_import_id,
            found: observation.import_id(),
        });
    }
    let expected = observation.descriptor_document_id();
    if object.document_id() != expected {
        return Err(ExternalImportObservationError::DescriptorDocumentMismatch {
            expected,
            found: object.document_id(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExternalImportObservationError {
    Decode(String),
    Encode(String),
    InvalidMagic,
    UnknownSchema(u32),
    UnknownPortablePathKeyVersion(u32),
    TooLarge(usize),
    TooManyEntries(usize),
    EmptyEntries,
    SourceBatchMismatch,
    PortablePathDigestMismatch(ManagedPath),
    DuplicateExactPath(ManagedPath),
    PortablePathCollision {
        first: ManagedPath,
        second: ManagedPath,
    },
    NonCanonicalEntries,
    NonCanonicalEncoding,
    InvalidAnnotations(ReceiptError),
    MissingObservation {
        import_id: ImportId,
    },
    UnexpectedObservation {
        origin: BatchOrigin,
    },
    WorkspaceBindingMismatch,
    BatchBindingMismatch,
    OriginImportMismatch {
        expected: ImportId,
        found: ImportId,
    },
    DescriptorDocumentMismatch {
        expected: DocumentId,
        found: DocumentId,
    },
    DuplicateObservation(DocumentId),
}

impl fmt::Display for ExternalImportObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "observation decode failed: {error}"),
            Self::Encode(error) => write!(formatter, "observation encode failed: {error}"),
            Self::InvalidMagic => formatter.write_str("invalid external-import observation magic"),
            Self::UnknownSchema(found) => write!(
                formatter,
                "unknown external-import observation schema {found}; expected \
                 {EXTERNAL_IMPORT_OBSERVATION_SCHEMA_VERSION}"
            ),
            Self::UnknownPortablePathKeyVersion(found) => write!(
                formatter,
                "unknown portable-path key version {found}; expected {PORTABLE_PATH_KEY_VERSION}"
            ),
            Self::TooLarge(length) => write!(
                formatter,
                "external-import observation is too large: {length} bytes exceeds \
                 {MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES}"
            ),
            Self::TooManyEntries(count) => {
                write!(
                    formatter,
                    "external-import observation has too many entries: {count}"
                )
            }
            Self::EmptyEntries => {
                formatter.write_str("external-import observation entries must not be empty")
            }
            Self::SourceBatchMismatch => {
                formatter.write_str("source batch does not match import-derived batch")
            }
            Self::PortablePathDigestMismatch(path) => {
                write!(
                    formatter,
                    "portable-path digest does not match exact path {path}"
                )
            }
            Self::DuplicateExactPath(path) => {
                write!(formatter, "duplicate exact external-import path {path}")
            }
            Self::PortablePathCollision { first, second } => write!(
                formatter,
                "portable-path collision between exact paths {first} and {second}"
            ),
            Self::NonCanonicalEntries => {
                formatter.write_str("external-import entries are not canonically ordered")
            }
            Self::NonCanonicalEncoding => {
                formatter.write_str("external-import observation encoding is not canonical")
            }
            Self::InvalidAnnotations(error) => {
                write!(
                    formatter,
                    "external-import annotations are invalid: {error}"
                )
            }
            Self::MissingObservation { import_id } => write!(
                formatter,
                "external reconciliation for import {import_id} requires exactly one external-import observation"
            ),
            Self::UnexpectedObservation { origin } => write!(
                formatter,
                "batch origin {origin:?} must not contain external-import observations"
            ),
            Self::WorkspaceBindingMismatch => {
                formatter.write_str("external-import workspace binding mismatch")
            }
            Self::BatchBindingMismatch => {
                formatter.write_str("external-import batch binding mismatch")
            }
            Self::OriginImportMismatch { expected, found } => write!(
                formatter,
                "external-import observation import mismatch: expected origin import {expected}, found {found}"
            ),
            Self::DescriptorDocumentMismatch { expected, found } => write!(
                formatter,
                "external-import descriptor document mismatch: expected {expected}, found {found}"
            ),
            Self::DuplicateObservation(document_id) => write!(
                formatter,
                "duplicate external-import observation document {document_id}"
            ),
        }
    }
}

impl std::error::Error for ExternalImportObservationError {}

fn deserialize_entries<'de, D>(
    deserializer: D,
) -> Result<Vec<ExternalImportObservationEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec::<D, ExternalImportObservationEntry>(
        deserializer,
        MAX_EXTERNAL_IMPORT_OBSERVATION_ENTRIES,
        "external-import entries",
    )
}

fn deserialize_annotations<'de, D>(deserializer: D) -> Result<Vec<AnnotatedIdentity>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec::<D, AnnotatedIdentity>(
        deserializer,
        MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES,
        "external-import annotations",
    )
}

fn deserialize_observed_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec::<D, u8>(
        deserializer,
        MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES,
        "external-import bytes",
    )
}

fn deserialize_bounded_vec<'de, D, T>(
    deserializer: D,
    limit: usize,
    kind: &'static str,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct BoundedVecVisitor<T> {
        marker: PhantomData<T>,
        limit: usize,
        kind: &'static str,
    }

    impl<'de, T> Visitor<'de> for BoundedVecVisitor<T>
    where
        T: Deserialize<'de>,
    {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{} with at most {} items", self.kind, self.limit)
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let hinted = sequence.size_hint().unwrap_or(0);
            if hinted > self.limit {
                return Err(A::Error::custom(format_args!(
                    "{} length {hinted} exceeds {}",
                    self.kind, self.limit
                )));
            }
            // Sequence length hints are untrusted. The input-size and item
            // limits bound eventual growth; this cap prevents a tiny,
            // truncated payload from reserving a large typed vector up front.
            let mut values = Vec::with_capacity(hinted.min(MAX_BOUNDED_VEC_PREALLOCATION));
            while let Some(value) = sequence.next_element()? {
                if values.len() == self.limit {
                    return Err(A::Error::custom(format_args!(
                        "{} exceeds {} items",
                        self.kind, self.limit
                    )));
                }
                values.push(value);
            }
            Ok(values)
        }
    }

    deserializer.deserialize_seq(BoundedVecVisitor {
        marker: PhantomData,
        limit,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        BatchCausalDot, BatchError, BatchOrigin, BlockId, CausalPeerId, DeviceId, FrontierV2,
        LineageDigest, LogseqUuid, SemanticEffectDigest, SessionId, StructuralLocator,
        StructuralSpan,
    };

    #[derive(Clone, Serialize)]
    struct RawObservation {
        schema_version: u32,
        workspace_id: WorkspaceId,
        source_batch_id: BatchId,
        import_id: ImportId,
        portable_path_key_version: u32,
        prospective_portable_path_root: PortablePathIndexRoot,
        entries: Vec<ExternalImportObservationEntry>,
    }

    #[derive(Serialize)]
    struct RawObservationWithStringRoot {
        schema_version: u32,
        workspace_id: WorkspaceId,
        source_batch_id: BatchId,
        import_id: ImportId,
        portable_path_key_version: u32,
        prospective_portable_path_root: String,
        entries: Vec<ExternalImportObservationEntry>,
    }

    #[derive(Serialize)]
    struct RawObservationWithUnitEntries {
        schema_version: u32,
        workspace_id: WorkspaceId,
        source_batch_id: BatchId,
        import_id: ImportId,
        portable_path_key_version: u32,
        prospective_portable_path_root: PortablePathIndexRoot,
        entries: Vec<()>,
    }

    #[allow(dead_code)]
    #[derive(Deserialize)]
    struct BoundedAnnotationsProbe(
        #[serde(deserialize_with = "deserialize_annotations")] Vec<AnnotatedIdentity>,
    );

    fn workspace(value: u128) -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(value))
    }

    fn document(value: u128) -> DocumentId {
        DocumentId::from_uuid(Uuid::from_u128(value))
    }

    fn import(value: u8) -> ImportId {
        ImportId::from_digest([value; 32])
    }

    fn absent(path: &str, kind: ManagedTextKind) -> ExternalImportObservationEntry {
        ExternalImportObservationEntry::new(
            ManagedPath::parse(path).unwrap(),
            kind,
            ExternalImportObservationState::Absent,
        )
        .unwrap()
    }

    fn annotation(
        locator: &[u32],
        start: u64,
        end: u64,
        block: u128,
        logseq: Option<u128>,
    ) -> AnnotatedIdentity {
        AnnotatedIdentity::new(
            StructuralLocator::new(locator.to_vec()).unwrap(),
            StructuralSpan::new(start, end).unwrap(),
            BlockId::from_uuid(Uuid::from_u128(block)),
            logseq.map(|value| LogseqUuid::from_uuid(Uuid::from_u128(value))),
        )
    }

    fn observation_with(entries: Vec<ExternalImportObservationEntry>) -> ExternalImportObservation {
        ExternalImportObservation::new(
            workspace(1),
            import(0x11),
            PortablePathIndexRoot::empty(),
            entries,
        )
        .unwrap()
    }

    fn raw(observation: &ExternalImportObservation) -> RawObservation {
        RawObservation {
            schema_version: observation.schema_version,
            workspace_id: observation.workspace_id,
            source_batch_id: observation.source_batch_id,
            import_id: observation.import_id,
            portable_path_key_version: observation.portable_path_key_version,
            prospective_portable_path_root: observation.prospective_portable_path_root,
            entries: observation.entries.clone(),
        }
    }

    fn encode_raw(raw: &impl Serialize) -> Vec<u8> {
        let mut bytes = OBSERVATION_MAGIC.to_vec();
        bytes.extend_from_slice(&postcard::to_allocvec(raw).unwrap());
        bytes
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn prepared_with_observation(
        observation: &ExternalImportObservation,
        object_workspace: WorkspaceId,
        object_document: DocumentId,
        manifest_workspace: WorkspaceId,
        manifest_batch: BatchId,
    ) -> Result<super::super::PreparedBatch, BatchError> {
        let semantic_payload = b"semantic";
        let semantic = OperationObject::new(
            manifest_workspace,
            document(7),
            ObjectKind::SemanticEffect,
            semantic_payload.to_vec(),
        )
        .unwrap();
        let observed = OperationObject::new(
            object_workspace,
            object_document,
            ObjectKind::ExternalImportObservation,
            observation.encode().unwrap(),
        )
        .unwrap();
        let objects = vec![semantic, observed];
        let descriptors = objects
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let device = DeviceId::from_uuid(Uuid::from_u128(8));
        let manifest = OperationBatch::new_with_causality(
            manifest_workspace,
            LineageDigest::of(b"lineage"),
            manifest_batch,
            device,
            SessionId::from_uuid(Uuid::from_u128(9)),
            BatchOrigin::ExternalReconciliation {
                import_id: observation.import_id,
            },
            BatchCausalDot::new(CausalPeerId::from_device_id(device), 1).unwrap(),
            Vec::new(),
            FrontierV2::default(),
            SemanticEffectDigest::of(semantic_payload),
            descriptors,
        )
        .unwrap();
        super::super::PreparedBatch::new(manifest, objects)
    }

    fn observation_object(observation: &ExternalImportObservation) -> OperationObject {
        OperationObject::new(
            observation.workspace_id(),
            observation.descriptor_document_id(),
            ObjectKind::ExternalImportObservation,
            observation.encode().unwrap(),
        )
        .unwrap()
    }

    fn prepared_for_origin(
        origin: BatchOrigin,
        manifest_batch: BatchId,
        mut observations: Vec<OperationObject>,
    ) -> Result<super::super::PreparedBatch, BatchError> {
        let workspace_id = workspace(1);
        let semantic_payload = b"semantic";
        let semantic = OperationObject::new(
            workspace_id,
            document(7),
            ObjectKind::SemanticEffect,
            semantic_payload.to_vec(),
        )
        .unwrap();
        observations.insert(0, semantic);
        let descriptors = observations
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let device = DeviceId::from_uuid(Uuid::from_u128(8));
        let manifest = OperationBatch::new_with_causality(
            workspace_id,
            LineageDigest::of(b"lineage"),
            manifest_batch,
            device,
            SessionId::from_uuid(Uuid::from_u128(9)),
            origin,
            BatchCausalDot::new(CausalPeerId::from_device_id(device), 1).unwrap(),
            Vec::new(),
            FrontierV2::default(),
            SemanticEffectDigest::of(semantic_payload),
            descriptors,
        )
        .unwrap();
        super::super::PreparedBatch::new(manifest, observations)
    }

    #[test]
    fn canonical_payload_and_envelope_have_frozen_golden_bytes() {
        let observation = observation_with(vec![absent("pages/a.md", ManagedTextKind::Page)]);
        let payload = observation.encode().unwrap();
        assert_eq!(
            hex(&payload),
            "54494e4545494f31011000000000000000000000000000000001103f7d7a8e2e708edd93045a6e11671c7f4031313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131014066333130653831633365303235643132643836326333323764366165646666643731353636346231306661643336656363613163613733666238306534316633010a70616765732f612e6d640075aecadc1aff7d757f2ca9b5e5963c473e640702365f473e50e13d5043fe052f01"
        );
        assert_eq!(
            ExternalImportObservation::decode(&payload).unwrap(),
            observation
        );

        let object = OperationObject::new(
            observation.workspace_id(),
            observation.descriptor_document_id(),
            ObjectKind::ExternalImportObservation,
            payload,
        )
        .unwrap();
        let encoded = object.encode().unwrap();
        assert_eq!(
            hex(&encoded),
            "54494e454f424a32000000c100000000000000dc7b22656e76656c6f70655f736368656d615f76657273696f6e223a322c22776f726b73706163655f6964223a2230303030303030302d303030302d303030302d303030302d303030303030303030303031222c22646f63756d656e745f6964223a2263663764373534352d386566622d383265312d613466622d626234386635306631633965222c226b696e64223a2265787465726e616c5f696d706f72745f6f62736572766174696f6e222c22656e6372797074696f6e223a226e6f6e65227d54494e4545494f31011000000000000000000000000000000001103f7d7a8e2e708edd93045a6e11671c7f4031313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131014066333130653831633365303235643132643836326333323764366165646666643731353636346231306661643336656363613163613733666238306534316633010a70616765732f612e6d640075aecadc1aff7d757f2ca9b5e5963c473e640702365f473e50e13d5043fe052f0159c0f5d0c05e8278f780ab280d825c65d721a57f437e5d688652781cc2299baa"
        );
        assert_eq!(OperationObject::decode(&encoded).unwrap(), object);
    }

    #[test]
    fn exact_opaque_bytes_absence_and_explicit_kind_roundtrip() {
        let exact = b"# noncanonical\r\n\xff\xfe\r\ntrailing  \r\n\0".to_vec();
        let present = ExternalImportObservationState::present(exact.clone(), Vec::new()).unwrap();
        let observation = observation_with(vec![
            ExternalImportObservationEntry::new(
                ManagedPath::parse("journals/not-inferred.md").unwrap(),
                ManagedTextKind::Page,
                present,
            )
            .unwrap(),
            absent("pages/missing.org", ManagedTextKind::Journal),
        ]);
        let decoded = ExternalImportObservation::decode(&observation.encode().unwrap()).unwrap();
        assert_eq!(decoded.entries().len(), 2);
        let present = &decoded.entries()[0];
        assert_eq!(present.kind(), ManagedTextKind::Page);
        assert_eq!(present.state().bytes(), Some(exact.as_slice()));
        assert!(present.state().annotations().is_empty());
        let absent = &decoded.entries()[1];
        assert_eq!(absent.kind(), ManagedTextKind::Journal);
        assert_eq!(absent.state().bytes(), None);
        assert!(absent.state().annotations().is_empty());
    }

    #[test]
    fn annotation_bounds_order_and_identity_uniqueness_are_enforced() {
        let bytes = b"abcdef".to_vec();
        let cases = [
            vec![
                annotation(&[1], 0, 1, 1, None),
                annotation(&[0], 1, 2, 2, None),
            ],
            vec![
                annotation(&[0], 0, 1, 1, None),
                annotation(&[0], 1, 2, 2, None),
            ],
            vec![
                annotation(&[0], 0, 1, 1, None),
                annotation(&[1], 1, 2, 1, None),
            ],
            vec![
                annotation(&[0], 0, 1, 1, Some(10)),
                annotation(&[1], 1, 2, 2, Some(10)),
            ],
            vec![annotation(&[0], 0, 7, 1, None)],
        ];
        for annotations in cases {
            assert!(matches!(
                ExternalImportObservationState::present(bytes.clone(), annotations),
                Err(ExternalImportObservationError::InvalidAnnotations(_))
            ));
        }

        let canonical = vec![
            annotation(&[0], 0, 1, 1, Some(10)),
            annotation(&[1], 1, 6, 2, Some(11)),
        ];
        assert!(
            ExternalImportObservationState::present(bytes, canonical).is_ok(),
            "canonical annotations must remain accepted"
        );
    }

    #[test]
    fn decode_rejects_magic_schema_trailing_noncanonical_and_oversized_payloads() {
        let observation = observation_with(vec![absent("pages/a.md", ManagedTextKind::Page)]);
        let canonical = observation.encode().unwrap();

        let mut wrong_magic = canonical.clone();
        wrong_magic[0] ^= 1;
        assert_eq!(
            ExternalImportObservation::decode(&wrong_magic).unwrap_err(),
            ExternalImportObservationError::InvalidMagic
        );

        let mut future = raw(&observation);
        future.schema_version = 2;
        assert_eq!(
            ExternalImportObservation::decode(&encode_raw(&future)).unwrap_err(),
            ExternalImportObservationError::UnknownSchema(2)
        );

        let mut trailing = canonical;
        trailing.push(0);
        assert!(matches!(
            ExternalImportObservation::decode(&trailing),
            Err(ExternalImportObservationError::Decode(_)
                | ExternalImportObservationError::NonCanonicalEncoding)
        ));

        let oversized = vec![0; MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES + 1];
        assert_eq!(
            ExternalImportObservation::decode(&oversized).unwrap_err(),
            ExternalImportObservationError::TooLarge(oversized.len())
        );
    }

    #[test]
    fn truncated_huge_annotation_length_cannot_drive_eager_typed_allocation() {
        let declared =
            postcard::to_allocvec(&(MAX_EXTERNAL_IMPORT_OBSERVATION_BYTES as u64)).unwrap();
        assert!(postcard::from_bytes::<BoundedAnnotationsProbe>(&declared).is_err());
    }

    #[test]
    fn payload_bindings_versions_digests_and_root_representation_fail_closed() {
        let observation = observation_with(vec![absent("pages/a.md", ManagedTextKind::Page)]);

        let mut wrong_batch = raw(&observation);
        wrong_batch.source_batch_id = BatchId::from_uuid(Uuid::from_u128(999));
        assert_eq!(
            ExternalImportObservation::decode(&encode_raw(&wrong_batch)).unwrap_err(),
            ExternalImportObservationError::SourceBatchMismatch
        );

        let mut wrong_version = raw(&observation);
        wrong_version.portable_path_key_version = PORTABLE_PATH_KEY_VERSION + 1;
        assert_eq!(
            ExternalImportObservation::decode(&encode_raw(&wrong_version)).unwrap_err(),
            ExternalImportObservationError::UnknownPortablePathKeyVersion(
                PORTABLE_PATH_KEY_VERSION + 1
            )
        );

        let mut wrong_digest = raw(&observation);
        wrong_digest.entries[0].portable_path_key_digest = ManagedPath::parse("pages/other.md")
            .unwrap()
            .portable_key()
            .digest();
        assert!(matches!(
            ExternalImportObservation::decode(&encode_raw(&wrong_digest)),
            Err(ExternalImportObservationError::PortablePathDigestMismatch(
                _
            ))
        ));

        let bad_root = RawObservationWithStringRoot {
            schema_version: observation.schema_version,
            workspace_id: observation.workspace_id,
            source_batch_id: observation.source_batch_id,
            import_id: observation.import_id,
            portable_path_key_version: observation.portable_path_key_version,
            prospective_portable_path_root: "not-a-root-digest".to_owned(),
            entries: observation.entries.clone(),
        };
        assert!(matches!(
            ExternalImportObservation::decode(&encode_raw(&bad_root)),
            Err(ExternalImportObservationError::Decode(_))
        ));
    }

    #[test]
    fn prepared_batch_enforces_workspace_batch_and_descriptor_bindings() {
        let observation = observation_with(vec![absent("pages/a.md", ManagedTextKind::Page)]);
        let workspace_id = observation.workspace_id();
        let document_id = observation.descriptor_document_id();
        let prepared = prepared_with_observation(
            &observation,
            workspace_id,
            document_id,
            workspace_id,
            observation.source_batch_id(),
        )
        .unwrap();

        let wrong_payload_workspace = ExternalImportObservation::new(
            workspace(2),
            observation.import_id(),
            observation.prospective_portable_path_root(),
            observation.entries().to_vec(),
        )
        .unwrap();
        assert!(matches!(
            prepared_with_observation(
                &wrong_payload_workspace,
                workspace_id,
                wrong_payload_workspace.descriptor_document_id(),
                workspace_id,
                wrong_payload_workspace.source_batch_id(),
            ),
            Err(BatchError::ExternalImportObject(_))
        ));

        assert!(matches!(
            prepared_with_observation(
                &observation,
                workspace_id,
                document(999),
                workspace_id,
                observation.source_batch_id(),
            ),
            Err(BatchError::ExternalImportObject(_))
        ));
        assert!(matches!(
            prepared_with_observation(
                &observation,
                workspace_id,
                document_id,
                workspace_id,
                BatchId::from_uuid(Uuid::from_u128(998)),
            ),
            Err(BatchError::ExternalImportObject(_))
        ));

        let second = ExternalImportObservation::new(
            workspace_id,
            observation.import_id(),
            observation.prospective_portable_path_root(),
            vec![absent("pages/b.md", ManagedTextKind::Page)],
        )
        .unwrap();
        let duplicate_objects = [&observation, &second]
            .into_iter()
            .map(|observation| {
                OperationObject::new(
                    workspace_id,
                    document_id,
                    ObjectKind::ExternalImportObservation,
                    observation.encode().unwrap(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            validate_external_import_object_set(prepared.manifest(), &duplicate_objects)
                .unwrap_err(),
            ExternalImportObservationError::DuplicateObservation(document_id)
        );
    }

    #[test]
    fn prepared_batch_requires_external_observation_cardinality_and_origin_import() {
        let observation = observation_with(vec![absent("pages/a.md", ManagedTextKind::Page)]);
        let matching_origin = BatchOrigin::ExternalReconciliation {
            import_id: observation.import_id(),
        };

        assert!(prepared_for_origin(
            matching_origin,
            observation.source_batch_id(),
            vec![observation_object(&observation)],
        )
        .is_ok());

        assert_eq!(
            prepared_for_origin(matching_origin, observation.source_batch_id(), Vec::new(),)
                .unwrap_err(),
            BatchError::ExternalImportObject(
                ExternalImportObservationError::MissingObservation {
                    import_id: observation.import_id(),
                }
                .to_string()
            )
        );

        let second = ExternalImportObservation::new(
            observation.workspace_id(),
            observation.import_id(),
            observation.prospective_portable_path_root(),
            vec![absent("pages/b.md", ManagedTextKind::Page)],
        )
        .unwrap();
        assert_eq!(
            prepared_for_origin(
                matching_origin,
                observation.source_batch_id(),
                vec![
                    observation_object(&observation),
                    observation_object(&second)
                ],
            )
            .unwrap_err(),
            BatchError::ExternalImportObject(
                ExternalImportObservationError::DuplicateObservation(
                    observation.descriptor_document_id(),
                )
                .to_string()
            )
        );

        for origin in [BatchOrigin::LocalMutation, BatchOrigin::BootstrapImport] {
            assert_eq!(
                prepared_for_origin(
                    origin,
                    observation.source_batch_id(),
                    vec![observation_object(&observation)],
                )
                .unwrap_err(),
                BatchError::ExternalImportObject(
                    ExternalImportObservationError::UnexpectedObservation { origin }.to_string()
                )
            );
        }

        let mismatched_origin = BatchOrigin::ExternalReconciliation {
            import_id: import(0x22),
        };
        assert_eq!(
            prepared_for_origin(
                mismatched_origin,
                observation.source_batch_id(),
                vec![observation_object(&observation)],
            )
            .unwrap_err(),
            BatchError::ExternalImportObject(
                ExternalImportObservationError::OriginImportMismatch {
                    expected: import(0x22),
                    found: observation.import_id(),
                }
                .to_string()
            )
        );
    }

    #[test]
    fn entry_order_duplicates_and_portable_collisions_are_rejected() {
        assert_eq!(
            ExternalImportObservation::new(
                workspace(1),
                import(1),
                PortablePathIndexRoot::empty(),
                Vec::new(),
            )
            .unwrap_err(),
            ExternalImportObservationError::EmptyEntries
        );

        let oversized_count = RawObservationWithUnitEntries {
            schema_version: EXTERNAL_IMPORT_OBSERVATION_SCHEMA_VERSION,
            workspace_id: workspace(1),
            source_batch_id: import(1).batch_id(),
            import_id: import(1),
            portable_path_key_version: PORTABLE_PATH_KEY_VERSION,
            prospective_portable_path_root: PortablePathIndexRoot::empty(),
            entries: vec![(); MAX_EXTERNAL_IMPORT_OBSERVATION_ENTRIES + 1],
        };
        assert!(matches!(
            ExternalImportObservation::decode(&encode_raw(&oversized_count)),
            Err(ExternalImportObservationError::Decode(_))
        ));

        let first = absent("pages/a.md", ManagedTextKind::Page);
        let second = absent("pages/b.md", ManagedTextKind::Page);
        let canonical = observation_with(vec![first.clone(), second.clone()]);
        let mut reordered = raw(&canonical);
        reordered.entries.reverse();
        assert_eq!(
            ExternalImportObservation::decode(&encode_raw(&reordered)).unwrap_err(),
            ExternalImportObservationError::NonCanonicalEntries
        );

        assert!(matches!(
            ExternalImportObservation::new(
                workspace(1),
                import(1),
                PortablePathIndexRoot::empty(),
                vec![first.clone(), first],
            ),
            Err(ExternalImportObservationError::DuplicateExactPath(_))
        ));

        for paths in [
            ("pages/Name.md", "pages/name.md"),
            ("pages/Café.md", "pages/CAFE\u{301}.md"),
        ] {
            assert!(matches!(
                ExternalImportObservation::new(
                    workspace(1),
                    import(1),
                    PortablePathIndexRoot::empty(),
                    vec![
                        absent(paths.0, ManagedTextKind::Page),
                        absent(paths.1, ManagedTextKind::Page),
                    ],
                ),
                Err(ExternalImportObservationError::PortablePathCollision { .. })
            ));
        }
    }
}
