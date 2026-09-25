//! Interim whole-graph read boundary. `Store` owns the legacy graph for this
//! batch; all questions below read its live cache. A first call can build that
//! cache in O(P + B + disk). The selected reads are bounded inside this module.
//! `WholeGraph` does not yet pin an immutable generation: two calls on one view
//! may observe different states. Immutable snapshots arrive with B7.
//!
//! Questions and costs after cache construction (`P` pages, `B` blocks):
//! `rev` O(1); `backlinks`, `unlinked_references`, `block_referrers`,
//! `block_ref_counts`, `find_blocks`, `property_facets`, and `templates` O(B)
//! worst case; `backlink_filter_context` O(B) with selected roots;
//! `blocks` and `preview_block` O(blocks of hinted pages), O(B) without a hint;
//! `complete_page_names` O(P + aliases + referenced names);
//! `export_query_subtrees` O(64 × B + selected nodes); `page_icons`
//! O(names + aliases); `journal_content_days` O(journals + their blocks).
//! Calls that exceed fixed request or result limits return `QueryError`; a
//! cancelled block search returns `Cancelled`, never a partial answer. Callers
//! need no cache state, budget constants, lane IDs, or disk paths.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockPreview, BoundedRefGroups, PageEntry,
    PageKind, RefGroup, TemplateDto,
};
pub use tine_core::model::{FileId, PageId};
use tine_core::query::{AdvancedResult, QueryExportBatch, QueryExportSpec};
use tine_core::query_plan::QueryExecution;

use crate::model::Graph;

const RESULT_BRIDGE_MAX_ROWS: usize = 20_000;
const RESULT_BRIDGE_MAX_BYTES: usize = 32 * 1024 * 1024;
const AUTOCOMPLETE_FACET_MAX_ITEMS: usize = 2_000;
const AUTOCOMPLETE_FACET_MAX_BYTES: usize = 2 * 1024 * 1024;
const QUERY_EXPORT_MAX_QUERIES: usize = 64;
const QUERY_EXPORT_REQUEST_MAX_QUERIES: usize = 1_024;
const QUERY_EXPORT_MAX_QUERY_BYTES: usize = 64 * 1024;
const QUERY_EXPORT_MAX_ROOTS: usize = 50;
const QUERY_EXPORT_MAX_NODES: usize = 2_000;
const QUERY_EXPORT_MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PREVIEW_NODES: usize = 2_000;
const PREVIEW_MAX_BYTES: usize = RESULT_BRIDGE_MAX_BYTES - 4 * 1024;

/// Interim owner of a legacy graph. Constructing it is O(1); reads can build
/// the whole cache in O(P + B + disk) on first use.
pub struct Store {
    graph: Arc<Graph>,
}

impl Store {
    /// Adopt the current graph without loading it. O(1). Removed in B7.
    pub fn from_legacy(graph: Arc<Graph>) -> Self {
        Self { graph }
    }

    /// Type a file name within one configured graph area. Validation repeats
    /// whenever an id is used, including after deserialization.
    pub fn file_id(&self, area: Area, rel: &str) -> Result<FileId, StoreError> {
        let directory = match area {
            Area::Pages => &self.graph.config.pages_dir,
            Area::Journals => &self.graph.config.journals_dir,
            Area::Assets => "assets",
            Area::Meta => "logseq",
            Area::Trash => "logseq/.tine-trash",
        };
        let id = FileId::from(format!("{directory}/{rel}"));
        self.validate_file(&id)?;
        Ok(id)
    }

    pub fn as_page(&self, file: &FileId) -> Option<PageId> {
        self.validate_file(file).ok()?;
        let path = file.as_str();
        let area = path.split('/').next()?;
        if area != self.graph.config.pages_dir && area != self.graph.config.journals_dir {
            return None;
        }
        let stem = std::path::Path::new(path).file_stem()?.to_str()?;
        if tine_core::model::is_sync_conflict(stem) {
            return None;
        }
        Some(PageId::from(path))
    }

