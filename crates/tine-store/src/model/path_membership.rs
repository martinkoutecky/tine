//! Exact existence of block path-reference rows, owned by published snapshots.
//! The core closure and query reference-class resolver supply all semantics.
//! Counts are memory-only persistent trees; no block DTO or per-block closure is
//! retained. Reads cost O(component × log targets); edits inspect changed pages.
use super::{persistent::Map, Document, PageEntry, ReadSnapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tine_core::query::{
    atom::ParseConfig,
    path_refs::{closure_names, PathRefCounts},
};

#[derive(Clone, Default)]
pub(super) struct PathMembership {
    names: Map<String, usize>,
    days: Map<i64, usize>,
    changed: Vec<String>,
}

/// Union of the existing block closure over every query row on one page.
/// Ancestor names already occur on their ancestor's row, so this existential
/// projection need not materialize inherited closures. Top-level suppression
/// cannot remove the first matching ancestor. Pre-blocks are independent peers.
fn page_names(entry: &PageEntry, doc: &Document, config: &ParseConfig) -> BTreeSet<String> {
    #[cfg(test)]
    PAGES.with(|count| count.set(count.get() + 1));
    let page = tine_core::refs::normalize(&entry.name);
    let mut names = BTreeSet::new();
    let mut add = |block: &tine_core::doc::DocBlock, _: &PathRefCounts| {
        names.extend(closure_names(
            &page,
            &block.projection().refs_norm(),
            &PathRefCounts::new(),
        ));
    };
    for block in doc
        .pre_block
        .as_deref()
        .into_iter()
        .flat_map(|pre| crate::query::pre_blocks_with_config(entry, pre, config))
    {
        add(&block, &PathRefCounts::new());
    }
    crate::query::eval::walk_path_refs(&doc.roots, false, &mut add);
    names
}

fn adjust<K: Ord + Clone>(map: &mut Map<K, usize>, key: K, amount: usize, add: bool) {
    let count = map.get(&key).copied().unwrap_or(0);
    let count = if add { count + amount } else { count - amount };
    if count == 0 {
        map.remove(&key);
    } else {
        map.insert(key, count);
    }
}

impl PathMembership {
    pub(super) fn capture(
        old: Option<&ReadSnapshot>,
        next: &ReadSnapshot,
        paths: &[String],
    ) -> Self {
        // Spelling selection is a shared graph answer. Warm its existing owner
        // at load so even the first date link patches it rather than scanning.
        next.warm_path_reference_spellings(old, paths);
        let config = ParseConfig::from_config(next.config());
        let Some(old) = old.filter(|old| !paths.is_empty() || Arc::ptr_eq(&old.pages, &next.pages))
        else {
            let mut counts = BTreeMap::<String, usize>::new();
            for (entry, doc) in next.pages.iter() {
                for name in page_names(entry, doc, &config) {
                    *counts.entry(name).or_default() += 1;
                }
            }
            let mut days = BTreeMap::<i64, usize>::new();
            for (name, count) in &counts {
                if let Some(day) = next.path_reference_day(name) {
                    *days.entry(day).or_default() += count;
                }
            }
            return Self {
                names: Map::from_sorted_unique(counts.into_iter().collect()),
                days: Map::from_sorted_unique(days.into_iter().collect()),
                changed: Vec::new(),
            };
        };
        let mut result = Self {
            names: old.path_membership.names.clone(),
            days: old.path_membership.days.clone(),
            changed: Vec::new(),
        };
        let mut dirty = BTreeSet::new();
        for path in paths {
            let names = |graph: &ReadSnapshot| {
                graph
                    .pages
                    .positions
                    .get(path)
                    .and_then(|&slot| graph.pages.get(slot))
                    .map(|(entry, doc)| {
                        (
                            page_names(entry, doc, &config),
                            Some(tine_core::refs::page_key(&entry.name)),
                        )
                    })
                    .unwrap_or_default()
            };
            let ((before, before_name), (after, after_name)) = (names(old), names(next));
            // Even an empty ordinary page can claim a date spelling and stop
            // it denoting a journal day, without contributing a block row.
            dirty.extend(before_name.into_iter().chain(after_name));
            for name in before.difference(&after) {
                adjust(&mut result.names, name.clone(), 1, false);
            }
            for name in after.difference(&before) {
                adjust(&mut result.names, name.clone(), 1, true);
            }
            // A spelling can change without changing its folded membership.
            dirty.extend(before.into_iter().chain(after));
        }
        dirty.extend(
            next.referenced_name_table()
                .changed_spellings()
                .iter()
                .cloned(),
        );
        for name in dirty {
            let was = old.path_membership.names.get(&name).copied().unwrap_or(0);
            let now = result.names.get(&name).copied().unwrap_or(0);
            let (old_day, new_day) = (
                old.path_reference_day(&name),
                next.path_reference_day(&name),
            );
            if was == now && old_day == new_day {
                continue;
            }
            if let Some(day) = old_day {
                adjust(&mut result.days, day, was, false);
            }
            if let Some(day) = new_day {
                adjust(&mut result.days, day, now, true);
            }
            if (was == 0) != (now == 0) || old_day != new_day {
                result.changed.push(name);
            }
        }
        result
    }
}

impl ReadSnapshot {
    /// Exact `(tag name)` existence. O(alias component × log targets), with
    /// no document visits, DTOs, I/O or query parsing; always this generation.
    pub(crate) fn tag_table_available(&self, name: &str) -> bool {
        let (keys, days) = crate::query::reference_class(self, name, false);
        keys.into_iter()
            .chain(std::iter::once(tine_core::refs::page_key(name)))
            .any(|key| self.path_membership.names.contains_key(&key))
            || days
                .iter()
                .any(|day| self.path_membership.days.contains_key(day))
    }

    /// Membership changes carried by this generation, for transport-only
    /// invalidation. Empty when the same snapshot is published again.
    pub(crate) fn path_reference_membership_changes<'a>(&'a self, old: &Self) -> &'a [String] {
        if std::ptr::eq(self, old) {
            &[]
        } else {
            &self.path_membership.changed
        }
    }

    pub(crate) fn query_index(&self) -> Arc<crate::query::index::QueryIndex> {
        let positions = Arc::clone(&self.reference_candidate_index.read().unwrap().positions);
        self.query_index
            .get(&self.pages, &positions, &self.config, self.cache_generation)
    }
    pub(crate) fn physical_explicit_referrers(&self, names: &[String]) -> Vec<crate::PageId> {
        let paths = self.explicit_index.candidates(names);
        self.physical_pages
            .iter()
            .filter(|(entry, _)| paths.contains(&entry.path))
            .filter_map(|(entry, _)| entry.rel_path.clone())
            .collect()
    }
}

#[cfg(test)]
thread_local! { static PAGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
#[path = "path_membership_tests.rs"]
mod tests;
