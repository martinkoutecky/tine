use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::hot_engine::{FatalEvidenceHandle, ImmutableHomeConflict};
use super::scratch_store::{ScratchBlobRef, ScratchPageKind, ScratchRoots, ScratchStore};
use super::{BlockId, ContentDigest};

const CONFLICT_INDEX_SCHEMA_VERSION: u32 = 3;
const DIRECTORY_KEY: &[u8] = b"tine/oplog/conflict-directory/v3";
const MAX_DIRECTORY_NODE_ENTRIES: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictRecord {
    schema_version: u32,
    conflict: ImmutableHomeConflict,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictDirectory {
    schema_version: u32,
    root: Option<ScratchBlobRef>,
    conflicting_block_count: u64,
    claim_count: u64,
}

impl Default for ConflictDirectory {
    fn default() -> Self {
        Self {
            schema_version: CONFLICT_INDEX_SCHEMA_VERSION,
            root: None,
            conflicting_block_count: 0,
            claim_count: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictDirectoryChild {
    max_key: BlockId,
    node: ScratchBlobRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ConflictDirectoryNode {
    Leaf {
        keys: Vec<BlockId>,
    },
    Branch {
        children: Vec<ConflictDirectoryChild>,
    },
}

pub(crate) fn upsert_conflict(
    store: &ScratchStore,
    roots: &ScratchRoots,
    handle: Option<FatalEvidenceHandle>,
    conflict: ImmutableHomeConflict,
) -> Result<(ScratchRoots, FatalEvidenceHandle), EvidenceIndexError> {
    validate_handle_root(roots, handle)?;
    let key = conflict_key(conflict.block_id());
    let prior = store
        .lookup(&roots.conflict_root, ScratchPageKind::Conflict, &key)?
        .map(|bytes| decode_record(conflict.block_id(), &bytes))
        .transpose()?;
    let record = ConflictRecord {
        schema_version: CONFLICT_INDEX_SCHEMA_VERSION,
        conflict,
    };
    validate_record(&record)?;

    let mut directory = load_directory(store, roots)?;
    validate_directory(&directory)?;
    validate_directory_handle(&directory, handle)?;
    if prior.is_none() {
        directory.root = Some(insert_conflict_key(
            store,
            directory.root.as_ref(),
            record.conflict.block_id(),
        )?);
        directory.conflicting_block_count = directory
            .conflicting_block_count
            .checked_add(1)
            .ok_or(EvidenceIndexError::Malformed)?;
    }
    let prior_claims = prior
        .as_ref()
        .map(|record| record.conflict.claims().len() as u64)
        .unwrap_or(0);
    let new_claims = record.conflict.claims().len() as u64;
    directory.claim_count = directory
        .claim_count
        .checked_sub(prior_claims)
        .and_then(|claims| claims.checked_add(new_claims))
        .ok_or(EvidenceIndexError::Misbound)?;

    let mut changes = BTreeMap::from([(key, Some(encode_canonical(&record)?))]);
    changes.insert(DIRECTORY_KEY.to_vec(), Some(encode_canonical(&directory)?));
    let mut next = roots.clone();
    next.conflict_root =
        store.insert_many(&next.conflict_root, ScratchPageKind::Conflict, &changes)?;
    let handle = handle_for_roots(&next, &directory)?;
    Ok((next, handle))
}

pub(crate) fn page_conflicts(
    store: &ScratchStore,
    roots: &ScratchRoots,
    handle: FatalEvidenceHandle,
    after: Option<BlockId>,
    limit: usize,
) -> Result<(Vec<ImmutableHomeConflict>, Option<BlockId>), EvidenceIndexError> {
    if limit == 0 || limit > MAX_DIRECTORY_NODE_ENTRIES {
        return Err(EvidenceIndexError::InvalidPageLimit);
    }
    validate_handle_root(roots, Some(handle))?;
    let directory = load_directory(store, roots)?;
    validate_directory(&directory)?;
    validate_directory_handle(&directory, Some(handle))?;
    let mut keys = Vec::with_capacity(limit.saturating_add(1));
    if let Some(root) = &directory.root {
        collect_keys_after(store, root, after, limit.saturating_add(1), &mut keys)?;
    }
    let has_more = keys.len() > limit;
    if has_more {
        keys.pop();
    }
    let conflicts = keys
        .iter()
        .map(|block_id| {
            let key = conflict_key(*block_id);
            let bytes = store
                .lookup(&roots.conflict_root, ScratchPageKind::Conflict, &key)?
                .ok_or(EvidenceIndexError::Misbound)?;
            Ok(decode_record(*block_id, &bytes)?.conflict)
        })
        .collect::<Result<Vec<_>, EvidenceIndexError>>()?;
    let next_after = has_more.then(|| *keys.last().expect("nonempty bounded page"));
    Ok((conflicts, next_after))
}

fn validate_handle_root(
    roots: &ScratchRoots,
    handle: Option<FatalEvidenceHandle>,
) -> Result<(), EvidenceIndexError> {
    let Some(handle) = handle else {
        return Ok(());
    };
    let root_bytes =
        postcard::to_allocvec(&roots.conflict_root).map_err(|_| EvidenceIndexError::Malformed)?;
    if ContentDigest::of(&root_bytes) != handle.conflict_root {
        return Err(EvidenceIndexError::Misbound);
    }
    Ok(())
}

fn handle_for_roots(
    roots: &ScratchRoots,
    directory: &ConflictDirectory,
) -> Result<FatalEvidenceHandle, EvidenceIndexError> {
    let root_bytes =
        postcard::to_allocvec(&roots.conflict_root).map_err(|_| EvidenceIndexError::Malformed)?;
    let conflict_root = ContentDigest::of(&root_bytes);
    let canonical_digest = ContentDigest::of(
        &postcard::to_allocvec(&(
            conflict_root,
            directory.conflicting_block_count,
            directory.claim_count,
        ))
        .map_err(|_| EvidenceIndexError::Malformed)?,
    );
    Ok(FatalEvidenceHandle {
        conflict_root,
        conflicting_block_count: directory.conflicting_block_count,
        claim_count: directory.claim_count,
        canonical_digest,
    })
}

fn load_directory(
    store: &ScratchStore,
    roots: &ScratchRoots,
) -> Result<ConflictDirectory, EvidenceIndexError> {
    store
        .lookup(
            &roots.conflict_root,
            ScratchPageKind::Conflict,
            DIRECTORY_KEY,
        )?
        .map(|bytes| decode_canonical(&bytes))
        .transpose()
        .map(|directory| directory.unwrap_or_default())
}

fn validate_directory(directory: &ConflictDirectory) -> Result<(), EvidenceIndexError> {
    if directory.schema_version != CONFLICT_INDEX_SCHEMA_VERSION
        || (directory.root.is_none()
            && (directory.conflicting_block_count != 0 || directory.claim_count != 0))
        || (directory.root.is_some()
            && (directory.conflicting_block_count == 0 || directory.claim_count < 2))
    {
        return Err(EvidenceIndexError::Malformed);
    }
    Ok(())
}

fn validate_directory_handle(
    directory: &ConflictDirectory,
    handle: Option<FatalEvidenceHandle>,
) -> Result<(), EvidenceIndexError> {
    if let Some(handle) = handle {
        if directory.conflicting_block_count != handle.conflicting_block_count
            || directory.claim_count != handle.claim_count
        {
            return Err(EvidenceIndexError::Misbound);
        }
    } else if directory.root.is_some() {
        return Err(EvidenceIndexError::Misbound);
    }
    Ok(())
}

fn insert_conflict_key(
    store: &ScratchStore,
    root: Option<&ScratchBlobRef>,
    key: BlockId,
) -> Result<ScratchBlobRef, EvidenceIndexError> {
    let children = match root {
        None => vec![child_for_node(
            store,
            write_node(store, &ConflictDirectoryNode::Leaf { keys: vec![key] })?,
        )?],
        Some(root) => insert_node(store, root, key)?,
    };
    if children.len() == 1 {
        return Ok(children.into_iter().next().expect("one root child").node);
    }
    write_node(store, &ConflictDirectoryNode::Branch { children })
}

fn insert_node(
    store: &ScratchStore,
    node_ref: &ScratchBlobRef,
    key: BlockId,
) -> Result<Vec<ConflictDirectoryChild>, EvidenceIndexError> {
    match read_node(store, node_ref)? {
        ConflictDirectoryNode::Leaf { mut keys } => {
            match keys.binary_search(&key) {
                Ok(_) => return Ok(vec![child_for_node(store, node_ref.clone())?]),
                Err(index) => keys.insert(index, key),
            }
            split_leaf(store, keys)
        }
        ConflictDirectoryNode::Branch { mut children } => {
            let index = children
                .iter()
                .position(|child| key <= child.max_key)
                .unwrap_or_else(|| children.len().saturating_sub(1));
            let replacements = insert_node(store, &children[index].node, key)?;
            children.splice(index..=index, replacements);
            split_branch(store, children)
        }
    }
}

fn split_leaf(
    store: &ScratchStore,
    keys: Vec<BlockId>,
) -> Result<Vec<ConflictDirectoryChild>, EvidenceIndexError> {
    if keys.len() <= MAX_DIRECTORY_NODE_ENTRIES {
        let node = write_node(store, &ConflictDirectoryNode::Leaf { keys })?;
        return Ok(vec![child_for_node(store, node)?]);
    }
    let split_at = keys.len() / 2;
    let left = write_node(
        store,
        &ConflictDirectoryNode::Leaf {
            keys: keys[..split_at].to_vec(),
        },
    )?;
    let right = write_node(
        store,
        &ConflictDirectoryNode::Leaf {
            keys: keys[split_at..].to_vec(),
        },
    )?;
    Ok(vec![
        child_for_node(store, left)?,
        child_for_node(store, right)?,
    ])
}

fn split_branch(
    store: &ScratchStore,
    children: Vec<ConflictDirectoryChild>,
) -> Result<Vec<ConflictDirectoryChild>, EvidenceIndexError> {
    if children.len() <= MAX_DIRECTORY_NODE_ENTRIES {
        let node = write_node(store, &ConflictDirectoryNode::Branch { children })?;
        return Ok(vec![child_for_node(store, node)?]);
    }
    let split_at = children.len() / 2;
    let left = write_node(
        store,
        &ConflictDirectoryNode::Branch {
            children: children[..split_at].to_vec(),
        },
    )?;
    let right = write_node(
        store,
        &ConflictDirectoryNode::Branch {
            children: children[split_at..].to_vec(),
        },
    )?;
    Ok(vec![
        child_for_node(store, left)?,
        child_for_node(store, right)?,
    ])
}

fn child_for_node(
    store: &ScratchStore,
    node: ScratchBlobRef,
) -> Result<ConflictDirectoryChild, EvidenceIndexError> {
    let max_key = node_max_key(&read_node(store, &node)?)?;
    Ok(ConflictDirectoryChild { max_key, node })
}

fn write_node(
    store: &ScratchStore,
    node: &ConflictDirectoryNode,
) -> Result<ScratchBlobRef, EvidenceIndexError> {
    validate_node(node)?;
    store
        .append_blob(&encode_canonical(node)?)
        .map_err(Into::into)
}

fn read_node(
    store: &ScratchStore,
    node_ref: &ScratchBlobRef,
) -> Result<ConflictDirectoryNode, EvidenceIndexError> {
    let node = decode_canonical(&store.read_blob(node_ref)?)?;
    validate_node(&node)?;
    Ok(node)
}

fn validate_node(node: &ConflictDirectoryNode) -> Result<(), EvidenceIndexError> {
    match node {
        ConflictDirectoryNode::Leaf { keys } => validate_sorted_keys(keys),
        ConflictDirectoryNode::Branch { children } => {
            if children.len() < 2 || children.len() > MAX_DIRECTORY_NODE_ENTRIES {
                return Err(EvidenceIndexError::Malformed);
            }
            validate_sorted_keys(
                &children
                    .iter()
                    .map(|child| child.max_key)
                    .collect::<Vec<_>>(),
            )
        }
    }
}

fn validate_sorted_keys(keys: &[BlockId]) -> Result<(), EvidenceIndexError> {
    if keys.is_empty()
        || keys.len() > MAX_DIRECTORY_NODE_ENTRIES
        || keys.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(EvidenceIndexError::Malformed);
    }
    Ok(())
}

fn node_max_key(node: &ConflictDirectoryNode) -> Result<BlockId, EvidenceIndexError> {
    match node {
        ConflictDirectoryNode::Leaf { keys } => keys.last().copied(),
        ConflictDirectoryNode::Branch { children } => children.last().map(|child| child.max_key),
    }
    .ok_or(EvidenceIndexError::Malformed)
}

fn collect_keys_after(
    store: &ScratchStore,
    node_ref: &ScratchBlobRef,
    after: Option<BlockId>,
    take: usize,
    keys: &mut Vec<BlockId>,
) -> Result<(), EvidenceIndexError> {
    if keys.len() >= take {
        return Ok(());
    }
    match read_node(store, node_ref)? {
        ConflictDirectoryNode::Leaf { keys: leaf } => {
            keys.extend(
                leaf.into_iter()
                    .filter(|key| after.is_none_or(|after| *key > after))
                    .take(take.saturating_sub(keys.len())),
            );
        }
        ConflictDirectoryNode::Branch { children } => {
            for child in children {
                if after.is_some_and(|after| child.max_key <= after) {
                    continue;
                }
                collect_keys_after(store, &child.node, after, take, keys)?;
                if keys.len() >= take {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn decode_record(
    expected_block_id: BlockId,
    bytes: &[u8],
) -> Result<ConflictRecord, EvidenceIndexError> {
    let record: ConflictRecord = decode_canonical(bytes)?;
    validate_record(&record)?;
    if record.conflict.block_id() != expected_block_id {
        return Err(EvidenceIndexError::Misbound);
    }
    Ok(record)
}

fn validate_record(record: &ConflictRecord) -> Result<(), EvidenceIndexError> {
    if record.schema_version != CONFLICT_INDEX_SCHEMA_VERSION
        || record.conflict.claims().len() < 2
        || record
            .conflict
            .claims()
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(EvidenceIndexError::Malformed);
    }
    Ok(())
}

fn conflict_key(block_id: BlockId) -> Vec<u8> {
    block_id.as_uuid().as_bytes().to_vec()
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, EvidenceIndexError> {
    postcard::to_allocvec(value).map_err(|_| EvidenceIndexError::Malformed)
}

fn decode_canonical<T: for<'de> Deserialize<'de> + Serialize>(
    bytes: &[u8],
) -> Result<T, EvidenceIndexError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| EvidenceIndexError::Malformed)?;
    if encode_canonical(&value)? != bytes {
        return Err(EvidenceIndexError::Malformed);
    }
    Ok(value)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceIndexError {
    Scratch(String),
    InvalidPageLimit,
    Malformed,
    Misbound,
}

impl fmt::Display for EvidenceIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scratch(error) => write!(f, "conflict scratch index failed: {error}"),
            Self::InvalidPageLimit => f.write_str("fatal-evidence page limit is outside 1..=32"),
            Self::Malformed => f.write_str("malformed conflict evidence record"),
            Self::Misbound => f.write_str("misbound conflict evidence root"),
        }
    }
}

impl std::error::Error for EvidenceIndexError {}

impl From<super::scratch_store::ScratchError> for EvidenceIndexError {
    fn from(error: super::scratch_store::ScratchError) -> Self {
        Self::Scratch(error.to_string())
    }
}
