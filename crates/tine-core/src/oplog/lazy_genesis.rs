//! Versioned lazy-genesis semantic pack for managed-storage activation.
//!
//! This module owns the durable byte format, not activation's process-only
//! parser handoff. It intentionally contains no search/query/reference facets:
//! those are disposable SQLite projection facts. Ordinary CRDT checkpoints are
//! added by the terminal constructor before this candidate becomes publishable.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    BlobDescription, BlockId, ContentDigest, DeviceId, DocumentDependencies, DocumentId,
    LineageDigest, LogseqUuid, ManagedPath, ManagedTextKind, PageId, WorkspaceId,
    OPLOG_PROTOCOL_VERSION,
};

/// 7: page shards use ordinary text children and physically remove live block
/// keys. Older private state takes the preserve-and-rebuild route (D-1).
const LAZY_GENESIS_SCHEMA_VERSION: u32 = 7;
const LAZY_GENESIS_PAGE_CAPSULE_SCHEMA_VERSION: u32 = 6;
const LAZY_GENESIS_SQLITE_RECEIPT_SCHEMA_VERSION: u32 = 1;
/// Bump whenever the parser-to-materialized-page projection changes. A stale
/// receipt remains readable but is ignored in favour of the retained parser.
const LAZY_GENESIS_PARSER_SCHEMA_VERSION: u32 = 1;
const LAZY_GENESIS_COMMIT_SCHEMA_VERSION: u32 = 1;
const LAZY_GENESIS_PROVIDER_INDEX_SCHEMA_VERSION: u32 = 1;
const LAZY_GENESIS_ACTIVATION_MARKER_SCHEMA_VERSION: u32 = 1;
const LAZY_GENESIS_ACTIVATION_MARKER_MAGIC: &[u8] = b"TINE-LAZY-GENESIS-ACTIVATION\0";
const LAZY_GENESIS_PROVIDER_INDEX_MAGIC: &[u8] = b"TINE-LAZY-GENESIS-PROVIDER-INDEX\0";
const LAZY_GENESIS_MANIFEST_FILE: &str = "manifest.postcard";
const LAZY_GENESIS_COMMIT_FILE: &str = "commit.postcard";
pub(crate) const LAZY_GENESIS_BASELINE_DIRECTORY: &str = "lazy-genesis";
pub(crate) const LAZY_GENESIS_ACTIVATION_MARKER_FILE: &str = "lazy-genesis.marker";
const MAX_LAZY_GENESIS_MANIFEST_BYTES: usize = 256 * 1024 * 1024;
const LAZY_GENESIS_SEGMENT_TARGET_BYTES: usize = 32 * 1024 * 1024;
const MAX_LAZY_GENESIS_CAPSULE_BYTES: usize = 256 * 1024 * 1024;
const MAX_LAZY_GENESIS_SQLITE_RECEIPT_BYTES: usize = 64 * 1024 * 1024;
/// One page row plus the existing one-million-block parser ceiling.
const MAX_LAZY_GENESIS_SQLITE_RECEIPT_ROWS: usize = 1_000_001;
const MAX_LAZY_GENESIS_CATALOG_CHECKPOINT_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const MAX_LAZY_GENESIS_PROVIDER_INDEX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const LAZY_GENESIS_PROVIDER_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_LAZY_GENESIS_PAGES: usize = 1_000_000;
const MAX_LAZY_GENESIS_BLOCKS: u64 = 100_000_000;

#[cfg(test)]
thread_local! {
    static TEST_RECEIPT_LIMITS: std::cell::Cell<Option<(usize, usize, usize)>> =
        const { std::cell::Cell::new(None) };
}

fn lazy_genesis_receipt_limits() -> (usize, usize, usize) {
    #[cfg(test)]
    if let Some(limits) = TEST_RECEIPT_LIMITS.with(std::cell::Cell::get) {
        return limits;
    }
    (
        MAX_LAZY_GENESIS_SQLITE_RECEIPT_BYTES,
        MAX_LAZY_GENESIS_SQLITE_RECEIPT_ROWS,
        MAX_LAZY_GENESIS_CAPSULE_BYTES,
    )
}

#[cfg(test)]
pub(crate) fn with_lazy_genesis_receipt_limits_for_test<T>(
    receipt_bytes: usize,
    receipt_rows: usize,
    capsule_bytes: usize,
    operation: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RECEIPT_LIMITS.with(|limits| limits.set(None));
        }
    }
    TEST_RECEIPT_LIMITS.with(|limits| {
        assert!(limits.get().is_none(), "lazy-genesis receipt limits nested");
        limits.set(Some((receipt_bytes, receipt_rows, capsule_bytes)));
    });
    let _reset = Reset;
    operation()
}
const LAZY_GENESIS_FRONTIER_BINDING_SCHEMA_VERSION: u32 = 1;
const CLEAN_SHARED_ENROLLMENT_SCHEMA_VERSION: u32 = 1;
const CLEAN_SHARED_STATE_SCHEMA_VERSION: u32 = 1;
const CLEAN_SHARED_ENROLLMENT_MAGIC: &[u8] = b"TINE-CLEAN-SHARED-ENROLLMENT\0";
const CLEAN_SHARED_STATE_MAGIC: &[u8] = b"TINE-CLEAN-SHARED-STATE\0";
pub(crate) const CLEAN_SHARED_STATE_FILE: &str = "lazy-genesis.shared";

/// Provider-visible identity of one clean shared graph.
///
/// The immutable baseline and accepted operation manifests are the durable
/// semantic authority.  This descriptor names that authority; it deliberately
/// carries no legacy enrollment head, promotion proof, Patricia root or
/// projection-work checkpoint.  SQLite remains a device-local derivative.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanSharedEnrollmentDescriptorV1 {
    schema_version: u32,
    oplog_protocol_version: u32,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    baseline_root: ContentDigest,
    baseline_index: BlobDescription,
    source_capture: BlobDescription,
    accepted_frontier_digest: ContentDigest,
    initiator_device_id: DeviceId,
    object_store_namespace: ContentDigest,
}

