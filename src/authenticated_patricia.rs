#![allow(clippy::result_large_err)]

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::ContentDigest;
use crate::filesystem::{read_optional_regular, FilesystemError, StagedExactImmutablePublication};
#[cfg(any(test, feature = "test-support"))]
use crate::packed_patricia::corrupt_packed_node_for_test;
use crate::packed_patricia::{
    lock_packed_operation_shared, publish_appended_catalog,
    publish_appended_catalog_bounded_streaming, reclaim_unreachable_packed_files,
    transition_catalog_head, PackedPatriciaCatalog, PackedPatriciaConstructionSink,
    PackedPatriciaError, PackedPatriciaPublicationWork, PackedPatriciaReclamationError,
    PackedPatriciaReclamationReport, PackedPatriciaResidencyBudget, HEAD_BYTES,
    MAX_CATALOG_PACK_BYTES,
};

/// Opaque publication failure returned by the policy-owning Patricia adapter.
pub struct PatriciaPublicationError(Box<dyn Any + Send>);

impl PatriciaPublicationError {
    pub fn new(error: impl Any + Send) -> Self {
        Self(Box::new(error))
    }

    pub fn downcast<T: Any + Send>(self) -> Result<T, Self> {
        match self.0.downcast::<T>() {
            Ok(error) => Ok(*error),
            Err(error) => Err(Self(error)),
        }
    }
}

impl fmt::Debug for PatriciaPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PatriciaPublicationError")
            .finish_non_exhaustive()
    }
}

/// Narrow publication boundary implemented by the domain that owns labels and
/// collision interpretation.
pub trait PatriciaNodePublisher: Send + Sync {
    fn publish(
        &self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
    ) -> Result<(), PatriciaPublicationError>;

    /// Opt in only when each successful publication is independently durable
    /// and calls are serialized beneath the archive's existing writer lease.
    /// Batched detached publishers must retain the loose path.
    fn permits_packed_head_transition(&self) -> bool {
        false
    }

    /// Independently durable immutable publication used only by an explicit
    /// private construction. Detached loose-node batches override this with a
    /// separate exact lane so a packed head can never outrun its prerequisites.
    fn publish_construction_exact(
        &self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
    ) -> Result<(), PatriciaPublicationError> {
        self.publish(dir, filename, bytes)
    }

    /// Opt in only when `publish_construction_exact` is independently durable
    /// and construction calls are serialized by the existing writer lease.
    fn permits_construction_packed_head_transition(&self) -> bool {
        self.permits_packed_head_transition()
    }

    /// Consume one synced construction prerequisite through the policy-owned
    /// collision/error boundary.
    fn publish_staged_construction_exact(
        &self,
        publication: StagedExactImmutablePublication,
    ) -> Result<(), PatriciaPublicationError> {
        publication.commit().map_err(PatriciaPublicationError::new)
    }
}

struct ConstructionExactPublisher<'a>(&'a dyn PatriciaNodePublisher);

impl PatriciaNodePublisher for ConstructionExactPublisher<'_> {
    fn publish(
        &self,
        dir: &Dir,
        filename: &str,
        bytes: &[u8],
    ) -> Result<(), PatriciaPublicationError> {
        self.0.publish_construction_exact(dir, filename, bytes)
    }

    fn publish_staged_construction_exact(
        &self,
        publication: StagedExactImmutablePublication,
    ) -> Result<(), PatriciaPublicationError> {
        self.0.publish_staged_construction_exact(publication)
    }
}

#[derive(Debug)]
pub enum PatriciaError {
    Filesystem(FilesystemError),
    Publication(PatriciaPublicationError),
    MissingNode(ContentDigest),
    PathMismatch(ContentDigest),
    Malformed,
}

/// Regular-file and byte accounting for one completed explicit packed-store
/// reclamation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatriciaIndexReclamationReport {
    pub examined_files: usize,
    pub examined_bytes: u64,
    pub deleted_files: usize,
    pub deleted_bytes: u64,
    pub retained_files: usize,
    pub retained_bytes: u64,
}

#[derive(Debug)]
pub enum PatriciaIndexReclamationError {
    Busy,
    Filesystem(FilesystemError),
    PathMismatch(ContentDigest),
    MalformedAuthority,
}

impl fmt::Display for PatriciaIndexReclamationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("packed Patricia storage is busy"),
            Self::Filesystem(error) => error.fmt(formatter),
            Self::PathMismatch(digest) => {
                write!(formatter, "packed Patricia content does not match {digest}")
            }
            Self::MalformedAuthority => formatter.write_str("malformed packed Patricia authority"),
        }
    }
}

impl std::error::Error for PatriciaIndexReclamationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Filesystem(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PackedPatriciaReclamationReport> for PatriciaIndexReclamationReport {
    fn from(report: PackedPatriciaReclamationReport) -> Self {
        Self {
            examined_files: report.examined_files,
            examined_bytes: report.examined_bytes,
            deleted_files: report.deleted_files,
            deleted_bytes: report.deleted_bytes,
            retained_files: report.retained_files,
            retained_bytes: report.retained_bytes,
        }
    }
}

impl From<PackedPatriciaReclamationError> for PatriciaIndexReclamationError {
    fn from(error: PackedPatriciaReclamationError) -> Self {
        match error {
            PackedPatriciaReclamationError::Busy => Self::Busy,
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::Filesystem(error)) => {
                Self::Filesystem(error)
            }
            PackedPatriciaReclamationError::Packed(PackedPatriciaError::PathMismatch(digest)) => {
                Self::PathMismatch(digest)
            }
            PackedPatriciaReclamationError::Packed(_) => Self::MalformedAuthority,
        }
    }
}

impl From<FilesystemError> for PatriciaError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error)
    }
}

impl From<std::io::Error> for PatriciaError {
    fn from(error: std::io::Error) -> Self {
        Self::Filesystem(FilesystemError::Io(error))
    }
}

impl From<PatriciaPublicationError> for PatriciaError {
    fn from(error: PatriciaPublicationError) -> Self {
        Self::Publication(error)
    }
}

const NODE_SCHEMA_VERSION: u32 = 1;
const MAX_KEY_BYTES: usize = 96;
const MAX_KEY_BITS: usize = MAX_KEY_BYTES * 8;
// Values are one immutable introduction each. Accumulated per-UUID history is
// structurally sharded across Patricia leaves and therefore never approaches
// this per-event corruption bound.
const MAX_VALUE_BYTES: usize = 4 * 1024;
const MAX_NODE_BYTES: u64 = 128 * 1024;
const NODE_SUFFIX: &str = ".patricia-node";

// Private bootstrap construction keeps newly addressed nodes hot across part
// boundaries. This is a total owned-memory ceiling, not a payload-byte target:
// the charge below includes every encoded node, a deliberately wide allowance
// for its Node/BTreeMap/allocator ownership and the authority sets, plus the
// complete worst-case mutation scratch reservation. Construction publication
// streams one postorder node at a time, so its only payload duplication is one
// bounded node encoding.
pub const DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES: usize = 512 * 1024 * 1024;

// One staged entry owns an inline digest and Node, up to three Vec allocations,
// a BTreeMap slot, and may also be named once by each authority set. On the
// repository-pinned 64-bit Rust target those inline values total less than 256
// bytes; 2 KiB per entry leaves more than 8x headroom for B-tree node slack and
// allocator metadata. The compile-time-sized portion is asserted in tests.
const CONSTRUCTION_ENTRY_OWNERSHIP_BYTES: usize = 2 * 1024;
const fn construction_min(left: usize, right: usize) -> usize {
    if left < right {
        left
    } else {
        right
    }
}

/// One sorted bulk range can create at most one leaf and one branch per
/// record. Bound both owned residency and the exact packed-catalog byte ceiling:
/// a machine may grant a 512 MiB construction budget while one canonical pack
/// remains capped at 64 MiB. The additional 128 bytes conservatively covers
/// each node's pack-index entry and framing beyond its maximum encoded body.
const CONSTRUCTION_BULK_NODE_PACK_BYTES: usize = CONSTRUCTION_MAX_VALID_NODE_BYTES + 128;
pub const MAX_PATRICIA_CONSTRUCTION_BULK_RECORDS: usize = construction_min(
    MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES / (2 * CONSTRUCTION_ENTRY_OWNERSHIP_BYTES),
    MAX_CATALOG_PACK_BYTES / (2 * CONSTRUCTION_BULK_NODE_PACK_BYTES),
);
// A valid leaf is bounded by the admitted key/value sizes; a branch is smaller.
// The extra 512 bytes covers postcard tags/lengths, and the factor of two
// covers Vec growth/allocator rounding while the encoded bytes overlap Node.
const CONSTRUCTION_MAX_VALID_NODE_BYTES: usize = MAX_VALUE_BYTES + MAX_KEY_BYTES + 512;
const CONSTRUCTION_ENCODING_SCRATCH_BYTES: usize = 2 * CONSTRUCTION_MAX_VALID_NODE_BYTES;
// An insertion can rebuild at most one branch per key bit plus its replacement
// leaf and split branch. This charge covers each possible new entry, the
// ancestor traversal frames, and the temporary encoding buffer before any
// mutation begins.
const CONSTRUCTION_BRANCH_PAYLOAD_BOUND: usize = MAX_KEY_BYTES + 256;
const CONSTRUCTION_TRAVERSAL_FRAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PatriciaIndexRoot(ContentDigest);

impl PatriciaIndexRoot {
    pub fn empty() -> Self {
        Self(ContentDigest::of(
            b"tine/authenticated-content-addressed-patricia/v1/empty",
        ))
    }

    pub const fn digest(self) -> ContentDigest {
        self.0
    }

    pub const fn from_digest(digest: ContentDigest) -> Self {
        Self(digest)
    }
}

impl Default for PatriciaIndexRoot {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatriciaIndexStats {
    pub reads: usize,
    pub writes: usize,
    pub bytes_read: usize,
    pub bytes_written: usize,
}

#[derive(Debug, Default)]
struct Counters {
    reads: AtomicUsize,
    writes: AtomicUsize,
    bytes_read: AtomicUsize,
    bytes_written: AtomicUsize,
}

pub struct PatriciaIndexStore {
    nodes: Dir,
    publisher: Box<dyn PatriciaNodePublisher>,
    packed: Mutex<Option<Option<PackedPatriciaCatalog>>>,
    counters: Counters,
    #[cfg(test)]
    packed_catalog_byte_limit: usize,
    #[cfg(test)]
    packed_publication_work: Mutex<Option<PackedPatriciaPublicationWork>>,
}

impl fmt::Debug for PatriciaIndexStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PatriciaIndexStore")
            .field("nodes", &self.nodes)
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Node {
    Leaf {
        schema_version: u32,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Branch {
        schema_version: u32,
        prefix: Vec<u8>,
        prefix_bit_len: u16,
        left: ContentDigest,
        right: ContentDigest,
    },
}

#[derive(Clone, Debug)]
struct ChildPathConstraint {
    parent_prefix: Vec<u8>,
    parent_prefix_bit_len: usize,
    right: bool,
}

#[derive(Debug)]
struct BranchFrame {
    prefix: Vec<u8>,
    prefix_bit_len: u16,
    left: ContentDigest,
    right: ContentDigest,
    rightward: bool,
}

#[derive(Clone, Copy)]
struct BulkRecord<'a> {
    key: &'a [u8],
    value: &'a [u8],
}

enum ConstructionSinkError {
    Capacity,
    Patricia(PatriciaError),
}

struct PackedConstructionPublication {
    work: PackedPatriciaPublicationWork,
    head_transition: bool,
    peak_resident_bytes: usize,
}

impl From<PatriciaError> for ConstructionSinkError {
    fn from(error: PatriciaError) -> Self {
        Self::Patricia(error)
    }
}

#[derive(Debug, Default)]
struct StagedNodes {
    nodes: BTreeMap<ContentDigest, Node>,
    encoded_bytes: usize,
}

impl StagedNodes {
    fn stage(&mut self, node: Node) -> Result<ContentDigest, PatriciaError> {
        validate_node(&node)?;
        let bytes = postcard::to_allocvec(&node).map_err(|_| PatriciaError::Malformed)?;
        if bytes.len() > CONSTRUCTION_MAX_VALID_NODE_BYTES || bytes.len() as u64 > MAX_NODE_BYTES {
            return Err(PatriciaError::Malformed);
        }
        let digest = ContentDigest::of(&bytes);
        if let std::collections::btree_map::Entry::Vacant(entry) = self.nodes.entry(digest) {
            self.encoded_bytes = self
                .encoded_bytes
                .checked_add(bytes.len())
                .ok_or(PatriciaError::Malformed)?;
            entry.insert(node);
        }
        Ok(digest)
    }

    fn clear(&mut self) {
        self.nodes.clear();
        self.encoded_bytes = 0;
    }

    fn owned_bytes(&self) -> usize {
        self.encoded_bytes.saturating_add(
            self.nodes
                .len()
                .saturating_mul(CONSTRUCTION_ENTRY_OWNERSHIP_BYTES),
        )
    }

    fn remove(&mut self, digest: ContentDigest, encoded_len: usize) {
        if self.nodes.remove(&digest).is_some() {
            self.encoded_bytes = self.encoded_bytes.saturating_sub(encoded_len);
        }
    }
}

fn sink_construction_node(
    sink: &mut PackedPatriciaConstructionSink,
    node: &Node,
) -> Result<ContentDigest, ConstructionSinkError> {
    validate_node(node)?;
    let bytes = postcard::to_allocvec(node)
        .map_err(|_| ConstructionSinkError::Patricia(PatriciaError::Malformed))?;
    if bytes.len() > CONSTRUCTION_MAX_VALID_NODE_BYTES || bytes.len() as u64 > MAX_NODE_BYTES {
        return Err(ConstructionSinkError::Patricia(PatriciaError::Malformed));
    }
    let digest = ContentDigest::of(&bytes);
    match sink.accept(digest, bytes) {
        Ok(true) => Ok(digest),
        Ok(false) => Err(ConstructionSinkError::Capacity),
        Err(error) => Err(ConstructionSinkError::Patricia(map_packed_patricia_error(
            error,
        ))),
    }
}

fn digest_node(node: &Node) -> Result<ContentDigest, PatriciaError> {
    validate_node(node)?;
    let bytes = postcard::to_allocvec(node).map_err(|_| PatriciaError::Malformed)?;
    if bytes.len() > CONSTRUCTION_MAX_VALID_NODE_BYTES || bytes.len() as u64 > MAX_NODE_BYTES {
        return Err(PatriciaError::Malformed);
    }
    Ok(ContentDigest::of(&bytes))
}

fn construction_sink_owned_limit(
    construction: &PatriciaIndexConstruction,
    retained_mutation_bytes: usize,
) -> Result<usize, PatriciaError> {
    construction
        .staged
        .owned_bytes()
        .checked_add(retained_mutation_bytes)
        .and_then(|retained| retained.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES))
        .and_then(|retained| construction.resident_budget_bytes.checked_sub(retained))
        .ok_or(PatriciaError::Malformed)
}

/// Single-use node construction shared by every checkpoint of one private
/// bootstrap session. Roots remain ordinary Patricia roots; only publication
/// timing changes.
#[derive(Debug)]
pub struct PatriciaIndexConstruction {
    staged: StagedNodes,
    checkpoint_roots: BTreeSet<ContentDigest>,
    live_roots: BTreeSet<ContentDigest>,
    resident_budget_bytes: usize,
    peak_resident_bytes: usize,
    peak_publication_resident_bytes: usize,
    flushes: usize,
    staged_nodes_at_publication: usize,
    published_staged_nodes: usize,
    logical_node_writes: usize,
    loose_publication_calls: usize,
    pack_publication_calls: usize,
    catalog_publication_calls: usize,
    head_transitions: usize,
    durability_barriers: usize,
    packed_bytes: usize,
    capacity_fallbacks: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatriciaIndexConstructionStats {
    pub peak_resident_bytes: usize,
    pub flushes: usize,
    pub logical_node_writes: usize,
    pub loose_publication_calls: usize,
    pub pack_publication_calls: usize,
    pub catalog_publication_calls: usize,
    pub immutable_publication_calls: usize,
    pub head_transitions: usize,
    pub durability_barriers: usize,
    pub packed_bytes: usize,
    pub capacity_fallbacks: usize,
}

/// Move-only evidence that one explicit construction published every retained
/// root through either completed packed-head transitions or its loose fallback.
pub struct CompletedPatriciaIndexConstruction {
    stats: PatriciaIndexConstructionStats,
}

impl CompletedPatriciaIndexConstruction {
    pub const fn stats(&self) -> PatriciaIndexConstructionStats {
        self.stats
    }
}

impl Default for PatriciaIndexConstruction {
    fn default() -> Self {
        Self {
            staged: StagedNodes::default(),
            checkpoint_roots: BTreeSet::new(),
            live_roots: BTreeSet::new(),
            resident_budget_bytes: DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
            peak_resident_bytes: 0,
            peak_publication_resident_bytes: 0,
            flushes: 0,
            staged_nodes_at_publication: 0,
            published_staged_nodes: 0,
            logical_node_writes: 0,
            loose_publication_calls: 0,
            pack_publication_calls: 0,
            catalog_publication_calls: 0,
            head_transitions: 0,
            durability_barriers: 0,
            packed_bytes: 0,
            capacity_fallbacks: 0,
        }
    }
}

impl PatriciaIndexConstruction {
    pub fn with_resident_budget(resident_budget_bytes: usize) -> Result<Self, PatriciaError> {
        if !(DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES
            ..=MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES)
            .contains(&resident_budget_bytes)
        {
            return Err(PatriciaError::Malformed);
        }
        Ok(Self {
            resident_budget_bytes,
            ..Self::default()
        })
    }

    pub const fn bulk_record_limit(&self) -> usize {
        construction_min(
            self.resident_budget_bytes / (2 * CONSTRUCTION_ENTRY_OWNERSHIP_BYTES),
            MAX_PATRICIA_CONSTRUCTION_BULK_RECORDS,
        )
    }

    pub const fn resident_budget_bytes(&self) -> usize {
        self.resident_budget_bytes
    }

    fn can_fit_residency(&self, additional_owned_bytes: usize) -> Result<bool, PatriciaError> {
        self.staged
            .owned_bytes()
            .checked_add(additional_owned_bytes)
            .map(|resident| resident <= self.resident_budget_bytes)
            .ok_or(PatriciaError::Malformed)
    }

    pub fn checkpoint(&mut self, roots: impl IntoIterator<Item = PatriciaIndexRoot>) {
        self.checkpoint_roots
            .extend(
                roots
                    .into_iter()
                    .map(PatriciaIndexRoot::digest)
                    .filter(|root| {
                        // A persisted root cannot reach an unpublished staged node.
                        self.staged.nodes.contains_key(root)
                    }),
            );
    }

    /// Replaces the complete set of roots that the caller currently treats as
    /// live construction authority. Historical roots are retained separately
    /// with [`Self::checkpoint`].
    pub fn set_live_roots(&mut self, roots: impl IntoIterator<Item = PatriciaIndexRoot>) {
        self.live_roots = roots
            .into_iter()
            .map(PatriciaIndexRoot::digest)
            .filter(|root| self.staged.nodes.contains_key(root))
            .collect();
    }

    fn note_residency(&mut self, additional_owned_bytes: usize) -> Result<(), PatriciaError> {
        let resident = self
            .staged
            .owned_bytes()
            .checked_add(additional_owned_bytes)
            .ok_or(PatriciaError::Malformed)?;
        self.peak_resident_bytes = self.peak_resident_bytes.max(resident);
        if resident > self.resident_budget_bytes {
            return Err(PatriciaError::Malformed);
        }
        Ok(())
    }

    fn prepare_mutation(
        &mut self,
        store: &PatriciaIndexStore,
        in_progress_root: PatriciaIndexRoot,
        mutation_reservation: usize,
    ) -> Result<(), PatriciaError> {
        let publication_reservation = construction_publication_reservation()?;
        let packed_publication_headroom = if store
            .publisher
            .permits_construction_packed_head_transition()
        {
            self.resident_budget_bytes
                .saturating_sub(self.resident_budget_bytes / 4)
        } else {
            publication_reservation
        };
        let projected = self
            .staged
            .owned_bytes()
            .checked_add(mutation_reservation.max(packed_publication_headroom))
            .ok_or(PatriciaError::Malformed)?;
        if projected > self.resident_budget_bytes && !self.staged.nodes.is_empty() {
            self.note_residency(publication_reservation)?;
            let staged_nodes = self.staged.nodes.len();
            let roots = self
                .checkpoint_roots
                .iter()
                .chain(&self.live_roots)
                .copied()
                .chain(std::iter::once(in_progress_root.digest()))
                .collect::<Vec<_>>();
            let (published, publication_peak) = store.publish_construction_roots(roots, self)?;
            self.peak_publication_resident_bytes =
                self.peak_publication_resident_bytes.max(publication_peak);
            self.peak_resident_bytes = self.peak_resident_bytes.max(publication_peak);
            self.staged_nodes_at_publication = self
                .staged_nodes_at_publication
                .saturating_add(staged_nodes);
            self.published_staged_nodes = self.published_staged_nodes.saturating_add(published);
            self.staged.clear();
            self.checkpoint_roots.clear();
            self.live_roots.clear();
            self.flushes = self.flushes.saturating_add(1);
        }
        self.note_residency(mutation_reservation)
    }

    fn persist_in_progress_root(
        &mut self,
        store: &PatriciaIndexStore,
        root: PatriciaIndexRoot,
    ) -> Result<(), PatriciaError> {
        if !self.staged.nodes.contains_key(&root.digest()) {
            return Ok(());
        }
        self.note_residency(construction_publication_reservation()?)?;
        let staged_nodes = self.staged.nodes.len();
        let (published, publication_peak) =
            store.publish_construction_roots([root.digest()], self)?;
        self.peak_publication_resident_bytes =
            self.peak_publication_resident_bytes.max(publication_peak);
        self.peak_resident_bytes = self.peak_resident_bytes.max(publication_peak);
        self.staged_nodes_at_publication = self
            .staged_nodes_at_publication
            .saturating_add(staged_nodes);
        self.published_staged_nodes = self.published_staged_nodes.saturating_add(published);
        Ok(())
    }

