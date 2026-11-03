//! All pages backlink counts over one graph snapshot (D-20261102-CM4
//! CM-COUNT): each page's count is the number of blocks its unfiltered Linked
//! References panel lists ([`crate::query::page_counts`]).
use crate::store::WholeGraph;
use tine_core::model::PageKind;
use tine_core::refs;

impl WholeGraph {
    /// The counts All pages shows, unfiltered (A14), as `(files, references)`:
    /// - `files`: one row per file that owns its name (the first claimant of
    ///   its kind, [`WholeGraph::resolve`]'s `id`), by graph-relative path. A
    ///   non-owner twin has no row, so All pages shows a dash for it
    ///   (D-20261102-CM4 A13): its name opens the owner's panel.
    /// - `references`: one row per reference-only page (`inventory(All)`'s
    ///   absent entries without a `link`), by that entry's name.
    ///
    /// One forward walk per call, O(P + B + R + T), with its fold table
    /// dropped on return; no file reads and no new record. All pages calls
    /// it on the HK3 quiet signal, one call in flight.
    ///
    /// I-12 exception (CM step 2): the counter and the panel's
    /// `backlinks_bounded` are two drivers of one reference rule. Retire: one
    /// forward target map (D3).
    pub fn page_backlink_counts(&self) -> (Vec<(String, usize)>, Vec<(String, usize)>) {
        let counts = crate::query::page_counts::referring_block_counts(&self.graph);
        let count = |name: &str| counts.get(&refs::page_key(name)).map_or(0, |&n| n as usize);
        let mut files = Vec::new();
        for entry in self.list.iter() {
            let Some(path) = entry.rel_path.as_ref() else {
                continue;
            };
            let owner = match self.resolve(&entry.name, entry.kind == PageKind::Journal) {
                crate::Resolved::Existing { id, .. } => id,
                _ => continue,
            };
            if owner.as_str() != path.as_str() {
                continue;
            }
            files.push((path.to_string(), count(&entry.name)));
        }
        let references = self
            .graph
            .reference_only_page_names()
            .into_iter()
            .map(|name| (name.to_owned(), count(name)))
            .collect();
        (files, references)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    /// D-20261102-CM4 CM-COUNT: a block counts once however it names the page
    /// (link, tag, property value, page properties); the page's own blocks and
    /// code spans do not count, matching the Linked References panel. Replaces
    /// `referring_blocks_count_once_including_properties_and_self_but_not_code`
    /// (retired by CM4: it pinned own-block counting).
    #[test]
    fn referring_blocks_count_once_as_the_panel_lists_them() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::create_dir(root.path().join("journals")).unwrap();
        std::fs::write(root.path().join("pages/Target.md"), "- [[Target]]\n").unwrap();
        std::fs::write(
            root.path().join("pages/Source.md"),
            "link:: [[Target]]\n\n- [[Target]] [[Target]] [[Ref only]]\n  - #Target\n- `[[Target]]`\n",
        )
        .unwrap();
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        let (files, references) = store.whole_graph().unwrap().page_backlink_counts();
        let files: HashMap<_, _> = files.into_iter().collect();
        assert_eq!(files.get("pages/Target.md"), Some(&3));
        assert_eq!(files.get("pages/Source.md"), Some(&0));
        // `link::` makes `link` a property-key page (CM4), named by one block.
        assert_eq!(
            references,
            vec![("link".to_owned(), 1), ("Ref only".to_owned(), 1)]
        );
    }

    /// CM step 2 batch 5 ruling: a block embed `{{embed ((uuid))}}` names no
    /// page. Its panel would list nothing (the Linked References rule never
    /// names it), so it is no reference-only page either.
    #[test]
    fn a_block_embed_names_no_reference_only_page() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::write(
            root.path().join("pages/A.md"),
            "- {{embed ((00000000-0000-4000-8000-000000000001))}} [[Ref only]]\n- target\n  id:: 00000000-0000-4000-8000-000000000001\n",
        )
        .unwrap();
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        let (_, references) = store.whole_graph().unwrap().page_backlink_counts();
        assert_eq!(references, vec![("Ref only".to_owned(), 1)]);
    }
}
