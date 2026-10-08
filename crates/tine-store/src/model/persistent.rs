//! Immutable snapshot collections. A mutation copies O(log N) tree nodes and
//! only the changed value; clones share all entries. Ordered iteration costs
//! O(N). Bulk construction sorts once and builds a balanced tree in O(N).
//! I-25 exemplar: never put a graph-sized Vec/HashMap behind Arc::make_mut.
use std::{borrow::Borrow, cmp::Ordering, sync::Arc};

#[derive(Debug)]
struct Node<K, V> {
    entry: Arc<(K, V)>,
    left: Option<Arc<Node<K, V>>>,
    right: Option<Arc<Node<K, V>>>,
    height: usize,
    len: usize,
}
impl<K, V> Clone for Node<K, V> {
    fn clone(&self) -> Self {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::shared_tree_node_copy();
        Self {
            entry: self.entry.clone(),
            left: self.left.clone(),
            right: self.right.clone(),
            height: self.height,
            len: self.len,
        }
    }
}
impl<K, V> Node<K, V> {
    fn height(node: &Option<Arc<Self>>) -> usize {
        node.as_ref().map_or(0, |n| n.height)
    }
    fn len(node: &Option<Arc<Self>>) -> usize {
        node.as_ref().map_or(0, |n| n.len)
    }
    fn update(&mut self) {
        self.height = 1 + Self::height(&self.left).max(Self::height(&self.right));
        self.len = 1 + Self::len(&self.left) + Self::len(&self.right);
    }
    fn rotate_left(mut node: Arc<Self>) -> Arc<Self> {
        let mut next = Arc::make_mut(&mut node).right.take().unwrap();
        Arc::make_mut(&mut node).right = Arc::make_mut(&mut next).left.take();
        Arc::make_mut(&mut node).update();
        Arc::make_mut(&mut next).left = Some(node);
        Arc::make_mut(&mut next).update();
        next
    }
    fn rotate_right(mut node: Arc<Self>) -> Arc<Self> {
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
                Arc::make_mut(&mut node).left = Some(Self::rotate_left(left));
            }
            return Self::rotate_right(node);
        }
        if Self::height(&node.right) > Self::height(&node.left) + 1 {
            let right = node.right.as_ref().unwrap();
            if Self::height(&right.left) > Self::height(&right.right) {
                let right = Arc::make_mut(&mut node).right.take().unwrap();
                Arc::make_mut(&mut node).right = Some(Self::rotate_right(right));
            }
            return Self::rotate_left(node);
        }
        node
    }
}

