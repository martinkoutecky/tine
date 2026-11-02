//! The generation-keyed name memos of [`super`]: the page list and the
//! effective-name claimant index, their walks, and how a publication's name
//! index becomes them (`adopt_name_index`). A child of `derived.rs`, so the
//! derived state's fields stay private to the two.

use super::*;

impl Graph {
    /// The claimant index's answer for `key` at cache generation `gen`, if
    /// it has one; `None` means only a walk can answer.
    pub(in crate::model) fn cached_claimants(
        &self,
        key: &(PageKind, String),
        gen: u64,
    ) -> Option<Vec<PageEntry>> {
        let cache = self.derived.find_entry_cache.read().unwrap();
        let (g, index) = cache.as_ref()?;
        let claimed = claimant_key(key);
        (*g == gen && (index.has_kind(key.0) || index.entries.contains_key(&claimed)))
            .then(|| index.entries.get(&claimed).cloned().unwrap_or_default())
    }

    #[cfg(test)]
    pub(in crate::model) fn claimant_memo_built(&self) -> bool {
        self.derived.find_entry_cache.read().unwrap().is_some()
    }

    /// Every page and journal in the graph, one per duplicate journal day.
    pub(crate) fn list_pages_shared(&self) -> Arc<Vec<PageEntry>> {
        self.page_list().materialize()
    }

