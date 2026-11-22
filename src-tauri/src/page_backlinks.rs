//! All pages' backlink-count column: a separate graph-wide count read, kept
//! out of `commands.rs` (an oversized file the size ratchet holds).

use crate::state::GraphContext;
use tine_graph_features::inventory_wire::{page_backlink_counts_wire, PageBacklinkCounts};

/// Initial count snapshot: owner files by path, reference-only pages by name,
/// and publication revision. Later answers arrive in publication deltas.
#[tauri::command]
pub(crate) async fn page_backlink_counts(
    state: GraphContext<'_>,
) -> Result<PageBacklinkCounts, String> {
    crate::commands::off_ui_graph_read(&state, |view| Ok(page_backlink_counts_wire(&view))).await
}
