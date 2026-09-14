//! The path-copied authenticated treap that backs the receiver-absence index,
//! and nothing else.
//!
//! ## Why this lives here
//!
//! `tine-storage` v0.23.0 retired the sealed authenticated treap: the sealed
//! accepted history and the cold-history locator index are immutable sorted
//! tables now (`sealed-v3`, P4c2). Receiver absence never was sealed accepted
//! history — it is a disposable per-workspace summary rebuilt from retained
//! receipts (D-3) that merely *borrowed* the same primitive for its three
//! composed maps. Retiring the primitive in the crate therefore leaves this one
//! consumer without an implementation, and it needs exactly the shape it always
//! had: incremental single-key upserts against a durable node namespace, not
//! cut-based table publication.
//!
//! So this module is a transcription of `tine-storage` v0.22.0's
//! `SealedAuthenticatedMapNodeV2` codec and its writer/reader traversal, moved
//! to its one remaining caller and narrowed to it. The *digest algebra* is NOT
//! duplicated: `authenticated_map_node_digest`, `authenticated_map_priority`,
//! `authenticated_map_priority_order` and `AuthenticatedMapRootV1` are still
//! `tine-storage`'s, so a root computed here is bit-for-bit the root the crate
//! would have computed and `authenticated_map_root` remains a usable oracle.
//!
//! ## Boundary
//!
//! This is NOT a second sealed-index format (D-1). It may never be used by the
//! sealed accepted history, the checkpoint generation or cold history; those
//! have exactly one current representation, the sorted tables of `sealed-v3`.
//! `sealed_index_source_guard` in `checkpoint_generation.rs` fails if this
//! module gains a second consumer.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use tine_storage::sealed_accepted_index::{
    authenticated_map_empty_digest, authenticated_map_node_digest,
    authenticated_map_priority_order, AuthenticatedMapKey, AuthenticatedMapLinkV1,
    AuthenticatedMapRootV1, SealedAcceptedIndexError,
};

use super::ContentDigest;

/// Retained from the retired `tine_storage::formats` constant of the same name,
/// so this index's node bytes are unchanged by the move.
const MAP_NODE_SCHEMA_VERSION: u32 = 3;

/// Retained from the retired `MAX_ACCEPTED_INDEX_DEPTH`.
const MAX_MAP_DEPTH: usize = 256;

/// The object-kind code the node namespace has always used for a map node.
pub(crate) const MAP_NODE_KIND_CODE: u8 = 1;

fn corrupt(message: impl Into<String>) -> SealedAcceptedIndexError {
    SealedAcceptedIndexError::Corrupt(message.into())
}

/// The node namespace this index reads and writes.
///
/// Reads may see not-yet-durable staged bytes; publication only stages, exactly
/// as before. Nothing here names a file: the caller owns the naming.
pub(crate) trait MapNodeObjects {
    fn read_map_node_object(
        &self,
        address: ContentDigest,
    ) -> Result<Option<Vec<u8>>, SealedAcceptedIndexError>;

