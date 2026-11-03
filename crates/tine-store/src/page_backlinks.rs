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
    /// One forward walk per call, O(P + B + R + T + F) (F the (block, panel)
    /// pairs reached; panels registered under their names, never a forward
    /// fold), its tables dropped on return; no file reads and no new record. All pages calls
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

    /// UI-CM-ALLPAGES-COUNT-ZERO-FOR-ALIAS-GROUP (batch 5 regression): every
    /// All pages count equals the size of the panel its name opens, also for
    /// a page reached through an alias, an alias chain or a journal-date
    /// spelling. Runs without the model bridge (`CM_BRIDGE`).
    #[test]
    fn every_count_is_its_panel_size_through_aliases_and_date_spellings() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals"] {
            std::fs::create_dir(root.path().join(dir)).unwrap();
        }
        let write = |path: &str, text: &str| std::fs::write(root.path().join(path), text).unwrap();
        // An alias member without a file (ML), a chain (B -> C -> D) and a
        // journal named by its title and by a looser date spelling.
        write("pages/Machine Learning.md", "alias:: ML\n\n- about it\n");
        write("pages/B.md", "alias:: C\n\n- b\n");
        write("pages/C.md", "alias:: D\n\n- c\n");
        write("journals/2026_10_27.md", "- the day\n");
        write(
            "pages/Notes.md",
            concat!(
                "- see [[ML]]\n",
                "- see [[Machine Learning]] and [[ML]]\n",
                "- chain [[C]]\n",
                "- chain [[D]]\n",
                "- on [[Oct 27th, 2026]]\n",
                "- on [[2026-10-27]]\n",
                "- [[Ref only]]\n",
            ),
        );
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        let view = store.whole_graph().unwrap();
        let panel = |name: &str| -> usize {
            view.backlinks(name)
                .unwrap()
                .iter()
                .map(|group| group.blocks.len())
                .sum()
        };
        let (files, references) = view.page_backlink_counts();
        let names: HashMap<String, String> = view
            .inventory(crate::InventoryScope::All)
            .0
            .iter()
            .filter_map(|entry| match &entry.target {
                crate::Resolved::Existing { id, .. } => {
                    Some((id.as_str().to_owned(), entry.name.clone()))
                }
                _ => None,
            })
            .collect();
        let mut seen = 0;
        for (path, count) in &files {
            let name = &names[path];
            assert_eq!(*count, panel(name), "{path} ({name}): count vs panel");
            seen += 1;
        }
        for (name, count) in &references {
            assert_eq!(*count, panel(name), "reference-only {name}: count vs panel");
        }
        assert_eq!(seen, 5, "every file has a row: {files:?}");
        let files: HashMap<_, _> = files.into_iter().collect();
        assert_eq!(
            files["pages/Machine Learning.md"], 2,
            "two blocks name ML or Machine Learning"
        );
        assert_eq!(
            files["pages/C.md"], 3,
            "C and D links, and B's alias:: C block"
        );
        assert_eq!(files["journals/2026_10_27.md"], 2, "both date spellings");
        // The lean counter's own table agrees, key by key.
        let counts = crate::query::page_counts::referring_block_counts(&view.graph);
        for (path, count) in &files {
            let key = tine_core::refs::page_key(&names[path]);
            assert_eq!(
                counts.get(&key).copied().unwrap_or(0) as usize,
                *count,
                "{key}"
            );
        }
    }
}