#[derive(Debug)]
pub(crate) struct Map<K, V> {
    root: Option<Arc<Node<K, V>>>,
}
impl<K, V> Clone for Map<K, V> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
        }
    }
}
impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self { root: None }
    }
}
impl<K: Ord, V> Map<K, V> {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn len(&self) -> usize {
        Node::len(&self.root)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.root.is_none()
    }
    pub(crate) fn get<Q: Ord + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        let mut node = self.root.as_deref();
        while let Some(n) = node {
            match key.cmp(n.entry.0.borrow()) {
                Ordering::Less => node = n.left.as_deref(),
                Ordering::Greater => node = n.right.as_deref(),
                Ordering::Equal => return Some(&n.entry.1),
            }
        }
        None
    }
    pub(crate) fn contains_key<Q: Ord + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.get(key).is_some()
    }
    pub(crate) fn insert(&mut self, key: K, value: V) {
        fn put<K: Ord, V>(node: Option<Arc<Node<K, V>>>, entry: Arc<(K, V)>) -> Arc<Node<K, V>> {
            let Some(mut node) = node else {
                return Arc::new(Node {
                    entry,
                    left: None,
                    right: None,
                    height: 1,
                    len: 1,
                });
            };
            match entry.0.cmp(&node.entry.0) {
                Ordering::Less => {
                    let left = Arc::make_mut(&mut node).left.take();
                    Arc::make_mut(&mut node).left = Some(put(left, entry));
                }
                Ordering::Greater => {
                    let right = Arc::make_mut(&mut node).right.take();
                    Arc::make_mut(&mut node).right = Some(put(right, entry));
                }
                Ordering::Equal => Arc::make_mut(&mut node).entry = entry,
            }
            Node::balance(node)
        }
        self.root = Some(put(self.root.take(), Arc::new((key, value))));
    }
    pub(crate) fn remove<Q: Ord + ?Sized>(&mut self, key: &Q)
    where
        K: Borrow<Q>,
    {
        fn pop_first<K, V>(mut node: Arc<Node<K, V>>) -> (Arc<(K, V)>, Option<Arc<Node<K, V>>>) {
            if let Some(left) = Arc::make_mut(&mut node).left.take() {
                let (entry, left) = pop_first(left);
                Arc::make_mut(&mut node).left = left;
                (entry, Some(Node::balance(node)))
            } else {
                (node.entry.clone(), node.right.clone())
            }
        }
        fn delete<K: Borrow<Q>, V, Q: Ord + ?Sized>(
            node: Option<Arc<Node<K, V>>>,
            key: &Q,
        ) -> Option<Arc<Node<K, V>>> {
            let mut node = node?;
            match key.cmp(node.entry.0.borrow()) {
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
                    let (entry, right) = pop_first(right);
                    Arc::make_mut(&mut node).entry = entry;
                    Arc::make_mut(&mut node).right = right;
                }
            }
            Some(Node::balance(node))
        }
        // A miss does no copying.
        if self.contains_key(key) {
            self.root = delete(self.root.take(), key);
        }
    }
    pub(crate) fn get_mut<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q> + Clone,
        V: Clone,
    {
        if !self.contains_key(key) {
            return None;
        }
        let mut node = self.root.as_mut();
        while let Some(n) = node {
            let n = Arc::make_mut(n);
            match key.cmp(n.entry.0.borrow()) {
                Ordering::Less => node = n.left.as_mut(),
                Ordering::Greater => node = n.right.as_mut(),
                Ordering::Equal => return Some(&mut Arc::make_mut(&mut n.entry).1),
            }
        }
        None
    }
    pub(crate) fn first_entry(&self) -> Option<&(K, V)> {
        let mut node = self.root.as_deref()?;
        while let Some(left) = node.left.as_deref() {
            node = left;
        }
        Some(node.entry.as_ref())
    }
    pub(crate) fn first(&self) -> Option<(&K, &V)> {
        let mut node = self.root.as_deref()?;
        while let Some(left) = node.left.as_deref() {
            node = left;
        }
        Some((&node.entry.0, &node.entry.1))
    }
    pub(crate) fn iter(&self) -> Iter<'_, K, V> {
        let mut it = Iter { stack: Vec::new() };
        it.descend(self.root.as_deref());
        it
    }
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
}
pub(crate) struct Iter<'a, K, V> {
    stack: Vec<&'a Node<K, V>>,
}
impl<'a, K, V> Iter<'a, K, V> {
    fn descend(&mut self, mut node: Option<&'a Node<K, V>>) {
        while let Some(n) = node {
            self.stack.push(n);
            node = n.left.as_deref();
        }
    }
}
impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.descend(node.right.as_deref());
        Some((&node.entry.0, &node.entry.1))
    }
}
impl<'a, K: Ord, V> IntoIterator for &'a Map<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<K: Ord, V> FromIterator<(K, V)> for Map<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut entries: Vec<_> = iter.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        // Last value wins, like a map collected by successive insertions.
        let mut unique: Vec<Arc<(K, V)>> = Vec::with_capacity(entries.len());
        for entry in entries {
            if unique.last().is_some_and(|last| last.0 == entry.0) {
                unique.pop();
            }
            unique.push(Arc::new(entry));
        }
        fn build<K, V>(rows: &[Arc<(K, V)>]) -> Option<Arc<Node<K, V>>> {
            if rows.is_empty() {
                return None;
            }
            let middle = rows.len() / 2;
            let mut node = Node {
                entry: rows[middle].clone(),
                left: build(&rows[..middle]),
                right: build(&rows[middle + 1..]),
                height: 0,
                len: 0,
            };
            node.update();
            Some(Arc::new(node))
        }
        Self {
            root: build(&unique),
        }
    }
}