    fn validate_file(&self, file: &FileId) -> Result<(), StoreError> {
        let path = file.as_str();
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        let area = path.split('/').next().unwrap_or_default();
        if area == self.graph.config.pages_dir || area == self.graph.config.journals_dir {
            if self.graph.resolve_rel(path).is_none() {
                return Err(StoreError::InvalidTarget(path.to_owned()));
            }
        } else if area != "assets" && area != "logseq" {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        Ok(())
    }

    /// Get a live-cache read view and record its current generation. O(1).
    /// This interim implementation has no load failure or wait; the first
    /// question on the view may build the cache in O(P + B + disk).
    pub fn whole_graph(&self) -> Result<WholeGraph, LoadError> {
        Ok(WholeGraph {
            graph: Arc::clone(&self.graph),
            rev: GraphRev(self.graph.cache_generation()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Pages,
    Journals,
    Assets,
    Meta,
    Trash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Day(pub i64);

#[derive(Debug)]
pub enum StoreError {
    /// Reserved for B4 single-file reads.
    NotFound,
    InvalidTarget(String),
    /// Reserved for B4 single-file reads.
    Undecodable,
    /// Reserved for B4 single-file reads.
    Unparseable(String),
    /// Reserved for B4 single-file reads.
    TooLarge {
        limit: u64,
        len: u64,
    },
    /// Reserved for B4 single-file reads.
    Io(std::io::Error),
    /// Reserved for B7 lifecycle.
    Closed,
}

pub enum Resolved {
    Existing { id: PageId, others: Vec<PageId> },
    Alias { owners: Vec<PageId> },
    Absent { id: PageId },
}
pub struct SearchRequest {
    pub text: String,
    pub within: Option<PageId>,
    pub page_limit: usize,
    pub block_limit: usize,
    pub explain: bool,
}
pub enum QueryDialect {
    Simple,
    Advanced,
}
pub enum QueryResult {
    Simple(Arc<Vec<RefGroup>>),
    Advanced(AdvancedResult),
}

/// Cache generation seen at `whole_graph()`. It is not a consistency guard.
/// Ordered and serialized as a decimal string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GraphRev(u64);

impl TryFrom<String> for GraphRev {
    type Error = std::num::ParseIntError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse().map(Self)
    }
}
impl From<GraphRev> for String {
    fn from(value: GraphRev) -> Self {
        value.0.to_string()
    }
}

/// Initial load failure. The interim `Store::whole_graph` cannot produce one.
#[derive(Debug)]
pub enum LoadError {
    /// Background load failed; introduced with B7.
    Failed { reason: String },
    /// Store closed; introduced with B7.
    Closed,
}

/// A whole-graph request failed before returning a partial answer.
#[derive(Debug)]
pub enum QueryError {
    InvalidTarget(String),
    /// Request exceeds the store's fixed input budget.
    RequestTooLarge {
        what: Budget,
        count: usize,
        limit: usize,
    },
    /// Export request exceeds the fixed macro or source-byte budget. Extra
    /// counts retain the existing frontend error text during this migration.
    ExportRequestTooLarge {
        macros: usize,
        bytes: usize,
        macro_limit: usize,
        byte_limit: usize,
        processing_cap: usize,
    },
    /// Evaluation reached the store's fixed result budget.
    ResultTooLarge {
        what: Budget,
        count: usize,
        limit: usize,
        bytes: Option<usize>,
        byte_limit: usize,
    },
    /// Query syntax error (reserved for later batches).
    Parse(String),
    /// Caller set the cancellation flag; no partial answer is returned.
    Cancelled,
}

#[derive(Clone, Copy, Debug)]
pub enum Budget {
    BacklinkFilterRoots,
    MatchingBlocks,
    BridgeMatchingBlocks,
    RequestedBlockRefs,
    ResolvedBlockRows,
    ExportBytes,
    PropertyFacets,
    AdvancedQueryMatches,
    SearchHits,
}

impl QueryError {
    /// Check the final transport estimate after the adapter has assembled groups.
    pub fn bridge_matching_blocks(rows: usize, bytes: usize) -> Option<Self> {
        (rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES).then_some(
            Self::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count: rows,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            },
        )
    }

    /// Check the final transport estimate after search serialization fields are known.
    pub fn bridge_search_hits(hits: usize, bytes: usize) -> Option<Self> {
        (hits > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES).then_some(
            Self::ResultTooLarge {
                what: Budget::SearchHits,
                count: hits,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            },
        )
    }
}

/// Caller-owned cancellation flag, checked before each search page and block.
pub struct Cancel(pub Arc<AtomicBool>);

/// Facet answer policy: reject oversized query-builder results or return the
/// editor's bounded autocomplete prefix. Both cost O(B) on a cold cache.
pub enum FacetPolicy {
    Budgeted,
    Truncated,
}

/// Live-cache graph-wide questions. Clone is O(1). A first question may build
/// the cache in O(P + B + disk); later calls do not wait for a load. This is
/// not yet an immutable snapshot: successive calls can see different states.
#[derive(Clone)]
pub struct WholeGraph {
    graph: Arc<Graph>,
    rev: GraphRev,
}

fn bounded(result: BoundedRefGroups, what: Budget) -> Result<Arc<Vec<RefGroup>>, QueryError> {
    if result.exceeded {
        Err(QueryError::ResultTooLarge {
            what,
            count: result.total,
            limit: RESULT_BRIDGE_MAX_ROWS,
            bytes: None,
            byte_limit: RESULT_BRIDGE_MAX_BYTES,
        })
    } else {
        Ok(result.groups)
    }
}

impl WholeGraph {
    /// Resolve a name using the configured file naming rules. Real files win
    /// before aliases; all claimants share the same deterministic order.
    pub fn resolve(&self, name: &str, is_journal: bool) -> Resolved {
        let kind = if is_journal {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let entries = self.graph.find_claimants(name, kind);
        if !entries.is_empty() {
            let mut ids = entries
                .into_iter()
                .map(|entry| entry.rel_path.expect("file claimant has a path"));
            return Resolved::Existing {
                id: ids.next().unwrap(),
                others: ids.collect(),
            };
        }
        if !is_journal {
            let owners: Vec<_> = self
                .graph
                .page_aliases_with_owners()
                .into_iter()
                .filter(|(alias, _, _)| tine_core::refs::same_page(alias, name))
                .map(|(_, _, path)| PageId::from(path))
                .collect();
            if !owners.is_empty() {
                return Resolved::Alias { owners };
            }
        }
        Resolved::Absent {
            id: PageId::from(self.graph.rel_path(&self.graph.path_for(name, kind))),
        }
    }

    fn validated_page(&self, id: &PageId) -> Result<(), QueryError> {
        let store = Store {
            graph: Arc::clone(&self.graph),
        };
        let file = id.file();
        if store.as_page(&file).is_none() {
            return Err(QueryError::InvalidTarget(id.as_str().to_owned()));
        }
        Ok(())
    }

    /// Execute one query macro with the same bounded evaluator as v0.6.5.
    /// The current evaluator ignores the current page; the id is validated.
    pub fn query(
        &self,
        source: &str,
        dialect: QueryDialect,
        current_page: Option<&PageId>,
    ) -> Result<QueryResult, QueryError> {
        if let Some(id) = current_page {
            self.validated_page(id)?;
        }
        match dialect {
            QueryDialect::Simple => bounded(
                self.graph.run_query_bounded(
                    source,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                ),
                Budget::MatchingBlocks,
            )
            .map(QueryResult::Simple),
            QueryDialect::Advanced => {
                let (result, exceeded, total) = self.graph.run_advanced_query_bounded_cached(
                    source,
                    None,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err(QueryError::ResultTooLarge {
                        what: Budget::AdvancedQueryMatches,
                        count: total,
                        limit: RESULT_BRIDGE_MAX_ROWS,
                        bytes: None,
                        byte_limit: RESULT_BRIDGE_MAX_BYTES,
                    })
                } else {
                    Ok(QueryResult::Advanced(result))
                }
            }
        }
    }

    /// Graph search, including an exact file scope and caller cancellation.
    pub fn search(
        &self,
        req: &SearchRequest,
        cancel: &Cancel,
    ) -> Result<QueryExecution, QueryError> {
        let scope = match &req.within {
            Some(id) => {
                self.validated_page(id)?;
                Some(crate::query_plan::QueryPageScope {
                    name: String::new(),
                    page_kind: PageKind::Page,
                    path: Some(id.as_str().to_owned()),
                })
            }
            None => None,
        };
        let page_limit = req.page_limit.min(RESULT_BRIDGE_MAX_ROWS);
        let block_limit = req.block_limit.min(RESULT_BRIDGE_MAX_ROWS - page_limit);
        let result = self.graph.run_graph_search_latest_scoped(
            cancel,
            &req.text,
            page_limit,
            block_limit,
            scope,
            req.explain,
        );
        if result.cancelled {
            Err(QueryError::Cancelled)
        } else {
            Ok(result)
        }
    }

    /// Cache generation at acquisition, O(1); later reads may see newer data.
    pub fn rev(&self) -> GraphRev {
        self.rev
    }

    /// Backlinks, worst case O(B), with early stop at fixed row/byte limits.
    pub fn backlinks(&self, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph
                .backlinks_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES),
            Budget::MatchingBlocks,
        )
    }

    /// Unlinked mentions, worst case O(B), with fixed row/byte limits.
    pub fn unlinked_references(&self, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph
                .unlinked_refs_bounded(name, RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES),
            Budget::MatchingBlocks,
        )
    }

    /// Metadata for selected backlink roots, O(B) in the worst case. Refuses
    /// more than 20,000 targets before scanning.
    pub fn backlink_filter_context(
        &self,
        name: &str,
        targets: &[BacklinkFilterTarget],
    ) -> Result<BacklinkFilterContext, QueryError> {
        if targets.len() > RESULT_BRIDGE_MAX_ROWS {
            return Err(QueryError::RequestTooLarge {
                what: Budget::BacklinkFilterRoots,
                count: targets.len(),
                limit: RESULT_BRIDGE_MAX_ROWS,
            });
        }
        Ok(crate::query::backlink_filter_context(
            &self.graph,
            name,
            targets,
        ))
    }

    /// Resolve block identities in request order; unknown ids yield `None`.
    /// Cost: hinted pages' blocks, or O(B) for unhinted ids. Fixed result cap.
    pub fn blocks(&self, uuids: &[String]) -> Result<Vec<Option<RefGroup>>, QueryError> {
        if uuids.len() > RESULT_BRIDGE_MAX_ROWS {
            return Err(QueryError::ResultTooLarge {
                what: Budget::RequestedBlockRefs,
                count: uuids.len(),
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: None,
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            });
        }
        let (groups, exceeded, total) = crate::query::resolve_blocks_bounded(
            &self.graph,
            uuids,
            RESULT_BRIDGE_MAX_ROWS,
            RESULT_BRIDGE_MAX_BYTES,
        );
        if exceeded {
            Err(QueryError::ResultTooLarge {
                what: Budget::ResolvedBlockRows,
                count: total,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: None,
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            })
        } else {
            Ok(groups)
        }
    }

    /// Bounded subtree preview; 1..=2000 nodes and a fixed byte cap. Cost:
    /// one hinted page's blocks, or O(B) if no hint. Unknown id yields `None`.
    pub fn preview_block(
        &self,
        uuid: &str,
        max_nodes: usize,
    ) -> Result<Option<BlockPreview>, QueryError> {
        let preview = self.graph.preview_block_with_budget(
            uuid,
            max_nodes.clamp(1, MAX_PREVIEW_NODES),
            PREVIEW_MAX_BYTES,
        );
        if let Some(value) = &preview {
            let groups = std::slice::from_ref(&value.group);
            let rows = groups.iter().map(|g| g.blocks.len()).sum::<usize>();
            let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
            if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
                return Err(QueryError::ResultTooLarge {
                    what: Budget::BridgeMatchingBlocks,
                    count: rows,
                    limit: RESULT_BRIDGE_MAX_ROWS,
                    bytes: Some(bytes),
                    byte_limit: RESULT_BRIDGE_MAX_BYTES,
                });
            }
        }
        Ok(preview)
    }

    /// Block referrers, worst case O(B), with fixed row/byte limits.
    pub fn block_referrers(&self, uuid: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
        bounded(
            self.graph.block_referrers_bounded(
                uuid,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            ),
            Budget::MatchingBlocks,
        )
    }

    /// Referenced block counts; O(B) on a cache miss.
    pub fn block_ref_counts(&self) -> Arc<HashMap<String, usize>> {
        self.graph.block_ref_counts()
    }

    /// `[[` completion over pages, journals, aliases and referenced names.
    /// Cost O(P + aliases + referenced names), at most `limit` entries.
    pub fn complete_page_names(&self, text: &str, limit: usize) -> Vec<PageEntry> {
        self.graph.quick_switch(text, limit)
    }

    /// Literal `((` block search, O(B), at most `limit` blocks. Checks `cancel`
    /// before each page and block; cancellation returns no partial result.
    pub fn find_blocks(
        &self,
        text: &str,
        limit: usize,
        cancel: &Cancel,
    ) -> Result<Vec<RefGroup>, QueryError> {
        let result = crate::query::search_cancellable_result(
            &self.graph,
            text,
            limit.min(RESULT_BRIDGE_MAX_ROWS),
            || cancel.0.load(Ordering::Acquire),
        );
        let groups = result.ok_or(QueryError::Cancelled)?;
        let rows = groups.iter().map(|g| g.blocks.len()).sum::<usize>();
        let bytes = tine_core::model::ref_groups_estimated_bytes(&groups);
        if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
            Err(QueryError::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count: rows,
                limit: RESULT_BRIDGE_MAX_ROWS,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            })
        } else {
            Ok(groups)
        }
    }

    /// Selected query subtrees, O(64 × B) plus selected nodes. Fixed request,
    /// root, node and byte limits; oversized requests fail before evaluation.
    pub fn export_query_subtrees(
        &self,
        specs: &[QueryExportSpec],
    ) -> Result<QueryExportBatch, QueryError> {
        let query_bytes = specs.iter().fold(0usize, |n, s| {
            n.saturating_add(s.key.len()).saturating_add(s.query.len())
        });
        if specs.len() > QUERY_EXPORT_REQUEST_MAX_QUERIES
            || query_bytes > QUERY_EXPORT_MAX_QUERY_BYTES
        {
            return Err(QueryError::ExportRequestTooLarge {
                macros: specs.len(),
                bytes: query_bytes,
                macro_limit: QUERY_EXPORT_REQUEST_MAX_QUERIES,
                byte_limit: QUERY_EXPORT_MAX_QUERY_BYTES,
                processing_cap: QUERY_EXPORT_MAX_QUERIES,
            });
        }
        let batch = crate::query::export_query_subtrees(
            &self.graph,
            specs,
            QUERY_EXPORT_MAX_QUERIES,
            QUERY_EXPORT_MAX_ROOTS,
            QUERY_EXPORT_MAX_NODES,
            QUERY_EXPORT_MAX_BYTES,
        );
        let bytes = batch
            .results
            .iter()
            .map(|r| {
                r.key.len()
                    + r.groups
                        .iter()
                        .map(|g| {
                            tine_core::model::ref_groups_estimated_bytes(std::slice::from_ref(g))
                        })
                        .sum::<usize>()
                    + 128
            })
            .sum::<usize>();
        if bytes > QUERY_EXPORT_MAX_BYTES {
            Err(QueryError::ResultTooLarge {
                what: Budget::ExportBytes,
                count: bytes,
                limit: QUERY_EXPORT_MAX_BYTES,
                bytes: Some(bytes),
                byte_limit: RESULT_BRIDGE_MAX_BYTES,
            })
        } else {
            Ok(batch)
        }
    }

    /// Query-builder facets reject overflow; editor autocomplete returns a
    /// bounded prefix. O(B), fixed item and byte limits.
    pub fn property_facets(
        &self,
        policy: FacetPolicy,
    ) -> Result<Vec<(String, Vec<String>)>, QueryError> {
        match policy {
            FacetPolicy::Budgeted => {
                let (facets, exceeded) = crate::query::property_facets_bounded(
                    &self.graph,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err(QueryError::ResultTooLarge {
                        what: Budget::PropertyFacets,
                        count: 0,
                        limit: RESULT_BRIDGE_MAX_ROWS,
                        bytes: None,
                        byte_limit: RESULT_BRIDGE_MAX_BYTES,
                    })
                } else {
                    Ok(facets)
                }
            }
            FacetPolicy::Truncated => Ok(crate::query::autocomplete_property_facets_bounded(
                &self.graph,
                AUTOCOMPLETE_FACET_MAX_ITEMS,
                AUTOCOMPLETE_FACET_MAX_BYTES,
            )
            .0),
        }
    }

    /// Template blocks, O(B) on a cache miss.
    pub fn templates(&self) -> Vec<TemplateDto> {
        self.graph.templates()
    }

    /// Icons for requested names, O(names + aliases).
    pub fn page_icons(&self, names: &[String]) -> HashMap<String, String> {
        self.graph.page_icons(names)
    }

    /// Journal days with content, O(journals + their blocks).
    pub fn journal_content_days(&self) -> Vec<Day> {
        self.graph
            .journal_content_days()
            .into_iter()
            .map(Day)
            .collect()
    }
}
