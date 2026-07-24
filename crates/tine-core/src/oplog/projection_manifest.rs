use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    AnnotatedIdentity, BatchId, BlobDescription, ContentDigest, DeviceId, DocumentId, FrontierV2,
    LogicalCompletionId, ManagedPath, ObjectDescriptor, ObjectKind, OperationBatch,
    OperationObject, PageId, PortablePathIndexRoot, PortablePathKeyDigest, ProjectionClaimEvidence,
    ProjectionEndpointId, SessionId, WorkspaceId, PORTABLE_PATH_KEY_VERSION,
};

pub const MANIFESTED_PROJECTION_SCHEMA_VERSION: u32 = 2;
pub const ANNOTATED_BASE_SCHEMA_VERSION: u32 = 1;
pub const MAX_MANIFESTED_PROJECTION_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ANNOTATED_BASE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PROJECTION_ANNOTATIONS: usize = 100_000;
pub const MAX_PROJECTION_CLAIM_EVIDENCE: usize = 100_000;

const INTENT_MAGIC: &[u8; 8] = b"TINEPRI2";
const BASE_MAGIC: &[u8; 8] = b"TINEPRB1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestObjectRef {
    document_id: DocumentId,
    content_digest: ContentDigest,
    encoded_byte_length: u64,
}

