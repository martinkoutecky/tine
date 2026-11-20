//! The generation-keyed name memos of [`super`]: the page list and the
//! effective-name claimant index, their walks, and how a publication's name
//! index becomes them (`adopt_name_index`). A child of `derived.rs`, so the
//! derived state's fields stay private to the two.
//!
//! **Stamp rule** (REG-NAME-LOOKUP-DURING-RELEASE-KEEPS-PAGE). A release or
//! an open's hold moves the held map and no generation, under the cache lock
//! and the writer. Name observations change a memo and its discovery
//! errors only at their (generation, held epoch), under the cache lock.

use super::*;

const LISTING_ERROR: &str = "Tine unreadable file producer=listing-name-discovery";

/// The claims that the installed parsed rows alone cannot describe, keyed
/// by folded leaf so an ownership change visits only its affected paths.
#[derive(Clone, Default)]
pub(super) struct Inventory(SharedMap<(String, PathBuf), Claim>);

#[derive(Clone)]
struct Claim {
    name_only: Option<PageEntry>,
    disk: bool,
    owner: bool,
    listed: bool,
}

impl Inventory {
    pub(super) fn affected<'a>(&'a self, fold: &'a str) -> impl Iterator<Item = &'a PathBuf> {
        self.0
            .iter_from(&(fold.to_owned(), PathBuf::new()))
            .take_while(move |((leaf, _), _)| leaf == fold)
            .map(|((_, path), _)| path)
    }

    /// Returns whether a name-only claim moved and whether disk was absent
    /// at a path whose owner has now released it.
    pub(super) fn settle(&mut self, path: &Path, authority: &Authority) -> (bool, bool) {
        let Some(fold) = leaf_fold(path) else {
            return (false, false);
        };
        let key = (fold, path.to_owned());
        let Some(claim) = self.0.get(&key) else {
            return (false, false);
        };
        let disk = matches!(authority, Authority::Disk);
        let listed = match authority {
            Authority::Disk => claim.disk,
            Authority::Owner(..) => claim.owner,
            Authority::Withheld => false,
        };
        let changed = listed != claim.listed && claim.name_only.is_some();
        let absent = disk && !claim.disk;
        if absent {
            self.0.remove(&key);
        } else if changed {
            self.0.get_mut(&key).unwrap().listed = listed;
        }
        (changed, absent)
    }

