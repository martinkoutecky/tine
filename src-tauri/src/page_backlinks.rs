//! All pages' backlink-count column: a separate graph-wide count read, kept
//! out of `commands.rs` (an oversized file the size ratchet holds).

use crate::state::GraphContext;

/// Separate graph-wide count read; All pages reads it on open and after the
/// graph is quiet (HK3), one read in flight: `(owner files by path,
/// reference-only pages by name)` (`WholeGraph::page_backlink_counts`).
#[tauri::command]
pub(crate) async fn page_backlink_counts(
    state: GraphContext<'_>,
) -> Result<(Vec<(String, usize)>, Vec<(String, usize)>), String> {
    crate::commands::off_ui_graph_read(&state, |view| Ok(view.page_backlink_counts())).await
}
