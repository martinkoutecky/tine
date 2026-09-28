//! The ONE `:block/path-refs` closure (SPEC §5.8, K22) and the ONE depth-first
//! walk that maintains it.
//!
//! OG materializes `:block/path-refs` per block: the block's own references,
//! every ancestor's, and the page it lives on. Two consumers need exactly
//! that set — the document walk's `refs` leaf and Direct Files'
//! `block_path_refs` rows — and a second implementation of it would be a
//! parity defect waiting to happen (I-19, I-12). So the counted ancestor
//! multiset, the membership rule and the traversal that maintains them live
//! here, and every consumer is a thin adapter over [`dfs_path_refs`].
//!
//! The state is O(nodes + rows) and transient: one counter per distinct
//! ancestor name, never a per-block ancestor vector.

use std::collections::HashMap;

/// The ancestor reference multiset. Counted, not a set: two ancestors may name
/// the same page, and popping one of them must not drop the name.
pub type PathRefCounts = HashMap<String, usize>;

/// Enter a node: its normalized references join the ancestor multiset.
pub fn push_refs(refs: &[String], counts: &mut PathRefCounts) {
    for reference in refs {
        *counts.entry(reference.clone()).or_default() += 1;
    }
}

/// Leave a node: its normalized references leave the ancestor multiset.
pub fn pop_refs(refs: &[String], counts: &mut PathRefCounts) {
    for reference in refs {
        let remove = if let Some(count) = counts.get_mut(reference) {
            *count -= 1;
            *count == 0
        } else {
            false
        };
        if remove {
            counts.remove(reference);
        }
    }
}

/// Whether `normalized` is in one block's path-refs closure.
///
/// The closure is the block's own normalized refs, its ancestors', and the
/// normalized name of the page it lives on. `page_key` is that page name
/// already put through [`crate::refs::normalize`] by the caller, because every
/// caller has it hoisted out of its own loop.
pub fn closure_contains(
    page_key: &str,
    own: &[String],
    ancestors: &PathRefCounts,
    normalized: &str,
) -> bool {
    own.iter().any(|name| name == normalized)
        || ancestors.contains_key(normalized)
        || page_key == normalized
}

/// One block's complete path-refs closure, de-duplicated and sorted.
///
/// Sorted because these become table rows two independent builds must agree on
/// byte for byte (§5.8 guard (a)); the closure itself is a set, so the order is
/// free to be the canonical one.
pub fn closure_names(page_key: &str, own: &[String], ancestors: &PathRefCounts) -> Vec<String> {
    let mut names: Vec<String> = own
        .iter()
        .cloned()
        .chain(ancestors.keys().cloned())
        .chain(std::iter::once(page_key.to_owned()))
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// What [`dfs_path_refs`] reports about each node.
///
/// One visitor rather than two closures because the interesting consumer -- the
/// OG result walk -- keeps a breadcrumb path and a matched-ancestor flag that
/// both hooks must own together.
pub trait PathRefVisitor<'a, N> {
    /// Pre-order, **before** the node's own refs join the multiset, so
    /// `ancestors` holds the ancestors' refs alone -- which is what a closure
    /// membership test wants, since the node's own refs are in hand separately.
    fn enter(&mut self, node: &'a N, ancestors: &PathRefCounts);
    /// Post-order, after the node's own refs have left the multiset again.
    fn leave(&mut self, node: &'a N) {
        let _ = node;
    }
}

impl<'a, N: 'a, F> PathRefVisitor<'a, N> for F
where
    F: FnMut(&'a N, &PathRefCounts),
{
    fn enter(&mut self, node: &'a N, ancestors: &PathRefCounts) {
        self(node, ancestors);
    }
}

/// The ONE depth-first traversal that maintains [`PathRefCounts`].
///
/// `track` is the walk's existing escape hatch: a query that never reads
/// `refs` does not pay for the counters. Producers always track.
pub fn dfs_path_refs<'a, N: 'a, C, R, V>(
    nodes: &'a [N],
    children: &C,
    refs: &R,
    counts: &mut PathRefCounts,
    track: bool,
    visitor: &mut V,
) where
    C: for<'x> Fn(&'x N) -> &'x [N],
    R: for<'x> Fn(&'x N) -> &'x [String],
    V: PathRefVisitor<'a, N>,
{
    for node in nodes {
        visitor.enter(node, counts);
        if track {
            push_refs(refs(node), counts);
        }
        dfs_path_refs(children(node), children, refs, counts, track, visitor);
        if track {
            pop_refs(refs(node), counts);
        }
        visitor.leave(node);
    }
}

/// One block of the flat row form the projection producers hold: an id, its
/// parent, and the block's OWN normalized references — `BlockProjection`'s
/// `refs_norm`, the walk's exact source, on both backends (§5.8 G1).
///
/// Reference postings are deliberately not an input: the walk's exact source
/// is `refs_norm`, and the postings encode more than that (they label
/// `tags::`/`alias::` values kind 3), so a kind filter over them would first
/// have to be proven equivalent.
#[derive(Debug, Clone, Copy)]
pub struct PathRefBlock<'a, Id> {
    pub id: Id,
    pub parent: Option<Id>,
    pub refs: &'a [String],
}

/// One node of the transient tree the flat form is threaded onto. Holding a
/// slice rather than the names keeps this allocation-light: one `Vec` of child
/// nodes per block, no copied strings.
struct FlatNode<'a, Id> {
    id: Id,
    refs: &'a [String],
    children: Vec<FlatNode<'a, Id>>,
}