impl ManifestObjectRef {
    pub fn from_descriptor(descriptor: &ObjectDescriptor) -> Self {
        Self {
            document_id: descriptor.document_id(),
            content_digest: descriptor.content_digest(),
            encoded_byte_length: descriptor.encoded_byte_length(),
        }
    }

    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub const fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }

    pub const fn encoded_byte_length(&self) -> u64 {
        self.encoded_byte_length
    }

    fn matches(&self, descriptor: &ObjectDescriptor) -> bool {
        self.document_id == descriptor.document_id()
            && self.content_digest == descriptor.content_digest()
            && self.encoded_byte_length == descriptor.encoded_byte_length()
            && descriptor.kind() == ObjectKind::AnnotatedBaseBlob
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestProjectionPrecondition {
    Absent,
    Present { base: ManifestObjectRef },
}

impl ManifestProjectionPrecondition {
    pub const fn base(&self) -> Option<&ManifestObjectRef> {
        match self {
            Self::Absent => None,
            Self::Present { base } => Some(base),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestProjectionTarget {
    Absent,
    Present {
        description: BlobDescription,
        bytes: Vec<u8>,
        annotations: Vec<AnnotatedIdentity>,
    },
}

impl ManifestProjectionTarget {
    pub fn present(
        bytes: Vec<u8>,
        annotations: Vec<AnnotatedIdentity>,
    ) -> Result<Self, ProjectionManifestError> {
        let target = Self::Present {
            description: BlobDescription::of(&bytes),
            bytes,
            annotations,
        };
        target.validate()?;
        Ok(target)
    }

    pub const fn description(&self) -> Option<BlobDescription> {
        match self {
            Self::Absent => None,
            Self::Present { description, .. } => Some(*description),
        }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Absent => None,
            Self::Present { bytes, .. } => Some(bytes),
        }
    }

    pub fn annotations(&self) -> &[AnnotatedIdentity] {
        match self {
            Self::Absent => &[],
            Self::Present { annotations, .. } => annotations,
        }
    }

    fn validate(&self) -> Result<(), ProjectionManifestError> {
        if let Self::Present {
            description,
            bytes,
            annotations,
        } = self
        {
            validate_blob(*description, bytes, "projection target")?;
            validate_annotations(annotations, bytes.len() as u64)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManifestedProjectionIntent {
    schema_version: u32,
    workspace_id: WorkspaceId,
    source_batch_id: BatchId,
    source_author_device_id: DeviceId,
    source_author_session_id: SessionId,
    source_endpoint_id: ProjectionEndpointId,
    page_id: PageId,
    path: ManagedPath,
    portable_path_key_version: u32,
    portable_path_key_digest: PortablePathKeyDigest,
    portable_path_index_root: PortablePathIndexRoot,
    precondition: ManifestProjectionPrecondition,
    render_base: Option<ManifestObjectRef>,
    target: ManifestProjectionTarget,
    post_frontier: FrontierV2,
    claim_evidence: Vec<ProjectionClaimEvidence>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestedProjectionIntentWire {
    schema_version: u32,
    workspace_id: WorkspaceId,
    source_batch_id: BatchId,
    source_author_device_id: DeviceId,
    source_author_session_id: SessionId,
    source_endpoint_id: ProjectionEndpointId,
    page_id: PageId,
    path: ManagedPath,
    portable_path_key_version: u32,
    portable_path_key_digest: PortablePathKeyDigest,
    portable_path_index_root: PortablePathIndexRoot,
    precondition: ManifestProjectionPrecondition,
    render_base: Option<ManifestObjectRef>,
    target: ManifestProjectionTarget,
    post_frontier: FrontierV2,
    claim_evidence: Vec<ProjectionClaimEvidence>,
}

impl ManifestedProjectionIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_id: WorkspaceId,
        source_batch_id: BatchId,
        source_author_device_id: DeviceId,
        source_author_session_id: SessionId,
        source_endpoint_id: ProjectionEndpointId,
        page_id: PageId,
        path: ManagedPath,
        portable_path_index_root: PortablePathIndexRoot,
        precondition: ManifestProjectionPrecondition,
        render_base: Option<ManifestObjectRef>,
        target: ManifestProjectionTarget,
        post_frontier: FrontierV2,
        mut claim_evidence: Vec<ProjectionClaimEvidence>,
    ) -> Result<Self, ProjectionManifestError> {
        claim_evidence.sort_unstable_by_key(ProjectionClaimEvidence::logseq_uuid);
        let portable_path_key_digest = path.portable_key().digest();
        let intent = Self {
            schema_version: MANIFESTED_PROJECTION_SCHEMA_VERSION,
            workspace_id,
            source_batch_id,
            source_author_device_id,
            source_author_session_id,
            source_endpoint_id,
            page_id,
            path,
            portable_path_key_version: PORTABLE_PATH_KEY_VERSION,
            portable_path_key_digest,
            portable_path_index_root,
            precondition,
            render_base,
            target,
            post_frontier,
            claim_evidence,
        };
        intent.validate()?;
        Ok(intent)
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProjectionManifestError> {
        self.validate()?;
        encode_canonical(INTENT_MAGIC, self, MAX_MANIFESTED_PROJECTION_BYTES)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProjectionManifestError> {
        if bytes.len() > MAX_MANIFESTED_PROJECTION_BYTES {
            return Err(ProjectionManifestError::TooLarge {
                kind: "projection intent",
                length: bytes.len(),
                limit: MAX_MANIFESTED_PROJECTION_BYTES,
            });
        }
        let body = bytes
            .strip_prefix(INTENT_MAGIC)
            .ok_or(ProjectionManifestError::InvalidMagic("projection intent"))?;
        let wire: ManifestedProjectionIntentWire = postcard::from_bytes(body)
            .map_err(|error| ProjectionManifestError::Decode(error.to_string()))?;
        let intent = Self {
            schema_version: wire.schema_version,
            workspace_id: wire.workspace_id,
            source_batch_id: wire.source_batch_id,
            source_author_device_id: wire.source_author_device_id,
            source_author_session_id: wire.source_author_session_id,
            source_endpoint_id: wire.source_endpoint_id,
            page_id: wire.page_id,
            path: wire.path,
            portable_path_key_version: wire.portable_path_key_version,
            portable_path_key_digest: wire.portable_path_key_digest,
            portable_path_index_root: wire.portable_path_index_root,
            precondition: wire.precondition,
            render_base: wire.render_base,
            target: wire.target,
            post_frontier: wire.post_frontier,
            claim_evidence: wire.claim_evidence,
        };
        intent.validate()?;
        if intent.encode()?.as_slice() != bytes {
            return Err(ProjectionManifestError::NonCanonical("projection intent"));
        }
        Ok(intent)
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }

    pub const fn source_author_device_id(&self) -> DeviceId {
        self.source_author_device_id
    }

    pub const fn source_author_session_id(&self) -> SessionId {
        self.source_author_session_id
    }

    pub const fn source_endpoint_id(&self) -> ProjectionEndpointId {
        self.source_endpoint_id
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub const fn portable_path_key_version(&self) -> u32 {
        self.portable_path_key_version
    }

    pub const fn portable_path_key_digest(&self) -> PortablePathKeyDigest {
        self.portable_path_key_digest
    }

    pub const fn portable_path_index_root(&self) -> PortablePathIndexRoot {
        self.portable_path_index_root
    }

    pub const fn precondition(&self) -> &ManifestProjectionPrecondition {
        &self.precondition
    }

    pub const fn render_base(&self) -> Option<&ManifestObjectRef> {
        self.render_base.as_ref()
    }

    pub const fn target(&self) -> &ManifestProjectionTarget {
        &self.target
    }

    pub const fn post_frontier(&self) -> &FrontierV2 {
        &self.post_frontier
    }

    pub fn claim_evidence(&self) -> &[ProjectionClaimEvidence] {
        &self.claim_evidence
    }

    pub fn descriptor_document_id(&self) -> DocumentId {
        projection_intent_document_id(
            self.source_batch_id,
            self.source_endpoint_id,
            self.page_id,
            &self.path,
        )
    }

    fn validate(&self) -> Result<(), ProjectionManifestError> {
        if self.schema_version != MANIFESTED_PROJECTION_SCHEMA_VERSION {
            return Err(ProjectionManifestError::UnknownVersion {
                kind: "projection intent",
                expected: MANIFESTED_PROJECTION_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if self.portable_path_key_version != PORTABLE_PATH_KEY_VERSION
            || self.portable_path_key_digest != self.path.portable_key().digest()
        {
            return Err(ProjectionManifestError::InvalidBinding(
                "projection intent portable-path key binding mismatch",
            ));
        }
        self.target.validate()?;
        validate_claim_evidence(&self.claim_evidence)?;
        if self.render_base.is_some() && matches!(self.target, ManifestProjectionTarget::Absent) {
            return Err(ProjectionManifestError::InvalidBinding(
                "Absent target cannot carry a render base",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AnnotatedProjectionBase {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    source_page_id: PageId,
    source_path: ManagedPath,
    prior_completion_id: Option<LogicalCompletionId>,
    prior_frontier: FrontierV2,
    description: BlobDescription,
    bytes: Vec<u8>,
    annotations: Vec<AnnotatedIdentity>,
    claim_evidence: Vec<ProjectionClaimEvidence>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnnotatedProjectionBaseWire {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    source_page_id: PageId,
    source_path: ManagedPath,
    prior_completion_id: Option<LogicalCompletionId>,
    prior_frontier: FrontierV2,
    description: BlobDescription,
    bytes: Vec<u8>,
    annotations: Vec<AnnotatedIdentity>,
    claim_evidence: Vec<ProjectionClaimEvidence>,
}

impl AnnotatedProjectionBase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        source_page_id: PageId,
        source_path: ManagedPath,
        prior_completion_id: Option<LogicalCompletionId>,
        prior_frontier: FrontierV2,
        bytes: Vec<u8>,
        annotations: Vec<AnnotatedIdentity>,
        mut claim_evidence: Vec<ProjectionClaimEvidence>,
    ) -> Result<Self, ProjectionManifestError> {
        claim_evidence.sort_unstable_by_key(ProjectionClaimEvidence::logseq_uuid);
        let base = Self {
            schema_version: ANNOTATED_BASE_SCHEMA_VERSION,
            workspace_id,
            endpoint_id,
            source_page_id,
            source_path,
            prior_completion_id,
            prior_frontier,
            description: BlobDescription::of(&bytes),
            bytes,
            annotations,
            claim_evidence,
        };
        base.validate()?;
        Ok(base)
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProjectionManifestError> {
        self.validate()?;
        encode_canonical(BASE_MAGIC, self, MAX_ANNOTATED_BASE_BYTES)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProjectionManifestError> {
        if bytes.len() > MAX_ANNOTATED_BASE_BYTES {
            return Err(ProjectionManifestError::TooLarge {
                kind: "annotated base",
                length: bytes.len(),
                limit: MAX_ANNOTATED_BASE_BYTES,
            });
        }
        let body = bytes
            .strip_prefix(BASE_MAGIC)
            .ok_or(ProjectionManifestError::InvalidMagic("annotated base"))?;
        let wire: AnnotatedProjectionBaseWire = postcard::from_bytes(body)
            .map_err(|error| ProjectionManifestError::Decode(error.to_string()))?;
        let base = Self {
            schema_version: wire.schema_version,
            workspace_id: wire.workspace_id,
            endpoint_id: wire.endpoint_id,
            source_page_id: wire.source_page_id,
            source_path: wire.source_path,
            prior_completion_id: wire.prior_completion_id,
            prior_frontier: wire.prior_frontier,
            description: wire.description,
            bytes: wire.bytes,
            annotations: wire.annotations,
            claim_evidence: wire.claim_evidence,
        };
        base.validate()?;
        if base.encode()?.as_slice() != bytes {
            return Err(ProjectionManifestError::NonCanonical("annotated base"));
        }
        Ok(base)
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn endpoint_id(&self) -> ProjectionEndpointId {
        self.endpoint_id
    }

    pub const fn source_page_id(&self) -> PageId {
        self.source_page_id
    }

    pub fn source_path(&self) -> &ManagedPath {
        &self.source_path
    }

    pub const fn prior_completion_id(&self) -> Option<LogicalCompletionId> {
        self.prior_completion_id
    }

    pub const fn prior_frontier(&self) -> &FrontierV2 {
        &self.prior_frontier
    }

    pub const fn description(&self) -> BlobDescription {
        self.description
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn annotations(&self) -> &[AnnotatedIdentity] {
        &self.annotations
    }

    pub fn claim_evidence(&self) -> &[ProjectionClaimEvidence] {
        &self.claim_evidence
    }

    pub fn descriptor_document_id(&self) -> Result<DocumentId, ProjectionManifestError> {
        Ok(annotated_base_document_id(&self.encode()?))
    }

    fn validate(&self) -> Result<(), ProjectionManifestError> {
        if self.schema_version != ANNOTATED_BASE_SCHEMA_VERSION {
            return Err(ProjectionManifestError::UnknownVersion {
                kind: "annotated base",
                expected: ANNOTATED_BASE_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        validate_blob(self.description, &self.bytes, "annotated base")?;
        validate_annotations(&self.annotations, self.bytes.len() as u64)?;
        validate_claim_evidence(&self.claim_evidence)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedProjectionObjects {
    intents: Vec<ManifestedProjectionIntent>,
    bases: BTreeMap<DocumentId, AnnotatedProjectionBase>,
}

impl ValidatedProjectionObjects {
    pub fn intents(&self) -> &[ManifestedProjectionIntent] {
        &self.intents
    }

    pub fn bases(&self) -> &BTreeMap<DocumentId, AnnotatedProjectionBase> {
        &self.bases
    }
}

pub fn validate_projection_object_set(
    manifest: &OperationBatch,
    objects: &[OperationObject],
) -> Result<ValidatedProjectionObjects, ProjectionManifestError> {
    let descriptors = manifest
        .required_objects()
        .iter()
        .map(|descriptor| (descriptor.content_digest(), descriptor))
        .collect::<BTreeMap<_, _>>();
    let mut bases = BTreeMap::new();
    let mut intents = Vec::new();
    let mut intent_keys = BTreeSet::new();

    for object in objects {
        let descriptor = object
            .descriptor()
            .map_err(|error| ProjectionManifestError::Object(error.to_string()))?;
        let expected = descriptors.get(&descriptor.content_digest()).ok_or(
            ProjectionManifestError::InvalidBinding("projection object has no manifest descriptor"),
        )?;
        if **expected != descriptor {
            return Err(ProjectionManifestError::InvalidBinding(
                "projection object descriptor mismatch",
            ));
        }
        match object.kind() {
            ObjectKind::ProjectionIntent => {
                let intent = ManifestedProjectionIntent::decode(object.payload())?;
                if object.document_id() != intent.descriptor_document_id() {
                    return Err(ProjectionManifestError::DescriptorDocumentMismatch {
                        kind: "projection intent",
                        expected: intent.descriptor_document_id(),
                        found: object.document_id(),
                    });
                }
                if intent.workspace_id() != manifest.workspace_id()
                    || intent.source_batch_id() != manifest.batch_id()
                    || intent.source_author_device_id() != manifest.author_device_id()
                    || intent.source_author_session_id() != manifest.author_session_id()
                {
                    return Err(ProjectionManifestError::InvalidBinding(
                        "projection intent source binding does not match manifest",
                    ));
                }
                let key = (
                    intent.source_endpoint_id(),
                    intent.page_id(),
                    intent.path().clone(),
                );
                if !intent_keys.insert(key) {
                    return Err(ProjectionManifestError::DuplicateIntent);
                }
                intents.push(intent);
            }
            ObjectKind::AnnotatedBaseBlob => {
                let base = AnnotatedProjectionBase::decode(object.payload())?;
                let expected_document_id = base.descriptor_document_id()?;
                if object.document_id() != expected_document_id {
                    return Err(ProjectionManifestError::DescriptorDocumentMismatch {
                        kind: "annotated base",
                        expected: expected_document_id,
                        found: object.document_id(),
                    });
                }
                if base.workspace_id() != manifest.workspace_id() {
                    return Err(ProjectionManifestError::InvalidBinding(
                        "annotated base workspace does not match manifest",
                    ));
                }
                if bases.insert(object.document_id(), base).is_some() {
                    return Err(ProjectionManifestError::DuplicateBase);
                }
            }
            ObjectKind::SemanticEffect | ObjectKind::CrdtUpdate => {}
        }
    }

    let mut referenced_bases = BTreeSet::new();
    for intent in &intents {
        for (reference, role) in [
            (intent.precondition().base(), "precondition"),
            (intent.render_base(), "render"),
        ] {
            let Some(reference) = reference else {
                continue;
            };
            let descriptor = manifest
                .required_objects()
                .iter()
                .find(|descriptor| descriptor.content_digest() == reference.content_digest())
                .ok_or(ProjectionManifestError::MissingBaseReference)?;
            if !reference.matches(descriptor) {
                return Err(ProjectionManifestError::BaseReferenceMismatch);
            }
            let base = bases
                .get(&reference.document_id())
                .ok_or(ProjectionManifestError::MissingBaseReference)?;
            if base.endpoint_id() != intent.source_endpoint_id()
                || base.source_page_id() != intent.page_id()
            {
                return Err(ProjectionManifestError::InvalidBinding(
                    "annotated base endpoint/page does not match intent",
                ));
            }
            if role == "precondition" && base.source_path() != intent.path() {
                return Err(ProjectionManifestError::InvalidBinding(
                    "precondition base path does not match intent path",
                ));
            }
            referenced_bases.insert(reference.document_id());
        }
    }
    if referenced_bases.len() != bases.len()
        || bases
            .keys()
            .any(|document_id| !referenced_bases.contains(document_id))
    {
        return Err(ProjectionManifestError::OrphanBase);
    }
    intents.sort_unstable_by_key(|intent| {
        (
            intent.source_endpoint_id(),
            intent.page_id(),
            intent.path().clone(),
        )
    });
    Ok(ValidatedProjectionObjects { intents, bases })
}

pub fn projection_intent_document_id(
    batch_id: BatchId,
    endpoint_id: ProjectionEndpointId,
    page_id: PageId,
    path: &ManagedPath,
) -> DocumentId {
    derive_document_id(
        b"tine/projection-intent-document-id/v2\0",
        &[
            batch_id.as_uuid().as_bytes(),
            endpoint_id.as_uuid().as_bytes(),
            page_id.as_uuid().as_bytes(),
            path.as_str().as_bytes(),
        ],
    )
}

pub fn annotated_base_document_id(canonical_payload: &[u8]) -> DocumentId {
    derive_document_id(
        b"tine/annotated-projection-base-document-id/v1\0",
        &[Sha256::digest(canonical_payload).as_slice()],
    )
}

fn derive_document_id(domain: &[u8], parts: &[&[u8]]) -> DocumentId {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    DocumentId::from_uuid(Uuid::from_bytes(bytes))
}

fn encode_canonical<T: Serialize>(
    magic: &[u8],
    value: &T,
    limit: usize,
) -> Result<Vec<u8>, ProjectionManifestError> {
    let body = postcard::to_allocvec(value)
        .map_err(|error| ProjectionManifestError::Encode(error.to_string()))?;
    let length = magic.len().saturating_add(body.len());
    if length > limit {
        return Err(ProjectionManifestError::TooLarge {
            kind: "projection object",
            length,
            limit,
        });
    }
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(magic);
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

fn validate_blob(
    description: BlobDescription,
    bytes: &[u8],
    kind: &'static str,
) -> Result<(), ProjectionManifestError> {
    let actual = BlobDescription::of(bytes);
    if actual != description {
        return Err(ProjectionManifestError::BlobMismatch(kind));
    }
    Ok(())
}

fn validate_annotations(
    annotations: &[AnnotatedIdentity],
    target_length: u64,
) -> Result<(), ProjectionManifestError> {
    if annotations.len() > MAX_PROJECTION_ANNOTATIONS {
        return Err(ProjectionManifestError::TooManyAnnotations(
            annotations.len(),
        ));
    }
    let mut locators = BTreeSet::new();
    let mut block_ids = BTreeSet::new();
    let mut logseq_uuids = BTreeSet::new();
    let mut prior_locator = None;
    for annotation in annotations {
        if prior_locator
            .as_ref()
            .is_some_and(|prior| prior >= annotation.locator())
        {
            return Err(ProjectionManifestError::NonCanonical(
                "projection annotations",
            ));
        }
        prior_locator = Some(annotation.locator().clone());
        if !locators.insert(annotation.locator().clone())
            || !block_ids.insert(annotation.block_id())
            || annotation
                .logseq_uuid()
                .is_some_and(|uuid| !logseq_uuids.insert(uuid))
        {
            return Err(ProjectionManifestError::InvalidAnnotations);
        }
        if annotation.span().end() > target_length {
            return Err(ProjectionManifestError::InvalidAnnotations);
        }
    }
    Ok(())
}

fn validate_claim_evidence(
    evidence: &[ProjectionClaimEvidence],
) -> Result<(), ProjectionManifestError> {
    if evidence.len() > MAX_PROJECTION_CLAIM_EVIDENCE {
        return Err(ProjectionManifestError::TooMuchClaimEvidence(
            evidence.len(),
        ));
    }
    let mut prior = None;
    for entry in evidence {
        if entry.participants().is_empty()
            || prior.is_some_and(|prior| prior >= entry.logseq_uuid())
            || !entry
                .participants()
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(ProjectionManifestError::NonCanonical(
                "projection claim evidence",
            ));
        }
        prior = Some(entry.logseq_uuid());
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionManifestError {
    Decode(String),
    Encode(String),
    Object(String),
    InvalidMagic(&'static str),
    UnknownVersion {
        kind: &'static str,
        expected: u32,
        found: u32,
    },
    TooLarge {
        kind: &'static str,
        length: usize,
        limit: usize,
    },
    TooManyAnnotations(usize),
    TooMuchClaimEvidence(usize),
    NonCanonical(&'static str),
    BlobMismatch(&'static str),
    InvalidAnnotations,
    DescriptorDocumentMismatch {
        kind: &'static str,
        expected: DocumentId,
        found: DocumentId,
    },
    InvalidBinding(&'static str),
    DuplicateIntent,
    DuplicateBase,
    MissingBaseReference,
    BaseReferenceMismatch,
    OrphanBase,
}

impl fmt::Display for ProjectionManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "projection object decode failed: {error}"),
            Self::Encode(error) => write!(f, "projection object encode failed: {error}"),
            Self::Object(error) => write!(f, "projection object envelope failed: {error}"),
            Self::InvalidMagic(kind) => write!(f, "invalid {kind} magic"),
            Self::UnknownVersion {
                kind,
                expected,
                found,
            } => write!(f, "unknown {kind} schema {found}; expected {expected}"),
            Self::TooLarge {
                kind,
                length,
                limit,
            } => write!(f, "{kind} is too large: {length} bytes exceeds {limit}"),
            Self::TooManyAnnotations(count) => {
                write!(f, "projection object has too many annotations: {count}")
            }
            Self::TooMuchClaimEvidence(count) => {
                write!(f, "projection object has too much claim evidence: {count}")
            }
            Self::NonCanonical(kind) => write!(f, "{kind} is not canonical"),
            Self::BlobMismatch(kind) => write!(f, "{kind} digest/length does not match bytes"),
            Self::InvalidAnnotations => f.write_str("projection annotations are invalid"),
            Self::DescriptorDocumentMismatch {
                kind,
                expected,
                found,
            } => write!(
                f,
                "{kind} descriptor document mismatch: expected {expected}, found {found}"
            ),
            Self::InvalidBinding(reason) => write!(f, "invalid projection binding: {reason}"),
            Self::DuplicateIntent => f.write_str("duplicate projection intent"),
            Self::DuplicateBase => f.write_str("duplicate annotated base"),
            Self::MissingBaseReference => {
                f.write_str("projection intent references a missing base")
            }
            Self::BaseReferenceMismatch => {
                f.write_str("projection intent base reference does not match its descriptor")
            }
            Self::OrphanBase => f.write_str("manifest contains an orphan annotated base"),
        }
    }
}

impl std::error::Error for ProjectionManifestError {}
