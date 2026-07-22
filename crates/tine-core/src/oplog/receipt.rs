use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use super::identity::{parse_digest, write_hex};
use super::{BatchId, BlockId, CrdtPeerId, DocumentId, ImportId, LogseqUuid, PageId, WorkspaceId};

/// Candidate receipt schema. These bytes are explicitly not a stable wire format.
pub const RECEIPT_SCHEMA_VERSION: u32 = 1;
pub const PROJECTION_SCHEMA_VERSION: u32 = 1;
pub const PROJECTION_POLICY_VERSION: u32 = 1;
pub const MANAGED_ENTITY_SET_VERSION: u32 = 1;
pub const DIFF_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceiptError {
    Decode(String),
    Encode(String),
    UnknownReceiptSchema(u32),
    UnknownProjectionSchema(u32),
    UnknownProjectionPolicyVersion(u32),
    UnknownManagedEntitySetVersion(u32),
    UnknownDiffSchema(u32),
    UnsafeManagedPath(String),
    InvalidSpan { start: u64, end: u64 },
    EmptyLocator,
    DuplicateLocator,
    DuplicateBlockIdentity(BlockId),
    DuplicateLogseqIdentity(LogseqUuid),
    EmptyDocumentFrontier(DocumentId),
    DuplicateDocument(DocumentId),
    DuplicateCrdtPeer(CrdtPeerId),
    DuplicateDependency(BatchId),
    NonCanonicalPeerCounters,
    NonCanonicalDependencies,
    BatchClosureDigestMismatch(DocumentId),
    NonCanonicalAnnotations,
    NonCanonicalInventory,
    NonCanonicalCompletionIds,
    BaseLengthMismatch { declared: u64, actual: u64 },
    BaseDigestMismatch,
    SpanOutsideTarget { end: u64, target_length: u64 },
    CompletionTargetMismatch,
    CompletionIntentMismatch,
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "receipt decode failed: {error}"),
            Self::Encode(error) => write!(f, "receipt encode failed: {error}"),
            Self::UnknownReceiptSchema(found) => write!(
                f,
                "unknown receipt schema {found}; expected {RECEIPT_SCHEMA_VERSION}"
            ),
            Self::UnknownProjectionSchema(found) => write!(
                f,
                "unknown projection schema {found}; expected {PROJECTION_SCHEMA_VERSION}"
            ),
            Self::UnknownProjectionPolicyVersion(found) => write!(
                f,
                "unknown projection policy version {found}; expected {PROJECTION_POLICY_VERSION}"
            ),
            Self::UnknownManagedEntitySetVersion(found) => write!(
                f,
                "unknown managed entity-set version {found}; expected {MANAGED_ENTITY_SET_VERSION}"
            ),
            Self::UnknownDiffSchema(found) => {
                write!(
                    f,
                    "unknown diff schema {found}; expected {DIFF_SCHEMA_VERSION}"
                )
            }
            Self::UnsafeManagedPath(path) => write!(f, "unsafe or unmanaged graph path {path:?}"),
            Self::InvalidSpan { start, end } => {
                write!(f, "invalid structural span {start}..{end}")
            }
            Self::EmptyLocator => f.write_str("structural locator must identify a block"),
            Self::DuplicateLocator => f.write_str("duplicate structural locator"),
            Self::DuplicateBlockIdentity(id) => write!(f, "duplicate BlockId identity claim {id}"),
            Self::DuplicateLogseqIdentity(id) => {
                write!(f, "duplicate Logseq UUID identity claim {id}")
            }
            Self::EmptyDocumentFrontier(id) => {
                write!(f, "document frontier entry {id} must not be empty")
            }
            Self::DuplicateDocument(id) => write!(f, "duplicate document dependency {id}"),
            Self::DuplicateCrdtPeer(id) => write!(f, "duplicate CRDT peer counter {id}"),
            Self::DuplicateDependency(id) => write!(f, "duplicate batch dependency {id}"),
            Self::NonCanonicalPeerCounters => {
                f.write_str("CRDT peer counters are not canonically sorted")
            }
            Self::NonCanonicalDependencies => {
                f.write_str("document dependencies are not canonically sorted")
            }
            Self::BatchClosureDigestMismatch(id) => {
                write!(f, "batch closure digest does not match document {id}")
            }
            Self::NonCanonicalAnnotations => {
                f.write_str("identity annotations are not canonically sorted")
            }
            Self::NonCanonicalInventory => {
                f.write_str("import inventory is not canonically sorted")
            }
            Self::NonCanonicalCompletionIds => {
                f.write_str("base completion IDs are not canonically sorted")
            }
            Self::BaseLengthMismatch { declared, actual } => write!(
                f,
                "base blob length mismatch: declared {declared}, actual {actual}"
            ),
            Self::BaseDigestMismatch => f.write_str("base blob digest does not match its bytes"),
            Self::SpanOutsideTarget { end, target_length } => write!(
                f,
                "annotation span ending at {end} exceeds target length {target_length}"
            ),
            Self::CompletionTargetMismatch => {
                f.write_str("completion bytes do not match the intent target")
            }
            Self::CompletionIntentMismatch => {
                f.write_str("projection completion is not bound to this intent")
            }
        }
    }
}

