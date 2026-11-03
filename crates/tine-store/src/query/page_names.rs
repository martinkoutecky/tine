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
        crate::model::persistent::Map<std::path::PathBuf, String>,
    >,
}

/// The real-name table with what ranks its twins: the owner of a normalized
/// name across kinds is the least claimant by
/// [`crate::model::compare_page_claimants`], the one order `resolve` uses per
/// kind (CM step 2 OW-1, I-12). A name with one claimant needs no lookup;
/// twins cost one entry-list lookup each.
#[derive(Clone)]
pub(crate) struct RealPageOwners {
    names: std::sync::Arc<RealPageNames>,
    entries: std::sync::Arc<crate::model::persistent::EntryList>,
    journal: JournalFormat,
    file_names: tine_core::config::FileNameFormat,
}
impl RealPageOwners {
    pub(crate) fn new(
        names: std::sync::Arc<RealPageNames>,
        entries: std::sync::Arc<crate::model::persistent::EntryList>,
        config: &tine_core::config::Config,
    ) -> Self {
        Self {
            names,
            entries,
            journal: config.journal_format(),
            file_names: config.file_name_format,
        }
    }
    /// No real pages (a scope that cannot change membership).
    pub(crate) fn empty(config: &tine_core::config::Config) -> Self {
        Self::new(
            std::sync::Arc::new(RealPageNames::new()),
            std::sync::Arc::default(),
            config,
        )
    }
    /// The owner of a normalized real page name, with its stored name.
    pub(crate) fn get(&self, key: &str) -> Option<(&std::path::PathBuf, &String)> {
        let bucket = self.names.claimants.get(key)?;
        if bucket.len() < 2 {
            return bucket.first();
        }
        // A claimant the entry list cannot place (none expected: both come
        // from one generation) ranks after every placed one, by path.
        bucket.iter().min_by(|(a, _), (b, _)| {
            match (self.entries.get_path(a), self.entries.get_path(b)) {
                (Some(a), Some(b)) => {
                    crate::model::compare_page_claimants(a, b, &self.journal, self.file_names)
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(b),
            }
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
            .flat_map(|bucket| bucket.iter())
    }
    fn update(&mut self, entry: &PageEntry, added: bool) {
        let key = refs::page_key(&entry.name);
        let mut bucket = self.claimants.get(&key).cloned().unwrap_or_default();
        if added {
            bucket.insert(entry.path.clone(), entry.name.clone());
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
