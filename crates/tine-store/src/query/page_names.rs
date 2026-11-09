//! Real page-name claimants used by publication and reference queries.
use super::*;

/// Every physical claimant of each real page name, shared by snapshot
/// publication and the query fallback. Each normalized name owns a persistent
/// claimant tree keyed by path, so removing a claimant costs O(log D), where D
/// is its duplicates, without scanning unrelated pages (I-12/I-25). The tree
/// is persisted in the launch checkpoint; it does not decide which claimant
/// owns the name: [`RealPageOwners::get`] asks the claimant order.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct RealPageNames {
    claimants: crate::model::persistent::Map<
        String,
        crate::model::persistent::Map<std::path::PathBuf, PageEntry>,
    >,
}

/// The real-name table with what ranks its twins: the owner of a normalized
/// name across kinds is the least claimant by
/// [`crate::model::compare_page_claimants`], the same across-kind order that
/// `resolve` uses (R13 OWN2, I-12). Only this key's claimant bucket is visited;
/// configuration and filename-format ranks play no part.
#[derive(Clone)]
pub(crate) struct RealPageOwners {
    names: std::sync::Arc<RealPageNames>,
}
impl RealPageOwners {
    pub(crate) fn new(names: std::sync::Arc<RealPageNames>) -> Self {
        Self { names }
    }
    /// No real pages (a scope that cannot change membership).
    pub(crate) fn empty() -> Self {
        Self::new(std::sync::Arc::new(RealPageNames::new()))
    }
    /// The owner of a normalized real page name, with its stored name.
    pub(crate) fn get(&self, key: &str) -> Option<(&std::path::PathBuf, &String)> {
        self.opening_entry(key)
            .map(|entry| (&entry.path, &entry.name))
    }
    pub(crate) fn opening_entry(&self, key: &str) -> Option<&PageEntry> {
        let bucket = self.names.claimants.get(key)?;
        Self::opening_claimant(bucket.iter().map(|(_, entry)| entry))
    }
    fn opening_claimant<'a>(entries: impl Iterator<Item = &'a PageEntry>) -> Option<&'a PageEntry> {
        entries.min_by(|a, b| crate::model::compare_page_claimants(a, b))
    }
    /// Active real-name owners under an identity prefix, in identity order.
    /// O(log P + C) for C matching physical claimants; no document reads.
    pub(crate) fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a PageEntry> + 'a {
        self.names
            .claimants
            .iter_from(prefix)
            .take_while(move |(key, _)| key.starts_with(prefix))
            .filter_map(|(_, bucket)| {
                Self::opening_claimant(bucket.iter().map(|(_, entry)| {
                    #[cfg(feature = "test-faults")]
                    crate::cost_counters::namespace_name_visit();
                    entry
                }))
            })
    }
}
impl RealPageNames {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    /// Every physical owner of this normalized name, including duplicates.
    /// Reuses the publication's claimant tree; no new query index is built.
    pub(crate) fn paths(&self, key: &str) -> impl Iterator<Item = &std::path::PathBuf> {
        self.claimants
            .get(key)
            .into_iter()
            .flat_map(|bucket| bucket.iter().map(|(path, _)| path))
    }
    /// Every physical owner of this normalized name with its effective name.
    pub(crate) fn owners(&self, key: &str) -> impl Iterator<Item = (&std::path::PathBuf, &String)> {
        self.claimants
            .get(key)
            .into_iter()
            .flat_map(|bucket| bucket.iter().map(|(path, entry)| (path, &entry.name)))
    }
    /// Every claim `(key, path, effective name)` whose key is at least
    /// `from`, in key order and path order within a key (`""` lists all).
    /// O(log K) to start, then O(1) amortized per claim.
    pub(crate) fn claims_from<'a>(
        &'a self,
        from: &str,
    ) -> impl Iterator<Item = (&'a String, &'a std::path::PathBuf, &'a String)> + 'a {
        self.claimants.iter_from(from).flat_map(|(key, bucket)| {
            bucket
                .iter()
                .map(move |(path, entry)| (key, path, &entry.name))
        })
    }
    fn update(&mut self, entry: &PageEntry, added: bool) {
        let key = refs::page_key(&entry.name);
        let mut bucket = self.claimants.get(&key).cloned().unwrap_or_default();
        if added {
            bucket.insert(entry.path.clone(), entry.clone());
        } else {
            bucket.remove(&entry.path);
        }
        if !bucket.is_empty() {
            self.claimants.insert(key, bucket);
        } else {
            self.claimants.remove(&key);
        }
    }
    pub(crate) fn capture(
        old: Option<(&Self, &crate::model::persistent::Pages)>,
        pages: &crate::model::persistent::Pages,
        paths: &[String],
    ) -> Self {
        if let Some((old, before)) = old {
            let mut next = old.clone();
            for path in paths {
                let was = before
                    .positions
                    .get(path)
                    .and_then(|&slot| before.get(slot))
                    .map(|p| &p.0);
                let now = pages
                    .positions
                    .get(path)
                    .and_then(|&slot| pages.get(slot))
                    .map(|p| &p.0);
                if was.map(|e| (&e.path, &e.name)) == now.map(|e| (&e.path, &e.name)) {
                    continue;
                }
                if let Some(e) = was {
                    next.update(e, false);
                }
                if let Some(e) = now {
                    next.update(e, true);
                }
            }
            next
        } else {
            let mut next = Self::new();
            for (entry, _) in pages {
                next.update(entry, true);
            }
            next
        }
    }
}

pub(crate) fn real_page_names(graph: &ReadSnapshot) -> RealPageOwners {
    graph.real_page_owners()
}
