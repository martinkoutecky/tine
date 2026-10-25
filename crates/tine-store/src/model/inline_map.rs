//! Persistent AVL for small cloneable entries, stored inside each shared node.
//! Path copying clones only O(log N) inline entries; bulk construction is O(N).
//! Unlike `persistent::Map`, there is no separate shared allocation per entry.
use std::{cmp::Ordering, sync::Arc};

#[derive(Debug)]
struct Node<K, V> {
    entry: (K, V),
    left: Option<Arc<Node<K, V>>>,
    right: Option<Arc<Node<K, V>>>,
    height: usize,
}
impl<K: Clone, V: Clone> Clone for Node<K, V> {
    fn clone(&self) -> Self {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::shared_tree_node_copy();
        Self {
            entry: self.entry.clone(),
            left: self.left.clone(),
            right: self.right.clone(),
            height: self.height,
        }
    }
}
impl<K: Clone, V: Clone> Node<K, V> {
    fn height(node: &Option<Arc<Self>>) -> usize {
        node.as_ref().map_or(0, |node| node.height)
    }
    fn update(&mut self) {
        self.height = 1 + Self::height(&self.left).max(Self::height(&self.right));
    }
    fn left(mut node: Arc<Self>) -> Arc<Self> {
        let mut next = Arc::make_mut(&mut node).right.take().unwrap();
        Arc::make_mut(&mut node).right = Arc::make_mut(&mut next).left.take();
        Arc::make_mut(&mut node).update();
        Arc::make_mut(&mut next).left = Some(node);
        Arc::make_mut(&mut next).update();
        next
    }
    fn right(mut node: Arc<Self>) -> Arc<Self> {
        let mut next = Arc::make_mut(&mut node).left.take().unwrap();
        Arc::make_mut(&mut node).left = Arc::make_mut(&mut next).right.take();
        Arc::make_mut(&mut node).update();
        Arc::make_mut(&mut next).right = Some(node);
        Arc::make_mut(&mut next).update();
        next
    }
    fn balance(mut node: Arc<Self>) -> Arc<Self> {
        Arc::make_mut(&mut node).update();
        if Self::height(&node.left) > Self::height(&node.right) + 1 {
            let left = node.left.as_ref().unwrap();
            if Self::height(&left.right) > Self::height(&left.left) {
                let left = Arc::make_mut(&mut node).left.take().unwrap();
                Arc::make_mut(&mut node).left = Some(Self::left(left));
            }
            return Self::right(node);
        }
        if Self::height(&node.right) > Self::height(&node.left) + 1 {
            let right = node.right.as_ref().unwrap();
            if Self::height(&right.left) > Self::height(&right.right) {
                let right = Arc::make_mut(&mut node).right.take().unwrap();
                Arc::make_mut(&mut node).right = Some(Self::right(right));
            }
            return Self::left(node);
        }
        node
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
        let mut node = self.root.as_deref();
        while let Some(n) = node {
            match key.cmp(&n.entry.0) {
                Ordering::Less => node = n.left.as_deref(),
                Ordering::Greater => node = n.right.as_deref(),
                Ordering::Equal => return Some(&n.entry.1),
            }
        }
        None
    }
    pub(super) fn from_sorted_unique(entries: Vec<(K, V)>) -> Self {
        debug_assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
        fn build<K: Clone, V: Clone>(
            rows: &mut impl Iterator<Item = (K, V)>,
            len: usize,
        ) -> Option<Arc<Node<K, V>>> {
            if len == 0 {
                return None;
            }
            let left = build(rows, len / 2);
            let entry = rows.next().expect("counted sorted entries");
            let right = build(rows, len - len / 2 - 1);
            let mut node = Node {
                entry,
                left,
                right,
                height: 0,
            };
            node.update();
            Some(Arc::new(node))
        }
        let len = entries.len();
        Self {
            root: build(&mut entries.into_iter(), len),
            len,
        }
    }
    pub(super) fn insert(&mut self, key: K, value: V) {
        fn put<K: Ord + Clone, V: Clone>(
            node: Option<Arc<Node<K, V>>>,
            entry: (K, V),
            added: &mut bool,
        ) -> Arc<Node<K, V>> {
            let Some(mut node) = node else {
                *added = true;
                return Arc::new(Node {
                    entry,
                    left: None,
                    right: None,
                    height: 1,
                });
            };
            match entry.0.cmp(&node.entry.0) {
                Ordering::Less => {
                    let left = Arc::make_mut(&mut node).left.take();
                    Arc::make_mut(&mut node).left = Some(put(left, entry, added));
                }
                Ordering::Greater => {
                    let right = Arc::make_mut(&mut node).right.take();
                    Arc::make_mut(&mut node).right = Some(put(right, entry, added));
                }
                Ordering::Equal => Arc::make_mut(&mut node).entry = entry,
            }
            Node::balance(node)
        }
        let mut added = false;
        self.root = Some(put(self.root.take(), (key, value), &mut added));
        self.len += usize::from(added);
    }
    pub(super) fn remove(&mut self, key: &K) {
        fn pop<K: Clone, V: Clone>(mut node: Arc<Node<K, V>>) -> ((K, V), Option<Arc<Node<K, V>>>) {
            if let Some(left) = Arc::make_mut(&mut node).left.take() {
                let (entry, left) = pop(left);
                Arc::make_mut(&mut node).left = left;
                (entry, Some(Node::balance(node)))
            } else {
                (node.entry.clone(), node.right.clone())
            }
        }
        fn delete<K: Ord + Clone, V: Clone>(
            node: Option<Arc<Node<K, V>>>,
            key: &K,
        ) -> Option<Arc<Node<K, V>>> {
            let mut node = node?;
            match key.cmp(&node.entry.0) {
                Ordering::Less => {
                    let left = Arc::make_mut(&mut node).left.take();
                    Arc::make_mut(&mut node).left = delete(left, key);
                }
                Ordering::Greater => {
                    let right = Arc::make_mut(&mut node).right.take();
                    Arc::make_mut(&mut node).right = delete(right, key);
                }
                Ordering::Equal => {
                    if node.left.is_none() {
                        return node.right.clone();
                    }
                    let Some(right) = Arc::make_mut(&mut node).right.take() else {
                        return node.left.clone();
                    };
                    let (entry, right) = pop(right);
                    Arc::make_mut(&mut node).entry = entry;
                    Arc::make_mut(&mut node).right = right;
                }
            }
            Some(Node::balance(node))
        }
        if self.get(key).is_some() {
            self.root = delete(self.root.take(), key);
            self.len -= 1;
        }
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let mut stack = Vec::new();
        let mut next = self.root.as_deref();
        std::iter::from_fn(move || {
            while let Some(node) = next {
                stack.push(node);
                next = node.left.as_deref();
            }
            let node = stack.pop()?;
            next = node.right.as_deref();
            Some((&node.entry.0, &node.entry.1))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
