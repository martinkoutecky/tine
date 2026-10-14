//! Request-local page completion and ephemeral cache measurements.
use super::*;

impl WholeGraph {
    /// Measurement-only count and owned payload bytes of ephemeral name keys.
    /// Hash table bucket/control allocation and Arc headers are excluded.
    #[cfg(feature = "test-faults")]
    pub fn transliteration_cache_payload(&self) -> (usize, usize) {
        let names = &self.graph.transliteration_names;
        (
            names.len(),
            names
                .iter()
                .map(|(name, keys)| name.capacity() + keys.allocated_bytes())
                .sum(),
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
    pub fn complete_page_names(
        &self,
        text: &str,
        limit: usize,
        editor: Option<(&str, &str, &str)>,
        transliteration: bool,
    ) -> Vec<PageEntry> {
        let mut plan = crate::query_plan::QueryPlan::legacy_page_search_with_policy(
            text,
            limit,
            self.graph.config().enable_search_remove_accents,
        )
        .with_transliteration(transliteration);
        if let Some((page, block, original)) = editor {
            plan.editor_reference_baseline =
                Some((page.to_owned(), block.to_owned(), original.to_owned()));
        }
        crate::query_plan::page_hits_to_entries(plan.execute(&self.graph, || false).hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckpointWrite, EditKind};

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
            changed.complete_page_names("bj", 8, None, true)[0].name,
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
            before.complete_page_names("nanjing", 8, None, true)[0].name,
            "南京"
        );
        assert!(before
            .complete_page_names("nanjing", 8, None, false)
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
            .complete_page_names("nanjing", 8, None, true)
            .is_empty());
        assert_eq!(
            after.complete_page_names("shanghai", 8, None, true)[0].name,
            "上海"
        );
        assert_eq!(
            before.complete_page_names("nanjing", 8, None, true)[0].name,
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
            view.complete_page_names("bj", 8, None, true)[0].name,
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
            view.complete_page_names("bj", 8, None, true)[0].name,
            "北京"
        );
        assert_eq!(
            view.complete_page_names("nj", 8, None, true)[0].name,
            "南京"
        );
        assert_eq!(
            view.complete_page_names("shanghai", 8, None, true)[0].name,
            "上海"
        );
        assert_eq!(
            reopened.diagnostics()["checkpoint"]["load"]["outcome"],
            "loaded"
        );
        reopened.close();
    }
}