    pub fn stats(&self) -> PatriciaIndexConstructionStats {
        PatriciaIndexConstructionStats {
            peak_resident_bytes: self.peak_resident_bytes,
            flushes: self.flushes,
            logical_node_writes: self.logical_node_writes,
            loose_publication_calls: self.loose_publication_calls,
            pack_publication_calls: self.pack_publication_calls,
            catalog_publication_calls: self.catalog_publication_calls,
            immutable_publication_calls: self
                .loose_publication_calls
                .saturating_add(self.pack_publication_calls)
                .saturating_add(self.catalog_publication_calls),
            head_transitions: self.head_transitions,
            durability_barriers: self.durability_barriers,
            packed_bytes: self.packed_bytes,
            capacity_fallbacks: self.capacity_fallbacks,
        }
    }
}

impl PatriciaIndexStore {
    pub fn new(nodes: Dir, publisher: impl PatriciaNodePublisher + 'static) -> Self {
        Self {
            nodes,
            publisher: Box::new(publisher),
            packed: Mutex::new(None),
            counters: Counters::default(),
            #[cfg(test)]
            packed_catalog_byte_limit: MAX_CATALOG_PACK_BYTES,
            #[cfg(test)]
            packed_publication_work: Mutex::new(None),
        }
    }

    pub fn with_publisher(
        &self,
        publisher: impl PatriciaNodePublisher + 'static,
    ) -> Result<Self, PatriciaError> {
        Ok(Self {
            nodes: self.nodes.try_clone()?,
            publisher: Box::new(publisher),
            packed: Mutex::new(None),
            counters: Counters::default(),
            #[cfg(test)]
            packed_catalog_byte_limit: self.packed_catalog_byte_limit,
            #[cfg(test)]
            packed_publication_work: Mutex::new(None),
        })
    }

    pub fn stats(&self) -> PatriciaIndexStats {
        PatriciaIndexStats {
            reads: self.counters.reads.load(Ordering::Relaxed),
            writes: self.counters.writes.load(Ordering::Relaxed),
            bytes_read: self.counters.bytes_read.load(Ordering::Relaxed),
            bytes_written: self.counters.bytes_written.load(Ordering::Relaxed),
        }
    }

    /// Explicitly reclaim unreachable packed Patricia artifacts.
    ///
    /// This is the only Patricia operation which scans the node directory. It
    /// never waits for active readers or publishers: contention is reported as
    /// [`PatriciaIndexReclamationError::Busy`].
    pub fn reclaim_unreachable_packed_files(
        &self,
    ) -> Result<PatriciaIndexReclamationReport, PatriciaIndexReclamationError> {
        reclaim_unreachable_packed_files(&self.nodes)
            .map(PatriciaIndexReclamationReport::from)
            .map_err(PatriciaIndexReclamationError::from)
    }

    /// Test-support seam for proving that the adapter rejects corrupted bytes
    /// beneath an existing packed content-addressed path. This method remains
    /// unavailable unless the crate is built for its own tests or with the
    /// explicit `test-support` feature.
    #[cfg(any(test, feature = "test-support"))]
    pub fn corrupt_packed_node_for_test(&self, digest: ContentDigest) -> Result<(), PatriciaError> {
        corrupt_packed_node_for_test(&self.nodes, digest).map_err(map_packed_patricia_error)?;
        *self.packed.lock().map_err(|_| PatriciaError::Malformed)? = None;
        Ok(())
    }