    /// The page list memo, listed afresh when it describes no current
    /// generation.
    fn page_list(&self) -> Arc<EntryList> {
        let gen = self.cache_gen.load(Ordering::Acquire);
        if let Some((g, entries)) = self.derived.page_list_cache.read().unwrap().as_ref() {
            if *g == gen {
                return Arc::clone(entries);
            }
        }
        let entries = self.certify(list_graph_pages(self), None);
        // A duplicate-day journal (canonical + leftover title-named file)
        // shows once in quick-switch and All Pages: both resolve to one page.
        let entries = Arc::new(EntryList::from(
            dedup_journal_days(
                entries.0,
                &self.current_journal_format(),
                self.current_config().file_name_format,
            )
            .as_slice(),
        ));
        *self.derived.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&entries)));
        entries
    }

    pub(crate) fn find_claimants(&self, name: &str, kind: PageKind) -> Vec<PageEntry> {
        let key = (kind, tine_core::refs::page_key(name));
        loop {
            let gen = self.cache_gen.load(Ordering::Acquire);
            if let Some(found) = self.cached_claimants(&key, gen) {
                return found;
            }
            let listed = self.certify(list_graph_pages_kind(self, Some(kind)), Some(kind));
            let mut built = FindEntryIndex::new();
            built.entries = Arc::new(page_claimants(self, &listed.0));
            built.mark_kind_loaded(kind);
            let key = claimant_key(&key);
            let found = {
                let mut guard = self.derived.find_entry_cache.write().unwrap();
                match guard.as_mut() {
                    Some((g, index)) if *g == gen => {
                        if !index.has_kind(kind) {
                            let journal = kind == PageKind::Journal;
                            let entries = Arc::make_mut(&mut index.entries);
                            let stale: Vec<_> = (entries.iter())
                                .filter(|((loaded, _), _)| *loaded == journal)
                                .map(|(key, _)| key.clone())
                                .collect();
                            for key in stale {
                                entries.remove(&key);
                            }
                            for (key, rows) in built.entries.iter() {
                                entries.insert(key.clone(), rows.clone());
                            }
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
            };
            if self.cache_gen.load(Ordering::Acquire) == gen {
                return found;
            }
        }
    }

    /// A direct path read can observe a new or retitled file before the
    /// watcher. A live file missing from its bucket joins it when no other
    /// bucket can name it: a journal (its file name is its name) or a page
    /// file this generation's page list does not list (a new file).
    /// It joins only a kind the index lists completely (a missing bucket of
    /// an unlisted kind would hide the name's other claimants) and only when
    /// the entry was named from the file's bytes, not from its file name
    /// alone (`trusted`). Otherwise the claimant memo drops (dropping a memo
    /// is always safe). A proposed destination that does not exist must not
    /// become a claimant.
    pub(in crate::model) fn observe_name_entry(&self, entry: &PageEntry, trusted: bool) {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let unlisted = entry.kind == PageKind::Journal
            || (self.derived.page_list_cache.read().unwrap().as_ref())
                .is_some_and(|(g, list)| *g == gen && list.get_path(&entry.path).is_none());
        let mut cache = self.derived.find_entry_cache.write().unwrap();
        let Some((g, index)) = cache.as_mut() else {
            return;
        };
        let key = claimant_key(&(entry.kind, tine_core::refs::page_key(&entry.name)));
        let known = index
            .entries
            .get(&key)
            .is_some_and(|entries| entries.iter().any(|candidate| candidate.path == entry.path));
        if *g != gen || known || !fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_file())
        {
            return;
        }
        if !(trusted && unlisted && index.has_kind(entry.kind)) {
            *cache = None;
            return;
        }
        let (format, name_format) = (
            self.current_journal_format(),
            self.current_config().file_name_format,
        );
        let entries = Arc::make_mut(&mut index.entries);
        let mut bucket = entries.get(&key).cloned().unwrap_or_default();
        bucket.push(entry.clone());
        bucket.sort_by(|a, b| compare_page_claimants(a, b, &format, name_format));
        entries.insert(key, bucket);
    }

    /// Cold journal inventory and its complete claimant index share one walk.
    /// Only called at open, before the watcher and load worker start.
    pub(crate) fn scan_journal_names(&self) -> Vec<PageEntry> {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let entries = self.certify(
            list_graph_pages_kind(self, Some(PageKind::Journal)),
            Some(PageKind::Journal),
        );
        let mut index = FindEntryIndex::new();
        index.entries = Arc::new(page_claimants(self, &entries.0));
        index.mark_kind_loaded(PageKind::Journal);
        *self.derived.find_entry_cache.write().unwrap() = Some((gen, index));
        entries.0
    }

    /// [`Self::snapshot_name_roots`] in the shapes the tests compare.
    #[cfg(test)]
    pub(crate) fn snapshot_name_index(
        &self,
    ) -> (
        Arc<Vec<PageEntry>>,
        HashMap<(PageKind, String), Vec<PageEntry>>,
    ) {
        let (list, claimants) = self.snapshot_name_roots();
        let kind = |journal: bool| match journal {
            true => PageKind::Journal,
            false => PageKind::Page,
        };
        let claimants = (claimants.iter())
            .map(|((journal, name), rows)| ((kind(*journal), name.clone()), rows.clone()))
            .collect();
        (list.materialize(), claimants)
    }

    /// The page list and the effective-name claimants, from one certified
    /// inventory (R2): the snapshot's roots and the memos hold the same rows.
    pub(crate) fn snapshot_name_roots(&self) -> (Arc<EntryList>, Arc<ClaimantMap>) {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let format = self.current_journal_format();
        // The build that produced this generation already listed and named
        // every file from its own reads (GH #623: one read per file); a
        // later generation walks again.
        let launch = self
            .derived
            .launch_listing
            .write()
            .unwrap()
            .take()
            .filter(|(listed_gen, _)| *listed_gen == gen);
        let entries = match launch {
            Some((_, entries)) => entries,
            None => self.certify(list_graph_pages(self), None),
        };
        let claimants = Arc::new(page_claimants(self, &entries.0));
        *self.derived.find_entry_cache.write().unwrap() = Some((
            gen,
            FindEntryIndex {
                entries: Arc::clone(&claimants),
                // Reuse known claims, but a first miss must discover files
                // that arrived after this published snapshot.
                pages_loaded: false,
                journals_loaded: false,
            },
        ));
        let list = Arc::new(EntryList::from(
            dedup_journal_days(entries.0, &format, self.current_config().file_name_format)
                .as_slice(),
        ));
        *self.derived.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&list)));
        (list, claimants)
    }

    /// Adopt a publication's name index as the name memos of generation `gen`
    /// (GH #623 R2): the capture patched it from the names this publication
    /// changed, so the next lookup or listing needs no walk. Called under the
    /// writer, only while `gen` is current; a memo already built at `gen` is
    /// kept. Claimant completeness carries over only from a memo that was
    /// complete at `from` (the previous publication's generation): a name the
    /// previous index lacked was absent then, and this publication patched
    /// every name it changed since. Files no publication has named yet stay
    /// unseen until the watcher names them, as they do in a memo built before
    /// they arrived.
    pub(crate) fn adopt_name_index(
        &self,
        from: Option<u64>,
        gen: u64,
        (list, claimants): (&Arc<EntryList>, &Arc<ClaimantMap>),
    ) {
        if self.cache_gen.load(Ordering::Acquire) != gen {
            return;
        }
        let mut memo = self.derived.page_list_cache.write().unwrap();
        if !memo.as_ref().is_some_and(|(g, _)| *g == gen) {
            *memo = Some((gen, Arc::clone(list)));
        }
        drop(memo);
        let mut memo = self.derived.find_entry_cache.write().unwrap();
        let (pages_loaded, journals_loaded) = match memo.as_ref() {
            Some((g, _)) if *g == gen => return,
            Some((g, index)) if Some(*g) == from => (index.pages_loaded, index.journals_loaded),
            _ => (false, false),
        };
        *memo = Some((
            gen,
            FindEntryIndex {
                entries: Arc::clone(claimants),
                pages_loaded,
                journals_loaded,
            },
        ));
    }

    /// The forced rebuild re-reads the listing: the generation-keyed memos
    /// go (dropping a memo is always safe).
    pub(in crate::model) fn forget_name_memos(&self) {
        *self.derived.page_list_cache.write().unwrap() = None;
        *self.derived.find_entry_cache.write().unwrap() = None;
    }

    /// The generation-keyed memos at `before` stay valid at `after`: the
    /// publication between them changed no listed name.
    pub(in crate::model) fn retag_name_memos(&self, before: u64, after: u64) {
        if let Some((gen, _)) = self.derived.page_list_cache.write().unwrap().as_mut() {
            if *gen == before {
                *gen = after;
            }
        }
        if let Some((gen, _)) = self.derived.find_entry_cache.write().unwrap().as_mut() {
            if *gen == before {
                *gen = after;
            }
        }
    }
}
