use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use caseless::Caseless;

use super::identity::{parse_digest, write_hex};
use super::{BatchId, BlockId, CrdtPeerId, DocumentId, ImportId, LogseqUuid, PageId, WorkspaceId};

/// Candidate receipt schema. These bytes are explicitly not a stable wire format.
pub const RECEIPT_SCHEMA_VERSION: u32 = 5;
pub const PROJECTION_SCHEMA_VERSION: u32 = 4;
pub const PROJECTION_POLICY_VERSION: u32 = 1;
pub const MANAGED_ENTITY_SET_VERSION: u32 = 2;
pub const DIFF_SCHEMA_VERSION: u32 = 2;
pub const PORTABLE_PATH_KEY_VERSION: u32 = 1;
pub const PORTABLE_PATH_NORMALIZATION_UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0);
pub const PORTABLE_PATH_CASE_FOLD_UNICODE_VERSION: (u64, u64, u64) = (16, 0, 0);

const _: () = {
    let (major, minor, patch) = unicode_normalization::UNICODE_VERSION;
    assert!(major == 17 && minor == 0 && patch == 0);
};
const _: () = {
    let (major, minor, patch) = caseless::UNICODE_VERSION;
    assert!(major == 16 && minor == 0 && patch == 0);
};

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
    CausalStateDigestMismatch(DocumentId),
    NonCanonicalAnnotations,
    EmptyProjectionClaimEvidence(LogseqUuid),
    NonCanonicalProjectionClaimEvidence,
    MissingProjectionClaimEvidence(LogseqUuid),
    MissingProjectionClaimDocument(DocumentId),
    NonCanonicalInventory,
    NonCanonicalLogicalCompletionIds,
    BaseLengthMismatch { declared: u64, actual: u64 },
    BaseDigestMismatch,
    SpanOutsideTarget { end: u64, target_length: u64 },
    CompletionTargetMismatch,
    CompletionIntentMismatch,
    CompletionIdentityMismatch,
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
            Self::CausalStateDigestMismatch(id) => {
                write!(
                    f,
                    "compact causal-state digest does not match document {id}"
                )
            }
            Self::NonCanonicalAnnotations => {
                f.write_str("identity annotations are not canonically sorted")
            }
            Self::EmptyProjectionClaimEvidence(uuid) => {
                write!(
                    f,
                    "projection claim evidence for {uuid} has no participants"
                )
            }
            Self::NonCanonicalProjectionClaimEvidence => {
                f.write_str("projection claim evidence is not canonical")
            }
            Self::MissingProjectionClaimEvidence(uuid) => {
                write!(f, "projection annotation for {uuid} has no claim evidence")
            }
            Self::MissingProjectionClaimDocument(document_id) => write!(
                f,
                "projection claim participant home {document_id} is absent from the frontier"
            ),
            Self::NonCanonicalInventory => {
                f.write_str("import inventory is not canonically sorted")
            }
            Self::NonCanonicalLogicalCompletionIds => {
                f.write_str("base logical completion IDs are not canonically sorted")
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
            Self::CompletionIdentityMismatch => {
                f.write_str("projection completion semantic identity does not match its evidence")
            }
        }
    }
}

impl std::error::Error for ReceiptError {}

/// A canonical graph-relative Markdown/Org path.
///
/// This type establishes portable lexical safety only. Whether the path belongs
/// to a configured managed root is authorized by the graph capability.
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

    /// Compute the versioned portable comparison key without changing the
    /// exact spelling retained and projected by this managed path.
    pub fn portable_key(&self) -> PortablePathKey {
        PortablePathKey::from_managed_path(self)
    }
}

/// Canonical portable comparison bytes. This value is not a projected path.
///
/// Each component uses `NFC(default_case_fold(NFD(component)))`; components
/// are then joined by a literal slash. Compatibility normalization is
/// deliberately excluded.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PortablePathKey(String);

