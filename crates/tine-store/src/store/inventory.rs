//! Name inventory of a whole-graph view: file-claimed names, aliases and
//! reference-only names, and the file-claimed subset a page rename needs.
use super::*;

impl WholeGraph {
    /// Names and file claimants in this view. Physical twins with the same
    /// decoded spelling produce one name entry whose target contains the
    /// canonical id and the other claimants; `scan_area` lists physical files.
    /// Ordinary page twins can both contribute parsed search and backlink
    /// content; duplicate-day journal strays are absent from view queries but
    /// can be read directly by file id. Such a read does not add them to this
    /// view or publish a change; they do not contribute references or backlinks.
    /// They still appear as `others` of the day's `Resolved::Existing` target.
    // The parsed whole-graph cache omits duplicate-day journal strays.
    /// A first call can traverse all
    /// page blocks and reference text to build indexes; later calls cost
    /// O(P + B + aliases + R + N log N) on the first call, where R is
    /// reference-only names and N is inventory entries. Alias owners are
    /// indexed once per snapshot instead of rescanned for every name.
    /// The result is stable in this view.
    pub fn inventory(&self) -> Arc<Inventory> {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        let mut claimed_names = HashSet::new();
        let mut page_claimed_names = HashSet::new();
        for page in self.list.iter() {
            let key = tine_core::refs::page_key(&page.name);
            if !visited.insert((page.kind == PageKind::Journal, key.clone())) {
                continue;
            }
            for entry in self.claimed_entries(page, key) {
                let key = tine_core::refs::page_key(&entry.name);
                if !entry.is_journal {
                    page_claimed_names.insert(key.clone());
                }
                claimed_names.insert(key);
                entries.push(entry);
            }
        }
        let references = self.graph.referenced_page_names();
        let reference_spelling: HashMap<_, _> = references
            .iter()
            .map(|name| (tine_core::refs::page_key(name), name.as_str()))
            .collect();
        // One entry per alias name, owners sorted by path (rev 5
        // `Resolved::Alias`). An alias that is also a page file's name gets no
        // entry: `resolve` prefers the file, and every entry's target must be
        // the answer `resolve` gives for its name.
        let mut alias_owners: BTreeMap<String, (String, Vec<PageId>)> = BTreeMap::new();
        for (alias, _, owner) in self.graph.page_aliases_with_owners() {
            let key = tine_core::refs::page_key(&alias);
            if page_claimed_names.contains(&key) {
                continue;
            }
            let spelling = reference_spelling
                .get(&key)
                .copied()
                .unwrap_or(&alias)
                .to_owned();
            let slot = alias_owners
                .entry(key)
                .or_insert_with(|| (spelling, Vec::new()));
            let owner = PageId::from(owner);
            if !slot.1.contains(&owner) {
                slot.1.push(owner);
            }
        }
        let alias_names: HashSet<String> = alias_owners.keys().cloned().collect();
        for (_, (name, mut owners)) in alias_owners {
            owners.sort();
            entries.push(InventoryEntry {
                name,
                target: Resolved::Alias { owners },
                is_journal: false,
                day: None,
            });
        }
        for name in references {
            let key = tine_core::refs::page_key(&name);
            if claimed_names.contains(&key) || alias_names.contains(&key) {
                continue;
            }
            entries.push(InventoryEntry {
                target: self.resolve(&name, false),
                name,
                is_journal: false,
                day: None,
            });
        }
        sort_inventory(&mut entries);
        #[cfg(feature = "test-faults")]
        crate::cost_counters::name_inventory_entries(entries.len());
        Arc::new(Inventory(entries))
    }

    /// The [`Self::inventory`] entries that a file claims (pages and
    /// journals, `is_journal` telling them apart) whose page key is `key` or
    /// lies under `key/`: a page and its namespace descendants, each with the
    /// same file target and in the same relative order as the full
    /// inventory. An alias or reference-only name is never a file claim (the
    /// inventory skips both for a claimed key), so this is exactly the
    /// file-claimed subset a rename moves, without building the alias and
    /// reference-name indexes the full inventory needs (GH #623). Cost O(P)
    /// page-key computations plus O(m log m) for m matches; no parse and no
    /// disk access.
    pub fn page_files_at_or_under(&self, key: &str) -> Vec<InventoryEntry> {
        let prefix = format!("{key}/");
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        for page in self.list.iter() {
            let page_key = tine_core::refs::page_key(&page.name);
            if (page_key != key && !page_key.starts_with(&prefix))
                || !visited.insert((page.kind == PageKind::Journal, page_key.clone()))
            {
                continue;
            }
            entries.extend(self.claimed_entries(page, page_key));
        }
        sort_inventory(&mut entries);
        entries
    }

    /// One entry per spelling of `page`'s claimant bucket (`key` is its page
    /// key), each with the target `resolve` gives: the bucket's first
    /// claimant, then every other claimant.
    fn claimed_entries(&self, page: &PageEntry, key: String) -> Vec<InventoryEntry> {
        let is_journal = page.kind == PageKind::Journal;
        let mut names: Vec<String> = Vec::new();
        let mut ids: Vec<PageId> = Vec::new();
        if let Some(claimants) = self.claimants.get(&(is_journal, key)) {
            for claimant in claimants {
                if let Some(id) = &claimant.rel_path {
                    if !names.contains(&claimant.name) {
                        names.push(claimant.name.clone());
                    }
                    ids.push(id.clone());
                }
            }
        }
        names
            .into_iter()
            .map(|name| InventoryEntry {
                name,
                target: Resolved::Existing {
                    id: ids[0].clone(),
                    others: ids[1..].to_vec(),
                },
                is_journal,
                day: page.date_key.map(Day),
            })
            .collect()
    }
}

fn sort_inventory(entries: &mut [InventoryEntry]) {
    entries
        .sort_by_cached_key(|entry| (tine_core::refs::page_key(&entry.name), entry.name.clone()));
}