/// The launch checkpoint (storage spec §7.6) stores a map as its ordered
/// (key, value) sequence and rebuilds it with the O(N) sorted bulk build.
impl<K: serde::Serialize + Ord, V: serde::Serialize> serde::Serialize for Map<K, V> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        // Postcard needs the length up front; the tree iterator cannot say it.
        let mut seq = s.serialize_seq(Some(self.len()))?;
        for entry in self.iter() {
            seq.serialize_element(&entry)?;
        }
        seq.end()
    }
}
impl<'de, K: serde::Deserialize<'de> + Ord, V: serde::Deserialize<'de>> serde::Deserialize<'de>
    for Map<K, V>
{
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Vec::<(K, V)>::deserialize(d)?.into_iter().collect())
    }
}

use super::{Document, PageEntry};
type Page = (PageEntry, Arc<Document>);
/// Stable slots retain the original iteration order through deletion. New
/// pages append; removed slots disappear without shifting any survivor/index.
#[derive(Clone, Default)]
pub(crate) struct Pages {
    rows: Map<usize, Page>,
    next: usize,
    pub(crate) positions: Arc<Map<String, usize>>,
}
impl Pages {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Page> {
        self.rows.values()
    }
    pub(crate) fn slots(&self) -> impl Iterator<Item = (usize, &Page)> {
        self.rows.iter().map(|(&i, p)| (i, p))
    }
    pub(crate) fn get(&self, slot: usize) -> Option<&Page> {
        self.rows.get(&slot)
    }
    pub(crate) fn get_mut(&mut self, slot: usize) -> Option<&mut Page> {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::cache_page_copies(1);
        self.rows.get_mut(&slot)
    }
    pub(crate) fn push(&mut self, page: Page) -> usize {
        let slot = self.next;
        self.next += 1;
        Arc::make_mut(&mut self.positions).insert(page.0.rel_path_str().to_owned(), slot);
        self.rows.insert(slot, page);
        slot
    }
    pub(crate) fn remove(&mut self, slot: usize) {
        if let Some(page) = self.rows.get(&slot) {
            Arc::make_mut(&mut self.positions).remove(page.0.rel_path_str());
        }
        self.rows.remove(&slot);
    }
}
impl Pages {
    /// The next free slot (the checkpoint keeps slots stable across a reload).
    pub(crate) fn next_slot(&self) -> usize {
        self.next
    }
    /// Pages at their recorded slots, as a launch checkpoint stored them.
    pub(crate) fn from_slots(rows: Vec<(usize, Page)>, next: usize) -> Self {
        Self {
            positions: Arc::new(
                rows.iter()
                    .map(|(slot, p)| (p.0.rel_path_str().to_owned(), *slot))
                    .collect(),
            ),
            rows: rows.into_iter().collect(),
            next,
        }
    }
}
impl From<Vec<Page>> for Pages {
    fn from(pages: Vec<Page>) -> Self {
        Self {
            next: pages.len(),
            positions: Arc::new(
                pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.0.rel_path_str().to_owned(), i))
                    .collect(),
            ),
            rows: pages.into_iter().enumerate().collect(),
        }
    }
}
impl std::ops::Index<usize> for Pages {
    type Output = Page;
    fn index(&self, i: usize) -> &Page {
        self.get(i).expect("live page slot")
    }
}
impl<'a> IntoIterator for &'a Pages {
    type Item = &'a Page;
    type IntoIter = std::iter::Map<Iter<'a, usize, Page>, fn((&'a usize, &'a Page)) -> &'a Page>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter().map(|(_, p)| p)
    }
}