    fn entries(&self, pages: Option<&Pages>) -> Certified {
        let parsed = pages
            .into_iter()
            .flat_map(Pages::iter)
            .map(|(entry, _)| entry);
        let names = self
            .0
            .values()
            .filter(|claim| claim.listed)
            .filter_map(|claim| claim.name_only.as_ref());
        Certified(parsed.chain(names).cloned().collect())
    }
}

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

    /// Certify the disk observations and record only the exceptions to the
    /// parsed rows: name-only files, withheld paths and owner-only paths.
    pub(super) fn prepare_inventory(
        &self,
        named: impl IntoIterator<Item = PageEntry>,
        parsed: impl Fn(&Path) -> bool,
        failures: &[(crate::FileId, crate::IoError)],
        owners: impl FnOnce() -> Vec<(PathBuf, Option<PageEntry>)>,
        memo: &mut Memo,
    ) -> Inventory {
        let held = !self.held.is_empty();
        let mut extra = HashMap::new();
        for entry in named {
            let listed = !held || self.disk_sourced_in(memo, &entry.path);
            if !listed || !parsed(&entry.path) {
                if let Some(fold) = leaf_fold(&entry.path) {
                    extra.insert(
                        (fold, entry.path.clone()),
                        Claim {
                            name_only: Some(entry),
                            disk: true,
                            owner: false,
                            listed,
                        },
                    );
                }
            }
        }
        #[cfg(test)]
        if held {
            crate::store::pause_at_hook(&self.certify_pause);
        }
        // A failed directory walk did not observe the absence of its files.
        let incomplete: HashSet<_> = failures
            .iter()
            .map(|(id, _)| self.root.join(id.as_str()))
            .collect();
        for (path, name_only) in owners() {
            let Some(fold) = leaf_fold(&path) else {
                continue;
            };
            let key = (fold, path);
            let disk = extra.remove(&key).is_some()
                || key.1.ancestors().any(|path| incomplete.contains(path));
            if name_only.is_some() || !disk {
                extra.insert(
                    key,
                    Claim {
                        name_only,
                        disk,
                        owner: true,
                        listed: true,
                    },
                );
            }
        }
        Inventory(extra.into_iter().collect())
    }

    /// The writer's rebuild publication can use the installed inventory.
    pub(crate) fn rebuild_names_are_current(&self) -> bool {
        let cache = self.derived.cache.read().unwrap();
        cache
            .inventory
            .as_ref()
            .is_some_and(|(gen, _)| *gen == self.cache_generation())
    }

    /// An installed generation was superseded before Rescan could publish.
    /// Take its replacement listing outside the writer, landing it only at
    /// the stamp it observed.
    pub(crate) fn prepare_rebuilt_name_listing(&self) {
        let stamp = self.name_stamp();
        let Some(pages) = self.peek_pages() else {
            return;
        };
        let (named, failures) = list_graph_pages(self);
        let parsed: HashSet<_> = pages
            .iter()
            .map(|(entry, _)| entry.path.as_path())
            .collect();
        let mut memo = Memo::default();
        let names = self.prepare_inventory(
            named,
            |path| parsed.contains(path),
            &failures,
            || {
                self.held
                    .published()
                    .into_iter()
                    .map(|(key, bytes)| {
                        let path = self.root.join(self.held.spellings().spelling(&key));
                        if parsed.contains(path.as_path()) {
                            (path, None)
                        } else {
                            (path, self.owned_entry(&key, &bytes))
                        }
                    })
                    .collect()
            },
            &mut memo,
        );
        let failures = self.classify_discovery_errors(&failures, &mut memo, LISTING_ERROR);
        self.install_names(stamp, None, Some(failures), |cache| {
            cache.inventory = Some((stamp.0, names));
        });
    }

    /// Install a walk's names and classified discovery failures at one stamp.
    fn install_names<T>(
        &self,
        stamp: (u64, u64),
        kind: Option<PageKind>,
        failures: Option<DiscoveryErrors>,
        install: impl FnOnce(&mut InstalledCache) -> T,
    ) -> Option<T> {
        let journals = self.journals_path();
        let journals = journals.strip_prefix(&self.root).unwrap_or(&journals);
        let mut cache = self.derived.cache.write().unwrap();
        if self.name_stamp() != stamp {
            return None;
        }
        let mut retired = Vec::new();
        if let Some(failures) = failures {
            if let Some(kind) = kind {
                cache.discovery.retain(|(id, _)| {
                    Path::new(id.as_str()).starts_with(journals) != (kind == PageKind::Journal)
                });
                cache.discovery.extend(failures);
            } else {
                retired = std::mem::replace(&mut cache.discovery, failures);
            }
        }
        let installed = install(&mut cache);
        drop(cache);
        drop(retired);
        Some(installed)
    }

    fn walk_names(&self, kind: Option<PageKind>) -> (Certified, DiscoveryErrors) {
        let (entries, failures) = list_graph_pages_kind(self, kind);
        let mut memo = Memo::default();
        let failures = self.classify_discovery_errors(&failures, &mut memo, LISTING_ERROR);
        (self.certify(entries, kind, &mut memo), failures)
    }

    pub(in crate::model) fn physical_page_entries(&self) -> Vec<PageEntry> {
        loop {
            let stamp = self.name_stamp();
            let (entries, failures) = self.walk_names(None);
            if self
                .install_names(stamp, None, Some(failures), |_| ())
                .is_some()
            {
                return entries.0;
            }
        }
    }

    #[cfg(test)]
    pub(in crate::model) fn test_install_discovery(&self, failures: DiscoveryErrors) {
        let stamp = self.name_stamp();
        let failures =
            self.classify_discovery_errors(&failures, &mut Memo::default(), LISTING_ERROR);
        self.install_names(stamp, None, Some(failures), |_| ())
            .unwrap();
    }

    fn observed_names(&self, gen: u64) -> (Certified, Option<DiscoveryErrors>) {
        let inventory = {
            let cache = self.derived.cache.read().unwrap();
            cache
                .inventory
                .as_ref()
                .filter(|(listed, _)| *listed == gen)
                .map(|(_, inventory)| (inventory.clone(), cache.pages.clone()))
        };
        match inventory {
            Some((inventory, pages)) => (inventory.entries(pages.as_deref()), None),
            None => {
                let (entries, failures) = self.walk_names(None);
                (entries, Some(failures))
            }
        }
    }

    /// Every page and journal in the graph, one per duplicate journal day.
    pub(crate) fn list_pages_shared(&self) -> Arc<Vec<PageEntry>> {
        self.page_list().materialize()
    }

    /// The page list memo, listed afresh when it describes no current
    /// generation.
    fn page_list(&self) -> Arc<EntryList> {
        loop {
            let stamp = self.name_stamp();
            if let Some((g, entries)) = self.derived.page_list_cache.read().unwrap().as_ref() {
                if *g == stamp.0 {
                    return Arc::clone(entries);
                }
            }
            let (entries, failures) = self.observed_names(stamp.0);
            // A duplicate-day journal (canonical + leftover title-named file)
            // shows once in quick-switch and All Pages: both resolve to one page.
            let entries = Arc::new(EntryList::from(
                dedup_journal_days(&self.current_config().journals_dir, entries.0).as_slice(),
            ));
            let installed = self.install_names(stamp, None, failures, |_| {
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
            let (listed, failures) = self.walk_names(Some(kind));
            let journals_dir = self.current_config().journals_dir.clone();
            let mut built = FindEntryIndex::new();
            built.entries = Arc::new(page_claimants(&journals_dir, &listed.0));
            built.mark_kind_loaded(kind);
            let key = claimant_key(&key);
            let (format, journals) = (self.current_journal_format(), self.journals_path());
            let name_format = self.current_config().file_name_format;
            // The rows a listing of `kind` replaces: every row of that kind,
            // except a day only a `title::` names, which a journal-kind
            // listing does not read (`list_graph_pages_kind`) and which this
            // generation's index already holds from a listing that did.
            let replaced = |row: &PageEntry| {
                kind == PageKind::Page
                    || page_identity::journal_listing_covers(
                        (&format, &journals, name_format),
                        &row.path,
                    )
            };
            let found = self.install_names(stamp, Some(kind), Some(failures), |_| {
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
                                let mut kept = entries.get(&key).cloned().unwrap_or_default();
                                entries.remove(&key);
                                kept.retain(|row| !replaced(row));
                                if !kept.is_empty() {
                                    entries.insert(key, kept);
                                }
                            }
                            for (key, rows) in built.entries.iter() {
                                let mut rows = rows.clone();
                                if let Some(kept) = entries.get(key) {
                                    let fresh: Vec<_> =
                                        rows.iter().map(|row| row.path.clone()).collect();
                                    rows.extend(
                                        kept.iter()
                                            .filter(|row| !fresh.contains(&row.path))
                                            .cloned(),
                                    );
                                    rows.sort_by(|a, b| {
                                        compare_page_claimants(&journals_dir, a, b)
                                    });
                                }
                                entries.insert(key.clone(), rows);
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
            });
            if let Some(found) = found {
                return found;
            }
        }
    }

    /// A direct path read can observe a new or retitled file before the
    /// watcher. A live file missing from its bucket joins it when no other
    /// bucket can name it: a journals-directory file (its file name is its
    /// name) or another file this generation's page list does not list (a
    /// new file).
    /// It joins only a kind the index lists completely (a missing bucket of
    /// an unlisted kind would hide the name's other claimants) and only when
    /// the entry was named from the file's bytes, not from its file name
    /// alone (`trusted`). Otherwise the claimant memo drops (dropping a memo
    /// is always safe). A proposed destination that does not exist must not
    /// become a claimant. A file a host holds is no observation at all: its
    /// owner's bytes, not the file, name it ((G)), so a withheld page or one
    /// its owner deleted must not join from disk. Decided at one stamp under
    /// the cache lock (module doc); a moved stamp drops the memo.
    pub(in crate::model) fn observe_name_entry(&self, entry: &PageEntry, trusted: bool) {
        let stamp = self.name_stamp();
        let observed = self.install_name_memo(stamp, || {
            if !self.disk_sourced(&entry.path) {
                return;
            }
            let gen = stamp.0;
            let unlisted = entry.path.starts_with(self.journals_path())
                || (self.derived.page_list_cache.read().unwrap().as_ref())
                    .is_some_and(|(g, list)| *g == gen && list.get_path(&entry.path).is_none());
            let mut cache = self.derived.find_entry_cache.write().unwrap();
            let Some((g, index)) = cache.as_mut() else {
                return;
            };
            let key = claimant_key(&(entry.kind, tine_core::refs::page_key(&entry.name)));
            let known = (index.entries.get(&key)).is_some_and(|entries| {
                entries.iter().any(|candidate| candidate.path == entry.path)
            });
            if *g != gen
                || known
                || !fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_file())
            {
                return;
            }
            if !(trusted && unlisted && index.has_kind(entry.kind)) {
                *cache = None;
                return;
            }
            let entries = Arc::make_mut(&mut index.entries);
            let mut bucket = entries.get(&key).cloned().unwrap_or_default();
            bucket.push(entry.clone());
            bucket
                .sort_by(|a, b| compare_page_claimants(&self.current_config().journals_dir, a, b));
            entries.insert(key, bucket);
        });
        if observed.is_none() {
            *self.derived.find_entry_cache.write().unwrap() = None;
        }
    }

    /// Cold journal inventory and its complete claimant index share one walk.
    /// Called at open and, before the first view, under the writer (a
    /// deferred publication, an owner's first journal): no transition can
    /// move the held map during its walk (stamp rule, module doc).
    pub(crate) fn scan_journal_names(&self) -> Vec<PageEntry> {
        loop {
            let stamp = self.name_stamp();
            let kind = Some(PageKind::Journal);
            let (entries, failures) = self.walk_names(kind);
            let mut index = FindEntryIndex::new();
            index.entries = Arc::new(page_claimants(
                &self.current_config().journals_dir,
                &entries.0,
            ));
            index.mark_kind_loaded(PageKind::Journal);
            if self
                .install_names(stamp, kind, Some(failures), |_| {
                    *self.derived.find_entry_cache.write().unwrap() = Some((stamp.0, index));
                })
                .is_some()
            {
                return entries.0;
            }
        }
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
    /// The capture calls it under the writer.
    pub(crate) fn snapshot_name_roots(&self) -> (Arc<EntryList>, Arc<ClaimantMap>) {
        loop {
            let stamp = self.name_stamp();
            let (entries, failures) = self.observed_names(stamp.0);
            let config = self.current_config();
            let claimants = Arc::new(page_claimants(&config.journals_dir, &entries.0));
            let list = Arc::new(EntryList::from(
                dedup_journal_days(&config.journals_dir, entries.0).as_slice(),
            ));
            let installed = self.install_names(stamp, None, failures, |cache| {
                *self.derived.find_entry_cache.write().unwrap() = Some((
                    stamp.0,
                    FindEntryIndex {
                        entries: Arc::clone(&claimants),
                        // The first miss can discover files that arrived after this view.
                        pages_loaded: false,
                        journals_loaded: false,
                    },
                ));
                *self.derived.page_list_cache.write().unwrap() = Some((stamp.0, Arc::clone(&list)));
                cache.inventory.take()
            });
            if installed.is_some() {
                return (list, claimants);
            }
        }
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

    /// When an ownership transition exposes a withheld listing row without
    /// moving the generation, memos at that generation must be recaptured.
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
