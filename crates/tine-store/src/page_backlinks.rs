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

    #[test]
    fn a_property_key_page_lists_the_block_whose_key_folds_to_another_title() {
        // `done-at::` folds to `Done At`; opening `done-at` has no exact
        // membership panel and must still list the block (review r1).
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::write(root.path().join("pages/Done At.md"), "- done at\n").unwrap();
        std::fs::write(
            root.path().join("pages/Source.md"),
            "- task\n  done-at:: today\n",
        )
        .unwrap();
        let store = Arc::new(
            crate::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        store.stop_watcher_for_tests();
        let view = store.whole_graph().unwrap();
        let groups = view.backlinks("done-at").unwrap();
        assert_eq!(groups.iter().map(|g| g.blocks.len()).sum::<usize>(), 1);
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

    /// CM D3: Question A ("which blocks reference page P") has one answerer,
    /// the snapshot membership. Every All pages row, file or reference-only,
    /// shows the size of the panel its name opens, on a graph with an alias,
    /// date spellings, the Favorites page, self references and a property
    /// key that folds to another page's title.
    #[test]
    fn counts_equal_panel_blocks_with_favorites_self_refs_and_folded_keys() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals", "logseq"] {
            std::fs::create_dir(root.path().join(dir)).unwrap();
        }
        let write = |path: &str, text: &str| std::fs::write(root.path().join(path), text).unwrap();
        write("logseq/config.edn", "{:tine/favorites-page \"Favs\"}");
        write("pages/Favs.md", "- [[Hub]]\n- [[H]]\n- [[Ref only]]\n");
        write(
            "pages/Hub.md",
            "alias:: H\n\n- [[Hub]] self\n- [[H]] self by alias\n- [[Other]]\n",
        );
        write("pages/H.md", "- [[Hub]] from the alias file\n");
        write("pages/Other.md", "- [[Other]] self\n- [[Hub]]\n");
        write("pages/Done At.md", "- done at\n");
        write(
            "journals/2026_10_27.md",
            "- [[Oct 27th, 2026]] self\n- [[Hub]]\n",
        );
        write(
            "pages/Notes.md",
            concat!(
                "- [[H]] and [[Hub]]\n",
                "- on [[Oct 27th, 2026]] and [[2026-10-27]]\n",
                "- task\n  done-at:: today\n",
                "- [[Ref only]] #[[Ref only]]\n",
            ),
        );
        let store = Arc::new(
            crate::Store::open(root.path(), Default::default())
                .unwrap()
                .0,
        );
        store.stop_watcher_for_tests();
        // Cold: every row (file or reference-only) is its panel's size.
        assert_membership_fresh(&store);
        let view = store.whole_graph().unwrap();
        let (files, references) = view.page_backlink_counts();
        let files: HashMap<_, _> = files.into_iter().collect();
        assert_eq!(files["pages/Favs.md"], 0, "nothing links to Favorites");
        assert_eq!(
            files["pages/Hub.md"], 4,
            "Notes (once), Other, the day and H.md; not Favorites, not its own blocks"
        );
        assert_eq!(files["pages/Done At.md"], 1, "done-at:: folds to Done At");
        assert_eq!(files["journals/2026_10_27.md"], 1, "both spellings, once");
        assert_eq!(
            references,
            vec![("done-at".to_owned(), 1), ("Ref only".to_owned(), 1)],
            "the property key's own page counts its block (was 0, panel 1)"
        );
        drop(view);
        // Incremental: removing and re-adding the key patches both panels.
        save(&store, "pages/Notes.md", |page| {
            page.blocks[2].raw = "task".to_owned()
        });
        assert_membership_fresh(&store);
        let (files, references) = store.whole_graph().unwrap().page_backlink_counts();
        assert_eq!(
            files
                .iter()
                .find(|(p, _)| p == "pages/Done At.md")
                .unwrap()
                .1,
            0
        );
        assert!(!references
            .iter()
            .any(|(name, n)| name == "done-at" && *n > 0));
        save(&store, "pages/Notes.md", |page| {
            page.blocks[2].raw = "task\ndone-at:: later".to_owned()
        });
        assert_membership_fresh(&store);
        let (_, references) = store.whole_graph().unwrap().page_backlink_counts();
        assert_eq!(references[0], ("done-at".to_owned(), 1));
    }

    /// CM D3: an opening no panel is registered for (`done_at`: no file, no
    /// reference spells it, `done-at::` reaches it by its property fold) reads
    /// the panels its names and folds reach, filtered by the parser evidence,
    /// so it lists exactly what the reference rule names, including a block
    /// on a reached panel's own page that the panel itself excludes. Before,
    /// these openings took a second, parser-evidence-only scan.
    #[test]
    fn an_unregistered_opening_reads_the_reached_memberships_exactly() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        let write = |path: &str, text: &str| std::fs::write(root.path().join(path), text).unwrap();
        write(
            "pages/done-at.md",
            "- own\n  done-at:: yesterday\n- [[done-at]] self\n",
        );
        write(
            "pages/Source.md",
            "- task\n  done-at:: today\n- [[done-at]] direct\n- other\n",
        );
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        let view = store.whole_graph().unwrap();
        let listed = |name: &str| -> Vec<(String, String)> {
            let mut rows: Vec<_> = view
                .backlinks(name)
                .unwrap()
                .iter()
                .flat_map(|group| {
                    group
                        .blocks
                        .iter()
                        .map(|block| {
                            (
                                group.page.clone(),
                                block.raw.lines().next().unwrap().to_owned(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            rows.sort();
            rows
        };
        assert_eq!(
            listed("done_at"),
            vec![
                ("Source".to_owned(), "task".to_owned()),
                ("done-at".to_owned(), "own".to_owned()),
            ],
            "both property-key blocks, not the direct [[done-at]] links"
        );
        assert_eq!(
            listed("done-at"),
            vec![
                ("Source".to_owned(), "[[done-at]] direct".to_owned()),
                ("Source".to_owned(), "task".to_owned()),
            ],
            "the registered page excludes its own blocks"
        );
        assert!(listed("nothing names this").is_empty());
    }

    /// CM D3 cost counter: one membership build per snapshot serves both
    /// surfaces. A panel open after the counts (or the counts after a panel)
    /// builds nothing and constructs evidence only for the listed blocks; an
    /// opening no panel is registered for builds nothing either.
    #[test]
    fn one_membership_build_serves_the_panel_and_the_counts() {
        let (_root, store) = fixture(100, 60);
        let view = store.whole_graph().unwrap();
        let work = view.graph.resident_backlink_memberships().unwrap();
        assert_eq!(work.full_builds, 1, "the fixture's counts read built it");
        assert_eq!(work.source_visits, view.graph.pages.len());
        tine_core::reference_evidence::reset_occurrence_constructions();
        let groups = view.backlinks("Target").unwrap();
        assert_eq!(
            groups.iter().map(|g| g.blocks.len()).sum::<usize>(),
            1,
            "Source's one block; Target's own block is excluded"
        );
        let work = view.graph.resident_backlink_memberships().unwrap();
        assert_eq!(work.full_builds, 1, "the panel reuses the counts' build");
        assert_eq!(
            tine_core::reference_evidence::occurrence_constructions(),
            2,
            "evidence for the listed block's two names only"
        );
        let _ = view.backlinks("never named");
        assert_eq!(work.full_builds, 1, "nothing reached, nothing built");
        let _ = view.page_backlink_counts();
        assert_eq!(work.full_builds, 1);
    }

    /// UI-CM-ALLPAGES-COUNT-ZERO-FOR-ALIAS-GROUP (batch 5 regression): every
    /// All pages count equals the size of the panel its name opens, also for
    /// a page reached through an alias, an alias chain or a journal-date
    /// spelling. Runs without the static model bridge (`CM_STATIC_BRIDGE`).
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

    /// The rich graph the cold/resident panel agreement runs on: aliases (one
    /// without a file, a chain), two date spellings, property keys (one folding
    /// to another title), namespaces, self references and the Favorites page.
    fn rich_graph() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals", "logseq"] {
            std::fs::create_dir(root.path().join(dir)).unwrap();
        }
        let write = |path: &str, text: &str| std::fs::write(root.path().join(path), text).unwrap();
        write("logseq/config.edn", "{:tine/favorites-page \"Favs\"}");
        write("pages/Favs.md", "- [[Hub]]\n");
        write("pages/Hub.md", "alias:: H\n\n- [[Hub]] and [[H]] self\n");
        write("pages/H.md", "- the H file\n");
        write("pages/Machine Learning.md", "alias:: ML\n\n- about it\n");
        write("pages/B.md", "alias:: C\n\n- b\n");
        write("pages/C.md", "alias:: D\n\n- c\n");
        write("pages/Done At.md", "- done at [[Hub]]\n  done-at:: own\n");
        write("pages/Space%2FChild.md", "- child [[Space]]\n");
        write("journals/2026_10_27.md", "- the day [[Hub]]\n");
        write(
            "pages/Notes.md",
            concat!(
                "link:: [[Hub]]\n\n",
                "- see [[ML]] and #Hub\n",
                "  - [[Machine Learning]] [[H]]\n",
                "- chain [[C]] [[D]]\n",
                "- on [[Oct 27th, 2026]]\n",
                "- on [[2026-10-27]] [[Space/Child]]\n",
                "- task\n  done-at:: today\n",
                "- `[[Hub]]` in code\n",
                "- [[Ref only]] [[done at]]\n",
            ),
        );
        root
    }

    fn panel_answer(view: &crate::store::WholeGraph, name: &str) -> Vec<(String, Vec<String>)> {
        view.backlinks(name)
            .unwrap()
            .iter()
            .map(|group| {
                (
                    group.page.clone(),
                    group.blocks.iter().map(|block| block.raw.clone()).collect(),
                )
            })
            .collect()
    }

    /// Fix for the first Linked References open (card …J-F1k): a panel read on
    /// a snapshot without the resident membership answers from the panel's own
    /// sources through the same membership rule, and agrees with the resident
    /// membership the All pages counts build, for every kind of opening.
    #[test]
    fn a_cold_panel_open_agrees_with_the_resident_membership_for_every_opening() {
        let root = rich_graph();
        let openings = [
            "Hub",
            "hub",
            "H",
            "Machine Learning",
            "ML",
            "B",
            "C",
            "D",
            "Done At",
            "done-at",
            "done_at",
            "done at",
            "Space",
            "Space/Child",
            "2026-10-27",
            "Oct 27th, 2026",
            "Ref only",
            "link",
            "Notes",
            "Favs",
            "Nothing names this",
        ];
        let cold = |name: &str| {
            let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
            store.stop_watcher_for_tests();
            let view = store.whole_graph().unwrap();
            let answer = panel_answer(&view, name);
            assert!(
                view.graph.resident_backlink_memberships().is_none(),
                "{name}: a panel read builds no graph-wide membership"
            );
            answer
        };
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        store.stop_watcher_for_tests();
        let view = store.whole_graph().unwrap();
        let (files, references) = view.page_backlink_counts();
        assert!(view.graph.resident_backlink_memberships().is_some());
        let mut listed = 0;
        for name in openings {
            let resident = panel_answer(&view, name);
            listed += resident
                .iter()
                .map(|(_, blocks)| blocks.len())
                .sum::<usize>();
            assert_eq!(cold(name), resident, "{name}: cold vs resident panel");
        }
        assert!(listed >= 20, "the openings list blocks: {listed}");
        // Every row still equals its panel, opened cold.
        let paths: HashMap<String, String> = view
            .list
            .iter()
            .filter_map(|entry| Some((entry.rel_path.as_ref()?.to_string(), entry.name.clone())))
            .collect();
        for (path, count) in files {
            let panel: usize = cold(&paths[&path]).iter().map(|(_, b)| b.len()).sum();
            assert_eq!(count, panel, "{path}: count vs cold panel");
        }
        let mut disagreements = Vec::new();
        for (name, count) in references {
            let panel: usize = cold(&name).iter().map(|(_, b)| b.len()).sum();
            if count != panel {
                disagreements.push((name, count, panel));
            }
        }
        // CM D3: the reference-only row `done-at` (a property key folding like
        // the file title `Done At`) has its own panel, so it counts what its
        // panel lists. Every row equals its panel.
        assert!(
            disagreements.is_empty(),
            "every row equals its cold panel: {disagreements:?}"
        );
    }

    /// Counter for the first open: a cold panel read visits only the sources
    /// the explicit index lists for its names (here Source.md and Target.md's
    /// self link), not the 100 unrelated pages, and builds no membership.
    #[test]
    fn a_cold_panel_open_visits_only_its_sources() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::write(
            root.path().join("pages/Target.md"),
            "alias:: Twin\n\n- [[Target]]\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("pages/Source.md"),
            "- [[Target]] [[Twin]]\n- plain\n",
        )
        .unwrap();
        for at in 0..100 {
            std::fs::write(
                root.path().join(format!("pages/Unrelated {at}.md")),
                "- [[Other]]\n",
            )
            .unwrap();
        }
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        store.stop_watcher_for_tests();
        let view = store.whole_graph().unwrap();
        crate::model::ONE_PANEL_SOURCE_VISITS.with(|c| c.set(0));
        assert_eq!(
            panel_answer(&view, "Twin"),
            vec![("Source".to_owned(), vec!["[[Target]] [[Twin]]".to_owned()])]
        );
        assert_eq!(crate::model::ONE_PANEL_SOURCE_VISITS.with(|c| c.get()), 2);
        assert!(view.graph.resident_backlink_memberships().is_none());
        // An opening nothing names reads no source.
        crate::model::ONE_PANEL_SOURCE_VISITS.with(|c| c.set(0));
        assert!(panel_answer(&view, "Ghost").is_empty());
        assert_eq!(crate::model::ONE_PANEL_SOURCE_VISITS.with(|c| c.get()), 0);
    }

    /// Fix for the outside-edit refresh with All pages counts resident (card
    /// …J-F3s): the fileless-day support the counts read builds is patched from
    /// the changed page alone. Neither snapshot builds the graph-wide per-page
    /// referenced-name index (before the fix both did, O(graph) per snapshot),
    /// for an outside edit and for an in-app save alike.
    #[test]
    fn an_edit_after_the_counts_read_patches_without_a_graph_wide_name_index() {
        let (root, store) = fixture(100, 1);
        let held = store.whole_graph().unwrap();
        assert!(held.graph.day_support_built(), "the counts read built it");
        std::fs::write(
            root.path().join("pages/Source.md"),
            "- [[Target]] [[Twin]]\n- outside [[Other]] [[Fresh name]]\n",
        )
        .unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        let outside = store.whole_graph().unwrap();
        assert!(outside.graph.day_support_built(), "carried, not dropped");
        save(&store, "pages/Source.md", |page| {
            page.blocks[0].raw = "plain [[Target]] [[Another name]]".to_owned()
        });
        let saved = store.whole_graph().unwrap();
        assert!(saved.graph.day_support_built());
        for (label, view) in [("held", &held), ("outside", &outside), ("saved", &saved)] {
            assert!(
                !view.graph.referenced_name_index_built(),
                "{label}: no graph-wide referenced-name index"
            );
        }
        // The patched counts equal a fresh store's on the same files.
        let patched = saved.page_backlink_counts();
        let (fresh, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        fresh.stop_watcher_for_tests();
        assert_eq!(patched, fresh.whole_graph().unwrap().page_backlink_counts());
        let references: Vec<&str> = patched.1.iter().map(|(name, _)| name.as_str()).collect();
        assert!(references.contains(&"Another name"), "{references:?}");
        assert!(references.contains(&"Fresh name"), "{references:?}");
    }
}