    fn publish_map_node_object(
        &mut self,
        address: ContentDigest,
        bytes: &[u8],
    ) -> Result<(), SealedAcceptedIndexError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct MapLinkWire {
    key: AuthenticatedMapKey,
    digest: [u8; 32],
}

impl From<AuthenticatedMapLinkV1> for MapLinkWire {
    fn from(value: AuthenticatedMapLinkV1) -> Self {
        Self {
            key: value.key,
            digest: *value.digest.as_bytes(),
        }
    }
}

impl From<MapLinkWire> for AuthenticatedMapLinkV1 {
    fn from(value: MapLinkWire) -> Self {
        Self {
            key: value.key,
            digest: ContentDigest::from_bytes(value.digest),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct MapNodeWire {
    schema: u32,
    key: AuthenticatedMapKey,
    value_digest: [u8; 32],
    left: Option<MapLinkWire>,
    right: Option<MapLinkWire>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MapNode {
    pub(crate) key: AuthenticatedMapKey,
    pub(crate) value_digest: ContentDigest,
    pub(crate) left: Option<AuthenticatedMapLinkV1>,
    pub(crate) right: Option<AuthenticatedMapLinkV1>,
}

fn canonical_encode<T: Serialize>(value: &T) -> Result<Vec<u8>, SealedAcceptedIndexError> {
    postcard::to_allocvec(value).map_err(|error| SealedAcceptedIndexError::Store(error.to_string()))
}

fn canonical_decode<T: for<'de> Deserialize<'de> + Serialize>(
    bytes: &[u8],
    what: &str,
) -> Result<T, SealedAcceptedIndexError> {
    let (value, trailing): (T, &[u8]) = postcard::take_from_bytes(bytes)
        .map_err(|error| corrupt(format!("invalid {what}: {error}")))?;
    if !trailing.is_empty() || canonical_encode(&value)? != bytes {
        return Err(corrupt(format!("non-canonical {what}")));
    }
    Ok(value)
}

impl MapNode {
    pub(crate) fn logical_digest(&self) -> ContentDigest {
        authenticated_map_node_digest(
            self.key,
            self.value_digest,
            self.left.map(|child| (child.key, child.digest)),
            self.right.map(|child| (child.key, child.digest)),
        )
    }

    fn encode(&self) -> Result<Vec<u8>, SealedAcceptedIndexError> {
        canonical_encode(&MapNodeWire {
            schema: MAP_NODE_SCHEMA_VERSION,
            key: self.key,
            value_digest: *self.value_digest.as_bytes(),
            left: self.left.map(Into::into),
            right: self.right.map(Into::into),
        })
    }

    pub(crate) fn decode(
        expected: AuthenticatedMapLinkV1,
        bytes: &[u8],
    ) -> Result<Self, SealedAcceptedIndexError> {
        let wire: MapNodeWire = canonical_decode(bytes, "authenticated-map node")?;
        if wire.schema != MAP_NODE_SCHEMA_VERSION || wire.key != expected.key {
            return Err(corrupt("authenticated-map node schema/key mismatch"));
        }
        let node = Self {
            key: wire.key,
            value_digest: ContentDigest::from_bytes(wire.value_digest),
            left: wire.left.map(Into::into),
            right: wire.right.map(Into::into),
        };
        if !valid_children(node.key, node.left.as_ref(), node.right.as_ref())
            || node.logical_digest() != expected.digest
        {
            return Err(corrupt("authenticated-map node binding mismatch"));
        }
        Ok(node)
    }
}

fn valid_children(
    key: AuthenticatedMapKey,
    left: Option<&AuthenticatedMapLinkV1>,
    right: Option<&AuthenticatedMapLinkV1>,
) -> bool {
    left.is_none_or(|child| {
        child.key < key && authenticated_map_priority_order(key, child.key).is_lt()
    }) && right.is_none_or(|child| {
        child.key > key && authenticated_map_priority_order(key, child.key).is_lt()
    })
}

fn validate_root(root: AuthenticatedMapRootV1) -> Result<(), SealedAcceptedIndexError> {
    if (root.count == 0) != root.root.is_none()
        || (root.count == 0 && root.root_digest() != authenticated_map_empty_digest())
    {
        return Err(corrupt("authenticated-map root count/binding mismatch"));
    }
    Ok(())
}

fn ensure_depth(depth: usize) -> Result<(), SealedAcceptedIndexError> {
    if depth >= MAX_MAP_DEPTH {
        Err(SealedAcceptedIndexError::Capacity)
    } else {
        Ok(())
    }
}

pub(crate) struct MapReader<'a, Store> {
    store: &'a Store,
}

impl<'a, Store: MapNodeObjects> MapReader<'a, Store> {
    pub(crate) const fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub(crate) fn map_value(
        &self,
        root: AuthenticatedMapRootV1,
        key: impl Into<AuthenticatedMapKey>,
    ) -> Result<Option<ContentDigest>, SealedAcceptedIndexError> {
        let key = key.into();
        validate_root(root)?;
        let mut current = root.root;
        for _ in 0..MAX_MAP_DEPTH {
            let Some(link) = current else { return Ok(None) };
            let node = self.read_map_node(link)?;
            match key.cmp(&node.key) {
                Ordering::Equal => return Ok(Some(node.value_digest)),
                Ordering::Less => current = node.left,
                Ordering::Greater => current = node.right,
            }
        }
        Err(SealedAcceptedIndexError::Capacity)
    }

    pub(crate) fn read_map_node(
        &self,
        link: AuthenticatedMapLinkV1,
    ) -> Result<MapNode, SealedAcceptedIndexError> {
        let bytes = self.store.read_map_node_object(link.digest)?.ok_or(
            SealedAcceptedIndexError::Missing {
                kind: tine_storage::sealed_accepted_index::SealedAcceptedObjectKind::Table,
                address: link.digest,
            },
        )?;
        MapNode::decode(link, &bytes)
    }
}

pub(crate) struct MapWriter<'a, Store> {
    store: &'a mut Store,
}

impl<'a, Store: MapNodeObjects> MapWriter<'a, Store> {
    pub(crate) fn new(store: &'a mut Store) -> Self {
        Self { store }
    }

    pub(crate) fn upsert_map(
        &mut self,
        root: AuthenticatedMapRootV1,
        key: impl Into<AuthenticatedMapKey>,
        value_digest: ContentDigest,
    ) -> Result<AuthenticatedMapRootV1, SealedAcceptedIndexError> {
        let key = key.into();
        validate_root(root)?;
        let (link, inserted) = self.upsert_child(root.root, key, value_digest, 0)?;
        Ok(AuthenticatedMapRootV1 {
            count: if inserted {
                root.count
                    .checked_add(1)
                    .ok_or(SealedAcceptedIndexError::Capacity)?
            } else {
                root.count
            },
            root: Some(link),
        })
    }

    /// Remove one key by path copying, preserving every previous root.
    pub(crate) fn remove_map(
        &mut self,
        root: AuthenticatedMapRootV1,
        key: impl Into<AuthenticatedMapKey>,
    ) -> Result<AuthenticatedMapRootV1, SealedAcceptedIndexError> {
        let key = key.into();
        validate_root(root)?;
        let (link, removed) = self.remove_child(root.root, key, 0)?;
        if !removed {
            return Ok(root);
        }
        let next = AuthenticatedMapRootV1 {
            count: root
                .count
                .checked_sub(1)
                .ok_or_else(|| corrupt("map removal underflow"))?,
            root: link,
        };
        validate_root(next)?;
        Ok(next)
    }

    fn upsert_child(
        &mut self,
        current: Option<AuthenticatedMapLinkV1>,
        key: AuthenticatedMapKey,
        value_digest: ContentDigest,
        depth: usize,
    ) -> Result<(AuthenticatedMapLinkV1, bool), SealedAcceptedIndexError> {
        ensure_depth(depth)?;
        let Some(current) = current else {
            return Ok((
                self.publish_map_node(&MapNode {
                    key,
                    value_digest,
                    left: None,
                    right: None,
                })?,
                true,
            ));
        };
        let mut node = self.read_map_node(current)?;
        let inserted;
        match key.cmp(&node.key) {
            Ordering::Equal => {
                node.value_digest = value_digest;
                inserted = false;
            }
            Ordering::Less => {
                let (left, was_inserted) =
                    self.upsert_child(node.left.take(), key, value_digest, depth + 1)?;
                node.left = Some(left);
                inserted = was_inserted;
                if authenticated_map_priority_order(left.key, node.key).is_lt() {
                    return Ok((self.rotate_right(node)?, inserted));
                }
            }
            Ordering::Greater => {
                let (right, was_inserted) =
                    self.upsert_child(node.right.take(), key, value_digest, depth + 1)?;
                node.right = Some(right);
                inserted = was_inserted;
                if authenticated_map_priority_order(right.key, node.key).is_lt() {
                    return Ok((self.rotate_left(node)?, inserted));
                }
            }
        }
        Ok((self.publish_map_node(&node)?, inserted))
    }

    fn remove_child(
        &mut self,
        current: Option<AuthenticatedMapLinkV1>,
        key: AuthenticatedMapKey,
        depth: usize,
    ) -> Result<(Option<AuthenticatedMapLinkV1>, bool), SealedAcceptedIndexError> {
        ensure_depth(depth)?;
        let Some(current) = current else {
            return Ok((None, false));
        };
        let mut node = self.read_map_node(current)?;
        let removed = match key.cmp(&node.key) {
            Ordering::Equal => {
                return Ok((self.join_children(node.left, node.right, depth + 1)?, true));
            }
            Ordering::Less => {
                let (left, removed) = self.remove_child(node.left, key, depth + 1)?;
                node.left = left;
                removed
            }
            Ordering::Greater => {
                let (right, removed) = self.remove_child(node.right, key, depth + 1)?;
                node.right = right;
                removed
            }
        };
        if !removed {
            return Ok((Some(current), false));
        }
        Ok((Some(self.publish_map_node(&node)?), true))
    }

    fn join_children(
        &mut self,
        left: Option<AuthenticatedMapLinkV1>,
        right: Option<AuthenticatedMapLinkV1>,
        depth: usize,
    ) -> Result<Option<AuthenticatedMapLinkV1>, SealedAcceptedIndexError> {
        ensure_depth(depth)?;
        let (Some(left), Some(right)) = (left, right) else {
            return Ok(left.or(right));
        };
        if left.key >= right.key {
            return Err(corrupt("map join children are not ordered"));
        }
        let mut node;
        if authenticated_map_priority_order(left.key, right.key).is_lt() {
            node = self.read_map_node(left)?;
            node.right = self.join_children(node.right, Some(right), depth + 1)?;
        } else {
            node = self.read_map_node(right)?;
            node.left = self.join_children(Some(left), node.left, depth + 1)?;
        }
        Ok(Some(self.publish_map_node(&node)?))
    }

    fn rotate_right(
        &mut self,
        mut node: MapNode,
    ) -> Result<AuthenticatedMapLinkV1, SealedAcceptedIndexError> {
        let left = node
            .left
            .take()
            .ok_or_else(|| corrupt("right rotation has no left child"))?;
        let mut left_node = self.read_map_node(left)?;
        node.left = left_node.right.take();
        left_node.right = Some(self.publish_map_node(&node)?);
        self.publish_map_node(&left_node)
    }

    fn rotate_left(
        &mut self,
        mut node: MapNode,
    ) -> Result<AuthenticatedMapLinkV1, SealedAcceptedIndexError> {
        let right = node
            .right
            .take()
            .ok_or_else(|| corrupt("left rotation has no right child"))?;
        let mut right_node = self.read_map_node(right)?;
        node.right = right_node.left.take();
        right_node.left = Some(self.publish_map_node(&node)?);
        self.publish_map_node(&right_node)
    }

    fn read_map_node(
        &self,
        link: AuthenticatedMapLinkV1,
    ) -> Result<MapNode, SealedAcceptedIndexError> {
        MapReader::new(&*self.store).read_map_node(link)
    }

    fn publish_map_node(
        &mut self,
        node: &MapNode,
    ) -> Result<AuthenticatedMapLinkV1, SealedAcceptedIndexError> {
        let address = node.logical_digest();
        let bytes = node.encode()?;
        self.store.publish_map_node_object(address, &bytes)?;
        Ok(AuthenticatedMapLinkV1 {
            key: node.key,
            digest: address,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MemoryNodes {
        objects: BTreeMap<ContentDigest, Vec<u8>>,
    }

    impl MapNodeObjects for MemoryNodes {
        fn read_map_node_object(
            &self,
            address: ContentDigest,
        ) -> Result<Option<Vec<u8>>, SealedAcceptedIndexError> {
            Ok(self.objects.get(&address).cloned())
        }

        fn publish_map_node_object(
            &mut self,
            address: ContentDigest,
            bytes: &[u8],
        ) -> Result<(), SealedAcceptedIndexError> {
            self.objects.insert(address, bytes.to_vec());
            Ok(())
        }
    }

    fn key(byte: u8) -> AuthenticatedMapKey {
        AuthenticatedMapKey::from([byte; 16])
    }

    /// The relocated treap still agrees bit for bit with `tine-storage`'s own
    /// canonical root builder, which is what makes this a transcription rather
    /// than a second format.
    #[test]
    fn the_relocated_treap_matches_the_shared_canonical_root() {
        let mut store = MemoryNodes::default();
        let mut root = AuthenticatedMapRootV1::empty();
        let mut model: BTreeMap<AuthenticatedMapKey, ContentDigest> = BTreeMap::new();
        for byte in 0..32_u8 {
            let value = ContentDigest::of(&[byte, 7]);
            root = MapWriter::new(&mut store)
                .upsert_map(root, key(byte), value)
                .unwrap();
            model.insert(key(byte), value);
            let entries: Vec<_> = model.iter().map(|(k, v)| (*k, *v)).collect();
            assert_eq!(
                root,
                tine_storage::sealed_accepted_index::authenticated_map_root(&entries).unwrap()
            );
        }
        for byte in (0..32_u8).step_by(3) {
            root = MapWriter::new(&mut store)
                .remove_map(root, key(byte))
                .unwrap();
            model.remove(&key(byte));
            let entries: Vec<_> = model.iter().map(|(k, v)| (*k, *v)).collect();
            assert_eq!(
                root,
                tine_storage::sealed_accepted_index::authenticated_map_root(&entries).unwrap()
            );
        }
        for (map_key, value) in &model {
            assert_eq!(
                MapReader::new(&store).map_value(root, *map_key).unwrap(),
                Some(*value)
            );
        }
        assert_eq!(
            MapReader::new(&store).map_value(root, key(0)).unwrap(),
            None
        );
    }
}
