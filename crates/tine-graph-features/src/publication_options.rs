//! Query publication input shared by the planner, CLI and quick share.
use crate::render;
use serde::{Deserialize, Serialize};
use tine_core::query::wire_parse::QueryTextDialect;

/// The same raw argument, dialect and host properties passed to `query_parse`.
/// A name chooses a portable query leaf in the graph; a current
/// page binds current-page inputs when reviewing the source graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryExportRequest {
    pub argument: String,
    pub dialect: QueryTextDialect,
    #[serde(default)]
    pub properties: Vec<(String, String)>,
    #[serde(default)]
    pub current_page: Option<String>,
    /// Focused route page for textual variables during source review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub textual_current_page: Option<String>,
    pub name: String,
    #[serde(default)]
    pub host_block_id: Option<String>,
    /// Explicit reviewed output name, otherwise derived from name.
    #[serde(default)]
    pub folder: Option<String>,
    /// Preserve and replace the current leaf rather than refusing a collision.
    #[serde(default)]
    pub replace: bool,
    /// Device-local cumulative asset budget; None uses 1 GiB.
    #[serde(default)]
    pub asset_budget_bytes: Option<u64>,
    /// "Include the query text" (PK3, off by default): only then does the
    /// published home carry the authored query macro; otherwise the home lists
    /// the exported pages.
    #[serde(default)]
    pub include_query_text: bool,
    #[serde(default)]
    pub include_comments: Option<bool>,
}
/// A page the query selected that the export leaves out, and why.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftOutPage {
    pub name: String,
    pub path: String,
    pub reason: render::LeftOut,
}

/// One complete owner page in a reviewed query selection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPage {
    pub name: String,
    pub path: String,
    pub journal: bool,
}

/// Stateless plan shown before publication. `fingerprint` binds selected
/// source documents, query input and result membership; no server-side session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryExportPlan {
    pub include_comments: bool,
    pub uses_current_page: bool,
    pub anchor: String,
    pub row_count: usize,
    pub pages: Vec<ExportPage>,
    /// Selected by the query but not exported (`public:: false`, the exports page).
    pub left_out: Vec<LeftOutPage>,
    /// The query's own page (`current_page`) is among the exported pages, so the
    /// export carries the page the query is written on (PK3 (c) quiet notice).
    pub selects_host_page: bool,
    pub folder: String,
    pub path: String,
    pub exists: bool,
    pub suggested_folder: Option<String>,
    pub fingerprint: String,
    /// Read-only membership evidence from the previous leaf; no new stored record.
    pub previous_pages: Option<Vec<String>>,
}

/// Published destination and number of selected source pages.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceipt {
    pub path: String,
    pub pages: usize,
    pub files: u64,
    pub retired: Option<String>,
    pub warnings: Vec<String>,
}
