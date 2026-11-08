//! Persistent B+tree for small cloneable entries, stored inside shared leaves.
//! A leaf holds entries next to each other, so an entry pays no node, pointer or
//! reference-count words of its own: at 1.84M block ids this costs about 37 B per
//! entry where the binary tree it replaced cost 80 B. Path copying clones one node
//! per level, O(log N) nodes of at most `MAX` entries; bulk construction is O(N).
//! Unlike `persistent::Map`, there is no separate shared allocation per entry.
use std::sync::Arc;

/// A node splits above this many entries. Bounds the bytes one path copy clones.
const MAX: usize = 32;
/// Bulk construction leaves room to insert without an immediate split.
const FILL: usize = 24;

#[derive(Debug)]
enum Node<K, V> {
    Leaf(Vec<(K, V)>),
    /// Children with a lower bound on their keys. The first child also takes
    /// every key below its bound, so a removal never has to update a bound.
    Inner(Vec<(K, Arc<Node<K, V>>)>),
}
impl<K: Clone, V: Clone> Clone for Node<K, V> {
    fn clone(&self) -> Self {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::shared_tree_node_copy();
        // One spare slot: a path is usually copied to change a row.
        match self {
            Self::Leaf(rows) => {
                let mut copy = Vec::with_capacity(rows.len() + 1);
                copy.extend_from_slice(rows);
                Self::Leaf(copy)
            }
            Self::Inner(rows) => {
                let mut copy = Vec::with_capacity(rows.len() + 1);
                copy.extend_from_slice(rows);
                Self::Inner(copy)
            }
        }
    }
}
fn child_at<K: Ord, V>(rows: &[(K, V)], key: &K) -> usize {
    // The first bound is never consulted: a key below it goes to child 0, and
    // only the later bounds are guaranteed partitioned.
    rows[1..].partition_point(|(bound, _)| bound <= key)
}
fn split<T>(rows: &mut Vec<T>) -> Vec<T> {
    let tail = rows.split_off(rows.len() / 2);
    rows.shrink_to_fit();
    tail
}
impl<K: Ord + Clone, V: Clone> Node<K, V> {
    /// Sets `key`; returns whether it is new, and the upper half of a split.
    fn put(node: &mut Arc<Self>, key: K, value: V) -> (bool, Option<(K, Arc<Self>)>) {
        match Arc::make_mut(node) {
            Self::Leaf(rows) => {
                let added = match rows.binary_search_by(|(k, _)| k.cmp(&key)) {
                    Ok(at) => {
                        rows[at].1 = value;
                        false
                    }
                    Err(at) => {
                        rows.reserve_exact(1);
                        rows.insert(at, (key, value));
                        true
                    }
                };
                if rows.len() <= MAX {
                    return (added, None);
                }
                let tail = split(rows);
                let bound = tail[0].0.clone();
                (added, Some((bound, Arc::new(Self::Leaf(tail)))))
            }
            Self::Inner(rows) => {
                let at = child_at(rows, &key);
                let (added, upper) = Self::put(&mut rows[at].1, key, value);
                if let Some(upper) = upper {
                    rows.reserve_exact(1);
                    rows.insert(at + 1, upper);
                    if rows.len() > MAX {
                        let tail = split(rows);
                        let bound = tail[0].0.clone();
                        return (added, Some((bound, Arc::new(Self::Inner(tail)))));
                    }
                }
                (added, None)
            }
        }
    }
    /// Removes a present `key`; returns whether this node is now empty.
    fn delete(node: &mut Arc<Self>, key: &K) -> bool {
        match Arc::make_mut(node) {
            Self::Leaf(rows) => {
                if let Ok(at) = rows.binary_search_by(|(k, _)| k.cmp(key)) {
                    rows.remove(at);
                }
                rows.is_empty()
            }
            Self::Inner(rows) => {
                let at = child_at(rows, key);
                if Self::delete(&mut rows[at].1, key) {
                    rows.remove(at);
                }
                rows.is_empty()
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct InlineMap<K, V> {
    root: Option<Arc<Node<K, V>>>,
    len: usize,
}
impl<K: Ord + Clone, V: Clone> InlineMap<K, V> {
    pub(super) fn new() -> Self {
        Self { root: None, len: 0 }
    }
    pub(super) fn len(&self) -> usize {
        self.len
    }
    pub(super) fn get(&self, key: &K) -> Option<&V> {
        let mut node = self.root.as_deref()?;
        loop {
            match node {
                Node::Leaf(rows) => {
                    let at = rows.binary_search_by(|(k, _)| k.cmp(key)).ok()?;
                    return Some(&rows[at].1);
                }
                Node::Inner(rows) => node = &rows[child_at(rows, key)].1,
            }
        }
    }
    pub(super) fn from_sorted_unique(entries: Vec<(K, V)>) -> Self {
        debug_assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let len = entries.len();
        fn chunk<T>(rows: impl ExactSizeIterator<Item = T>) -> Vec<Vec<T>> {
            let mut left = rows.len();
            let mut chunks = Vec::with_capacity(left / FILL + 1);
            let mut rows = rows.peekable();
            while rows.peek().is_some() {
                let take = FILL.min(left);
                left -= take;
                let mut chunk = Vec::with_capacity(take);
                chunk.extend(rows.by_ref().take(take));
                chunks.push(chunk);
            }
            chunks
        }
        let mut level: Vec<(K, Arc<Node<K, V>>)> = chunk(entries.into_iter())
            .into_iter()
            .map(|rows| (rows[0].0.clone(), Arc::new(Node::Leaf(rows))))
            .collect();
        while level.len() > 1 {
            level = chunk(level.into_iter())
                .into_iter()
                .map(|rows| (rows[0].0.clone(), Arc::new(Node::Inner(rows))))
                .collect();
        }
        Self {
            root: level.pop().map(|(_, root)| root),
            len,
        }
    }
    pub(super) fn insert(&mut self, key: K, value: V) {
        let Some(root) = self.root.as_mut() else {
            self.root = Some(Arc::new(Node::Leaf(vec![(key, value)])));
            self.len = 1;
            return;
        };
        let (added, upper) = Node::put(root, key, value);
        self.len += usize::from(added);
        if let Some((bound, upper)) = upper {
            let lower = self.root.take().expect("just used");
            let first = match &*lower {
                Node::Leaf(rows) => rows[0].0.clone(),
                Node::Inner(rows) => rows[0].0.clone(),
            };
            self.root = Some(Arc::new(Node::Inner(vec![(first, lower), (bound, upper)])));
        }
    }
    pub(super) fn remove(&mut self, key: &K) {
        if self.get(key).is_none() {
            return;
        }
        self.len -= 1;
        let root = self.root.as_mut().expect("the key is present");
        if Node::delete(root, key) {
            self.root = None;
        }
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let mut stack: Vec<std::slice::Iter<'_, (K, Arc<Node<K, V>>)>> = Vec::new();
        let mut leaf: std::slice::Iter<'_, (K, V)> = [].iter();
        let mut next = self.root.as_deref();
        std::iter::from_fn(move || loop {
            if let Some((key, value)) = leaf.next() {
                return Some((key, value));
            }
            match next.take() {
                Some(Node::Leaf(rows)) => leaf = rows.iter(),
                Some(Node::Inner(rows)) => {
                    stack.push(rows.iter());
                    next = Some(&*stack.last_mut()?.next()?.1);
                }
                None => {
                    let top = stack.last_mut()?;
                    match top.next() {
                        Some((_, child)) => next = Some(child),
                        None => {
                            stack.pop();
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descending_inserts_below_every_bound_keep_every_key() {
        let mut map =
            InlineMap::from_sorted_unique((100_000..105_000u32).map(|i| (i, i)).collect());
        for key in (0..100_000u32).rev() {
            map.insert(key, key);
        }
        assert_eq!(map.len(), 105_000);
        for key in 0..105_000u32 {
            assert_eq!(map.get(&key), Some(&key));
        }
        assert!(map.iter().map(|(&k, _)| k).eq(0..105_000u32));
    }
    #[test]
    fn splits_and_empties_match_a_btree_and_preserve_held_roots() {
        let mut map = InlineMap::from_sorted_unique((0..5000u32).map(|i| (i * 2, i)).collect());
        let mut expected: std::collections::BTreeMap<_, _> =
            (0..5000u32).map(|i| (i * 2, i)).collect();
        let held = map.clone();
        let mut seed = 5u64;
        for step in 0..40_000u32 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = ((seed >> 32) % 12_000) as u32;
            // Grow, then drain almost everything so whole nodes empty out.
            if (step < 20_000 && seed % 4 == 0) || (step >= 20_000 && seed % 8 != 0) {
                map.remove(&key);
                expected.remove(&key);
            } else {
                map.insert(key, step);
                expected.insert(key, step);
            }
            assert_eq!(map.len(), expected.len());
            assert_eq!(map.get(&key), expected.get(&key));
        }
        for key in 0..12_000u32 {
            assert_eq!(map.get(&key), expected.get(&key));
        }
        assert_eq!(
            map.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            expected.into_iter().collect::<Vec<_>>()
        );
        for key in map.iter().map(|(&k, _)| k).collect::<Vec<_>>() {
            map.remove(&key);
        }
        assert_eq!(map.len(), 0);
        assert_eq!(map.iter().count(), 0);
        map.insert(7, 1);
        assert_eq!(map.get(&7), Some(&1));
        assert_eq!(
            held.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            (0..5000u32).map(|i| (i * 2, i)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn inline_updates_match_a_btree_and_preserve_held_roots() {
        let mut map = InlineMap::from_sorted_unique((0..90).map(|i| (i, i)).collect());
        let mut expected: std::collections::BTreeMap<_, _> = (0..90).map(|i| (i, i)).collect();
        let held = map.clone();
        let mut seed = 17u64;
        for step in 0..2000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = (seed >> 32) % 150;
            if seed % 3 == 0 {
                map.remove(&key);
                expected.remove(&key);
            } else {
                map.insert(key, step);
                expected.insert(key, step);
            }
            assert_eq!(map.len(), expected.len());
            assert_eq!(
                map.iter().collect::<Vec<_>>(),
                expected.iter().collect::<Vec<_>>()
            );
            for (&key, value) in &expected {
                assert_eq!(map.get(&key), Some(value));
            }
        }
        assert_eq!(
            held.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            (0..90).map(|i| (i, i)).collect::<Vec<_>>()
        );
    }
}
