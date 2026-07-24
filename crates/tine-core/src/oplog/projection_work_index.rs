//! Terminal work transitions are proof-bearing crate internals, not public
//! work-index mutators:
//!
//! ```compile_fail
//! use tine_core::oplog::{ProjectionWorkId, ProjectionWorkIndex};
//! let _: fn(&ProjectionWorkIndex, ProjectionWorkId) -> _ =
//!     ProjectionWorkIndex::mark_completed;
//! ```
//!
//! ```compile_fail
//! use tine_core::oplog::{ProjectionWorkId, ProjectionWorkIndex};
//! let _: fn(&ProjectionWorkIndex, ProjectionWorkId) -> _ =
//!     ProjectionWorkIndex::mark_blocked;
//! ```

use std::fmt;
use std::io::{ErrorKind, Write};
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::identity::{parse_digest, write_hex};
use super::object_store::{
    StoreError, open_dir_nofollow, publish_immutable_exact, read_optional_regular,
    sync_dir_required,
};
use super::{
    BatchId, BlobDescription, ContentDigest, FrontierV2, LogicalCompletionId, ManagedPath,
    ManifestObjectRef, PORTABLE_PATH_KEY_VERSION, PageId, PortablePathIndexRoot,
    PortablePathKeyDigest, ProjectionEndpointId, ProjectionIntentId, WorkspaceId,
};

const WORK_SCHEMA_VERSION: u32 = 3;
const INDEX_SCHEMA_VERSION: u32 = 5;
const MAX_WORK_ROW_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PREPARED_BATCH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INDEX_NODE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_INDEX_KEY_BYTES: usize = 4 * 1024;
const MAX_READY_PAGE: usize = 256;
const MAX_PENDING_PAGE: usize = 256;
const MAX_PREFLIGHT_NODES: usize = 2_000_000;
const MAX_PREFLIGHT_RECORDS: usize = 2_000_000;
const MAX_PREFLIGHT_ROOTS: usize = 2_000_000;
const MAX_PREFLIGHT_BYTES: usize = 512 * 1024 * 1024;
const CLAIM_FILE: &str = "projection-work.claim";
const HEAD_FILE: &str = "projection-work.head";
const PREPARED_SUFFIX: &str = ".prepared";
const NODE_SUFFIX: &str = ".work-node";
const ROOT_SUFFIX: &str = ".work-root";

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectionWorkId([u8; 32]);

impl ProjectionWorkId {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ProjectionWorkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProjectionWorkId({self})")
    }
}

impl fmt::Display for ProjectionWorkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(&self.0, f)
    }
}

impl FromStr for ProjectionWorkId {
    type Err = super::identity::DigestParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_digest(value).map(Self)
    }
}

impl Serialize for ProjectionWorkId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ProjectionWorkId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionWorkTarget {
    Absent,
    Present(BlobDescription),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionWork {
    schema_version: u32,
    work_id: ProjectionWorkId,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    batch_id: BatchId,
    page_id: PageId,
    path: ManagedPath,
    portable_path_key_version: u32,
    portable_path_key_digest: PortablePathKeyDigest,
    portable_path_index_root: PortablePathIndexRoot,
    intent: ManifestObjectRef,
    post_frontier: FrontierV2,
    target: ProjectionWorkTarget,
}

impl ProjectionWork {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        batch_id: BatchId,
        page_id: PageId,
        path: ManagedPath,
        portable_path_index_root: PortablePathIndexRoot,
        intent: ManifestObjectRef,
        post_frontier: FrontierV2,
        target: ProjectionWorkTarget,
    ) -> Self {
        let portable_path_key_digest = path.portable_key().digest();
        let work_id = work_id(
            endpoint_id,
            graph_resource_id,
            batch_id,
            page_id,
            &path,
            portable_path_key_digest,
            portable_path_index_root,
        );
        Self {
            schema_version: WORK_SCHEMA_VERSION,
            work_id,
            workspace_id,
            endpoint_id,
            graph_resource_id,
            batch_id,
            page_id,
            path,
            portable_path_key_version: PORTABLE_PATH_KEY_VERSION,
            portable_path_key_digest,
            portable_path_index_root,
            intent,
            post_frontier,
            target,
        }
    }

