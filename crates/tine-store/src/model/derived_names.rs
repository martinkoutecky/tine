//! The generation-keyed name memos of [`super`]: the page list and the
//! effective-name claimant index, their walks, and how a publication's name
//! index becomes them (`adopt_name_index`). A child of `derived.rs`, so the
//! derived state's fields stay private to the two.
//!
//! **Stamp rule** (REG-NAME-LOOKUP-DURING-RELEASE-KEEPS-PAGE). A release or
//! an open's hold moves the held map and no generation, under the cache lock
//! and the writer. A memo walked or patched without the writer changes only
//! at the (generation, held epoch) it began at, checked under the cache lock
//! ([`Graph::install_name_memo`]). Snapshot capture, adopting the published
//! index, and retagging hold the writer.

use super::*;

fn name_roots(journals_dir: &str, entries: Vec<PageEntry>) -> NameRoots {
    let claimants = Arc::new(page_claimants(journals_dir, entries.iter().cloned()));
    // Keep the list's first position for each day and take its row from S4.
    let mut seen_days = HashSet::new();
    let list: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| match entry.date_key {
            Some(day) if seen_days.insert(day) => {
                let bucket = claimants.get(&Claim::Day(day)).expect("listed day bucket");
                Some(bucket[0].clone())
            }
            Some(_) => None,
            None => Some(entry),
        })
        .collect();
    (Arc::new(EntryList::from(list.as_slice())), claimants)
}

impl Graph {
    /// The installed answer for an exact-name check. A host hold on a
    /// proposed path leaves these published claims valid. Creation checks
    /// also consult unindexed held files before admitting a new claimant.
    fn cached_claimants(&self, key: &Claim, gen: u64) -> Option<Vec<PageEntry>> {
        let cache = self.derived.find_entry_cache.read().unwrap();
        let (g, index) = cache.as_ref()?;
        (*g == gen && (index.complete || index.entries.contains_key(key)))
            .then(|| index.entries.get(key).cloned().unwrap_or_default())
    }

