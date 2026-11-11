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
    ///   absent entries), by that entry's canonical name. An absent journal's
    ///   link can retain an unclaimed spelling beside an ordinary same-title file.
    ///
    /// O(P) output over the snapshot's shared Linked References membership
    /// owner. Its first read registers targets, then constructs memberships;
    /// publications then replace changed source contributions. No file reads.
    pub fn page_backlink_counts(&self) -> (Vec<(String, usize)>, Vec<(String, usize)>) {
        let counts = crate::query::page_counts::referring_block_counts(&self.graph);
        let count = |name: &str, journal: bool| {
            counts
                .get(&(journal, refs::page_key(name)))
                .map_or(0, |&n| n as usize)
        };
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
            files.push((
                path.to_string(),
                count(&entry.name, entry.kind == PageKind::Journal),
            ));
        }
        let references = self
            .graph
            .reference_only_pages()
            .into_iter()
            .map(|answer| {
                let value = count(&answer.name, answer.day.is_some());
                (answer.name, value)
            })
            .collect();
        (files, references)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    fn save(
        store: &Arc<crate::Store>,
        path: &str,
        edit: impl FnOnce(&mut tine_core::PageDto),
    ) -> serde_json::Value {
        let changes = store.subscribe();
        let id = crate::PageId::from(path);
        let mut read = store.page(&id).unwrap();
        edit(&mut read.doc);
        crate::test_host_writer::HostWriter::start(store)
            .save(
                crate::EditKind::ReplacePage,
                &id,
                crate::SaveBase::Existing(read.rev),
                &read.doc,
            )
            .unwrap();
        let change = changes.try_recv().unwrap().expect("save publication");
        assert!(
            changes.try_recv().unwrap().is_none(),
            "one publication per edit"
        );
        serde_json::to_value(change).unwrap()
    }

    fn fixture(extra: usize, source_blocks: usize) -> (tempfile::TempDir, Arc<crate::Store>) {
        let root = tempfile::tempdir().unwrap();
        for name in ["pages", "journals", "logseq"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        for (path, body) in [
            ("pages/Target.md", "alias:: Twin\n\n- [[Target]]\n"),
            ("pages/Other.md", "- other\n"),
            ("journals/2026_10_27.md", "- day\n"),
        ] {
            std::fs::write(root.path().join(path), body).unwrap();
        }
        let source: String = std::iter::once("- [[Target]] [[Twin]]\n")
            .chain(std::iter::repeat_n(
                "- unchanged block\n",
                source_blocks - 1,
            ))
            .collect();
        std::fs::write(root.path().join("pages/Source.md"), source).unwrap();
        for at in 0..extra {
            std::fs::write(
                root.path().join(format!("pages/Unrelated {at}.md")),
                "- unrelated\n",
            )
            .unwrap();
        }
        let before =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        for entry in std::fs::read_dir(root.path().join("pages")).unwrap() {
            std::fs::File::options()
                .write(true)
                .open(entry.unwrap().path())
                .unwrap()
                .set_modified(before)
                .unwrap();
        }
        let store = Arc::new(
            crate::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        store.stop_watcher_for_tests();
        store.whole_graph().unwrap().page_backlink_counts();
        (root, store)
    }

    #[test]
    fn publications_patch_one_stamp_and_only_affected_counts() {
        let mut metric_sizes = Vec::new();
        for source_blocks in [1, 60] {
            let (root, store) = fixture(100, source_blocks);
            let held = store.whole_graph().unwrap();
            let started = std::time::Instant::now();
            let body = save(&store, "pages/Source.md", |page| {
                page.blocks[0].raw = "plain [[Target]] [[Twin]]".to_owned()
            });
            assert_eq!(body["pageModified"].as_array().unwrap().len(), 1);
            assert_eq!(body["pageModified"][0][0], "pages/Source.md");
            assert_eq!(body["pageBacklinkCounts"], serde_json::json!([]));
            let metric_bytes = serde_json::to_vec(&serde_json::json!({"pageModified": body["pageModified"], "pageBacklinkCounts": body["pageBacklinkCounts"]})).unwrap().len();
            metric_sizes.push(metric_bytes);
            eprintln!("Unit cost: source_blocks={source_blocks} metric_bytes={metric_bytes} publication_bytes={} graph_file_bytes={}", serde_json::to_vec(&body).unwrap().len(), std::fs::metadata(root.path().join("pages/Source.md")).unwrap().len());
            let view = store.whole_graph().unwrap();
            let work = view.graph.resident_backlink_memberships().unwrap();
            assert_eq!((work.full_builds, work.source_visits), (0, 1));
            assert_eq!(
                work.snapshot_reads
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
            let reference = save(&store, "pages/Source.md", |page| {
                page.blocks[0].raw = "plain [[Other]]".to_owned()
            });
            assert_eq!(reference["pageModified"].as_array().unwrap().len(), 1);
            assert_eq!(
                reference["pageBacklinkCounts"],
                serde_json::json!([["pages/Other.md", 1], ["pages/Target.md", 0]])
            );
            let view = store.whole_graph().unwrap();
            let work = view.graph.resident_backlink_memberships().unwrap();
            assert_eq!((work.full_builds, work.source_visits), (0, 1));
            assert_eq!(
                work.snapshot_reads
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
            assert_eq!(
                held.page_backlink_counts()
                    .0
                    .into_iter()
                    .collect::<HashMap<_, _>>()["pages/Target.md"],
                1,
                "held snapshot unchanged"
            );
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "two small edits on a 104-page fixture"
            );
        }
        assert_eq!(
            metric_sizes[0], metric_sizes[1],
            "metric transport costs the edited row, not the page size"
        );
    }

    #[test]
    fn zero_count_namespace_rows_are_removed_without_retaining_metric_keys() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::write(root.path().join("pages/Space%2FChild.md"), "- text\n").unwrap();
        let store = Arc::new(
            crate::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        store.stop_watcher_for_tests();
        assert_eq!(
            store.whole_graph().unwrap().page_backlink_counts().1,
            vec![("Space".to_owned(), 0)]
        );
        let wire = save(&store, "pages/Space%2FChild.md", |page| page.blocks.clear());
        assert_eq!(
            wire["pageBacklinkCounts"],
            serde_json::json!([["\0Space", null]])
        );
    }

    #[test]
    fn a_panel_only_owner_leaves_the_unopened_metric_seed_lazy() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::write(root.path().join("pages/Target.md"), "- target\n").unwrap();
        std::fs::write(root.path().join("pages/Source.md"), "- [[Target]]\n").unwrap();
        let store = Arc::new(
            crate::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        store.stop_watcher_for_tests();
        let view = store.whole_graph().unwrap();
        assert_eq!(view.backlinks("Target").unwrap()[0].blocks.len(), 1);
        assert!(!view.graph.day_support_built());
        let wire = save(&store, "pages/Source.md", |page| {
            page.blocks[0].raw = "plain [[Target]]".to_owned()
        });
        assert!(
            wire["pageBacklinkCounts"].is_null(),
            "a pre-seed publication asks an in-flight old seed to retry"
        );
        assert!(!store.whole_graph().unwrap().graph.day_support_built());
    }

    fn assert_membership_fresh(store: &crate::Store) {
        let view = store.whole_graph().unwrap();
        assert_eq!(
            view.graph.scan_all_backlink_memberships().scan_all_counts(),
            view.graph.fresh_backlink_counts()
        );
        let (files, references) = view.page_backlink_counts();
        for (path, count) in files {
            let name = view
                .list
                .iter()
                .find(|entry| entry.rel_path_str() == path)
                .unwrap()
                .name
                .clone();
            assert_eq!(
                count,
                view.backlinks(&name)
                    .unwrap()
                    .iter()
                    .map(|group| group.blocks.len())
                    .sum::<usize>(),
                "{name}"
            );
        }
        for (name, count) in references {
            assert_eq!(
                count,
                view.backlinks(&name)
                    .unwrap()
                    .iter()
                    .map(|group| group.blocks.len())
                    .sum::<usize>(),
                "{name}"
            );
        }
    }

    #[test]
    fn memberships_follow_alias_date_property_namespace_and_removal_deltas() {
        let (root, store) = fixture(20, 1);
        save(&store, "pages/Source.md", |page| {
            page.blocks[0].raw = "[[Target]] [[Twin]] [[2026-10-27]] [[Oct 27th, 2026]] [[Space/Child]]\nprop:: [[Other]]".to_owned()
        });
        assert_membership_fresh(&store);
        let counts: HashMap<_, _> = store
            .whole_graph()
            .unwrap()
            .page_backlink_counts()
            .0
            .into_iter()
            .collect();
        assert_eq!(counts["pages/Target.md"], 1);
        assert_eq!(counts["pages/Other.md"], 1);
        assert_eq!(
            counts["journals/2026_10_27.md"], 1,
            "two date names in one block count once"
        );
        save(&store, "pages/Target.md", |page| {
            page.pre_block = Some("alias:: Replacement".to_owned())
        });
        assert_membership_fresh(&store);
        save(&store, "pages/Target.md", |page| {
            page.pre_block = Some("title:: Renamed\nalias:: Replacement".to_owned())
        });
        assert_membership_fresh(&store);
        // File creation/removal moves a fileless count to/from its file row.
        std::fs::write(root.path().join("pages/Twin.md"), "- twin\n").unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        assert_membership_fresh(&store);
        std::fs::remove_file(root.path().join("pages/Twin.md")).unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        assert_membership_fresh(&store);
        std::fs::write(
            root.path().join("logseq/config.edn"),
            "{:journal/page-title-format \"yyyy-MM-dd\"}",
        )
        .unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        assert_membership_fresh(&store);
        save(&store, "pages/Source.md", |page| {
            page.blocks[0].raw = "no references".to_owned()
        });
        assert_membership_fresh(&store);
    }

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
                counts
                    .get(&(path.starts_with("journals/"), key.clone()))
                    .copied()
                    .unwrap_or(0) as usize,
                *count,
                "{key}"
            );
        }
    }
}