impl CleanSharedEnrollmentDescriptorV1 {
    pub(crate) fn new(
        marker: LazyGenesisActivationMarkerV1,
        baseline_index: BlobDescription,
        catalog_document_id: DocumentId,
        accepted_frontier_digest: ContentDigest,
        initiator_device_id: DeviceId,
        object_store_namespace: ContentDigest,
    ) -> io::Result<Self> {
        let descriptor = Self {
            schema_version: CLEAN_SHARED_ENROLLMENT_SCHEMA_VERSION,
            oplog_protocol_version: OPLOG_PROTOCOL_VERSION,
            workspace_id: marker.workspace_id(),
            lineage_digest: marker.lineage_digest(),
            catalog_document_id,
            baseline_root: marker.baseline_root(),
            baseline_index,
            source_capture: marker.source_capture(),
            accepted_frontier_digest,
            initiator_device_id,
            object_store_namespace,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let body = postcard::to_allocvec(self).map_err(|error| invalid(error.to_string()))?;
        let mut bytes = Vec::with_capacity(CLEAN_SHARED_ENROLLMENT_MAGIC.len() + body.len());
        bytes.extend_from_slice(CLEAN_SHARED_ENROLLMENT_MAGIC);
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        let body = bytes
            .strip_prefix(CLEAN_SHARED_ENROLLMENT_MAGIC)
            .ok_or_else(|| invalid("clean shared enrollment descriptor has invalid magic"))?;
        let descriptor: Self =
            postcard::from_bytes(body).map_err(|error| invalid(error.to_string()))?;
        descriptor.validate()?;
        if descriptor.encode()? != bytes {
            return Err(invalid(
                "clean shared enrollment descriptor is not canonically encoded",
            ));
        }
        Ok(descriptor)
    }

    pub(crate) fn digest(&self) -> io::Result<ContentDigest> {
        Ok(ContentDigest::of(&self.encode()?))
    }

    fn validate(&self) -> io::Result<()> {
        if self.schema_version != CLEAN_SHARED_ENROLLMENT_SCHEMA_VERSION
            || self.oplog_protocol_version != OPLOG_PROTOCOL_VERSION
            || self.baseline_index.byte_length() == 0
            || self.baseline_index.byte_length() > MAX_LAZY_GENESIS_PROVIDER_INDEX_BYTES as u64
        {
            return Err(invalid(
                "clean shared enrollment descriptor has an unsupported schema or protocol",
            ));
        }
        Ok(())
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn catalog_document_id(&self) -> DocumentId {
        self.catalog_document_id
    }

    pub(crate) const fn baseline_root(&self) -> ContentDigest {
        self.baseline_root
    }

    pub(crate) const fn baseline_index(&self) -> BlobDescription {
        self.baseline_index
    }

    pub(crate) const fn source_capture(&self) -> BlobDescription {
        self.source_capture
    }

    pub(crate) const fn accepted_frontier_digest(&self) -> ContentDigest {
        self.accepted_frontier_digest
    }

    pub(crate) const fn initiator_device_id(&self) -> DeviceId {
        self.initiator_device_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CleanSharedRoleV1 {
    Initiator,
    Joiner,
}

/// The only device-private state added by clean sharing.  It records the
/// descriptor and this device's role; provider ingress and current projection
/// state are reconstructed from the descriptor, accepted manifests and
/// SQLite rather than another lifecycle/history tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanSharedStateV1 {
    schema_version: u32,
    descriptor: CleanSharedEnrollmentDescriptorV1,
    descriptor_digest: ContentDigest,
    role: CleanSharedRoleV1,
}

impl CleanSharedStateV1 {
    pub(crate) fn new(
        descriptor: CleanSharedEnrollmentDescriptorV1,
        role: CleanSharedRoleV1,
    ) -> io::Result<Self> {
        let descriptor_digest = descriptor.digest()?;
        let state = Self {
            schema_version: CLEAN_SHARED_STATE_SCHEMA_VERSION,
            descriptor,
            descriptor_digest,
            role,
        };
        state.validate()?;
        Ok(state)
    }

    fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let body = postcard::to_allocvec(self).map_err(|error| invalid(error.to_string()))?;
        let mut bytes = Vec::with_capacity(CLEAN_SHARED_STATE_MAGIC.len() + body.len());
        bytes.extend_from_slice(CLEAN_SHARED_STATE_MAGIC);
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> io::Result<Self> {
        let body = bytes
            .strip_prefix(CLEAN_SHARED_STATE_MAGIC)
            .ok_or_else(|| invalid("clean shared state has invalid magic"))?;
        let state: Self = postcard::from_bytes(body).map_err(|error| invalid(error.to_string()))?;
        state.validate()?;
        if state.encode()? != bytes {
            return Err(invalid("clean shared state is not canonically encoded"));
        }
        Ok(state)
    }

    fn validate(&self) -> io::Result<()> {
        if self.schema_version != CLEAN_SHARED_STATE_SCHEMA_VERSION
            || self.descriptor.digest()? != self.descriptor_digest
        {
            return Err(invalid("clean shared state is malformed"));
        }
        Ok(())
    }

    pub(crate) fn descriptor(&self) -> &CleanSharedEnrollmentDescriptorV1 {
        &self.descriptor
    }

    pub(crate) const fn role(&self) -> CleanSharedRoleV1 {
        self.role
    }
}

/// Constant-size durable identity of the immutable baseline carried by an
/// accepted frontier. The manifest root transitively binds the source capture,
/// every page capsule/checkpoint, and the catalog checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisFrontierBindingV1 {
    schema_version: u32,
    root: ContentDigest,
    source_capture: BlobDescription,
    document_count: u64,
    block_count: u64,
}

impl LazyGenesisFrontierBindingV1 {
    fn new(candidate: &LazyGenesisCandidate) -> io::Result<Self> {
        let document_count = candidate
            .manifest
            .page_count
            .checked_add(u64::from(candidate.manifest.catalog_dependencies.is_some()))
            .ok_or_else(|| invalid("lazy genesis document count overflowed"))?;
        let binding = Self {
            schema_version: LAZY_GENESIS_FRONTIER_BINDING_SCHEMA_VERSION,
            root: candidate.root,
            source_capture: candidate.manifest.source_capture,
            document_count,
            block_count: candidate.manifest.block_count,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub(crate) fn validate(self) -> io::Result<()> {
        if self.schema_version != LAZY_GENESIS_FRONTIER_BINDING_SCHEMA_VERSION {
            return Err(invalid("lazy genesis frontier binding is malformed"));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) const fn root(self) -> ContentDigest {
        self.root
    }

    pub(crate) const fn document_count(self) -> u64 {
        self.document_count
    }

    pub(crate) const fn block_count(self) -> u64 {
        self.block_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisBlockInput {
    pub(crate) block_id: BlockId,
    pub(crate) home_document_id: DocumentId,
    pub(crate) parent: Option<BlockId>,
    pub(crate) order: String,
    pub(crate) content: String,
    pub(crate) external_uuid_claims: Vec<LogseqUuid>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisPageInput {
    pub(crate) source_leaf: [u8; 32],
    pub(crate) exact_source_bytes: Vec<u8>,
    pub(crate) page_id: PageId,
    pub(crate) home_document_id: DocumentId,
    pub(crate) name: String,
    pub(crate) path: ManagedPath,
    pub(crate) kind: ManagedTextKind,
    pub(crate) preamble: Option<String>,
    pub(crate) blocks: Vec<LazyGenesisBlockInput>,
    pub(crate) document_checkpoint: Vec<u8>,
    pub(crate) document_dependencies: Option<DocumentDependencies>,
    pub(crate) sqlite_receipt: Option<LazyGenesisSqliteReceiptV1>,
}

/// Bounded, disposable-projection handoff carried by the authenticated
/// baseline capsule. It is not independent authority: invalid, absent, or
/// parser-stale receipts fall back to reparsing the capsule's exact bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisSqliteReceiptV1 {
    schema_version: u32,
    parser_schema_version: u32,
    exact_source_digest: ContentDigest,
    semantic_digest: ContentDigest,
    row_count: u32,
    /// Opaque until both receipt and parser schema versions match. Keeping the
    /// parser-owned codec behind bytes lets a future binary ignore an old
    /// receipt and reach the receiptless parser fallback.
    payload: Vec<u8>,
}

impl LazyGenesisSqliteReceiptV1 {
    pub(crate) fn new(
        exact_source_bytes: &[u8],
        payload: &super::MaterializedPageInput,
    ) -> io::Result<Option<Self>> {
        let rows = payload.blocks.len().saturating_add(1);
        if !sqlite_receipt_within_bounds(0, rows) {
            return Ok(None);
        }
        let payload = postcard::to_allocvec(payload).map_err(|error| invalid(error.to_string()))?;
        let semantic_digest = sqlite_receipt_semantic_digest(&payload);
        let receipt = Self {
            schema_version: LAZY_GENESIS_SQLITE_RECEIPT_SCHEMA_VERSION,
            parser_schema_version: LAZY_GENESIS_PARSER_SCHEMA_VERSION,
            exact_source_digest: ContentDigest::of(exact_source_bytes),
            semantic_digest,
            row_count: rows as u32,
            payload,
        };
        if !sqlite_receipt_within_bounds(
            postcard::to_allocvec(&receipt)
                .map_err(|error| invalid(error.to_string()))?
                .len(),
            rows,
        ) {
            return Ok(None);
        }
        Ok(Some(receipt))
    }

    pub(crate) fn verified_payload(
        &self,
        page: &LazyGenesisPageInput,
    ) -> io::Result<Option<super::MaterializedPageInput>> {
        if self.schema_version != LAZY_GENESIS_SQLITE_RECEIPT_SCHEMA_VERSION
            || self.parser_schema_version != LAZY_GENESIS_PARSER_SCHEMA_VERSION
        {
            return Ok(None);
        }
        let encoded_len = match postcard::to_allocvec(self) {
            Ok(bytes) => bytes.len(),
            Err(_) => return Ok(None),
        };
        if !sqlite_receipt_within_bounds(encoded_len, self.row_count as usize)
            || self.exact_source_digest != ContentDigest::of(&page.exact_source_bytes)
            || self.semantic_digest != sqlite_receipt_semantic_digest(&self.payload)
        {
            return Ok(None);
        }
        let payload: super::MaterializedPageInput = match postcard::from_bytes(&self.payload) {
            Ok(payload) => payload,
            Err(_) => return Ok(None),
        };
        if payload.blocks.len().saturating_add(1) != self.row_count as usize
            || !sqlite_receipt_matches_capsule(&payload, page)
        {
            return Ok(None);
        }
        Ok(Some(payload))
    }

    #[cfg(test)]
    pub(crate) fn corrupt_semantic_digest_for_test(&mut self) {
        self.semantic_digest = ContentDigest::of(b"corrupt lazy-genesis sqlite receipt");
    }

    #[cfg(test)]
    pub(crate) fn mark_parser_stale_for_test(&mut self) {
        self.parser_schema_version = self.parser_schema_version.saturating_add(1);
    }

    #[cfg(test)]
    pub(crate) const fn semantic_digest_for_test(&self) -> ContentDigest {
        self.semantic_digest
    }

    #[cfg(test)]
    pub(crate) const fn parser_schema_version_for_test(&self) -> u32 {
        self.parser_schema_version
    }
}

fn sqlite_receipt_semantic_digest(payload: &[u8]) -> ContentDigest {
    let mut bytes = Vec::with_capacity(35 + payload.len());
    bytes.extend_from_slice(b"tine/lazy-genesis/sqlite-receipt/v1\0");
    bytes.extend_from_slice(payload);
    ContentDigest::of(&bytes)
}

fn sqlite_receipt_matches_capsule(
    payload: &super::MaterializedPageInput,
    page: &LazyGenesisPageInput,
) -> bool {
    // The capsule independently binds page/block identity, topology, source
    // content, and UUID claims. Parser-derived facets such as search text,
    // properties, tags, task state, headings, and references are instead
    // guarded by LAZY_GENESIS_PARSER_SCHEMA_VERSION and its digest tripwire.
    payload.page_id == page.page_id
        && payload.home_document_id == page.home_document_id
        && payload.name == page.name
        && payload.path == page.path
        && payload.kind == page.kind
        && payload.preamble == page.preamble
        && payload.blocks.len() == page.blocks.len()
        && payload
            .blocks
            .iter()
            .zip(&page.blocks)
            .all(|(payload, stored)| {
                let expected_uuid = if stored.external_uuid_claims.len() == 1 {
                    Some(stored.external_uuid_claims[0])
                } else {
                    None
                };
                payload.block_id == stored.block_id
                    && payload.home_document_id == stored.home_document_id
                    && payload.parent == stored.parent
                    && payload.order == stored.order
                    && payload.content == stored.content
                    && payload.logseq_uuid == expected_uuid
            })
}

fn sqlite_receipt_within_bounds(encoded_bytes: usize, rows: usize) -> bool {
    let (max_bytes, max_rows, _) = lazy_genesis_receipt_limits();
    encoded_bytes <= max_bytes && rows <= max_rows
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LazyGenesisPageCapsuleV1 {
    schema_version: u32,
    source_leaf: [u8; 32],
    exact_source_bytes: Vec<u8>,
    page_id: PageId,
    home_document_id: DocumentId,
    name: String,
    path: ManagedPath,
    kind: ManagedTextKind,
    preamble: Option<String>,
    blocks: Vec<LazyGenesisBlockInput>,
    document_checkpoint: Vec<u8>,
    #[serde(default)]
    sqlite_receipt: Option<LazyGenesisSqliteReceiptV1>,
}

impl LazyGenesisPageCapsuleV1 {
    fn from_input(input: LazyGenesisPageInput) -> io::Result<Self> {
        let capsule = Self {
            schema_version: LAZY_GENESIS_PAGE_CAPSULE_SCHEMA_VERSION,
            source_leaf: input.source_leaf,
            exact_source_bytes: input.exact_source_bytes,
            page_id: input.page_id,
            home_document_id: input.home_document_id,
            name: input.name,
            path: input.path,
            kind: input.kind,
            preamble: input.preamble,
            blocks: input.blocks,
            document_checkpoint: input.document_checkpoint,
            sqlite_receipt: input.sqlite_receipt,
        };
        let mut capsule = capsule;
        if capsule.sqlite_receipt.is_some()
            && postcard::to_allocvec(&capsule)
                .map_err(|error| invalid(error.to_string()))?
                .len()
                > lazy_genesis_receipt_limits().2
        {
            capsule.sqlite_receipt = None;
        }
        capsule.validate()?;
        Ok(capsule)
    }

    fn validate(&self) -> io::Result<()> {
        if self.schema_version != LAZY_GENESIS_PAGE_CAPSULE_SCHEMA_VERSION
            || self.name.is_empty()
            || self.document_checkpoint.is_empty()
            || self.exact_source_bytes.len() > MAX_LAZY_GENESIS_CAPSULE_BYTES
        {
            return Err(invalid("lazy genesis page capsule has an invalid header"));
        }
        let mut block_ids = BTreeSet::new();
        for block in &self.blocks {
            if block.home_document_id != self.home_document_id
                || !block_ids.insert(block.block_id)
                || block.parent.is_some_and(|parent| parent == block.block_id)
            {
                return Err(invalid(
                    "lazy genesis page capsule has invalid block identity",
                ));
            }
            if block
                .external_uuid_claims
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid(
                    "lazy genesis external UUID claims are not canonical",
                ));
            }
        }
        if self
            .blocks
            .iter()
            .filter_map(|block| block.parent)
            .any(|parent| !block_ids.contains(&parent))
        {
            return Err(invalid("lazy genesis block parent is outside its page"));
        }
        Ok(())
    }

    fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = postcard::to_allocvec(self).map_err(|error| invalid(error.to_string()))?;
        if bytes.len() > MAX_LAZY_GENESIS_CAPSULE_BYTES {
            return Err(invalid("lazy genesis page capsule exceeds its fixed cap"));
        }
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_LAZY_GENESIS_CAPSULE_BYTES {
            return Err(invalid("lazy genesis page capsule exceeds its fixed cap"));
        }
        let capsule: Self =
            postcard::from_bytes(bytes).map_err(|error| invalid(error.to_string()))?;
        capsule.validate()?;
        Ok(capsule)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LazyGenesisPageDescriptorV1 {
    page_id: PageId,
    home_document_id: DocumentId,
    path: ManagedPath,
    capsule: BlobDescription,
    segment: u32,
    offset: u64,
    length: u64,
    blocks: u32,
    document_dependencies: DocumentDependencies,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LazyGenesisManifestV1 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    source_capture: BlobDescription,
    catalog_checkpoint: BlobDescription,
    catalog_dependencies: Option<DocumentDependencies>,
    pages: Vec<LazyGenesisPageDescriptorV1>,
    segments: Vec<BlobDescription>,
    page_count: u64,
    block_count: u64,
}

/// Small provider-visible inventory for one immutable lazy-genesis pack.
///
/// Large pack files are transported as independently exact bounded chunks;
/// this record binds their complete byte descriptions without duplicating the
/// page catalog carried by the manifest itself. The clean shared descriptor
/// binds this index before it advertises a joinable graph.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisProviderIndexV1 {
    schema_version: u32,
    baseline_root: ContentDigest,
    manifest: BlobDescription,
    commit: BlobDescription,
    catalog: BlobDescription,
    segments: Vec<BlobDescription>,
    chunk_bytes: u32,
}

impl LazyGenesisProviderIndexV1 {
    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_LAZY_GENESIS_PROVIDER_INDEX_BYTES {
            return Err(invalid("lazy genesis provider index exceeds its fixed cap"));
        }
        let body = bytes
            .strip_prefix(LAZY_GENESIS_PROVIDER_INDEX_MAGIC)
            .ok_or_else(|| invalid("lazy genesis provider index has invalid magic"))?;
        let index: Self = postcard::from_bytes(body).map_err(|error| invalid(error.to_string()))?;
        index.validate()?;
        if index.encode()? != bytes {
            return Err(invalid(
                "lazy genesis provider index is not canonically encoded",
            ));
        }
        Ok(index)
    }

    pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let body = postcard::to_allocvec(self).map_err(|error| invalid(error.to_string()))?;
        let mut bytes = Vec::with_capacity(LAZY_GENESIS_PROVIDER_INDEX_MAGIC.len() + body.len());
        bytes.extend_from_slice(LAZY_GENESIS_PROVIDER_INDEX_MAGIC);
        bytes.extend_from_slice(&body);
        if bytes.len() > MAX_LAZY_GENESIS_PROVIDER_INDEX_BYTES {
            return Err(invalid("lazy genesis provider index exceeds its fixed cap"));
        }
        Ok(bytes)
    }

    fn validate(&self) -> io::Result<()> {
        if self.schema_version != LAZY_GENESIS_PROVIDER_INDEX_SCHEMA_VERSION
            || self.chunk_bytes as usize != LAZY_GENESIS_PROVIDER_CHUNK_BYTES
            || self.manifest.byte_length() == 0
            || self.manifest.byte_length() > MAX_LAZY_GENESIS_MANIFEST_BYTES as u64
            || self.commit.byte_length() == 0
            || self.commit.byte_length() > 1024
            || self.catalog.byte_length() == 0
            || self.catalog.byte_length() > MAX_LAZY_GENESIS_CATALOG_CHECKPOINT_BYTES as u64
            || self.segments.len() > MAX_LAZY_GENESIS_PAGES
            || self.segments.iter().any(|segment| {
                segment.byte_length() == 0
                    || segment.byte_length()
                        > (MAX_LAZY_GENESIS_CAPSULE_BYTES as u64).saturating_add(8)
            })
        {
            return Err(invalid("lazy genesis provider index is malformed"));
        }
        Ok(())
    }

    pub(crate) const fn baseline_root(&self) -> ContentDigest {
        self.baseline_root
    }

    pub(crate) fn file_descriptions(&self) -> Vec<(String, BlobDescription)> {
        let mut files = Vec::with_capacity(self.segments.len().saturating_add(3));
        files.push((LAZY_GENESIS_MANIFEST_FILE.into(), self.manifest));
        files.push((LAZY_GENESIS_COMMIT_FILE.into(), self.commit));
        files.push(("catalog.snapshot".into(), self.catalog));
        files.extend(
            self.segments
                .iter()
                .enumerate()
                .map(|(index, description)| (format!("segment-{index:08}.pack"), *description)),
        );
        files
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisCommitV1 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    source_capture: BlobDescription,
    manifest: BlobDescription,
    root: ContentDigest,
}

/// The sole durable authority transition for the next activation generation.
///
/// The Tauri binding records the user's intent to use managed storage; it is
/// not this marker and cannot make a partially built baseline authoritative.
/// This record is published only after the baseline and its accepted frontier
/// are durable and a final byte-exact source observation has completed. SQLite
/// is deliberately absent because it is a disposable projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LazyGenesisActivationMarkerV1 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    generation: u64,
    baseline_root: ContentDigest,
    source_capture: BlobDescription,
    accepted_frontier_digest: ContentDigest,
    watcher_fence: u64,
}

impl LazyGenesisActivationMarkerV1 {
    pub(crate) fn new(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        baseline_root: ContentDigest,
        source_capture: BlobDescription,
        accepted_frontier_digest: ContentDigest,
        watcher_fence: u64,
    ) -> io::Result<Self> {
        Self::new_for_generation(
            workspace_id,
            lineage_digest,
            0,
            baseline_root,
            source_capture,
            accepted_frontier_digest,
            watcher_fence,
        )
    }

    pub(crate) fn new_for_generation(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        generation: u64,
        baseline_root: ContentDigest,
        source_capture: BlobDescription,
        accepted_frontier_digest: ContentDigest,
        watcher_fence: u64,
    ) -> io::Result<Self> {
        let marker = Self {
            schema_version: LAZY_GENESIS_ACTIVATION_MARKER_SCHEMA_VERSION,
            workspace_id,
            lineage_digest,
            generation,
            baseline_root,
            source_capture,
            accepted_frontier_digest,
            watcher_fence,
        };
        marker.validate()?;
        Ok(marker)
    }

    pub(crate) fn encode(self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let body = postcard::to_allocvec(&self).map_err(|error| invalid(error.to_string()))?;
        let mut bytes = Vec::with_capacity(LAZY_GENESIS_ACTIVATION_MARKER_MAGIC.len() + body.len());
        bytes.extend_from_slice(LAZY_GENESIS_ACTIVATION_MARKER_MAGIC);
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        let body = bytes
            .strip_prefix(LAZY_GENESIS_ACTIVATION_MARKER_MAGIC)
            .ok_or_else(|| invalid("lazy genesis activation marker has invalid magic"))?;
        let marker: Self =
            postcard::from_bytes(body).map_err(|error| invalid(error.to_string()))?;
        marker.validate()?;
        if marker.encode()? != bytes {
            return Err(invalid(
                "lazy genesis activation marker is not canonically encoded",
            ));
        }
        Ok(marker)
    }

    fn validate(self) -> io::Result<()> {
        if self.schema_version != LAZY_GENESIS_ACTIVATION_MARKER_SCHEMA_VERSION {
            return Err(invalid(
                "lazy genesis activation marker has an unsupported schema",
            ));
        }
        Ok(())
    }

    pub(crate) const fn workspace_id(self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) const fn baseline_root(self) -> ContentDigest {
        self.baseline_root
    }

    pub(crate) const fn source_capture(self) -> BlobDescription {
        self.source_capture
    }

    pub(crate) const fn accepted_frontier_digest(self) -> ContentDigest {
        self.accepted_frontier_digest
    }

    #[cfg(test)]
    pub(crate) const fn watcher_fence(self) -> u64 {
        self.watcher_fence
    }
}

impl LazyGenesisCommitV1 {
    pub(crate) const fn root(self) -> ContentDigest {
        self.root
    }

    fn for_manifest(manifest: &LazyGenesisManifestV1, bytes: &[u8]) -> Self {
        Self {
            schema_version: LAZY_GENESIS_COMMIT_SCHEMA_VERSION,
            workspace_id: manifest.workspace_id,
            lineage_digest: manifest.lineage_digest,
            source_capture: manifest.source_capture,
            manifest: BlobDescription::of(bytes),
            root: lazy_genesis_manifest_root(bytes),
        }
    }

    fn encode(self) -> io::Result<Vec<u8>> {
        postcard::to_allocvec(&self).map_err(|error| invalid(error.to_string()))
    }

    fn decode(bytes: &[u8]) -> io::Result<Self> {
        let commit: Self =
            postcard::from_bytes(bytes).map_err(|error| invalid(error.to_string()))?;
        if commit.schema_version != LAZY_GENESIS_COMMIT_SCHEMA_VERSION || commit.encode()? != bytes
        {
            return Err(invalid("lazy genesis commit is malformed or non-canonical"));
        }
        Ok(commit)
    }
}

pub(crate) struct LazyGenesisPackBuilder {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    source_capture: BlobDescription,
    scratch: PathBuf,
    current: Vec<u8>,
    descriptors: Vec<LazyGenesisPageDescriptorV1>,
    segments: Vec<BlobDescription>,
    block_count: u64,
    seen_pages: BTreeSet<PageId>,
    seen_homes: BTreeSet<DocumentId>,
    seen_paths: BTreeSet<ManagedPath>,
    last_path: Option<ManagedPath>,
}

impl LazyGenesisPackBuilder {
    pub(crate) fn new(
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
        source_capture: BlobDescription,
        scratch_parent: &Path,
    ) -> io::Result<Self> {
        let scratch = scratch_parent.join(format!("tine-lazy-genesis-{}", Uuid::new_v4().simple()));
        fs::create_dir(&scratch)?;
        Ok(Self {
            workspace_id,
            lineage_digest,
            catalog_document_id,
            source_capture,
            scratch,
            current: Vec::with_capacity(LAZY_GENESIS_SEGMENT_TARGET_BYTES),
            descriptors: Vec::new(),
            segments: Vec::new(),
            block_count: 0,
            seen_pages: BTreeSet::new(),
            seen_homes: BTreeSet::new(),
            seen_paths: BTreeSet::new(),
            last_path: None,
        })
    }

    pub(crate) fn push(&mut self, input: LazyGenesisPageInput) -> io::Result<()> {
        if self.descriptors.len() == MAX_LAZY_GENESIS_PAGES {
            return Err(invalid("lazy genesis page-count cap exceeded"));
        }
        let document_dependencies = input
            .document_dependencies
            .clone()
            .ok_or_else(|| invalid("lazy genesis page has no sealed causal dependencies"))?;
        let capsule = LazyGenesisPageCapsuleV1::from_input(input)?;
        if document_dependencies.document_id() != capsule.home_document_id
            || !document_dependencies.direct_dependency_heads().is_empty()
        {
            return Err(invalid(
                "lazy genesis page dependencies do not name an unheaded home document",
            ));
        }
        if self
            .last_path
            .as_ref()
            .is_some_and(|last_path| last_path >= &capsule.path)
        {
            return Err(invalid(
                "lazy genesis pages are not in canonical path order",
            ));
        }
        if !self.seen_pages.insert(capsule.page_id)
            || !self.seen_homes.insert(capsule.home_document_id)
            || !self.seen_paths.insert(capsule.path.clone())
        {
            return Err(invalid("lazy genesis repeats page, home, or path identity"));
        }
        self.last_path = Some(capsule.path.clone());
        self.block_count = self
            .block_count
            .checked_add(capsule.blocks.len() as u64)
            .filter(|count| *count <= MAX_LAZY_GENESIS_BLOCKS)
            .ok_or_else(|| invalid("lazy genesis block-count cap exceeded"))?;
        let encoded = capsule.encode()?;
        let frame_bytes = 8_usize
            .checked_add(encoded.len())
            .ok_or_else(|| invalid("lazy genesis frame length overflow"))?;
        if !self.current.is_empty()
            && self.current.len().saturating_add(frame_bytes) > LAZY_GENESIS_SEGMENT_TARGET_BYTES
        {
            self.flush_segment()?;
        }
        let segment = u32::try_from(self.segments.len())
            .map_err(|_| invalid("lazy genesis segment count overflow"))?;
        let offset = self.current.len() as u64 + 8;
        self.current
            .extend_from_slice(&(encoded.len() as u64).to_be_bytes());
        self.current.extend_from_slice(&encoded);
        self.descriptors.push(LazyGenesisPageDescriptorV1 {
            page_id: capsule.page_id,
            home_document_id: capsule.home_document_id,
            path: capsule.path,
            capsule: BlobDescription::of(&encoded),
            segment,
            offset,
            length: encoded.len() as u64,
            blocks: capsule.blocks.len() as u32,
            document_dependencies,
        });
        if self.current.len() >= LAZY_GENESIS_SEGMENT_TARGET_BYTES {
            self.flush_segment()?;
        }
        Ok(())
    }

    fn flush_segment(&mut self) -> io::Result<()> {
        if self.current.is_empty() {
            return Ok(());
        }
        let description = BlobDescription::of(&self.current);
        let path = segment_path(&self.scratch, self.segments.len());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&self.current)?;
        self.current.clear();
        self.segments.push(description);
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        catalog_checkpoint: Vec<u8>,
        catalog_dependencies: Option<DocumentDependencies>,
    ) -> io::Result<LazyGenesisCandidate> {
        if catalog_checkpoint.is_empty()
            || catalog_checkpoint.len() > MAX_LAZY_GENESIS_CATALOG_CHECKPOINT_BYTES
        {
            return Err(invalid(
                "lazy genesis catalog checkpoint is empty or exceeds its fixed cap",
            ));
        }
        if catalog_dependencies.as_ref().is_some_and(|dependencies| {
            dependencies.document_id() != self.catalog_document_id
                || !dependencies.direct_dependency_heads().is_empty()
                || self
                    .descriptors
                    .iter()
                    .any(|page| page.home_document_id == dependencies.document_id())
        }) {
            return Err(invalid(
                "lazy genesis catalog dependencies are headed or alias a page home",
            ));
        }
        self.flush_segment()?;
        let catalog_description = BlobDescription::of(&catalog_checkpoint);
        let catalog_path = self.scratch.join("catalog.snapshot");
        let mut catalog_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(catalog_path)?;
        catalog_file.write_all(&catalog_checkpoint)?;
        let manifest = LazyGenesisManifestV1 {
            schema_version: LAZY_GENESIS_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            lineage_digest: self.lineage_digest,
            catalog_document_id: self.catalog_document_id,
            source_capture: self.source_capture,
            catalog_checkpoint: catalog_description,
            catalog_dependencies,
            page_count: self.descriptors.len() as u64,
            block_count: self.block_count,
            pages: std::mem::take(&mut self.descriptors),
            segments: self.segments.clone(),
        };
        validate_manifest(&manifest)?;
        let manifest_bytes =
            postcard::to_allocvec(&manifest).map_err(|error| invalid(error.to_string()))?;
        let root = lazy_genesis_manifest_root(&manifest_bytes);
        let index = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.page_id, index))
            .collect();
        let home_index = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.home_document_id, index))
            .collect();
        let scratch = std::mem::take(&mut self.scratch);
        let segment_seals = SegmentSealMemo::new(manifest.segments.len());
        Ok(LazyGenesisCandidate {
            scratch,
            manifest,
            manifest_bytes,
            root,
            index,
            home_index,
            cleanup_on_drop: true,
            segment_seals,
        })
    }
}