    pub const fn work_id(&self) -> ProjectionWorkId {
        self.work_id
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn endpoint_id(&self) -> ProjectionEndpointId {
        self.endpoint_id
    }

    pub const fn graph_resource_id(&self) -> super::CanonicalGraphResourceId {
        self.graph_resource_id
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
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

    pub const fn intent(&self) -> &ManifestObjectRef {
        &self.intent
    }

    pub const fn post_frontier(&self) -> &FrontierV2 {
        &self.post_frontier
    }

    pub const fn target(&self) -> ProjectionWorkTarget {
        self.target
    }

    fn encode(&self) -> Result<Vec<u8>, ProjectionWorkError> {
        postcard::to_allocvec(self).map_err(|error| ProjectionWorkError::Encode(error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionWorkStatus {
    Reserved,
    Ready,
    Completed,
    Blocked,
    Superseded { by: ProjectionWorkId },
}

pub(crate) struct ProjectionWorkCompletionAuthority {
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    work_id: ProjectionWorkId,
    page_id: PageId,
    path: ManagedPath,
    target: ProjectionWorkTarget,
    intent_id: ProjectionIntentId,
    logical_completion_id: LogicalCompletionId,
}

impl ProjectionWorkCompletionAuthority {
    pub(super) fn from_durable_completion(
        work: &ProjectionWork,
        receipt_store_id: super::ProjectionReceiptStoreId,
        intent_id: ProjectionIntentId,
        logical_completion_id: LogicalCompletionId,
    ) -> Self {
        Self {
            workspace_id: work.workspace_id(),
            endpoint_id: work.endpoint_id(),
            graph_resource_id: work.graph_resource_id(),
            receipt_store_id,
            work_id: work.work_id(),
            page_id: work.page_id(),
            path: work.path().clone(),
            target: work.target(),
            intent_id,
            logical_completion_id,
        }
    }
}

pub(crate) struct ProjectionWorkBlockAuthority {
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    work_id: ProjectionWorkId,
    page_id: PageId,
    path: ManagedPath,
    target: ProjectionWorkTarget,
    observed: Option<BlobDescription>,
}

impl ProjectionWorkBlockAuthority {
    pub(super) fn guarded_conflict(
        work: &ProjectionWork,
        receipt_store_id: super::ProjectionReceiptStoreId,
        observed: Option<BlobDescription>,
    ) -> Self {
        Self {
            workspace_id: work.workspace_id(),
            endpoint_id: work.endpoint_id(),
            graph_resource_id: work.graph_resource_id(),
            receipt_store_id,
            work_id: work.work_id(),
            page_id: work.page_id(),
            path: work.path().clone(),
            target: work.target(),
            observed,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWorkCursor {
    root: ContentDigest,
    after: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWorkPage {
    work: Vec<ProjectionWork>,
    next: Option<ProjectionWorkCursor>,
}

impl ProjectionWorkPage {
    pub fn work(&self) -> &[ProjectionWork] {
        &self.work
    }

    pub const fn next(&self) -> Option<&ProjectionWorkCursor> {
        self.next.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionPendingActivation {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    prepared_digest: ContentDigest,
    work_ids: Vec<ProjectionWorkId>,
}

impl ProjectionPendingActivation {
    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub const fn manifest_fingerprint(&self) -> ContentDigest {
        self.manifest_fingerprint
    }

    pub const fn prepared_digest(&self) -> ContentDigest {
        self.prepared_digest
    }

    pub fn work_ids(&self) -> &[ProjectionWorkId] {
        &self.work_ids
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPendingCursor {
    root: ContentDigest,
    after: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPendingPage {
    pending: Vec<ProjectionPendingActivation>,
    next: Option<ProjectionPendingCursor>,
}

impl ProjectionPendingPage {
    pub fn pending(&self) -> &[ProjectionPendingActivation] {
        &self.pending
    }

    pub const fn next(&self) -> Option<&ProjectionPendingCursor> {
        self.next.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectionWorkIndexStats {
    pub node_reads: usize,
    pub node_writes: usize,
    pub root_reads: usize,
    pub prepared_reads: usize,
    pub pending_entries_read: usize,
    pub preflight_nodes: usize,
    pub preflight_records: usize,
    pub preflight_roots: usize,
    pub preflight_bytes: usize,
}

#[derive(Debug, Default)]
struct ProjectionWorkCounters {
    node_reads: AtomicUsize,
    node_writes: AtomicUsize,
    root_reads: AtomicUsize,
    prepared_reads: AtomicUsize,
    pending_entries_read: AtomicUsize,
    preflight_nodes: AtomicUsize,
    preflight_records: AtomicUsize,
    preflight_roots: AtomicUsize,
    preflight_bytes: AtomicUsize,
}

pub struct ProjectionWorkIndex {
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    control: Dir,
    nodes: Dir,
    roots: Dir,
    prepared: Dir,
    transition: Mutex<()>,
    authoritative_head: Mutex<Option<ContentDigest>>,
    counters: ProjectionWorkCounters,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProjectionWorkPreflightStats {
    nodes: usize,
    records: usize,
    roots: usize,
    bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PreflightTree {
    Rows,
    Ready,
    Paths,
    Accepted,
    Pending,
}

impl fmt::Debug for ProjectionWorkIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectionWorkIndex")
            .field("workspace_id", &self.workspace_id)
            .field("endpoint_id", &self.endpoint_id)
            .finish_non_exhaustive()
    }
}

impl ProjectionWorkIndex {
    #[cfg(test)]
    pub(crate) fn preflight_existing(
        control: &Dir,
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
    ) -> Result<(), ProjectionWorkError> {
        Self::open_sealed_existing(
            control.try_clone()?,
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
        )
        .map(|_| ())
    }

    pub(crate) fn open_sealed_existing(
        control: Dir,
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
    ) -> Result<Self, ProjectionWorkError> {
        let head = read_optional_regular(&control, HEAD_FILE, 64, None)?;
        let claim = read_optional_regular(&control, CLAIM_FILE, 256, None)?;
        match (head, claim) {
            (None, None) => Err(ProjectionWorkError::MissingHead),
            (Some(head), Some(claim)) => {
                validate_projection_index_claim(
                    &claim,
                    workspace_id,
                    endpoint_id,
                    graph_resource_id,
                    receipt_store_id,
                )?;
                let nodes = open_dir_nofollow(&control, "nodes")?;
                let roots = open_dir_nofollow(&control, "roots")?;
                let prepared = open_dir_nofollow(&control, "prepared")?;
                let text =
                    std::str::from_utf8(&head).map_err(|_| ProjectionWorkError::NonCanonical)?;
                let digest = parse_digest(text)
                    .map(ContentDigest::from_bytes)
                    .map_err(|_| ProjectionWorkError::NonCanonical)?;
                if digest.to_string().as_bytes() != head {
                    return Err(ProjectionWorkError::NonCanonical);
                }
                let bytes = read_optional_regular(
                    &roots,
                    &root_filename(digest),
                    MAX_INDEX_NODE_BYTES,
                    None,
                )?
                .ok_or(ProjectionWorkError::MissingRoot(digest))?;
                if ContentDigest::of(&bytes) != digest {
                    return Err(ProjectionWorkError::RootDigestMismatch(digest));
                }
                let root: ProjectionRoot = decode_canonical(&bytes)?;
                validate_projection_root_binding(
                    &root,
                    workspace_id,
                    endpoint_id,
                    graph_resource_id,
                    receipt_store_id,
                )?;
                let index = Self {
                    workspace_id,
                    endpoint_id,
                    graph_resource_id,
                    receipt_store_id,
                    control,
                    nodes,
                    roots,
                    prepared,
                    transition: Mutex::new(()),
                    authoritative_head: Mutex::new(Some(digest)),
                    counters: ProjectionWorkCounters::default(),
                };
                let stats = index.preflight_reachable(&root)?;
                index.record_preflight(stats);
                Ok(index)
            }
            _ => Err(ProjectionWorkError::MissingHead),
        }
    }

    pub(crate) fn new(
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
        control: Dir,
        nodes: Dir,
        roots: Dir,
        prepared: Dir,
    ) -> Result<Self, ProjectionWorkError> {
        let index = Self {
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
            control,
            nodes,
            roots,
            prepared,
            transition: Mutex::new(()),
            authoritative_head: Mutex::new(None),
            counters: ProjectionWorkCounters::default(),
        };
        index.initialize()?;
        Ok(index)
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn endpoint_id(&self) -> ProjectionEndpointId {
        self.endpoint_id
    }

    pub const fn graph_resource_id(&self) -> super::CanonicalGraphResourceId {
        self.graph_resource_id
    }

    pub const fn receipt_store_id(&self) -> super::ProjectionReceiptStoreId {
        self.receipt_store_id
    }

    pub fn stats(&self) -> ProjectionWorkIndexStats {
        ProjectionWorkIndexStats {
            node_reads: self.counters.node_reads.load(Ordering::Relaxed),
            node_writes: self.counters.node_writes.load(Ordering::Relaxed),
            root_reads: self.counters.root_reads.load(Ordering::Relaxed),
            prepared_reads: self.counters.prepared_reads.load(Ordering::Relaxed),
            pending_entries_read: self.counters.pending_entries_read.load(Ordering::Relaxed),
            preflight_nodes: self.counters.preflight_nodes.load(Ordering::Relaxed),
            preflight_records: self.counters.preflight_records.load(Ordering::Relaxed),
            preflight_roots: self.counters.preflight_roots.load(Ordering::Relaxed),
            preflight_bytes: self.counters.preflight_bytes.load(Ordering::Relaxed),
        }
        }

    fn record_preflight(&self, stats: ProjectionWorkPreflightStats) {
        self.counters
            .preflight_nodes
            .store(stats.nodes, Ordering::Relaxed);
        self.counters
            .preflight_records
            .store(stats.records, Ordering::Relaxed);
        self.counters
            .preflight_roots
            .store(stats.roots, Ordering::Relaxed);
        self.counters
            .preflight_bytes
            .store(stats.bytes, Ordering::Relaxed);
    }

    /// Validate exactly the records reachable from the current authenticated
    /// root. Complexity is O(reachable nodes + reachable records + referenced
    /// pending-source roots/prepared batches); unrelated archived roots and
    /// prepared files are never enumerated.
    fn preflight_reachable(
        &self,
        root: &ProjectionRoot,
    ) -> Result<ProjectionWorkPreflightStats, ProjectionWorkError> {
        let mut stats = ProjectionWorkPreflightStats {
            roots: 1,
            ..ProjectionWorkPreflightStats::default()
        };
        charge_preflight(
            &mut stats.bytes,
            encode_canonical(root)?.len(),
            MAX_PREFLIGHT_BYTES,
        )?;
        let mut pending = vec![
            (PreflightTree::Rows, root.rows_root),
            (PreflightTree::Ready, root.ready_root),
            (PreflightTree::Paths, root.paths_root),
            (PreflightTree::Accepted, root.accepted_root),
            (PreflightTree::Pending, root.pending_root),
        ];
        let mut visited = std::collections::BTreeSet::new();
        while let Some((tree, digest)) = pending.pop() {
            if digest == empty_tree_root() || !visited.insert((tree, digest)) {
                continue;
            }
            let node = self.preflight_read_node(digest, &mut stats)?;
            match node {
                IndexNode::Branch { left, right, .. } => {
                    pending.push((tree, right));
                    pending.push((tree, left));
                }
                IndexNode::Leaf { value, .. } => {
                    charge_preflight(&mut stats.records, 1, MAX_PREFLIGHT_RECORDS)?;
                    match tree {
                        PreflightTree::Rows => {
                            let state: StoredWork = decode_canonical(&value)?;
                            if state.schema_version != INDEX_SCHEMA_VERSION {
                                return Err(ProjectionWorkError::BindingMismatch);
                            }
                            self.require_binding(&state.work)?;
                        }
                        PreflightTree::Ready => {
                            decode_work_id(&value)?;
                        }
                        PreflightTree::Paths => {
                            let ids: Vec<ProjectionWorkId> = decode_canonical(&value)?;
                            if !strictly_sorted(&ids) {
                                return Err(ProjectionWorkError::NonCanonical);
                            }
                        }
                        PreflightTree::Pending => {
                            let activation: ProjectionPendingActivation = decode_canonical(&value)?;
                            self.require_pending_binding(&activation)?;
                            self.preflight_prepared(&activation, &mut stats)?;
                        }
                        PreflightTree::Accepted => {
                            let witness: AcceptedBatchWitness = decode_canonical(&value)?;
                            self.preflight_accepted(&witness, &mut stats)?;
                        }
                    }
                }
            }
        }
        Ok(stats)
    }

    fn preflight_read_node(
        &self,
        digest: ContentDigest,
        stats: &mut ProjectionWorkPreflightStats,
    ) -> Result<IndexNode, ProjectionWorkError> {
        charge_preflight(&mut stats.nodes, 1, MAX_PREFLIGHT_NODES)?;
        let bytes = read_optional_regular(
            &self.nodes,
            &node_filename(digest),
            MAX_INDEX_NODE_BYTES,
            None,
        )?
        .ok_or(ProjectionWorkError::MissingNode(digest))?;
        charge_preflight(&mut stats.bytes, bytes.len(), MAX_PREFLIGHT_BYTES)?;
        if ContentDigest::of(&bytes) != digest {
            return Err(ProjectionWorkError::NodeDigestMismatch(digest));
        }
        let node: IndexNode = decode_canonical(&bytes)?;
        validate_node(&node)?;
        self.counters.node_reads.fetch_add(1, Ordering::Relaxed);
        Ok(node)
    }

    fn preflight_prepared(
        &self,
        pending: &ProjectionPendingActivation,
        stats: &mut ProjectionWorkPreflightStats,
    ) -> Result<(), ProjectionWorkError> {
        let prepared = self.load_prepared(pending.batch_id)?;
        let bytes = encode_canonical(&prepared)?;
        charge_preflight(&mut stats.bytes, bytes.len(), MAX_PREFLIGHT_BYTES)?;
        if prepared.manifest_fingerprint != pending.manifest_fingerprint
            || ContentDigest::of(&bytes) != pending.prepared_digest
            || prepared
                .work
                .iter()
                .map(ProjectionWork::work_id)
                .collect::<Vec<_>>()
                != pending.work_ids
        {
            return Err(ProjectionWorkError::PendingActivationMismatch);
        }
        Ok(())
    }

    fn preflight_accepted(
        &self,
        witness: &AcceptedBatchWitness,
        stats: &mut ProjectionWorkPreflightStats,
    ) -> Result<(), ProjectionWorkError> {
        if witness.schema_version != INDEX_SCHEMA_VERSION
            || witness.workspace_id != self.workspace_id
            || witness.endpoint_id != self.endpoint_id
            || !strictly_sorted(&witness.work_ids)
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        charge_preflight(&mut stats.roots, 1, MAX_PREFLIGHT_ROOTS)?;
        let source_root = self.load_root(witness.pending_root_digest)?;
        charge_preflight(
            &mut stats.bytes,
            encode_canonical(&source_root)?.len(),
            MAX_PREFLIGHT_BYTES,
        )?;
        let value = self
            .preflight_lookup(
                source_root.pending_root,
                &batch_key(witness.batch_id),
                stats,
            )?
            .ok_or(ProjectionWorkError::PendingActivationMissing)?;
        let pending: ProjectionPendingActivation = decode_canonical(&value)?;
        self.require_pending_binding(&pending)?;
        if witness.batch_id != pending.batch_id
            || witness.manifest_fingerprint != pending.manifest_fingerprint
            || witness.prepared_digest != pending.prepared_digest
            || witness.work_ids != pending.work_ids
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        self.preflight_prepared(&pending, stats)
    }

    fn preflight_lookup(
        &self,
        root: ContentDigest,
        key: &[u8],
        stats: &mut ProjectionWorkPreflightStats,
    ) -> Result<Option<Vec<u8>>, ProjectionWorkError> {
        validate_key(key)?;
        if root == empty_tree_root() {
            return Ok(None);
        }
        let mut digest = root;
        loop {
            match self.preflight_read_node(digest, stats)? {
                IndexNode::Leaf {
                    key: found, value, ..
                } => return Ok((found == key).then_some(value)),
                IndexNode::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    if !prefix_matches(key, &prefix, prefix_bit_len as usize) {
                        return Ok(None);
                    }
                    digest = if key_bit(key, prefix_bit_len as usize)? {
                        right
                    } else {
                        left
                    };
                }
            }
        }
    }

    pub(crate) fn validate_sealed_open(&self) -> Result<(), ProjectionWorkError> {
        let claim = read_optional_regular(&self.control, CLAIM_FILE, 256, None)?
            .ok_or(ProjectionWorkError::MissingHead)?;
        validate_projection_index_claim(
            &claim,
            self.workspace_id,
            self.endpoint_id,
            self.graph_resource_id,
            self.receipt_store_id,
        )?;
        let expected = self
            .authoritative_head
            .lock()
            .map_err(|_| ProjectionWorkError::Poisoned)?
            .ok_or(ProjectionWorkError::MissingHead)?;
        let (live, root) = self.read_live_head_root()?;
        if live != expected {
            return Err(ProjectionWorkError::ConcurrentRootTransition);
        }
        self.require_root_binding(&root)
    }

    pub(crate) fn validate_runtime_open(&self) -> Result<(), ProjectionWorkError> {
        let claim = read_optional_regular(&self.control, CLAIM_FILE, 256, None)?
            .ok_or(ProjectionWorkError::MissingHead)?;
        validate_projection_index_claim(
            &claim,
            self.workspace_id,
            self.endpoint_id,
            self.graph_resource_id,
            self.receipt_store_id,
        )?;
        let (_, root) = self.load_head_root()?;
        self.require_root_binding(&root)
    }

    pub(crate) fn prepare_batch(
        &self,
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        work: &[ProjectionWork],
        superseded: &[ProjectionWorkId],
    ) -> Result<(), ProjectionWorkError> {
        let mut work = work.to_vec();
        work.sort_unstable_by_key(ProjectionWork::work_id);
        if !strictly_sorted_by(&work, ProjectionWork::work_id) {
            return Err(ProjectionWorkError::NonCanonical);
        }
        for row in &work {
            self.require_binding(row)?;
            if row.batch_id() != batch_id || row.encode()?.len() as u64 > MAX_WORK_ROW_BYTES {
                return Err(ProjectionWorkError::BindingMismatch);
            }
        }
        let mut superseded = superseded.to_vec();
        superseded.sort_unstable();
        superseded.dedup();
        let prepared = PreparedBatch {
            schema_version: INDEX_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            endpoint_id: self.endpoint_id,
            batch_id,
            manifest_fingerprint,
            work,
            superseded,
        };
        let bytes = encode_canonical(&prepared)?;
        if bytes.len() as u64 > MAX_PREPARED_BATCH_BYTES {
            return Err(ProjectionWorkError::TooLarge(bytes.len()));
        }
        if read_optional_regular(
            &self.prepared,
            &prepared_filename(batch_id),
            MAX_PREPARED_BATCH_BYTES,
            None,
        )?
        .is_none()
        {
            let (_, root) = self.load_head_root()?;
            let key = batch_key(batch_id);
            if self.tree_lookup(root.pending_root, &key)?.is_some()
                || self.tree_lookup(root.accepted_root, &key)?.is_some()
            {
                return Err(ProjectionWorkError::MissingPreparedBatch(batch_id));
            }
        }
        publish_immutable_exact(
            &self.prepared,
            &prepared_filename(batch_id),
            &bytes,
            "prepared projection work batch",
        )?;
        let prepared_digest = ContentDigest::of(&bytes);
        let pending = ProjectionPendingActivation {
            schema_version: INDEX_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            endpoint_id: self.endpoint_id,
            batch_id,
            manifest_fingerprint,
            prepared_digest,
            work_ids: prepared.work.iter().map(ProjectionWork::work_id).collect(),
        };
        self.transition(|index, _, mut root| {
            let key = batch_key(batch_id);
            if let Some(existing) = index.tree_lookup(root.accepted_root, &key)? {
                let witness: AcceptedBatchWitness = decode_canonical(&existing)?;
                index.require_accepted_witness(&witness, &pending)?;
                return Ok(root);
            }
            if let Some(existing) = index.tree_lookup(root.pending_root, &key)? {
                let existing: ProjectionPendingActivation = decode_canonical(&existing)?;
                index.require_pending_binding(&existing)?;
                if existing != pending {
                    return Err(ProjectionWorkError::PendingActivationMismatch);
                }
                return Ok(root);
            }
            root.pending_root =
                index.tree_insert(root.pending_root, key, encode_canonical(&pending)?)?;
            Ok(root)
        })
    }

    pub(crate) fn accept_batch_at_history(
        &self,
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        engine_history_generation: u64,
        engine_history_root: ContentDigest,
    ) -> Result<(), ProjectionWorkError> {
        if engine_history_generation == 0
            || engine_history_root == super::object_store::EngineHistoryStore::empty_root()
        {
            return Err(ProjectionWorkError::HistoryBindingMismatch);
        }
        let prepared = self.load_prepared(batch_id)?;
        if prepared.manifest_fingerprint != manifest_fingerprint {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        let prepared_bytes = encode_canonical(&prepared)?;
        let prepared_digest = ContentDigest::of(&prepared_bytes);
        self.transition(|index, pending_root_digest, mut root| {
            let accepted_key = batch_key(batch_id);
            let expected_pending = ProjectionPendingActivation {
                schema_version: INDEX_SCHEMA_VERSION,
                workspace_id: index.workspace_id,
                endpoint_id: index.endpoint_id,
                batch_id,
                manifest_fingerprint,
                prepared_digest,
                work_ids: prepared.work.iter().map(ProjectionWork::work_id).collect(),
            };
            let witness = AcceptedBatchWitness {
                schema_version: INDEX_SCHEMA_VERSION,
                workspace_id: index.workspace_id,
                endpoint_id: index.endpoint_id,
                batch_id,
                manifest_fingerprint,
                prepared_digest,
                work_ids: prepared.work.iter().map(ProjectionWork::work_id).collect(),
                pending_root_digest,
            };
            if let Some(existing) = index.tree_lookup(root.accepted_root, &accepted_key)? {
                let existing = decode_canonical::<AcceptedBatchWitness>(&existing)?;
                index.require_accepted_witness(&existing, &expected_pending)?;
                if index
                    .tree_lookup(root.pending_root, &accepted_key)?
                    .is_some()
                {
                    return Err(ProjectionWorkError::PendingActivationMismatch);
                }
                root.engine_history_generation = engine_history_generation;
                root.engine_history_root = engine_history_root;
                return Ok(root);
            }
            let pending = index
                .tree_lookup(root.pending_root, &accepted_key)?
                .ok_or(ProjectionWorkError::PendingActivationMissing)?;
            let pending: ProjectionPendingActivation = decode_canonical(&pending)?;
            index.require_pending_binding(&pending)?;
            if pending != expected_pending {
                return Err(ProjectionWorkError::PendingActivationMismatch);
            }

            for work_id in &prepared.superseded {
                let Some(mut state) = index.load_state(&root, *work_id)? else {
                    return Err(ProjectionWorkError::MissingWork(*work_id));
                };
                if !matches!(state.status, StoredWorkStatus::Ready) {
                    continue;
                }
                let by = prepared
                    .work
                    .iter()
                    .find(|candidate| {
                        candidate.path() == state.work.path()
                            && candidate.work_id() != state.work.work_id()
                    })
                    .map(ProjectionWork::work_id)
                    .ok_or(ProjectionWorkError::BindingMismatch)?;
                root = index.remove_ready(root, &state.work)?;
                state.status = StoredWorkStatus::Superseded {
                    by,
                    accepted_batch: batch_id,
                    manifest_fingerprint,
                    engine_history_root,
                };
                root.rows_root = index.tree_insert(
                    root.rows_root,
                    work_key(*work_id),
                    encode_canonical(&state)?,
                )?;
            }

            for work in &prepared.work {
                if let Some(existing) = index.load_state(&root, work.work_id())? {
                    if existing.work != *work || !matches!(existing.status, StoredWorkStatus::Ready)
                    {
                        return Err(ProjectionWorkError::ConflictingStatus);
                    }
                    continue;
                }
                let state = StoredWork {
                    schema_version: INDEX_SCHEMA_VERSION,
                    work: work.clone(),
                    status: StoredWorkStatus::Ready,
                };
                root.rows_root = index.tree_insert(
                    root.rows_root,
                    work_key(work.work_id()),
                    encode_canonical(&state)?,
                )?;
                root.ready_root = index.tree_insert(
                    root.ready_root,
                    ready_key(work)?,
                    work.work_id().as_bytes().to_vec(),
                )?;
                root = index.add_path_work(root, work)?;
            }
            root.accepted_root = index.tree_insert(
                root.accepted_root,
                accepted_key.clone(),
                encode_canonical(&witness)?,
            )?;
            root.pending_root = index.tree_remove(root.pending_root, &accepted_key)?;
            root.engine_history_generation = engine_history_generation;
            root.engine_history_root = engine_history_root;
            Ok(root)
        })
    }

    #[cfg(test)]
    pub(crate) fn accept_batch(
        &self,
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
    ) -> Result<(), ProjectionWorkError> {
        self.accept_batch_at_history(
            batch_id,
            manifest_fingerprint,
            1,
            ContentDigest::of(b"projection-work-test-history-root"),
        )
    }

    pub fn pending_activation_page(
        &self,
        cursor: Option<&ProjectionPendingCursor>,
        limit: usize,
    ) -> Result<ProjectionPendingPage, ProjectionWorkError> {
        if limit == 0 || limit > MAX_PENDING_PAGE {
            return Err(ProjectionWorkError::InvalidPageLimit(limit));
        }
        let (root_digest, root) = match cursor {
            Some(cursor) => (cursor.root, self.load_root(cursor.root)?),
            None => self.load_head_root()?,
        };
        let mut after = cursor.map(|cursor| cursor.after.clone());
        let mut pending = Vec::with_capacity(limit);
        let mut has_more = false;
        while pending.len() <= limit {
            let Some((key, value)) = self.tree_first_after(root.pending_root, after.as_deref())?
            else {
                break;
            };
            let record: ProjectionPendingActivation = decode_canonical(&value)?;
            self.require_pending_binding(&record)?;
            if batch_key(record.batch_id) != key {
                return Err(ProjectionWorkError::PendingActivationMismatch);
            }
            self.require_pending_prepared(&record)?;
            self.counters
                .pending_entries_read
                .fetch_add(1, Ordering::Relaxed);
            after = Some(key);
            if pending.len() == limit {
                has_more = true;
                break;
            }
            pending.push(record);
        }
        let next = has_more.then(|| ProjectionPendingCursor {
            root: root_digest,
            after: batch_key(
                pending
                    .last()
                    .expect("full pending page has a last row")
                    .batch_id,
            ),
        });
        Ok(ProjectionPendingPage { pending, next })
    }

    pub(crate) fn retire_pending_activation_at_history(
        &self,
        pending: &ProjectionPendingActivation,
        engine_history_generation: u64,
        engine_history_root: ContentDigest,
    ) -> Result<(), ProjectionWorkError> {
        self.require_pending_binding(pending)?;
        self.require_pending_prepared(pending)?;
        self.transition(|index, _, mut root| {
            let key = batch_key(pending.batch_id);
            let existing = index
                .tree_lookup(root.pending_root, &key)?
                .ok_or(ProjectionWorkError::PendingActivationMissing)?;
            let existing: ProjectionPendingActivation = decode_canonical(&existing)?;
            index.require_pending_binding(&existing)?;
            if existing != *pending {
                return Err(ProjectionWorkError::PendingActivationMismatch);
            }
            root.pending_root = index.tree_remove(root.pending_root, &key)?;
            root.engine_history_generation = engine_history_generation;
            root.engine_history_root = engine_history_root;
            Ok(root)
        })
    }

    #[cfg(test)]
    pub(crate) fn retire_pending_activation(
        &self,
        pending: &ProjectionPendingActivation,
    ) -> Result<(), ProjectionWorkError> {
        self.retire_pending_activation_at_history(
            pending,
            1,
            ContentDigest::of(b"projection-work-test-history-root"),
        )
    }

    pub(crate) fn require_current_history_binding(
        &self,
        engine_history_generation: u64,
        engine_history_root: ContentDigest,
    ) -> Result<(), ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        if root.engine_history_generation != engine_history_generation
            || root.engine_history_root != engine_history_root
        {
            return Err(ProjectionWorkError::HistoryBindingMismatch);
        }
        Ok(())
    }

    pub fn get(
        &self,
        work_id: ProjectionWorkId,
    ) -> Result<Option<ProjectionWork>, ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        Ok(self.load_state(&root, work_id)?.map(|state| state.work))
    }

    pub fn status(
        &self,
        work_id: ProjectionWorkId,
    ) -> Result<Option<ProjectionWorkStatus>, ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        Ok(self
            .load_state(&root, work_id)?
            .map(|state| state.status.into_public()))
    }

    pub(crate) fn completed_release(
        &self,
        batch_id: BatchId,
        manifest_fingerprint: ContentDigest,
        page_id: PageId,
        path: &ManagedPath,
    ) -> Result<ProjectionWork, ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        let bytes = self
            .tree_lookup(root.accepted_root, &batch_key(batch_id))?
            .ok_or(ProjectionWorkError::AcceptedWitnessMissing)?;
        let witness: AcceptedBatchWitness = decode_canonical(&bytes)?;
        if witness.schema_version != INDEX_SCHEMA_VERSION
            || witness.workspace_id != self.workspace_id
            || witness.endpoint_id != self.endpoint_id
            || witness.batch_id != batch_id
            || witness.manifest_fingerprint != manifest_fingerprint
            || !strictly_sorted(&witness.work_ids)
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        let mut found = None;
        for work_id in witness.work_ids {
            let state = self
                .load_state(&root, work_id)?
                .ok_or(ProjectionWorkError::MissingWork(work_id))?;
            self.require_binding(&state.work)?;
            if state.work.page_id() != page_id
                || state.work.path() != path
                || state.work.target() != ProjectionWorkTarget::Absent
            {
                continue;
            }
            if found.is_some() {
                return Err(ProjectionWorkError::AcceptedWitnessMismatch);
            }
            if !matches!(state.status, StoredWorkStatus::Completed { .. }) {
                return Err(ProjectionWorkError::ConflictingStatus);
            }
            found = Some(state.work);
        }
        found.ok_or(ProjectionWorkError::AcceptedWitnessMissing)
    }

    pub fn next(&self) -> Result<Option<ProjectionWork>, ProjectionWorkError> {
        Ok(self.ready_page(None, 1)?.work.into_iter().next())
    }

    pub fn ready_page(
        &self,
        cursor: Option<&ProjectionWorkCursor>,
        limit: usize,
    ) -> Result<ProjectionWorkPage, ProjectionWorkError> {
        if limit == 0 || limit > MAX_READY_PAGE {
            return Err(ProjectionWorkError::InvalidPageLimit(limit));
        }
        let (root_digest, root) = match cursor {
            Some(cursor) => (cursor.root, self.load_root(cursor.root)?),
            None => self.load_head_root()?,
        };
        let mut after = cursor.map(|cursor| cursor.after.clone());
        let mut work = Vec::with_capacity(limit);
        let mut has_more = false;
        while work.len() <= limit {
            let Some((key, value)) = self.tree_first_after(root.ready_root, after.as_deref())?
            else {
                break;
            };
            let work_id = decode_work_id(&value)?;
            let state = self
                .load_state(&root, work_id)?
                .ok_or(ProjectionWorkError::MissingReadyRow)?;
            if !matches!(state.status, StoredWorkStatus::Ready) || ready_key(&state.work)? != key {
                return Err(ProjectionWorkError::MissingReadyRow);
            }
            after = Some(key);
            if work.len() == limit {
                has_more = true;
                break;
            }
            work.push(state.work);
        }
        let next = has_more.then(|| ProjectionWorkCursor {
            root: root_digest,
            after: ready_key(work.last().expect("full page has a last row"))
                .expect("validated work key"),
        });
        Ok(ProjectionWorkPage { work, next })
    }

    pub fn pending_for_path(
        &self,
        path: &ManagedPath,
    ) -> Result<Vec<ProjectionWork>, ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        let Some(bytes) = self.tree_lookup(root.paths_root, &path_key(path))? else {
            return Ok(Vec::new());
        };
        let ids: Vec<ProjectionWorkId> = decode_canonical(&bytes)?;
        if !strictly_sorted(&ids) {
            return Err(ProjectionWorkError::NonCanonical);
        }
        let mut work = Vec::with_capacity(ids.len());
        for work_id in ids {
            let state = self
                .load_state(&root, work_id)?
                .ok_or(ProjectionWorkError::MissingReadyRow)?;
            if !matches!(state.status, StoredWorkStatus::Ready) || state.work.path() != path {
                return Err(ProjectionWorkError::BindingMismatch);
            }
            work.push(state.work);
        }
        work.sort_unstable_by_key(|row| (row.batch_id(), row.work_id()));
        Ok(work)
    }

    pub(crate) fn require_accepted_ready(
        &self,
        work: &ProjectionWork,
        manifest_fingerprint: ContentDigest,
    ) -> Result<(), ProjectionWorkError> {
        self.require_binding(work)?;
        let (_, root) = self.load_head_root()?;
        let state = self
            .load_state(&root, work.work_id())?
            .ok_or(ProjectionWorkError::MissingWork(work.work_id()))?;
        if state.work != *work || !matches!(state.status, StoredWorkStatus::Ready) {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        let bytes = self
            .tree_lookup(root.accepted_root, &batch_key(work.batch_id()))?
            .ok_or(ProjectionWorkError::AcceptedWitnessMissing)?;
        let witness: AcceptedBatchWitness = decode_canonical(&bytes)?;
        if witness.schema_version != INDEX_SCHEMA_VERSION
            || witness.workspace_id != self.workspace_id
            || witness.endpoint_id != self.endpoint_id
            || witness.batch_id != work.batch_id()
            || witness.manifest_fingerprint != manifest_fingerprint
            || witness.work_ids.binary_search(&work.work_id()).is_err()
            || !strictly_sorted(&witness.work_ids)
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        let source_root = self.load_root(witness.pending_root_digest)?;
        let source = self
            .tree_lookup(source_root.pending_root, &batch_key(work.batch_id()))?
            .ok_or(ProjectionWorkError::PendingActivationMissing)?;
        let source: ProjectionPendingActivation = decode_canonical(&source)?;
        self.require_pending_binding(&source)?;
        self.require_pending_prepared(&source)?;
        if source.batch_id != witness.batch_id
            || source.manifest_fingerprint != witness.manifest_fingerprint
            || source.prepared_digest != witness.prepared_digest
            || source.work_ids != witness.work_ids
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        Ok(())
    }

    pub fn accepted_preparation_root(
        &self,
        batch_id: BatchId,
    ) -> Result<ContentDigest, ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        let bytes = self
            .tree_lookup(root.accepted_root, &batch_key(batch_id))?
            .ok_or(ProjectionWorkError::AcceptedWitnessMissing)?;
        let witness: AcceptedBatchWitness = decode_canonical(&bytes)?;
        if witness.schema_version != INDEX_SCHEMA_VERSION
            || witness.workspace_id != self.workspace_id
            || witness.endpoint_id != self.endpoint_id
            || witness.batch_id != batch_id
            || !strictly_sorted(&witness.work_ids)
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        let pending_root = self.load_root(witness.pending_root_digest)?;
        let pending = self
            .tree_lookup(pending_root.pending_root, &batch_key(batch_id))?
            .ok_or(ProjectionWorkError::PendingActivationMissing)?;
        let pending: ProjectionPendingActivation = decode_canonical(&pending)?;
        self.require_pending_binding(&pending)?;
        self.require_pending_prepared(&pending)?;
        if pending.manifest_fingerprint != witness.manifest_fingerprint
            || pending.prepared_digest != witness.prepared_digest
            || pending.work_ids != witness.work_ids
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        Ok(witness.pending_root_digest)
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn mark_completed(
        &self,
        authority: ProjectionWorkCompletionAuthority,
    ) -> Result<(), ProjectionWorkError> {
        self.transition(|index, _, mut root| {
            let Some(mut state) = index.load_state(&root, authority.work_id)? else {
                return Err(ProjectionWorkError::MissingWork(authority.work_id));
            };
            index.require_completion_authority(&state.work, &authority)?;
            let terminal = StoredWorkStatus::Completed {
                intent_id: authority.intent_id,
                logical_completion_id: authority.logical_completion_id,
            };
            if state.status == terminal {
                return Ok(root);
            }
            if !matches!(state.status, StoredWorkStatus::Ready) {
                return Err(ProjectionWorkError::ConflictingStatus);
            }
            root = index.remove_ready(root, &state.work)?;
            state.status = terminal;
            root.rows_root = index.tree_insert(
                root.rows_root,
                work_key(authority.work_id),
                encode_canonical(&state)?,
            )?;
            Ok(root)
        })
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn require_completed(
        &self,
        authority: &ProjectionWorkCompletionAuthority,
    ) -> Result<(), ProjectionWorkError> {
        let (_, root) = self.load_head_root()?;
        let state = self
            .load_state(&root, authority.work_id)?
            .ok_or(ProjectionWorkError::MissingWork(authority.work_id))?;
        self.require_completion_authority(&state.work, authority)?;
        if state.status
            != (StoredWorkStatus::Completed {
                intent_id: authority.intent_id,
                logical_completion_id: authority.logical_completion_id,
            })
        {
            return Err(ProjectionWorkError::ConflictingStatus);
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn mark_blocked(
        &self,
        authority: ProjectionWorkBlockAuthority,
    ) -> Result<(), ProjectionWorkError> {
        self.transition(|index, _, mut root| {
            let Some(mut state) = index.load_state(&root, authority.work_id)? else {
                return Err(ProjectionWorkError::MissingWork(authority.work_id));
            };
            index.require_block_authority(&state.work, &authority)?;
            let terminal = StoredWorkStatus::Blocked {
                observed: authority.observed,
            };
            if state.status == terminal {
                return Ok(root);
            }
            if !matches!(state.status, StoredWorkStatus::Ready) {
                return Err(ProjectionWorkError::ConflictingStatus);
            }
            root = index.remove_ready(root, &state.work)?;
            state.status = terminal;
            root.rows_root = index.tree_insert(
                root.rows_root,
                work_key(authority.work_id),
                encode_canonical(&state)?,
            )?;
            Ok(root)
        })
    }

    #[allow(clippy::result_large_err)]
    fn require_completion_authority(
        &self,
        work: &ProjectionWork,
        authority: &ProjectionWorkCompletionAuthority,
    ) -> Result<(), ProjectionWorkError> {
        if authority.workspace_id != self.workspace_id
            || authority.endpoint_id != self.endpoint_id
            || authority.graph_resource_id != self.graph_resource_id
            || authority.receipt_store_id != self.receipt_store_id
            || authority.work_id != work.work_id()
            || authority.page_id != work.page_id()
            || authority.path != *work.path()
            || authority.target != work.target()
        {
            return Err(ProjectionWorkError::BindingMismatch);
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn require_block_authority(
        &self,
        work: &ProjectionWork,
        authority: &ProjectionWorkBlockAuthority,
    ) -> Result<(), ProjectionWorkError> {
        if authority.workspace_id != self.workspace_id
            || authority.endpoint_id != self.endpoint_id
            || authority.graph_resource_id != self.graph_resource_id
            || authority.receipt_store_id != self.receipt_store_id
            || authority.work_id != work.work_id()
            || authority.page_id != work.page_id()
            || authority.path != *work.path()
            || authority.target != work.target()
        {
            return Err(ProjectionWorkError::BindingMismatch);
        }
        Ok(())
    }

    fn initialize(&self) -> Result<(), ProjectionWorkError> {
        let head = read_optional_regular(&self.control, HEAD_FILE, 64, None)?;
        let claim = read_optional_regular(&self.control, CLAIM_FILE, 256, None)?;
        match (head, claim) {
            (None, None) => {
                let empty = ProjectionRoot::empty(
                    self.workspace_id,
                    self.endpoint_id,
                    self.graph_resource_id,
                    self.receipt_store_id,
                );
                let empty_digest = self.publish_root(&empty)?;
                publish_immutable_exact(
                    &self.control,
                    HEAD_FILE,
                    empty_digest.to_string().as_bytes(),
                    "projection work root head",
                )?;
                let expected_claim = encode_canonical(&ProjectionIndexClaim {
                    schema_version: INDEX_SCHEMA_VERSION,
                    workspace_id: self.workspace_id,
                    endpoint_id: self.endpoint_id,
                    graph_resource_id: self.graph_resource_id,
                    receipt_store_id: self.receipt_store_id,
                })?;
                publish_immutable_exact(
                    &self.control,
                    CLAIM_FILE,
                    &expected_claim,
                    "projection work index claim",
                )?;
            }
            (Some(_), Some(claim)) => validate_projection_index_claim(
                &claim,
                self.workspace_id,
                self.endpoint_id,
                self.graph_resource_id,
                self.receipt_store_id,
            )?,
            _ => return Err(ProjectionWorkError::MissingHead),
        }
        let (_, root) = self.read_live_head_root()?;
        self.require_root_binding(&root)?;
        Ok(())
    }

    fn transition(
        &self,
        update: impl FnOnce(
            &Self,
            ContentDigest,
            ProjectionRoot,
        ) -> Result<ProjectionRoot, ProjectionWorkError>,
    ) -> Result<(), ProjectionWorkError> {
        let _guard = self
            .transition
            .lock()
            .map_err(|_| ProjectionWorkError::Poisoned)?;
        let (before_digest, before) = self.load_head_root()?;
        let mut after = update(self, before_digest, before.clone())?;
        if after == before {
            return Ok(());
        }
        after.generation = before
            .generation
            .checked_add(1)
            .ok_or(ProjectionWorkError::NonCanonical)?;
        self.require_root_binding(&after)?;
        let after_digest = self.publish_root(&after)?;
        self.replace_head(before_digest, after_digest)
    }

    fn replace_head(
        &self,
        expected: ContentDigest,
        replacement: ContentDigest,
    ) -> Result<(), ProjectionWorkError> {
        let current = self.read_head_digest()?;
        if current != expected {
            return Err(ProjectionWorkError::ConcurrentRootTransition);
        }
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut temp = self.control.open_with(&temp_name, &options)?;
        let result = (|| {
            temp.write_all(replacement.to_string().as_bytes())?;
            temp.sync_all()?;
            drop(temp);
            self.control.rename(&temp_name, &self.control, HEAD_FILE)?;
            sync_dir_required(&self.control)?;
            Ok::<_, ProjectionWorkError>(())
        })();
        let cleanup = self.control.remove_file(&temp_name);
        if let Err(error) = result {
            let _ = cleanup;
            return Err(error);
        }
        if cleanup
            .as_ref()
            .is_err_and(|error| error.kind() != ErrorKind::NotFound)
        {
            cleanup?;
        }
        *self
            .authoritative_head
            .lock()
            .map_err(|_| ProjectionWorkError::Poisoned)? = Some(replacement);
        Ok(())
    }

    fn load_head_root(&self) -> Result<(ContentDigest, ProjectionRoot), ProjectionWorkError> {
        let sealed = self
            .authoritative_head
            .lock()
            .map_err(|_| ProjectionWorkError::Poisoned)?
            .to_owned();
        match sealed {
            Some(expected) => {
                let (live, root) = self.read_live_head_root()?;
                if live != expected {
                    return Err(ProjectionWorkError::ConcurrentRootTransition);
                }
                Ok((live, root))
            }
            None => self.read_live_head_root(),
        }
    }

    fn read_live_head_root(&self) -> Result<(ContentDigest, ProjectionRoot), ProjectionWorkError> {
        let digest = self.read_head_digest()?;
        Ok((digest, self.load_root(digest)?))
    }

    fn read_head_digest(&self) -> Result<ContentDigest, ProjectionWorkError> {
        let bytes = read_optional_regular(&self.control, HEAD_FILE, 64, None)?
            .ok_or(ProjectionWorkError::MissingHead)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| ProjectionWorkError::NonCanonical)?;
        let digest = parse_digest(text)
            .map(ContentDigest::from_bytes)
            .map_err(|_| ProjectionWorkError::NonCanonical)?;
        if digest.to_string().as_bytes() != bytes {
            return Err(ProjectionWorkError::NonCanonical);
        }
        Ok(digest)
    }

    fn publish_root(&self, root: &ProjectionRoot) -> Result<ContentDigest, ProjectionWorkError> {
        self.require_root_binding(root)?;
        let bytes = encode_canonical(root)?;
        let digest = ContentDigest::of(&bytes);
        publish_immutable_exact(
            &self.roots,
            &root_filename(digest),
            &bytes,
            "projection work authenticated root",
        )?;
        Ok(digest)
    }

    fn load_root(&self, digest: ContentDigest) -> Result<ProjectionRoot, ProjectionWorkError> {
        let bytes = read_optional_regular(
            &self.roots,
            &root_filename(digest),
            MAX_INDEX_NODE_BYTES,
            None,
        )?
        .ok_or(ProjectionWorkError::MissingRoot(digest))?;
        if ContentDigest::of(&bytes) != digest {
            return Err(ProjectionWorkError::RootDigestMismatch(digest));
        }
        let root: ProjectionRoot = decode_canonical(&bytes)?;
        self.require_root_binding(&root)?;
        self.counters.root_reads.fetch_add(1, Ordering::Relaxed);
        Ok(root)
    }

    fn require_root_binding(&self, root: &ProjectionRoot) -> Result<(), ProjectionWorkError> {
        validate_projection_root_binding(
            root,
            self.workspace_id,
            self.endpoint_id,
            self.graph_resource_id,
            self.receipt_store_id,
        )
    }

    fn load_prepared(&self, batch_id: BatchId) -> Result<PreparedBatch, ProjectionWorkError> {
        let bytes = read_optional_regular(
            &self.prepared,
            &prepared_filename(batch_id),
            MAX_PREPARED_BATCH_BYTES,
            None,
        )?
        .ok_or(ProjectionWorkError::MissingPreparedBatch(batch_id))?;
        let prepared: PreparedBatch = decode_canonical(&bytes)?;
        if prepared.schema_version != INDEX_SCHEMA_VERSION
            || prepared.workspace_id != self.workspace_id
            || prepared.endpoint_id != self.endpoint_id
            || prepared.batch_id != batch_id
            || !strictly_sorted_by(&prepared.work, ProjectionWork::work_id)
            || !strictly_sorted(&prepared.superseded)
        {
            return Err(ProjectionWorkError::BindingMismatch);
        }
        for work in &prepared.work {
            self.require_binding(work)?;
            if work.batch_id() != batch_id {
                return Err(ProjectionWorkError::BindingMismatch);
            }
        }
        self.counters.prepared_reads.fetch_add(1, Ordering::Relaxed);
        Ok(prepared)
    }

    fn load_state(
        &self,
        root: &ProjectionRoot,
        work_id: ProjectionWorkId,
    ) -> Result<Option<StoredWork>, ProjectionWorkError> {
        self.tree_lookup(root.rows_root, &work_key(work_id))?
            .map(|bytes| {
                let state: StoredWork = decode_canonical(&bytes)?;
                if state.schema_version != INDEX_SCHEMA_VERSION || state.work.work_id() != work_id {
                    return Err(ProjectionWorkError::BindingMismatch);
                }
                self.require_binding(&state.work)?;
                Ok(state)
            })
            .transpose()
    }

    fn add_path_work(
        &self,
        mut root: ProjectionRoot,
        work: &ProjectionWork,
    ) -> Result<ProjectionRoot, ProjectionWorkError> {
        let key = path_key(work.path());
        let mut ids: Vec<ProjectionWorkId> = self
            .tree_lookup(root.paths_root, &key)?
            .map(|bytes| decode_canonical(&bytes))
            .transpose()?
            .unwrap_or_default();
        match ids.binary_search(&work.work_id()) {
            Ok(_) => {}
            Err(index) => ids.insert(index, work.work_id()),
        }
        root.paths_root = self.tree_insert(root.paths_root, key, encode_canonical(&ids)?)?;
        Ok(root)
    }

    fn remove_ready(
        &self,
        mut root: ProjectionRoot,
        work: &ProjectionWork,
    ) -> Result<ProjectionRoot, ProjectionWorkError> {
        root.ready_root = self.tree_remove(root.ready_root, &ready_key(work)?)?;
        let key = path_key(work.path());
        let bytes = self
            .tree_lookup(root.paths_root, &key)?
            .ok_or(ProjectionWorkError::MissingReadyRow)?;
        let mut ids: Vec<ProjectionWorkId> = decode_canonical(&bytes)?;
        let index = ids
            .binary_search(&work.work_id())
            .map_err(|_| ProjectionWorkError::MissingReadyRow)?;
        ids.remove(index);
        root.paths_root = if ids.is_empty() {
            self.tree_remove(root.paths_root, &key)?
        } else {
            self.tree_insert(root.paths_root, key, encode_canonical(&ids)?)?
        };
        Ok(root)
    }

    fn require_binding(&self, work: &ProjectionWork) -> Result<(), ProjectionWorkError> {
        if work.schema_version != WORK_SCHEMA_VERSION
            || work.workspace_id != self.workspace_id
            || work.endpoint_id != self.endpoint_id
            || work.graph_resource_id != self.graph_resource_id
            || work.portable_path_key_version != PORTABLE_PATH_KEY_VERSION
            || work.portable_path_key_digest != work.path.portable_key().digest()
            || work.work_id
                != work_id(
                    work.endpoint_id,
                    work.graph_resource_id,
                    work.batch_id,
                    work.page_id,
                    &work.path,
                    work.portable_path_key_digest,
                    work.portable_path_index_root,
                )
        {
            return Err(ProjectionWorkError::BindingMismatch);
        }
        Ok(())
    }

    fn require_pending_binding(
        &self,
        pending: &ProjectionPendingActivation,
    ) -> Result<(), ProjectionWorkError> {
        if pending.schema_version != INDEX_SCHEMA_VERSION
            || pending.workspace_id != self.workspace_id
            || pending.endpoint_id != self.endpoint_id
            || !strictly_sorted(&pending.work_ids)
        {
            return Err(ProjectionWorkError::PendingActivationMismatch);
        }
        Ok(())
    }

    fn require_pending_prepared(
        &self,
        pending: &ProjectionPendingActivation,
    ) -> Result<PreparedBatch, ProjectionWorkError> {
        let prepared = self.load_prepared(pending.batch_id)?;
        let bytes = encode_canonical(&prepared)?;
        if prepared.manifest_fingerprint != pending.manifest_fingerprint
            || ContentDigest::of(&bytes) != pending.prepared_digest
            || prepared
                .work
                .iter()
                .map(ProjectionWork::work_id)
                .collect::<Vec<_>>()
                != pending.work_ids
        {
            return Err(ProjectionWorkError::PendingActivationMismatch);
        }
        Ok(prepared)
    }

    fn require_accepted_witness(
        &self,
        witness: &AcceptedBatchWitness,
        pending: &ProjectionPendingActivation,
    ) -> Result<(), ProjectionWorkError> {
        if witness.schema_version != INDEX_SCHEMA_VERSION
            || witness.workspace_id != self.workspace_id
            || witness.endpoint_id != self.endpoint_id
            || witness.batch_id != pending.batch_id
            || witness.manifest_fingerprint != pending.manifest_fingerprint
            || witness.prepared_digest != pending.prepared_digest
            || witness.work_ids != pending.work_ids
            || !strictly_sorted(&witness.work_ids)
        {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        let source_root = self.load_root(witness.pending_root_digest)?;
        let source = self
            .tree_lookup(source_root.pending_root, &batch_key(pending.batch_id))?
            .ok_or(ProjectionWorkError::PendingActivationMissing)?;
        let source: ProjectionPendingActivation = decode_canonical(&source)?;
        self.require_pending_binding(&source)?;
        if source != *pending {
            return Err(ProjectionWorkError::AcceptedWitnessMismatch);
        }
        Ok(())
    }

    fn tree_lookup(
        &self,
        root: ContentDigest,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, ProjectionWorkError> {
        validate_key(key)?;
        if root == empty_tree_root() {
            return Ok(None);
        }
        let mut digest = root;
        loop {
            match self.read_node(digest)? {
                IndexNode::Leaf {
                    key: found, value, ..
                } => return Ok((found == key).then_some(value)),
                IndexNode::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    if !prefix_matches(key, &prefix, prefix_bit_len as usize) {
                        return Ok(None);
                    }
                    digest = if key_bit(key, prefix_bit_len as usize)? {
                        right
                    } else {
                        left
                    };
                }
            }
        }
    }

    fn tree_insert(
        &self,
        root: ContentDigest,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<ContentDigest, ProjectionWorkError> {
        validate_record(&key, &value)?;
        if root == empty_tree_root() {
            return self.publish_node(&IndexNode::Leaf {
                schema_version: INDEX_SCHEMA_VERSION,
                key,
                value,
            });
        }
        self.insert_at(root, &key, &value)
    }

    fn insert_at(
        &self,
        digest: ContentDigest,
        key: &[u8],
        value: &[u8],
    ) -> Result<ContentDigest, ProjectionWorkError> {
        let node = self.read_node(digest)?;
        let prefix = node.prefix();
        let prefix_bits = node.prefix_bits();
        let shared = common_prefix_bits(key, prefix, prefix_bits);
        if shared < prefix_bits {
            let leaf = self.publish_node(&IndexNode::Leaf {
                schema_version: INDEX_SCHEMA_VERSION,
                key: key.to_vec(),
                value: value.to_vec(),
            })?;
            return self.publish_split(key, shared, digest, prefix, leaf);
        }
        match node {
            IndexNode::Leaf {
                key: found,
                value: found_value,
                ..
            } => {
                if found == key {
                    if found_value == value {
                        return Ok(digest);
                    }
                    return self.publish_node(&IndexNode::Leaf {
                        schema_version: INDEX_SCHEMA_VERSION,
                        key: key.to_vec(),
                        value: value.to_vec(),
                    });
                }
                let shared = common_prefix_bits(key, &found, key.len() * 8);
                let leaf = self.publish_node(&IndexNode::Leaf {
                    schema_version: INDEX_SCHEMA_VERSION,
                    key: key.to_vec(),
                    value: value.to_vec(),
                })?;
                self.publish_split(key, shared, digest, &found, leaf)
            }
            IndexNode::Branch {
                prefix,
                prefix_bit_len,
                left,
                right,
                ..
            } => {
                let split = prefix_bit_len as usize;
                let (left, right) = if key_bit(key, split)? {
                    (left, self.insert_at(right, key, value)?)
                } else {
                    (self.insert_at(left, key, value)?, right)
                };
                self.publish_branch(prefix, prefix_bit_len, left, right)
            }
        }
    }

    fn publish_split(
        &self,
        key: &[u8],
        shared: usize,
        existing: ContentDigest,
        existing_prefix: &[u8],
        leaf: ContentDigest,
    ) -> Result<ContentDigest, ProjectionWorkError> {
        let key_right = key_bit(key, shared)?;
        if key_right == key_bit(existing_prefix, shared)? {
            return Err(ProjectionWorkError::NonCanonical);
        }
        let (left, right) = if key_right {
            (existing, leaf)
        } else {
            (leaf, existing)
        };
        self.publish_branch(
            masked_prefix(key, shared),
            u16::try_from(shared).map_err(|_| ProjectionWorkError::NonCanonical)?,
            left,
            right,
        )
    }

    fn publish_branch(
        &self,
        prefix: Vec<u8>,
        prefix_bit_len: u16,
        left: ContentDigest,
        right: ContentDigest,
    ) -> Result<ContentDigest, ProjectionWorkError> {
        let left_node = self.read_node(left)?;
        let right_node = self.read_node(right)?;
        self.publish_node(&IndexNode::Branch {
            schema_version: INDEX_SCHEMA_VERSION,
            prefix,
            prefix_bit_len,
            key_min: left_node.key_min().to_vec(),
            key_max: right_node.key_max().to_vec(),
            left,
            right,
        })
    }

    fn tree_remove(
        &self,
        root: ContentDigest,
        key: &[u8],
    ) -> Result<ContentDigest, ProjectionWorkError> {
        if root == empty_tree_root() {
            return Ok(root);
        }
        Ok(self.remove_at(root, key)?.unwrap_or_else(empty_tree_root))
    }

    fn remove_at(
        &self,
        digest: ContentDigest,
        key: &[u8],
    ) -> Result<Option<ContentDigest>, ProjectionWorkError> {
        match self.read_node(digest)? {
            IndexNode::Leaf { key: found, .. } => Ok((found != key).then_some(digest)),
            IndexNode::Branch {
                prefix,
                prefix_bit_len,
                left,
                right,
                ..
            } => {
                let split = prefix_bit_len as usize;
                if !prefix_matches(key, &prefix, split) {
                    return Ok(Some(digest));
                }
                let (left, right) = if key_bit(key, split)? {
                    let Some(right) = self.remove_at(right, key)? else {
                        return Ok(Some(left));
                    };
                    (left, right)
                } else {
                    let Some(left) = self.remove_at(left, key)? else {
                        return Ok(Some(right));
                    };
                    (left, right)
                };
                Ok(Some(self.publish_branch(
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                )?))
            }
        }
    }

    fn tree_first_after(
        &self,
        root: ContentDigest,
        after: Option<&[u8]>,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, ProjectionWorkError> {
        if root == empty_tree_root() {
            return Ok(None);
        }
        self.first_after_at(root, after)
    }

    fn first_after_at(
        &self,
        digest: ContentDigest,
        after: Option<&[u8]>,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, ProjectionWorkError> {
        let node = self.read_node(digest)?;
        if after.is_some_and(|after| node.key_max() <= after) {
            return Ok(None);
        }
        match node {
            IndexNode::Leaf { key, value, .. } => Ok(Some((key, value))),
            IndexNode::Branch { left, right, .. } => {
                if let Some(found) = self.first_after_at(left, after)? {
                    return Ok(Some(found));
                }
                self.first_after_at(right, after)
            }
        }
    }

    fn publish_node(&self, node: &IndexNode) -> Result<ContentDigest, ProjectionWorkError> {
        validate_node(node)?;
        let bytes = encode_canonical(node)?;
        if bytes.len() as u64 > MAX_INDEX_NODE_BYTES {
            return Err(ProjectionWorkError::TooLarge(bytes.len()));
        }
        let digest = ContentDigest::of(&bytes);
        publish_immutable_exact(
            &self.nodes,
            &node_filename(digest),
            &bytes,
            "projection work index node",
        )?;
        self.counters.node_writes.fetch_add(1, Ordering::Relaxed);
        Ok(digest)
    }

    fn read_node(&self, digest: ContentDigest) -> Result<IndexNode, ProjectionWorkError> {
        let bytes = read_optional_regular(
            &self.nodes,
            &node_filename(digest),
            MAX_INDEX_NODE_BYTES,
            None,
        )?
        .ok_or(ProjectionWorkError::MissingNode(digest))?;
        if ContentDigest::of(&bytes) != digest {
            return Err(ProjectionWorkError::NodeDigestMismatch(digest));
        }
        let node: IndexNode = decode_canonical(&bytes)?;
        validate_node(&node)?;
        self.counters.node_reads.fetch_add(1, Ordering::Relaxed);
        Ok(node)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionIndexClaim {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionIndexClaimV4 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
}

fn validate_projection_index_claim(
    bytes: &[u8],
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
) -> Result<(), ProjectionWorkError> {
    if let Ok(claim) = decode_canonical::<ProjectionIndexClaim>(bytes) {
        if claim.schema_version < INDEX_SCHEMA_VERSION {
            return Err(ProjectionWorkError::UpgradeRequired {
                found: claim.schema_version,
                current: INDEX_SCHEMA_VERSION,
            });
        }
        if claim.schema_version > INDEX_SCHEMA_VERSION {
            return Err(ProjectionWorkError::UnsupportedVersion(
                claim.schema_version,
            ));
        }
        if claim.workspace_id != workspace_id
            || claim.endpoint_id != endpoint_id
            || claim.graph_resource_id != graph_resource_id
            || claim.receipt_store_id != receipt_store_id
        {
            return Err(ProjectionWorkError::BindingMismatch);
        }
        return Ok(());
    }
    if let Ok(claim) = postcard::from_bytes::<ProjectionIndexClaimV4>(bytes) {
        if postcard::to_allocvec(&claim).ok().as_deref() == Some(bytes) && claim.schema_version == 4
        {
            return Err(ProjectionWorkError::UpgradeRequired {
                found: claim.schema_version,
                current: INDEX_SCHEMA_VERSION,
            });
        }
    }
    Err(ProjectionWorkError::NonCanonical)
}

fn validate_projection_root_binding(
    root: &ProjectionRoot,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
) -> Result<(), ProjectionWorkError> {
    if root.schema_version < INDEX_SCHEMA_VERSION {
        return Err(ProjectionWorkError::UpgradeRequired {
            found: root.schema_version,
            current: INDEX_SCHEMA_VERSION,
        });
    }
    if root.schema_version > INDEX_SCHEMA_VERSION {
        return Err(ProjectionWorkError::UnsupportedVersion(root.schema_version));
    }
    if root.workspace_id != workspace_id
        || root.endpoint_id != endpoint_id
        || root.graph_resource_id != graph_resource_id
        || root.receipt_store_id != receipt_store_id
        || (root.engine_history_generation == 0)
            != (root.engine_history_root == super::object_store::EngineHistoryStore::empty_root())
    {
        return Err(ProjectionWorkError::BindingMismatch);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedBatch {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    work: Vec<ProjectionWork>,
    superseded: Vec<ProjectionWorkId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedBatchWitness {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    prepared_digest: ContentDigest,
    work_ids: Vec<ProjectionWorkId>,
    pending_root_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWork {
    schema_version: u32,
    work: ProjectionWork,
    status: StoredWorkStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredWorkStatus {
    Ready,
    Completed {
        intent_id: ProjectionIntentId,
        logical_completion_id: LogicalCompletionId,
    },
    Blocked {
        observed: Option<BlobDescription>,
    },
    Superseded {
        by: ProjectionWorkId,
        accepted_batch: BatchId,
        manifest_fingerprint: ContentDigest,
        engine_history_root: ContentDigest,
    },
}

impl StoredWorkStatus {
    fn into_public(self) -> ProjectionWorkStatus {
        match self {
            Self::Ready => ProjectionWorkStatus::Ready,
            Self::Completed { .. } => ProjectionWorkStatus::Completed,
            Self::Blocked { .. } => ProjectionWorkStatus::Blocked,
            Self::Superseded { by, .. } => ProjectionWorkStatus::Superseded { by },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionRoot {
    schema_version: u32,
    workspace_id: WorkspaceId,
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    receipt_store_id: super::ProjectionReceiptStoreId,
    generation: u64,
    engine_history_generation: u64,
    engine_history_root: ContentDigest,
    rows_root: ContentDigest,
    ready_root: ContentDigest,
    paths_root: ContentDigest,
    accepted_root: ContentDigest,
    pending_root: ContentDigest,
}

impl ProjectionRoot {
    fn empty(
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        graph_resource_id: super::CanonicalGraphResourceId,
        receipt_store_id: super::ProjectionReceiptStoreId,
    ) -> Self {
        Self {
            schema_version: INDEX_SCHEMA_VERSION,
            workspace_id,
            endpoint_id,
            graph_resource_id,
            receipt_store_id,
            generation: 0,
            engine_history_generation: 0,
            engine_history_root: super::object_store::EngineHistoryStore::empty_root(),
            rows_root: empty_tree_root(),
            ready_root: empty_tree_root(),
            paths_root: empty_tree_root(),
            accepted_root: empty_tree_root(),
            pending_root: empty_tree_root(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum IndexNode {
    Leaf {
        schema_version: u32,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Branch {
        schema_version: u32,
        prefix: Vec<u8>,
        prefix_bit_len: u16,
        key_min: Vec<u8>,
        key_max: Vec<u8>,
        left: ContentDigest,
        right: ContentDigest,
    },
}

impl IndexNode {
    fn prefix(&self) -> &[u8] {
        match self {
            Self::Leaf { key, .. } => key,
            Self::Branch { prefix, .. } => prefix,
        }
    }

    fn prefix_bits(&self) -> usize {
        match self {
            Self::Leaf { key, .. } => key.len() * 8,
            Self::Branch { prefix_bit_len, .. } => *prefix_bit_len as usize,
        }
    }

    fn key_min(&self) -> &[u8] {
        match self {
            Self::Leaf { key, .. } => key,
            Self::Branch { key_min, .. } => key_min,
        }
    }

    fn key_max(&self) -> &[u8] {
        match self {
            Self::Leaf { key, .. } => key,
            Self::Branch { key_max, .. } => key_max,
        }
    }
}

fn validate_node(node: &IndexNode) -> Result<(), ProjectionWorkError> {
    match node {
        IndexNode::Leaf {
            schema_version,
            key,
            value,
        } => {
            if *schema_version != INDEX_SCHEMA_VERSION {
                return Err(ProjectionWorkError::NonCanonical);
            }
            validate_record(key, value)
        }
        IndexNode::Branch {
            schema_version,
            prefix,
            prefix_bit_len,
            key_min,
            key_max,
            left,
            right,
        } => {
            let bits = *prefix_bit_len as usize;
            if *schema_version != INDEX_SCHEMA_VERSION
                || bits >= MAX_INDEX_KEY_BYTES * 8
                || prefix.len() != bits.div_ceil(8)
                || masked_prefix(prefix, bits) != *prefix
                || left == right
                || *left == empty_tree_root()
                || *right == empty_tree_root()
                || key_min > key_max
                || !prefix_matches(key_min, prefix, bits)
                || !prefix_matches(key_max, prefix, bits)
            {
                return Err(ProjectionWorkError::NonCanonical);
            }
            validate_key(key_min)?;
            validate_key(key_max)
        }
    }
}

fn validate_record(key: &[u8], value: &[u8]) -> Result<(), ProjectionWorkError> {
    validate_key(key)?;
    if value.is_empty() || value.len() as u64 > MAX_INDEX_NODE_BYTES {
        return Err(ProjectionWorkError::NonCanonical);
    }
    Ok(())
}

fn validate_key(key: &[u8]) -> Result<(), ProjectionWorkError> {
    if key.is_empty() || key.len() > MAX_INDEX_KEY_BYTES {
        return Err(ProjectionWorkError::NonCanonical);
    }
    Ok(())
}

fn empty_tree_root() -> ContentDigest {
    ContentDigest::of(b"tine/projection-work-index/patricia-v1/empty")
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, ProjectionWorkError> {
    postcard::to_allocvec(value).map_err(|error| ProjectionWorkError::Encode(error.to_string()))
}

fn decode_canonical<T>(bytes: &[u8]) -> Result<T, ProjectionWorkError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let value: T = postcard::from_bytes(bytes)
        .map_err(|error| ProjectionWorkError::Decode(error.to_string()))?;
    if encode_canonical(&value)? != bytes {
        return Err(ProjectionWorkError::NonCanonical);
    }
    Ok(value)
}

fn work_id(
    endpoint_id: ProjectionEndpointId,
    graph_resource_id: super::CanonicalGraphResourceId,
    batch_id: BatchId,
    page_id: PageId,
    path: &ManagedPath,
    portable_path_key_digest: PortablePathKeyDigest,
    portable_path_index_root: PortablePathIndexRoot,
) -> ProjectionWorkId {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/projection-work-id/v3\0");
    for part in [
        endpoint_id.as_uuid().as_bytes().as_slice(),
        graph_resource_id.as_bytes(),
        batch_id.as_uuid().as_bytes().as_slice(),
        page_id.as_uuid().as_bytes().as_slice(),
        path.as_str().as_bytes(),
        portable_path_key_digest.as_bytes(),
        portable_path_index_root.digest().as_bytes(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    ProjectionWorkId(hasher.finalize().into())
}

fn path_digest(path: &ManagedPath) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/projection-work-path/v1\0");
    hasher.update((path.as_str().len() as u64).to_be_bytes());
    hasher.update(path.as_str().as_bytes());
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn ready_key(work: &ProjectionWork) -> Result<Vec<u8>, ProjectionWorkError> {
    let path = work.path().as_str().as_bytes();
    let path_len = u32::try_from(path.len()).map_err(|_| ProjectionWorkError::NonCanonical)?;
    let mut key = Vec::with_capacity(68 + path.len());
    key.extend_from_slice(work.batch_id().as_uuid().as_bytes());
    key.extend_from_slice(work.page_id().as_uuid().as_bytes());
    key.extend_from_slice(&path_len.to_be_bytes());
    key.extend_from_slice(path);
    key.extend_from_slice(work.work_id().as_bytes());
    validate_key(&key)?;
    Ok(key)
}

fn work_key(work_id: ProjectionWorkId) -> Vec<u8> {
    work_id.as_bytes().to_vec()
}

fn batch_key(batch_id: BatchId) -> Vec<u8> {
    batch_id.as_uuid().as_bytes().to_vec()
}

fn path_key(path: &ManagedPath) -> Vec<u8> {
    path_digest(path).as_bytes().to_vec()
}

fn decode_work_id(bytes: &[u8]) -> Result<ProjectionWorkId, ProjectionWorkError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ProjectionWorkError::NonCanonical)?;
    Ok(ProjectionWorkId(bytes))
}

fn prepared_filename(batch_id: BatchId) -> String {
    format!("{batch_id}{PREPARED_SUFFIX}")
}

fn node_filename(digest: ContentDigest) -> String {
    format!("{digest}{NODE_SUFFIX}")
}

fn root_filename(digest: ContentDigest) -> String {
    format!("{digest}{ROOT_SUFFIX}")
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn strictly_sorted_by<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

fn charge_preflight(
    current: &mut usize,
    amount: usize,
    limit: usize,
) -> Result<(), ProjectionWorkError> {
    *current = current
        .checked_add(amount)
        .ok_or(ProjectionWorkError::PreflightLimitExceeded)?;
    if *current > limit {
        return Err(ProjectionWorkError::PreflightLimitExceeded);
    }
    Ok(())
}

fn common_prefix_bits(left: &[u8], right: &[u8], limit: usize) -> usize {
    let limit = limit.min(left.len() * 8).min(right.len() * 8);
    (0..limit)
        .find(|bit| key_bit_unchecked(left, *bit) != key_bit_unchecked(right, *bit))
        .unwrap_or(limit)
}

fn prefix_matches(key: &[u8], prefix: &[u8], bits: usize) -> bool {
    key.len() * 8 >= bits
        && prefix.len() * 8 >= bits
        && common_prefix_bits(key, prefix, bits) == bits
}

fn key_bit(key: &[u8], bit: usize) -> Result<bool, ProjectionWorkError> {
    if bit >= key.len() * 8 {
        return Err(ProjectionWorkError::NonCanonical);
    }
    Ok(key_bit_unchecked(key, bit))
}

fn key_bit_unchecked(key: &[u8], bit: usize) -> bool {
    key[bit / 8] & (0x80 >> (bit % 8)) != 0
}

fn masked_prefix(key: &[u8], bits: usize) -> Vec<u8> {
    let mut prefix = key[..bits.div_ceil(8).min(key.len())].to_vec();
    if !bits.is_multiple_of(8) {
        let mask = 0xff << (8 - bits % 8);
        if let Some(last) = prefix.last_mut() {
            *last &= mask;
        }
    }
    prefix
}

#[derive(Debug)]
pub enum ProjectionWorkError {
    Store(StoreError),
    Encode(String),
    Decode(String),
    TooLarge(usize),
    NonCanonical,
    BindingMismatch,
    MissingWork(ProjectionWorkId),
    MissingReadyRow,
    MissingPreparedBatch(BatchId),
    MissingHead,
    UpgradeRequired { found: u32, current: u32 },
    UnsupportedVersion(u32),
    MissingRoot(ContentDigest),
    MissingNode(ContentDigest),
    RootDigestMismatch(ContentDigest),
    NodeDigestMismatch(ContentDigest),
    AcceptedWitnessMissing,
    AcceptedWitnessMismatch,
    PendingActivationMissing,
    PendingActivationMismatch,
    HistoryBindingMismatch,
    ConflictingStatus,
    ConcurrentRootTransition,
    InvalidPageLimit(usize),
    PreflightLimitExceeded,
    Poisoned,
}

impl fmt::Display for ProjectionWorkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::Encode(error) => write!(f, "projection work encode failed: {error}"),
            Self::Decode(error) => write!(f, "projection work decode failed: {error}"),
            Self::TooLarge(length) => {
                write!(f, "projection work value is too large: {length} bytes")
            }
            Self::NonCanonical => f.write_str("projection work row/index is non-canonical"),
            Self::BindingMismatch => f.write_str("projection work endpoint/workspace mismatch"),
            Self::MissingWork(work_id) => write!(f, "projection work {work_id} is missing"),
            Self::MissingReadyRow => f.write_str("projection ready entry has no bound work row"),
            Self::MissingPreparedBatch(batch_id) => {
                write!(
                    f,
                    "prepared projection work for batch {batch_id} is missing"
                )
            }
            Self::MissingHead => f.write_str("projection work authenticated root head is missing"),
            Self::UpgradeRequired { found, current } => write!(
                f,
                "projection work index version {found} requires upgrade to {current}"
            ),
            Self::UnsupportedVersion(version) => {
                write!(f, "projection work index version {version} is unsupported")
            }
            Self::MissingRoot(digest) => write!(f, "projection work root {digest} is missing"),
            Self::MissingNode(digest) => write!(f, "projection work node {digest} is missing"),
            Self::RootDigestMismatch(digest) => {
                write!(f, "projection work root does not match digest {digest}")
            }
            Self::NodeDigestMismatch(digest) => {
                write!(f, "projection work node does not match digest {digest}")
            }
            Self::AcceptedWitnessMissing => {
                f.write_str("projection work has no authenticated accepted-batch witness")
            }
            Self::AcceptedWitnessMismatch => {
                f.write_str("projection work accepted-batch witness is misbound")
            }
            Self::PendingActivationMissing => {
                f.write_str("projection work pending activation is missing")
            }
            Self::PendingActivationMismatch => {
                f.write_str("projection work pending activation is misbound")
            }
            Self::HistoryBindingMismatch => {
                f.write_str("projection work root is not bound to current engine history")
            }
            Self::ConflictingStatus => f.write_str("projection work has conflicting status"),
            Self::ConcurrentRootTransition => {
                f.write_str("projection work root changed during a transition")
            }
            Self::InvalidPageLimit(limit) => {
                write!(f, "projection work page limit {limit} is invalid")
            }
            Self::PreflightLimitExceeded => {
                f.write_str("projection work reachable preflight limit exceeded")
            }
            Self::Poisoned => f.write_str("projection work transition lock is poisoned"),
        }
    }
}

impl std::error::Error for ProjectionWorkError {}

impl From<StoreError> for ProjectionWorkError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<std::io::Error> for ProjectionWorkError {
    fn from(error: std::io::Error) -> Self {
        Self::Store(StoreError::Io(error))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;

    use uuid::Uuid;

    use super::*;
    use crate::oplog::{DocumentId, ObjectDescriptor, ObjectKind, ObjectStore};

    fn snapshot_tree(root: &std::path::Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut snapshot = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                snapshot.insert(relative, None);
                for entry in fs::read_dir(path).unwrap() {
                    pending.push(entry.unwrap().path());
                }
            } else {
                snapshot.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
        snapshot
    }

    struct Fixture {
        path: PathBuf,
        workspace_id: WorkspaceId,
        endpoint_id: ProjectionEndpointId,
        graph_resource_id: super::super::CanonicalGraphResourceId,
        index: ProjectionWorkIndex,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-projection-work-{name}-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(1));
            let endpoint_id = ProjectionEndpointId::from_uuid(Uuid::from_u128(2));
            let graph_resource_id =
                super::super::CanonicalGraphResourceId::from_capability_identity(
                    b"test",
                    name.as_bytes(),
                );
            let store = ObjectStore::open(&path, workspace_id).unwrap();
            let index = store
                .open_projection_work_index(super::super::hot_engine::ProjectionStorageBinding {
                    endpoint: super::super::ProjectionEndpointBinding {
                        endpoint_id,
                        device_id: super::super::DeviceId::from_uuid(Uuid::from_u128(3)),
                        graph_resource_id,
                    },
                    receipt_store_id:
                        super::super::ProjectionReceiptStoreId::from_capability_identity(
                            b"test",
                            b"projection-work-index",
                        ),
                })
                .unwrap();
            Self {
                path,
                workspace_id,
                endpoint_id,
                graph_resource_id,
                index,
            }
        }

        fn work(&self, sequence: u128, path: &str) -> ProjectionWork {
            let descriptor = ObjectDescriptor::new(
                DocumentId::from_uuid(Uuid::from_u128(10_000 + sequence)),
                ObjectKind::ProjectionIntent,
                ContentDigest::of(&sequence.to_be_bytes()),
                1,
            )
            .unwrap();
            ProjectionWork::new(
                self.workspace_id,
                self.endpoint_id,
                self.graph_resource_id,
                BatchId::from_uuid(Uuid::from_u128(20_000 + sequence)),
                PageId::from_uuid(Uuid::from_u128(30_000 + sequence)),
                ManagedPath::parse(path).unwrap(),
                PortablePathIndexRoot::empty(),
                ManifestObjectRef::from_descriptor(&descriptor),
                FrontierV2::default(),
                ProjectionWorkTarget::Present(BlobDescription::of(&sequence.to_be_bytes())),
            )
        }

        fn prepare(&self, work: &ProjectionWork) -> ContentDigest {
            let fingerprint = ContentDigest::of(work.batch_id().as_uuid().as_bytes());
            self.index
                .prepare_batch(
                    work.batch_id(),
                    fingerprint,
                    std::slice::from_ref(work),
                    &[],
                )
                .unwrap();
            fingerprint
        }

        fn completion_authority(&self, work: &ProjectionWork) -> ProjectionWorkCompletionAuthority {
            ProjectionWorkCompletionAuthority {
                workspace_id: work.workspace_id(),
                endpoint_id: work.endpoint_id(),
                graph_resource_id: work.graph_resource_id(),
                receipt_store_id: self.index.receipt_store_id(),
                work_id: work.work_id(),
                page_id: work.page_id(),
                path: work.path().clone(),
                target: work.target(),
                intent_id: ProjectionIntentId::test_only_zero(),
                logical_completion_id: serde_json::from_str(&format!("\"{}\"", "00".repeat(32)))
                    .unwrap(),
            }
        }

        fn binding(&self) -> super::super::hot_engine::ProjectionStorageBinding {
            super::super::hot_engine::ProjectionStorageBinding {
                endpoint: super::super::ProjectionEndpointBinding {
                    endpoint_id: self.endpoint_id,
                    device_id: super::super::DeviceId::from_uuid(Uuid::from_u128(3)),
                    graph_resource_id: self.graph_resource_id,
                },
                receipt_store_id: self.index.receipt_store_id(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }

    #[test]
    fn fail_before_prepared_work_is_never_ready_without_accepted_transition() {
        let fixture = Fixture::new("prepared-not-ready");
        let work = fixture.work(1, "pages/prepared.md");
        fixture.prepare(&work);

        assert!(fixture.index.next().unwrap().is_none());
        assert_eq!(fixture.index.status(work.work_id()).unwrap(), None);
        let pending = fixture.index.pending_activation_page(None, 8).unwrap();
        assert_eq!(pending.pending().len(), 1);
        assert_eq!(pending.pending()[0].batch_id(), work.batch_id());
        assert_eq!(
            pending.pending()[0].work_ids(),
            std::slice::from_ref(&work.work_id())
        );
        assert!(pending.next().is_none());
        assert!(
            fixture
            .index
            .pending_for_path(work.path())
            .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn enrolled_seal_rejects_reachable_future_prepared_work_without_mutation() {
        let fixture = Fixture::new("future-prepared-preflight");
        let work = fixture.work(1, "pages/future-prepared.md");
        fixture.prepare(&work);
        let mut pending = fixture
            .index
            .pending_activation_page(None, 1)
            .unwrap()
            .pending()[0]
            .clone();
        let prepared_path = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string())
            .join("prepared")
            .join(prepared_filename(work.batch_id()));
        let mut prepared: PreparedBatch =
            decode_canonical(&fs::read(&prepared_path).unwrap()).unwrap();
        prepared.work[0].schema_version = WORK_SCHEMA_VERSION + 1;
        let prepared_bytes = encode_canonical(&prepared).unwrap();
        fs::write(&prepared_path, &prepared_bytes).unwrap();
        pending.prepared_digest = ContentDigest::of(&prepared_bytes);
        fixture
            .index
            .transition(|index, _, mut root| {
                root.pending_root = index.tree_insert(
                    root.pending_root,
                    batch_key(pending.batch_id),
                    encode_canonical(&pending)?,
                )?;
                Ok(root)
            })
            .unwrap();

        let store = ObjectStore::open(&fixture.path, fixture.workspace_id).unwrap();
        let before = snapshot_tree(&fixture.path);
        assert!(store.seal_enrolled_projection(fixture.binding()).is_err());
        assert_eq!(snapshot_tree(&fixture.path), before);
        assert!(!fixture.path.join("scratch-v1").exists());
        assert!(!fixture.path.join("logseq-uuid-claim-index-v1").exists());
        assert!(!fixture.path.join("portable-path-index-v1").exists());
    }

    #[test]
    fn sealed_head_claim_or_root_swap_before_consumption_rejects_without_mutation() {
        enum Swap {
            WorkHead,
            WorkClaim,
            WorkRoot,
            HistoryHead,
        }

        for (label, swap) in [
            ("work-head", Swap::WorkHead),
            ("work-claim", Swap::WorkClaim),
            ("work-root", Swap::WorkRoot),
            ("history-head", Swap::HistoryHead),
        ] {
            let fixture = Fixture::new(&format!("sealed-{label}-swap"));
            let history_store = ObjectStore::open(&fixture.path, fixture.workspace_id).unwrap();
            drop(history_store.open_engine_history(fixture.binding()).unwrap());
            let store = ObjectStore::open(&fixture.path, fixture.workspace_id).unwrap();
            let open = store.seal_enrolled_projection(fixture.binding()).unwrap();
            let work_control = fixture
                .path
                .join("projection-work-index-v1")
                .join(fixture.endpoint_id.to_string());
            let target = match swap {
                Swap::WorkHead => work_control.join(HEAD_FILE),
                Swap::WorkClaim => work_control.join(CLAIM_FILE),
                Swap::WorkRoot => {
                    let digest = std::str::from_utf8(&fs::read(work_control.join(HEAD_FILE)).unwrap())
                        .map(parse_digest)
                        .unwrap()
                        .map(ContentDigest::from_bytes)
                        .unwrap();
                    work_control.join("roots").join(root_filename(digest))
                }
                Swap::HistoryHead => fixture
                    .path
                    .join("engine-history")
                    .join(fixture.endpoint_id.to_string())
                    .join("engine-history.head"),
            };
            let attacked = Rc::new(RefCell::new(None));
            let attacked_hook = Rc::clone(&attacked);
            let root = fixture.path.clone();
            super::super::object_store::set_enrolled_open_use_hook(move || {
                fs::write(&target, b"substituted authenticated name").unwrap();
                *attacked_hook.borrow_mut() = Some(snapshot_tree(&root));
            });

            assert!(open.into_runtime().is_err(), "swap {label} was accepted");
            assert_eq!(
                snapshot_tree(&fixture.path),
                attacked.borrow().clone().expect("open cut hook ran"),
                "swap {label} mutated storage after rejection"
            );
            assert!(!fixture.path.join("scratch-v1").exists());
            assert!(!fixture.path.join("logseq-uuid-claim-index-v1").exists());
            assert!(!fixture.path.join("portable-path-index-v1").exists());
        }
    }

    #[test]
    fn sealed_work_head_rollback_after_validation_rejects_without_mutation() {
        let fixture = Fixture::new("sealed-work-valid-rollback");
        let endpoint = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string());
        let original = fs::read(endpoint.join(HEAD_FILE)).unwrap();
        fixture.prepare(&fixture.work(1, "pages/rollback.md"));
        let store = ObjectStore::open(&fixture.path, fixture.workspace_id).unwrap();
        let open = store.seal_enrolled_projection(fixture.binding()).unwrap();
        let attacked = Rc::new(RefCell::new(None));
        let attacked_hook = Rc::clone(&attacked);
        let root = fixture.path.clone();
        super::super::object_store::set_enrolled_open_act_hook(move || {
            fs::write(endpoint.join(HEAD_FILE), original).unwrap();
            *attacked_hook.borrow_mut() = Some(snapshot_tree(&root));
        });

        assert!(open.into_runtime().is_err());
        assert_eq!(
            snapshot_tree(&fixture.path),
            attacked.borrow().clone().expect("attack hook ran")
        );
    }

    #[test]
    fn sealed_work_baseline_survives_reads_until_its_transition_advances() {
        let fixture = Fixture::new("sealed-work-subsequent-rollback");
        let endpoint = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string());
        let original = fs::read(endpoint.join(HEAD_FILE)).unwrap();
        let work = fixture.work(1, "pages/rollback.md");
        fixture.prepare(&work);
        let accepted = fs::read(endpoint.join(HEAD_FILE)).unwrap();
        let (_, _, reopened) = ObjectStore::open(&fixture.path, fixture.workspace_id)
            .unwrap()
            .seal_enrolled_projection(fixture.binding())
            .unwrap()
            .into_runtime()
            .unwrap();
        assert!(reopened.next().unwrap().is_none());

        fs::write(endpoint.join(HEAD_FILE), original).unwrap();
        let attacked = snapshot_tree(&fixture.path);
        assert!(reopened.next().is_err());
        assert_eq!(snapshot_tree(&fixture.path), attacked);

        fs::write(endpoint.join(HEAD_FILE), accepted).unwrap();
        assert!(reopened.next().unwrap().is_none());
    }

    #[test]
    fn fail_before_accepted_history_replays_exact_prepared_root() {
        let fixture = Fixture::new("accepted-replay");
        let work = fixture.work(1, "pages/replay.md");
        let fingerprint = fixture.prepare(&work);

        fixture
            .index
            .accept_batch(work.batch_id(), fingerprint)
            .unwrap();
        fixture
            .index
            .accept_batch(work.batch_id(), fingerprint)
            .unwrap();

        assert!(
            fixture
            .index
            .pending_activation_page(None, 8)
            .unwrap()
            .pending()
                .is_empty()
        );
        assert_eq!(fixture.index.next().unwrap(), Some(work.clone()));
        fixture
            .index
            .require_accepted_ready(&work, fingerprint)
            .unwrap();
        assert!(
            fixture
            .index
            .require_accepted_ready(&work, ContentDigest::of(b"wrong manifest"))
                .is_err()
        );
    }

    #[test]
    fn ready_and_path_indexes_remain_affected_only_over_lifetime() {
        let fixture = Fixture::new("lifetime");

        let early = fixture.work(1, "pages/live.md");
        let early_fingerprint = fixture.prepare(&early);
        let before_early = fixture.index.stats();
        fixture
            .index
            .accept_batch(early.batch_id(), early_fingerprint)
            .unwrap();
        assert_eq!(fixture.index.next().unwrap(), Some(early.clone()));
        assert_eq!(
            fixture.index.pending_for_path(early.path()).unwrap(),
            vec![early.clone()]
        );
        let after_early = fixture.index.stats();
        let early_reads = after_early.node_reads - before_early.node_reads;
        let early_writes = after_early.node_writes - before_early.node_writes;
        fixture
            .index
            .mark_completed(fixture.completion_authority(&early))
            .unwrap();

        for sequence in 2..=130 {
            let historical = fixture.work(sequence, "pages/live.md");
            let fingerprint = fixture.prepare(&historical);
            fixture
                .index
                .accept_batch(historical.batch_id(), fingerprint)
                .unwrap();
            fixture
                .index
                .mark_completed(fixture.completion_authority(&historical))
                .unwrap();
        }

        let late = fixture.work(131, "pages/live.md");
        let late_fingerprint = fixture.prepare(&late);
        let before_late = fixture.index.stats();
        fixture
            .index
            .accept_batch(late.batch_id(), late_fingerprint)
            .unwrap();
        assert_eq!(fixture.index.next().unwrap(), Some(late.clone()));
        assert_eq!(
            fixture.index.pending_for_path(late.path()).unwrap(),
            vec![late]
        );
        let after_late = fixture.index.stats();
        let late_reads = after_late.node_reads - before_late.node_reads;
        let late_writes = after_late.node_writes - before_late.node_writes;

        assert!(
            late_reads <= early_reads + 64,
            "authenticated point work grew with lifetime: early={early_reads}, late={late_reads}"
        );
        assert!(
            late_writes <= early_writes + 64,
            "authenticated affected writes grew with lifetime: early={early_writes}, late={late_writes}"
        );
        assert!(late_reads < 256);
        assert!(late_writes < 256);
    }

    #[test]
    fn pending_activation_page_reads_only_the_current_authenticated_set() {
        let fixture = Fixture::new("pending-lifetime");
        for sequence in 1..=130 {
            let historical = fixture.work(sequence, &format!("pages/{sequence}.md"));
            fixture.prepare(&historical);
            let pending = fixture.index.pending_activation_page(None, 1).unwrap();
            assert_eq!(pending.pending().len(), 1);
            fixture
                .index
                .retire_pending_activation(&pending.pending()[0])
                .unwrap();
        }

        let current = fixture.work(131, "pages/current.md");
        fixture.prepare(&current);
        let before = fixture.index.stats();
        let page = fixture.index.pending_activation_page(None, 8).unwrap();
        let after = fixture.index.stats();

        assert_eq!(page.pending().len(), 1);
        assert_eq!(page.pending()[0].batch_id(), current.batch_id());
        assert_eq!(after.pending_entries_read - before.pending_entries_read, 1);
        assert_eq!(after.prepared_reads - before.prepared_reads, 1);
        assert!(after.node_reads - before.node_reads < 16);
    }

    #[test]
    fn missing_pending_prepared_file_fails_closed() {
        let fixture = Fixture::new("missing-pending-prepared");
        let work = fixture.work(1, "pages/missing.md");
        fixture.prepare(&work);
        let prepared = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string())
            .join("prepared")
            .join(prepared_filename(work.batch_id()));
        fs::remove_file(prepared).unwrap();

        assert!(matches!(
            fixture.index.pending_activation_page(None, 8),
            Err(ProjectionWorkError::MissingPreparedBatch(batch_id))
                if batch_id == work.batch_id()
        ));
        let fingerprint = ContentDigest::of(work.batch_id().as_uuid().as_bytes());
        assert!(matches!(
            fixture.index.prepare_batch(
                work.batch_id(),
                fingerprint,
                std::slice::from_ref(&work),
                &[]
            ),
            Err(ProjectionWorkError::MissingPreparedBatch(batch_id))
                if batch_id == work.batch_id()
        ));
    }

    #[test]
    fn missing_accepted_pending_root_fails_closed() {
        let fixture = Fixture::new("missing-accepted-source");
        let work = fixture.work(1, "pages/accepted-source.md");
        let fingerprint = fixture.prepare(&work);
        fixture
            .index
            .accept_batch(work.batch_id(), fingerprint)
            .unwrap();
        let source_root = fixture
            .index
            .accepted_preparation_root(work.batch_id())
            .unwrap();
        let root_path = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string())
            .join("roots")
            .join(root_filename(source_root));
        fs::remove_file(root_path).unwrap();

        assert!(matches!(
            fixture.index.require_accepted_ready(&work, fingerprint),
            Err(ProjectionWorkError::MissingRoot(found)) if found == source_root
        ));
    }

    #[test]
    fn fail_before_terminal_authority_cannot_cross_work_or_graph_resource() {
        let fixture = Fixture::new("terminal-proof-binding");
        let first = fixture.work(1, "pages/first.md");
        let second = fixture.work(2, "pages/second.md");
        for work in [&first, &second] {
            let fingerprint = fixture.prepare(work);
            fixture
                .index
                .accept_batch(work.batch_id(), fingerprint)
                .unwrap();
        }

        let mut forged = fixture.completion_authority(&first);
        forged.work_id = second.work_id();
        assert!(matches!(
            fixture.index.mark_completed(forged),
            Err(ProjectionWorkError::BindingMismatch)
        ));
        assert_eq!(
            fixture.index.status(first.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Ready)
        );
        assert_eq!(
            fixture.index.status(second.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Ready)
        );

        let foreign = Fixture::new("terminal-proof-foreign-root");
        let foreign_work = foreign.work(1, "pages/first.md");
        let fingerprint = foreign.prepare(&foreign_work);
        foreign
            .index
            .accept_batch(foreign_work.batch_id(), fingerprint)
            .unwrap();
        assert!(
            foreign
            .index
            .mark_completed(fixture.completion_authority(&first))
                .is_err()
        );
        assert_eq!(
            foreign.index.status(foreign_work.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Ready)
        );

        let mut forged_block = ProjectionWorkBlockAuthority::guarded_conflict(
            &first,
            fixture.index.receipt_store_id(),
            Some(BlobDescription::of(b"external")),
        );
        forged_block.work_id = second.work_id();
        assert!(matches!(
            fixture.index.mark_blocked(forged_block),
            Err(ProjectionWorkError::BindingMismatch)
        ));
        assert_eq!(
            fixture.index.status(second.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Ready)
        );
    }

    #[test]
    fn accepted_causal_activation_is_the_only_supersession_path() {
        let fixture = Fixture::new("causal-supersession");
        let older = fixture.work(1, "pages/same.md");
        let older_fingerprint = fixture.prepare(&older);
        fixture
            .index
            .accept_batch(older.batch_id(), older_fingerprint)
            .unwrap();

        let newer = fixture.work(2, "pages/same.md");
        let newer_fingerprint = ContentDigest::of(newer.batch_id().as_uuid().as_bytes());
        fixture
            .index
            .prepare_batch(
                newer.batch_id(),
                newer_fingerprint,
                std::slice::from_ref(&newer),
                std::slice::from_ref(&older.work_id()),
            )
            .unwrap();
        assert_eq!(
            fixture.index.status(older.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Ready)
        );
        fixture
            .index
            .accept_batch(newer.batch_id(), newer_fingerprint)
            .unwrap();

        assert_eq!(
            fixture.index.status(older.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Superseded {
                by: newer.work_id()
            })
        );
        assert_eq!(
            fixture.index.status(newer.work_id()).unwrap(),
            Some(ProjectionWorkStatus::Ready)
        );
    }

    #[test]
    fn ready_cursor_stays_bound_to_its_authenticated_root() {
        let fixture = Fixture::new("stable-cursor");
        let mut rows = Vec::new();
        for sequence in 1..=3 {
            let work = fixture.work(sequence, &format!("pages/{sequence}.md"));
            let fingerprint = fixture.prepare(&work);
            fixture
                .index
                .accept_batch(work.batch_id(), fingerprint)
                .unwrap();
            rows.push(work);
        }

        let first = fixture.index.ready_page(None, 2).unwrap();
        assert_eq!(first.work(), &rows[..2]);
        let cursor = first.next().unwrap().clone();
        fixture
            .index
            .mark_completed(fixture.completion_authority(&rows[2]))
            .unwrap();
        assert_eq!(
            fixture.index.ready_page(Some(&cursor), 2).unwrap().work(),
            &rows[2..]
        );
        assert!(
            fixture
            .index
            .pending_for_path(rows[2].path())
            .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn pending_cursor_stays_bound_to_its_authenticated_root() {
        let fixture = Fixture::new("stable-pending-cursor");
        let mut rows = Vec::new();
        for sequence in 1..=3 {
            let work = fixture.work(sequence, &format!("pages/pending-{sequence}.md"));
            fixture.prepare(&work);
            rows.push(work);
        }

        let first = fixture.index.pending_activation_page(None, 2).unwrap();
        assert_eq!(
            first
                .pending()
                .iter()
                .map(ProjectionPendingActivation::batch_id)
                .collect::<Vec<_>>(),
            rows[..2]
                .iter()
                .map(ProjectionWork::batch_id)
                .collect::<Vec<_>>()
        );
        let cursor = first.next().unwrap().clone();
        let current = fixture.index.pending_activation_page(None, 8).unwrap();
        let third = current
            .pending()
            .iter()
            .find(|pending| pending.batch_id() == rows[2].batch_id())
            .unwrap()
            .clone();
        fixture.index.retire_pending_activation(&third).unwrap();

        assert_eq!(
            fixture
                .index
                .pending_activation_page(Some(&cursor), 2)
                .unwrap()
                .pending()
                .iter()
                .map(ProjectionPendingActivation::batch_id)
                .collect::<Vec<_>>(),
            vec![rows[2].batch_id()]
        );
        assert!(
            fixture
            .index
            .pending_activation_page(None, 8)
            .unwrap()
            .pending()
            .iter()
                .all(|pending| pending.batch_id() != rows[2].batch_id())
        );
    }

    #[test]
    fn missing_or_tampered_authenticated_root_fails_closed() {
        let fixture = Fixture::new("tampered-root");
        let work = fixture.work(1, "pages/tampered.md");
        let fingerprint = fixture.prepare(&work);
        fixture
            .index
            .accept_batch(work.batch_id(), fingerprint)
            .unwrap();

        let endpoint = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string());
        let head = fs::read_to_string(endpoint.join(HEAD_FILE)).unwrap();
        let root_path = endpoint.join("roots").join(format!("{head}{ROOT_SUFFIX}"));
        let mut bytes = fs::read(&root_path).unwrap();
        bytes[0] ^= 0x80;
        fs::write(&root_path, bytes).unwrap();
        assert!(matches!(
            fixture.index.next(),
            Err(ProjectionWorkError::RootDigestMismatch(_))
        ));
    }

    #[test]
    fn prior_version_completed_work_requires_upgrade_without_writes() {
        fn snapshot(path: &std::path::Path) -> BTreeMap<PathBuf, Vec<u8>> {
            let mut result = BTreeMap::new();
            let mut pending = vec![path.to_path_buf()];
            while let Some(directory) = pending.pop() {
                for entry in fs::read_dir(&directory).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_type().unwrap().is_dir() {
                        pending.push(entry.path());
                    } else {
                        result.insert(
                            entry.path().strip_prefix(path).unwrap().to_path_buf(),
                            fs::read(entry.path()).unwrap(),
                        );
                    }
                }
            }
            result
        }

        let fixture = Fixture::new("prior-completed");
        let work = fixture.work(1, "pages/completed.md");
        let fingerprint = fixture.prepare(&work);
        fixture
            .index
            .accept_batch(work.batch_id(), fingerprint)
            .unwrap();
        fixture
            .index
            .mark_completed(fixture.completion_authority(&work))
            .unwrap();
        let endpoint = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string());
        let prior_claim = postcard::to_allocvec(&ProjectionIndexClaimV4 {
            schema_version: 4,
            workspace_id: fixture.workspace_id,
            endpoint_id: fixture.endpoint_id,
            graph_resource_id: fixture.graph_resource_id,
        })
        .unwrap();
        fs::write(endpoint.join(CLAIM_FILE), prior_claim).unwrap();
        let before = snapshot(&fixture.path);

        let store = ObjectStore::open(&fixture.path, fixture.workspace_id).unwrap();
        let error = store
            .open_projection_work_index(super::super::hot_engine::ProjectionStorageBinding {
                endpoint: super::super::ProjectionEndpointBinding {
                    endpoint_id: fixture.endpoint_id,
                    device_id: super::super::DeviceId::from_uuid(Uuid::from_u128(3)),
                    graph_resource_id: fixture.graph_resource_id,
                },
                receipt_store_id: fixture.index.receipt_store_id(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("requires upgrade"));
        assert_eq!(snapshot(&fixture.path), before);
    }

    #[test]
    fn synthetic_future_work_claim_rejects_before_creating_index_layout() {
        fn snapshot(path: &std::path::Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
            let mut result = BTreeMap::new();
            let mut pending = vec![path.to_path_buf()];
            while let Some(entry_path) = pending.pop() {
                let relative = entry_path.strip_prefix(path).unwrap().to_path_buf();
                if entry_path.is_dir() {
                    result.insert(relative, None);
                    for entry in fs::read_dir(&entry_path).unwrap() {
                        pending.push(entry.unwrap().path());
                    }
                } else {
                    result.insert(relative, Some(fs::read(entry_path).unwrap()));
                }
            }
            result
        }

        let path = std::env::temp_dir().join(format!(
            "tine-projection-work-future-synthetic-{}",
            Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(101));
        let endpoint_id = ProjectionEndpointId::from_uuid(Uuid::from_u128(102));
        let graph_resource_id = super::super::CanonicalGraphResourceId::from_capability_identity(
            b"test",
            b"future-work",
        );
        let receipt_store_id = super::super::ProjectionReceiptStoreId::from_capability_identity(
            b"test",
            b"future-work-receipts",
        );
        let binding = super::super::hot_engine::ProjectionStorageBinding {
            endpoint: super::super::ProjectionEndpointBinding {
                endpoint_id,
                device_id: super::super::DeviceId::from_uuid(Uuid::from_u128(103)),
                graph_resource_id,
            },
            receipt_store_id,
        };
        let store = ObjectStore::open(&path, workspace_id).unwrap();
        let control = path
            .join("projection-work-index-v1")
            .join(endpoint_id.to_string());
        fs::create_dir_all(&control).unwrap();
        fs::write(control.join(HEAD_FILE), b"future-head").unwrap();
        fs::write(
            control.join(CLAIM_FILE),
            encode_canonical(&ProjectionIndexClaim {
                schema_version: INDEX_SCHEMA_VERSION + 1,
                workspace_id,
                endpoint_id,
                graph_resource_id,
                receipt_store_id,
            })
            .unwrap(),
        )
        .unwrap();
        let before = snapshot(&path);

        let error = store.open_projection_work_index(binding).unwrap_err();
        assert!(error.to_string().contains(&format!(
            "version {} is unsupported",
            INDEX_SCHEMA_VERSION + 1
        )));
        assert_eq!(snapshot(&path), before);
        assert!(!control.join("nodes").exists());
        assert!(!control.join("roots").exists());
        assert!(!control.join("prepared").exists());
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn authenticated_projection_root_version_matrix_rejects_without_writes() {
        fn snapshot(path: &std::path::Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
            let mut result = BTreeMap::new();
            let mut pending = vec![path.to_path_buf()];
            while let Some(entry_path) = pending.pop() {
                let relative = entry_path.strip_prefix(path).unwrap().to_path_buf();
                if entry_path.is_dir() {
                    result.insert(relative, None);
                    for entry in fs::read_dir(&entry_path).unwrap() {
                        pending.push(entry.unwrap().path());
                    }
                } else {
                    result.insert(relative, Some(fs::read(entry_path).unwrap()));
                }
            }
            result
        }

        let fixture = Fixture::new("projection-root-version-matrix");
        let receipt_store_id = fixture.index.receipt_store_id();
        let binding = super::super::hot_engine::ProjectionStorageBinding {
            endpoint: super::super::ProjectionEndpointBinding {
                endpoint_id: fixture.endpoint_id,
                device_id: super::super::DeviceId::from_uuid(Uuid::from_u128(104)),
                graph_resource_id: fixture.graph_resource_id,
            },
            receipt_store_id,
        };
        let control = fixture
            .path
            .join("projection-work-index-v1")
            .join(fixture.endpoint_id.to_string());
        let roots = control.join("roots");

        for version in [INDEX_SCHEMA_VERSION - 1, INDEX_SCHEMA_VERSION + 1] {
            let mut authenticated_root = ProjectionRoot::empty(
                fixture.workspace_id,
                fixture.endpoint_id,
                fixture.graph_resource_id,
                receipt_store_id,
            );
            authenticated_root.schema_version = version;
            let bytes = encode_canonical(&authenticated_root).unwrap();
            let digest = ContentDigest::of(&bytes);
            fs::write(roots.join(root_filename(digest)), &bytes).unwrap();
            fs::write(control.join(HEAD_FILE), digest.to_string()).unwrap();
            let before = snapshot(&fixture.path);

            let store = ObjectStore::open(&fixture.path, fixture.workspace_id).unwrap();
            let error = store.open_projection_work_index(binding).unwrap_err();
            if version < INDEX_SCHEMA_VERSION {
                assert!(matches!(
                    error,
                    StoreError::Scratch(message)
                        if message
                            == format!(
                                "projection work index version {version} requires upgrade to {INDEX_SCHEMA_VERSION}"
                            )
                ));
            } else {
                assert!(matches!(
                    error,
                    StoreError::Scratch(message)
                        if message == format!("projection work index version {version} is unsupported")
                ));
            }
            assert_eq!(snapshot(&fixture.path), before);
            assert!(
                !fixture
                    .path
                    .join(super::super::scratch_store::SCRATCH_DIR)
                    .exists()
            );
        }
    }
}
