//! The ONE `:block/path-refs` closure (SPEC §5.8, K22) and the ONE depth-first
//! walk that maintains it.
//!
//! OG materializes `:block/path-refs` per block: the block's own references
//! (including namespace ancestors), every ancestor block's, and the page it lives on. The
//! document walk's `refs` leaf needs exactly that set, and a second
//! implementation of it would be a parity defect waiting to happen (I-19,
//! I-12). So the counted ancestor multiset, the membership rule and the
//! traversal that maintains them live here, and every consumer is a thin
//! adapter over [`dfs_path_refs`].
//!
//! The state is O(nodes + rows) and transient: one counter per distinct
//! ancestor name, never a per-block ancestor vector.

use std::collections::HashMap;

/// The ancestor reference multiset. Counted, not a set: two ancestors may name
/// the same page, and popping one of them must not drop the name.
pub type PathRefCounts = HashMap<String, usize>;

/// Enter a node: its normalized references join the ancestor multiset.
pub fn push_refs(refs: &[String], counts: &mut PathRefCounts) {
    for reference in refs
        .iter()
        .flat_map(|name| crate::refs::namespace_refs(name))
    {
        *counts.entry(reference.to_owned()).or_default() += 1;
    }
}

/// Leave a node: its normalized references leave the ancestor multiset.
pub fn pop_refs(refs: &[String], counts: &mut PathRefCounts) {
    for reference in refs
        .iter()
        .flat_map(|name| crate::refs::namespace_refs(name))
    {
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
/// The closure is the block's own normalized refs and their namespace ancestors,
/// its ancestors' expanded refs, and the
/// normalized name of the page it lives on. `page_key` is that page name
/// already put through [`crate::refs::normalize`] by the caller, because every
/// caller has it hoisted out of its own loop.
pub fn closure_contains(
    page_key: &str,
    own: &[String],
    ancestors: &PathRefCounts,
    normalized: &str,
) -> bool {
    own.iter()
        .flat_map(|name| crate::refs::namespace_refs(name))
        .any(|name| name == normalized)
        || ancestors.contains_key(normalized)
        || page_key == normalized
}

/// One block's complete path-refs closure, de-duplicated and sorted.
///
/// Sorted so the answer is deterministic: the closure itself is a set, so the
/// order is free to be the canonical one.
pub fn closure_names(page_key: &str, own: &[String], ancestors: &PathRefCounts) -> Vec<String> {
    let mut names: Vec<String> = own
        .iter()
        .flat_map(|name| crate::refs::namespace_refs(name))
        .map(str::to_owned)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_closure_membership_rows_and_counted_pop_agree() {
        let own = names(&["a/b/c", "a/b"]);
        let mut counts = PathRefCounts::new();
        push_refs(&own, &mut counts);
        push_refs(&names(&["a/d"]), &mut counts);
        pop_refs(&own, &mut counts);
        assert_eq!(
            closure_names("home", &[], &counts),
            names(&["a", "a/d", "home"])
        );
        assert!(closure_contains("home", &[], &counts, "a"));
        assert!(!closure_contains("home", &[], &counts, "a/b"));
        assert_eq!(
            closure_names("home", &own, &counts),
            names(&["a", "a/b", "a/b/c", "a/d", "home"])
        );
        assert!(closure_contains("home", &own, &counts, "a/b"));
        assert!(!closure_contains("home", &own, &counts, "a/bb"));
        // Owning-page identity does not itself imply namespace ancestry.
        assert!(!closure_contains("a/b", &[], &PathRefCounts::new(), "a"));
    }

    fn names(refs: &[&str]) -> Vec<String> {
        refs.iter().map(|name| (*name).to_owned()).collect()
    }

    /// A tiny owned tree, walked through the one [`dfs_path_refs`].
    struct Node {
        id: u32,
        refs: Vec<String>,
        children: Vec<Node>,
    }

    fn node(id: u32, refs: &[&str], children: Vec<Node>) -> Node {
        Node {
            id,
            refs: names(refs),
            children,
        }
    }

    /// `(block id, closure name)` rows in walk order, as a producer emits them.
    fn closure_rows(page: &str, roots: &[Node]) -> Vec<(u32, String)> {
        let page_key = crate::refs::normalize(page);
        let mut rows = Vec::new();
        dfs_path_refs(
            roots,
            &|node: &Node| &node.children[..],
            &|node: &Node| &node.refs[..],
            &mut PathRefCounts::new(),
            true,
            &mut |node: &Node, ancestors: &PathRefCounts| {
                for name in closure_names(&page_key, &node.refs, ancestors) {
                    rows.push((node.id, name));
                }
            },
        );
        rows
    }

    #[test]
    fn closure_is_own_refs_ancestors_and_the_page() {
        let roots = vec![node(1, &["alpha"], vec![node(2, &["beta"], Vec::new())])];
        assert_eq!(
            closure_rows("Home", &roots),
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
        // root(shared) -> mid(shared) -> leaf: the counted multiset must not
        // drop the name when the first occurrence is popped.
        let roots = vec![node(
            1,
            &["shared"],
            vec![node(2, &["shared"], vec![node(3, &[], Vec::new())])],
        )];
        let rows = closure_rows("Home", &roots);
        assert!(rows.contains(&(3, "shared".to_owned())));
        assert_eq!(rows.iter().filter(|(id, _)| *id == 3).count(), 2);
    }
}