impl Drop for LazyGenesisPackBuilder {
    fn drop(&mut self) {
        if !self.scratch.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.scratch);
        }
    }
}

pub(crate) struct LazyGenesisCandidate {
    scratch: PathBuf,
    manifest: LazyGenesisManifestV1,
    manifest_bytes: Vec<u8>,
    root: ContentDigest,
    index: BTreeMap<PageId, usize>,
    home_index: BTreeMap<DocumentId, usize>,
    cleanup_on_drop: bool,
    /// Which sealed segment packs this candidate has already proved against
    /// the manifest's segment digests, and how many such whole-pack proofs it
    /// has run. A sealed segment is written once and never rewritten, so
    /// re-hashing the entire pack for every page read costs
    /// `O(pages × segment bytes)` and proves nothing the first proof did not.
    /// Every page still verifies its own capsule bytes against the descriptor
    /// digest on every read, so localized corruption of the bytes actually
    /// returned is caught regardless of this memo.
    segment_seals: SegmentSealMemo,
}

/// One-shot per-segment seal proofs for a sealed lazy-genesis pack.
#[derive(Debug, Default)]
struct SegmentSealMemo {
    proved: Vec<AtomicBool>,
    proofs: AtomicUsize,
    #[cfg(test)]
    page_decodes: AtomicUsize,
}

