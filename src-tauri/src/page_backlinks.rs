//! All pages' backlink-count column: a separate graph-wide count read, kept
//! out of `commands.rs` (an oversized file the size ratchet holds).

use crate::state::GraphContext;

/// Initial count snapshot: owner files by path, reference-only pages by name,
/// and publication revision. Later answers arrive in publication deltas.
#[tauri::command]
pub(crate) async fn page_backlink_counts(
    state: GraphContext<'_>,
) -> Result<(Vec<(String, usize)>, Vec<(String, usize)>, String), String> {
    crate::commands::off_ui_graph_read(&state, |view| {
        let (files, references) = view.page_backlink_counts();
        Ok((files, references, String::from(view.rev())))
    })
    .await
}
