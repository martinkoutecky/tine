//! Snapshot reads shared by desktop IPC and the browser core. These adapt
//! captured answers without I/O. Reference reads keep the store's row and
//! byte limits, and errors keep the existing command wire families.

use std::collections::HashMap;
use std::sync::Arc;

use tine_core::model::{BacklinkFilterContext, BacklinkFilterTarget, GraphMeta, RefGroup};
use tine_core::BlockPreview;
use tine_store::{ConfigState, FacetPolicy, QueryError, WholeGraph};

use crate::query_error_wire::query_error;

/// Graph open metadata for the supplied root and effective config. Cost is
/// proportional to config copied into the response, with no page access.
pub fn graph_meta_wire(root: String, config: &ConfigState) -> GraphMeta {
    GraphMeta::from_config(root, &config.config, &config.journal_format())
}

/// Copy the indexed journal content days as ordinals, O(content days).
pub fn journal_content_days(view: &WholeGraph) -> Vec<i64> {
    view.journal_content_days()
        .into_iter()
        .map(|day| day.0)
        .collect()
}

/// Icons for the requested names, with indexed alias lookup.
pub fn page_icons(view: &WholeGraph, names: &[String]) -> HashMap<String, String> {
    view.page_icons(names)
}

/// Linked reference groups, bounded by the store, O(blocks) worst case.
pub fn backlinks(view: &WholeGraph, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
    view.backlinks(name)
}

/// Visible text and facets for selected backlink roots, O(blocks) worst case.
pub fn backlink_filter_context(
    view: &WholeGraph,
    name: &str,
    targets: &[BacklinkFilterTarget],
) -> Result<BacklinkFilterContext, String> {
    view.backlink_filter_context(name, targets)
        .map_err(query_error)
}

/// Unlinked mention groups, bounded by the store, O(blocks + matching text).
pub fn unlinked_refs(view: &WholeGraph, name: &str) -> Result<Arc<Vec<RefGroup>>, QueryError> {
    view.unlinked_references(name)
}

/// The captured block-reference counts, shared with no copy until serialization.
pub fn block_ref_counts(view: &WholeGraph) -> Arc<HashMap<String, usize>> {
    view.block_ref_counts()
}

/// The bounded referrers of one block identifier.
pub fn block_referrers(view: &WholeGraph, uuid: &str) -> Result<Arc<Vec<RefGroup>>, String> {
    view.block_referrers(uuid).map_err(query_error)
}

/// Shallow active block resolution, one indexed lookup per requested id.
pub fn resolve_blocks(
    view: &WholeGraph,
    uuids: &[String],
) -> Result<Vec<Option<RefGroup>>, String> {
    view.blocks(uuids, true).map_err(query_error)
}

/// A bounded subtree preview of one block.
pub fn preview_block(
    view: &WholeGraph,
    uuid: &str,
    max_nodes: usize,
) -> Result<Option<BlockPreview>, String> {
    view.preview_block(uuid, max_nodes).map_err(query_error)
}

/// Property completion asks for the keys, or one key's values.
#[derive(Debug, serde::Deserialize)]
pub struct AutocompleteFacetScope {
    keys: Option<bool>,
    key: Option<String>,
}

impl AutocompleteFacetScope {
    /// Decode the request before acquiring a graph publication, O(1).
    pub fn into_policy(self) -> Result<FacetPolicy, String> {
        match self {
            Self { key: Some(key), .. } => Ok(FacetPolicy::CompletionValues(key)),
            Self {
                keys: Some(true), ..
            } => Ok(FacetPolicy::CompletionKeys),
            _ => Err("query_facets needs a `key` or `keys: true` scope".into()),
        }
    }
}

/// Indexed, bounded property completions for the selected scope.
pub fn query_facets(view: &WholeGraph, policy: FacetPolicy) -> Vec<(String, Vec<String>)> {
    view.property_facets(policy)
}