impl std::error::Error for ReceiptError {}

/// A graph-relative path inside the explicitly managed page/journal scope.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedPath(String);

impl ManagedPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, ReceiptError> {
        let value = value.into();
        if is_managed_path(&value) {
            Ok(Self(value))
        } else {
            Err(ReceiptError::UnsafeManagedPath(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ManagedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ManagedPath {
    type Err = ReceiptError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for ManagedPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ManagedPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

fn is_managed_path(value: &str) -> bool {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return false;
    }
    let mut segments = value.split('/');
    let Some(scope) = segments.next() else {
        return false;
    };
    if !matches!(scope, "pages" | "journals") {
        return false;
    }
    let remainder: Vec<_> = segments.collect();
    if remainder.is_empty()
        || remainder
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains(':'))
    {
        return false;
    }
    matches!(
        remainder
            .last()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, extension)| extension),
        Some("md" | "org")
    )
}

/// A structural address expressed as zero-based child indexes from the page root.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct StructuralLocator(Vec<u32>);

impl StructuralLocator {
    pub fn new(components: Vec<u32>) -> Result<Self, ReceiptError> {
        if components.is_empty() {
            Err(ReceiptError::EmptyLocator)
        } else {
            Ok(Self(components))
        }
    }

    pub fn components(&self) -> &[u32] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for StructuralLocator {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let components = Vec::<u32>::deserialize(deserializer)?;
        Self::new(components).map_err(serde::de::Error::custom)
    }
}

/// Byte offsets into the exact target projection bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StructuralSpanWire")]
pub struct StructuralSpan {
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuralSpanWire {
    start: u64,
    end: u64,
}

impl TryFrom<StructuralSpanWire> for StructuralSpan {
    type Error = ReceiptError;

    fn try_from(value: StructuralSpanWire) -> Result<Self, Self::Error> {
        Self::new(value.start, value.end)
    }
}

impl StructuralSpan {
    pub fn new(start: u64, end: u64) -> Result<Self, ReceiptError> {
        if start > end {
            Err(ReceiptError::InvalidSpan { start, end })
        } else {
            Ok(Self { start, end })
        }
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }
}

/// SHA-256 and exact byte length of an immutable blob.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlobDescription {
    sha256: [u8; 32],
    byte_length: u64,
}

impl BlobDescription {
    pub fn of(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut sha256 = [0_u8; 32];
        sha256.copy_from_slice(&digest);
        Self {
            sha256,
            byte_length: bytes.len() as u64,
        }
    }

    pub const fn from_parts(sha256: [u8; 32], byte_length: u64) -> Self {
        Self {
            sha256,
            byte_length,
        }
    }