impl SegmentSealMemo {
    fn new(segments: usize) -> Self {
        Self {
            proved: (0..segments).map(|_| AtomicBool::new(false)).collect(),
            proofs: AtomicUsize::new(0),
            #[cfg(test)]
            page_decodes: AtomicUsize::new(0),
        }
    }
}

impl LazyGenesisCandidate {
    pub(crate) const fn root(&self) -> ContentDigest {
        self.root
    }

    pub(crate) fn frontier_binding(&self) -> io::Result<LazyGenesisFrontierBindingV1> {
        LazyGenesisFrontierBindingV1::new(self)
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.manifest.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.manifest.lineage_digest
    }

    pub(crate) const fn source_capture(&self) -> BlobDescription {
        self.manifest.source_capture
    }

    pub(crate) const fn catalog_document_id(&self) -> DocumentId {
        self.manifest.catalog_document_id
    }

    pub(crate) fn provider_index(&self) -> io::Result<LazyGenesisProviderIndexV1> {
        let commit_bytes = read_bounded(&self.scratch.join(LAZY_GENESIS_COMMIT_FILE), 1024)?;
        let commit = LazyGenesisCommitV1::decode(&commit_bytes)?;
        if commit.root() != self.root
            || commit.manifest != BlobDescription::of(&self.manifest_bytes)
            || commit.workspace_id != self.manifest.workspace_id
            || commit.lineage_digest != self.manifest.lineage_digest
            || commit.source_capture != self.manifest.source_capture
        {
            return Err(invalid(
                "lazy genesis sealed files no longer match their provider inventory",
            ));
        }
        let index = LazyGenesisProviderIndexV1 {
            schema_version: LAZY_GENESIS_PROVIDER_INDEX_SCHEMA_VERSION,
            baseline_root: self.root,
            manifest: BlobDescription::of(&self.manifest_bytes),
            commit: BlobDescription::of(&commit_bytes),
            catalog: self.manifest.catalog_checkpoint,
            segments: self.manifest.segments.clone(),
            chunk_bytes: LAZY_GENESIS_PROVIDER_CHUNK_BYTES as u32,
        };
        index.validate()?;
        Ok(index)
    }

    pub(crate) fn read_provider_file(&self, name: &str) -> io::Result<Vec<u8>> {
        let index = self.provider_index()?;
        let expected = index
            .file_descriptions()
            .into_iter()
            .find_map(|(candidate, description)| (candidate == name).then_some(description))
            .ok_or_else(|| invalid("lazy genesis provider requested an unknown pack file"))?;
        let limit = usize::try_from(expected.byte_length())
            .map_err(|_| invalid("lazy genesis provider file length is not addressable"))?;
        let bytes = read_bounded(&self.scratch.join(name), limit)?;
        if BlobDescription::of(&bytes) != expected {
            return Err(invalid("lazy genesis provider pack file changed"));
        }
        Ok(bytes)
    }

    pub(crate) fn page_count(&self) -> usize {
        self.manifest.pages.len()
    }