impl PortablePathKey {
    fn from_managed_path(path: &ManagedPath) -> Self {
        let mut key = String::with_capacity(path.as_str().len());
        for (index, component) in path.as_str().split('/').enumerate() {
            if index != 0 {
                key.push('/');
            }
            key.extend(component.chars().nfd().default_case_fold().nfc());
        }
        Self(key)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn digest(&self) -> PortablePathKeyDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/portable-path-key/v1\0");
        hasher.update(PORTABLE_PATH_KEY_VERSION.to_be_bytes());
        hasher.update((self.0.len() as u64).to_be_bytes());
        hasher.update(self.0.as_bytes());
        PortablePathKeyDigest(hasher.finalize().into())
    }
}

/// Domain-separated authenticated-index key for a portable path.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PortablePathKeyDigest([u8; 32]);

impl PortablePathKeyDigest {
    pub const fn as_bytes(&self) -> &[u8; 32] {
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
    if value.is_empty() || value != value.trim() || value.starts_with('/') || value.contains('\\') {
        return false;
    }
    let segments: Vec<_> = value.split('/').collect();
    if segments.len() < 2
        || segments
            .iter()
            .any(|part| !managed_component_is_portable(part))
    {
        return false;
    }
    matches!(
        segments
            .last()
            .and_then(|name| name.rsplit_once('.'))
            .filter(|(stem, _)| !stem.is_empty())
            .map(|(_, extension)| extension),
        Some("md" | "org"),
    )
}

pub(crate) fn managed_component_is_portable(component: &str) -> bool {
    if component.is_empty()
        || matches!(component, "." | "..")
        || component.ends_with(' ')
        || component.ends_with('.')
        || component.chars().any(is_forbidden_win32_path_character)
    {
        return false;
    }
    let device_stem = component
        .split_once('.')
        .map_or(component, |(stem, _)| stem)
        .to_ascii_uppercase();
    !matches!(
        device_stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    )
}

fn is_forbidden_win32_path_character(character: char) -> bool {
    character == '\0'
        || character.is_control()
        || matches!(character, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
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
enum ProjectionPolicyWire {
    SparseLogseqIds,
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

/// Canonical digest of one document's exact CRDT counters and direct batch heads.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DocumentCausalDigest([u8; 32]);

impl DocumentCausalDigest {
    fn of(
        document_id: DocumentId,
        peer_counters: &[CrdtPeerCounter],
        direct_dependency_heads: &[BatchId],
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/frontier-v2/document-causal-state/v1\0");
        hasher.update(document_id.as_uuid().as_bytes());
        hasher.update((peer_counters.len() as u64).to_be_bytes());
        for counter in peer_counters {
            hasher.update(counter.peer_id().as_u64().to_be_bytes());
            hasher.update(counter.max_counter().to_be_bytes());
        }
        hasher.update((direct_dependency_heads.len() as u64).to_be_bytes());
        for head in direct_dependency_heads {
            hasher.update(head.as_uuid().as_bytes());
        }
        Self(hasher.finalize().into())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DocumentCausalDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DocumentCausalDigest({self})")
    }
}

impl fmt::Display for DocumentCausalDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl Serialize for DocumentCausalDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DocumentCausalDigest {
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
    direct_dependency_heads: Vec<BatchId>,
    causal_state_digest: DocumentCausalDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentDependenciesWire {
    document_id: DocumentId,
    peer_counters: Vec<CrdtPeerCounter>,
    direct_dependency_heads: Vec<BatchId>,
    causal_state_digest: DocumentCausalDigest,
}

impl DocumentDependencies {
    /// Builds a canonical compact document frontier. Heads are only the
    /// maximal/direct batches whose complete atomic ancestry is accepted.
    pub fn new(
        document_id: DocumentId,
        mut peer_counters: Vec<CrdtPeerCounter>,
        mut direct_dependency_heads: Vec<BatchId>,
    ) -> Result<Self, ReceiptError> {
        peer_counters.sort_unstable_by_key(|counter| counter.peer_id);
        direct_dependency_heads.sort_unstable();
        let causal_state_digest =
            DocumentCausalDigest::of(document_id, &peer_counters, &direct_dependency_heads);
        let document = Self {
            document_id,
            peer_counters,
            direct_dependency_heads,
            causal_state_digest,
        };
        document.validate_canonical()?;
        Ok(document)
    }

    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn direct_dependency_heads(&self) -> &[BatchId] {
        &self.direct_dependency_heads
    }

    pub fn peer_counters(&self) -> &[CrdtPeerCounter] {
        &self.peer_counters
    }

    pub const fn causal_state_digest(&self) -> DocumentCausalDigest {
        self.causal_state_digest
    }

    fn validate_canonical(&self) -> Result<(), ReceiptError> {
        if self.peer_counters.is_empty() && self.direct_dependency_heads.is_empty() {
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
        if !is_strictly_sorted(&self.direct_dependency_heads) {
            if let Some(duplicate) = adjacent_duplicate(&self.direct_dependency_heads) {
                return Err(ReceiptError::DuplicateDependency(*duplicate));
            }
            return Err(ReceiptError::NonCanonicalDependencies);
        }
        if self.causal_state_digest
            != DocumentCausalDigest::of(
                self.document_id,
                &self.peer_counters,
                &self.direct_dependency_heads,
            )
        {
            return Err(ReceiptError::CausalStateDigestMismatch(self.document_id));
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
            direct_dependency_heads: wire.direct_dependency_heads,
            causal_state_digest: wire.causal_state_digest,
        };
        document
            .validate_canonical()
            .map_err(serde::de::Error::custom)?;
        Ok(document)
    }
}

/// ADR 0049's compact sharding-neutral frontier: canonical `DocumentId`
/// entries with exact CRDT counters and maximal/direct atomic batch heads.
/// Cold validation reconstructs ancestry from immutable manifests.
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
    Base(BlobDescription),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionClaimParticipant {
    block_id: BlockId,
    home_document_id: DocumentId,
}

impl ProjectionClaimParticipant {
    pub const fn new(block_id: BlockId, home_document_id: DocumentId) -> Self {
        Self {
            block_id,
            home_document_id,
        }
    }

    pub const fn block_id(self) -> BlockId {
        self.block_id
    }

    pub const fn home_document_id(self) -> DocumentId {
        self.home_document_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionClaimEvidence {
    logseq_uuid: LogseqUuid,
    participants: Vec<ProjectionClaimParticipant>,
}

impl ProjectionClaimEvidence {
    pub fn new(
        logseq_uuid: LogseqUuid,
        mut participants: Vec<ProjectionClaimParticipant>,
    ) -> Result<Self, ReceiptError> {
        participants.sort_unstable();
        participants.dedup();
        let evidence = Self {
            logseq_uuid,
            participants,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    pub const fn logseq_uuid(&self) -> LogseqUuid {
        self.logseq_uuid
    }

    pub fn participants(&self) -> &[ProjectionClaimParticipant] {
        &self.participants
    }

    fn validate(&self) -> Result<(), ReceiptError> {
        if self.participants.is_empty() {
            return Err(ReceiptError::EmptyProjectionClaimEvidence(self.logseq_uuid));
        }
        if !is_strictly_sorted(&self.participants) {
            return Err(ReceiptError::NonCanonicalProjectionClaimEvidence);
        }
        Ok(())
    }
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
    policy: ProjectionPolicyWire,
    frontier: FrontierV2,
    claim_evidence: Vec<ProjectionClaimEvidence>,
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
    policy: ProjectionPolicyWire,
    frontier: FrontierV2,
    claim_evidence: Vec<ProjectionClaimEvidence>,
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
        frontier: FrontierV2,
        mut claim_evidence: Vec<ProjectionClaimEvidence>,
        precondition: ProjectionPrecondition,
        target: BlobDescription,
        mut annotations: Vec<AnnotatedIdentity>,
    ) -> Result<Self, ReceiptError> {
        annotations.sort_unstable_by(|left, right| left.locator.cmp(&right.locator));
        claim_evidence.sort_unstable_by_key(ProjectionClaimEvidence::logseq_uuid);
        let intent = Self {
            receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            projection_policy_version: PROJECTION_POLICY_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
            workspace_id,
            page_id,
            path,
            policy: ProjectionPolicyWire::SparseLogseqIds,
            frontier,
            claim_evidence,
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

    pub fn id(&self) -> Result<ProjectionIntentId, ReceiptError> {
        let mut hasher = Sha256::new();
        hasher.update(b"tine/projection-intent-semantic-id/v1\0");
        hasher.update(self.receipt_schema_version.to_be_bytes());
        hasher.update(self.projection_schema_version.to_be_bytes());
        hasher.update(self.projection_policy_version.to_be_bytes());
        hasher.update(self.managed_entity_set_version.to_be_bytes());
        hasher.update(self.workspace_id.as_uuid().as_bytes());
        hasher.update(self.page_id.as_uuid().as_bytes());
        hash_length_delimited(&mut hasher, self.path.as_str().as_bytes());
        hasher.update([0]);

        hasher.update((self.frontier.documents().len() as u64).to_be_bytes());
        for document in self.frontier.documents() {
            hasher.update(document.document_id().as_uuid().as_bytes());
            hasher.update((document.peer_counters().len() as u64).to_be_bytes());
            for counter in document.peer_counters() {
                hasher.update(counter.peer_id().as_u64().to_be_bytes());
                hasher.update(counter.max_counter().to_be_bytes());
            }
            hasher.update((document.direct_dependency_heads().len() as u64).to_be_bytes());
            for dependency in document.direct_dependency_heads() {
                hasher.update(dependency.as_uuid().as_bytes());
            }
            hasher.update(document.causal_state_digest().as_bytes());
        }

        hasher.update((self.claim_evidence.len() as u64).to_be_bytes());
        for evidence in &self.claim_evidence {
            hasher.update(evidence.logseq_uuid.as_uuid().as_bytes());
            hasher.update((evidence.participants.len() as u64).to_be_bytes());
            for participant in &evidence.participants {
                hasher.update(participant.block_id.as_uuid().as_bytes());
                hasher.update(participant.home_document_id.as_uuid().as_bytes());
            }
        }

        match &self.precondition {
            ProjectionPrecondition::Absent => hasher.update([0]),
            ProjectionPrecondition::Base(description) => {
                hasher.update([1]);
                hash_blob_description(&mut hasher, *description);
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

        Ok(ProjectionIntentId::from_digest(hasher.finalize().into()))
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

    pub fn frontier(&self) -> &FrontierV2 {
        &self.frontier
    }

    pub fn claim_evidence(&self) -> &[ProjectionClaimEvidence] {
        &self.claim_evidence
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
            claim_evidence: wire.claim_evidence,
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
        validate_annotations(&self.annotations, self.target.byte_length)?;
        if !is_strictly_sorted_by_key(&self.claim_evidence, |evidence| evidence.logseq_uuid) {
            return Err(ReceiptError::NonCanonicalProjectionClaimEvidence);
        }
        for evidence in &self.claim_evidence {
            evidence.validate()?;
            for participant in evidence.participants() {
                if self
                    .frontier
                    .documents()
                    .binary_search_by_key(
                        &participant.home_document_id(),
                        DocumentDependencies::document_id,
                    )
                    .is_err()
                {
                    return Err(ReceiptError::MissingProjectionClaimDocument(
                        participant.home_document_id(),
                    ));
                }
            }
        }
        for uuid in self
            .annotations
            .iter()
            .filter_map(AnnotatedIdentity::logseq_uuid)
        {
            if self
                .claim_evidence
                .binary_search_by_key(&uuid, ProjectionClaimEvidence::logseq_uuid)
                .is_err()
            {
                return Err(ReceiptError::MissingProjectionClaimEvidence(uuid));
            }
        }
        Ok(())
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

pub(crate) fn validate_annotations(
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

/// Replica-stable digest binding one immutable projection intent.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectionIntentId([u8; 32]);

impl ProjectionIntentId {
    const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[cfg(test)]
    pub(crate) const fn test_only_zero() -> Self {
        Self([0; 32])
    }
}

impl fmt::Debug for ProjectionIntentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProjectionIntentId({self})")
    }
}

impl fmt::Display for ProjectionIntentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl Serialize for ProjectionIntentId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ProjectionIntentId {
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

/// Replica-stable logical completion identity. This type is intentionally
/// distinct from local projection-attempt and forensic-evidence identities so
/// the importer cannot accidentally derive an ImportId from device-local data.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LogicalCompletionId([u8; 32]);

impl LogicalCompletionId {
    const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for LogicalCompletionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LogicalCompletionId({self})")
    }
}

impl fmt::Display for LogicalCompletionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl Serialize for LogicalCompletionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LogicalCompletionId {
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

/// Stable completion receipt. Local recovery filenames and displacement
/// observations live only in the separate immutable forensic catalog.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectionCompletion {
    receipt_schema_version: u32,
    projection_schema_version: u32,
    projection_policy_version: u32,
    managed_entity_set_version: u32,
    intent_id: ProjectionIntentId,
    logical_completion_id: LogicalCompletionId,
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
    intent_id: ProjectionIntentId,
    logical_completion_id: LogicalCompletionId,
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
        let intent_id = intent.id()?;
        Ok(Self {
            receipt_schema_version: RECEIPT_SCHEMA_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            projection_policy_version: PROJECTION_POLICY_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
            intent_id,
            logical_completion_id: logical_completion_id(intent_id, observed),
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
        if self.logical_completion_id != logical_completion_id(self.intent_id, self.target) {
            return Err(ReceiptError::CompletionIdentityMismatch);
        }
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

    pub const fn intent_id(&self) -> ProjectionIntentId {
        self.intent_id
    }

    pub const fn logical_completion_id(&self) -> LogicalCompletionId {
        self.logical_completion_id
    }

    fn from_wire(wire: ProjectionCompletionWire) -> Result<Self, ReceiptError> {
        validate_versions(
            wire.receipt_schema_version,
            wire.projection_schema_version,
            wire.projection_policy_version,
            wire.managed_entity_set_version,
        )?;
        let completion = Self {
            receipt_schema_version: wire.receipt_schema_version,
            projection_schema_version: wire.projection_schema_version,
            projection_policy_version: wire.projection_policy_version,
            managed_entity_set_version: wire.managed_entity_set_version,
            intent_id: wire.intent_id,
            logical_completion_id: wire.logical_completion_id,
            workspace_id: wire.workspace_id,
            page_id: wire.page_id,
            path: wire.path,
            target: wire.target,
        };
        if completion.logical_completion_id
            != logical_completion_id(completion.intent_id, completion.target)
        {
            return Err(ReceiptError::CompletionIdentityMismatch);
        }
        Ok(completion)
    }
}

fn logical_completion_id(
    intent_id: ProjectionIntentId,
    target: BlobDescription,
) -> LogicalCompletionId {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/logical-projection-completion-id/v1\0");
    hasher.update(intent_id.as_bytes());
    hash_blob_description(&mut hasher, target);
    LogicalCompletionId::from_digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportInventoryState {
    Present(BlobDescription),
    Absent,
}

/// Durable classification of a managed text path at the graph capability
/// boundary. It is part of the reconciliation identity, not inferred from a
/// basename or a normalized path spelling.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedTextKind {
    Page,
    Journal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportInventoryEntry {
    kind: ManagedTextKind,
    path: ManagedPath,
    state: ImportInventoryState,
}

impl ImportInventoryEntry {
    /// Construct an entry using the legacy fixed-layout path classification.
    ///
    /// New callers that have a graph capability must use
    /// [`Self::with_kind`] with [`ManagedTextKind`] returned by its configured
    /// root classifier. This compatibility constructor preserves the existing
    /// unactivated raw-inventory boundary until that caller is updated.
    pub fn new(path: ManagedPath, state: ImportInventoryState) -> Self {
        let kind = if path.as_str().starts_with("journals/") {
            ManagedTextKind::Journal
        } else {
            ManagedTextKind::Page
        };
        Self { kind, path, state }
    }

    pub fn with_kind(
        kind: ManagedTextKind,
        path: ManagedPath,
        state: ImportInventoryState,
    ) -> Self {
        Self { kind, path, state }
    }

    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub const fn kind(&self) -> ManagedTextKind {
        self.kind
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
        logical_completion_ids: &[LogicalCompletionId],
        inventory: &[ImportInventoryEntry],
        diff_schema_version: u32,
    ) -> Result<Self, ReceiptError> {
        if diff_schema_version != DIFF_SCHEMA_VERSION {
            return Err(ReceiptError::UnknownDiffSchema(diff_schema_version));
        }
        if !is_strictly_sorted(logical_completion_ids) && logical_completion_ids.len() > 1 {
            return Err(ReceiptError::NonCanonicalLogicalCompletionIds);
        }
        if !is_strictly_sorted_by_key(inventory, |entry| entry.path.clone()) && inventory.len() > 1
        {
            return Err(ReceiptError::NonCanonicalInventory);
        }

        let mut hasher = Sha256::new();
        hasher.update(b"tine/import/reconciliation-id/v2\0");
        hasher.update(workspace_id.as_uuid().as_bytes());
        hasher.update(diff_schema_version.to_be_bytes());
        hasher.update((logical_completion_ids.len() as u64).to_be_bytes());
        for id in logical_completion_ids {
            hasher.update(id.as_bytes());
        }
        hasher.update((inventory.len() as u64).to_be_bytes());
        for entry in inventory {
            let path = entry.path.as_str().as_bytes();
            hasher.update((path.len() as u64).to_be_bytes());
            hasher.update(path);
            hasher.update([match entry.kind {
                ManagedTextKind::Page => 0,
                ManagedTextKind::Journal => 1,
            }]);
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn workspace() -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(0x1020_3040_5060_7080_90a0_b0c0_d0e0_f001))
    }

    fn entry(
        kind: ManagedTextKind,
        path: &str,
        state: ImportInventoryState,
    ) -> ImportInventoryEntry {
        ImportInventoryEntry::with_kind(kind, ManagedPath::parse(path).unwrap(), state)
    }

    #[test]
    fn prior_managed_entity_set_version_fails_closed() {
        assert_eq!(
            validate_versions(
                RECEIPT_SCHEMA_VERSION,
                PROJECTION_SCHEMA_VERSION,
                PROJECTION_POLICY_VERSION,
                MANAGED_ENTITY_SET_VERSION - 1,
            ),
            Err(ReceiptError::UnknownManagedEntitySetVersion(
                MANAGED_ENTITY_SET_VERSION - 1
            ))
        );
    }

    #[test]
    fn publisher_p1_import_id_v2_golden_vector_binds_text_kind_before_observation_state() {
        let inventory = vec![
            entry(
                ManagedTextKind::Journal,
                "journals/2026/07/24.md",
                ImportInventoryState::Absent,
            ),
            entry(
                ManagedTextKind::Page,
                "pages/nested/café.md",
                ImportInventoryState::Present(BlobDescription::of(b"contents")),
            ),
        ];
        let id = ImportId::derive(workspace(), &[], &inventory, DIFF_SCHEMA_VERSION).unwrap();
        assert_eq!(
            id,
            ImportId::derive(workspace(), &[], &inventory, DIFF_SCHEMA_VERSION).unwrap()
        );
        assert_eq!(
            id.to_string(),
            "40db2d59719cc970dfc3f9009e0d0045c591da4f42782b7935584db9eed3a3dc"
        );
    }

    #[test]
    fn publisher_p1_import_inventory_kind_is_canonical_identity_input_and_sort_key() {
        let page = entry(
            ManagedTextKind::Page,
            "pages/same.md",
            ImportInventoryState::Absent,
        );
        let journal = entry(
            ManagedTextKind::Journal,
            "pages/same.md",
            ImportInventoryState::Absent,
        );
        let page_id =
            ImportId::derive(workspace(), &[], &[page.clone()], DIFF_SCHEMA_VERSION).unwrap();
        let journal_id =
            ImportId::derive(workspace(), &[], &[journal.clone()], DIFF_SCHEMA_VERSION).unwrap();
        assert_ne!(page_id, journal_id);

        assert_eq!(
            ImportId::derive(
                workspace(),
                &[],
                &[page.clone(), journal.clone()],
                DIFF_SCHEMA_VERSION,
            ),
            Err(ReceiptError::NonCanonicalInventory)
        );
        assert_eq!(
            ImportId::derive(workspace(), &[], &[journal, page], DIFF_SCHEMA_VERSION,),
            Err(ReceiptError::NonCanonicalInventory)
        );
        assert_eq!(
            ImportId::derive(workspace(), &[], &[], DIFF_SCHEMA_VERSION - 1),
            Err(ReceiptError::UnknownDiffSchema(DIFF_SCHEMA_VERSION - 1))
        );
    }
}