    pub const fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    pub const fn byte_length(self) -> u64 {
        self.byte_length
    }
}

impl fmt::Debug for BlobDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobDescription")
            .field("sha256", &DigestDisplay(&self.sha256))
            .field("byte_length", &self.byte_length)
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlobDescriptionWire {
    sha256: String,
    byte_length: u64,
}

impl Serialize for BlobDescription {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        BlobDescriptionWire {
            sha256: DigestDisplay(&self.sha256).to_string(),
            byte_length: self.byte_length,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BlobDescription {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BlobDescriptionWire::deserialize(deserializer)?;
        let sha256 = parse_digest(&wire.sha256).map_err(serde::de::Error::custom)?;
        Ok(Self::from_parts(sha256, wire.byte_length))
    }
}

struct DigestDisplay<'a>(&'a [u8]);

impl fmt::Display for DigestDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(self.0, f)
    }
}

impl fmt::Debug for DigestDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Exact immutable pre-projection bytes plus their self-validating description.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BaseBlob {
    description: BlobDescription,
    bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseBlobWire {
    description: BlobDescription,
    bytes: Vec<u8>,
}

impl BaseBlob {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            description: BlobDescription::of(&bytes),
            bytes,
        }
    }

    pub fn from_parts(description: BlobDescription, bytes: Vec<u8>) -> Result<Self, ReceiptError> {
        let blob = Self { description, bytes };
        blob.validate()?;
        Ok(blob)
    }

    pub const fn description(&self) -> BlobDescription {
        self.description
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn validate(&self) -> Result<(), ReceiptError> {
        let actual = self.bytes.len() as u64;
        if self.description.byte_length != actual {
            return Err(ReceiptError::BaseLengthMismatch {
                declared: self.description.byte_length,
                actual,
            });
        }
        if BlobDescription::of(&self.bytes).sha256 != self.description.sha256 {
            return Err(ReceiptError::BaseDigestMismatch);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for BaseBlob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BaseBlobWire::deserialize(deserializer)?;
        Self::from_parts(wire.description, wire.bytes).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionPolicy {
    SparseLogseqIds,
    /// Test/developer instrumentation only; never a production migration mode.
    DenseLogseqIds,
}

/// One peer's inclusive maximum counter in a document's CRDT frontier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrdtPeerCounter {
    peer_id: CrdtPeerId,
    max_counter: u64,
}

impl CrdtPeerCounter {
    pub const fn new(peer_id: CrdtPeerId, max_counter: u64) -> Self {
        Self {
            peer_id,
            max_counter,
        }
    }

    pub const fn peer_id(self) -> CrdtPeerId {
        self.peer_id
    }

    pub const fn max_counter(self) -> u64 {
        self.max_counter
    }
}

/// Canonical digest of exactly one document frontier's transitive BatchId closure.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BatchClosureDigest([u8; 32]);

impl BatchClosureDigest {
    fn of(dependencies: &[BatchId]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/frontier-v2/batch-closure/v1\0");
        hasher.update((dependencies.len() as u64).to_be_bytes());
        for dependency in dependencies {
            hasher.update(dependency.as_uuid().as_bytes());
        }
        Self(hasher.finalize().into())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for BatchClosureDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BatchClosureDigest({self})")
    }
}

impl fmt::Display for BatchClosureDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl Serialize for BatchClosureDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BatchClosureDigest {
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentDependencies {
    document_id: DocumentId,
    peer_counters: Vec<CrdtPeerCounter>,
    batch_closure: Vec<BatchId>,
    batch_closure_digest: BatchClosureDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentDependenciesWire {
    document_id: DocumentId,
    peer_counters: Vec<CrdtPeerCounter>,
    batch_closure: Vec<BatchId>,
    batch_closure_digest: BatchClosureDigest,
}

impl DocumentDependencies {
    /// Builds a canonical document entry from the caller's complete transitive
    /// BatchId closure. This primitive can canonicalize and bind the supplied
    /// closure, but discovering that closure belongs to the causal engine.
    pub fn new(
        document_id: DocumentId,
        mut peer_counters: Vec<CrdtPeerCounter>,
        mut batch_closure: Vec<BatchId>,
    ) -> Result<Self, ReceiptError> {
        peer_counters.sort_unstable_by_key(|counter| counter.peer_id);
        batch_closure.sort_unstable();
        let document = Self {
            document_id,
            peer_counters,
            batch_closure_digest: BatchClosureDigest::of(&batch_closure),
            batch_closure,
        };
        document.validate_canonical()?;
        Ok(document)
    }

    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn batch_closure(&self) -> &[BatchId] {
        &self.batch_closure
    }

    pub fn peer_counters(&self) -> &[CrdtPeerCounter] {
        &self.peer_counters
    }

    pub const fn batch_closure_digest(&self) -> BatchClosureDigest {
        self.batch_closure_digest
    }

    fn validate_canonical(&self) -> Result<(), ReceiptError> {
        if self.peer_counters.is_empty() && self.batch_closure.is_empty() {
            return Err(ReceiptError::EmptyDocumentFrontier(self.document_id));
        }
        if !is_strictly_sorted_by_key(&self.peer_counters, |counter| counter.peer_id) {
            if let Some(pair) = self
                .peer_counters
                .windows(2)
                .find(|pair| pair[0].peer_id == pair[1].peer_id)
            {
                return Err(ReceiptError::DuplicateCrdtPeer(pair[0].peer_id));
            }
            return Err(ReceiptError::NonCanonicalPeerCounters);
        }
        if !is_strictly_sorted(&self.batch_closure) {
            if let Some(duplicate) = adjacent_duplicate(&self.batch_closure) {
                return Err(ReceiptError::DuplicateDependency(*duplicate));
            }
            return Err(ReceiptError::NonCanonicalDependencies);
        }
        if self.batch_closure_digest != BatchClosureDigest::of(&self.batch_closure) {
            return Err(ReceiptError::BatchClosureDigestMismatch(self.document_id));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for DocumentDependencies {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DocumentDependenciesWire::deserialize(deserializer)?;
        let document = Self {
            document_id: wire.document_id,
            peer_counters: wire.peer_counters,
            batch_closure: wire.batch_closure,
            batch_closure_digest: wire.batch_closure_digest,
        };
        document
            .validate_canonical()
            .map_err(serde::de::Error::custom)?;
        Ok(document)
    }
}

/// ADR 0049's sharding-neutral frontier: canonical `DocumentId` entries,
/// each containing canonical CRDT peer counters and the complete transitive
/// causal `BatchId` closure needed to materialize that document.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FrontierV2(Vec<DocumentDependencies>);

impl FrontierV2 {
    pub fn new(mut documents: Vec<DocumentDependencies>) -> Result<Self, ReceiptError> {
        documents.sort_unstable_by_key(DocumentDependencies::document_id);
        let frontier = Self(documents);
        frontier.validate()?;
        Ok(frontier)
    }

    pub fn documents(&self) -> &[DocumentDependencies] {
        &self.0
    }

    fn validate(&self) -> Result<(), ReceiptError> {
        if !is_strictly_sorted_by_key(&self.0, DocumentDependencies::document_id) {
            if let Some(pair) = self
                .0
                .windows(2)
                .find(|pair| pair[0].document_id() == pair[1].document_id())
            {
                return Err(ReceiptError::DuplicateDocument(pair[0].document_id()));
            }
            return Err(ReceiptError::NonCanonicalDependencies);
        }
        for document in &self.0 {
            document.validate_canonical()?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for FrontierV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let documents = Vec::<DocumentDependencies>::deserialize(deserializer)?;
        let frontier = Self(documents);
        frontier.validate().map_err(serde::de::Error::custom)?;
        Ok(frontier)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotatedIdentity {
    locator: StructuralLocator,
    span: StructuralSpan,
    block_id: BlockId,
    logseq_uuid: Option<LogseqUuid>,
}

impl AnnotatedIdentity {
    pub fn new(
        locator: StructuralLocator,
        span: StructuralSpan,
        block_id: BlockId,
        logseq_uuid: Option<LogseqUuid>,
    ) -> Self {
        Self {
            locator,
            span,
            block_id,
            logseq_uuid,
        }
    }

    pub fn locator(&self) -> &StructuralLocator {
        &self.locator
    }

    pub const fn span(&self) -> StructuralSpan {
        self.span
    }

    pub const fn block_id(&self) -> BlockId {
        self.block_id
    }

    pub const fn logseq_uuid(&self) -> Option<LogseqUuid> {
        self.logseq_uuid
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionPrecondition {
    Absent,
    Base(BaseBlob),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionIntent {
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
    workspace_id: WorkspaceId,
    page_id: PageId,
    path: ManagedPath,
    policy: ProjectionPolicy,
    frontier: FrontierV2,
    precondition: ProjectionPrecondition,
    target: BlobDescription,
    annotations: Vec<AnnotatedIdentity>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionIntentWire {
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
    workspace_id: WorkspaceId,
    page_id: PageId,
    path: ManagedPath,
    policy: ProjectionPolicy,
    frontier: FrontierV2,
    precondition: ProjectionPrecondition,
    target: BlobDescription,
    annotations: Vec<AnnotatedIdentity>,
}

impl ProjectionIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_id: WorkspaceId,
        page_id: PageId,
        path: ManagedPath,
        policy: ProjectionPolicy,
        frontier: FrontierV2,
        precondition: ProjectionPrecondition,
        target: BlobDescription,
        mut annotations: Vec<AnnotatedIdentity>,
    ) -> Result<Self, ReceiptError> {
        annotations.sort_unstable_by(|left, right| left.locator.cmp(&right.locator));
        let intent = Self {
            receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            projection_policy_version: PROJECTION_POLICY_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
            workspace_id,
            page_id,
            path,
            policy,
            frontier,
            precondition,
            target,
            annotations,
        };
        intent.validate()?;
        Ok(intent)
    }

    pub fn encode(&self) -> Result<Vec<u8>, ReceiptError> {
        serde_json::to_vec(self).map_err(|error| ReceiptError::Encode(error.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ReceiptError> {
        serde_json::from_slice(bytes).map_err(|error| ReceiptError::Decode(error.to_string()))
    }

    pub fn id(&self) -> Result<CompletionId, ReceiptError> {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/projection-intent-semantic-id/v1\0");
        hasher.update(self.receipt_schema_version.to_be_bytes());
        hasher.update(self.projection_schema_version.to_be_bytes());
        hasher.update(self.projection_policy_version.to_be_bytes());
        hasher.update(self.managed_entity_set_version.to_be_bytes());
        hasher.update(self.workspace_id.as_uuid().as_bytes());
        hasher.update(self.page_id.as_uuid().as_bytes());
        hash_length_delimited(&mut hasher, self.path.as_str().as_bytes());
        hasher.update([match self.policy {
            ProjectionPolicy::SparseLogseqIds => 0,
            ProjectionPolicy::DenseLogseqIds => 1,
        }]);

        hasher.update((self.frontier.documents().len() as u64).to_be_bytes());
        for document in self.frontier.documents() {
            hasher.update(document.document_id().as_uuid().as_bytes());
            hasher.update((document.peer_counters().len() as u64).to_be_bytes());
            for counter in document.peer_counters() {
                hasher.update(counter.peer_id().as_u64().to_be_bytes());
                hasher.update(counter.max_counter().to_be_bytes());
            }
            hasher.update((document.batch_closure().len() as u64).to_be_bytes());
            for dependency in document.batch_closure() {
                hasher.update(dependency.as_uuid().as_bytes());
            }
            hasher.update(document.batch_closure_digest().as_bytes());
        }

        match &self.precondition {
            ProjectionPrecondition::Absent => hasher.update([0]),
            ProjectionPrecondition::Base(base) => {
                hasher.update([1]);
                hash_blob_description(&mut hasher, base.description());
            }
        }
        hash_blob_description(&mut hasher, self.target);

        hasher.update((self.annotations.len() as u64).to_be_bytes());
        for annotation in &self.annotations {
            hasher.update((annotation.locator.components().len() as u64).to_be_bytes());
            for component in annotation.locator.components() {
                hasher.update(component.to_be_bytes());
            }
            hasher.update(annotation.span.start().to_be_bytes());
            hasher.update(annotation.span.end().to_be_bytes());
            hasher.update(annotation.block_id.as_uuid().as_bytes());
            match annotation.logseq_uuid {
                None => hasher.update([0]),
                Some(logseq_uuid) => {
                    hasher.update([1]);
                    hasher.update(logseq_uuid.as_uuid().as_bytes());
                }
            }
        }

        Ok(CompletionId::from_digest(hasher.finalize().into()))
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub const fn policy(&self) -> ProjectionPolicy {
        self.policy
    }

    pub fn frontier(&self) -> &FrontierV2 {
        &self.frontier
    }

    pub fn precondition(&self) -> &ProjectionPrecondition {
        &self.precondition
    }

    pub const fn target(&self) -> BlobDescription {
        self.target
    }

    pub fn annotations(&self) -> &[AnnotatedIdentity] {
        &self.annotations
    }

    fn from_wire(wire: ProjectionIntentWire) -> Result<Self, ReceiptError> {
        let intent = Self {
            receipt_schema_version: wire.receipt_schema_version,
            projection_schema_version: wire.projection_schema_version,
            projection_policy_version: wire.projection_policy_version,
            managed_entity_set_version: wire.managed_entity_set_version,
            workspace_id: wire.workspace_id,
            page_id: wire.page_id,
            path: wire.path,
            policy: wire.policy,
            frontier: wire.frontier,
            precondition: wire.precondition,
            target: wire.target,
            annotations: wire.annotations,
        };
        intent.validate()?;
        Ok(intent)
    }

    fn validate(&self) -> Result<(), ReceiptError> {
        validate_versions(
            self.receipt_schema_version,
            self.projection_schema_version,
            self.projection_policy_version,
            self.managed_entity_set_version,
        )?;
        self.frontier.validate()?;
        if let ProjectionPrecondition::Base(base) = &self.precondition {
            base.validate()?;
        }
        validate_annotations(&self.annotations, self.target.byte_length)
    }
}

impl<'de> Deserialize<'de> for ProjectionIntent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProjectionIntentWire::deserialize(deserializer)?;
        Self::from_wire(wire).map_err(serde::de::Error::custom)
    }
}

fn validate_annotations(
    annotations: &[AnnotatedIdentity],
    target_length: u64,
) -> Result<(), ReceiptError> {
    if !is_strictly_sorted_by_key(annotations, |annotation| annotation.locator.clone()) {
        if annotations
            .windows(2)
            .any(|pair| pair[0].locator == pair[1].locator)
        {
            return Err(ReceiptError::DuplicateLocator);
        }
        return Err(ReceiptError::NonCanonicalAnnotations);
    }

    let mut block_ids = HashSet::with_capacity(annotations.len());
    let mut logseq_ids = HashSet::with_capacity(annotations.len());
    for annotation in annotations {
        if !block_ids.insert(annotation.block_id) {
            return Err(ReceiptError::DuplicateBlockIdentity(annotation.block_id));
        }
        if let Some(logseq_uuid) = annotation.logseq_uuid {
            if !logseq_ids.insert(logseq_uuid) {
                return Err(ReceiptError::DuplicateLogseqIdentity(logseq_uuid));
            }
        }
        if annotation.span.end > target_length {
            return Err(ReceiptError::SpanOutsideTarget {
                end: annotation.span.end,
                target_length,
            });
        }
    }
    Ok(())
}

/// Digest binding a completion to the complete candidate intent semantics.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CompletionId([u8; 32]);

impl CompletionId {
    const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CompletionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CompletionId({self})")
    }
}

impl fmt::Display for CompletionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl Serialize for CompletionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CompletionId {
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

/// Candidate completion evidence. Publishing and durability are intentionally
/// outside P0B; this type only expresses and verifies intent binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionCompletion {
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
    intent_id: CompletionId,
    workspace_id: WorkspaceId,
    page_id: PageId,
    path: ManagedPath,
    target: BlobDescription,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionCompletionWire {
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
    intent_id: CompletionId,
    workspace_id: WorkspaceId,
    page_id: PageId,
    path: ManagedPath,
    target: BlobDescription,
}

impl ProjectionCompletion {
    pub fn for_intent(
        intent: &ProjectionIntent,
        reread_bytes: &[u8],
    ) -> Result<Self, ReceiptError> {
        let observed = BlobDescription::of(reread_bytes);
        if observed != intent.target {
            return Err(ReceiptError::CompletionTargetMismatch);
        }
        Ok(Self {
            receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            projection_policy_version: PROJECTION_POLICY_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
            intent_id: intent.id()?,
            workspace_id: intent.workspace_id,
            page_id: intent.page_id,
            path: intent.path.clone(),
            target: observed,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, ReceiptError> {
        serde_json::to_vec(self).map_err(|error| ReceiptError::Encode(error.to_string()))
    }

    pub fn decode_bound(bytes: &[u8], intent: &ProjectionIntent) -> Result<Self, ReceiptError> {
        let wire: ProjectionCompletionWire = serde_json::from_slice(bytes)
            .map_err(|error| ReceiptError::Decode(error.to_string()))?;
        let completion = Self::from_wire(wire)?;
        completion.validate_against(intent)?;
        Ok(completion)
    }

    pub fn validate_against(&self, intent: &ProjectionIntent) -> Result<(), ReceiptError> {
        if self.intent_id != intent.id()?
            || self.workspace_id != intent.workspace_id
            || self.page_id != intent.page_id
            || self.path != intent.path
            || self.target != intent.target
        {
            return Err(ReceiptError::CompletionIntentMismatch);
        }
        Ok(())
    }

    pub const fn intent_id(&self) -> CompletionId {
        self.intent_id
    }

    fn from_wire(wire: ProjectionCompletionWire) -> Result<Self, ReceiptError> {
        validate_versions(
            wire.receipt_schema_version,
            wire.projection_schema_version,
            wire.projection_policy_version,
            wire.managed_entity_set_version,
        )?;
        Ok(Self {
            receipt_schema_version: wire.receipt_schema_version,
            projection_schema_version: wire.projection_schema_version,
            projection_policy_version: wire.projection_policy_version,
            managed_entity_set_version: wire.managed_entity_set_version,
            intent_id: wire.intent_id,
            workspace_id: wire.workspace_id,
            page_id: wire.page_id,
            path: wire.path,
            target: wire.target,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportInventoryState {
    Present(BlobDescription),
    Absent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportInventoryEntry {
    path: ManagedPath,
    state: ImportInventoryState,
}

impl ImportInventoryEntry {
    pub fn new(path: ManagedPath, state: ImportInventoryState) -> Self {
        Self { path, state }
    }

    pub fn path(&self) -> &ManagedPath {
        &self.path
    }
}

/// Stable import derivation input for one unmatched page or block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportLocator {
    path: ManagedPath,
    block: Option<StructuralLocator>,
}

impl ImportLocator {
    pub fn page(path: ManagedPath) -> Self {
        Self { path, block: None }
    }

    pub fn block(path: ManagedPath, locator: StructuralLocator) -> Self {
        Self {
            path,
            block: Some(locator),
        }
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.path.as_str().len() + 16);
        bytes.extend_from_slice(&(self.path.as_str().len() as u64).to_be_bytes());
        bytes.extend_from_slice(self.path.as_str().as_bytes());
        match &self.block {
            None => bytes.push(0),
            Some(locator) => {
                bytes.push(1);
                bytes.extend_from_slice(&(locator.components().len() as u64).to_be_bytes());
                for component in locator.components() {
                    bytes.extend_from_slice(&component.to_be_bytes());
                }
            }
        }
        bytes
    }
}

impl ImportId {
    /// Derive a reconciliation identity from canonical completion and inventory evidence.
    pub fn derive(
        workspace_id: WorkspaceId,
        completion_ids: &[CompletionId],
        inventory: &[ImportInventoryEntry],
        diff_schema_version: u32,
    ) -> Result<Self, ReceiptError> {
        if diff_schema_version != DIFF_SCHEMA_VERSION {
            return Err(ReceiptError::UnknownDiffSchema(diff_schema_version));
        }
        if !is_strictly_sorted(completion_ids) && completion_ids.len() > 1 {
            return Err(ReceiptError::NonCanonicalCompletionIds);
        }
        if !is_strictly_sorted_by_key(inventory, |entry| entry.path.clone()) && inventory.len() > 1
        {
            return Err(ReceiptError::NonCanonicalInventory);
        }

        let mut hasher = Sha256::new();
        hasher.update(b"tine/import/reconciliation-id/v1\0");
        hasher.update(workspace_id.as_uuid().as_bytes());
        hasher.update(diff_schema_version.to_be_bytes());
        hasher.update((completion_ids.len() as u64).to_be_bytes());
        for id in completion_ids {
            hasher.update(id.as_bytes());
        }
        hasher.update((inventory.len() as u64).to_be_bytes());
        for entry in inventory {
            let path = entry.path.as_str().as_bytes();
            hasher.update((path.len() as u64).to_be_bytes());
            hasher.update(path);
            match entry.state {
                ImportInventoryState::Absent => hasher.update([0]),
                ImportInventoryState::Present(description) => {
                    hasher.update([1]);
                    hasher.update(description.sha256());
                    hasher.update(description.byte_length().to_be_bytes());
                }
            }
        }
        let digest = hasher.finalize();
        let mut value = [0_u8; 32];
        value.copy_from_slice(&digest);
        Ok(Self::from_digest(value))
    }

    pub fn unmatched_page_id(self, locator: &ImportLocator) -> PageId {
        PageId::for_unmatched_import(self, &locator.canonical_bytes())
    }

    pub fn unmatched_block_id(self, locator: &ImportLocator) -> BlockId {
        BlockId::for_unmatched_import(self, &locator.canonical_bytes())
    }

    pub fn batch_id(self) -> BatchId {
        BatchId::for_import(self)
    }
}

fn validate_versions(
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
) -> Result<(), ReceiptError> {
    if receipt_schema_version != RECEIPT_SCHEMA_VERSION {
        return Err(ReceiptError::UnknownReceiptSchema(receipt_schema_version));
    }
    if projection_schema_version != PROJECTION_SCHEMA_VERSION {
        return Err(ReceiptError::UnknownProjectionSchema(
            projection_schema_version,
        ));
    }
    if projection_policy_version != PROJECTION_POLICY_VERSION {
        return Err(ReceiptError::UnknownProjectionPolicyVersion(
            projection_policy_version,
        ));
    }
    if managed_entity_set_version != MANAGED_ENTITY_SET_VERSION {
        return Err(ReceiptError::UnknownManagedEntitySetVersion(
            managed_entity_set_version,
        ));
    }
    Ok(())
}

fn hash_length_delimited(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hash_blob_description(hasher: &mut Sha256, description: BlobDescription) {
    hasher.update(description.sha256());
    hasher.update(description.byte_length().to_be_bytes());
}

fn adjacent_duplicate<T: Eq>(values: &[T]) -> Option<&T> {
    values
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| &pair[0])
}

fn is_strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn is_strictly_sorted_by_key<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}