    pub(crate) fn page_ids(&self) -> impl Iterator<Item = PageId> + '_ {
        self.manifest.pages.iter().map(|page| page.page_id)
    }

    /// The complete causal baseline bound by the sealed manifest. These rows
    /// are small dependency records, not eagerly opened CRDT documents.
    pub(crate) fn frontier_documents(&self) -> Vec<DocumentDependencies> {
        let mut documents = Vec::with_capacity(self.manifest.pages.len() + 1);
        documents.extend(self.manifest.catalog_dependencies.iter().cloned());
        documents.extend(
            self.manifest
                .pages
                .iter()
                .map(|page| page.document_dependencies.clone()),
        );
        documents.sort_unstable_by_key(DocumentDependencies::document_id);
        documents
    }

    /// Resolve one causal baseline row without opening a CRDT checkpoint or a
    /// page capsule. Ordinary accepted events keep only rows that supersede
    /// this immutable baseline.
    pub(crate) fn frontier_document(
        &self,
        document_id: DocumentId,
    ) -> Option<DocumentDependencies> {
        if let Some(dependencies) = &self.manifest.catalog_dependencies {
            if dependencies.document_id() == document_id {
                return Some(dependencies.clone());
            }
        }
        self.home_index
            .get(&document_id)
            .map(|index| self.manifest.pages[*index].document_dependencies.clone())
    }

    /// Resolve immutable page ownership without decoding its capsule or the
    /// graph-wide catalog checkpoint. The sealed manifest has already proved
    /// uniqueness of both page and home identities.
    pub(crate) fn page_home_document_id(&self, page_id: PageId) -> Option<DocumentId> {
        self.index
            .get(&page_id)
            .map(|index| self.manifest.pages[*index].home_document_id)
    }

    /// Prove one sealed segment pack against its manifest digest at most once
    /// per candidate lifetime and return its path.
    ///
    /// The in-scope failure this defends against is a damaged sealed pack —
    /// a truncated write, a crash between write and fsync, or a disk error —
    /// not an adversary rewriting private storage. That failure is a property
    /// of the file as it was opened, so one proof per pack answers it; running
    /// the same proof once per page turns a linear baseline read into
    /// `O(pages × segment bytes)`. `page` still checks every capsule against
    /// its own descriptor digest, so damage to the bytes a caller actually
    /// receives is rejected on every read.
    fn prove_segment_seal(&self, segment: usize) -> io::Result<PathBuf> {
        let expected = *self
            .manifest
            .segments
            .get(segment)
            .ok_or_else(|| invalid("lazy genesis descriptor names a missing segment"))?;
        let path = segment_path(&self.scratch, segment);
        let proved = self
            .segment_seals
            .proved
            .get(segment)
            .ok_or_else(|| invalid("lazy genesis descriptor names a missing segment"))?;
        if proved.load(Ordering::Acquire) {
            return Ok(path);
        }
        if describe_file(&path)? != expected {
            return Err(invalid("lazy genesis segment bytes changed"));
        }
        self.segment_seals.proofs.fetch_add(1, Ordering::Relaxed);
        proved.store(true, Ordering::Release);
        Ok(path)
    }

    /// How many whole-pack seal proofs this candidate has run. A sealed pack
    /// is immutable, so this must not grow with the number of pages read.
    #[cfg(test)]
    pub(crate) fn segment_seal_proofs(&self) -> usize {
        self.segment_seals.proofs.load(Ordering::Relaxed)
    }

    pub(crate) fn page(&self, page_id: PageId) -> io::Result<Option<LazyGenesisPageInput>> {
        let Some(&index) = self.index.get(&page_id) else {
            return Ok(None);
        };
        let descriptor = &self.manifest.pages[index];
        let path = self.prove_segment_seal(descriptor.segment as usize)?;
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(descriptor.offset))?;
        let mut bytes = vec![0_u8; descriptor.length as usize];
        file.read_exact(&mut bytes)?;
        if BlobDescription::of(&bytes) != descriptor.capsule {
            return Err(invalid("lazy genesis page capsule bytes changed"));
        }
        #[cfg(test)]
        self.segment_seals
            .page_decodes
            .fetch_add(1, Ordering::Relaxed);
        let capsule = LazyGenesisPageCapsuleV1::decode(&bytes)?;
        if capsule.page_id != descriptor.page_id
            || capsule.home_document_id != descriptor.home_document_id
            || capsule.path != descriptor.path
            || capsule.blocks.len() as u32 != descriptor.blocks
        {
            return Err(invalid("lazy genesis descriptor and capsule disagree"));
        }
        Ok(Some(LazyGenesisPageInput {
            source_leaf: capsule.source_leaf,
            exact_source_bytes: capsule.exact_source_bytes,
            page_id: capsule.page_id,
            home_document_id: capsule.home_document_id,
            name: capsule.name,
            path: capsule.path,
            kind: capsule.kind,
            preamble: capsule.preamble,
            blocks: capsule.blocks,
            document_checkpoint: capsule.document_checkpoint,
            document_dependencies: Some(descriptor.document_dependencies.clone()),
            sqlite_receipt: capsule.sqlite_receipt,
        }))
    }

    /// How many page capsules this candidate has actually sent through the
    /// production decoder. The counter belongs to the opened candidate, so
    /// parallel tests and unrelated engines cannot contribute to it.
    #[cfg(test)]
    pub(crate) fn page_capsule_decodes(&self) -> usize {
        self.segment_seals.page_decodes.load(Ordering::Relaxed)
    }

    pub(crate) fn catalog_checkpoint(&self) -> io::Result<Vec<u8>> {
        let bytes = fs::read(self.scratch.join("catalog.snapshot"))?;
        if bytes.len() > MAX_LAZY_GENESIS_CATALOG_CHECKPOINT_BYTES
            || BlobDescription::of(&bytes) != self.manifest.catalog_checkpoint
        {
            return Err(invalid("lazy genesis catalog checkpoint bytes changed"));
        }
        Ok(bytes)
    }

    pub(crate) fn document_checkpoint(
        &self,
        document_id: DocumentId,
    ) -> io::Result<Option<Vec<u8>>> {
        let Some(&index) = self.home_index.get(&document_id) else {
            return Ok(None);
        };
        let page_id = self.manifest.pages[index].page_id;
        Ok(self.page(page_id)?.map(|page| page.document_checkpoint))
    }

    pub(crate) fn stage_into(
        mut self,
        destination: &Path,
    ) -> io::Result<(Self, LazyGenesisCommitV1)> {
        if destination.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "lazy genesis destination already exists",
            ));
        }
        if self.manifest_bytes.len() > MAX_LAZY_GENESIS_MANIFEST_BYTES {
            return Err(invalid("lazy genesis manifest exceeds its fixed cap"));
        }
        let commit = LazyGenesisCommitV1::for_manifest(&self.manifest, &self.manifest_bytes);
        if commit.root() != self.root {
            return Err(invalid(
                "lazy genesis candidate root changed before staging",
            ));
        }
        write_new(
            &self.scratch.join(LAZY_GENESIS_MANIFEST_FILE),
            &self.manifest_bytes,
        )?;
        write_new(
            &self.scratch.join(LAZY_GENESIS_COMMIT_FILE),
            &commit.encode()?,
        )?;
        fs::rename(&self.scratch, destination)?;
        self.scratch = destination.to_path_buf();
        Ok((self, commit))
    }

    /// Publish and flush the immutable pack while it is still disposable.
    /// The returned candidate deliberately retains cleanup ownership: only a
    /// successfully published activation marker may convert it into authority.
    pub(crate) fn publish_durable(
        self,
        destination: &Path,
    ) -> io::Result<(Self, LazyGenesisCommitV1)> {
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("lazy genesis destination has no parent"))?;
        let (candidate, commit) = self.stage_into(destination)?;
        crate::filesystem_durability::sync_private_tree(destination)?;
        crate::filesystem_durability::sync_reconstructible_directory_path(parent)?;
        Ok((candidate, commit))
    }

    /// Transfer cleanup ownership to the durable activation marker. Calling
    /// this before marker publication would strand Direct Files authority.
    pub(crate) fn retain_as_authoritative(mut self) -> Self {
        self.cleanup_on_drop = false;
        self
    }

    pub(crate) fn open_sealed(directory: &Path, expected: LazyGenesisCommitV1) -> io::Result<Self> {
        let manifest_bytes = read_bounded(
            &directory.join(LAZY_GENESIS_MANIFEST_FILE),
            MAX_LAZY_GENESIS_MANIFEST_BYTES,
        )?;
        let commit_bytes = read_bounded(&directory.join(LAZY_GENESIS_COMMIT_FILE), 1024)?;
        let commit = LazyGenesisCommitV1::decode(&commit_bytes)?;
        if commit != expected
            || commit.manifest != BlobDescription::of(&manifest_bytes)
            || commit.root != lazy_genesis_manifest_root(&manifest_bytes)
        {
            return Err(invalid(
                "lazy genesis sealed commit does not bind its manifest",
            ));
        }
        // The commit above already proved these are the exact sealed bytes, so
        // a manifest that no longer decodes, or decodes to a non-current schema,
        // is a recognized pre-0.7 containing format rather than damage. Carry
        // the durable scenario marker so the graph-open lifecycle preserves the
        // private root and rebuilds automatically instead of refusing (D-1).
        let manifest: LazyGenesisManifestV1 = postcard::from_bytes(&manifest_bytes)
            .map_err(|error| superseded_containing_format(&error.to_string()))?;
        validate_manifest(&manifest)?;
        if manifest.workspace_id != commit.workspace_id
            || manifest.lineage_digest != commit.lineage_digest
            || manifest.source_capture != commit.source_capture
        {
            return Err(invalid(
                "lazy genesis commit and manifest identity disagree",
            ));
        }
        let index = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| (page.page_id, index))
            .collect();
        let home_index = manifest
            .pages
            .iter()
            .enumerate()
            .map(|(index, descriptor)| (descriptor.home_document_id, index))
            .collect();
        let segment_seals = SegmentSealMemo::new(manifest.segments.len());
        Ok(Self {
            scratch: directory.to_path_buf(),
            manifest,
            manifest_bytes,
            root: commit.root,
            index,
            home_index,
            cleanup_on_drop: false,
            segment_seals,
        })
    }

    pub(crate) fn open_sealed_for_marker(
        directory: &Path,
        marker: LazyGenesisActivationMarkerV1,
    ) -> io::Result<Self> {
        let commit_bytes = read_bounded(&directory.join(LAZY_GENESIS_COMMIT_FILE), 1024)?;
        let commit = LazyGenesisCommitV1::decode(&commit_bytes)?;
        if commit.workspace_id != marker.workspace_id()
            || commit.lineage_digest != marker.lineage_digest()
            || commit.source_capture != marker.source_capture()
            || commit.root != marker.baseline_root()
        {
            return Err(invalid(
                "lazy genesis activation marker does not bind the sealed baseline",
            ));
        }
        Self::open_sealed(directory, commit)
    }

    pub(crate) fn open_provider_staged(
        directory: &Path,
        expected: &LazyGenesisProviderIndexV1,
    ) -> io::Result<Self> {
        for (name, description) in expected.file_descriptions() {
            if describe_file(&directory.join(&name))? != description {
                return Err(invalid(format!(
                    "provider-staged lazy genesis file {name} differs from its index"
                )));
            }
        }
        let commit_bytes = read_bounded(&directory.join(LAZY_GENESIS_COMMIT_FILE), 1024)?;
        let commit = LazyGenesisCommitV1::decode(&commit_bytes)?;
        let candidate = Self::open_sealed(directory, commit)?;
        if candidate.provider_index()? != *expected {
            return Err(invalid(
                "provider-staged lazy genesis pack differs from its exact index",
            ));
        }
        Ok(candidate)
    }
}

impl Drop for LazyGenesisCandidate {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = fs::remove_dir_all(&self.scratch);
        }
    }
}

fn validate_manifest(manifest: &LazyGenesisManifestV1) -> io::Result<()> {
    let declared_blocks = manifest.pages.iter().try_fold(0_u64, |total, page| {
        total
            .checked_add(u64::from(page.blocks))
            .ok_or_else(|| invalid("lazy genesis manifest block count overflows"))
    })?;
    if manifest.schema_version != LAZY_GENESIS_SCHEMA_VERSION {
        return Err(superseded_containing_format(&format!(
            "lazy genesis manifest schema {} is not the current schema {}",
            manifest.schema_version, LAZY_GENESIS_SCHEMA_VERSION
        )));
    }
    if manifest.page_count != manifest.pages.len() as u64
        || manifest
            .pages
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
        || declared_blocks != manifest.block_count
    {
        return Err(invalid("lazy genesis manifest is malformed"));
    }
    let mut pages = BTreeSet::new();
    let mut homes = BTreeSet::new();
    let mut dependency_documents = BTreeSet::new();
    if manifest
        .catalog_dependencies
        .as_ref()
        .is_some_and(|dependencies| {
            dependencies.document_id() != manifest.catalog_document_id
                || !dependencies.direct_dependency_heads().is_empty()
                || !dependency_documents.insert(dependencies.document_id())
        })
    {
        return Err(invalid("lazy genesis catalog dependencies are headed"));
    }
    for page in &manifest.pages {
        if !pages.insert(page.page_id)
            || !homes.insert(page.home_document_id)
            || page.document_dependencies.document_id() != page.home_document_id
            || !page
                .document_dependencies
                .direct_dependency_heads()
                .is_empty()
            || !dependency_documents.insert(page.document_dependencies.document_id())
            || page.segment as usize >= manifest.segments.len()
            || page.length > MAX_LAZY_GENESIS_CAPSULE_BYTES as u64
        {
            return Err(invalid("lazy genesis manifest identity is malformed"));
        }
    }
    Ok(())
}

fn segment_path(root: &Path, index: usize) -> PathBuf {
    root.join(format!("segment-{index:08}.pack"))
}

fn lazy_genesis_manifest_root(bytes: &[u8]) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/lazy-genesis/root/v1\0");
    hasher.update(bytes);
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)
}

pub(crate) fn read_activation_marker(
    enrollment_root: &Path,
) -> io::Result<Option<LazyGenesisActivationMarkerV1>> {
    let path = enrollment_root.join(LAZY_GENESIS_ACTIVATION_MARKER_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err(invalid(
            "lazy genesis activation marker is not a bounded regular file",
        ));
    }
    LazyGenesisActivationMarkerV1::decode(&fs::read(path)?).map(Some)
}

/// Publish the sole authority-changing activation record. The exact baseline
/// and SQLite candidate must already be complete; the caller must perform the
/// final source comparison immediately before this call.
pub(crate) fn publish_activation_marker(
    enrollment_root: &Path,
    marker: LazyGenesisActivationMarkerV1,
) -> io::Result<()> {
    fs::create_dir_all(enrollment_root)?;
    let bytes = marker.encode()?;
    let destination = enrollment_root.join(LAZY_GENESIS_ACTIVATION_MARKER_FILE);
    if destination.exists() {
        if read_activation_marker(enrollment_root)? == Some(marker) {
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a different lazy genesis activation marker already exists",
        ));
    }
    let temporary = enrollment_root.join(format!(
        ".{LAZY_GENESIS_ACTIVATION_MARKER_FILE}.{}",
        Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        crate::durability_counters::sync_file(&file)?;
        drop(file);
        fs::rename(&temporary, &destination)?;
        crate::filesystem_durability::sync_reconstructible_directory_path(enrollment_root)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Replace one already-authoritative local marker during a semantically
/// verified shared join. The caller holds the workspace's sole writer lease
/// throughout. The shared durable replacement primitive keeps the old name
/// visible until its single replacement commit point; it never moves authority
/// aside. A retry after an uncertain result accepts the exact replacement.
pub(crate) fn replace_activation_marker_for_join(
    enrollment_root: &Path,
    expected_prior: LazyGenesisActivationMarkerV1,
    replacement: LazyGenesisActivationMarkerV1,
) -> io::Result<()> {
    let observed = read_activation_marker(enrollment_root)?;
    if observed != Some(expected_prior) && observed != Some(replacement) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "clean activation marker changed before shared join installation",
        ));
    }
    let directory =
        cap_std::fs::Dir::open_ambient_dir(enrollment_root, cap_std::ambient_authority())?;
    tine_storage::DurableDirectoryPublication::open(&directory)
        .and_then(|publication| {
            publication.replace_exact(
                LAZY_GENESIS_ACTIVATION_MARKER_FILE,
                &expected_prior.encode()?,
                &replacement.encode()?,
            )
        })
        .map_err(io::Error::other)
}

