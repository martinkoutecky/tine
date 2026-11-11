//! Carry only the open completion sessions whose name universe did not move.
//! Check changed documents, including each editor's original reference text;
//! never enumerate the graph to validate an incremental rank.
use super::*;

/// One graph lifetime, including config/rescan/checkpoint generations. Keep
/// the existing supplemental-key map API; rank sessions register weak owners
/// so close can release pools even in held, invalidated older snapshots.
#[derive(Default)]
pub(crate) struct Sessions {
    keys: std::sync::Mutex<HashMap<String, Arc<NameKeyMap>>>,
    pub(crate) ranks: Arc<crate::query_plan::completion::Registry>,
}
impl std::ops::Deref for Sessions {
    type Target = std::sync::Mutex<HashMap<String, Arc<NameKeyMap>>>;
    fn deref(&self) -> &Self::Target {
        &self.keys
    }
}

impl ReadSnapshot {
    pub(super) fn carry_completion_rank(&mut self, old: &Self, changed: &[String]) {
        if self.config.enable_search_remove_accents != old.config.enable_search_remove_accents
            || self.config.property_pages_enabled != old.config.property_pages_enabled
            || self.config.property_pages_excludelist != old.config.property_pages_excludelist
        {
            return;
        }
        let next = old.completion_rank.carry(|baseline| {
            if changed.is_empty() {
                return Arc::ptr_eq(&self.pages, &old.pages) && Arc::ptr_eq(&self.list, &old.list);
            }
            changed.iter().all(|path| {
                let before = snapshot_page_by_rel(&old.pages, None, path);
                let after = snapshot_page_by_rel(&self.pages, None, path);
                let (Some((before, old_doc)), Some((after, doc))) = (before, after) else {
                    return false;
                };
                if before.name != after.name
                    || before.kind != after.kind
                    || before.date_key != after.date_key
                    || before.path != after.path
                    || before.rel_path != after.rel_path
                    || crate::query::document_alias_spellings(old_doc)
                        != crate::query::document_alias_spellings(doc)
                    || self.file_supplies_parents(&after.path)
                        != old.file_supplies_parents(&before.path)
                {
                    return false;
                }
                let baseline = baseline
                    .as_ref()
                    .filter(|(owner, _, _)| *owner == before.name)
                    .map(|(_, id, raw)| (id.as_str(), raw.as_str()));
                let names = |doc: &Document, config: &Config| {
                    // The resident spelling table need not populate the optional
                    // per-page cache. Carry already has these changed documents;
                    // compare their parser-owned names without building a cache.
                    collect_document_referenced_names_with_baseline(doc, baseline, config)
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>()
                };
                names(old_doc, &old.config) == names(doc, &self.config)
            })
        });
        self.completion_rank = next;
    }
}
