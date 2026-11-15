use super::*;
use crate::model::BlockOwners;
use std::collections::{BTreeMap, HashSet};
use std::mem::size_of;

// Requested tree allocation bytes, including Arc headers and Vec capacity.
// Shared nodes are counted once per root; old snapshots are measured separately.
fn shape<K, V>(node: &Node<K, V>) -> (usize, usize) {
    let header = size_of::<Node<K, V>>() + 2 * size_of::<usize>();
    match node {
        Node::Leaf(rows) => (header + rows.capacity() * size_of::<(K, V)>(), 1),
        Node::Inner(rows) => {
            let children: Vec<_> = rows.iter().map(|(_, child)| shape(child)).collect();
            (
                header
                    + rows.capacity() * size_of::<(K, Arc<Node<K, V>>)>()
                    + children.iter().map(|(bytes, _)| bytes).sum::<usize>(),
                1 + children.iter().map(|(_, height)| height).max().unwrap(),
            )
        }
    }
}

fn check_node<K: Ord, V>(node: &Node<K, V>, root: bool) -> (usize, usize, &K, &K) {
    let (len, capacity) = match node {
        Node::Leaf(rows) => (rows.len(), rows.capacity()),
        Node::Inner(rows) => (rows.len(), rows.capacity()),
    };
    assert!(
        len <= MAX && len >= if root { 1 } else { MAX / 2 },
        "occupancy {len}"
    );
    assert!(
        capacity <= len + 1,
        "retained capacity {capacity} for {len} rows"
    );
    match node {
        Node::Leaf(rows) => {
            assert!(rows.windows(2).all(|pair| pair[0].0 < pair[1].0));
            (len, 1, &rows[0].0, &rows[len - 1].0)
        }
        Node::Inner(rows) => {
            assert!(len >= 2, "unary root");
            let children: Vec<_> = rows
                .iter()
                .map(|(_, child)| check_node(child, false))
                .collect();
            assert!(children.windows(2).all(|pair| pair[0].1 == pair[1].1));
            for at in 1..len {
                assert!(children[at - 1].3 < &rows[at].0);
                assert!(&rows[at].0 <= children[at].2);
            }
            (
                children.iter().map(|child| child.0).sum(),
                children[0].1 + 1,
                children[0].2,
                children[len - 1].3,
            )
        }
    }
}

fn check<K: Ord, V>(map: &InlineMap<K, V>) {
    if let Some(root) = &map.root {
        let (entries, height, _, _) = check_node(root, true);
        assert_eq!(entries, map.len);
        // For height h >= 2, two root children each lead to at least
        // MIN^(h-1) entries. This also bounds height by surviving entries.
        if height >= 2 {
            assert!(entries >= 2 * (MAX / 2).pow((height - 1) as u32));
        }
    } else {
        assert_eq!(map.len, 0);
    }
}

#[test]
fn bulk_builds_balance_partial_leaves_and_inner_levels() {
    for len in 0..1100u64 {
        let map = InlineMap::from_sorted_unique((0..len).map(|key| (key, key)).collect());
        check(&map);
        assert_eq!(map.len(), len as usize);
    }
}

#[test]
fn bulk_pruning_releases_owner_tree_memory_and_height() {
    let mut map = InlineMap::from_sorted_unique(
        (0..240_000u128)
            .map(|key| (key, BlockOwners::One(key as usize)))
            .collect(),
    );
    let held = map.clone();
    for key in 0..240_000 {
        if key % 24 != 0 {
            map.remove(&key);
        }
    }
    assert_eq!(held.len(), 240_000);
    assert_eq!(held.get(&239_999), Some(&BlockOwners::One(239_999)));
    drop(held);
    let fresh = InlineMap::from_sorted_unique(
        (0..240_000u128)
            .step_by(24)
            .map(|key| (key, BlockOwners::One(key as usize)))
            .collect(),
    );
    assert!(map.iter().eq(fresh.iter()));
    let (bytes, height) = shape(map.root.as_deref().unwrap());
    let (fresh_bytes, fresh_height) = shape(fresh.root.as_deref().unwrap());
    eprintln!(
        "pruned: {bytes} B / height {height}; fresh: {fresh_bytes} B / height {fresh_height}"
    );
    assert!(
        bytes <= 4 * fresh_bytes,
        "pruned {bytes} B exceeds 4x fresh {fresh_bytes} B"
    );
    assert!(height <= 1 + fresh_height);
    check(&map);
    for key in (24..240_000).step_by(24) {
        map.remove(&key);
    }
    check(&map);
    assert_eq!(shape(map.root.as_deref().unwrap()).1, 1);
    map.remove(&0);
    check(&map);
}

#[test]
fn random_updates_preserve_contents_occupancy_and_snapshot_paths() {
    for initial in [0, 25, 769, 5000] {
        let mut map = InlineMap::from_sorted_unique((0..initial).map(|key| (key, key)).collect());
        let mut expected: BTreeMap<_, _> = (0..initial).map(|key| (key, key)).collect();
        check(&map);
        let mut seed = 179u64;
        for step in 0..8000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = (seed >> 32) % 6000;
            let held = map.clone();
            let old_value = expected.get(&key).copied();
            let mut old_nodes = HashSet::new();
            fn pointers<K, V>(node: &Arc<Node<K, V>>, nodes: &mut HashSet<usize>) {
                nodes.insert(Arc::as_ptr(node) as usize);
                if let Node::Inner(rows) = &**node {
                    for (_, child) in rows {
                        pointers(child, nodes);
                    }
                }
            }
            if let Some(root) = &held.root {
                pointers(root, &mut old_nodes);
            }
            if seed % 4 != 0 {
                map.remove(&key);
                expected.remove(&key);
            } else {
                map.insert(key, step);
                expected.insert(key, step);
            }
            assert_eq!(held.get(&key).copied(), old_value);
            assert_eq!(map.len(), expected.len());
            assert!(map.iter().eq(expected.iter()));
            check(&map);
            let mut new_nodes = HashSet::new();
            if let Some(root) = &map.root {
                pointers(root, &mut new_nodes);
                let height = shape(root).1;
                // One search path plus at most one repaired sibling per level;
                // insertion may also create a new root.
                assert!(new_nodes.difference(&old_nodes).count() <= 2 * height + 1);
            }
        }
    }
}