pub(crate) fn read_clean_shared_state(
    enrollment_root: &Path,
) -> io::Result<Option<CleanSharedStateV1>> {
    let path = enrollment_root.join(CLEAN_SHARED_STATE_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 16 * 1024 {
        return Err(invalid("clean shared state is not a bounded regular file"));
    }
    CleanSharedStateV1::decode(&fs::read(path)?).map(Some)
}

/// Publish the clean shared role only after the provider descriptor and any
/// baseline tail are completely visible.  Repeating the same transition is
/// idempotent; changing either the descriptor or the role requires an explicit
/// future leave/rejoin transition rather than overwriting lifecycle evidence.
pub(crate) fn publish_clean_shared_state(
    enrollment_root: &Path,
    state: &CleanSharedStateV1,
) -> io::Result<()> {
    fs::create_dir_all(enrollment_root)?;
    let bytes = state.encode()?;
    let destination = enrollment_root.join(CLEAN_SHARED_STATE_FILE);
    if destination.exists() {
        if read_clean_shared_state(enrollment_root)?.as_ref() == Some(state) {
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a different clean shared state already exists",
        ));
    }
    let temporary = enrollment_root.join(format!(
        ".{CLEAN_SHARED_STATE_FILE}.{}",
        Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        crate::durability_counters::sync_file(&file)?;
        drop(file);
        fs::rename(&temporary, &destination)?;
        crate::filesystem_durability::sync_reconstructible_directory_path(enrollment_root)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_bounded(path: &Path, cap: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > cap as u64 {
        return Err(invalid(
            "lazy genesis sealed file is not a bounded regular file",
        ));
    }
    fs::read(path)
}

fn describe_file(path: &Path) -> io::Result<BlobDescription> {
    let bytes = fs::read(path)?;
    Ok(BlobDescription::of(&bytes))
}

/// A sealed lazy-genesis baseline this build does not decode. The message
/// carries `MS-REF-PROTOCOL-INCOMPATIBLE` so `SyncRuntimeOpenStatus::OpenRefused`
/// classifies as a durable protocol refusal, which is the only refusal the
/// Tauri graph-open lifecycle answers with preserve-and-rebuild
/// (`SparseV2Binding::requires_blank_slate_rebuild`). Without the marker the
/// same failure surfaced as a retryable dead end (wave-2 review H-1).
#[derive(Debug)]
struct SupersededContainingFormatError(String);

impl std::fmt::Display for SupersededContainingFormatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SupersededContainingFormatError {}

pub(crate) fn is_superseded_containing_format(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|source| source.is::<SupersededContainingFormatError>())
}

fn superseded_containing_format(detail: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        SupersededContainingFormatError(format!(
            "{detail}: this pre-0.7 private baseline must be backed up and rebuilt from the \
             intact Markdown/Org tree by the graph-open lifecycle [{}]",
            crate::oplog::refusal::ManagedStorageRefusalScenario::ProtocolIncompatible.as_str()
        )),
    )
}

/// Test-only: rewrite an installed, marker-bound sealed baseline as if an
/// earlier build had sealed it under `schema_version`. The commit and the
/// activation marker are re-bound to the rewritten manifest exactly as that
/// build would have left them, so the real open path meets a consistent
/// pre-current store rather than a torn one.
#[cfg(test)]
pub(crate) fn rewrite_sealed_baseline_schema_for_test(
    enrollment_root: &Path,
    baseline_directory: &Path,
    schema_version: u32,
) -> io::Result<()> {
    let manifest_path = baseline_directory.join(LAZY_GENESIS_MANIFEST_FILE);
    let commit_path = baseline_directory.join(LAZY_GENESIS_COMMIT_FILE);
    let mut manifest: LazyGenesisManifestV1 = postcard::from_bytes(&fs::read(&manifest_path)?)
        .map_err(|error| invalid(error.to_string()))?;
    manifest.schema_version = schema_version;
    let manifest_bytes =
        postcard::to_allocvec(&manifest).map_err(|error| invalid(error.to_string()))?;
    let mut commit = LazyGenesisCommitV1::decode(&fs::read(&commit_path)?)?;
    commit.manifest = BlobDescription::of(&manifest_bytes);
    commit.root = lazy_genesis_manifest_root(&manifest_bytes);
    fs::write(&manifest_path, &manifest_bytes)?;
    fs::write(&commit_path, commit.encode()?)?;
    let marker = read_activation_marker(enrollment_root)?
        .ok_or_else(|| invalid("no activation marker to re-bind"))?;
    let rebound = LazyGenesisActivationMarkerV1 {
        baseline_root: commit.root,
        ..marker
    };
    fs::remove_file(enrollment_root.join(LAZY_GENESIS_ACTIVATION_MARKER_FILE))?;
    publish_activation_marker(enrollment_root, rebound)
}

fn invalid(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(ordinal: u128, path: &str, blocks: usize) -> LazyGenesisPageInput {
        let home = DocumentId::from_uuid(Uuid::from_u128(10_000 + ordinal));
        let source_digest = ContentDigest::of(path.as_bytes());
        LazyGenesisPageInput {
            source_leaf: *source_digest.as_bytes(),
            exact_source_bytes: vec![b'x'; blocks * 8],
            page_id: PageId::from_uuid(Uuid::from_u128(ordinal)),
            home_document_id: home,
            name: path.to_owned(),
            path: ManagedPath::parse(path).unwrap(),
            kind: ManagedTextKind::Page,
            preamble: None,
            document_checkpoint: vec![ordinal as u8, blocks as u8, 0x47],
            document_dependencies: Some(
                DocumentDependencies::new(
                    home,
                    vec![super::super::CrdtPeerCounter::new(
                        super::super::CrdtPeerId::from_u64(7),
                        ordinal as u64,
                    )],
                    Vec::new(),
                )
                .unwrap(),
            ),
            blocks: (0..blocks)
                .map(|index| LazyGenesisBlockInput {
                    block_id: BlockId::from_uuid(Uuid::from_u128(
                        100_000 + ordinal * 100 + index as u128,
                    )),
                    home_document_id: home,
                    parent: None,
                    order: format!("{index:08}"),
                    content: format!("block {index}"),
                    external_uuid_claims: Vec::new(),
                })
                .collect(),
            sqlite_receipt: None,
        }
    }

    fn catalog_dependencies() -> DocumentDependencies {
        DocumentDependencies::new(
            DocumentId::from_uuid(Uuid::from_u128(99_999)),
            vec![super::super::CrdtPeerCounter::new(
                super::super::CrdtPeerId::from_u64(7),
                1,
            )],
            Vec::new(),
        )
        .unwrap()
    }

    fn catalog_document_id() -> DocumentId {
        catalog_dependencies().document_id()
    }

    #[test]
    fn lazy_genesis_pack_is_deterministic_bounded_and_point_readable() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let lineage = LineageDigest::of(b"lazy-genesis-test");
        let source = BlobDescription::of(b"capture");
        let build = || {
            let mut builder = LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog_document_id(),
                source,
                &std::env::temp_dir(),
            )
            .unwrap();
            let pages = vec![page(1, "pages/a.md", 2), page(2, "pages/b.org", 1)];
            for page in pages {
                builder.push(page).unwrap();
            }
            builder
                .finish(vec![0x43, 0x41, 0x54], Some(catalog_dependencies()))
                .unwrap()
        };
        let first = build();
        let second = build();
        assert_eq!(first.root(), second.root());
        assert_eq!(first.manifest_bytes, second.manifest_bytes);
        assert_eq!(first.page_count(), 2);
        assert_eq!(first.manifest.block_count, 3);
        assert_eq!(first.catalog_document_id(), catalog_document_id());
        let read = first
            .page(PageId::from_uuid(Uuid::from_u128(2)))
            .unwrap()
            .unwrap();
        assert_eq!(read.path.as_str(), "pages/b.org");
        assert_eq!(read.blocks.len(), 1);
        assert_eq!(read.exact_source_bytes, vec![b'x'; 8]);

        let mut previous_manifest = first.manifest.clone();
        previous_manifest.schema_version = 4;
        assert!(
            validate_manifest(&previous_manifest).is_err(),
            "a schema-4 containing manifest must be rejected instead of partially decoding old capsules"
        );
    }

    #[test]
    fn lazy_genesis_page_home_index_agrees_before_and_after_reopen() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0xa180));
        let lineage = LineageDigest::of(b"lazy-genesis-document-index-test");
        let inputs = [
            page(81, "pages/indexed-a.md", 2),
            page(82, "pages/indexed-b.md", 1),
        ];
        let expected = inputs
            .iter()
            .map(|input| {
                (
                    input.home_document_id,
                    (
                        input.page_id,
                        input.document_dependencies.clone().unwrap(),
                        input.document_checkpoint.clone(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut builder = LazyGenesisPackBuilder::new(
            workspace,
            lineage,
            catalog_document_id(),
            BlobDescription::of(b"capture"),
            &std::env::temp_dir(),
        )
        .unwrap();
        for input in inputs {
            builder.push(input).unwrap();
        }
        let candidate = builder
            .finish(vec![0x43, 0x41, 0x54], Some(catalog_dependencies()))
            .unwrap();
        // The index is derived from a manifest that must name each page home,
        // and each home's causal row, exactly once.
        let mut shared_home = candidate.manifest.clone();
        shared_home.pages[1].home_document_id = shared_home.pages[0].home_document_id;
        shared_home.pages[1].document_dependencies =
            shared_home.pages[0].document_dependencies.clone();
        let mut repeated_row = candidate.manifest.clone();
        repeated_row.pages[1].document_dependencies =
            repeated_row.pages[0].document_dependencies.clone();
        for duplicate in [&shared_home, &repeated_row] {
            assert!(
                validate_manifest(duplicate)
                    .unwrap_err()
                    .to_string()
                    .contains("identity is malformed"),
                "the derived index must refuse a repeated page home or causal row"
            );
        }
        let sealed = std::env::temp_dir().join(format!(
            "tine-lazy-genesis-document-index-{}",
            Uuid::new_v4()
        ));
        let (candidate, commit) = candidate.stage_into(&sealed).unwrap();
        let reopened = LazyGenesisCandidate::open_sealed(&sealed, commit).unwrap();

        for indexed in [&candidate, &reopened] {
            assert_eq!(
                indexed.frontier_document(catalog_document_id()),
                Some(catalog_dependencies())
            );
            for (home, (page_id, dependencies, checkpoint)) in &expected {
                assert_eq!(
                    indexed.frontier_document(*home).as_ref(),
                    Some(dependencies)
                );
                assert_eq!(indexed.page_home_document_id(*page_id), Some(*home));
                assert_eq!(
                    indexed.document_checkpoint(*home).unwrap().as_ref(),
                    Some(checkpoint)
                );
            }
            let unknown = Uuid::from_u128(0xdead_beef);
            assert_eq!(
                indexed.frontier_document(DocumentId::from_uuid(unknown)),
                None
            );
            assert_eq!(
                indexed
                    .document_checkpoint(DocumentId::from_uuid(unknown))
                    .unwrap(),
                None
            );
            assert_eq!(
                indexed.page_home_document_id(PageId::from_uuid(unknown)),
                None
            );
        }
        assert_eq!(
            candidate.frontier_documents(),
            reopened.frontier_documents()
        );
    }

    #[test]
    fn capsule_decoder_accepts_only_the_current_receipted_shape() {
        #[derive(Serialize)]
        struct PreviousCapsule {
            schema_version: u32,
            source_leaf: [u8; 32],
            exact_source_bytes: Vec<u8>,
            page_id: PageId,
            home_document_id: DocumentId,
            name: String,
            path: ManagedPath,
            kind: ManagedTextKind,
            preamble: Option<String>,
            blocks: Vec<LazyGenesisBlockInput>,
            document_checkpoint: Vec<u8>,
        }

        let current = LazyGenesisPageCapsuleV1::from_input(page(7, "pages/legacy.md", 2)).unwrap();
        let current_bytes = current.encode().unwrap();
        assert_eq!(
            LazyGenesisPageCapsuleV1::decode(&current_bytes).unwrap(),
            current
        );

        let previous = PreviousCapsule {
            schema_version: 4,
            source_leaf: current.source_leaf,
            exact_source_bytes: current.exact_source_bytes.clone(),
            page_id: current.page_id,
            home_document_id: current.home_document_id,
            name: current.name.clone(),
            path: current.path.clone(),
            kind: current.kind,
            preamble: current.preamble.clone(),
            blocks: current.blocks.clone(),
            document_checkpoint: current.document_checkpoint.clone(),
        };
        let bytes = postcard::to_allocvec(&previous).unwrap();
        assert!(
            LazyGenesisPageCapsuleV1::decode(&bytes).is_err(),
            "the current capsule decoder must not retain a schema-4 fallback"
        );
    }

    #[test]
    fn sqlite_receipt_bounds_are_inclusive_and_omit_after_either_cap() {
        assert!(sqlite_receipt_within_bounds(
            MAX_LAZY_GENESIS_SQLITE_RECEIPT_BYTES,
            MAX_LAZY_GENESIS_SQLITE_RECEIPT_ROWS
        ));
        assert!(!sqlite_receipt_within_bounds(
            MAX_LAZY_GENESIS_SQLITE_RECEIPT_BYTES + 1,
            MAX_LAZY_GENESIS_SQLITE_RECEIPT_ROWS
        ));
        assert!(!sqlite_receipt_within_bounds(
            MAX_LAZY_GENESIS_SQLITE_RECEIPT_BYTES,
            MAX_LAZY_GENESIS_SQLITE_RECEIPT_ROWS + 1
        ));
    }

    /// Build a sealed single-segment pack of `pages` tiny pages.
    fn sealed_pack(seed: u128, pages: usize) -> LazyGenesisCandidate {
        let mut builder = LazyGenesisPackBuilder::new(
            WorkspaceId::from_uuid(Uuid::from_u128(seed)),
            LineageDigest::of(b"lazy-genesis-segment-seal-test"),
            catalog_document_id(),
            BlobDescription::of(b"capture"),
            &std::env::temp_dir(),
        )
        .unwrap();
        for index in 0..pages {
            builder
                .push(page(index as u128 + 1, &format!("pages/p{index:06}.md"), 1))
                .unwrap();
        }
        builder
            .finish(vec![0x43, 0x41, 0x54], Some(catalog_dependencies()))
            .unwrap()
    }

    fn page_id_at(ordinal: usize) -> PageId {
        PageId::from_uuid(Uuid::from_u128(ordinal as u128 + 1))
    }

    /// A sealed segment pack is written once and never rewritten, so proving
    /// it against its manifest digest is a property of the pack, not of the
    /// read. Re-proving per page would make every baseline read — including
    /// the clean watcher's full scan — cost `O(pages x segment bytes)`.
    #[test]
    fn lazy_genesis_proves_each_sealed_segment_at_most_once() {
        const PAGES: usize = 64;
        let candidate = sealed_pack(0xa181, PAGES);
        assert_eq!(candidate.manifest.segments.len(), 1);
        assert_eq!(candidate.segment_seal_proofs(), 0);
        for ordinal in 0..PAGES {
            assert!(candidate.page(page_id_at(ordinal)).unwrap().is_some());
        }
        assert_eq!(
            candidate.segment_seal_proofs(),
            1,
            "reading {PAGES} pages must not re-hash the sealed segment once per page"
        );

        let contract = include_str!("../../../../docs/storage-sync-contract.md");
        assert!(contract.contains("Reading one baseline page costs that page, not the pack."));
        assert!(contract
            .contains("proved against\nthe sealed manifest at most once per opened baseline"));
    }

    /// The retained proof must not blind the per-capsule integrity check: the
    /// bytes a caller actually receives are verified against their descriptor
    /// digest on every read.
    #[test]
    fn lazy_genesis_rejects_a_damaged_capsule_after_its_segment_was_proved() {
        let candidate = sealed_pack(0xa182, 8);
        assert!(candidate.page(page_id_at(0)).unwrap().is_some());
        assert_eq!(candidate.segment_seal_proofs(), 1);

        let descriptor = candidate
            .manifest
            .pages
            .iter()
            .find(|descriptor| descriptor.page_id == page_id_at(5))
            .expect("sealed pack describes every pushed page")
            .clone();
        let segment = segment_path(&candidate.scratch, descriptor.segment as usize);
        let mut bytes = fs::read(&segment).unwrap();
        let target = descriptor.offset as usize;
        bytes[target] ^= 0xff;
        fs::write(&segment, &bytes).unwrap();

        let error = candidate.page(page_id_at(5)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error.to_string().contains("capsule bytes changed"),
            "damaged capsule bytes must be rejected on every read: {error}"
        );
    }

    /// The whole-pack seal proof is retained, not deleted: damage that no
    /// individual capsule read would notice is still rejected the first time
    /// the segment is touched.
    #[test]
    fn lazy_genesis_rejects_a_damaged_segment_on_its_first_read() {
        let candidate = sealed_pack(0xa183, 8);
        let segment = segment_path(&candidate.scratch, 0);
        let mut bytes = fs::read(&segment).unwrap();
        bytes.push(0x00);
        fs::write(&segment, &bytes).unwrap();

        let error = candidate.page(page_id_at(0)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error.to_string().contains("segment bytes changed"),
            "a resized sealed segment must be rejected: {error}"
        );
        assert_eq!(candidate.segment_seal_proofs(), 0);
    }

    #[test]
    fn activation_marker_is_minimal_canonical_and_excludes_sqlite() {
        let marker = LazyGenesisActivationMarkerV1::new(
            WorkspaceId::from_uuid(Uuid::from_u128(1)),
            LineageDigest::of(b"activation-marker-lineage"),
            ContentDigest::of(b"baseline"),
            BlobDescription::of(b"source capture"),
            ContentDigest::of(b"accepted frontier"),
            73,
        )
        .unwrap();
        let encoded = marker.encode().unwrap();
        assert_eq!(
            LazyGenesisActivationMarkerV1::decode(&encoded).unwrap(),
            marker
        );
        assert_eq!(marker.watcher_fence(), 73);
        assert_eq!(marker.generation(), 0);
        assert_eq!(
            marker.workspace_id(),
            WorkspaceId::from_uuid(Uuid::from_u128(1))
        );
        assert_eq!(marker.baseline_root(), ContentDigest::of(b"baseline"));
        assert_eq!(
            marker.source_capture(),
            BlobDescription::of(b"source capture")
        );
        assert_eq!(
            marker.accepted_frontier_digest(),
            ContentDigest::of(b"accepted frontier")
        );
        assert_eq!(
            marker.lineage_digest(),
            LineageDigest::of(b"activation-marker-lineage")
        );

        let source = include_str!("lazy_genesis.rs");
        let marker_source = source
            .split_once("pub(crate) struct LazyGenesisActivationMarkerV1")
            .and_then(|(_, tail)| tail.split_once("impl LazyGenesisActivationMarkerV1"))
            .map(|(body, _)| body)
            .expect("activation marker definition must remain identifiable");
        assert!(
            marker_source.contains("generation: u64"),
            "the activation marker must name the authority-directory generation"
        );
        for field in [
            "schema_version: u32",
            "workspace_id: WorkspaceId",
            "lineage_digest: LineageDigest",
            "generation: u64",
            "baseline_root: ContentDigest",
            "source_capture: BlobDescription",
            "accepted_frontier_digest: ContentDigest",
            "watcher_fence: u64",
        ] {
            assert!(
                marker_source.contains(field),
                "missing marker field {field}"
            );
        }
        assert!(!marker_source.contains("sqlite"));
        assert!(!marker_source.contains("database"));

        let contract = include_str!("../../../../docs/storage-sync-contract.md");
        assert!(contract.contains("one final lazy-genesis authority marker"));
        assert!(contract.contains("SQLite identity is deliberately absent"));
        assert!(contract.contains("authority-directory generation"));
        assert!(contract.contains("one marker rename is\nthe commit point"));
        assert!(contract.contains("Tauri binding records opt-in\nintent"));
    }

    #[test]
    fn clean_shared_descriptor_and_private_role_bind_only_clean_authority() {
        let root = std::env::temp_dir().join(format!(
            "tine-clean-shared-state-test-{}",
            Uuid::new_v4().simple()
        ));
        let marker = LazyGenesisActivationMarkerV1::new(
            WorkspaceId::from_uuid(Uuid::from_u128(1)),
            LineageDigest::of(b"clean-shared-lineage"),
            ContentDigest::of(b"baseline"),
            BlobDescription::of(b"source capture"),
            ContentDigest::of(b"baseline frontier"),
            9,
        )
        .unwrap();
        let descriptor = CleanSharedEnrollmentDescriptorV1::new(
            marker,
            BlobDescription::of(b"provider baseline index"),
            catalog_document_id(),
            ContentDigest::of(b"accepted baseline plus tail"),
            DeviceId::from_uuid(Uuid::from_u128(2)),
            ContentDigest::of(b"provider namespace"),
        )
        .unwrap();
        let encoded = descriptor.encode().unwrap();
        assert_eq!(
            CleanSharedEnrollmentDescriptorV1::decode(&encoded).unwrap(),
            descriptor
        );
        assert_eq!(descriptor.baseline_root(), marker.baseline_root());
        assert_eq!(
            descriptor.baseline_index(),
            BlobDescription::of(b"provider baseline index")
        );
        assert_eq!(descriptor.source_capture(), marker.source_capture());
        assert_eq!(
            descriptor.accepted_frontier_digest(),
            ContentDigest::of(b"accepted baseline plus tail")
        );

        let state =
            CleanSharedStateV1::new(descriptor.clone(), CleanSharedRoleV1::Initiator).unwrap();
        publish_clean_shared_state(&root, &state).unwrap();
        publish_clean_shared_state(&root, &state).unwrap();
        assert_eq!(read_clean_shared_state(&root).unwrap(), Some(state));
        let conflicting = CleanSharedStateV1::new(descriptor, CleanSharedRoleV1::Joiner).unwrap();
        assert!(publish_clean_shared_state(&root, &conflicting).is_err());
        fs::remove_dir_all(root).unwrap();

        let source = include_str!("lazy_genesis.rs");
        let descriptor_source = source
            .split_once("pub(crate) struct CleanSharedEnrollmentDescriptorV1")
            .and_then(|(_, tail)| tail.split_once("impl CleanSharedEnrollmentDescriptorV1"))
            .map(|(body, _)| body)
            .expect("clean shared descriptor definition remains identifiable");
        for forbidden in [
            "patricia",
            "projection_work",
            "sqlite",
            "verification_digest",
        ] {
            assert!(
                !descriptor_source.to_ascii_lowercase().contains(forbidden),
                "clean shared descriptor unexpectedly retained {forbidden}"
            );
        }
    }

    #[test]
    fn lazy_genesis_rejects_cross_page_parent_and_duplicate_identity() {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let lineage = LineageDigest::of(b"lazy-genesis-test");
        let mut builder = LazyGenesisPackBuilder::new(
            workspace,
            lineage,
            catalog_document_id(),
            BlobDescription::of(b"capture"),
            &std::env::temp_dir(),
        )
        .unwrap();
        let mut invalid_page = page(1, "pages/a.md", 1);
        invalid_page.blocks[0].parent = Some(BlockId::from_uuid(Uuid::from_u128(999)));
        assert!(builder.push(invalid_page).is_err());

        let mut builder = LazyGenesisPackBuilder::new(
            workspace,
            lineage,
            catalog_document_id(),
            BlobDescription::of(b"capture"),
            &std::env::temp_dir(),
        )
        .unwrap();
        builder.push(page(2, "pages/b.md", 0)).unwrap();
        assert!(builder.push(page(1, "pages/a.md", 0)).is_err());
    }

    #[test]
    fn join_marker_replacement_is_exact_retryable_and_never_removes_authority() {
        let root = std::env::temp_dir().join(format!("tine-marker-replace-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let marker = |generation| {
            LazyGenesisActivationMarkerV1::new_for_generation(
                WorkspaceId::from_uuid(Uuid::from_u128(1)),
                LineageDigest::of(b"marker replacement"),
                generation,
                ContentDigest::of(b"baseline"),
                BlobDescription::of(b"source"),
                ContentDigest::of(b"frontier"),
                9,
            )
            .unwrap()
        };
        let prior = marker(0);
        let replacement = marker(1);
        assert!(replace_activation_marker_for_join(root.as_path(), prior, replacement).is_err());
        publish_activation_marker(root.as_path(), prior).unwrap();
        assert!(
            replace_activation_marker_for_join(root.as_path(), marker(2), replacement).is_err()
        );
        assert_eq!(read_activation_marker(root.as_path()).unwrap(), Some(prior));

        // An independent reader observes the actual authority name during
        // repeated durable replacements. Every complete read must be one of
        // the two valid records, never NotFound or partially written bytes.
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let observations = std::sync::Arc::new(AtomicUsize::new(0));
        let reader = {
            let path = root.as_path().to_path_buf();
            let stop = stop.clone();
            let observations = observations.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let observed = read_activation_marker(&path).unwrap();
                    assert!(observed == Some(prior) || observed == Some(replacement));
                    observations.fetch_add(1, Ordering::Release);
                }
            })
        };
        while observations.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        for _ in 0..32 {
            replace_activation_marker_for_join(root.as_path(), prior, replacement).unwrap();
            // This models a retry after losing the success response.
            replace_activation_marker_for_join(root.as_path(), prior, replacement).unwrap();
            replace_activation_marker_for_join(root.as_path(), replacement, prior).unwrap();
        }
        stop.store(true, Ordering::Release);
        reader.join().unwrap();
        assert!(observations.load(Ordering::Acquire) > 0);
        assert_eq!(read_activation_marker(root.as_path()).unwrap(), Some(prior));
        assert_eq!(std::fs::read_dir(root.as_path()).unwrap().count(), 1);

        std::fs::write(
            root.as_path().join(LAZY_GENESIS_ACTIVATION_MARKER_FILE),
            b"damaged",
        )
        .unwrap();
        assert!(replace_activation_marker_for_join(root.as_path(), prior, replacement).is_err());
        assert_eq!(
            std::fs::read(root.as_path().join(LAZY_GENESIS_ACTIVATION_MARKER_FILE)).unwrap(),
            b"damaged"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn join_marker_uses_the_shared_durable_replacement_boundary() {
        let source = include_str!("lazy_genesis.rs");
        let body = source
            .split_once("pub(crate) fn replace_activation_marker_for_join(")
            .unwrap()
            .1
            .split_once("pub(crate) fn read_clean_shared_state(")
            .unwrap()
            .0;
        assert!(body.contains("DurableDirectoryPublication::open"));
        assert!(body.contains("publication.replace_exact("));
        for forbidden in [
            "fs::rename",
            "fs::remove_file",
            "OpenOptions",
            "sync_reconstructible",
        ] {
            assert!(
                !body.contains(forbidden),
                "marker replacement bypasses shared durability: {forbidden}"
            );
        }
    }

    #[test]
    fn baseline_remains_disposable_until_the_marker_is_published_last() {
        let root = std::env::temp_dir().join(format!(
            "tine-lazy-genesis-marker-test-{}",
            Uuid::new_v4().simple()
        ));
        let archive = root.join("archive");
        let enrollment = root.join("enrollment");
        fs::create_dir_all(&archive).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let lineage = LineageDigest::of(b"lazy-genesis-marker-test");
        let source = BlobDescription::of(b"sealed source capture");
        let build = || {
            let mut builder = LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog_document_id(),
                source,
                &root,
            )
            .unwrap();
            builder.push(page(1, "pages/a.md", 1)).unwrap();
            builder
                .finish(vec![0x43, 0x41, 0x54], Some(catalog_dependencies()))
                .unwrap()
        };

        let destination = archive.join(LAZY_GENESIS_BASELINE_DIRECTORY);
        let (disposable, _) = build().publish_durable(&destination).unwrap();
        assert!(destination.is_dir());
        assert_eq!(read_activation_marker(&enrollment).unwrap(), None);
        drop(disposable);
        assert!(!destination.exists());

        let (candidate, _) = build().publish_durable(&destination).unwrap();
        let marker = LazyGenesisActivationMarkerV1::new(
            workspace,
            lineage,
            candidate.root(),
            candidate.source_capture(),
            ContentDigest::of(b"accepted frontier"),
            73,
        )
        .unwrap();
        assert_eq!(read_activation_marker(&enrollment).unwrap(), None);
        publish_activation_marker(&enrollment, marker).unwrap();
        publish_activation_marker(&enrollment, marker).unwrap();
        drop(candidate.retain_as_authoritative());

        assert_eq!(read_activation_marker(&enrollment).unwrap(), Some(marker));
        let reopened = LazyGenesisCandidate::open_sealed_for_marker(&destination, marker).unwrap();
        assert_eq!(reopened.root(), marker.baseline_root());
        assert_eq!(reopened.page_count(), 1);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    /// Wave-2 review H-1: a sealed baseline from an earlier schema is a
    /// recognized pre-0.7 containing format. Its refusal must carry the
    /// durable protocol marker, because that marker is what routes the store
    /// into preserve-and-rebuild instead of a retryable dead end.
    #[test]
    fn previous_schema_sealed_baseline_refuses_with_the_protocol_marker() {
        let root = std::env::temp_dir().join(format!(
            "tine-lazy-genesis-schema-marker-{}",
            Uuid::new_v4().simple()
        ));
        let enrollment = root.join("enrollment");
        let destination = root.join("archive/lazy-genesis");
        fs::create_dir_all(root.join("archive")).unwrap();
        fs::create_dir_all(&enrollment).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(11));
        let lineage = LineageDigest::of(b"lazy-genesis-schema-marker");
        let build = || {
            let scratch = root.join(format!("scratch-{}", Uuid::new_v4().simple()));
            fs::create_dir_all(&scratch).unwrap();
            let mut builder = LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog_document_id(),
                BlobDescription::of(b"capture"),
                &scratch,
            )
            .unwrap();
            builder.push(page(1, "pages/a.md", 1)).unwrap();
            builder
                .finish(vec![0x43, 0x41, 0x54], Some(catalog_dependencies()))
                .unwrap()
        };
        let (candidate, _) = build().publish_durable(&destination).unwrap();
        let marker = LazyGenesisActivationMarkerV1::new(
            workspace,
            lineage,
            candidate.root(),
            candidate.source_capture(),
            ContentDigest::of(b"accepted frontier"),
            1,
        )
        .unwrap();
        publish_activation_marker(&enrollment, marker).unwrap();
        drop(candidate.retain_as_authoritative());
        LazyGenesisCandidate::open_sealed_for_marker(&destination, marker).unwrap();

        rewrite_sealed_baseline_schema_for_test(&enrollment, &destination, 4).unwrap();
        let rebound = read_activation_marker(&enrollment).unwrap().unwrap();
        let error = LazyGenesisCandidate::open_sealed_for_marker(&destination, rebound)
            .err()
            .expect("a schema-4 baseline is not decoded as current");
        let detail = error.to_string();
        assert!(
            detail.contains("lazy genesis manifest schema 4"),
            "the refusal names the component: {detail}"
        );
        assert!(
            detail.contains(
                crate::oplog::refusal::ManagedStorageRefusalScenario::ProtocolIncompatible.as_str()
            ),
            "the refusal carries the durable protocol marker: {detail}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// K-c (`d7c57e2b`): two activation pages may never share one immutable
    /// page home. Under page shards a shared home would merge two pages'
    /// owners, memberships and content into one shard document.
    #[test]
    fn lazy_genesis_checkpoint_builder_refuses_two_pages_sharing_one_home() {
        let first = page(91, "pages/first.md", 1);
        let mut second = page(92, "pages/second.md", 0);
        second.home_document_id = first.home_document_id;
        let mut builder =
            super::super::hot_engine::LazyGenesisCheckpointBuilder::new(catalog_document_id())
                .unwrap();
        builder.push_page(&first, &BTreeMap::new()).unwrap();
        let error = builder
            .push_page(&second, &BTreeMap::new())
            .expect_err("a repeated page home is refused");
        assert!(
            matches!(
                &error,
                super::super::EngineError::InvalidTransaction(detail)
                    if detail.contains("repeats or aliases")
            ),
            "{error:?}"
        );
    }

    #[test]
    fn lazy_genesis_seal_reopens_and_detects_payload_corruption() {
        let parent = std::env::temp_dir().join(format!(
            "tine-lazy-genesis-seal-test-{}",
            Uuid::new_v4().simple()
        ));
        let moved_parent = parent.with_extension("sealed");
        fs::create_dir(&parent).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let lineage = LineageDigest::of(b"lazy-genesis-seal-test");
        let mut builder = LazyGenesisPackBuilder::new(
            workspace,
            lineage,
            catalog_document_id(),
            BlobDescription::of(b"capture"),
            &parent,
        )
        .unwrap();
        builder.push(page(1, "pages/a.md", 1)).unwrap();
        let candidate = builder
            .finish(vec![0x43, 0x41, 0x54], Some(catalog_dependencies()))
            .unwrap();
        let (candidate, commit) = candidate.stage_into(&parent.join("genesis")).unwrap();
        fs::rename(&parent, &moved_parent).unwrap();
        drop(candidate);

        let reopened =
            LazyGenesisCandidate::open_sealed(&moved_parent.join("genesis"), commit).unwrap();
        assert_eq!(reopened.page_count(), 1);
        let segment = segment_path(&moved_parent.join("genesis"), 0);
        fs::OpenOptions::new()
            .append(true)
            .open(segment)
            .unwrap()
            .write_all(b"corrupt")
            .unwrap();
        assert!(reopened
            .page(PageId::from_uuid(Uuid::from_u128(1)))
            .is_err());
        drop(reopened);
        fs::remove_dir_all(moved_parent).unwrap();
    }
}