/// SPEC §5.8's shared closure function: a pure, streaming function over flat
/// rows that needs no `Document`.
///
/// It builds the child lists, runs [`dfs_path_refs`] once, and emits
/// `(block_id, name)` rows as it goes. `page_name` is the raw page name; the
/// caller need not normalize it.
///
/// A row whose parent is not in `blocks` is treated as a root rather than
/// dropped, and every block is visited exactly once, so a malformed parent
/// chain loses no rows and cannot loop.
pub fn path_refs_closure<Id, F>(page_name: &str, blocks: &[PathRefBlock<'_, Id>], mut emit: F)
where
    Id: Copy + Eq + std::hash::Hash,
    F: FnMut(Id, &str),
{
    let page_key = crate::refs::normalize(page_name);
    let index: HashMap<Id, usize> = blocks
        .iter()
        .enumerate()
        .map(|(at, block)| (block.id, at))
        .collect();
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (at, block) in blocks.iter().enumerate() {
        match block.parent.and_then(|parent| index.get(&parent)) {
            // A block that is its own ancestor is malformed input, not a
            // reason to hang: it becomes a root and is emitted once.
            Some(&parent) if parent != at => children_of[parent].push(at),
            _ => roots.push(at),
        }
    }
    let mut placed = vec![false; blocks.len()];
    let mut forest: Vec<FlatNode<'_, Id>> = Vec::new();
    for &root in &roots {
        placed[root] = true;
        forest.push(build_flat_node(blocks, &children_of, &mut placed, root));
    }
    // A parent cycle leaves every block in it with a parent, so the sweep above
    // finds no root for it. Rooting the survivors in row order emits each of
    // them exactly once instead of silently losing the whole cycle.
    for at in 0..blocks.len() {
        if !std::mem::replace(&mut placed[at], true) {
            forest.push(build_flat_node(blocks, &children_of, &mut placed, at));
        }
    }
    let mut counts = PathRefCounts::new();
    dfs_path_refs(
        &forest,
        &|node: &FlatNode<'_, Id>| &node.children[..],
        &|node: &FlatNode<'_, Id>| node.refs,
        &mut counts,
        true,
        &mut |node: &FlatNode<'_, Id>, ancestors: &PathRefCounts| {
            for name in closure_names(&page_key, node.refs, ancestors) {
                emit(node.id, &name);
            }
        },
    );
}

fn build_flat_node<'a, Id: Copy>(
    blocks: &[PathRefBlock<'a, Id>],
    children_of: &[Vec<usize>],
    placed: &mut [bool],
    at: usize,
) -> FlatNode<'a, Id> {
    let children = children_of[at]
        .iter()
        .filter(|&&child| !std::mem::replace(&mut placed[child], true))
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .map(|child| build_flat_node(blocks, children_of, placed, child))
        .collect();
    FlatNode {
        id: blocks[at].id,
        refs: blocks[at].refs,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(refs: &[&str]) -> Vec<String> {
        refs.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn closure_is_own_refs_ancestors_and_the_page() {
        let a = names(&["alpha"]);
        let b = names(&["beta"]);
        let blocks = vec![
            PathRefBlock {
                id: 1_u32,
                parent: None,
                refs: &a,
            },
            PathRefBlock {
                id: 2,
                parent: Some(1),
                refs: &b,
            },
        ];
        let mut rows: Vec<(u32, String)> = Vec::new();
        path_refs_closure("Home", &blocks, |id, name| rows.push((id, name.to_owned())));
        assert_eq!(
            rows,
            vec![
                (1, "alpha".to_owned()),
                (1, "home".to_owned()),
                (2, "alpha".to_owned()),
                (2, "beta".to_owned()),
                (2, "home".to_owned()),
            ]
        );
    }

    #[test]
    fn a_repeated_ancestor_name_survives_its_first_pop() {
        let shared = names(&["shared"]);
        let empty: Vec<String> = Vec::new();
        // root -> mid(shared) -> leaf, with a sibling of `mid` also naming
        // `shared`: the counted multiset must not drop the name when the first
        // occurrence is popped.
        let blocks = vec![
            PathRefBlock {
                id: 1_u32,
                parent: None,
                refs: &shared,
            },
            PathRefBlock {
                id: 2,
                parent: Some(1),
                refs: &shared,
            },
            PathRefBlock {
                id: 3,
                parent: Some(2),
                refs: &empty,
            },
        ];
        let mut rows: Vec<(u32, String)> = Vec::new();
        path_refs_closure("Home", &blocks, |id, name| rows.push((id, name.to_owned())));
        assert!(rows.contains(&(3, "shared".to_owned())));
        assert_eq!(rows.iter().filter(|(id, _)| *id == 3).count(), 2);
    }

    #[test]
    fn a_cyclic_parent_chain_emits_every_block_once() {
        let empty: Vec<String> = Vec::new();
        let blocks = vec![
            PathRefBlock {
                id: 1_u32,
                parent: Some(2),
                refs: &empty,
            },
            PathRefBlock {
                id: 2,
                parent: Some(1),
                refs: &empty,
            },
        ];
        let mut rows: Vec<(u32, String)> = Vec::new();
        path_refs_closure("Home", &blocks, |id, name| rows.push((id, name.to_owned())));
        assert_eq!(rows.len(), 2);
    }
}
