//! Thin IPC for reviewed query and whole-graph live export. Heavy work runs on
//! the blocking pool; a graph binding is checked before an export starts.

use crate::state::{slot_for_context, GraphContext};
use std::path::PathBuf;
use tauri::Manager;
use tine_graph_features::publish_query::{
    self, AssetBudgetExceeded, ExportReceipt, QueryExportPlan, QueryExportRequest,
};
use tine_graph_features::{SheetExport, SheetInput};

fn embedded_bundle(app: &tauri::AppHandle) -> Vec<(String, Vec<u8>)> {
    let resolver = app.asset_resolver();
    let mut files: Vec<_> = resolver
        .iter()
        .map(|(path, _)| path.into_owned())
        .filter(|path| {
            let name = path.trim_start_matches('/');
            name == "index.html" || (name.starts_with("assets/") && !name.contains(".."))
        })
        .filter_map(|path| {
            resolver
                .get(path.clone())
                .map(|asset| (path.trim_start_matches('/').to_owned(), asset.bytes))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Resolve a query's owner pages without writing. Cost O(graph query plus
/// selected source bytes); errors are surfaced as a refusal in the dialog.
#[tauri::command]
pub(crate) async fn publish_query_plan(
    request: QueryExportRequest,
    state: GraphContext<'_>,
) -> Result<QueryExportPlan, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        publish_query::plan_query(&slot.store, &request).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicationError {
    kind: &'static str,
    message: String,
}
impl PublicationError {
    fn other(message: String) -> Self {
        Self {
            kind: "publication",
            message,
        }
    }
    fn from_io(error: std::io::Error) -> Self {
        let budget = error
            .get_ref()
            .is_some_and(|e| e.is::<AssetBudgetExceeded>());
        Self {
            kind: if budget { "assetBudget" } else { "publication" },
            message: error.to_string(),
        }
    }
}

/// Publish a reviewed query into the graph's query-output leaf. Typed asset
/// refusals let the dialog offer Settings; all heavy work runs off the UI thread.
#[tauri::command]
pub(crate) async fn publish_query(
    request: QueryExportRequest,
    fingerprint: String,
    sheets: Vec<SheetExport>,
    state: GraphContext<'_>,
) -> Result<ExportReceipt, PublicationError> {
    let slot = slot_for_context(&state).map_err(PublicationError::other)?;
    let bundle = embedded_bundle(state.window.app_handle());
    tauri::async_runtime::spawn_blocking(move || {
        publish_query::publish_query_with_sheets(
            &slot.store,
            &request,
            &fingerprint,
            &bundle,
            sheets,
        )
        .map_err(PublicationError::from_io)
    })
    .await
    .map_err(|e| PublicationError::other(e.to_string()))?
}

/// Publish the public graph, or explicitly all pages, as a read-only browser
/// app with static HTML fallback. Writes only to a user-selected destination.
#[tauri::command]
pub(crate) async fn publish_live(
    destination: String,
    name: String,
    all_pages: bool,
    sheets: Vec<SheetExport>,
    state: GraphContext<'_>,
) -> Result<ExportReceipt, String> {
    let slot = slot_for_context(&state)?;
    let bundle = embedded_bundle(state.window.app_handle());
    tauri::async_runtime::spawn_blocking(move || {
        publish_query::publish_live_with_sheets(
            &slot.store,
            &PathBuf::from(destination),
            &name,
            all_pages,
            &bundle,
            sheets,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The sheet blocks (`tine.view`) of the named pages, or of every page, with the
/// data the frontend's sheet evaluator needs to compute each for a static export
/// (print or publish). The frontend answers with one `SheetExport` per block;
/// the Rust publisher lays them out and computes nothing itself (I-12).
#[tauri::command]
pub(crate) async fn sheet_export_inputs(
    pages: Option<Vec<String>>,
    scope: Option<tine_graph_features::publish::SheetScope>,
    state: GraphContext<'_>,
) -> Result<Vec<SheetInput>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::publish::sheet_export_inputs(
            &slot.store,
            pages.as_deref(),
            scope.as_ref(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
