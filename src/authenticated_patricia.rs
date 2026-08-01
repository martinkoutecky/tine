#![allow(clippy::result_large_err)]

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::ContentDigest;
use crate::filesystem::{read_optional_regular, FilesystemError};

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
}

#[derive(Debug)]
pub enum PatriciaError {
    Filesystem(FilesystemError),
    Publication(PatriciaPublicationError),
    MissingNode(ContentDigest),
    PathMismatch(ContentDigest),
    Malformed,
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
// boundaries. Once this conservative encoded-size budget is crossed, nodes
// reachable from construction authority are immutable-published and the
// buffer is cleared. This is a single-use construction buffer, not a second
// persistent cache.
pub const MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES: usize = 64 * 1024 * 1024;

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
    counters: Counters,
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

#[derive(Debug, Default)]
struct StagedNodes {
    nodes: BTreeMap<ContentDigest, Node>,
    encoded_bytes: usize,
}

impl StagedNodes {
    fn stage(&mut self, node: Node) -> Result<ContentDigest, PatriciaError> {
        validate_node(&node)?;
        let bytes = postcard::to_allocvec(&node).map_err(|_| PatriciaError::Malformed)?;
        if bytes.len() as u64 > MAX_NODE_BYTES {
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
    flushes: usize,
    staged_nodes_at_publication: usize,
    published_staged_nodes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatriciaIndexConstructionStats {
    pub peak_resident_bytes: usize,
    pub flushes: usize,
}

impl Default for PatriciaIndexConstruction {
    fn default() -> Self {
        Self {
            staged: StagedNodes::default(),
            checkpoint_roots: BTreeSet::new(),
            live_roots: BTreeSet::new(),
            resident_budget_bytes: MAX_PATRICIA_CONSTRUCTION_RESIDENT_BYTES,
            peak_resident_bytes: 0,
            flushes: 0,
            staged_nodes_at_publication: 0,
            published_staged_nodes: 0,
        }
    }
}

impl PatriciaIndexConstruction {
    pub fn checkpoint(&mut self, roots: impl IntoIterator<Item = PatriciaIndexRoot>) {
        self.checkpoint_roots
            .extend(roots.into_iter().map(PatriciaIndexRoot::digest));
    }

    /// Replaces the complete set of roots that the caller currently treats as
    /// live construction authority. Historical roots are retained separately
    /// with [`Self::checkpoint`].
    pub fn set_live_roots(&mut self, roots: impl IntoIterator<Item = PatriciaIndexRoot>) {
        self.live_roots = roots.into_iter().map(PatriciaIndexRoot::digest).collect();
    }

    fn note_residency(&mut self) {
        self.peak_resident_bytes = self.peak_resident_bytes.max(self.staged.encoded_bytes);
    }

    fn flush_if_over_budget(
        &mut self,
        store: &PatriciaIndexStore,
        in_progress_root: PatriciaIndexRoot,
    ) -> Result<(), PatriciaError> {
        self.note_residency();
        if self.staged.encoded_bytes > self.resident_budget_bytes {
            let mut roots = self.checkpoint_roots.clone();
            roots.extend(self.live_roots.iter().copied());
            roots.insert(in_progress_root.digest());
            let published = store.publish_staged_roots(&roots, &self.staged)?;
            self.staged_nodes_at_publication = self
                .staged_nodes_at_publication
                .saturating_add(self.staged.nodes.len());
            self.published_staged_nodes = self.published_staged_nodes.saturating_add(published);
            self.staged.clear();
            self.flushes = self.flushes.saturating_add(1);
        }
        Ok(())
    }

    pub fn stats(&self) -> PatriciaIndexConstructionStats {
        PatriciaIndexConstructionStats {
            peak_resident_bytes: self.peak_resident_bytes,
            flushes: self.flushes,
        }
    }
}

impl PatriciaIndexStore {
    pub fn new(nodes: Dir, publisher: impl PatriciaNodePublisher + 'static) -> Self {
        Self {
            nodes,
            publisher: Box::new(publisher),
            counters: Counters::default(),
        }
    }

    pub fn with_publisher(
        &self,
        publisher: impl PatriciaNodePublisher + 'static,
    ) -> Result<Self, PatriciaError> {
        Ok(Self {
            nodes: self.nodes.try_clone()?,
            publisher: Box::new(publisher),
            counters: Counters::default(),
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

    pub fn construction_insert_many(
        &self,
        construction: &mut PatriciaIndexConstruction,
        mut root: PatriciaIndexRoot,
        records: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<PatriciaIndexRoot, PatriciaError> {
        for (key, value) in records {
            validate_record(key, value)?;
            root = PatriciaIndexRoot(self.insert_staged(
                root,
                key,
                value,
                &mut construction.staged,
            )?);
            construction.flush_if_over_budget(self, root)?;
        }
        Ok(root)
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
            root = self.remove_constructed(construction, root, key)?;
            construction.flush_if_over_budget(self, root)?;
        }
        Ok(root)
    }

    pub fn finish_construction(
        &self,
        construction: &mut PatriciaIndexConstruction,
    ) -> Result<(), PatriciaError> {
        construction.note_residency();
        let mut roots = construction.checkpoint_roots.clone();
        roots.extend(construction.live_roots.iter().copied());
        let published = self.publish_staged_roots(&roots, &construction.staged)?;
        construction.staged_nodes_at_publication = construction
            .staged_nodes_at_publication
            .saturating_add(construction.staged.nodes.len());
        construction.published_staged_nodes = construction
            .published_staged_nodes
            .saturating_add(published);
        construction.staged.clear();
        Ok(())
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

    fn publish_staged_reachable(
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
            self.publish_node(node)?;
        }
        Ok(())
    }

    fn publish_staged_roots(
        &self,
        roots: &BTreeSet<ContentDigest>,
        staged: &StagedNodes,
    ) -> Result<usize, PatriciaError> {
        let mut pending = roots.iter().copied().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        let mut published = 0_usize;
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
            self.publish_node(node)?;
            published = published.saturating_add(1);
        }
        Ok(published)
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
        let bytes =
            read_optional_regular(&self.nodes, &node_filename(digest), MAX_NODE_BYTES, None)?
                .ok_or(PatriciaError::MissingNode(digest))?;
        if ContentDigest::of(&bytes) != digest {
            return Err(PatriciaError::PathMismatch(digest));
        }
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
    use std::sync::{Arc, Mutex};

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
            fs::read_dir(loose_path.join("nodes")).unwrap().count(),
            reachable.len()
        );

        drop(reopened);
        drop(pack_dir);
        drop(loose);
        fs::remove_dir_all(loose_path).unwrap();
        fs::remove_dir_all(pack_path).unwrap();
    }

    #[test]
    fn construction_flushes_only_checkpoint_live_and_in_progress_roots() {
        const RESIDENT_BUDGET: usize = 4 * 1024;
        const ROUNDS: usize = 48;

        let (baseline_path, baseline) = store("construction-baseline");
        let (construction_path, constructed) = store("construction-reachable");
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
        let mut removal_flushes = 0_usize;
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
                            all_records(&constructed, live_roots[index]),
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
                    removal_flushes = removal_flushes.saturating_add(1);
                    for index in 0..3 {
                        assert_eq!(
                            all_records(&constructed, live_roots[index]),
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
        assert!(removal_flushes > 0, "fixture must flush during a removal");
        assert!(
            construction.peak_resident_bytes > RESIDENT_BUDGET,
            "the check must retain the existing post-insert overshoot"
        );
        assert!(
            construction.peak_resident_bytes <= RESIDENT_BUDGET + 4 * 1024,
            "the fixture must stay within one insert's old overshoot"
        );
        assert!(
            !construction.staged.nodes.is_empty(),
            "fixture must leave reachable work for finalization"
        );
        assert!(
            construction.published_staged_nodes < construction.staged_nodes_at_publication,
            "budget flushes must omit transient staged path copies before finalization"
        );
        let publications_before_finish = construction.published_staged_nodes;
        constructed.finish_construction(&mut construction).unwrap();
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
            "optimized construction must create fewer immutable node files"
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
