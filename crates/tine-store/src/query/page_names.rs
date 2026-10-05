//! Real page-name winners used by publication and reference queries.
use super::*;

/// One lexicographic physical-path winner answer for real page names, shared
/// by snapshot publication and the query fallback. Each normalized name owns a
/// persistent ordered claimant tree, so removing its winner costs O(log D),
/// where D is its duplicates, without scanning unrelated pages (I-12/I-25).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct RealPageNames {
    claimants: crate::model::persistent::Map<
        String,
        crate::model::persistent::Map<std::path::PathBuf, String>,
    >,
}
impl RealPageNames {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn get(&self, key: &str) -> Option<&(std::path::PathBuf, String)> {
        self.claimants
            .get(key)
            .and_then(|bucket| bucket.first_entry())
    }
    /// Every physical owner of this normalized name, including duplicates.
    /// Reuses the publication's claimant tree; no new query index is built.
    pub(crate) fn paths(&self, key: &str) -> impl Iterator<Item = &std::path::PathBuf> {
        self.claimants
            .get(key)
            .into_iter()
            .flat_map(|bucket| bucket.iter().map(|(path, _)| path))
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

pub(crate) fn real_page_names(graph: &impl GraphRead) -> std::sync::Arc<RealPageNames> {
    graph.reference_real_page_names().unwrap_or_else(|| {
        std::sync::Arc::new(graph.with_pages(|pages| RealPageNames::capture(None, pages, &[])))
    })
}
