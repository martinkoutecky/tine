//! The generation-keyed name memos of [`super`]: the page list and the
//! effective-name claimant index, and how a walk installs them only at the
//! generation and held epoch it began at. A child of `derived.rs`, so the
//! derived state's fields stay private to the two.

use super::*;

impl Graph {
    /// The claimant index's answer for `key` at cache generation `gen`, if
    /// it has one; `None` means only a walk can answer.
    pub(crate) fn cached_claimants(
        &self,
        key: &(PageKind, String),
        gen: u64,
    ) -> Option<Vec<PageEntry>> {
        let cache = self.derived.find_entry_cache.read().unwrap();
        let (g, index) = cache.as_ref()?;
        (*g == gen && (index.has_kind(key.0) || index.entries.contains_key(key)))
            .then(|| index.entries.get(key).cloned().unwrap_or_default())
    }

    /// The cache generation and the held epoch a name memo is derived at.
    fn name_stamp(&self) -> (u64, u64) {
        (self.cache_gen.load(Ordering::Acquire), self.held.epoch())
    }

    /// Run `install` for a name memo derived since `stamp` only while
    /// neither moved, checked under the cache lock: a transition moves the
    /// held map and bumps the epoch inside it. A release or an open's hold
    /// moves no generation, so a walk whose certify read the held map on both
    /// sides of one (the page dropped, or listed twice) would otherwise stand
    /// for the whole generation (REG-NAME-LOOKUP-DURING-RELEASE-KEEPS-PAGE).
    /// None: the caller walks again.
    fn install_name_memo<T>(&self, stamp: (u64, u64), install: impl FnOnce() -> T) -> Option<T> {
        let _cache = self.derived.cache.read().unwrap();
        (self.name_stamp() == stamp).then(install)
    }

    /// Every page and journal in the graph, one per duplicate journal day.
    pub(crate) fn list_pages_shared(&self) -> Arc<Vec<PageEntry>> {
        loop {
            let stamp = self.name_stamp();
            if let Some((g, entries)) = self.derived.page_list_cache.read().unwrap().as_ref() {
                if *g == stamp.0 {
                    return Arc::clone(entries);
                }
            }
            let entries = self.certify(list_graph_pages(self), None);
            // A duplicate-day journal (canonical + leftover title-named file)
            // shows once in quick-switch and All Pages: both resolve to one page.
            let entries = Arc::new(dedup_journal_days(
                entries.0,
                &self.current_journal_format(),
                self.current_config().file_name_format,
            ));
            let installed = self.install_name_memo(stamp, || {
                *self.derived.page_list_cache.write().unwrap() =
                    Some((stamp.0, Arc::clone(&entries)));
            });
            if installed.is_some() {
                return entries;
            }
        }
    }

    pub(crate) fn find_claimants(&self, name: &str, kind: PageKind) -> Vec<PageEntry> {
        let key = (kind, tine_core::refs::page_key(name));
        loop {
            let stamp = self.name_stamp();
            let gen = stamp.0;
            if let Some(found) = self.cached_claimants(&key, gen) {
                return found;
            }
            let listed = self.certify(list_graph_pages_kind(self, Some(kind)), Some(kind));
            let mut built = FindEntryIndex::new();
            built.entries = page_claimants(self, &listed.0);
            built.mark_kind_loaded(kind);
            let found = self.install_name_memo(stamp, || {
                let mut guard = self.derived.find_entry_cache.write().unwrap();
                match guard.as_mut() {
                    Some((g, index)) if *g == gen => {
                        if !index.has_kind(kind) {
                            index
                                .entries
                                .retain(|(loaded_kind, _), _| *loaded_kind != kind);
                            index.entries.extend(built.entries);
                            index.mark_kind_loaded(kind);
                        }
                        index.entries.get(&key).cloned().unwrap_or_default()
                    }
                    _ => {
                        let found = built.entries.get(&key).cloned().unwrap_or_default();
                        *guard = Some((gen, built));
                        found
                    }
                }
            });
            if let Some(found) = found {
                return found;
            }
        }
    }

    /// A direct path read can observe a new or retitled file before the
    /// watcher. Drop the claimant memo if that live file is missing from its
    /// bucket (dropping a memo is always safe); a proposed destination that
    /// does not exist must not become a claimant.
    pub(crate) fn observe_name_entry(&self, entry: &PageEntry) {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let mut cache = self.derived.find_entry_cache.write().unwrap();
        if let Some((g, index)) = cache.as_ref() {
            let key = (entry.kind, tine_core::refs::page_key(&entry.name));
            let known = index.entries.get(&key).is_some_and(|entries| {
                entries.iter().any(|candidate| candidate.path == entry.path)
            });
            if *g == gen
                && !known
                && fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_file())
            {
                *cache = None;
            }
        }
    }
}