    pub fn validate_root(&self, root: PatriciaIndexRoot) -> Result<(), PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return Ok(());
        }
        self.read_node(root.digest()).map(|_| ())
    }

    pub fn lookup(
        &self,
        root: PatriciaIndexRoot,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, PatriciaError> {
        validate_key(key)?;
        if root == PatriciaIndexRoot::empty() {
            return Ok(None);
        }
        let mut digest = root.digest();
        let mut constraint = None;
        let mut remaining_nodes = traversal_node_budget(key.len())?;
        loop {
            remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_node(digest)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf {
                    key: found, value, ..
                } => return Ok((found == key).then_some(value)),
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    if !prefix_matches(key, &prefix, split)? {
                        return Ok(None);
                    }
                    let rightward = key_bit(key, split)?;
                    digest = if rightward { right } else { left };
                    constraint = Some(ChildPathConstraint {
                        parent_prefix: prefix,
                        parent_prefix_bit_len: split,
                        right: rightward,
                    });
                }
            }
        }
    }

    #[allow(dead_code)] // consumed by the intentionally unwired P2N2 foundation
    pub fn lookup_many(
        &self,
        root: PatriciaIndexRoot,
        keys: &[Vec<u8>],
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, PatriciaError> {
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PatriciaError::Malformed);
        }
        keys.iter()
            .filter_map(|key| {
                self.lookup(root, key)
                    .transpose()
                    .map(|result| result.map(|value| (key.clone(), value)))
            })
            .collect()
    }

    pub fn lookup_prefix(
        &self,
        root: PatriciaIndexRoot,
        prefix: &[u8],
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, PatriciaError> {
        self.lookup_prefix_limited(root, prefix, usize::MAX)
    }

    pub fn lookup_prefix_limited(
        &self,
        root: PatriciaIndexRoot,
        prefix: &[u8],
        limit: usize,
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, PatriciaError> {
        validate_key(prefix)?;
        let mut found = BTreeMap::new();
        if root == PatriciaIndexRoot::empty() || limit == 0 {
            return Ok(found);
        }
        self.collect_prefix(root.digest(), prefix, limit, &mut found)?;
        Ok(found)
    }

    pub fn visit_all(
        &self,
        root: PatriciaIndexRoot,
        mut visit: impl FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return Ok(());
        }
        let budget = traversal_node_budget(MAX_KEY_BYTES)?;
        let mut pending = vec![(root.digest(), None, budget)];
        while let Some((digest, constraint, remaining_nodes)) = pending.pop() {
            let remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_node(digest)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf { key, value, .. } => {
                    if !visit(&key, &value) {
                        return Ok(());
                    }
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    pending.push((
                        right,
                        Some(ChildPathConstraint {
                            parent_prefix: prefix.clone(),
                            parent_prefix_bit_len: split,
                            right: true,
                        }),
                        remaining_nodes,
                    ));
                    pending.push((
                        left,
                        Some(ChildPathConstraint {
                            parent_prefix: prefix,
                            parent_prefix_bit_len: split,
                            right: false,
                        }),
                        remaining_nodes,
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn insert_many(
        &self,
        root: PatriciaIndexRoot,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        let (root, staged) = self.stage_many(root, records)?;
        self.publish_staged_reachable(root, &staged)?;
        Ok(root)
    }

    pub fn insert_many_verify_existing(
        &self,
        root: PatriciaIndexRoot,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        let (root, staged) = self.stage_many(root, records)?;
        self.verify_staged_reachable(root, &staged)?;
        Ok(root)
    }

    /// Derive the canonical root for a complete sorted record set without
    /// publishing nodes. Bootstrap cold replay uses this to compare semantic
    /// input with an already-published terminal index instead of rebuilding
    /// that index through the ordinary point-update path.
    pub fn derive_complete_root(
        &self,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        if records.is_empty() {
            return Ok(PatriciaIndexRoot::empty());
        }
        for (key, value) in records {
            validate_record(key, value)?;
        }
        let sorted = records
            .iter()
            .map(|(key, value)| BulkRecord {
                key: key.as_slice(),
                value: value.as_slice(),
            })
            .collect::<Vec<_>>();
        self.derive_complete_records(&sorted).map(PatriciaIndexRoot)
    }

    fn stage_many(
        &self,
        root: PatriciaIndexRoot,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<(PatriciaIndexRoot, StagedNodes), PatriciaError> {
        for (key, value) in records {
            validate_record(key, value)?;
        }
        let mut root = root;
        let mut staged = StagedNodes::default();
        for (key, value) in records {
            root = PatriciaIndexRoot(self.insert_staged(root, key, value, &mut staged)?);
        }
        Ok((root, staged))
    }

    pub fn construction_lookup(
        &self,
        construction: &PatriciaIndexConstruction,
        root: PatriciaIndexRoot,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, PatriciaError> {
        validate_key(key)?;
        if root == PatriciaIndexRoot::empty() {
            return Ok(None);
        }
        let mut digest = root.digest();
        let mut constraint = None;
        let mut remaining_nodes = traversal_node_budget(key.len())?;
        loop {
            remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_staged_or_persisted(digest, &construction.staged)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf {
                    key: found, value, ..
                } => return Ok((found == key).then_some(value)),
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    if !prefix_matches(key, &prefix, split)? {
                        return Ok(None);
                    }
                    let rightward = key_bit(key, split)?;
                    digest = if rightward { right } else { left };
                    constraint = Some(ChildPathConstraint {
                        parent_prefix: prefix,
                        parent_prefix_bit_len: split,
                        right: rightward,
                    });
                }
            }
        }
    }

    pub fn construction_validate_root(
        &self,
        construction: &PatriciaIndexConstruction,
        root: PatriciaIndexRoot,
    ) -> Result<(), PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return Ok(());
        }
        self.read_staged_or_persisted(root.digest(), &construction.staged)
            .and_then(|node| validate_node(&node))
    }

    pub fn construction_lookup_prefix_limited(
        &self,
        construction: &PatriciaIndexConstruction,
        root: PatriciaIndexRoot,
        prefix: &[u8],
        limit: usize,
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, PatriciaError> {
        validate_key(prefix)?;
        let mut found = BTreeMap::new();
        if root == PatriciaIndexRoot::empty() || limit == 0 {
            return Ok(found);
        }
        let budget = traversal_node_budget(MAX_KEY_BYTES)?;
        let mut pending = vec![(root.digest(), None, budget)];
        while let Some((digest, constraint, remaining_nodes)) = pending.pop() {
            let remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_staged_or_persisted(digest, &construction.staged)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf { key, value, .. } => {
                    if key.starts_with(prefix) {
                        found.insert(key, value);
                        if found.len() == limit {
                            break;
                        }
                    }
                }
                Node::Branch {
                    prefix: branch_prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    let requested_bits = key_bit_len(prefix)?;
                    let compared = split.min(requested_bits);
                    if !prefix_matches(prefix, &branch_prefix, compared)? {
                        continue;
                    }
                    if requested_bits <= split {
                        pending.push((
                            right,
                            Some(ChildPathConstraint {
                                parent_prefix: branch_prefix.clone(),
                                parent_prefix_bit_len: split,
                                right: true,
                            }),
                            remaining_nodes,
                        ));
                        pending.push((
                            left,
                            Some(ChildPathConstraint {
                                parent_prefix: branch_prefix,
                                parent_prefix_bit_len: split,
                                right: false,
                            }),
                            remaining_nodes,
                        ));
                    } else {
                        let rightward = key_bit(prefix, split)?;
                        pending.push((
                            if rightward { right } else { left },
                            Some(ChildPathConstraint {
                                parent_prefix: branch_prefix,
                                parent_prefix_bit_len: split,
                                right: rightward,
                            }),
                            remaining_nodes,
                        ));
                    }
                }
            }
        }
        Ok(found)
    }

    pub fn construction_visit_all(
        &self,
        construction: &PatriciaIndexConstruction,
        root: PatriciaIndexRoot,
        mut visit: impl FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return Ok(());
        }
        let budget = traversal_node_budget(MAX_KEY_BYTES)?;
        let mut pending = vec![(root.digest(), None, budget)];
        while let Some((digest, constraint, remaining_nodes)) = pending.pop() {
            let remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_staged_or_persisted(digest, &construction.staged)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf { key, value, .. } => {
                    if !visit(&key, &value) {
                        return Ok(());
                    }
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    pending.push((
                        right,
                        Some(ChildPathConstraint {
                            parent_prefix: prefix.clone(),
                            parent_prefix_bit_len: split,
                            right: true,
                        }),
                        remaining_nodes,
                    ));
                    pending.push((
                        left,
                        Some(ChildPathConstraint {
                            parent_prefix: prefix,
                            parent_prefix_bit_len: split,
                            right: false,
                        }),
                        remaining_nodes,
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn construction_insert_many(
        &self,
        construction: &mut PatriciaIndexConstruction,
        mut root: PatriciaIndexRoot,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        for (key, value) in records {
            validate_record(key, value)?;
            construction.prepare_mutation(
                self,
                root,
                construction_mutation_reservation(key.len(), value.len(), true)?,
            )?;
            root = PatriciaIndexRoot(self.insert_staged(
                root,
                key,
                value,
                &mut construction.staged,
            )?);
        }
        Ok(root)
    }

    /// Construction-only canonical insertion which merges sorted record
    /// ranges at each affected subtree. Completed children publish before
    /// their parent, so the merge retains only its bounded traversal stack
    /// instead of staging every path copy. Exact immutable publication makes a
    /// retry after any child or parent failure idempotent.
    pub fn construction_insert_many_bulk(
        &self,
        construction: &mut PatriciaIndexConstruction,
        mut root: PatriciaIndexRoot,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        if records.is_empty() {
            return Ok(root);
        }
        for (key, value) in records {
            validate_record(key, value)?;
        }
        let record_limit = construction.bulk_record_limit().max(1);
        if records.len() > record_limit {
            // A complete sorted range may legitimately exceed one bounded
            // construction sink (dense reverse-reference indices are the
            // common case). Falling back to per-key insertion here turns a
            // handful of packed publications into hundreds of thousands of
            // loose immutable files, especially costly on Windows. Preserve
            // canonical Patricia semantics by applying consecutive sorted
            // ranges to the prior range's root. Each recursive call is within
            // the same construction residency ceiling and retains the exact
            // existing per-range capacity fallback.
            let mut chunk = BTreeMap::new();
            for (key, value) in records {
                chunk.insert(key.clone(), value.clone());
                if chunk.len() == record_limit {
                    root = self.construction_insert_many_bulk(construction, root, &chunk)?;
                    chunk.clear();
                }
            }
            if !chunk.is_empty() {
                root = self.construction_insert_many_bulk(construction, root, &chunk)?;
            }
            return Ok(root);
        }
        let minimum_record_refs_bytes = records
            .len()
            .checked_mul(std::mem::size_of::<BulkRecord<'_>>())
            .ok_or(PatriciaError::Malformed)?;
        let minimum_reservation = construction_bulk_reservation(minimum_record_refs_bytes)?;
        if minimum_reservation > construction.resident_budget_bytes {
            return self.construction_insert_many(construction, root, records);
        }

        // First preflight the minimum required capacity against all retained
        // construction ownership. Only then allocate the bounded reference
        // vector. Rust may grant more capacity than requested, so charge the
        // actual allocation before populating it or publishing any bulk node.
        construction.prepare_mutation(self, root, minimum_reservation)?;
        let mut sorted = Vec::new();
        if sorted.try_reserve_exact(records.len()).is_err() {
            return self.construction_insert_many(construction, root, records);
        }
        let actual_record_refs_bytes = sorted
            .capacity()
            .checked_mul(std::mem::size_of::<BulkRecord<'_>>())
            .ok_or(PatriciaError::Malformed)?;
        let reservation = construction_bulk_reservation(actual_record_refs_bytes)?;
        if reservation > construction.resident_budget_bytes {
            drop(sorted);
            return self.construction_insert_many(construction, root, records);
        }
        if !construction.can_fit_residency(reservation)? {
            // Drop the first allocation before publishing the staged roots
            // needed to make room. Reallocate at most once afterward, then
            // charge that allocation's independently observed capacity.
            drop(sorted);
            construction.prepare_mutation(self, root, reservation)?;
            sorted = Vec::new();
            if sorted.try_reserve_exact(records.len()).is_err() {
                return self.construction_insert_many(construction, root, records);
            }
            let retry_record_refs_bytes = sorted
                .capacity()
                .checked_mul(std::mem::size_of::<BulkRecord<'_>>())
                .ok_or(PatriciaError::Malformed)?;
            let retry_reservation = construction_bulk_reservation(retry_record_refs_bytes)?;
            if retry_reservation > construction.resident_budget_bytes {
                drop(sorted);
                return self.construction_insert_many(construction, root, records);
            }
            construction.note_residency(retry_reservation)?;
        } else {
            construction.note_residency(reservation)?;
        }
        construction.persist_in_progress_root(self, root)?;
        sorted.extend(records.iter().map(|(key, value)| BulkRecord {
            key: key.as_slice(),
            value: value.as_slice(),
        }));
        let sink_owned_limit = construction_sink_owned_limit(construction, reservation)?;
        let mut sink = PackedPatriciaConstructionSink::new(sink_owned_limit);
        let rebuilt = if root == PatriciaIndexRoot::empty() {
            self.sink_bulk_records(&sorted, &mut sink)
        } else {
            self.sink_bulk_insert_at(root.digest(), &sorted, construction, None, &mut sink)
        };
        let rebuilt = match rebuilt {
            Ok(rebuilt) => rebuilt,
            Err(ConstructionSinkError::Capacity) => {
                construction.capacity_fallbacks = construction.capacity_fallbacks.saturating_add(1);
                construction.peak_resident_bytes = construction.peak_resident_bytes.max(
                    construction
                        .staged
                        .owned_bytes()
                        .saturating_add(reservation)
                        .saturating_add(sink.owned_bytes()),
                );
                drop(sink);
                drop(sorted);
                return self.construction_insert_many(construction, root, records);
            }
            Err(ConstructionSinkError::Patricia(error)) => return Err(error),
        };
        self.publish_construction_sink(construction, &sink, reservation)?;
        Ok(PatriciaIndexRoot(rebuilt))
    }

    pub fn construction_remove_many(
        &self,
        construction: &mut PatriciaIndexConstruction,
        mut root: PatriciaIndexRoot,
        keys: &[Vec<u8>],
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PatriciaError::Malformed);
        }
        for key in keys {
            validate_key(key)?;
            construction.prepare_mutation(
                self,
                root,
                construction_mutation_reservation(key.len(), 0, false)?,
            )?;
            root = self.remove_constructed(construction, root, key)?;
        }
        Ok(root)
    }

    pub fn finish_construction(
        &self,
        construction: &mut PatriciaIndexConstruction,
    ) -> Result<CompletedPatriciaIndexConstruction, PatriciaError> {
        let staged_nodes = construction.staged.nodes.len();
        construction.note_residency(construction_publication_reservation()?)?;
        let roots = construction
            .checkpoint_roots
            .iter()
            .chain(&construction.live_roots)
            .copied()
            .collect::<Vec<_>>();
        let (published, publication_peak) = self.publish_construction_roots(roots, construction)?;
        construction.peak_resident_bytes = construction.peak_resident_bytes.max(publication_peak);
        construction.peak_publication_resident_bytes = construction
            .peak_publication_resident_bytes
            .max(publication_peak);
        if publication_peak > construction.resident_budget_bytes {
            return Err(PatriciaError::Malformed);
        }
        construction.staged_nodes_at_publication = construction
            .staged_nodes_at_publication
            .saturating_add(staged_nodes);
        construction.published_staged_nodes = construction
            .published_staged_nodes
            .saturating_add(published);
        construction.staged.clear();
        construction.checkpoint_roots.clear();
        construction.live_roots.clear();
        Ok(CompletedPatriciaIndexConstruction {
            stats: construction.stats(),
        })
    }

    pub fn remove_many(
        &self,
        root: PatriciaIndexRoot,
        keys: &[Vec<u8>],
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PatriciaError::Malformed);
        }
        let mut root = root;
        for key in keys {
            validate_key(key)?;
            root = self.remove(root, key)?;
        }
        Ok(root)
    }

    fn remove(
        &self,
        root: PatriciaIndexRoot,
        key: &[u8],
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return Ok(root);
        }
        let mut digest = root.digest();
        let mut constraint = None;
        let mut remaining_nodes = traversal_node_budget(key.len())?;
        let mut ancestors = Vec::new();
        loop {
            remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_node(digest)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf { key: found, .. } => {
                    if found != key {
                        return Ok(root);
                    }
                    break;
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    if !prefix_matches(key, &prefix, split)? {
                        return Ok(root);
                    }
                    let rightward = key_bit(key, split)?;
                    digest = if rightward { right } else { left };
                    constraint = Some(ChildPathConstraint {
                        parent_prefix: prefix.clone(),
                        parent_prefix_bit_len: split,
                        right: rightward,
                    });
                    ancestors.push(BranchFrame {
                        prefix,
                        prefix_bit_len,
                        left,
                        right,
                        rightward,
                    });
                }
            }
        }

        let Some(parent) = ancestors.pop() else {
            return Ok(PatriciaIndexRoot::empty());
        };
        let replacement = if parent.rightward {
            parent.left
        } else {
            parent.right
        };
        let rebuilt = ancestors
            .into_iter()
            .rev()
            .try_fold(replacement, |child, ancestor| {
                let (left, right) = if ancestor.rightward {
                    (ancestor.left, child)
                } else {
                    (child, ancestor.right)
                };
                self.publish_node(&Node::Branch {
                    schema_version: NODE_SCHEMA_VERSION,
                    prefix: ancestor.prefix,
                    prefix_bit_len: ancestor.prefix_bit_len,
                    left,
                    right,
                })
            })?;
        Ok(PatriciaIndexRoot(rebuilt))
    }

    fn remove_constructed(
        &self,
        construction: &mut PatriciaIndexConstruction,
        root: PatriciaIndexRoot,
        key: &[u8],
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return Ok(root);
        }
        let mut digest = root.digest();
        let mut constraint = None;
        let mut remaining_nodes = traversal_node_budget(key.len())?;
        let mut ancestors = Vec::new();
        loop {
            remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_staged_or_persisted(digest, &construction.staged)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf { key: found, .. } => {
                    if found != key {
                        return Ok(root);
                    }
                    break;
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    if !prefix_matches(key, &prefix, split)? {
                        return Ok(root);
                    }
                    let rightward = key_bit(key, split)?;
                    digest = if rightward { right } else { left };
                    constraint = Some(ChildPathConstraint {
                        parent_prefix: prefix.clone(),
                        parent_prefix_bit_len: split,
                        right: rightward,
                    });
                    ancestors.push(BranchFrame {
                        prefix,
                        prefix_bit_len,
                        left,
                        right,
                        rightward,
                    });
                }
            }
        }

        let Some(parent) = ancestors.pop() else {
            return Ok(PatriciaIndexRoot::empty());
        };
        let replacement = if parent.rightward {
            parent.left
        } else {
            parent.right
        };
        let rebuilt = ancestors
            .into_iter()
            .rev()
            .try_fold(replacement, |child, ancestor| {
                let (left, right) = if ancestor.rightward {
                    (ancestor.left, child)
                } else {
                    (child, ancestor.right)
                };
                construction.staged.stage(Node::Branch {
                    schema_version: NODE_SCHEMA_VERSION,
                    prefix: ancestor.prefix,
                    prefix_bit_len: ancestor.prefix_bit_len,
                    left,
                    right,
                })
            })?;
        Ok(PatriciaIndexRoot(rebuilt))
    }

    fn sink_bulk_records(
        &self,
        records: &[BulkRecord<'_>],
        sink: &mut PackedPatriciaConstructionSink,
    ) -> Result<ContentDigest, ConstructionSinkError> {
        let first = records.first().ok_or(PatriciaError::Malformed)?;
        if records.len() == 1 {
            return sink_construction_node(
                sink,
                &Node::Leaf {
                    schema_version: NODE_SCHEMA_VERSION,
                    key: first.key.to_vec(),
                    value: first.value.to_vec(),
                },
            );
        }
        let last = records.last().ok_or(PatriciaError::Malformed)?;
        let split = common_prefix_bits(first.key, last.key, key_bit_len(first.key)?)?;
        let partition = bulk_partition(records, split)?;
        if partition == 0 || partition == records.len() {
            return Err(PatriciaError::Malformed.into());
        }
        let left = self.sink_bulk_records(&records[..partition], sink)?;
        let right = self.sink_bulk_records(&records[partition..], sink)?;
        let node = Node::Branch {
            schema_version: NODE_SCHEMA_VERSION,
            prefix: masked_prefix(first.key, split),
            prefix_bit_len: u16::try_from(split).map_err(|_| PatriciaError::Malformed)?,
            left,
            right,
        };
        sink_construction_node(sink, &node)
    }

    fn derive_complete_records(
        &self,
        records: &[BulkRecord<'_>],
    ) -> Result<ContentDigest, PatriciaError> {
        let first = records.first().ok_or(PatriciaError::Malformed)?;
        if records.len() == 1 {
            return digest_node(&Node::Leaf {
                schema_version: NODE_SCHEMA_VERSION,
                key: first.key.to_vec(),
                value: first.value.to_vec(),
            });
        }
        let last = records.last().ok_or(PatriciaError::Malformed)?;
        let split = common_prefix_bits(first.key, last.key, key_bit_len(first.key)?)?;
        let partition = bulk_partition(records, split)?;
        if partition == 0 || partition == records.len() {
            return Err(PatriciaError::Malformed);
        }
        let left = self.derive_complete_records(&records[..partition])?;
        let right = self.derive_complete_records(&records[partition..])?;
        digest_node(&Node::Branch {
            schema_version: NODE_SCHEMA_VERSION,
            prefix: masked_prefix(first.key, split),
            prefix_bit_len: u16::try_from(split).map_err(|_| PatriciaError::Malformed)?,
            left,
            right,
        })
    }

    fn sink_bulk_insert_at(
        &self,
        digest: ContentDigest,
        records: &[BulkRecord<'_>],
        construction: &mut PatriciaIndexConstruction,
        constraint: Option<&ChildPathConstraint>,
        sink: &mut PackedPatriciaConstructionSink,
    ) -> Result<ContentDigest, ConstructionSinkError> {
        let node = self.read_constructed_or_persisted(digest, construction)?;
        validate_node_path(&node, constraint)?;
        let first = records.first().ok_or(PatriciaError::Malformed)?;
        let last = records.last().ok_or(PatriciaError::Malformed)?;
        let prefix = node_prefix(&node);
        let prefix_bits = node_prefix_bits(&node)?;
        let shared = common_prefix_bits(first.key, prefix, prefix_bits)?.min(common_prefix_bits(
            last.key,
            prefix,
            prefix_bits,
        )?);
        if shared < prefix_bits {
            let partition = bulk_partition(records, shared)?;
            let existing_right = key_bit(prefix, shared)?;
            let (left, right) = if existing_right {
                if partition == 0 {
                    return Err(PatriciaError::Malformed.into());
                }
                let left = self.sink_bulk_records(&records[..partition], sink)?;
                let right = if partition == records.len() {
                    digest
                } else {
                    self.sink_bulk_insert_at(
                        digest,
                        &records[partition..],
                        construction,
                        constraint,
                        sink,
                    )?
                };
                (left, right)
            } else {
                if partition == records.len() {
                    return Err(PatriciaError::Malformed.into());
                }
                let left = if partition == 0 {
                    digest
                } else {
                    self.sink_bulk_insert_at(
                        digest,
                        &records[..partition],
                        construction,
                        constraint,
                        sink,
                    )?
                };
                let right = self.sink_bulk_records(&records[partition..], sink)?;
                (left, right)
            };
            let node = Node::Branch {
                schema_version: NODE_SCHEMA_VERSION,
                prefix: masked_prefix(prefix, shared),
                prefix_bit_len: u16::try_from(shared).map_err(|_| PatriciaError::Malformed)?,
                left,
                right,
            };
            return sink_construction_node(sink, &node);
        }

        match node {
            Node::Leaf {
                key,
                value: found_value,
                ..
            } => {
                if records.len() != 1 || first.key != key {
                    return Err(PatriciaError::Malformed.into());
                }
                if first.value == found_value {
                    Ok(digest)
                } else {
                    sink_construction_node(
                        sink,
                        &Node::Leaf {
                            schema_version: NODE_SCHEMA_VERSION,
                            key: first.key.to_vec(),
                            value: first.value.to_vec(),
                        },
                    )
                }
            }
            Node::Branch {
                prefix,
                prefix_bit_len,
                left,
                right,
                ..
            } => {
                let split = prefix_bit_len as usize;
                let partition = bulk_partition(records, split)?;
                let left = if partition == 0 {
                    left
                } else {
                    let child_constraint = ChildPathConstraint {
                        parent_prefix: prefix.clone(),
                        parent_prefix_bit_len: split,
                        right: false,
                    };
                    self.sink_bulk_insert_at(
                        left,
                        &records[..partition],
                        construction,
                        Some(&child_constraint),
                        sink,
                    )?
                };
                let right = if partition == records.len() {
                    right
                } else {
                    let child_constraint = ChildPathConstraint {
                        parent_prefix: prefix.clone(),
                        parent_prefix_bit_len: split,
                        right: true,
                    };
                    self.sink_bulk_insert_at(
                        right,
                        &records[partition..],
                        construction,
                        Some(&child_constraint),
                        sink,
                    )?
                };
                let node = Node::Branch {
                    schema_version: NODE_SCHEMA_VERSION,
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                };
                sink_construction_node(sink, &node)
            }
        }
    }

    fn insert_staged(
        &self,
        root: PatriciaIndexRoot,
        key: &[u8],
        value: &[u8],
        staged: &mut StagedNodes,
    ) -> Result<ContentDigest, PatriciaError> {
        if root == PatriciaIndexRoot::empty() {
            return staged.stage(Node::Leaf {
                schema_version: NODE_SCHEMA_VERSION,
                key: key.to_vec(),
                value: value.to_vec(),
            });
        }
        self.insert_at_staged(root.digest(), key, value, staged)
    }

    fn insert_at_staged(
        &self,
        root: ContentDigest,
        key: &[u8],
        value: &[u8],
        staged: &mut StagedNodes,
    ) -> Result<ContentDigest, PatriciaError> {
        let mut digest = root;
        let mut constraint = None;
        let mut remaining_nodes = traversal_node_budget(key.len())?;
        let mut ancestors = Vec::new();
        let replacement = loop {
            remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_staged_or_persisted(digest, staged)?;
            validate_node_path(&node, constraint.as_ref())?;
            let node_prefix = node_prefix(&node);
            let node_prefix_bits = node_prefix_bits(&node)?;
            let shared = common_prefix_bits(key, node_prefix, node_prefix_bits)?;
            if shared < node_prefix_bits {
                let leaf = staged.stage(Node::Leaf {
                    schema_version: NODE_SCHEMA_VERSION,
                    key: key.to_vec(),
                    value: value.to_vec(),
                })?;
                break Self::stage_split(staged, key, shared, digest, node_prefix, leaf)?;
            }

            match node {
                Node::Leaf {
                    key: found_key,
                    value: found_value,
                    ..
                } => {
                    if found_key == key {
                        if found_value == value {
                            break digest;
                        }
                        break staged.stage(Node::Leaf {
                            schema_version: NODE_SCHEMA_VERSION,
                            key: key.to_vec(),
                            value: value.to_vec(),
                        })?;
                    }
                    let shared = common_prefix_bits(key, &found_key, key_bit_len(key)?)?;
                    let leaf = staged.stage(Node::Leaf {
                        schema_version: NODE_SCHEMA_VERSION,
                        key: key.to_vec(),
                        value: value.to_vec(),
                    })?;
                    break Self::stage_split(staged, key, shared, digest, &found_key, leaf)?;
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    let rightward = key_bit(key, split)?;
                    digest = if rightward { right } else { left };
                    constraint = Some(ChildPathConstraint {
                        parent_prefix: prefix.clone(),
                        parent_prefix_bit_len: split,
                        right: rightward,
                    });
                    ancestors.push(BranchFrame {
                        prefix,
                        prefix_bit_len,
                        left,
                        right,
                        rightward,
                    });
                }
            }
        };

        ancestors
            .into_iter()
            .rev()
            .try_fold(replacement, |child, ancestor| {
                let (left, right) = if ancestor.rightward {
                    (ancestor.left, child)
                } else {
                    (child, ancestor.right)
                };
                staged.stage(Node::Branch {
                    schema_version: NODE_SCHEMA_VERSION,
                    prefix: ancestor.prefix,
                    prefix_bit_len: ancestor.prefix_bit_len,
                    left,
                    right,
                })
            })
    }

    fn stage_split(
        staged: &mut StagedNodes,
        key: &[u8],
        shared: usize,
        existing: ContentDigest,
        existing_prefix: &[u8],
        leaf: ContentDigest,
    ) -> Result<ContentDigest, PatriciaError> {
        let key_right = key_bit(key, shared)?;
        let existing_right = key_bit(existing_prefix, shared)?;
        if key_right == existing_right {
            return Err(PatriciaError::Malformed);
        }
        let (left, right) = if key_right {
            (existing, leaf)
        } else {
            (leaf, existing)
        };
        staged.stage(Node::Branch {
            schema_version: NODE_SCHEMA_VERSION,
            prefix: masked_prefix(key, shared),
            prefix_bit_len: u16::try_from(shared).map_err(|_| PatriciaError::Malformed)?,
            left,
            right,
        })
    }

    fn read_staged_or_persisted(
        &self,
        digest: ContentDigest,
        staged: &StagedNodes,
    ) -> Result<Node, PatriciaError> {
        match staged.nodes.get(&digest) {
            Some(node) => Ok(node.clone()),
            None => self.read_node(digest),
        }
    }

    fn read_constructed_or_persisted(
        &self,
        digest: ContentDigest,
        construction: &PatriciaIndexConstruction,
    ) -> Result<Node, PatriciaError> {
        construction
            .staged
            .nodes
            .get(&digest)
            .cloned()
            .map_or_else(|| self.read_node(digest), Ok)
    }

    fn publish_staged_reachable(
        &self,
        root: PatriciaIndexRoot,
        staged: &StagedNodes,
    ) -> Result<(), PatriciaError> {
        self.publish_staged_roots(&BTreeSet::from([root.digest()]), staged)
            .map(|_| ())
    }

    /// Publish construction-owned nodes child-before-parent. Construction-
    /// capable publishers first collect one bounded exact-byte sink and append
    /// it through the existing packed format; every capacity refusal occurs
    /// before the first packed prerequisite and uses the original loose path.
    fn publish_construction_roots(
        &self,
        roots: impl IntoIterator<Item = ContentDigest>,
        construction: &mut PatriciaIndexConstruction,
    ) -> Result<(usize, usize), PatriciaError> {
        let roots = roots.into_iter().collect::<Vec<_>>();
        if !self.publisher.permits_construction_packed_head_transition() {
            return self.publish_construction_roots_loose(&roots, construction);
        }

        let retained_traversal_bytes = construction_publication_reservation()?
            .checked_add(
                roots
                    .capacity()
                    .checked_mul(std::mem::size_of::<ContentDigest>())
                    .ok_or(PatriciaError::Malformed)?,
            )
            .ok_or(PatriciaError::Malformed)?;
        let sink_owned_limit =
            construction_sink_owned_limit(construction, retained_traversal_bytes)?;
        let mut sink = PackedPatriciaConstructionSink::new(sink_owned_limit);
        let mut peak_owned_bytes = construction
            .staged
            .owned_bytes()
            .checked_add(retained_traversal_bytes)
            .and_then(|bytes| bytes.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES))
            .ok_or(PatriciaError::Malformed)?;
        for root in &roots {
            let mut pending = vec![(*root, false)];
            while let Some((digest, expanded)) = pending.pop() {
                let Some(node) = construction.staged.nodes.get(&digest) else {
                    continue;
                };
                if !expanded {
                    pending.push((digest, true));
                    if let Node::Branch { left, right, .. } = node {
                        pending.push((*right, false));
                        pending.push((*left, false));
                    }
                    peak_owned_bytes = peak_owned_bytes.max(
                        construction
                            .staged
                            .owned_bytes()
                            .saturating_add(retained_traversal_bytes)
                            .saturating_add(sink.owned_bytes())
                            .saturating_add(
                                pending.capacity().saturating_mul(std::mem::size_of::<(
                                    ContentDigest,
                                    bool,
                                )>(
                                )),
                            ),
                    );
                    continue;
                }

                let bytes = postcard::to_allocvec(node).map_err(|_| PatriciaError::Malformed)?;
                if bytes.len() > CONSTRUCTION_MAX_VALID_NODE_BYTES
                    || bytes.len() as u64 > MAX_NODE_BYTES
                    || ContentDigest::of(&bytes) != digest
                {
                    return Err(PatriciaError::PathMismatch(digest));
                }
                match sink.accept(digest, bytes) {
                    Ok(true) => {}
                    Ok(false) => {
                        construction.capacity_fallbacks =
                            construction.capacity_fallbacks.saturating_add(1);
                        let sink_peak = construction
                            .staged
                            .owned_bytes()
                            .checked_add(retained_traversal_bytes)
                            .and_then(|bytes| bytes.checked_add(sink.owned_bytes()))
                            .and_then(|bytes| {
                                bytes.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES)
                            })
                            .ok_or(PatriciaError::Malformed)?;
                        construction.peak_resident_bytes =
                            construction.peak_resident_bytes.max(sink_peak);
                        drop(sink);
                        return self.publish_construction_roots_loose(&roots, construction);
                    }
                    Err(error) => return Err(map_packed_patricia_error(error)),
                }
            }
        }
        peak_owned_bytes = peak_owned_bytes.max(
            construction
                .staged
                .owned_bytes()
                .checked_add(retained_traversal_bytes)
                .and_then(|bytes| bytes.checked_add(sink.owned_bytes()))
                .and_then(|bytes| bytes.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES))
                .ok_or(PatriciaError::Malformed)?,
        );
        let publication_peak =
            self.publish_construction_sink(construction, &sink, retained_traversal_bytes)?;
        peak_owned_bytes = peak_owned_bytes.max(publication_peak);
        for digest in sink.child_before_parent() {
            if let Some(bytes) = sink.entries().get(digest) {
                construction.staged.remove(*digest, bytes.len());
            }
        }
        Ok((sink.len(), peak_owned_bytes))
    }

    fn publish_construction_roots_loose(
        &self,
        roots: &[ContentDigest],
        construction: &mut PatriciaIndexConstruction,
    ) -> Result<(usize, usize), PatriciaError> {
        let _operation =
            lock_packed_operation_shared(&self.nodes).map_err(map_packed_patricia_error)?;
        let mut published = 0_usize;
        let root_bytes = roots
            .len()
            .saturating_mul(std::mem::size_of::<ContentDigest>());
        let mut peak_owned_bytes = construction.staged.owned_bytes().saturating_add(root_bytes);
        if peak_owned_bytes > construction.resident_budget_bytes {
            return Err(PatriciaError::Malformed);
        }
        for root in roots {
            let mut pending = vec![(*root, false)];
            while let Some((digest, expanded)) = pending.pop() {
                let Some(node) = construction.staged.nodes.get(&digest) else {
                    continue;
                };
                if !expanded {
                    pending.push((digest, true));
                    if let Node::Branch { left, right, .. } = node {
                        pending.push((*right, false));
                        pending.push((*left, false));
                    }
                    peak_owned_bytes = peak_owned_bytes.max(
                        construction
                            .staged
                            .owned_bytes()
                            .saturating_add(root_bytes)
                            .saturating_add(
                                pending.capacity().saturating_mul(std::mem::size_of::<(
                                    ContentDigest,
                                    bool,
                                )>(
                                )),
                            ),
                    );
                    if peak_owned_bytes > construction.resident_budget_bytes {
                        return Err(PatriciaError::Malformed);
                    }
                    continue;
                }

                let bytes = postcard::to_allocvec(node).map_err(|_| PatriciaError::Malformed)?;
                if bytes.len() > CONSTRUCTION_MAX_VALID_NODE_BYTES
                    || bytes.len() as u64 > MAX_NODE_BYTES
                    || ContentDigest::of(&bytes) != digest
                {
                    return Err(PatriciaError::PathMismatch(digest));
                }
                peak_owned_bytes = peak_owned_bytes.max(
                    construction
                        .staged
                        .owned_bytes()
                        .saturating_add(root_bytes)
                        .saturating_add(bytes.capacity())
                        .saturating_add(pending.capacity().saturating_mul(std::mem::size_of::<(
                            ContentDigest,
                            bool,
                        )>(
                        ))),
                );
                if peak_owned_bytes > construction.resident_budget_bytes {
                    return Err(PatriciaError::Malformed);
                }
                self.publisher
                    .publish(&self.nodes, &node_filename(digest), &bytes)
                    .map_err(PatriciaError::Publication)?;
                construction.staged.remove(digest, bytes.len());
                published = published.saturating_add(1);
                construction.logical_node_writes =
                    construction.logical_node_writes.saturating_add(1);
                construction.loose_publication_calls =
                    construction.loose_publication_calls.saturating_add(1);
                self.counters.writes.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .bytes_written
                    .fetch_add(bytes.len(), Ordering::Relaxed);
            }
        }
        Ok((published, peak_owned_bytes))
    }

    fn publish_construction_sink(
        &self,
        construction: &mut PatriciaIndexConstruction,
        sink: &PackedPatriciaConstructionSink,
        retained_mutation_bytes: usize,
    ) -> Result<usize, PatriciaError> {
        if sink.is_empty() {
            return Ok(construction.staged.owned_bytes());
        }
        let staged_owned_bytes = construction.staged.owned_bytes();
        if self.publisher.permits_construction_packed_head_transition() {
            match self.publish_packed_construction_sink(
                sink,
                staged_owned_bytes,
                retained_mutation_bytes,
                construction.resident_budget_bytes,
            ) {
                Ok(publication) => {
                    let work = publication.work;
                    construction.logical_node_writes =
                        construction.logical_node_writes.saturating_add(sink.len());
                    construction.pack_publication_calls = construction
                        .pack_publication_calls
                        .saturating_add(work.packs_published);
                    let catalog_publications =
                        usize::from(work.catalog_metadata_bytes_published != 0);
                    construction.catalog_publication_calls = construction
                        .catalog_publication_calls
                        .saturating_add(catalog_publications);
                    construction.head_transitions = construction
                        .head_transitions
                        .saturating_add(usize::from(publication.head_transition));
                    construction.durability_barriers = construction
                        .durability_barriers
                        .saturating_add(work.packs_published)
                        .saturating_add(catalog_publications)
                        .saturating_add(usize::from(publication.head_transition));
                    construction.packed_bytes = construction
                        .packed_bytes
                        .saturating_add(work.pack_bytes_published)
                        .saturating_add(work.catalog_metadata_bytes_published)
                        .saturating_add(
                            usize::from(publication.head_transition).saturating_mul(HEAD_BYTES),
                        );
                    construction.peak_resident_bytes = construction
                        .peak_resident_bytes
                        .max(publication.peak_resident_bytes);
                    self.note_writes(sink.entries());
                    return Ok(publication.peak_resident_bytes);
                }
                Err(error) if packed_capacity_error(&error) => {
                    construction.capacity_fallbacks =
                        construction.capacity_fallbacks.saturating_add(1);
                }
                Err(error) => return Err(map_packed_patricia_error(error)),
            }
        }

        let _operation =
            lock_packed_operation_shared(&self.nodes).map_err(map_packed_patricia_error)?;
        let mut peak_owned_bytes = staged_owned_bytes
            .saturating_add(retained_mutation_bytes)
            .saturating_add(sink.owned_bytes());
        for digest in sink.child_before_parent() {
            let bytes = sink.entries().get(digest).ok_or(PatriciaError::Malformed)?;
            peak_owned_bytes = peak_owned_bytes.max(
                staged_owned_bytes
                    .saturating_add(retained_mutation_bytes)
                    .saturating_add(sink.owned_bytes()),
            );
            self.publisher
                .publish(&self.nodes, &node_filename(*digest), bytes)
                .map_err(PatriciaError::Publication)?;
        }
        construction.logical_node_writes =
            construction.logical_node_writes.saturating_add(sink.len());
        construction.loose_publication_calls = construction
            .loose_publication_calls
            .saturating_add(sink.len());
        construction.peak_resident_bytes = construction.peak_resident_bytes.max(peak_owned_bytes);
        self.note_writes(sink.entries());
        Ok(peak_owned_bytes)
    }

    fn publish_staged_roots(
        &self,
        roots: &BTreeSet<ContentDigest>,
        staged: &StagedNodes,
    ) -> Result<usize, PatriciaError> {
        let (entries, publication_order) = self.reachable_staged_bytes(roots, staged)?;
        if entries.is_empty() {
            return Ok(0);
        }
        if self.publisher.permits_packed_head_transition() {
            match self.publish_packed_entries(&entries) {
                Ok(()) => {
                    self.note_writes(&entries);
                    return Ok(entries.len());
                }
                Err(error) if packed_capacity_error(&error) => {}
                Err(error) => return Err(map_packed_patricia_error(error)),
            }
        }
        let _operation =
            lock_packed_operation_shared(&self.nodes).map_err(map_packed_patricia_error)?;
        for digest in publication_order {
            let bytes = entries
                .get(&digest)
                .expect("reachable publication order names an exact staged node");
            self.publisher
                .publish(&self.nodes, &node_filename(digest), bytes)
                .map_err(PatriciaError::Publication)?;
        }
        self.note_writes(&entries);
        Ok(entries.len())
    }

    fn reachable_staged_bytes(
        &self,
        roots: &BTreeSet<ContentDigest>,
        staged: &StagedNodes,
    ) -> Result<(BTreeMap<ContentDigest, Vec<u8>>, Vec<ContentDigest>), PatriciaError> {
        let mut pending = roots.iter().copied().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        let mut entries = BTreeMap::new();
        let mut publication_order = Vec::new();
        while let Some(digest) = pending.pop() {
            if !visited.insert(digest) {
                continue;
            }
            let Some(node) = staged.nodes.get(&digest) else {
                continue;
            };
            if let Node::Branch { left, right, .. } = node {
                pending.push(*left);
                pending.push(*right);
            }
            let bytes = postcard::to_allocvec(node).map_err(|_| PatriciaError::Malformed)?;
            if ContentDigest::of(&bytes) != digest {
                return Err(PatriciaError::PathMismatch(digest));
            }
            entries.insert(digest, bytes);
            publication_order.push(digest);
        }
        Ok((entries, publication_order))
    }

    fn publish_packed_construction_sink(
        &self,
        sink: &PackedPatriciaConstructionSink,
        staged_owned_bytes: usize,
        retained_mutation_bytes: usize,
        resident_budget_bytes: usize,
    ) -> Result<PackedConstructionPublication, PackedPatriciaError> {
        let _operation = lock_packed_operation_shared(&self.nodes)?;
        let retained_bytes = staged_owned_bytes
            .checked_add(retained_mutation_bytes)
            .and_then(|bytes| bytes.checked_add(sink.owned_bytes()))
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let sink_peak_bytes = retained_bytes
            .checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        if sink_peak_bytes > resident_budget_bytes {
            return Err(PackedPatriciaError::PackTooLarge);
        }
        let remaining_resolver_bytes = resident_budget_bytes
            .checked_sub(retained_bytes)
            .ok_or(PackedPatriciaError::PackTooLarge)?;
        let mut resolver = self
            .packed
            .lock()
            .map_err(|_| PackedPatriciaError::Malformed)?;
        let mut discovery_peak_bytes = sink_peak_bytes;
        if resolver.is_none() {
            let (discovered, resolver_peak_bytes) =
                PackedPatriciaCatalog::discover_under_guard_bounded(
                    &self.nodes,
                    remaining_resolver_bytes,
                )?;
            discovery_peak_bytes = retained_bytes
                .checked_add(resolver_peak_bytes)
                .ok_or(PackedPatriciaError::PackTooLarge)?;
            *resolver = Some(discovered);
        }
        let current = resolver.as_ref().expect("packed discovery initialized");

        let expected = current.as_ref().map(PackedPatriciaCatalog::authority);
        #[cfg(test)]
        let catalog_pack_byte_limit = self.packed_catalog_byte_limit;
        #[cfg(not(test))]
        let catalog_pack_byte_limit = MAX_CATALOG_PACK_BYTES;
        let construction_publisher = ConstructionExactPublisher(self.publisher.as_ref());
        let (pending, mut work) = publish_appended_catalog_bounded_streaming(
            &self.nodes,
            &construction_publisher,
            current.as_ref(),
            sink.entries(),
            catalog_pack_byte_limit,
            PackedPatriciaResidencyBudget {
                retained_bytes: staged_owned_bytes
                    .checked_add(retained_mutation_bytes)
                    .and_then(|bytes| bytes.checked_add(sink.owned_bytes()))
                    .ok_or(PackedPatriciaError::PackTooLarge)?,
                maximum_bytes: resident_budget_bytes,
            },
        )?;
        work.peak_resident_bytes = work.peak_resident_bytes.max(discovery_peak_bytes);
        #[cfg(test)]
        {
            *self
                .packed_publication_work
                .lock()
                .map_err(|_| PackedPatriciaError::Malformed)? = Some(work);
        }
        let Some(pending) = pending else {
            return Ok(PackedConstructionPublication {
                work,
                head_transition: false,
                peak_resident_bytes: work.peak_resident_bytes,
            });
        };
        let transition =
            transition_catalog_head(&self.nodes, expected, pending.published_catalog());
        *resolver = None;
        transition?;
        let completed_peak = staged_owned_bytes
            .checked_add(retained_mutation_bytes)
            .and_then(|bytes| bytes.checked_add(sink.owned_bytes()))
            .ok_or(PackedPatriciaError::PackTooLarge)?
            .max(work.peak_resident_bytes);
        if completed_peak > resident_budget_bytes || completed_peak > work.peak_resident_bytes {
            return Err(PackedPatriciaError::Malformed);
        }
        Ok(PackedConstructionPublication {
            work,
            head_transition: true,
            peak_resident_bytes: completed_peak,
        })
    }

    fn publish_packed_entries(
        &self,
        new_entries: &BTreeMap<ContentDigest, Vec<u8>>,
    ) -> Result<(), PackedPatriciaError> {
        let _operation = lock_packed_operation_shared(&self.nodes)?;
        let mut resolver = self
            .packed
            .lock()
            .map_err(|_| PackedPatriciaError::Malformed)?;
        if resolver.is_none() {
            *resolver = Some(PackedPatriciaCatalog::discover_under_guard(&self.nodes)?);
        }
        let current = resolver.as_ref().expect("packed discovery initialized");
        let expected = current.as_ref().map(PackedPatriciaCatalog::authority);
        #[cfg(test)]
        let catalog_pack_byte_limit = self.packed_catalog_byte_limit;
        #[cfg(not(test))]
        let catalog_pack_byte_limit = MAX_CATALOG_PACK_BYTES;
        let (pending, work) = publish_appended_catalog(
            &self.nodes,
            self.publisher.as_ref(),
            current.as_ref(),
            new_entries,
            catalog_pack_byte_limit,
        )?;
        #[cfg(test)]
        {
            *self
                .packed_publication_work
                .lock()
                .map_err(|_| PackedPatriciaError::Malformed)? = Some(work);
        }
        #[cfg(not(test))]
        let _ = work;
        let Some(pending) = pending else {
            return Ok(());
        };
        transition_catalog_head(&self.nodes, expected, pending.published_catalog())?;
        let current = resolver
            .take()
            .expect("packed discovery initialized before transition");
        *resolver = Some(Some(pending.finish(current)));
        Ok(())
    }

    fn note_writes(&self, entries: &BTreeMap<ContentDigest, Vec<u8>>) {
        self.counters
            .writes
            .fetch_add(entries.len(), Ordering::Relaxed);
        self.counters.bytes_written.fetch_add(
            entries.values().map(Vec::len).sum::<usize>(),
            Ordering::Relaxed,
        );
    }

    fn verify_staged_reachable(
        &self,
        root: PatriciaIndexRoot,
        staged: &StagedNodes,
    ) -> Result<(), PatriciaError> {
        let mut pending = vec![root.digest()];
        let mut visited = BTreeSet::new();
        while let Some(digest) = pending.pop() {
            if !visited.insert(digest) {
                continue;
            }
            let Some(node) = staged.nodes.get(&digest) else {
                continue;
            };
            if let Node::Branch { left, right, .. } = node {
                pending.push(*left);
                pending.push(*right);
            }
            self.read_node(digest)?;
        }
        Ok(())
    }

    fn collect_prefix(
        &self,
        root: ContentDigest,
        requested: &[u8],
        limit: usize,
        found: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<(), PatriciaError> {
        let budget = traversal_node_budget(MAX_KEY_BYTES)?;
        let mut pending = vec![(root, None, budget)];
        while let Some((digest, constraint, remaining_nodes)) = pending.pop() {
            let remaining_nodes = consume_node_budget(remaining_nodes)?;
            let node = self.read_node(digest)?;
            validate_node_path(&node, constraint.as_ref())?;
            match node {
                Node::Leaf { key, value, .. } => {
                    if key.starts_with(requested) {
                        found.insert(key, value);
                        if found.len() == limit {
                            return Ok(());
                        }
                    }
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    let requested_bits = key_bit_len(requested)?;
                    let compared = split.min(requested_bits);
                    if !prefix_matches(requested, &prefix, compared)? {
                        continue;
                    }
                    if requested_bits <= split {
                        pending.push((
                            right,
                            Some(ChildPathConstraint {
                                parent_prefix: prefix.clone(),
                                parent_prefix_bit_len: split,
                                right: true,
                            }),
                            remaining_nodes,
                        ));
                        pending.push((
                            left,
                            Some(ChildPathConstraint {
                                parent_prefix: prefix,
                                parent_prefix_bit_len: split,
                                right: false,
                            }),
                            remaining_nodes,
                        ));
                    } else {
                        let rightward = key_bit(requested, split)?;
                        pending.push((
                            if rightward { right } else { left },
                            Some(ChildPathConstraint {
                                parent_prefix: prefix,
                                parent_prefix_bit_len: split,
                                right: rightward,
                            }),
                            remaining_nodes,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn publish_node(&self, node: &Node) -> Result<ContentDigest, PatriciaError> {
        validate_node(node)?;
        let bytes = postcard::to_allocvec(node).map_err(|_| PatriciaError::Malformed)?;
        if bytes.len() as u64 > MAX_NODE_BYTES {
            return Err(PatriciaError::Malformed);
        }
        let digest = ContentDigest::of(&bytes);
        let filename = node_filename(digest);
        self.publisher
            .publish(&self.nodes, &filename, &bytes)
            .map_err(PatriciaError::Publication)?;
        self.counters.writes.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_written
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(digest)
    }

    fn read_node(&self, digest: ContentDigest) -> Result<Node, PatriciaError> {
        let loose =
            read_optional_regular(&self.nodes, &node_filename(digest), MAX_NODE_BYTES, None)?;
        if loose
            .as_ref()
            .is_some_and(|bytes| ContentDigest::of(bytes) != digest)
        {
            return Err(PatriciaError::PathMismatch(digest));
        }
        let packed = self.read_packed_node(digest)?;
        let bytes = match (loose, packed) {
            (Some(loose), Some(packed)) if loose != packed => {
                return Err(PatriciaError::PathMismatch(digest));
            }
            (Some(loose), _) => loose,
            (None, Some(packed)) => packed,
            (None, None) => return Err(PatriciaError::MissingNode(digest)),
        };
        let node: Node = postcard::from_bytes(&bytes).map_err(|_| PatriciaError::Malformed)?;
        validate_node(&node)?;
        if postcard::to_allocvec(&node).map_err(|_| PatriciaError::Malformed)? != bytes {
            return Err(PatriciaError::Malformed);
        }
        self.counters.reads.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_read
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(node)
    }

    fn read_packed_node(&self, digest: ContentDigest) -> Result<Option<Vec<u8>>, PatriciaError> {
        let mut resolver = self.packed.lock().map_err(|_| PatriciaError::Malformed)?;
        if resolver.is_none() {
            *resolver = Some(
                PackedPatriciaCatalog::discover(&self.nodes).map_err(map_packed_patricia_error)?,
            );
        }
        Ok(resolver
            .as_ref()
            .expect("packed discovery initialized")
            .as_ref()
            .and_then(|catalog| catalog.get(digest).map(<[u8]>::to_vec)))
    }
}

fn map_packed_patricia_error(error: PackedPatriciaError) -> PatriciaError {
    match error {
        PackedPatriciaError::Filesystem(error) => PatriciaError::Filesystem(error),
        PackedPatriciaError::Publication(error) => PatriciaError::Publication(error),
        PackedPatriciaError::PathMismatch(digest) => PatriciaError::PathMismatch(digest),
        PackedPatriciaError::Empty
        | PackedPatriciaError::TooManyEntries
        | PackedPatriciaError::EntryTooLarge(_)
        | PackedPatriciaError::PackTooLarge
        | PackedPatriciaError::UnexpectedHead
        | PackedPatriciaError::Malformed => PatriciaError::Malformed,
    }
}

fn packed_capacity_error(error: &PackedPatriciaError) -> bool {
    match error {
        PackedPatriciaError::TooManyEntries | PackedPatriciaError::PackTooLarge => true,
        PackedPatriciaError::EntryTooLarge(digest) => {
            let _ = digest;
            true
        }
        _ => false,
    }
}

fn bulk_partition(records: &[BulkRecord<'_>], split: usize) -> Result<usize, PatriciaError> {
    let mut partition = 0;
    while partition < records.len() && !key_bit(records[partition].key, split)? {
        partition += 1;
    }
    for record in &records[partition..] {
        if !key_bit(record.key, split)? {
            return Err(PatriciaError::Malformed);
        }
    }
    Ok(partition)
}

fn validate_record(key: &[u8], value: &[u8]) -> Result<(), PatriciaError> {
    validate_key(key)?;
    if value.is_empty() || value.len() > MAX_VALUE_BYTES {
        return Err(PatriciaError::Malformed);
    }
    Ok(())
}

fn validate_key(key: &[u8]) -> Result<(), PatriciaError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(PatriciaError::Malformed);
    }
    key_bit_len(key)?;
    Ok(())
}

fn construction_mutation_reservation(
    key_bytes: usize,
    value_bytes: usize,
    inserts_leaf: bool,
) -> Result<usize, PatriciaError> {
    let path_nodes = traversal_node_budget(key_bytes)?;
    let rebuilt_entries = path_nodes
        .checked_add(usize::from(inserts_leaf))
        .ok_or(PatriciaError::Malformed)?;
    let branch_entry = CONSTRUCTION_ENTRY_OWNERSHIP_BYTES
        .checked_add(CONSTRUCTION_BRANCH_PAYLOAD_BOUND)
        .ok_or(PatriciaError::Malformed)?;
    let retained_branches = rebuilt_entries
        .checked_mul(branch_entry)
        .ok_or(PatriciaError::Malformed)?;
    let leaf = if inserts_leaf {
        CONSTRUCTION_ENTRY_OWNERSHIP_BYTES
            .checked_add(key_bytes)
            .and_then(|bytes| bytes.checked_add(value_bytes))
            .ok_or(PatriciaError::Malformed)?
    } else {
        0
    };
    let retained = retained_branches
        .checked_add(leaf)
        .ok_or(PatriciaError::Malformed)?;
    let traversal = path_nodes
        .checked_mul(CONSTRUCTION_TRAVERSAL_FRAME_BYTES)
        .ok_or(PatriciaError::Malformed)?;
    retained
        .checked_add(traversal)
        .and_then(|bytes| bytes.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES))
        .ok_or(PatriciaError::Malformed)
}

fn construction_publication_reservation() -> Result<usize, PatriciaError> {
    traversal_node_budget(MAX_KEY_BYTES)?
        .checked_add(1)
        .and_then(|frames| frames.checked_mul(CONSTRUCTION_TRAVERSAL_FRAME_BYTES))
        .and_then(|bytes| bytes.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES))
        .ok_or(PatriciaError::Malformed)
}

fn construction_bulk_reservation(record_refs_bytes: usize) -> Result<usize, PatriciaError> {
    let traversal = traversal_node_budget(MAX_KEY_BYTES)?
        .checked_mul(CONSTRUCTION_TRAVERSAL_FRAME_BYTES)
        .ok_or(PatriciaError::Malformed)?;
    let publication = construction_publication_reservation()?;
    record_refs_bytes
        .checked_add(traversal)
        .and_then(|bytes| bytes.checked_add(publication))
        .and_then(|bytes| bytes.checked_add(CONSTRUCTION_ENCODING_SCRATCH_BYTES))
        .ok_or(PatriciaError::Malformed)
}

fn validate_node(node: &Node) -> Result<(), PatriciaError> {
    match node {
        Node::Leaf {
            schema_version,
            key,
            value,
        } => {
            if *schema_version != NODE_SCHEMA_VERSION {
                return Err(PatriciaError::Malformed);
            }
            validate_record(key, value)
        }
        Node::Branch {
            schema_version,
            prefix,
            prefix_bit_len,
            left,
            right,
        } => {
            let bits = *prefix_bit_len as usize;
            if *schema_version != NODE_SCHEMA_VERSION
                || bits >= MAX_KEY_BITS
                || prefix.len() != bits.div_ceil(8)
                || masked_prefix(prefix, bits) != *prefix
                || left == right
                || *left == PatriciaIndexRoot::empty().digest()
                || *right == PatriciaIndexRoot::empty().digest()
            {
                return Err(PatriciaError::Malformed);
            }
            Ok(())
        }
    }
}

fn validate_node_path(
    node: &Node,
    constraint: Option<&ChildPathConstraint>,
) -> Result<(), PatriciaError> {
    let Some(constraint) = constraint else {
        return Ok(());
    };
    let prefix = node_prefix(node);
    let bits = node_prefix_bits(node)?;
    if bits <= constraint.parent_prefix_bit_len
        || !prefix_matches(
            prefix,
            &constraint.parent_prefix,
            constraint.parent_prefix_bit_len,
        )?
        || key_bit(prefix, constraint.parent_prefix_bit_len)? != constraint.right
    {
        return Err(PatriciaError::Malformed);
    }
    Ok(())
}

fn node_prefix(node: &Node) -> &[u8] {
    match node {
        Node::Leaf { key, .. } => key,
        Node::Branch { prefix, .. } => prefix,
    }
}

fn node_prefix_bits(node: &Node) -> Result<usize, PatriciaError> {
    match node {
        Node::Leaf { key, .. } => key_bit_len(key),
        Node::Branch { prefix_bit_len, .. } => Ok(*prefix_bit_len as usize),
    }
}

fn common_prefix_bits(left: &[u8], right: &[u8], limit: usize) -> Result<usize, PatriciaError> {
    let limit = limit.min(key_bit_len(left)?).min(key_bit_len(right)?);
    Ok((0..limit)
        .find(|bit| key_bit_unchecked(left, *bit) != key_bit_unchecked(right, *bit))
        .unwrap_or(limit))
}

fn prefix_matches(key: &[u8], prefix: &[u8], bits: usize) -> Result<bool, PatriciaError> {
    Ok(key_bit_len(key)? >= bits
        && key_bit_len(prefix)? >= bits
        && common_prefix_bits(key, prefix, bits)? == bits)
}

fn key_bit(key: &[u8], bit: usize) -> Result<bool, PatriciaError> {
    if bit >= key_bit_len(key)? {
        return Err(PatriciaError::Malformed);
    }
    Ok(key_bit_unchecked(key, bit))
}

fn key_bit_len(key: &[u8]) -> Result<usize, PatriciaError> {
    key.len().checked_mul(8).ok_or(PatriciaError::Malformed)
}

fn traversal_node_budget(key_bytes: usize) -> Result<usize, PatriciaError> {
    key_bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_add(1))
        .ok_or(PatriciaError::Malformed)
}

fn consume_node_budget(remaining_nodes: usize) -> Result<usize, PatriciaError> {
    remaining_nodes
        .checked_sub(1)
        .ok_or(PatriciaError::Malformed)
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

fn node_filename(digest: ContentDigest) -> String {
    format!("{digest}{NODE_SUFFIX}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    use cap_std::ambient_authority;
    use uuid::Uuid;

    use super::*;
    use crate::{ensure_directory_nofollow, open_dir_nofollow, publish_immutable_exact};

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

    struct PackedExactPublisher;

    impl PatriciaNodePublisher for PackedExactPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }

        fn permits_packed_head_transition(&self) -> bool {
            true
        }
    }

    #[derive(Clone, Default)]
    struct ConstructionPublicationRecorder {
        final_publication_calls: Arc<AtomicUsize>,
    }

    impl PatriciaNodePublisher for ConstructionPublicationRecorder {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }

        fn permits_packed_head_transition(&self) -> bool {
            true
        }

        fn publish_construction_exact(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            self.final_publication_calls.fetch_add(1, Ordering::Relaxed);
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }

        fn publish_staged_construction_exact(
            &self,
            publication: StagedExactImmutablePublication,
        ) -> Result<(), PatriciaPublicationError> {
            self.final_publication_calls.fetch_add(1, Ordering::Relaxed);
            publication.commit().map_err(PatriciaPublicationError::new)
        }

        fn permits_construction_packed_head_transition(&self) -> bool {
            true
        }
    }

    #[derive(Clone)]
    struct PausingPackedPublisher {
        armed: Arc<AtomicBool>,
        catalog_published: Arc<Barrier>,
        release: Arc<Barrier>,
    }

    impl PatriciaNodePublisher for PausingPackedPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)?;
            if filename.ends_with(".patricia-catalog-v1")
                && self.armed.swap(false, Ordering::SeqCst)
            {
                self.catalog_published.wait();
                self.release.wait();
            }
            Ok(())
        }

        fn permits_packed_head_transition(&self) -> bool {
            true
        }
    }

    #[derive(Clone, Default)]
    struct MeasuringPackedPublisher {
        publications: Arc<Mutex<Vec<(String, usize)>>>,
    }

    impl MeasuringPackedPublisher {
        fn take(&self) -> Vec<(String, usize)> {
            std::mem::take(&mut *self.publications.lock().unwrap())
        }
    }

    impl PatriciaNodePublisher for MeasuringPackedPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            self.publications
                .lock()
                .unwrap()
                .push((filename.to_owned(), bytes.len()));
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }

        fn permits_packed_head_transition(&self) -> bool {
            true
        }
    }

    #[derive(Clone)]
    struct BoundaryCrashPublisher {
        calls: Arc<AtomicUsize>,
        fail_at: usize,
        after_commit: bool,
    }

    impl PatriciaNodePublisher for BoundaryCrashPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call == self.fail_at && !self.after_commit {
                return Err(PatriciaPublicationError::new("injected pre-commit crash"));
            }
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)?;
            if call == self.fail_at && self.after_commit {
                return Err(PatriciaPublicationError::new("injected post-commit crash"));
            }
            Ok(())
        }

        fn permits_packed_head_transition(&self) -> bool {
            true
        }

        fn permits_construction_packed_head_transition(&self) -> bool {
            false
        }
    }

    #[derive(Clone)]
    struct ConstructionCrashPublisher {
        calls: Arc<AtomicUsize>,
        fail_at: usize,
        after_commit: bool,
    }

    impl PatriciaNodePublisher for ConstructionCrashPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }

        fn publish_construction_exact(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call == self.fail_at && !self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected construction prerequisite failure",
                ));
            }
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)?;
            if call == self.fail_at && self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected post-publication construction prerequisite failure",
                ));
            }
            Ok(())
        }

        fn publish_staged_construction_exact(
            &self,
            publication: StagedExactImmutablePublication,
        ) -> Result<(), PatriciaPublicationError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call == self.fail_at && !self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected staged construction prerequisite failure",
                ));
            }
            publication
                .commit()
                .map_err(PatriciaPublicationError::new)?;
            if call == self.fail_at && self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected post-publication staged construction prerequisite failure",
                ));
            }
            Ok(())
        }

        fn permits_construction_packed_head_transition(&self) -> bool {
            true
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum BulkPublicationBoundary {
        Leaf,
        ChildBranch,
        ParentBranch,
    }

    #[derive(Clone)]
    struct BulkBoundaryCrashPublisher {
        armed: Arc<AtomicBool>,
        boundary: BulkPublicationBoundary,
        after_commit: bool,
    }

    impl PatriciaNodePublisher for BulkBoundaryCrashPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            let node: Node = postcard::from_bytes(bytes)
                .map_err(|_| PatriciaPublicationError::new("test publisher received bad node"))?;
            let at_boundary = match (&self.boundary, node) {
                (BulkPublicationBoundary::Leaf, Node::Leaf { ref key, .. }) => {
                    matches!(key.as_slice(), [0x00] | [0x40] | [0x80] | [0xc0])
                }
                (
                    BulkPublicationBoundary::ChildBranch,
                    Node::Branch {
                        prefix_bit_len: 1, ..
                    },
                ) => true,
                (
                    BulkPublicationBoundary::ParentBranch,
                    Node::Branch {
                        prefix_bit_len: 0, ..
                    },
                ) => true,
                _ => false,
            };
            let fail = at_boundary && self.armed.swap(false, Ordering::Relaxed);
            if fail && !self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected bulk pre-commit crash",
                ));
            }
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)?;
            if fail && self.after_commit {
                return Err(PatriciaPublicationError::new(
                    "injected bulk post-commit crash",
                ));
            }
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct RecordingPublisher {
        publications: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    }

    impl RecordingPublisher {
        fn take(&self) -> Vec<(String, String)> {
            std::mem::take(&mut *self.publications.lock().unwrap())
                .into_iter()
                .map(|(filename, bytes)| (filename, hex_bytes(&bytes)))
                .collect()
        }
    }

    impl PatriciaNodePublisher for RecordingPublisher {
        fn publish(
            &self,
            dir: &Dir,
            filename: &str,
            bytes: &[u8],
        ) -> Result<(), PatriciaPublicationError> {
            self.publications
                .lock()
                .unwrap()
                .push((filename.to_owned(), bytes.to_vec()));
            publish_immutable_exact(dir, filename, bytes).map_err(PatriciaPublicationError::new)
        }
    }

    fn store(name: &str) -> (std::path::PathBuf, PatriciaIndexStore) {
        let path = std::env::temp_dir().join(format!("tine-claim-index-{name}-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let root = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        ensure_directory_nofollow(&root, "nodes").unwrap();
        let nodes = open_dir_nofollow(&root, "nodes").unwrap();
        (path, PatriciaIndexStore::new(nodes, ExactPublisher))
    }

    fn store_with_publisher(
        name: &str,
        publisher: impl PatriciaNodePublisher + 'static,
    ) -> (std::path::PathBuf, PatriciaIndexStore) {
        let path = std::env::temp_dir().join(format!("tine-claim-index-{name}-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let root = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        ensure_directory_nofollow(&root, "nodes").unwrap();
        let nodes = open_dir_nofollow(&root, "nodes").unwrap();
        (path, PatriciaIndexStore::new(nodes, publisher))
    }

    fn count_suffix(path: &std::path::Path, suffix: &str) -> usize {
        fs::read_dir(path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(suffix))
            .count()
    }

    fn regular_file_inventory(path: &std::path::Path) -> BTreeMap<String, u64> {
        fs::read_dir(path)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then(|| {
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        metadata.len(),
                    )
                })
            })
            .collect()
    }

    fn publish_leaf(store: &PatriciaIndexStore, key: &[u8]) -> ContentDigest {
        store
            .publish_node(&Node::Leaf {
                schema_version: NODE_SCHEMA_VERSION,
                key: key.to_vec(),
                value: b"value".to_vec(),
            })
            .unwrap()
    }

    fn publish_branch(
        store: &PatriciaIndexStore,
        prefix_source: &[u8],
        split: usize,
        left: ContentDigest,
        right: ContentDigest,
    ) -> ContentDigest {
        store
            .publish_node(&Node::Branch {
                schema_version: NODE_SCHEMA_VERSION,
                prefix: masked_prefix(prefix_source, split),
                prefix_bit_len: u16::try_from(split).unwrap(),
                left,
                right,
            })
            .unwrap()
    }

    fn assert_point_traversals_reject(
        store: &PatriciaIndexStore,
        root: PatriciaIndexRoot,
        key: &[u8],
    ) {
        assert!(matches!(
            store.lookup(root, key),
            Err(PatriciaError::Malformed)
        ));
        assert!(matches!(
            store.insert_many(
                root,
                &BTreeMap::from([(key.to_vec(), b"replacement".to_vec())])
            ),
            Err(PatriciaError::Malformed)
        ));
        assert!(matches!(
            store.lookup_prefix(root, key),
            Err(PatriciaError::Malformed)
        ));
    }

    fn all_records(
        store: &PatriciaIndexStore,
        root: PatriciaIndexRoot,
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let mut records = BTreeMap::new();
        store
            .visit_all(root, |key, value| {
                records.insert(key.to_vec(), value.to_vec());
                true
            })
            .unwrap();
        records
    }

    fn all_construction_records(
        store: &PatriciaIndexStore,
        construction: &PatriciaIndexConstruction,
        root: PatriciaIndexRoot,
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let mut records = BTreeMap::new();
        store
            .construction_visit_all(construction, root, |key, value| {
                records.insert(key.to_vec(), value.to_vec());
                true
            })
            .unwrap();
        records
    }

    fn reachable_node_bytes(
        store: &PatriciaIndexStore,
        roots: impl IntoIterator<Item = PatriciaIndexRoot>,
    ) -> BTreeMap<ContentDigest, Vec<u8>> {
        let mut pending = roots
            .into_iter()
            .filter(|root| *root != PatriciaIndexRoot::empty())
            .map(PatriciaIndexRoot::digest)
            .collect::<Vec<_>>();
        let mut bytes = BTreeMap::new();
        while let Some(digest) = pending.pop() {
            if bytes.contains_key(&digest) {
                continue;
            }
            let node = store.read_node(digest).unwrap();
            if let Node::Branch { left, right, .. } = &node {
                pending.push(*left);
                pending.push(*right);
            }
            bytes.insert(digest, postcard::to_allocvec(&node).unwrap());
        }
        bytes
    }

    fn packed_records(
        pack: &crate::packed_patricia::PackedPatriciaPack,
        root: PatriciaIndexRoot,
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let mut pending = vec![(root.digest(), None)];
        let mut visited = BTreeSet::new();
        let mut records = BTreeMap::new();
        while let Some((digest, constraint)) = pending.pop() {
            assert!(
                visited.insert(digest),
                "fixture must be an acyclic Patricia graph"
            );
            let bytes = pack
                .get(digest)
                .expect("pack must contain every reachable node");
            let node: Node = postcard::from_bytes(bytes).unwrap();
            validate_node(&node).unwrap();
            validate_node_path(&node, constraint.as_ref()).unwrap();
            match node {
                Node::Leaf { key, value, .. } => {
                    records.insert(key, value);
                }
                Node::Branch {
                    prefix,
                    prefix_bit_len,
                    left,
                    right,
                    ..
                } => {
                    let split = prefix_bit_len as usize;
                    pending.push((
                        left,
                        Some(ChildPathConstraint {
                            parent_prefix: prefix.clone(),
                            parent_prefix_bit_len: split,
                            right: false,
                        }),
                    ));
                    pending.push((
                        right,
                        Some(ChildPathConstraint {
                            parent_prefix: prefix,
                            parent_prefix_bit_len: split,
                            right: true,
                        }),
                    ));
                }
            }
        }
        records
    }

    fn publish_cataloged_nodes(
        dir: &Dir,
        nodes: &BTreeMap<ContentDigest, Vec<u8>>,
        entries_per_pack: usize,
    ) {
        let mut packs = Vec::new();
        for chunk in nodes.iter().collect::<Vec<_>>().chunks(entries_per_pack) {
            let entries = chunk
                .iter()
                .map(|(digest, bytes)| (**digest, (*bytes).clone()))
                .collect();
            let publication =
                crate::packed_patricia::PackedPatriciaPublication::build(&entries).unwrap();
            packs.push(publication.publish(dir, &ExactPublisher).unwrap());
        }
        let catalog =
            crate::packed_patricia::PackedPatriciaCatalogPublication::build(&packs).unwrap();
        let catalog = catalog.publish(dir, &ExactPublisher).unwrap();
        crate::packed_patricia::publish_catalog_head(dir, &catalog, &ExactPublisher).unwrap();
    }

    #[test]
    fn packed_primitive_reopens_a_real_patricia_history_semantically() {
        const RECORDS: usize = 256;

        let (loose_path, loose) = store("packed-semantic-loose");
        let expected = (0..RECORDS)
            .map(|index| {
                (
                    format!("pages/Unicode-α-{index:04}.md").into_bytes(),
                    format!("值-{index:04}").into_bytes(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let root = loose
            .insert_many(PatriciaIndexRoot::empty(), &expected)
            .unwrap();
        let reachable = reachable_node_bytes(&loose, [root]);
        assert_eq!(
            reachable.len(),
            RECORDS * 2 - 1,
            "a full binary Patricia tree has one leaf per record and one fewer branch"
        );

        let pack_path =
            std::env::temp_dir().join(format!("tine-patricia-semantic-pack-{}", Uuid::new_v4()));
        fs::create_dir(&pack_path).unwrap();
        let pack_dir = Dir::open_ambient_dir(&pack_path, ambient_authority()).unwrap();
        let publication =
            crate::packed_patricia::PackedPatriciaPublication::build(&reachable).unwrap();
        let completed = publication.publish(&pack_dir, &ExactPublisher).unwrap();
        let reopened =
            crate::packed_patricia::PackedPatriciaPack::open(&pack_dir, completed.digest())
                .unwrap();

        assert_eq!(packed_records(&reopened, root), expected);
        assert_eq!(reopened.len(), reachable.len());
        assert_eq!(fs::read_dir(&pack_path).unwrap().count(), 1);
        assert_eq!(
            count_suffix(&loose_path.join("nodes"), NODE_SUFFIX),
            reachable.len()
        );

        drop(reopened);
        drop(pack_dir);
        drop(loose);
        fs::remove_dir_all(loose_path).unwrap();
        fs::remove_dir_all(pack_path).unwrap();
    }

    #[test]
    fn packed_reclamation_contracts_long_history_to_exact_live_authority() {
        const VERSIONS: usize = 24;

        let (path, store) =
            store_with_publisher("packed-reclamation-history", PackedExactPublisher);
        let nodes_path = path.join("nodes");
        let mut root = PatriciaIndexRoot::empty();
        let mut roots = Vec::new();
        for version in 0..VERSIONS {
            let key = format!("pages/history-{version:03}").into_bytes();
            let value = format!("value-{version:03}").into_bytes();
            root = store
                .insert_many(root, &BTreeMap::from([(key.clone(), value.clone())]))
                .unwrap();
            roots.push((root, key, value));
        }
        for (historical_root, key, value) in &roots {
            assert_eq!(
                store.lookup(*historical_root, key).unwrap(),
                Some(value.clone())
            );
        }

        let nodes = Dir::open_ambient_dir(&nodes_path, ambient_authority()).unwrap();
        let active = crate::packed_patricia::PackedPatriciaCatalog::discover(&nodes)
            .unwrap()
            .unwrap()
            .live_filenames();
        let before = regular_file_inventory(&nodes_path);
        let physical_bytes = before.values().copied().sum::<u64>();
        let active_bytes = active
            .iter()
            .map(|name| before.get(name).copied().unwrap())
            .sum::<u64>();
        assert!(physical_bytes > active_bytes);
        assert!(before.keys().any(|name| !active.contains(name)));

        let report = store.reclaim_unreachable_packed_files().unwrap();
        let after = regular_file_inventory(&nodes_path);
        assert_eq!(after.keys().cloned().collect::<BTreeSet<_>>(), active);
        assert_eq!(report.examined_files, before.len());
        assert_eq!(report.examined_bytes, physical_bytes);
        assert_eq!(report.deleted_files, before.len() - after.len());
        assert_eq!(
            report.deleted_bytes,
            physical_bytes - after.values().copied().sum::<u64>()
        );
        assert_eq!(report.retained_files, after.len());
        assert_eq!(report.retained_bytes, after.values().copied().sum::<u64>());
        for (historical_root, key, value) in &roots {
            assert_eq!(
                store.lookup(*historical_root, key).unwrap(),
                Some(value.clone())
            );
        }

        drop(nodes);
        drop(store);
        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let reopened = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        for (historical_root, key, value) in &roots {
            assert_eq!(
                reopened.lookup(*historical_root, key).unwrap(),
                Some(value.clone())
            );
        }
        drop(reopened);
        drop(root_dir);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn packed_reclamation_preserves_unknown_and_loose_files() {
        let (path, store) =
            store_with_publisher("packed-reclamation-preserve", PackedExactPublisher);
        let nodes_path = path.join("nodes");
        let first = store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"first".to_vec(), b"one".to_vec())]),
            )
            .unwrap();
        let current = store
            .insert_many(
                first,
                &BTreeMap::from([(b"second".to_vec(), b"two".to_vec())]),
            )
            .unwrap();
        let loose_bytes = b"preserved loose bytes";
        let loose_name = format!("{}.patricia-node", ContentDigest::of(loose_bytes));
        fs::write(nodes_path.join(&loose_name), loose_bytes).unwrap();
        fs::write(nodes_path.join("unknown-owner-file"), b"unknown").unwrap();
        let uppercase_pack =
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.patricia-pack-v1";
        fs::write(
            nodes_path.join(uppercase_pack),
            b"not a recognized lower-case digest name",
        )
        .unwrap();
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        fs::write(nodes_path.join(&temp_name), b"stale temp").unwrap();

        let report = store.reclaim_unreachable_packed_files().unwrap();
        assert!(report.deleted_files >= 1);
        assert!(!nodes_path.join(temp_name).exists());
        assert_eq!(fs::read(nodes_path.join(loose_name)).unwrap(), loose_bytes);
        assert_eq!(
            fs::read(nodes_path.join("unknown-owner-file")).unwrap(),
            b"unknown"
        );
        assert!(nodes_path.join(uppercase_pack).exists());
        assert_eq!(
            store.lookup(current, b"second").unwrap(),
            Some(b"two".to_vec())
        );
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn malformed_packed_authority_scans_and_deletes_nothing() {
        let (path, store) =
            store_with_publisher("packed-reclamation-malformed", PackedExactPublisher);
        let nodes_path = path.join("nodes");
        store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"authority".to_vec(), b"valid".to_vec())]),
            )
            .unwrap();
        drop(store);
        let orphan_bytes = b"recognized orphan bytes";
        let orphan_name = format!("{}.patricia-pack-v1", ContentDigest::of(orphan_bytes));
        fs::write(nodes_path.join(orphan_name), orphan_bytes).unwrap();
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        fs::write(nodes_path.join(temp_name), b"stale temp").unwrap();
        fs::write(
            nodes_path.join(crate::packed_patricia::HEAD_FILENAME),
            b"malformed authority",
        )
        .unwrap();
        let before = regular_file_inventory(&nodes_path);

        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        let scans_before = crate::packed_patricia::reclamation_directory_scans();
        assert!(matches!(
            store.reclaim_unreachable_packed_files(),
            Err(PatriciaIndexReclamationError::Filesystem(_)
                | PatriciaIndexReclamationError::MalformedAuthority
                | PatriciaIndexReclamationError::PathMismatch(_))
        ));
        assert_eq!(
            crate::packed_patricia::reclamation_directory_scans(),
            scans_before
        );
        assert_eq!(regular_file_inventory(&nodes_path), before);
        drop(store);
        drop(root_dir);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn absent_packed_authority_scans_and_deletes_nothing() {
        let (path, store) = store_with_publisher("packed-reclamation-absent", PackedExactPublisher);
        let nodes_path = path.join("nodes");
        store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"authority".to_vec(), b"valid".to_vec())]),
            )
            .unwrap();
        drop(store);
        fs::remove_file(nodes_path.join(crate::packed_patricia::HEAD_FILENAME)).unwrap();
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        fs::write(nodes_path.join(&temp_name), b"stale temp").unwrap();
        let before = regular_file_inventory(&nodes_path);
        assert!(before
            .keys()
            .any(|name| name.ends_with(".patricia-pack-v1")));
        assert!(before
            .keys()
            .any(|name| name.ends_with(".patricia-catalog-v1")));
        assert!(before.contains_key(&temp_name));

        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        let scans_before = crate::packed_patricia::reclamation_directory_scans();
        assert!(matches!(
            store.reclaim_unreachable_packed_files(),
            Err(PatriciaIndexReclamationError::MalformedAuthority)
        ));
        assert_eq!(
            crate::packed_patricia::reclamation_directory_scans(),
            scans_before
        );
        assert_eq!(regular_file_inventory(&nodes_path), before);
        drop(store);
        drop(root_dir);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn reader_holding_old_head_observation_blocks_nonblocking_reclamation() {
        let (path, store) = store_with_publisher("packed-reclamation-reader", PackedExactPublisher);
        store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"reader".to_vec(), b"value".to_vec())]),
            )
            .unwrap();
        let nodes_path = path.join("nodes");
        let (observed_tx, observed_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let reader_path = nodes_path.clone();
        let reader = std::thread::spawn(move || {
            let nodes = Dir::open_ambient_dir(reader_path, ambient_authority()).unwrap();
            let _guard = crate::packed_patricia::lock_packed_operation_shared(&nodes).unwrap();
            let head =
                read_optional_regular(&nodes, crate::packed_patricia::HEAD_FILENAME, 128, None)
                    .unwrap()
                    .unwrap();
            observed_tx.send(head).unwrap();
            release_rx.recv().unwrap();
            crate::packed_patricia::PackedPatriciaCatalog::discover_under_guard(&nodes)
                .unwrap()
                .unwrap();
        });
        assert!(!observed_rx.recv().unwrap().is_empty());

        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let independent = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        assert!(matches!(
            independent.reclaim_unreachable_packed_files(),
            Err(PatriciaIndexReclamationError::Busy)
        ));
        release_tx.send(()).unwrap();
        reader.join().unwrap();
        independent.reclaim_unreachable_packed_files().unwrap();
        drop(independent);
        drop(root_dir);
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn publication_prerequisites_remain_protected_until_head_transition() {
        let armed = Arc::new(AtomicBool::new(false));
        let catalog_published = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let publisher = PausingPackedPublisher {
            armed: armed.clone(),
            catalog_published: catalog_published.clone(),
            release: release.clone(),
        };
        let (path, store) = store_with_publisher("packed-reclamation-publication", publisher);
        let store = Arc::new(store);
        let first = store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"first".to_vec(), b"one".to_vec())]),
            )
            .unwrap();
        armed.store(true, Ordering::SeqCst);
        let writer_store = store.clone();
        let writer = std::thread::spawn(move || {
            writer_store.insert_many(
                first,
                &BTreeMap::from([(b"second".to_vec(), b"two".to_vec())]),
            )
        });
        catalog_published.wait();

        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let independent = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        assert!(matches!(
            independent.reclaim_unreachable_packed_files(),
            Err(PatriciaIndexReclamationError::Busy)
        ));
        release.wait();
        let current = writer.join().unwrap().unwrap();
        assert_eq!(
            independent.lookup(current, b"second").unwrap(),
            Some(b"two".to_vec())
        );
        independent.reclaim_unreachable_packed_files().unwrap();
        assert_eq!(
            independent.lookup(current, b"first").unwrap(),
            Some(b"one".to_vec())
        );
        drop(independent);
        drop(root_dir);
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[ignore = "subprocess helper invoked by packed_lock_process_death_releases_reclamation"]
    fn packed_operational_lock_subprocess_helper() {
        use std::io::{BufRead as _, Write as _};

        let Ok(path) = std::env::var("TINE_PACKED_LOCK_HELPER_PATH") else {
            return;
        };
        let nodes = Dir::open_ambient_dir(path, ambient_authority()).unwrap();
        let _guard = crate::packed_patricia::lock_packed_operation_shared(&nodes).unwrap();
        println!("locked");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(std::io::stdin())
            .read_line(&mut line)
            .unwrap();
    }

    #[test]
    fn packed_lock_process_death_releases_reclamation() {
        use std::io::BufRead as _;
        use std::process::{Command, Stdio};

        let (path, store) =
            store_with_publisher("packed-reclamation-process", PackedExactPublisher);
        store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"authority".to_vec(), b"valid".to_vec())]),
            )
            .unwrap();
        let nodes_path = path.join("nodes");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("packed_operational_lock_subprocess_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env("TINE_PACKED_LOCK_HELPER_PATH", &nodes_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(output.read_line(&mut line).unwrap(), 0);
            if line.trim() == "locked" {
                break;
            }
        }
        assert!(matches!(
            store.reclaim_unreachable_packed_files(),
            Err(PatriciaIndexReclamationError::Busy)
        ));
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        store.reclaim_unreachable_packed_files().unwrap();
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn normal_patricia_operations_never_scan_the_node_directory() {
        let scans_before = crate::packed_patricia::reclamation_directory_scans();
        let (path, store) = store_with_publisher("packed-no-normal-scan", PackedExactPublisher);
        let root = store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"normal".to_vec(), b"operation".to_vec())]),
            )
            .unwrap();
        assert_eq!(
            store.lookup(root, b"normal").unwrap(),
            Some(b"operation".to_vec())
        );
        drop(store);
        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let reopened = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        assert_eq!(
            reopened.lookup(root, b"normal").unwrap(),
            Some(b"operation".to_vec())
        );
        assert_eq!(
            crate::packed_patricia::reclamation_directory_scans(),
            scans_before
        );
        drop(reopened);
        drop(root_dir);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn packed_writer_first_update_and_mixed_loose_ancestor_are_semantic_with_low_fanout() {
        let records = (0..512)
            .map(|index| {
                (
                    format!("pages/packed-writer-{index:04}.md").into_bytes(),
                    format!("value-{index:04}-α").into_bytes(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let (loose_path, loose) = store("writer-fanout-loose");
        let loose_root = loose
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let (packed_path, packed) =
            store_with_publisher("writer-fanout-packed", PackedExactPublisher);
        let packed_root = packed
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        assert_eq!(packed_root, loose_root);
        assert_eq!(all_records(&packed, packed_root), records);

        let packed_nodes = packed_path.join("nodes");
        let loose_nodes = loose_path.join("nodes");
        assert_eq!(count_suffix(&packed_nodes, NODE_SUFFIX), 0);
        assert_eq!(
            count_suffix(&packed_nodes, crate::packed_patricia::PACK_SUFFIX),
            1
        );
        assert_eq!(fs::read_dir(&packed_nodes).unwrap().count(), 4);

        let update = BTreeMap::from([
            (
                b"pages/packed-writer-0256.md".to_vec(),
                b"updated-value".to_vec(),
            ),
            (
                b"pages/packed-writer-new.md".to_vec(),
                b"new-value".to_vec(),
            ),
        ]);
        let updated_packed = packed.insert_many(packed_root, &update).unwrap();
        let updated_loose = loose.insert_many(loose_root, &update).unwrap();
        assert_eq!(updated_packed, updated_loose);
        let mut expected = records.clone();
        expected.extend(update);
        assert_eq!(all_records(&packed, updated_packed), expected);
        assert_eq!(count_suffix(&packed_nodes, NODE_SUFFIX), 0);
        assert!(
            fs::read_dir(&loose_nodes).unwrap().count()
                > fs::read_dir(&packed_nodes).unwrap().count() * 50,
            "a real two-write journey must materially reduce immutable directory fan-out"
        );

        let (mixed_path, mixed_loose) = store("writer-mixed-ancestor");
        let ancestor_records = records.into_iter().take(128).collect::<BTreeMap<_, _>>();
        let ancestor_root = mixed_loose
            .insert_many(PatriciaIndexRoot::empty(), &ancestor_records)
            .unwrap();
        let mixed_nodes = mixed_path.join("nodes");
        let loose_before = count_suffix(&mixed_nodes, NODE_SUFFIX);
        drop(mixed_loose);
        let mixed_dir = Dir::open_ambient_dir(&mixed_path, ambient_authority()).unwrap();
        let mixed = PatriciaIndexStore::new(
            open_dir_nofollow(&mixed_dir, "nodes").unwrap(),
            PackedExactPublisher,
        );
        let mixed_update = BTreeMap::from([(
            b"pages/packed-writer-0064.md".to_vec(),
            b"mixed-update".to_vec(),
        )]);
        let mixed_root = mixed.insert_many(ancestor_root, &mixed_update).unwrap();
        let mut mixed_expected = ancestor_records;
        mixed_expected.extend(mixed_update);
        assert_eq!(all_records(&mixed, mixed_root), mixed_expected);
        assert_eq!(count_suffix(&mixed_nodes, NODE_SUFFIX), loose_before);
        assert_eq!(
            count_suffix(&mixed_nodes, crate::packed_patricia::PACK_SUFFIX),
            1
        );

        drop(mixed);
        drop(mixed_dir);
        drop(packed);
        drop(loose);
        fs::remove_dir_all(mixed_path).unwrap();
        fs::remove_dir_all(packed_path).unwrap();
        fs::remove_dir_all(loose_path).unwrap();
    }

    #[test]
    fn packed_corruption_test_seam_preserves_path_mismatch_class() {
        let (path, store) = store_with_publisher("packed-corruption-seam", PackedExactPublisher);
        let records = BTreeMap::from([
            (b"pages/corruption-left.md".to_vec(), b"left".to_vec()),
            (b"pages/corruption-right.md".to_vec(), b"right".to_vec()),
        ]);
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        assert_eq!(all_records(&store, root), records);

        store.corrupt_packed_node_for_test(root.digest()).unwrap();
        assert!(matches!(
            store.validate_root(root),
            Err(PatriciaError::PathMismatch(_))
        ));

        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn packed_writer_capacity_falls_back_to_loose_without_changing_head() {
        let (path, mut store) =
            store_with_publisher("writer-capacity-fallback", PackedExactPublisher);
        let initial = (0..64)
            .map(|index| {
                (
                    format!("capacity/{index:04}").into_bytes(),
                    vec![index as u8; 32],
                )
            })
            .collect::<BTreeMap<_, _>>();
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &initial)
            .unwrap();
        let nodes = path.join("nodes");
        let head_before = fs::read(nodes.join(crate::packed_patricia::HEAD_FILENAME)).unwrap();
        let packs_before = count_suffix(&nodes, crate::packed_patricia::PACK_SUFFIX);
        let catalogs_before = count_suffix(&nodes, ".patricia-catalog-v1");
        store.packed_catalog_byte_limit = 1;
        let update = BTreeMap::from([(b"capacity/0032".to_vec(), b"fallback-value".to_vec())]);
        let updated = store.insert_many(root, &update).unwrap();
        let mut expected = initial;
        expected.extend(update);
        assert_eq!(all_records(&store, updated), expected);
        assert_eq!(
            fs::read(nodes.join(crate::packed_patricia::HEAD_FILENAME)).unwrap(),
            head_before
        );
        assert!(count_suffix(&nodes, NODE_SUFFIX) > 0);
        assert_eq!(
            count_suffix(&nodes, crate::packed_patricia::PACK_SUFFIX),
            packs_before,
            "capacity fallback must fail before publishing an orphan delta pack"
        );
        assert_eq!(
            count_suffix(&nodes, ".patricia-catalog-v1"),
            catalogs_before,
            "capacity fallback must fail before publishing a replacement catalog"
        );

        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn packed_construction_capacity_falls_back_before_pack_or_head_mutation() {
        let publisher = ConstructionPublicationRecorder::default();
        let final_publication_calls = Arc::clone(&publisher.final_publication_calls);
        let (path, mut store) = store_with_publisher("construction-capacity-fallback", publisher);
        let initial = bulk_differential_records(0, 96, 0);
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &initial)
            .unwrap();
        let nodes = path.join("nodes");
        let head_before = fs::read(nodes.join(crate::packed_patricia::HEAD_FILENAME)).unwrap();
        let packs_before = count_suffix(&nodes, crate::packed_patricia::PACK_SUFFIX);
        let catalogs_before = count_suffix(&nodes, ".patricia-catalog-v1");
        store.packed_catalog_byte_limit = 1;

        let update = bulk_differential_records(48, 96, 50_000);
        let mut construction = PatriciaIndexConstruction::default();
        let updated = store
            .construction_insert_many_bulk(&mut construction, root, &update)
            .unwrap();
        construction.set_live_roots([updated]);
        construction.checkpoint([updated]);
        let completion = store.finish_construction(&mut construction).unwrap();
        let stats = completion.stats();
        assert_eq!(stats.capacity_fallbacks, 1);
        assert!(stats.loose_publication_calls > 0);
        assert_eq!(
            fs::read(nodes.join(crate::packed_patricia::HEAD_FILENAME)).unwrap(),
            head_before
        );
        assert_eq!(
            count_suffix(&nodes, crate::packed_patricia::PACK_SUFFIX),
            packs_before,
        );
        assert_eq!(
            count_suffix(&nodes, ".patricia-catalog-v1"),
            catalogs_before,
        );
        assert_eq!(
            final_publication_calls.load(Ordering::Relaxed),
            0,
            "capacity refusal must precede every final pack/catalog publisher call",
        );
        assert_eq!(
            fs::read_dir(&nodes)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(".tmp-"))
                .count(),
            0,
            "capacity fallback must drop every unpublished staged temp",
        );
        let mut expected = initial;
        expected.extend(update);
        assert_eq!(all_records(&store, updated), expected);
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn packed_writer_large_history_update_touches_only_delta_payload_and_bounded_metadata() {
        const RECORDS: usize = 4_096;

        let publisher = MeasuringPackedPublisher::default();
        let (path, store) = store_with_publisher("writer-large-history-work", publisher.clone());
        let initial = (0..RECORDS)
            .map(|index| {
                (
                    format!("history/{index:05}").into_bytes(),
                    format!("historical-value-{index:05}-{}", "x".repeat(48)).into_bytes(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &initial)
            .unwrap();
        let nodes = path.join("nodes");
        let historical_pack_bytes = fs::read_dir(&nodes)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(crate::packed_patricia::PACK_SUFFIX)
            })
            .map(|entry| entry.metadata().unwrap().len() as usize)
            .sum::<usize>();
        assert!(historical_pack_bytes > 512 * 1024);
        publisher.take();
        let stats_before = store.stats();

        let updated = store
            .insert_many(
                root,
                &BTreeMap::from([(b"history/02048".to_vec(), b"one-small-update".to_vec())]),
            )
            .unwrap();
        let stats_after = store.stats();
        let work = store
            .packed_publication_work
            .lock()
            .unwrap()
            .expect("successful packed update records its physical work");
        let publications = publisher.take();

        assert_ne!(updated, root);
        assert_eq!(work.existing_payload_bytes_compared, 0);
        assert_eq!(work.pack_bytes_encoded, work.pack_bytes_published);
        assert_eq!(
            work.catalog_metadata_bytes_encoded,
            work.catalog_metadata_bytes_published
        );
        assert_eq!(work.packs_published, 1);
        assert_eq!(
            publications
                .iter()
                .filter(|(name, _)| name.ends_with(crate::packed_patricia::PACK_SUFFIX))
                .count(),
            1
        );
        assert_eq!(
            publications
                .iter()
                .filter(|(name, _)| name.ends_with(".patricia-catalog-v1"))
                .count(),
            1
        );
        assert!(work.new_payload_bytes < historical_pack_bytes / 50);
        assert!(work.pack_bytes_published < historical_pack_bytes / 50);
        assert!(work.catalog_metadata_bytes_published <= 1_024);
        assert!(stats_after.bytes_read - stats_before.bytes_read < historical_pack_bytes / 50);
        eprintln!(
            "large-history packed update: history_pack_bytes={historical_pack_bytes}, mutation_read_bytes={}, new_payload_bytes={}, pack_bytes_encoded={}, pack_bytes_published={}, catalog_bytes={}",
            stats_after.bytes_read - stats_before.bytes_read,
            work.new_payload_bytes,
            work.pack_bytes_encoded,
            work.pack_bytes_published,
            work.catalog_metadata_bytes_published,
        );

        assert_eq!(
            store.lookup(updated, b"history/02048").unwrap(),
            Some(b"one-small-update".to_vec())
        );
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn packed_writer_reopens_and_retries_pack_and_catalog_crash_boundaries() {
        let records = (0..96)
            .map(|index| {
                (
                    format!("crash/{index:04}").into_bytes(),
                    format!("value-{index:04}").into_bytes(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for (fail_at, after_commit) in [(1, false), (1, true), (2, false), (2, true)] {
            let publisher = BoundaryCrashPublisher {
                calls: Arc::new(AtomicUsize::new(0)),
                fail_at,
                after_commit,
            };
            let (path, failed) =
                store_with_publisher(&format!("writer-crash-{fail_at}-{after_commit}"), publisher);
            assert!(matches!(
                failed.insert_many(PatriciaIndexRoot::empty(), &records),
                Err(PatriciaError::Publication(_))
            ));
            assert!(!path
                .join("nodes")
                .join(crate::packed_patricia::HEAD_FILENAME)
                .exists());
            drop(failed);

            let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
            let retry = PatriciaIndexStore::new(
                open_dir_nofollow(&root_dir, "nodes").unwrap(),
                PackedExactPublisher,
            );
            let root = retry
                .insert_many(PatriciaIndexRoot::empty(), &records)
                .unwrap();
            assert_eq!(all_records(&retry, root), records);
            drop(retry);
            drop(root_dir);
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn adapter_is_differential_for_legacy_packed_mixed_and_loose_advancement() {
        const RECORDS: usize = 192;

        let (legacy_path, legacy) = store("adapter-legacy-source");
        let expected = (0..RECORDS)
            .map(|index| {
                (
                    format!("pages/adapter-α-{index:04}.md").into_bytes(),
                    format!("值-{index:04}").into_bytes(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let root = legacy
            .insert_many(PatriciaIndexRoot::empty(), &expected)
            .unwrap();
        let reachable = reachable_node_bytes(&legacy, [root]);
        let legacy_file_count = fs::read_dir(legacy_path.join("nodes")).unwrap().count();
        drop(legacy);

        let legacy_root = Dir::open_ambient_dir(&legacy_path, ambient_authority()).unwrap();
        let legacy = PatriciaIndexStore::new(
            open_dir_nofollow(&legacy_root, "nodes").unwrap(),
            ExactPublisher,
        );
        assert_eq!(all_records(&legacy, root), expected);
        assert_eq!(
            fs::read_dir(legacy_path.join("nodes")).unwrap().count(),
            legacy_file_count,
            "head absence must not migrate, delete, or eagerly repack legacy history"
        );

        let packed_path =
            std::env::temp_dir().join(format!("tine-patricia-adapter-packed-{}", Uuid::new_v4()));
        fs::create_dir(&packed_path).unwrap();
        let packed_dir = Dir::open_ambient_dir(&packed_path, ambient_authority()).unwrap();
        publish_cataloged_nodes(&packed_dir, &reachable, reachable.len());
        assert_eq!(fs::read_dir(&packed_path).unwrap().count(), 3);
        let packed = PatriciaIndexStore::new(packed_dir, ExactPublisher);
        assert_eq!(all_records(&packed, root), expected);
        assert_eq!(
            packed
                .lookup(root, "pages/adapter-α-0096.md".as_bytes())
                .unwrap(),
            Some("值-0096".as_bytes().to_vec())
        );
        assert_eq!(fs::read_dir(&packed_path).unwrap().count(), 4);

        let root_node = reachable.get(&root.digest()).unwrap();
        fs::write(
            packed_path.join(node_filename(root.digest())),
            b"conflicting loose bytes",
        )
        .unwrap();
        assert!(matches!(
            packed.lookup(root, "pages/adapter-α-0096.md".as_bytes()),
            Err(PatriciaError::PathMismatch(digest)) if digest == root.digest()
        ));
        fs::remove_file(packed_path.join(node_filename(root.digest()))).unwrap();
        publish_immutable_exact(
            &Dir::open_ambient_dir(&packed_path, ambient_authority()).unwrap(),
            &node_filename(root.digest()),
            root_node,
        )
        .unwrap();

        let advanced = packed
            .insert_many(
                root,
                &BTreeMap::from([(b"pages/adapter-new.md".to_vec(), b"new".to_vec())]),
            )
            .unwrap();
        let mut advanced_expected = expected.clone();
        advanced_expected.insert(b"pages/adapter-new.md".to_vec(), b"new".to_vec());
        assert_eq!(all_records(&packed, advanced), advanced_expected);
        assert!(
            fs::read_dir(&packed_path).unwrap().count() > 3,
            "the current writer may safely add loose nodes over a packed history"
        );

        let mixed_path =
            std::env::temp_dir().join(format!("tine-patricia-adapter-mixed-{}", Uuid::new_v4()));
        fs::create_dir(&mixed_path).unwrap();
        let mixed_dir = Dir::open_ambient_dir(&mixed_path, ambient_authority()).unwrap();
        let entries = reachable.iter().collect::<Vec<_>>();
        let split = entries.len() / 2;
        let cataloged = entries[..split]
            .iter()
            .map(|(digest, bytes)| (**digest, (*bytes).clone()))
            .collect::<BTreeMap<_, _>>();
        publish_cataloged_nodes(&mixed_dir, &cataloged, cataloged.len());
        for (digest, bytes) in &entries[split..] {
            publish_immutable_exact(&mixed_dir, &node_filename(**digest), bytes).unwrap();
        }
        let mixed = PatriciaIndexStore::new(mixed_dir, ExactPublisher);
        assert_eq!(all_records(&mixed, root), expected);
        mixed.validate_root(root).unwrap();

        drop(mixed);
        drop(packed);
        drop(legacy);
        drop(legacy_root);
        fs::remove_dir_all(mixed_path).unwrap();
        fs::remove_dir_all(packed_path).unwrap();
        fs::remove_dir_all(legacy_path).unwrap();
    }

    #[test]
    fn construction_flushes_only_checkpoint_live_and_in_progress_roots() {
        const RESIDENT_BUDGET: usize = 1024 * 1024;
        const ROUNDS: usize = 128;

        assert!(
            std::mem::size_of::<ContentDigest>()
                + std::mem::size_of::<Node>()
                + 2 * std::mem::size_of::<ContentDigest>()
                < 256,
            "the documented 8x inline-ownership margin must remain conservative"
        );

        let (baseline_path, baseline) = store("construction-baseline");
        let (construction_path, constructed) =
            store_with_publisher("construction-reachable", PackedExactPublisher);
        let mut construction = PatriciaIndexConstruction {
            resident_budget_bytes: RESIDENT_BUDGET,
            ..PatriciaIndexConstruction::default()
        };
        let mut baseline_roots = [PatriciaIndexRoot::empty(); 3];
        let mut live_roots = [PatriciaIndexRoot::empty(); 3];
        let mut expected: [BTreeMap<Vec<u8>, Vec<u8>>; 3] =
            std::array::from_fn(|_| BTreeMap::new());
        let mut checkpoints: Vec<([PatriciaIndexRoot; 3], [BTreeMap<Vec<u8>, Vec<u8>>; 3])> =
            Vec::new();

        // Seed the target through its packed writer. Construction then adds
        // loose streamed nodes over that authenticated catalog, exercising the
        // mixed packed/loose compatibility used by existing archives.
        for sibling in 0..3 {
            let key = format!("sibling-{sibling}/packed-seed").into_bytes();
            let value = vec![sibling as u8; 32];
            let records = BTreeMap::from([(key.clone(), value.clone())]);
            baseline_roots[sibling] = baseline
                .insert_many(baseline_roots[sibling], &records)
                .unwrap();
            live_roots[sibling] = constructed
                .insert_many(live_roots[sibling], &records)
                .unwrap();
            expected[sibling].insert(key, value);
            assert_eq!(live_roots[sibling], baseline_roots[sibling]);
        }
        assert!(fs::read_dir(construction_path.join("nodes"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".patricia-pack-v1")));
        construction.set_live_roots(live_roots);

        for round in 0..ROUNDS {
            for sibling in 0..3 {
                let key = format!("sibling-{sibling}/record-{round:03}").into_bytes();
                let value = vec![(round * 3 + sibling) as u8; 96];
                let records = BTreeMap::from([(key.clone(), value.clone())]);
                baseline_roots[sibling] = baseline
                    .insert_many(baseline_roots[sibling], &records)
                    .unwrap();
                let flushes_before = construction.flushes;
                live_roots[sibling] = constructed
                    .construction_insert_many(&mut construction, live_roots[sibling], &records)
                    .unwrap();
                expected[sibling].insert(key, value);
                assert_eq!(live_roots[sibling], baseline_roots[sibling]);
                construction.set_live_roots(live_roots);

                if construction.flushes != flushes_before {
                    for (roots, records) in &checkpoints {
                        for index in 0..3 {
                            assert_eq!(all_records(&constructed, roots[index]), records[index]);
                        }
                    }
                    for index in 0..3 {
                        assert_eq!(
                            all_construction_records(
                                &constructed,
                                &construction,
                                live_roots[index]
                            ),
                            expected[index]
                        );
                    }
                }
            }

            if round >= 8 && round % 7 == 3 {
                let sibling = round % 3;
                let key = format!("sibling-{sibling}/record-{:03}", round - 8).into_bytes();
                let keys = vec![key.clone()];
                baseline_roots[sibling] = baseline
                    .remove_many(baseline_roots[sibling], &keys)
                    .unwrap();
                let flushes_before = construction.flushes;
                live_roots[sibling] = constructed
                    .construction_remove_many(&mut construction, live_roots[sibling], &keys)
                    .unwrap();
                expected[sibling].remove(&key);
                assert_eq!(live_roots[sibling], baseline_roots[sibling]);
                construction.set_live_roots(live_roots);
                if construction.flushes != flushes_before {
                    for index in 0..3 {
                        assert_eq!(
                            all_construction_records(
                                &constructed,
                                &construction,
                                live_roots[index]
                            ),
                            expected[index]
                        );
                    }
                }
            }

            if round % 6 == 5 && round + 1 != ROUNDS {
                construction.checkpoint(live_roots);
                checkpoints.push((live_roots, expected.clone()));
            }
        }

        assert!(
            construction.flushes >= 3,
            "fixture must force several flushes"
        );
        assert!(
            construction.peak_resident_bytes <= RESIDENT_BUDGET,
            "pre-mutation accounting and streaming publication must honor the total bound: peak={} budget={RESIDENT_BUDGET}",
            construction.peak_resident_bytes,
        );
        assert!(
            !construction.staged.nodes.is_empty(),
            "fixture must leave reachable work for finalization"
        );
        assert!(
            construction.published_staged_nodes < construction.staged_nodes_at_publication,
            "budget flushes must omit transient staged path copies before finalization"
        );
        for (root, records) in live_roots.iter().copied().zip(expected.iter()) {
            constructed
                .construction_validate_root(&construction, root)
                .unwrap();
            let mut visited = BTreeMap::new();
            constructed
                .construction_visit_all(&construction, root, |key, value| {
                    visited.insert(key.to_vec(), value.to_vec());
                    true
                })
                .unwrap();
            assert_eq!(&visited, records);
        }
        assert_eq!(
            constructed
                .construction_lookup_prefix_limited(
                    &construction,
                    live_roots[1],
                    b"sibling-1/record-0",
                    5,
                )
                .unwrap(),
            expected[1]
                .iter()
                .filter(|(key, _)| key.starts_with(b"sibling-1/record-0"))
                .take(5)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        );
        let publications_before_finish = construction.published_staged_nodes;
        constructed.finish_construction(&mut construction).unwrap();
        assert!(construction.peak_publication_resident_bytes > 0);
        assert!(
            construction.peak_publication_resident_bytes <= RESIDENT_BUDGET,
            "streaming publication must remain inside the configured total bound"
        );
        assert!(
            construction.peak_resident_bytes >= construction.peak_publication_resident_bytes,
            "the reported total peak must include publication"
        );
        assert!(construction.staged.nodes.is_empty());
        assert!(construction.published_staged_nodes > publications_before_finish);
        let publications_after_finish = construction.published_staged_nodes;
        let writes_after_finish = constructed.stats().writes;
        constructed.finish_construction(&mut construction).unwrap();
        assert_eq!(
            construction.published_staged_nodes, publications_after_finish,
            "final reachable staged nodes publish only once"
        );
        assert_eq!(constructed.stats().writes, writes_after_finish);
        let authority_roots = checkpoints
            .iter()
            .flat_map(|(roots, _)| roots.iter().copied())
            .chain(live_roots);
        assert_eq!(
            reachable_node_bytes(&constructed, authority_roots.clone()),
            reachable_node_bytes(&baseline, authority_roots),
            "every reachable node must retain its prior canonical bytes"
        );
        assert!(
            fs::read_dir(baseline_path.join("nodes")).unwrap().count()
                > fs::read_dir(construction_path.join("nodes"))
                    .unwrap()
                    .count(),
            "the real construction journey must create fewer immutable files through packs"
        );

        drop(constructed);
        let construction_root =
            Dir::open_ambient_dir(&construction_path, ambient_authority()).unwrap();
        let reopened = PatriciaIndexStore::new(
            open_dir_nofollow(&construction_root, "nodes").unwrap(),
            ExactPublisher,
        );
        for (roots, records) in checkpoints {
            for index in 0..3 {
                assert_eq!(all_records(&reopened, roots[index]), records[index]);
            }
        }
        for index in 0..3 {
            assert_eq!(all_records(&reopened, live_roots[index]), expected[index]);
        }

        drop(reopened);
        drop(construction_root);
        drop(baseline);
        fs::remove_dir_all(baseline_path).unwrap();
        fs::remove_dir_all(construction_path).unwrap();
    }

    #[test]
    fn streaming_construction_publication_retries_from_the_failed_postorder_node() {
        let (baseline_path, baseline) = store("construction-retry-baseline");
        let path = std::env::temp_dir().join(format!(
            "tine-claim-index-construction-retry-{}",
            Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        let dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        ensure_directory_nofollow(&dir, "nodes").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let constructed = PatriciaIndexStore::new(
            open_dir_nofollow(&dir, "nodes").unwrap(),
            BoundaryCrashPublisher {
                calls: Arc::clone(&calls),
                fail_at: 3,
                after_commit: false,
            },
        );
        let records = (0..48)
            .map(|index| {
                (
                    format!("pages/retry-{index:03}.md").into_bytes(),
                    vec![index as u8; 64],
                )
            })
            .collect::<BTreeMap<_, _>>();
        let expected_root = baseline
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let mut construction = PatriciaIndexConstruction::default();
        let root = constructed
            .construction_insert_many(&mut construction, PatriciaIndexRoot::empty(), &records)
            .unwrap();
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        let staged_before = construction.staged.nodes.len();

        assert!(matches!(
            constructed.finish_construction(&mut construction),
            Err(PatriciaError::Publication(_))
        ));
        assert!(construction.staged.nodes.len() < staged_before);
        assert!(!construction.staged.nodes.is_empty());

        constructed.finish_construction(&mut construction).unwrap();
        assert_eq!(root, expected_root);
        assert_eq!(all_records(&constructed, root), records);
        assert!(construction.peak_resident_bytes <= construction.resident_budget_bytes);

        drop(constructed);
        drop(dir);
        drop(baseline);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(baseline_path).unwrap();
    }

    fn bulk_differential_records(
        start: usize,
        count: usize,
        value_bias: usize,
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        (start..start + count)
            .map(|index| {
                (
                    bulk_differential_key(index),
                    format!("value-{:05}", index + value_bias).into_bytes(),
                )
            })
            .collect()
    }

    fn bulk_differential_key(index: usize) -> Vec<u8> {
        ContentDigest::of(format!("bulk-key-{index:05}").as_bytes())
            .to_string()
            .into_bytes()
    }

    fn bulk_boundary_records(value_bias: u8) -> BTreeMap<Vec<u8>, Vec<u8>> {
        [0x00_u8, 0x40, 0x80, 0xc0]
            .into_iter()
            .enumerate()
            .map(|(index, key)| (vec![key], vec![value_bias + index as u8]))
            .collect()
    }

    fn small_budget_records(count: usize, value_bias: u8) -> BTreeMap<Vec<u8>, Vec<u8>> {
        (0..count)
            .map(|index| {
                (
                    vec![u8::try_from(index + 1).unwrap()],
                    vec![value_bias.wrapping_add(index as u8); 32],
                )
            })
            .collect()
    }

    fn assert_construction_differential(
        baseline: &PatriciaIndexStore,
        constructed: &PatriciaIndexStore,
        baseline_root: PatriciaIndexRoot,
        constructed_root: PatriciaIndexRoot,
        expected: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) {
        assert_eq!(constructed_root, baseline_root);
        assert_eq!(all_records(constructed, constructed_root), *expected);
        assert_eq!(
            reachable_node_bytes(constructed, [constructed_root]),
            reachable_node_bytes(baseline, [baseline_root]),
            "small-budget construction must retain canonical node semantics and bytes",
        );
    }

    #[test]
    fn complete_root_derivation_matches_in_memory_point_semantics_for_4096_records() {
        let records = bulk_differential_records(0, 4096, 17);
        let (path, store) = store("complete-root-derivation");
        let (staged_root, staged) = store
            .stage_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let construction = PatriciaIndexConstruction {
            staged,
            ..PatriciaIndexConstruction::default()
        };

        assert_eq!(store.derive_complete_root(&records).unwrap(), staged_root);
        assert_eq!(
            all_construction_records(&store, &construction, staged_root),
            records
        );
        assert_eq!(
            fs::read_dir(path.join("nodes")).unwrap().count(),
            0,
            "the routine 4096-record semantic check must not perform physical publication"
        );

        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn physical_publication_reopens_with_exact_bytes_for_96_records() {
        let records = bulk_differential_records(0, 96, 23);
        let (path, store) = store("physical-reopen-96");
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let exact_before = reachable_node_bytes(&store, [root]);
        drop(store);

        let authority = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let nodes = open_dir_nofollow(&authority, "nodes").unwrap();
        let reopened = PatriciaIndexStore::new(nodes, ExactPublisher);
        assert_eq!(all_records(&reopened, root), records);
        assert_eq!(reachable_node_bytes(&reopened, [root]), exact_before);
        assert_eq!(reopened.derive_complete_root(&records).unwrap(), root);
        assert_eq!(count_suffix(&path.join("nodes"), ".tmp"), 0);

        drop(reopened);
        // On Windows the retained root capability intentionally omits
        // FILE_SHARE_DELETE. Close it before removing the physical fixture;
        // the publication/reopen proof above has already completed.
        drop(authority);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[ignore = "certification burn-in: physically publishes and reopens 4096 records"]
    fn physical_publication_reopens_exactly_for_4096_records_burn_in() {
        let records = bulk_differential_records(0, 4096, 17);
        let (path, store) = store("physical-reopen-4096-burn-in");
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let exact_before = reachable_node_bytes(&store, [root]);
        drop(store);

        let authority = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let nodes = open_dir_nofollow(&authority, "nodes").unwrap();
        let reopened = PatriciaIndexStore::new(nodes, ExactPublisher);
        assert_eq!(all_records(&reopened, root), records);
        assert_eq!(reachable_node_bytes(&reopened, [root]), exact_before);
        assert_eq!(count_suffix(&path.join("nodes"), ".tmp"), 0);

        drop(reopened);
        drop(authority);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn construction_sink_growth_refuses_within_small_budget_and_stays_semantic() {
        const RESIDENT_BUDGET: usize = 2 * 1024 * 1024;

        let mut retained_with_spare = Vec::with_capacity(4 * 1024);
        retained_with_spare.extend_from_slice(b"retained spare capacity");
        let retained_capacity = retained_with_spare.capacity();
        let mut capacity_sink = PackedPatriciaConstructionSink::new(8 * 1024);
        assert!(capacity_sink
            .accept(ContentDigest::of(&retained_with_spare), retained_with_spare)
            .unwrap());
        assert!(capacity_sink.owned_bytes() >= retained_capacity);

        let mut refused_with_spare = Vec::with_capacity(64 * 1024);
        refused_with_spare.extend_from_slice(b"refused spare capacity");
        let mut capacity_sink = PackedPatriciaConstructionSink::new(32 * 1024);
        assert!(!capacity_sink
            .accept(ContentDigest::of(&refused_with_spare), refused_with_spare)
            .unwrap());

        let records = small_budget_records(96, 7);
        let (baseline_path, baseline) = store("construction-small-sink-baseline");
        let (path, constructed) =
            store_with_publisher("construction-small-sink", PackedExactPublisher);
        let baseline_root = baseline
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let reachable = reachable_node_bytes(&baseline, [baseline_root]);
        let mut sink = PackedPatriciaConstructionSink::new(64 * 1024);
        let mut refused = false;
        for (digest, bytes) in &reachable {
            if !sink.accept(*digest, bytes.clone()).unwrap() {
                refused = true;
                break;
            }
        }
        assert!(
            refused,
            "owned sink growth must refuse its small residency budget"
        );
        assert!(sink.owned_bytes() <= 64 * 1024);

        let mut construction = PatriciaIndexConstruction {
            resident_budget_bytes: RESIDENT_BUDGET,
            ..PatriciaIndexConstruction::default()
        };
        let root = constructed
            .construction_insert_many(&mut construction, PatriciaIndexRoot::empty(), &records)
            .unwrap();
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        let completion = constructed.finish_construction(&mut construction).unwrap();
        assert!(completion.stats().peak_resident_bytes <= RESIDENT_BUDGET);
        assert_construction_differential(&baseline, &constructed, baseline_root, root, &records);

        drop(constructed);
        drop(baseline);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(baseline_path).unwrap();
    }

    #[test]
    fn construction_ordinary_append_reports_complete_small_budget_peak() {
        const RESIDENT_BUDGET: usize = 2 * 1024 * 1024;

        let initial = small_budget_records(48, 11);
        let update = small_budget_records(12, 101);
        let mut expected = initial.clone();
        expected.extend(update.clone());
        let (baseline_path, baseline) = store("construction-small-append-baseline");
        let (path, constructed) =
            store_with_publisher("construction-small-append", PackedExactPublisher);
        let baseline_initial = baseline
            .insert_many(PatriciaIndexRoot::empty(), &initial)
            .unwrap();
        let constructed_initial = constructed
            .insert_many(PatriciaIndexRoot::empty(), &initial)
            .unwrap();
        let baseline_root = baseline.insert_many(baseline_initial, &update).unwrap();
        let mut construction = PatriciaIndexConstruction {
            resident_budget_bytes: RESIDENT_BUDGET,
            ..PatriciaIndexConstruction::default()
        };
        let root = constructed
            .construction_insert_many_bulk(&mut construction, constructed_initial, &update)
            .unwrap();
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        let completion = constructed.finish_construction(&mut construction).unwrap();
        let work = constructed
            .packed_publication_work
            .lock()
            .unwrap()
            .expect("ordinary append records bounded packed work");
        assert_eq!(work.compaction_packs_selected, 0);
        assert!(work.peak_resident_bytes > 0);
        assert_eq!(
            completion.stats().peak_resident_bytes,
            work.peak_resident_bytes
        );
        assert!(completion.stats().peak_resident_bytes <= RESIDENT_BUDGET);
        assert_construction_differential(&baseline, &constructed, baseline_root, root, &expected);

        drop(constructed);
        drop(baseline);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(baseline_path).unwrap();
    }

    #[test]
    fn construction_tier_carry_reports_complete_small_budget_peak() {
        const RESIDENT_BUDGET: usize = 2 * 1024 * 1024;

        let key = vec![0x80];
        let (baseline_path, baseline) = store("construction-small-carry-baseline");
        let (path, constructed) =
            store_with_publisher("construction-small-carry", PackedExactPublisher);
        let mut baseline_root = PatriciaIndexRoot::empty();
        let mut constructed_root = PatriciaIndexRoot::empty();
        for version in 0..4_u8 {
            let records = BTreeMap::from([(key.clone(), vec![version; 64])]);
            baseline_root = baseline.insert_many(baseline_root, &records).unwrap();
            constructed_root = constructed.insert_many(constructed_root, &records).unwrap();
        }
        let update = BTreeMap::from([(key.clone(), vec![4; 64])]);
        baseline_root = baseline.insert_many(baseline_root, &update).unwrap();
        let mut construction = PatriciaIndexConstruction {
            resident_budget_bytes: RESIDENT_BUDGET,
            ..PatriciaIndexConstruction::default()
        };
        constructed_root = constructed
            .construction_insert_many_bulk(&mut construction, constructed_root, &update)
            .unwrap();
        construction.set_live_roots([constructed_root]);
        construction.checkpoint([constructed_root]);
        let completion = constructed.finish_construction(&mut construction).unwrap();
        let work = constructed
            .packed_publication_work
            .lock()
            .unwrap()
            .expect("tier carry records bounded packed work");
        assert_eq!(work.compaction_packs_selected, 5);
        assert!(work.compaction_pack_bytes_selected > work.delta_pack_bytes_encoded);
        assert_eq!(
            completion.stats().peak_resident_bytes,
            work.peak_resident_bytes
        );
        assert!(completion.stats().peak_resident_bytes <= RESIDENT_BUDGET);
        assert_construction_differential(
            &baseline,
            &constructed,
            baseline_root,
            constructed_root,
            &update,
        );

        drop(constructed);
        drop(baseline);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(baseline_path).unwrap();
    }

    fn exercise_packed_construction_prerequisite_retry(fail_at: usize, after_commit: bool) {
        let calls = Arc::new(AtomicUsize::new(0));
        let (path, store) = store_with_publisher(
            &format!("construction-packed-prerequisite-{fail_at}-{after_commit}"),
            ConstructionCrashPublisher {
                calls: Arc::clone(&calls),
                fail_at,
                after_commit,
            },
        );
        let records = bulk_differential_records(0, 96, fail_at);
        let mut construction = PatriciaIndexConstruction::default();
        assert!(matches!(
            store.construction_insert_many_bulk(
                &mut construction,
                PatriciaIndexRoot::empty(),
                &records,
            ),
            Err(PatriciaError::Publication(_))
        ));
        assert!(crate::packed_patricia::PackedPatriciaCatalog::discover(
            &Dir::open_ambient_dir(path.join("nodes"), ambient_authority()).unwrap()
        )
        .unwrap()
        .is_none());

        let root = store
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &records)
            .unwrap();
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        let completion = store.finish_construction(&mut construction).unwrap();
        assert_eq!(completion.stats().capacity_fallbacks, 0);
        assert_eq!(all_records(&store, root), records);

        drop(store);
        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let reopened = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            ExactPublisher,
        );
        assert_eq!(all_records(&reopened, root), records);
        drop(reopened);
        drop(root_dir);
        fs::remove_dir_all(path).unwrap();
    }

    fn exercise_packed_construction_head_retry(
        failure: crate::packed_patricia::HeadTransitionFailureForTest,
        authority_visible_after_failure: bool,
    ) {
        let (baseline_path, baseline) = store("construction-packed-head-retry-baseline");
        let (path, store) =
            store_with_publisher("construction-packed-head-retry", PackedExactPublisher);
        let records = bulk_differential_records(0, 96, 30_000);
        let expected_root = baseline
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let mut construction = PatriciaIndexConstruction::default();
        crate::packed_patricia::fail_next_head_transition_for_test(failure);
        assert!(matches!(
            store.construction_insert_many_bulk(
                &mut construction,
                PatriciaIndexRoot::empty(),
                &records,
            ),
            Err(PatriciaError::Filesystem(_))
        ));
        let nodes = Dir::open_ambient_dir(path.join("nodes"), ambient_authority()).unwrap();
        assert_eq!(
            crate::packed_patricia::PackedPatriciaCatalog::discover(&nodes)
                .unwrap()
                .is_some(),
            authority_visible_after_failure,
        );
        if authority_visible_after_failure {
            assert_eq!(all_records(&store, expected_root), records);
        }

        let root = store
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &records)
            .unwrap();
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        store.finish_construction(&mut construction).unwrap();
        assert_eq!(all_records(&store, root), records);
        drop(nodes);
        drop(store);
        drop(baseline);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(baseline_path).unwrap();
    }

    #[test]
    fn packed_construction_pack_catalog_and_head_failures_retry_exactly() {
        for fail_at in [1, 2] {
            exercise_packed_construction_prerequisite_retry(fail_at, false);
            exercise_packed_construction_prerequisite_retry(fail_at, true);
        }
        exercise_packed_construction_head_retry(
            crate::packed_patricia::HeadTransitionFailureForTest::Before,
            false,
        );
        exercise_packed_construction_head_retry(
            crate::packed_patricia::HeadTransitionFailureForTest::After,
            true,
        );
    }

    fn exercise_bulk_publication_retry(boundary: BulkPublicationBoundary, after_commit: bool) {
        const RESIDENT_BUDGET: usize = 512 * 1024;

        let boundary_name = match boundary {
            BulkPublicationBoundary::Leaf => "leaf",
            BulkPublicationBoundary::ChildBranch => "child",
            BulkPublicationBoundary::ParentBranch => "parent",
        };
        let (baseline_path, baseline) = store(&format!(
            "construction-bulk-{boundary_name}-{after_commit}-baseline"
        ));
        let armed = Arc::new(AtomicBool::new(false));
        let (path, bulk) = store_with_publisher(
            &format!("construction-bulk-{boundary_name}-{after_commit}"),
            BulkBoundaryCrashPublisher {
                armed: Arc::clone(&armed),
                boundary,
                after_commit,
            },
        );
        let first = bulk_boundary_records(10);
        let expected_first_root = baseline
            .insert_many(PatriciaIndexRoot::empty(), &first)
            .unwrap();
        let mut construction = PatriciaIndexConstruction {
            resident_budget_bytes: RESIDENT_BUDGET,
            ..PatriciaIndexConstruction::default()
        };

        // Force the construction preflight to flush unrelated staged work.
        // The fault is armed only afterward, so the observed error must come
        // from the selected leaf/child/parent boundary in the bulk builder.
        let filler_roots = (0..64)
            .map(|index| {
                construction
                    .staged
                    .stage(Node::Leaf {
                        schema_version: NODE_SCHEMA_VERSION,
                        key: vec![0xf0, index],
                        value: b"filler".to_vec(),
                    })
                    .map(PatriciaIndexRoot)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        construction.set_live_roots(filler_roots);
        let reservation =
            construction_bulk_reservation(first.len() * std::mem::size_of::<BulkRecord<'_>>())
                .unwrap();
        construction
            .prepare_mutation(&bulk, PatriciaIndexRoot::empty(), reservation)
            .unwrap();
        assert_eq!(construction.flushes, 1);

        armed.store(true, Ordering::Relaxed);
        assert!(matches!(
            bulk.construction_insert_many_bulk(
                &mut construction,
                PatriciaIndexRoot::empty(),
                &first,
            ),
            Err(PatriciaError::Publication(_))
        ));
        assert!(!armed.load(Ordering::Relaxed));

        let first_root = bulk
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &first)
            .unwrap();
        assert_eq!(first_root, expected_first_root);
        construction.set_live_roots([first_root]);
        construction.checkpoint([first_root]);

        let updates = BTreeMap::from([
            (vec![0x00], b"overwritten".to_vec()),
            (vec![0x20], b"added-left".to_vec()),
            (vec![0xe0], b"added-right".to_vec()),
        ]);
        let expected_second_root = baseline.insert_many(expected_first_root, &updates).unwrap();
        let second_root = bulk
            .construction_insert_many_bulk(&mut construction, first_root, &updates)
            .unwrap();
        assert_eq!(second_root, expected_second_root);
        construction.set_live_roots([second_root]);
        construction.checkpoint([second_root]);
        bulk.finish_construction(&mut construction).unwrap();
        assert!(construction.peak_resident_bytes <= RESIDENT_BUDGET);

        let mut expected_second = first.clone();
        expected_second.extend(updates);
        let checkpoints = [(first_root, first), (second_root, expected_second)];
        drop(bulk);
        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let reopened = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            ExactPublisher,
        );
        for (root, expected) in checkpoints {
            assert_eq!(all_records(&reopened, root), expected);
        }

        drop(reopened);
        drop(root_dir);
        drop(baseline);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(baseline_path).unwrap();
    }

    #[derive(Clone, Copy, Debug)]
    enum PersistedNodeDamage {
        Delete,
        Tamper,
    }

    fn exercise_bulk_rejects_damaged_historical_child(damage: PersistedNodeDamage) {
        let (path, bulk) = store(&format!("construction-bulk-damage-{damage:?}"));
        let mut construction = PatriciaIndexConstruction::default();
        let first = bulk_boundary_records(20);
        let first_root = bulk
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &first)
            .unwrap();
        construction.set_live_roots([first_root]);
        construction.checkpoint([first_root]);

        let left = match bulk.read_node(first_root.digest()).unwrap() {
            Node::Branch { left, .. } => left,
            Node::Leaf { .. } => panic!("four-record fixture must have a branch root"),
        };
        assert!(matches!(bulk.read_node(left).unwrap(), Node::Branch { .. }));
        let child_path = path.join("nodes").join(node_filename(left));
        match damage {
            PersistedNodeDamage::Delete => fs::remove_file(&child_path).unwrap(),
            PersistedNodeDamage::Tamper => fs::write(&child_path, b"tampered-node").unwrap(),
        }

        let updates = BTreeMap::from([(vec![0x00], b"next-part".to_vec())]);
        let result = bulk.construction_insert_many_bulk(&mut construction, first_root, &updates);
        match (damage, result) {
            (PersistedNodeDamage::Delete, Err(PatriciaError::MissingNode(digest))) => {
                assert_eq!(digest, left);
            }
            (PersistedNodeDamage::Tamper, Err(PatriciaError::PathMismatch(digest))) => {
                assert_eq!(digest, left);
            }
            (_, result) => panic!("damaged historical child was not rejected: {result:?}"),
        }

        drop(bulk);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn construction_bulk_is_byte_canonical_for_inserts_overwrites_and_removals() {
        let (legacy_path, legacy) = store("construction-bulk-differential-legacy");
        let (bulk_path, bulk) = store("construction-bulk-differential-bulk");
        let mut construction = PatriciaIndexConstruction::default();
        let first = bulk_differential_records(0, 512, 0);
        let mut legacy_root = legacy
            .insert_many(PatriciaIndexRoot::empty(), &first)
            .unwrap();
        let mut bulk_root = bulk
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &first)
            .unwrap();
        assert_eq!(bulk_root, legacy_root);
        construction.set_live_roots([bulk_root]);
        construction.checkpoint([bulk_root]);
        let first_root = bulk_root;

        let mut second = bulk_differential_records(384, 384, 10_000);
        second.extend(bulk_differential_records(900, 128, 20_000));
        legacy_root = legacy.insert_many(legacy_root, &second).unwrap();
        bulk_root = bulk
            .construction_insert_many_bulk(&mut construction, bulk_root, &second)
            .unwrap();
        assert_eq!(bulk_root, legacy_root);
        construction.set_live_roots([bulk_root]);
        construction.checkpoint([bulk_root]);
        let second_root = bulk_root;

        let mut removals = (32..768)
            .step_by(7)
            .map(bulk_differential_key)
            .collect::<Vec<_>>();
        removals.sort_unstable();
        legacy_root = legacy.remove_many(legacy_root, &removals).unwrap();
        bulk_root = bulk
            .construction_remove_many(&mut construction, bulk_root, &removals)
            .unwrap();
        assert_eq!(bulk_root, legacy_root);
        construction.set_live_roots([bulk_root]);
        construction.checkpoint([bulk_root]);

        assert_eq!(
            all_construction_records(&bulk, &construction, first_root),
            first
        );
        let mut expected_second = first.clone();
        expected_second.extend(second);
        assert_eq!(
            all_construction_records(&bulk, &construction, second_root),
            expected_second
        );
        bulk.finish_construction(&mut construction).unwrap();
        assert_eq!(
            all_records(&bulk, bulk_root),
            all_records(&legacy, legacy_root)
        );
        assert_eq!(
            reachable_node_bytes(&bulk, [first_root, second_root, bulk_root]),
            reachable_node_bytes(&legacy, [first_root, second_root, legacy_root]),
            "bulk construction must retain the exact legacy node bytes for every checkpoint"
        );

        drop(bulk);
        drop(legacy);
        fs::remove_dir_all(bulk_path).unwrap();
        fs::remove_dir_all(legacy_path).unwrap();
    }

    #[test]
    fn construction_bulk_chunks_ranges_larger_than_one_resident_sink() {
        let (legacy_path, legacy) = store("construction-bulk-chunked-legacy");
        let (bulk_path, bulk) =
            store_with_publisher("construction-bulk-chunked", PackedExactPublisher);
        let mut construction = PatriciaIndexConstruction::default();
        let record_count = construction.bulk_record_limit() + 1_024;
        let records = bulk_differential_records(0, record_count, 77_000);
        let expected = legacy
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();

        let root = bulk
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &records)
            .unwrap();
        assert_eq!(root, expected);
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        let completed = bulk.finish_construction(&mut construction).unwrap();
        assert_eq!(completed.stats().capacity_fallbacks, 0);
        assert_eq!(completed.stats().loose_publication_calls, 0);
        assert!(completed.stats().pack_publication_calls >= 2);
        assert_eq!(all_records(&bulk, root), records);

        drop(bulk);
        drop(legacy);
        fs::remove_dir_all(bulk_path).unwrap();
        fs::remove_dir_all(legacy_path).unwrap();
    }

    #[test]
    fn construction_bulk_budget_flush_retries_idempotently_after_publication_failure() {
        for boundary in [
            BulkPublicationBoundary::Leaf,
            BulkPublicationBoundary::ChildBranch,
            BulkPublicationBoundary::ParentBranch,
        ] {
            exercise_bulk_publication_retry(boundary, false);
            exercise_bulk_publication_retry(boundary, true);
        }
    }

    #[test]
    fn construction_bulk_reads_persisted_historical_children_between_parts() {
        exercise_bulk_rejects_damaged_historical_child(PersistedNodeDamage::Delete);
        exercise_bulk_rejects_damaged_historical_child(PersistedNodeDamage::Tamper);
    }

    #[test]
    fn construction_bulk_falls_back_before_publication_when_plan_exceeds_budget() {
        let records = BTreeMap::from([(vec![0x80], b"value".to_vec())]);
        let minimum_bulk_reservation =
            construction_bulk_reservation(std::mem::size_of::<BulkRecord<'_>>()).unwrap();
        let legacy_reservation =
            construction_mutation_reservation(1, b"value".len(), true).unwrap();
        assert!(legacy_reservation < minimum_bulk_reservation);

        let publisher = RecordingPublisher::default();
        let (path, store) =
            store_with_publisher("construction-bulk-capacity-fallback", publisher.clone());
        let mut construction = PatriciaIndexConstruction {
            resident_budget_bytes: minimum_bulk_reservation - 1,
            ..PatriciaIndexConstruction::default()
        };
        let root = store
            .construction_insert_many_bulk(&mut construction, PatriciaIndexRoot::empty(), &records)
            .unwrap();
        assert!(
            publisher.take().is_empty(),
            "legacy fallback must stage its leaf before any bulk publication"
        );
        assert!(construction.staged.nodes.contains_key(&root.digest()));
        construction.set_live_roots([root]);
        construction.checkpoint([root]);
        store.finish_construction(&mut construction).unwrap();
        assert_eq!(all_records(&store, root), records);
        assert!(construction.peak_resident_bytes <= construction.resident_budget_bytes);

        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn construction_budget_is_explicitly_bounded_and_overflow_is_refused() {
        assert_eq!(
            PatriciaIndexConstruction::default().resident_budget_bytes,
            DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES
        );
        let enlarged = PatriciaIndexConstruction::with_resident_budget(
            MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
        )
        .unwrap();
        assert_eq!(
            enlarged.bulk_record_limit(),
            MAX_PATRICIA_CONSTRUCTION_BULK_RECORDS,
            "the format pack ceiling must remain binding even when more construction memory is available"
        );
        assert!(
            PatriciaIndexConstruction::default().bulk_record_limit()
                <= enlarged.bulk_record_limit()
        );
        assert!(PatriciaIndexConstruction::with_resident_budget(
            DEFAULT_PATRICIA_CONSTRUCTION_RESIDENT_BYTES - 1,
        )
        .is_err());
        assert!(PatriciaIndexConstruction::with_resident_budget(
            MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES + 1,
        )
        .is_err());
        assert!(matches!(
            construction_bulk_reservation(usize::MAX),
            Err(PatriciaError::Malformed)
        ));
    }

    #[test]
    fn insertion_is_canonical_and_historical_roots_remain_queryable() {
        let (path, store) = store("canonical");
        let records = BTreeMap::from([
            (b"a/one".to_vec(), b"1".to_vec()),
            (b"a/two".to_vec(), b"2".to_vec()),
            (b"b/one".to_vec(), b"3".to_vec()),
        ]);
        let forward = store
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let reverse =
            records
                .iter()
                .rev()
                .fold(PatriciaIndexRoot::empty(), |root, (key, value)| {
                    store
                        .insert_many(root, &BTreeMap::from([(key.clone(), value.clone())]))
                        .unwrap()
                });
        assert_eq!(forward, reverse);
        assert_eq!(
            store.lookup_prefix(forward, b"a/").unwrap(),
            BTreeMap::from([
                (b"a/one".to_vec(), b"1".to_vec()),
                (b"a/two".to_vec(), b"2".to_vec()),
            ])
        );

        let advanced = store
            .insert_many(
                forward,
                &BTreeMap::from([(b"a/one".to_vec(), b"new".to_vec())]),
            )
            .unwrap();
        assert_eq!(
            store.lookup(forward, b"a/one").unwrap(),
            Some(b"1".to_vec())
        );
        assert_eq!(
            store.lookup(advanced, b"a/one").unwrap(),
            Some(b"new".to_vec())
        );
        assert!(store.stats().reads < 100);
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn frozen_v1_bytes_roots_filenames_and_publication_order_are_unchanged() {
        let path = std::env::temp_dir().join(format!("tine-patricia-frozen-v1-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let root_dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        ensure_directory_nofollow(&root_dir, "nodes").unwrap();
        let publisher = RecordingPublisher::default();
        let store = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            publisher.clone(),
        );
        let inserted = store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([
                    (b"alpha".to_vec(), b"one".to_vec()),
                    (b"beta".to_vec(), b"two".to_vec()),
                    (b"gamma".to_vec(), b"three".to_vec()),
                ]),
            )
            .unwrap();
        assert_eq!(
            inserted.digest().to_string(),
            "9976fbe04eaa635f6abadec835be4dc410cb8b12b0ee519addf5b1579aa32d84"
        );
        assert_eq!(
            publisher.take(),
            vec![
                ("9976fbe04eaa635f6abadec835be4dc410cb8b12b0ee519addf5b1579aa32d84.patricia-node".into(), "010101600540303932643132633264383739323862613035623036353837653662663865306531626564333765353430613764313336303531653065316537303835666363304061356531323164316632623032383464313738383663353364313266313339376363333832613930376264656335633032356365333934313736323531626264".into()),
                ("a5e121d1f2b0284d17886c53d12f1397cc382a907bdec5c025ce394176251bbd.patricia-node".into(), "00010567616d6d61057468726565".into()),
                ("092d12c2d87928ba05b06587e6bf8e0e1bed37e540a7d136051e0e1e7085fcc0.patricia-node".into(), "010101600640656165323930343739306633613564623638383363383464333863613262343436353630333434326433373861386232646162363235353834353031396435324064303230636530323136326435306532643433393233376439316465633637383431316265616331323833346634363438646536653165303035353565326265".into()),
                ("d020ce02162d50e2d439237d91dec678411beac12834f4648de6e1e00555e2be.patricia-node".into(), "000104626574610374776f".into()),
                ("eae2904790f3a5db6883c84d38ca2b4465603442d378a8b2dab6255845019d52.patricia-node".into(), "000105616c706861036f6e65".into()),
            ]
        );

        let replaced = store
            .insert_many(
                inserted,
                &BTreeMap::from([(b"beta".to_vec(), b"TWO".to_vec())]),
            )
            .unwrap();
        assert_eq!(
            replaced.digest().to_string(),
            "f91bb4967d7676181f9a437cf4992d490cdfe2d6b55b6b32bc492d71d255c9ec"
        );
        assert_eq!(
            publisher.take(),
            vec![
                ("f91bb4967d7676181f9a437cf4992d490cdfe2d6b55b6b32bc492d71d255c9ec.patricia-node".into(), "010101600540616232333863373162643136373330363836626133623531636661646137333838356362373733376330626562633930303232613139356239383633363432324061356531323164316632623032383464313738383663353364313266313339376363333832613930376264656335633032356365333934313736323531626264".into()),
                ("ab238c71bd16730686ba3b51cfada73885cb7737c0bebc90022a195b98636422.patricia-node".into(), "010101600640656165323930343739306633613564623638383363383464333863613262343436353630333434326433373861386232646162363235353834353031396435324031343331386665656330393661643464626531376235656636626132643537656535653030633562643964613230356433376565646663653536616561323734".into()),
                ("14318feec096ad4dbe17b5ef6ba2d57ee5e00c5bd9da205d37eedfce56aea274.patricia-node".into(), "000104626574610354574f".into()),
            ]
        );

        let removed = store
            .remove_many(replaced, &[b"alpha".to_vec(), b"gamma".to_vec()])
            .unwrap();
        assert_eq!(
            removed.digest().to_string(),
            "14318feec096ad4dbe17b5ef6ba2d57ee5e00c5bd9da205d37eedfce56aea274"
        );
        assert_eq!(
            publisher.take(),
            vec![("0c2eb300bc2b7a5cce7c41b74f3cf134367f4689305f5a3c2faefa3f44239cfb.patricia-node".into(), "010101600540313433313866656563303936616434646265313762356566366261326435376565356530306335626439646132303564333765656466636535366165613237344061356531323164316632623032383464313738383663353364313266313339376363333832613930376264656335633032356365333934313736323531626264".into())]
        );

        let reopened = PatriciaIndexStore::new(
            open_dir_nofollow(&root_dir, "nodes").unwrap(),
            ExactPublisher,
        );
        assert_eq!(
            reopened.lookup(removed, b"beta").unwrap(),
            Some(b"TWO".to_vec())
        );
        assert_eq!(reopened.lookup(removed, b"alpha").unwrap(), None);
        drop(reopened);
        drop(store);
        drop(root_dir);
        fs::remove_dir_all(path).unwrap();
    }

    fn hex_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn duplicate_heavy_prefix_is_sharded_beyond_the_old_record_ceiling() {
        const INTRODUCTIONS: usize = 1_200;

        let (path, store) = store("duplicate-heavy");
        let prefix = [0x5a; 16];
        let records = (0..INTRODUCTIONS)
            .map(|index| {
                let mut key = prefix.to_vec();
                key.extend_from_slice(&(index as u128).to_be_bytes());
                (key, vec![index as u8; 96])
            })
            .collect::<BTreeMap<_, _>>();
        assert!(
            records.values().map(Vec::len).sum::<usize>() > 64 * 1024,
            "fixture must exceed the former monolithic record ceiling"
        );
        let root = store
            .insert_many(PatriciaIndexRoot::empty(), &records)
            .unwrap();
        let before = store.stats();
        let found = store.lookup_prefix(root, &prefix).unwrap();
        let after = store.stats();
        assert_eq!(found, records);
        assert!(
            after.reads - before.reads <= INTRODUCTIONS * 3,
            "prefix lookup must read only the participant subtree"
        );
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_truncated_tampered_and_noncanonical_nodes_refuse() {
        let (path, store) = store("corrupt-bytes");
        let root = store
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(b"key".to_vec(), b"value".to_vec())]),
            )
            .unwrap();
        let node_path = path.join("nodes").join(node_filename(root.digest()));
        let original = fs::read(&node_path).unwrap();

        fs::write(&node_path, &original[..original.len() - 1]).unwrap();
        assert!(matches!(
            store.lookup(root, b"key"),
            Err(PatriciaError::PathMismatch(digest)) if digest == root.digest()
        ));

        fs::write(&node_path, &original).unwrap();
        let mut tampered = original.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        fs::write(&node_path, &tampered).unwrap();
        assert!(matches!(
            store.lookup(root, b"key"),
            Err(PatriciaError::PathMismatch(digest)) if digest == root.digest()
        ));

        let mut noncanonical = original;
        noncanonical.push(0);
        let noncanonical_digest = ContentDigest::of(&noncanonical);
        fs::write(
            path.join("nodes").join(node_filename(noncanonical_digest)),
            noncanonical,
        )
        .unwrap();
        assert!(matches!(
            store.lookup(PatriciaIndexRoot::from_digest(noncanonical_digest), b"key"),
            Err(PatriciaError::Malformed)
        ));

        let missing = ContentDigest::of(b"missing Patricia node");
        assert!(matches!(
            store.validate_root(PatriciaIndexRoot::from_digest(missing)),
            Err(PatriciaError::MissingNode(digest)) if digest == missing
        ));
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn key_and_value_limits_refuse_without_publication() {
        let (path, store) = store("record-limits");
        let root = PatriciaIndexRoot::empty();
        for records in [
            BTreeMap::from([(Vec::new(), b"value".to_vec())]),
            BTreeMap::from([(vec![0; MAX_KEY_BYTES + 1], b"value".to_vec())]),
            BTreeMap::from([(b"key".to_vec(), Vec::new())]),
            BTreeMap::from([(b"key".to_vec(), vec![0; MAX_VALUE_BYTES + 1])]),
        ] {
            assert!(matches!(
                store.insert_many(root, &records),
                Err(PatriciaError::Malformed)
            ));
        }
        assert_eq!(store.stats().writes, 0);
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn repeated_nonprogressing_branches_and_wrong_path_leaves_refuse() {
        let (path, store) = store("malformed-paths");
        let key = [0_u8];
        let left = publish_leaf(&store, &key);
        let right = publish_leaf(&store, &[0x80]);

        let repeated_child = publish_branch(&store, &key, 0, left, right);
        let repeated_root =
            PatriciaIndexRoot::from_digest(publish_branch(&store, &key, 0, repeated_child, right));
        assert_point_traversals_reject(&store, repeated_root, &key);

        let shallower_child = publish_branch(&store, &key, 1, left, right);
        let nonprogressing_root =
            PatriciaIndexRoot::from_digest(publish_branch(&store, &key, 2, shallower_child, right));
        assert_point_traversals_reject(&store, nonprogressing_root, &key);

        let wrong_direction_leaf = publish_leaf(&store, &[0x40]);
        let wrong_leaf_root = PatriciaIndexRoot::from_digest(publish_branch(
            &store,
            &key,
            1,
            wrong_direction_leaf,
            right,
        ));
        assert_point_traversals_reject(&store, wrong_leaf_root, &key);

        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn overdeep_content_addressed_branch_chain_refuses_within_key_bound() {
        let (path, store) = store("overdeep");
        let key = vec![0_u8; MAX_KEY_BYTES];
        let matching_leaf = publish_leaf(&store, &key);
        let other_leaf = publish_leaf(&store, &vec![0xff; MAX_KEY_BYTES]);

        let mut chain = publish_branch(&store, &key, MAX_KEY_BITS - 1, matching_leaf, other_leaf);
        for split in (0..MAX_KEY_BITS).rev() {
            chain = publish_branch(&store, &key, split, chain, other_leaf);
        }
        let root = PatriciaIndexRoot::from_digest(chain);
        let hard_bound = traversal_node_budget(key.len()).unwrap();

        let before = store.stats();
        assert!(matches!(
            store.lookup(root, &key),
            Err(PatriciaError::Malformed)
        ));
        let after_lookup = store.stats();
        assert!(after_lookup.reads - before.reads <= hard_bound);

        assert!(matches!(
            store.insert_many(
                root,
                &BTreeMap::from([(key.clone(), b"replacement".to_vec())])
            ),
            Err(PatriciaError::Malformed)
        ));
        let after_insert = store.stats();
        assert!(after_insert.reads - after_lookup.reads <= hard_bound);

        assert!(matches!(
            store.lookup_prefix(root, &key),
            Err(PatriciaError::Malformed)
        ));
        let after_prefix = store.stats();
        assert!(after_prefix.reads - after_insert.reads <= hard_bound);

        drop(store);
        fs::remove_dir_all(path).unwrap();
    }
}