/// Snapshot page listings keep their order without copying the whole list when
/// one path moves. The legacy Vec result is materialized only by an explicit read.
#[derive(Default)]
pub(crate) struct EntryList {
    rows: Map<usize, PageEntry>,
    paths: Map<std::path::PathBuf, usize>,
    days: Map<i64, usize>,
    next: usize,
    projected: std::sync::OnceLock<Arc<Vec<PageEntry>>>,
}
impl Clone for EntryList {
    fn clone(&self) -> Self {
        Self {
            rows: self.rows.clone(),
            paths: self.paths.clone(),
            days: self.days.clone(),
            next: self.next,
            projected: std::sync::OnceLock::new(),
        }
    }
}
impl EntryList {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &PageEntry> {
        self.rows.values()
    }
    pub(crate) fn remove_path(&mut self, path: &std::path::Path) {
        if let Some(slot) = self.paths.get(path).copied() {
            if let Some(day) = self.rows.get(&slot).and_then(|e| e.date_key) {
                if self.days.get(&day) == Some(&slot) {
                    self.days.remove(&day);
                }
            }
            self.rows.remove(&slot);
            self.paths.remove(path);
            self.projected.take();
        }
    }
    pub(crate) fn remove_day(&mut self, day: i64) {
        if let Some(path) = self
            .days
            .get(&day)
            .and_then(|slot| self.rows.get(slot))
            .map(|e| e.path.clone())
        {
            self.remove_path(&path);
        }
    }
    pub(crate) fn push(&mut self, entry: PageEntry) {
        let slot = self.next;
        self.next += 1;
        self.paths.insert(entry.path.clone(), slot);
        if entry.kind == super::PageKind::Journal {
            if let Some(day) = entry.date_key {
                self.days.insert(day, slot);
            }
        }
        self.rows.insert(slot, entry);
        self.projected.take();
    }
    pub(crate) fn materialize(&self) -> Arc<Vec<PageEntry>> {
        Arc::clone(
            self.projected
                .get_or_init(|| Arc::new(self.iter().cloned().collect())),
        )
    }
}
/// An `EntryList`'s fields, as the launch checkpoint stores them (entry
/// paths are rebuilt from their graph-relative identity on load).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct EntryListParts {
    pub(crate) rows: Map<usize, PageEntry>,
    pub(crate) paths: Map<std::path::PathBuf, usize>,
    pub(crate) days: Map<i64, usize>,
    pub(crate) next: usize,
}
impl EntryList {
    pub(crate) fn to_parts(&self) -> EntryListParts {
        EntryListParts {
            rows: self.rows.clone(),
            paths: self.paths.clone(),
            days: self.days.clone(),
            next: self.next,
        }
    }
    pub(crate) fn from_parts(parts: EntryListParts) -> Self {
        Self {
            rows: parts.rows,
            paths: parts.paths,
            days: parts.days,
            next: parts.next,
            projected: std::sync::OnceLock::new(),
        }
    }
}
impl From<&[PageEntry]> for EntryList {
    fn from(entries: &[PageEntry]) -> Self {
        Self {
            rows: entries.iter().cloned().enumerate().collect(),
            paths: entries
                .iter()
                .enumerate()
                .map(|(i, e)| (e.path.clone(), i))
                .collect(),
            days: entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    (e.kind == super::PageKind::Journal)
                        .then_some(e.date_key)
                        .flatten()
                        .map(|d| (d, i))
                })
                .collect(),
            next: entries.len(),
            projected: std::sync::OnceLock::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn persistent_mutations_match_ordered_map_and_retain_old_roots() {
        fn check(node: &Option<Arc<Node<u32, u32>>>) -> (usize, usize) {
            let Some(n) = node else {
                return (0, 0);
            };
            let (lh, ll) = check(&n.left);
            let (rh, rl) = check(&n.right);
            assert!(lh.abs_diff(rh) <= 1);
            assert_eq!(n.height, lh.max(rh) + 1);
            assert_eq!(n.len, ll + rl + 1);
            (n.height, n.len)
        }
        let mut tree = Map::new();
        let mut oracle = BTreeMap::new();
        let mut history = Vec::new();
        let mut rng = 0x12345678u32;
        for step in 0..4000 {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = (rng >> 8) % 257;
            if step % 47 == 0 {
                history.push((tree.clone(), oracle.clone()));
            }
            match rng % 3 {
                0 => {
                    tree.remove(&key);
                    oracle.remove(&key);
                }
                1 => {
                    tree.insert(key, rng);
                    oracle.insert(key, rng);
                }
                _ => {
                    if let Some(v) = tree.get_mut(&key) {
                        *v ^= rng;
                    }
                    if let Some(v) = oracle.get_mut(&key) {
                        *v ^= rng;
                    }
                }
            }
            check(&tree.root);
            assert_eq!(
                tree.iter().collect::<Vec<_>>(),
                oracle.iter().collect::<Vec<_>>()
            );
        }
        for (old, expected) in history {
            check(&old.root);
            assert_eq!(
                old.iter().collect::<Vec<_>>(),
                expected.iter().collect::<Vec<_>>()
            );
        }
        let bulk: Map<_, _> = [(1, 2), (1, 3), (0, 4)].into_iter().collect();
        assert_eq!(
            bulk.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            vec![(0, 4), (1, 3)]
        );
    }
}
