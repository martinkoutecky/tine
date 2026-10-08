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
    Simple(String, Option<String>),
    Advanced(String, Option<String>),
    Tql(String, Option<String>),
}

impl QueryCacheKey {
    fn source_len(&self) -> usize {
        match self {
            Self::Simple(source, page) | Self::Advanced(source, page) | Self::Tql(source, page) => {
                source.len() + page.as_ref().map_or(0, String::len)
            }
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

use crate::render::RenderGraph;
use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::wire_parse::{anchored_view, parse_query_pair, QueryTextDialect};
use tine_store::{IrAnswer, IrRequest};

impl RenderGraph<'_> {
    pub(crate) fn query_bounded(
        &self,
        source: &str,
        dialect: QueryTextDialect,
        current_page: Option<&str>,
    ) -> BoundedGroups {
        self.query_parsed(source, dialect, &[], current_page)
            .map(|(_, bounded)| bounded)
            .unwrap_or_else(|| BoundedGroups {
                groups: Vec::new(),
                pages: Vec::new(),
                total: 0,
                exceeded: false,
            })
    }

    /// The ONE static query run: parse `source` under the host block's `tine.*`
    /// properties, run it under the view the app runs it under, and return the
    /// parse (its `view` and block presentation drive a query-backed sheet)
    /// beside the bounded answer.
    pub(crate) fn query_parsed(
        &self,
        source: &str,
        dialect: QueryTextDialect,
        block_properties: &[(String, String)],
        current_page: Option<&str>,
    ) -> Option<(tine_core::query::wire_parse::ParsedQuery, BoundedGroups)> {
        let Ok(IrAnswer::Registry(registry)) = self.whole.query_ir(IrRequest::Registry) else {
            return None;
        };
        // Only the block's `tine.*` properties reach the engine, as in the live macro.
        let host: Vec<(String, String)> = block_properties
            .iter()
            .filter(|(key, _)| key.starts_with("tine."))
            .cloned()
            .collect();
        let execution = current_page.and_then(|page| substitute_current_page(source, page));
        let parsed = parse_query_pair(
            execution.as_deref().unwrap_or(source),
            dialect,
            &host,
            &registry,
        );
        let view = anchored_view(&parsed, parsed.query.anchor);
        let context = ExecutionContext {
            current_page: current_page.map(str::to_owned),
        };
        let Ok(IrAnswer::Result(answer)) = self.whole.query_ir(IrRequest::Run {
            query: &parsed.query,
            view: &view,
            context: &context,
        }) else {
            return None;
        };
        let (groups, pages) = match answer.rows {
            QueryRows::Block { groups } => (groups, Vec::new()),
            QueryRows::Page { pages } => (Vec::new(), pages),
        };
        let bounded = BoundedGroups {
            groups,
            pages,
            total: answer.total,
            exceeded: answer.exceeded,
        };
        Some((parsed, bounded))
    }
}

pub(crate) fn substitute_current_page(argument: &str, page: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::new();
    let mut rest = argument;
    while let Some(start) = rest.find("<%") {
        let Some(end) = rest[start..].find("%>") else {
            break;
        };
        let inner = &rest[start + 2..start + end];
        if inner.trim().eq_ignore_ascii_case("current page") {
            out.push_str(&rest[..start]);
            out.push_str(&format!("[[{page}]]"));
            changed = true;
        } else {
            out.push_str(&rest[..start + end + 2]);
        }
        rest = &rest[start + end + 2..];
    }
    if !changed {
        return None;
    }
    out.push_str(rest);
    Some(out)
}
