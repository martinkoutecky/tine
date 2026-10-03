//! A save that only folds or unfolds blocks (GH #623 item 3).
//!
//! Folding writes `collapsed:: true` into a block, so the file is saved like
//! any edit. No answer the graph derives reads that property: reference
//! evidence treats it as structural (`reference_evidence::structural_property`),
//! queries and the property registry exclude it (`query::internal_property_keys`)
//! and, with its value limited to `true`/`false`, it holds no block reference
//! or page name. A fold therefore changes no index, memo or claimant, and the
//! publication carries them instead of re-deriving them.
use std::sync::Arc;
use tine_core::block_regions::{edit, Edit};
use tine_core::doc::{DocBlock, Document};

use super::ReadSnapshot;

/// Whether `current` differs from `previous`, and only in blocks'
/// `collapsed::` property: the same pre-block, the same outline shape, every
/// changed block equal once the parser removes that property, and the
/// property's value `true`, `false` or absent on both sides. Cost O(blocks),
/// plus one block parse per changed block.
pub(crate) fn collapse_only(previous: &Document, current: &Document) -> bool {
    let mut changed = false;
    previous.pre_block == current.pre_block
        && same_outline(&previous.roots, &current.roots, &mut changed)
        && changed
}

fn same_outline(previous: &[DocBlock], current: &[DocBlock], changed: &mut bool) -> bool {
    previous.len() == current.len()
        && previous.iter().zip(current).all(|(old, new)| {
            same_block(old, new, changed) && same_outline(&old.children, &new.children, changed)
        })
}

fn same_block(old: &DocBlock, new: &DocBlock, changed: &mut bool) -> bool {
    if old.raw() == new.raw() {
        return true;
    }
    *changed = true;
    let plain = |block: &DocBlock| {
        matches!(
            block.property("collapsed").as_deref(),
            None | Some("true") | Some("false")
        )
    };
    let unfolded = |block: &DocBlock| {
        edit(
            block.raw(),
            block.is_org(),
            Edit::Property {
                key: "collapsed".into(),
                value: None,
            },
        )
        .ok()
    };
    old.is_org() == new.is_org()
        && plain(old)
        && plain(new)
        && unfolded(old).is_some_and(|old| Some(old) == unfolded(new))
}

impl ReadSnapshot {
    /// The documents `changed_paths` held in `old` and hold here, or `None`
    /// when a path is missing from either (created, removed, not cached).
    pub(crate) fn edited_pages(
        &self,
        old: &Self,
        changed_paths: &[String],
    ) -> Option<Vec<(tine_core::model::PageEntry, Arc<Document>, Arc<Document>)>> {
        let old_positions = Arc::clone(&old.reference_candidate_index.read().unwrap().positions);
        let new_positions = Arc::clone(&self.reference_candidate_index.read().unwrap().positions);
        let mut edits = Vec::new();
        for path in changed_paths {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::memo_page_probes(2);
            let before = old_positions
                .get(path)
                .and_then(|&i| old.pages.get(i))
                .filter(|(e, _)| e.rel_path_str() == path);
            let after = new_positions
                .get(path)
                .and_then(|&i| self.pages.get(i))
                .filter(|(e, _)| e.rel_path_str() == path);
            let (Some((_, previous)), Some((entry, current))) = (before, after) else {
                return None;
            };
            edits.push((entry.clone(), Arc::clone(previous), Arc::clone(current)));
        }
        Some(edits)
    }
}

#[cfg(test)]
mod tests {
    use super::collapse_only;
    use tine_core::doc::{parse, Document};

    fn doc(text: &str) -> Document {
        parse(text)
    }

    #[test]
    fn a_fold_alone_is_collapse_only_and_any_other_change_is_not() {
        let open = "- parent [[A]]\n  - child\n- other\n";
        let folded = "- parent [[A]]\n  collapsed:: true\n  - child\n- other\n";
        assert!(collapse_only(&doc(open), &doc(folded)));
        assert!(collapse_only(&doc(folded), &doc(open)));
        assert!(!collapse_only(&doc(open), &doc(open)), "nothing changed");
        for other in [
            "- parent [[B]]\n  collapsed:: true\n  - child\n- other\n",
            "- parent [[A]]\n  collapsed:: true\n  - child\n- other edited\n",
            "- parent [[A]]\n  collapsed:: true\n  - child\n",
            "- parent [[A]]\n  collapsed:: true\n  tags:: x\n  - child\n- other\n",
            "- parent [[A]]\n  collapsed:: [[Page]]\n  - child\n- other\n",
            "title:: T\n\n- parent [[A]]\n  collapsed:: true\n  - child\n- other\n",
        ] {
            assert!(!collapse_only(&doc(open), &doc(other)), "{other}");
        }
    }
}
