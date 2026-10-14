//! Request-local page completion and ephemeral cache measurements.
use super::*;

impl WholeGraph {
    /// Referenced journal days from the published explicit-reference/alias
    /// index, including days without a journal file. Ordinary date-named pages
    /// and their aliases retain ordinary page identity. No document or disk
    /// scan; O(R log D + date-name alias lookups), R indexed reference names,
    /// D distinct referenced dates. Publications update the existing index.
    pub(super) fn referenced_journal_days(&self) -> Vec<Day> {
        let journal_prefix = format!("{}/", self.config.config.journals_dir);
        self.graph
            .explicit_reference_names()
            .filter_map(|name| {
                let date = self.journal_format.parse(name)?;
                let title = self.journal_format.title(date);
                let spellings =
                    crate::query::journal_spelling_keys(&self.journal_format, date, &title);
                if spellings.iter().any(|spelling| {
                    self.name_claimants(spelling, PageKind::Page)
                        .iter()
                        .any(|entry| entry.kind == PageKind::Page)
                        || self
                            .graph
                            .alias_owner_paths(&tine_core::refs::page_key(spelling))
                            .is_some_and(|owners| {
                                owners
                                    .iter()
                                    .any(|owner| !owner.starts_with(&journal_prefix))
                            })
                }) {
                    return None;
                }
                Some(Day(date.ordinal_key()))
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Measurement-only count and owned payload bytes of ephemeral name keys.
    /// Includes ownership counts/names; persistent tree nodes and Arc headers
    /// are excluded. Open editor sessions are separate, lifetime-bound state.
    #[cfg(feature = "test-faults")]
    pub fn transliteration_cache_payload(&self) -> (usize, usize) {
        let names = &self.graph.transliteration_names;
        (
            names.len(),
            names
                .iter()
                .map(|(name, keys)| name.capacity() + keys.allocated_bytes())
                .sum::<usize>()
                + self
                    .graph
                    .transliteration_owners
                    .iter()
                    .map(|(name, _)| name.capacity() + std::mem::size_of::<usize>())
                    .sum::<usize>(),
        )
    }

    /// `[[` completion over pages, journals, aliases and referenced names.
    /// A first call can traverse all page blocks and reference text to build
    /// indexes; later calls scan page, alias, and referenced-name candidates
    /// and rank or sort matches, returning at most `limit` entries. Once an
    /// index has been built, a later publication carries it forward by updating
    /// changed pages; the first completion after every save does not rescan B.
    /// An inline editor can supply its (page, block, pre-completion text) to
    /// retain the reference baseline while its live draft autosaves. This
    /// examines that page's blocks in addition to the name candidates above.
    /// `transliteration` enables Mandarin name matching; legacy callers pass false.
    /// Completion evidence stays attached until the UI's final ordering step.
    /// A zero limit releases this editor session's supplemental baseline keys.
    /// Sessions are transient UI lifetimes, never graph/checkpoint state.
    pub fn complete_page_names(
        &self,
        text: &str,
        limit: usize,
        editor: Option<(&str, &str, &str)>,
        transliteration: bool,
        session: Option<&str>,
    ) -> Vec<(PageEntry, tine_core::query_plan::ObjectiveMatchClass)> {
        let identity = editor.map(|(page, block, original)| {
            session
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{page}\0{block}\0{original}"))
        });
        if limit == 0 {
            if let Some(identity) = identity {
                self.graph
                    .completion_name_keys
                    .lock()
                    .unwrap()
                    .remove(&identity);
            }
            return Vec::new();
        }
        let mut plan = crate::query_plan::QueryPlan::legacy_page_search_with_policy(
            text,
            limit,
            self.graph.config().enable_search_remove_accents,
        )
        .with_transliteration(transliteration);
        if let Some((page, block, original)) = editor {
            plan.editor_reference_baseline =
                Some((page.to_owned(), block.to_owned(), original.to_owned()));
            if transliteration {
                let baseline = {
                    let mut sessions = self.graph.completion_name_keys.lock().unwrap();
                    Arc::clone(sessions.entry(identity.unwrap()).or_insert_with(|| {
                        // Parse the original block once per completion lifetime,
                        // and reuse published keys whenever the name still exists.
                        let mut reference = tine_core::doc::DocBlock::new(original);
                        if let Some((_, doc)) = self
                            .graph
                            .pages
                            .iter()
                            .find(|(entry, _)| entry.name == page)
                        {
                            reference
                                .set_org(crate::query::page_properties::page_document_is_org(doc));
                        }
                        let projection = reference.projection();
                        let names = projection.refs_page().iter().map(String::as_str).chain(
                            tine_core::reference_evidence::linkable_property_names(
                                projection.reference_source(),
                                &projection.regions,
                            ),
                        );
                        let mut keys = crate::model::NameKeyMap::new();
                        for name in names {
                            if let Some(key) = crate::transliteration::cache_identity(name) {
                                let value = self
                                    .graph
                                    .transliteration_names
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        Arc::new(crate::transliteration::NameKeys::build(&key))
                                    });
                                keys.insert(key, value);
                            }
                        }
                        Arc::new(keys)
                    }))
                };
                let mut keys = self.graph.transliteration_names.as_ref().clone();
                for (name, value) in baseline.iter() {
                    keys.insert(name.clone(), Arc::clone(value));
                }
                plan.name_keys = Some(Arc::new(keys));
            }
        }
        plan.execute(&self.graph, || false)
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                tine_core::query_plan::QueryHit::Page {
                    mut page,
                    matched_alias,
                    match_class,
                    ..
                } => {
                    if let Some(alias) = matched_alias {
                        page.name = alias;
                    }
                    Some((page, match_class))
                }
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckpointWrite, EditKind};

    #[test]
    fn pinyin_body_save_shares_cache_on_many_han_names() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        for n in 0..5000 {
            fs::write(root.path().join(format!("pages/北京{n}.md")), "- body\n").unwrap();
        }
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let before = store.whole_graph().unwrap();
        let id = PageId::from("pages/北京0.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "changed body".into();
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let after = store.whole_graph().unwrap();
        assert_eq!(
            after.graph.name_cache_work, 2,
            "only the changed page's old/new Han name may be visited"
        );
        assert!(
            Arc::ptr_eq(
                &before.graph.transliteration_names,
                &after.graph.transliteration_names
            ),
            "body edit rebuilt the Han cache container"
        );
        store.close();
    }

    #[test]
    fn pinyin_editor_baseline_survives_autosave() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/Owner.md"), "- [[北京]]\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let id = PageId::from("pages/Owner.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        let block = doc.blocks[0].id.clone();
        let original = doc.blocks[0].raw.clone();
        let editor = Some(("Owner", block.as_str(), original.as_str()));
        assert_eq!(
            store
                .whole_graph()
                .unwrap()
                .complete_page_names("bj", 8, editor, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "北京"
        );
        doc.blocks[0].raw = "[[b".into();
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let after = store.whole_graph().unwrap();
        assert!(after
            .complete_page_names("bj", 8, editor, true, None)
            .into_iter()
            .map(|(page, _)| page)
            .collect::<Vec<_>>()
            .iter()
            .any(|p| p.name == "北京"));
        assert!(after
            .complete_page_names("bj", 8, None, true, None)
            .into_iter()
            .map(|(page, _)| page)
            .collect::<Vec<_>>()
            .is_empty());
        let builds = crate::transliteration::BUILDS.with(|count| count.get());
        for query in ["b", "be", "bei", "beij", "bj"] {
            assert!(after
                .complete_page_names(query, 8, editor, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()
                .iter()
                .any(|p| p.name == "北京"));
        }
        assert_eq!(
            crate::transliteration::BUILDS.with(|count| count.get()),
            builds,
            "unchanged live baseline must not call the dictionary per keystroke"
        );
        after
            .complete_page_names("", 0, editor, false, None)
            .into_iter()
            .map(|(page, _)| page)
            .collect::<Vec<_>>();
        assert!(
            after.graph.completion_name_keys.lock().unwrap().is_empty(),
            "closing completion releases obsolete names"
        );
        assert!(!after.graph.transliteration_names.contains_key("北京"));
        store.close();
    }

    #[test]
    fn pinyin_name_ownership_keeps_keys_until_the_last_alias_or_reference_disappears() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/A.md"), "alias:: 北京\n- [[北京]]\n").unwrap();
        fs::write(root.path().join("pages/B.md"), "- [[北京]]\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let before = store.whole_graph().unwrap();
        let keys = Arc::clone(before.graph.transliteration_names.get("北京").unwrap());
        for page in ["A", "B"] {
            let id = PageId::from(format!("pages/{page}.md"));
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.pre_block = None;
            doc.blocks[0].raw = "body".into();
            assert!(matches!(
                store.save(
                    EditKind::ReplacePage,
                    &id,
                    SaveBase::Existing(read.rev),
                    &doc
                ),
                SaveOutcome::Saved(_)
            ));
            let view = store.whole_graph().unwrap();
            if page == "A" {
                assert!(Arc::ptr_eq(
                    &keys,
                    view.graph.transliteration_names.get("北京").unwrap()
                ));
            } else {
                assert!(!view.graph.transliteration_names.contains_key("北京"));
            }
        }
        assert_eq!(
            before
                .complete_page_names("bj", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "北京"
        );
        store.close();
    }

    #[test]
    fn pinyin_expansion_never_changes_saved_macro_query_results() {
        use tine_core::query::ir::{ExecutionContext, QueryRows};
        use tine_core::query::registry::Registry;
        use tine_core::query::{parse_query_input, QueryInput};
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/北京.md"), "- 北京\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let graph = store.whole_graph().unwrap();
        let saved = |source: &str| {
            let (query, view) = parse_query_input(
                source,
                QueryInput::MacroQuery,
                tine_core::date::JournalDate::today(),
                Registry::none(),
            );
            let IrAnswer::Result(result) = graph
                .query_ir(IrRequest::Run {
                    query: &query,
                    view: &view,
                    context: &ExecutionContext::default(),
                })
                .unwrap()
            else {
                panic!("saved evaluator result")
            };
            let QueryRows::Block { groups } = result.rows else {
                panic!("block query")
            };
            groups.len()
        };
        assert_eq!(saved(r#"(search "beijing")"#), 0);
        assert_eq!(
            saved(r#"(search "北京")"#),
            1,
            "positive saved-query control"
        );
        assert_eq!(
            graph
                .complete_page_names("beijing", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "北京"
        );
        assert_eq!(
            saved(r#"(search "beijing")"#),
            0,
            "live expansion must not affect subsequent saved evaluation"
        );
        store.close();
    }

    #[test]
    fn pinyin_empty_cache_is_shared_until_a_changed_name_contains_han() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/Notes.md"), "- 北京 body only\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let before = store.whole_graph().unwrap();
        assert!(before.graph.transliteration_names.is_empty());
        let id = PageId::from("pages/Notes.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "new body".into();
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let after = store.whole_graph().unwrap();
        assert!(Arc::ptr_eq(
            &before.graph.transliteration_names,
            &after.graph.transliteration_names
        ));
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.pre_block = Some("alias:: 北京".into());
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let changed = store.whole_graph().unwrap();
        assert_eq!(
            changed
                .complete_page_names("bj", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "北京"
        );
        assert_eq!(changed.graph.transliteration_names.len(), 1);
        store.close();
    }

    #[test]
    fn pinyin_cache_follows_guarded_alias_save_and_keeps_old_snapshot() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/北京.md"), "alias:: 南京\n- body\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let before = store.whole_graph().unwrap();
        assert_eq!(
            before
                .complete_page_names("nanjing", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "南京"
        );
        assert!(before
            .complete_page_names("nanjing", 8, None, false, None)
            .into_iter()
            .map(|(page, _)| page)
            .collect::<Vec<_>>()
            .is_empty());
        let old_keys = Arc::clone(before.graph.transliteration_names.get("北京").unwrap());
        let id = PageId::from("pages/北京.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.pre_block = Some("alias:: 上海".into());
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let after = store.whole_graph().unwrap();
        assert!(after
            .complete_page_names("nanjing", 8, None, true, None)
            .into_iter()
            .map(|(page, _)| page)
            .collect::<Vec<_>>()
            .is_empty());
        assert_eq!(
            after
                .complete_page_names("shanghai", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "上海"
        );
        assert_eq!(
            before
                .complete_page_names("nanjing", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "南京"
        );
        assert!(!after.graph.transliteration_names.contains_key("南京"));
        assert!(Arc::ptr_eq(
            &old_keys,
            after.graph.transliteration_names.get("北京").unwrap()
        ));
        store.close();
    }

    #[test]
    fn pinyin_cache_rebuilds_from_checkpoint_without_changing_its_shape() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        let page = root.path().join("pages/北京.md");
        fs::write(&page, "alias:: 南京\n- [[上海]]\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&page)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        let cp = root.path().join("launch.bin");
        let open = || {
            Store::open(
                root.path(),
                OpenOptions {
                    launch_checkpoint: Some(cp.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
            .0
        };
        let store = open();
        let view = store.whole_graph().unwrap();
        assert_eq!(
            view.complete_page_names("bj", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "北京"
        );
        assert!(matches!(
            store.write_checkpoint_now(),
            Some(CheckpointWrite::Written { .. })
        ));
        drop(view);
        store.close();
        let reopened = open();
        let view = reopened.whole_graph().unwrap();
        assert_eq!(
            view.complete_page_names("bj", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "北京"
        );
        assert_eq!(
            view.complete_page_names("nj", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "南京"
        );
        assert_eq!(
            view.complete_page_names("shanghai", 8, None, true, None)
                .into_iter()
                .map(|(page, _)| page)
                .collect::<Vec<_>>()[0]
                .name,
            "上海"
        );
        assert_eq!(
            reopened.diagnostics()["checkpoint"]["load"]["outcome"],
            "loaded"
        );
        reopened.close();
    }
}
