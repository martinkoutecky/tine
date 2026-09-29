//! Per-export query answer cache shared by static pages. It retains only a
//! bounded number of bounded IR results so repeated macros do not rerun the
//! whole-graph answerer for every page.

use std::cell::RefCell;
use std::collections::HashMap;
use tine_core::model::RefGroup;
use tine_core::query::ir::PageRow;

#[derive(Clone)]
pub(crate) struct BoundedGroups {
    pub groups: Vec<RefGroup>,
    pub pages: Vec<PageRow>,
    pub total: usize,
    pub exceeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum QueryCacheKey {
    Simple(String),
    Advanced(String),
    Tql(String),
}

impl QueryCacheKey {
    fn source_len(&self) -> usize {
        match self {
            Self::Simple(source) | Self::Advanced(source) | Self::Tql(source) => source.len(),
        }
    }
}

pub(crate) const QUERY_CACHE_MAX_ENTRIES: usize = 64;
pub(crate) const QUERY_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
/// A 64-entry, 32 MiB export-local memo. A result too large to cache is
/// rendered once and omitted from the memo; publication still proceeds.
pub(crate) struct QueryCache {
    pub(crate) entries: HashMap<QueryCacheKey, BoundedGroups>,
    pub(crate) bytes: usize,
}

impl QueryCache {
    /// Return an owned cached answer for this exact source and dialect.
    pub fn get(&self, key: &QueryCacheKey) -> Option<BoundedGroups> {
        self.entries.get(key).cloned()
    }

    /// Retain a bounded answer when it fits the per-export memo budget.
    pub fn insert(&mut self, key: QueryCacheKey, groups: BoundedGroups) {
        if self.entries.contains_key(&key) || self.entries.len() >= QUERY_CACHE_MAX_ENTRIES {
            return;
        }
        let bytes = key
            .source_len()
            .saturating_add(tine_core::model::ref_groups_estimated_bytes(&groups.groups))
            .saturating_add(
                groups
                    .pages
                    .iter()
                    .map(|page| page.name.len() + page.path.len() + 256)
                    .sum::<usize>(),
            )
            .saturating_add(256);
        if bytes > QUERY_CACHE_MAX_BYTES || self.bytes.saturating_add(bytes) > QUERY_CACHE_MAX_BYTES
        {
            return;
        }
        self.bytes += bytes;
        self.entries.insert(key, groups);
    }
}

pub(crate) type SharedQueryCache = RefCell<QueryCache>;