    #[cfg(test)]
    pub(in crate::model) fn claimant_memo_built(&self) -> bool {
        self.derived.find_entry_cache.read().unwrap().is_some()
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
    #[cfg(test)]
    pub(crate) fn list_pages_shared(&self) -> Arc<Vec<PageEntry>> {
        self.page_list().materialize()
    }

    /// The page list memo, listed afresh when it describes no current
    /// generation.
    #[cfg(test)]
    fn page_list(&self) -> Arc<EntryList> {
        loop {
            let stamp = self.name_stamp();
            if let Some((g, entries)) = self.derived.page_list_cache.read().unwrap().as_ref() {
                if *g == stamp.0 {
                    return Arc::clone(entries);
                }
            }
            let entries = self.certify(list_graph_pages(self), None);
            let (entries, _) = name_roots(&self.current_config().journals_dir, entries.0);
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
        let identity = OpeningName::new(
            &self.current_journal_format(),
            name,
            kind == PageKind::Journal,
        );
        let key = identity.claim();
        let mut entries = self
            .cached_claimants(&key, self.cache_gen.load(Ordering::Acquire))
            .unwrap_or_else(|| {
                self.complete_name_claimants()
                    .get(&key)
                    .cloned()
                    .unwrap_or_default()
            });
        entries.retain(|entry| entry.kind == kind);
        entries
    }

    /// Complete physical claims for cold openings. On a miss, walk once and
    /// read at most one preamble per non-journal file. Warm lookups share the
    /// generation's claimant map.
    pub(crate) fn complete_name_claimants(&self) -> Arc<ClaimantMap> {
        if let Some(entries) = self.complete_claim_memo(self.name_stamp()) {
            return entries;
        }
        let _walk = self
            .derived
            .cold_claim_walk
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            let stamp = self.name_stamp();
            if let Some(entries) = self.complete_claim_memo(stamp) {
                return entries;
            }
            let listed = self.certify(list_graph_pages(self), None);
            let entries = Arc::new(page_claimants(
                &self.current_config().journals_dir,
                listed.0,
            ));
            if self
                .install_name_memo(stamp, || {
                    *self.derived.find_entry_cache.write().unwrap() = Some((
                        stamp.0,
                        FindEntryIndex {
                            entries: Arc::clone(&entries),
                            complete: true,
                            held_epoch: stamp.1,
                        },
                    ));
                })
                .is_some()
            {
                return entries;
            }
        }
    }

    fn complete_claim_memo(&self, (gen, epoch): (u64, u64)) -> Option<Arc<ClaimantMap>> {
        let memo = self.derived.find_entry_cache.read().unwrap();
        let (cached_gen, index) = memo.as_ref()?;
        (*cached_gen == gen && index.held_epoch == epoch && index.complete)
            .then(|| Arc::clone(&index.entries))
    }

    /// A direct path read can observe a new or retitled file before the
    /// watcher. A live file missing from its bucket joins it when no other
    /// bucket can name it: a journals-directory file (its file name is its
    /// name) or another file this generation's page list does not list (a
    /// new file).
    /// It joins only a complete memo and only when the entry was named from
    /// the file's bytes (`trusted`). Otherwise the claimant memo drops (dropping a memo
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
            let key = Claim::of(entry);
            let known = (index.entries.get(&key)).is_some_and(|entries| {
                entries.iter().any(|candidate| candidate.path == entry.path)
            });
            if *g != gen
                || index.held_epoch != stamp.1
                || known
                || !fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_file())
            {
                return;
            }
            if !(trusted && unlisted && index.complete) {
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

    /// [`Self::snapshot_name_roots`] in the shapes the tests compare.
    #[cfg(test)]
    pub(crate) fn snapshot_name_index(
        &self,
    ) -> (
        Arc<Vec<PageEntry>>,
        HashMap<(PageKind, String), Vec<PageEntry>>,
    ) {
        let (list, claimants) = self.snapshot_name_roots();
        let mut by_name: HashMap<_, Vec<_>> = HashMap::new();
        for entry in claimants.values().flatten() {
            by_name
                .entry((entry.kind, tine_core::refs::page_key(&entry.name)))
                .or_default()
                .push(entry.clone());
        }
        (list.materialize(), by_name)
    }

    /// The launch build's roots (R2), shared by the page list memo and the
    /// first publication.
    pub(in crate::model) fn install_launch_listing(&self, gen: u64, named: Vec<PageEntry>) {
        let entries = self.certify(named, None);
        let (list, claimants) = name_roots(&self.current_config().journals_dir, entries.0);
        *self.derived.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&list)));
        *self.derived.launch_names.write().unwrap() = Some((gen, (list, claimants)));
    }

    /// The page list and claimants from one certified inventory (R2). Capture
    /// calls this under the writer, so a fallback walk needs no stamp.
    pub(crate) fn snapshot_name_roots(&self) -> NameRoots {
        let gen = self.cache_gen.load(Ordering::Acquire);
        // The build that produced this generation already listed and named
        // every file from its own reads (GH #623: one read per file); a
        // later generation walks again.
        let launch = self
            .derived
            .launch_names
            .write()
            .unwrap()
            .take()
            .filter(|(listed_gen, _)| *listed_gen == gen);
        let (list, claimants) = match launch {
            Some((_, roots)) => roots,
            None => {
                let entries = self.certify(list_graph_pages(self), None);
                name_roots(&self.current_config().journals_dir, entries.0)
            }
        };
        *self.derived.find_entry_cache.write().unwrap() = Some((
            gen,
            FindEntryIndex {
                entries: Arc::clone(&claimants),
                // Reuse known claims, but a first miss must discover files
                // that arrived after this published snapshot.
                complete: false,
                held_epoch: self.held.epoch(),
            },
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
        let complete = match memo.as_ref() {
            Some((g, _)) if *g == gen => return,
            Some((g, index)) if Some(*g) == from => index.complete,
            _ => false,
        };
        *memo = Some((
            gen,
            FindEntryIndex {
                entries: Arc::clone(claimants),
                complete,
                held_epoch: self.held.epoch(),
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
