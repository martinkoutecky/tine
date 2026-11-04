//! Asset references by page, with a graph-wide count per asset name: what the
//! orphan-asset guard and the unused-asset listing ask (cost-model audit
//! 2026-11-02, row 9a; D-10). In memory only, never checkpointed: a
//! generation builds it on first use with one walk of its pages, and a later
//! generation carries it by the pages its publication changed, as the
//! referenced-name index does. Each page's names come from the one collector
//! the full scan used (`tine_core::asset_refs`: parser-accepted links and
//! images in Markdown and Org, macros' `assets/` mentions and the conservative
//! plaintext fallback), so the index protects exactly what the scan did.
use super::*;

#[derive(Clone, Default)]
pub(crate) struct AssetRefIndex {
    /// Each page holding at least one asset reference, by its absolute path.
    by_page: SharedMap<PathBuf, Arc<[String]>>,
    /// How many pages reference each asset name (raw and decoded spellings).
    by_name: SharedMap<String, usize>,
}

impl AssetRefIndex {
    /// One walk of every page. Cost O(P + scanned text bytes).
    pub(super) fn build(pages: &Pages) -> Self {
        let mut index = Self::default();
        for (entry, doc) in pages.iter() {
            index.add(&entry.path, doc);
        }
        index
    }

    /// `previous` (built for `previous_pages`) moved to `pages` by the pages
    /// at `changed_paths`. `None` when the change list cannot describe the
    /// move (a whole-table replacement): the next reader rebuilds. Cost
    /// O(changed pages' text + changed names x log A).
    pub(super) fn carry(
        previous: &Self,
        previous_pages: &Arc<Pages>,
        pages: &Arc<Pages>,
        changed_paths: &[String],
        positions: Option<(&SharedMap<String, usize>, &SharedMap<String, usize>)>,
    ) -> Option<Self> {
        if changed_paths.is_empty() {
            return Arc::ptr_eq(previous_pages, pages).then(|| previous.clone());
        }
        let mut index = previous.clone();
        for rel in changed_paths {
            if let Some((entry, _)) =
                snapshot_page_by_rel(previous_pages, positions.map(|(before, _)| before), rel)
            {
                index.remove(&entry.path);
            }
            if let Some((entry, doc)) =
                snapshot_page_by_rel(pages, positions.map(|(_, after)| after), rel)
            {
                index.remove(&entry.path);
                index.add(&entry.path, doc);
            }
        }
        Some(index)
    }

    fn add(&mut self, path: &Path, doc: &Document) {
        let mut names = HashSet::new();
        collect_document_asset_refs(doc, &mut names);
        if names.is_empty() {
            return;
        }
        for name in &names {
            let count = self.by_name.get(name.as_str()).copied().unwrap_or(0);
            self.by_name.insert(name.clone(), count + 1);
        }
        self.by_page
            .insert(path.to_path_buf(), names.into_iter().collect());
    }

    fn remove(&mut self, path: &Path) {
        let Some(names) = self.by_page.get(path).cloned() else {
            return;
        };
        self.by_page.remove(path);
        for name in names.iter() {
            match self.by_name.get(name.as_str()).copied() {
                Some(count) if count > 1 => self.by_name.insert(name.clone(), count - 1),
                _ => self.by_name.remove(name.as_str()),
            }
        }
    }

    /// Whether any page references `name`. Cost O(log A).
    pub(crate) fn references(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Every referenced name. Cost O(A).
    pub(crate) fn names(&self) -> impl Iterator<Item = &String> {
        self.by_name.iter().map(|(name, _)| name)
    }
}

impl ReadSnapshot {
    /// Whether this generation holds its asset index (built or inherited).
    pub(crate) fn asset_refs_built(&self) -> bool {
        self.asset_refs.get().is_some()
    }

    /// This generation's asset-reference index; the first reader of a
    /// generation that did not inherit one builds it.
    pub(crate) fn asset_refs(&self) -> &AssetRefIndex {
        let index = self
            .asset_refs
            .get_or_init(|| AssetRefIndex::build(&self.pages));
        #[cfg(test)]
        {
            // Oracle: the carried index answers what a full scan does.
            let mut scanned = HashSet::new();
            for (_, doc) in self.pages.iter() {
                tine_core::asset_refs::collect_document_asset_refs(doc, &mut scanned);
            }
            let indexed: HashSet<String> = index.names().cloned().collect();
            assert_eq!(
                indexed, scanned,
                "asset reference index drifted from a full scan"
            );
        }
        index
    }
}

impl ReadSnapshot {
    /// Whether this generation holds a parsed page for the file `path`
    /// (graph-relative `rel`). Cost O(log P).
    pub(crate) fn holds_page_at(&self, rel: &str, path: &Path) -> bool {
        snapshot_page_by_rel(&self.pages, None, rel).is_some_and(|(entry, _)| entry.path == path)
    }

    /// Whether `other` shares this generation's page table.
    pub(crate) fn same_pages(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pages, &other.pages)
    }
}

impl ReadSnapshot {
    /// The parsed page at graph-relative `rel` with its slot (page order).
    /// Cost O(log P).
    pub(crate) fn parsed_page(&self, rel: &str) -> Option<(usize, &(PageEntry, Arc<Document>))> {
        let slot = *self.pages.positions.get(rel)?;
        let page = self.pages.get(slot)?;
        (page.0.rel_path_str() == rel).then_some((slot, page))
    }

    /// Every parsed page in page order (test oracles only).
    #[cfg(test)]
    pub(crate) fn pages_in_order(&self) -> impl Iterator<Item = &(PageEntry, Arc<Document>)> {
        self.pages.iter()
    }
}
